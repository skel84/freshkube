//! Coroot's reported checks and evidence, without local health thresholds.
use super::*;
use freshkube_core::coroot as api;

pub(super) struct ReportSnapshot {
    key: ReportKey,
    tabs: Vec<(String, SharedString)>,
    rest: Option<Evidence>,
    extended: Option<Evidence>,
    subject: Option<api::ObjectSubject>,
}

#[derive(PartialEq)]
struct ReportKey {
    identity: Option<connection::ReadIdentity>,
    range: api::TimeRange,
    app: api::AppId,
    report: String,
}

struct Evidence {
    status: Status,
    entries: Vec<EvidenceEntry>,
    pages: Vec<EvidencePage>,
    page: usize,
}
struct EvidenceEntry {
    status: Status,
    label: String,
    link: Option<(api::AppId, SharedString)>,
}
struct EvidencePage {
    entries: std::ops::Range<usize>,
    label: String,
}
fn number(value: Option<f64>) -> String {
    value
        .filter(|v| v.is_finite())
        .map_or("Not reported".into(), |v| format!("{v:.4}"))
}
fn summary(title: &str, series: &api::SeriesSummary) -> String {
    let mut text = format!(
        "{title} · {} · last {} · min {} · max {} · avg {} · {} summary points ({} missing)",
        series.name,
        number(series.last),
        number(series.min),
        number(series.max),
        number(series.avg),
        series.sparkline.len(),
        series.sparkline.iter().filter(|v| v.is_none()).count()
    );
    if !series.labels.is_empty() {
        text.push_str(" · labels: ");
        text.push_str(
            &series
                .labels
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    text
}
fn evidence(health: &api::AppHealth, name: &str, prefix: &str) -> Evidence {
    let mut rows = vec![];
    if let Some(report) = health
        .reports
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(name))
    {
        rows.push((
            report.status.into(),
            format!("{} · {}", report.name, Status::from(report.status).label()),
        ));
        if report.issues.is_empty() {
            rows.push((
                Status::Info,
                "No issue entries reported. The report's status is shown above.".into(),
            ));
        }
        for issue in &report.issues {
            rows.push((
                issue.status.into(),
                format!("{} · {}\n{}", issue.id, issue.title, issue.message),
            ));
        }
        for chart in &report.charts {
            if chart.series.is_empty() {
                rows.push((
                    Status::Unknown,
                    format!("{} · No series reported", chart.title),
                ));
            }
            for series in &chart.series {
                rows.push((Status::Info, summary(&chart.title, series)));
            }
            if chart.series_omitted > 0 {
                rows.push((
                    Status::Unknown,
                    format!(
                        "{} · {} series omitted by Coroot",
                        chart.title, chart.series_omitted
                    ),
                ));
            }
        }
        for log in &report.log_patterns {
            rows.push((
                Status::Info,
                format!(
                    "Log pattern {} · {} · {} messages\n{}",
                    log.hash, log.severity, log.messages, log.sample
                ),
            ));
        }
    } else {
        rows.push((
            Status::Absent,
            format!("{name} · Not reported by this source"),
        ));
    }
    for vital in &health.vitals {
        rows.push((Status::Info, summary("Vital", vital)));
    }
    let mut entries: Vec<_> = rows
        .into_iter()
        .map(|(status, label)| EvidenceEntry {
            status,
            label,
            link: None,
        })
        .collect();
    entries.extend(health.dependencies.iter().map(|d| EvidenceEntry {
        status: d.status.into(),
        label: format!(
            "Calls {} · {}\nConnectivity: {} · {}\n{} rps · RTT {} s · errors {} /s · latency avg {} s · p95 {} s · protocols {}",
            d.id.short(), Status::from(d.status).label(), Status::from(d.connectivity).label(),
            d.connectivity_message, number(d.rps), number(d.rtt_seconds), number(d.errors_per_sec),
            number(d.latency_seconds.as_ref().and_then(|l| l.avg)),
            number(d.latency_seconds.as_ref().and_then(|l| l.p95)), d.protocols.join(", "),
        ),
        link: Some((d.id.clone(), format!("obs-{prefix}-dependency-{}", d.id).into())),
    }));
    entries.extend(health.clients.iter().map(|c| EvidenceEntry {
        status: c.status.into(),
        label: format!(
            "Called by {} · {} · {} rps · latency {} s",
            c.id.short(),
            Status::from(c.status).label(),
            number(c.rps),
            number(c.latency_seconds)
        ),
        link: Some((c.id.clone(), format!("obs-{prefix}-client-{}", c.id).into())),
    }));
    // Retain every bounded entry, but lay out only a small page. Long Coroot
    // messages must not turn an otherwise small report into a huge frame.
    let mut pages = vec![];
    let (mut start, mut bytes) = (0, 0);
    for (ix, entry) in entries.iter().enumerate() {
        if ix > start && (ix - start == 24 || bytes + entry.label.len() > 16_384) {
            pages.push(start..ix);
            start = ix;
            bytes = 0;
        }
        bytes += entry.label.len();
    }
    pages.push(start..entries.len());
    let pages = pages
        .into_iter()
        .map(|range| EvidencePage {
            label: format!(
                "Evidence {}–{} of {}",
                range.start + 1,
                range.end,
                entries.len()
            ),
            entries: range,
        })
        .collect();
    Evidence {
        status: health.status.into(),
        entries,
        pages,
        page: 0,
    }
}
impl ObservabilityPage {
    pub(super) fn prepare_report(&mut self) {
        let Some(id) = &self.selected_app else {
            self.report_snapshot = None;
            return;
        };
        let key = ReportKey {
            identity: self
                .live
                .identity(connection::Subject::Report(id.clone(), false)),
            range: self.live.range,
            app: id.clone(),
            report: self.report_name.clone(),
        };
        let previous = self
            .report_snapshot
            .as_ref()
            .filter(|previous| previous.key == key);
        let pages = previous.map_or([0, 0], |previous| {
            [
                previous.rest.as_ref().map_or(0, |e| e.page),
                previous.extended.as_ref().map_or(0, |e| e.page),
            ]
        });
        let prepare = |health, prefix, page: usize| {
            let mut evidence = evidence(health, &self.report_name, prefix);
            evidence.page = page.min(evidence.pages.len() - 1);
            evidence
        };
        let fixture_rest;
        let fixture_extended;
        let (rest, extended) = if self.fixture {
            fixture_rest = example::health(id, false);
            fixture_extended = example::health(id, true);
            (Some(&fixture_rest), Some(&fixture_extended))
        } else {
            (self.live.rest.data(), self.live.extended.data())
        };
        let tabs: std::collections::BTreeSet<_> = rest
            .into_iter()
            .chain(extended)
            .flat_map(|h| h.reports.iter().map(|r| r.name.clone()))
            .collect();
        self.report_snapshot = Some(ReportSnapshot {
            key,
            tabs: tabs
                .into_iter()
                .map(|name| {
                    let key = format!("obs-report-{}", name.to_lowercase());
                    (name, key.into())
                })
                .collect(),
            rest: rest.map(|h| prepare(h, "rest", pages[0])),
            extended: extended.map(|h| prepare(h, "extended", pages[1])),
            subject: self
                .live
                .source
                .as_ref()
                .and_then(|s| api::ObjectSubject::for_app(s, id)),
        });
    }
    pub(super) fn render_report(&self, _window: &Window, cx: &Context<Self>) -> AnyElement {
        let Some(id) = &self.selected_app else {
            return text("Select an application to inspect its reports").into_any_element();
        };
        let mut content=v_flex().gap(dp(14.)).child(self.breadcrumbs("Applications",Destination::Applications,cx))
            .child(muted("Coroot supplies health and check results. This integration is read-only. Charts are summaries; complete histories are not available in the client.",cx));
        if self.fixture && *id == example::id(example::WORKER) {
            content = content.child(
                line()
                    .flex_wrap()
                    .child(action("obs-open-pod", "Example pod").on_click(cx.listener(
                        |_, _, _, cx| {
                            cx.emit(ObservabilityEvent::OpenExamplePod {
                                namespace: "payments".into(),
                                name: example::POD.into(),
                                logs: false,
                            })
                        },
                    )))
                    .child(
                        action("obs-open-logs", "Example pod logs").on_click(cx.listener(
                            |_, _, _, cx| {
                                cx.emit(ObservabilityEvent::OpenExamplePod {
                                    namespace: "payments".into(),
                                    name: example::POD.into(),
                                    logs: true,
                                })
                            },
                        )),
                    )
                    .child(action("obs-threshold", "Example threshold…").on_click(
                        cx.listener(|this, _, window, cx| this.edit_threshold(window, cx)),
                    ))
                    .child(action("obs-app-rollback", "Preview rollback…").on_click(
                        cx.listener(|this, _, window, cx| this.preview("Roll back", window, cx)),
                    )),
            );
        }
        if let Some(snapshot) = &self.report_snapshot {
            content = content.child(line().flex_wrap().children(snapshot.tabs.iter().map(
                |(name, key)| {
                    let name = name.clone();
                    action(key.clone(), name.clone())
                        .selected(self.report_name == name)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.report_name = name.clone();
                            this.prepare_report();
                            cx.notify();
                        }))
                },
            )));
            if let Some(subject) = &snapshot.subject {
                let subject = subject.clone();
                let source = self
                    .live
                    .source
                    .clone()
                    .expect("a subject has an associated source");
                let app = id.clone();
                content = content.child(
                    action("obs-open-object", "Open Kubernetes object").on_click(cx.listener(
                        move |_, _, _, cx| {
                            cx.emit(ObservabilityEvent::OpenObject {
                                source: source.clone(),
                                app: app.clone(),
                                subject: Box::new(subject.clone()),
                            });
                        },
                    )),
                );
            } else if !self.fixture {
                content=content.child(muted("Kubernetes link unavailable: external, unsupported kind, or cluster not explicitly associated.",cx));
            }
            content = content
                .child(self.render_evidence(
                    "Coroot reports (REST)",
                    snapshot.rest.as_ref(),
                    false,
                    cx,
                ))
                .child(self.render_evidence(
                    "Additional evidence (MCP)",
                    snapshot.extended.as_ref(),
                    true,
                    cx,
                ));
        } else {
            content = content.child(text("Reading application reports…"));
        }
        content.into_any_element()
    }
    fn render_evidence(
        &self,
        title: &str,
        evidence: Option<&Evidence>,
        extended: bool,
        cx: &Context<Self>,
    ) -> Div {
        let snapshot = if extended {
            &self.live.extended
        } else {
            &self.live.rest
        };
        let mut content = body();
        if !self.fixture {
            if snapshot.is_loading() {
                content = content.child(muted("Reading…", cx));
            }
            if snapshot.is_stale() {
                content = content.child(text("Last known evidence · stale"));
            }
            if let Some(error) = snapshot.error() {
                content = content
                    .child(text(error.to_string()).text_color(palette(cx).warn_ink))
                    .child(
                        action(
                            if extended {
                                "obs-retry-extended"
                            } else {
                                "obs-retry-rest"
                            },
                            "Retry this window",
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    );
            }
        }
        if let Some(evidence) = evidence {
            content = content.child(
                line()
                    .child(status(evidence.status, cx))
                    .child(text(format!("Application: {}", evidence.status.label()))),
            );
            let page = &evidence.pages[evidence.page];
            if evidence.pages.len() > 1 {
                content = content.child(
                    line()
                        .flex_wrap()
                        .child(muted(page.label.clone(), cx))
                        .children([(false, "Previous evidence"), (true, "Next evidence")].map(
                            |(next, label)| {
                                action(
                                    SharedString::from(format!("obs-evidence-{extended}-{next}")),
                                    label,
                                )
                                .disabled(if next {
                                    evidence.page + 1 == evidence.pages.len()
                                } else {
                                    evidence.page == 0
                                })
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if let Some(snapshot) = &mut this.report_snapshot {
                                            let evidence = if extended {
                                                &mut snapshot.extended
                                            } else {
                                                &mut snapshot.rest
                                            };
                                            if let Some(evidence) = evidence {
                                                evidence.page = if next {
                                                    (evidence.page + 1)
                                                        .min(evidence.pages.len() - 1)
                                                } else {
                                                    evidence.page.saturating_sub(1)
                                                };
                                                cx.notify();
                                            }
                                        }
                                    },
                                ))
                            },
                        )),
                );
            }
            content =
                content.children(evidence.entries[page.entries.clone()].iter().map(|entry| {
                    v_flex()
                        .gap(dp(6.))
                        .child(
                            line()
                                .items_start()
                                .child(status(entry.status, cx))
                                .child(text(entry.label.clone()).flex_1().min_w_0()),
                        )
                        .when_some(entry.link.clone(), |row, (id, key)| {
                            row.child(action(key, "Open application report").on_click(cx.listener(
                                move |this, _, _, cx| this.open_app(id.clone(), Report::Net, cx),
                            )))
                        })
                }));
        }
        card(title.to_string(), cx).child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Destination, Status, connection, example};
    use super::{ReportKey, ReportSnapshot, api, evidence};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, TestAppContext};

    #[gpui_kit::test]
    fn large_report_evidence_is_paged_without_losing_source_text(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = super::super::tests::mount(cx, true);
        let id = api::AppId::new("fixture:prod:Deployment:large-report");
        let mut health = example::health(&id, false);
        let report = health.reports.iter_mut().find(|r| r.name == "Net").unwrap();
        report.status = api::Status::Unknown;
        report.issues = (0..200)
            .map(|ix| api::Issue {
                id: format!("source-{ix}"),
                status: api::Status::Unknown,
                title: "Sanitized title ".repeat(256),
                message: "Sanitized evidence ".repeat(400),
            })
            .collect();
        let start = std::time::Instant::now();
        let evidence = evidence(&health, "Net", "rest");
        eprintln!(
            "Coroot report: prepare 200 long issues {:?}",
            start.elapsed()
        );
        assert_eq!(evidence.entries.len(), 201);
        assert!(evidence.pages.len() > 1);
        assert_eq!(
            evidence
                .pages
                .iter()
                .map(|p| p.entries.len())
                .sum::<usize>(),
            201
        );
        assert!(evidence.pages.iter().all(|p| p.entries.len() <= 24));
        assert!(
            evidence.entries[1].label.ends_with(
                &health
                    .reports
                    .iter()
                    .find(|r| r.name == "Net")
                    .unwrap()
                    .issues[0]
                    .message
            )
        );
        assert_eq!(evidence.entries[1].status, Status::Unknown);
        cx.update(|cx| {
            page.update(cx, |page, _| {
                page.destination = Destination::Application;
                page.selected_app = Some(id);
                page.report_snapshot = Some(ReportSnapshot {
                    key: ReportKey {
                        identity: None,
                        range: page.live.range,
                        app: page.selected_app.clone().unwrap(),
                        report: page.report_name.clone(),
                    },
                    tabs: vec![],
                    rest: Some(evidence),
                    extended: None,
                    subject: None,
                });
            })
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let start = std::time::Instant::now();
            for _ in 0..10 {
                window.render_frame(cx);
            }
            eprintln!("Coroot report: ten headless frames {:?}", start.elapsed());
            window.click("obs-evidence-false-true", cx);
            assert_eq!(
                page.read(cx)
                    .report_snapshot
                    .as_ref()
                    .unwrap()
                    .rest
                    .as_ref()
                    .unwrap()
                    .page,
                1
            );
            window.click("obs-evidence-false-false", cx);
            assert_eq!(
                page.read(cx)
                    .report_snapshot
                    .as_ref()
                    .unwrap()
                    .rest
                    .as_ref()
                    .unwrap()
                    .page,
                0
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn delayed_other_source_keeps_the_current_evidence_page(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = super::super::tests::mount(cx, false);
        let id = api::AppId::new("fixture:prod:Deployment:paged-report");
        let mut health = example::health(&id, false);
        health
            .reports
            .iter_mut()
            .find(|r| r.name == "Net")
            .unwrap()
            .issues = (0..60)
            .map(|ix| api::Issue {
                id: ix.to_string(),
                title: "Source check".into(),
                message: "Source evidence".into(),
                status: api::Status::Unknown,
            })
            .collect();
        let extended = cx.update(|cx| {
            page.update(cx, |page, _| {
                let provider =
                    api::Provider::new("http://127.0.0.1:1", api::Credentials::None).unwrap();
                page.live.source = Some(provider.source(&api::ProjectInfo {
                    id: "p".into(),
                    name: "Fixture".into(),
                }));
                page.live.provider = Some(provider);
                page.destination = Destination::Application;
                page.selected_app = Some(id.clone());
                let key = page
                    .live
                    .identity(connection::Subject::Report(id.clone(), false))
                    .unwrap();
                let request = page.live.rest.begin(key);
                page.live.rest.apply(&request, Ok(health.clone()));
                page.prepare_report();
                let key = page
                    .live
                    .identity(connection::Subject::Report(id.clone(), true))
                    .unwrap();
                page.live.extended.begin(key)
            })
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("obs-evidence-false-true", cx);
            assert_eq!(
                page.read(cx)
                    .report_snapshot
                    .as_ref()
                    .unwrap()
                    .rest
                    .as_ref()
                    .unwrap()
                    .page,
                1
            );
        })
        .unwrap();
        cx.update(|cx| {
            page.update(cx, |page, _| {
                assert!(page.live.extended.apply(&extended, Ok(health.clone())));
                page.prepare_report();
                assert_eq!(
                    page.report_snapshot
                        .as_ref()
                        .unwrap()
                        .rest
                        .as_ref()
                        .unwrap()
                        .page,
                    1
                );
                let key = page
                    .live
                    .identity(connection::Subject::Report(id.clone(), true))
                    .unwrap();
                let retry = page.live.extended.begin(key);
                assert!(
                    page.live
                        .extended
                        .apply(&retry, Err("Additional evidence unavailable".into()))
                );
                page.prepare_report();
                assert_eq!(
                    page.report_snapshot
                        .as_ref()
                        .unwrap()
                        .rest
                        .as_ref()
                        .unwrap()
                        .page,
                    1
                );
                assert!(page.live.extended.is_stale());

                // A shorter successful observation clamps the retained page.
                let key = page
                    .live
                    .identity(connection::Subject::Report(id.clone(), false))
                    .unwrap();
                let retry = page.live.rest.begin(key);
                page.live
                    .rest
                    .apply(&retry, Ok(example::health(&id, false)));
                page.prepare_report();
                assert_eq!(
                    page.report_snapshot
                        .as_ref()
                        .unwrap()
                        .rest
                        .as_ref()
                        .unwrap()
                        .page,
                    0
                );
                // Another report and window reset pagination independently of
                // same-identity completion/retry behavior.
                page.report_snapshot
                    .as_mut()
                    .unwrap()
                    .extended
                    .as_mut()
                    .unwrap()
                    .page = 1;
                page.report_name = "CPU".into();
                page.prepare_report();
                assert_eq!(
                    page.report_snapshot
                        .as_ref()
                        .unwrap()
                        .extended
                        .as_ref()
                        .unwrap()
                        .page,
                    0
                );
                page.report_name = "Net".into();
                page.prepare_report();
                page.report_snapshot
                    .as_mut()
                    .unwrap()
                    .extended
                    .as_mut()
                    .unwrap()
                    .page = 1;
                page.live.range.to = page
                    .live
                    .range
                    .to
                    .map(|to| to + chrono::Duration::minutes(1));
                page.prepare_report();
                assert_eq!(
                    page.report_snapshot
                        .as_ref()
                        .unwrap()
                        .extended
                        .as_ref()
                        .unwrap()
                        .page,
                    0
                );
            })
        });
    }
}
