//! Coroot's reported checks and evidence, without local health thresholds.
use super::*;
use freshkube_core::coroot as api;

pub(super) struct ReportSnapshot {
    key: ReportKey,
    tabs: Vec<(String, SharedString, Status)>,
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
    entries: Vec<EvidenceEntry>,
    pages: Vec<EvidencePage>,
    page: usize,
}
struct EvidenceEntry {
    status: Status,
    /// What the row is: a report's verdict, an issue title, a chart.
    title: String,
    /// The source's own words, shown in full.
    detail: String,
    /// Reported figures only, already formatted with their units.
    metrics: String,
    samples: Vec<f64>,
    /// An application the row points at, with the link's id.
    link: Option<(api::AppId, SharedString)>,
}
struct EvidencePage {
    entries: std::ops::Range<usize>,
    label: String,
}
impl EvidenceEntry {
    fn new(status: Status, title: impl Into<String>) -> Self {
        Self {
            status,
            title: title.into(),
            detail: String::new(),
            metrics: String::new(),
            samples: vec![],
            link: None,
        }
    }
    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }
    fn metrics(mut self, metrics: impl IntoIterator<Item = Option<String>>) -> Self {
        self.metrics = metrics
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        self
    }
    /// The same evidence, whichever source and link id it came with.
    fn same(&self, other: &Self) -> bool {
        self.status == other.status
            && self.title == other.title
            && self.detail == other.detail
            && self.metrics == other.metrics
            && self.samples == other.samples
            && self.link.as_ref().map(|l| &l.0) == other.link.as_ref().map(|l| &l.0)
    }
    fn bytes(&self) -> usize {
        self.title.len() + self.detail.len() + self.metrics.len()
    }
}
fn finite(value: Option<f64>) -> Option<f64> {
    value.filter(|v| v.is_finite())
}
/// A unitless figure with at most three significant digits.
fn figure(value: f64) -> String {
    let digits = match value.abs() {
        v if v >= 100. || v == 0. => 0,
        v if v >= 10. => 1,
        v if v >= 1. => 2,
        _ => 3,
    };
    let text = format!("{value:.digits$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').into()
    } else {
        text
    }
}
fn seconds(value: f64) -> String {
    if value.abs() < 1. {
        format!("{} ms", figure(value * 1000.))
    } else {
        format!("{} s", figure(value))
    }
}
fn labelled(label: &str, value: Option<f64>, unit: fn(f64) -> String) -> Option<String> {
    finite(value).map(|v| format!("{label} {}", unit(v)))
}
fn rps(value: Option<f64>) -> Option<String> {
    finite(value).map(|v| format!("{} rps", figure(v)))
}
fn series(title: &str, series: &api::SeriesSummary) -> EvidenceEntry {
    let mut detail = series.name.clone();
    if !series.labels.is_empty() {
        let labels = series
            .labels
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(", ");
        detail = if detail.is_empty() {
            labels
        } else {
            format!("{detail} · {labels}")
        };
    }
    let missing = series.sparkline.iter().filter(|v| v.is_none()).count();
    let mut entry = EvidenceEntry::new(Status::Info, title)
        .detail(detail)
        .metrics([
            labelled("last", series.last, figure),
            labelled("min", series.min, figure),
            labelled("max", series.max, figure),
            labelled("avg", series.avg, figure),
            (missing > 0)
                .then(|| format!("{missing} of {} points missing", series.sparkline.len())),
        ]);
    entry.samples = series
        .sparkline
        .iter()
        .flatten()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    entry
}
fn evidence(health: &api::AppHealth, name: &str, prefix: &str) -> Evidence {
    let mut entries = vec![];
    if let Some(report) = health
        .reports
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(name))
    {
        let state = Status::from(report.status);
        entries.push(
            EvidenceEntry::new(state, format!("{} · {}", report.name, state.label())).detail(
                if report.issues.is_empty() {
                    "Coroot reported no issues for this check."
                } else {
                    ""
                },
            ),
        );
        for issue in &report.issues {
            entries.push(
                EvidenceEntry::new(issue.status.into(), issue.title.clone())
                    .detail(issue.message.clone())
                    .metrics([Some(issue.id.clone())]),
            );
        }
        for chart in &report.charts {
            if chart.series.is_empty() {
                entries.push(
                    EvidenceEntry::new(Status::Unknown, chart.title.clone())
                        .detail("No series reported"),
                );
            }
            for s in &chart.series {
                entries.push(series(&chart.title, s));
            }
            if chart.series_omitted > 0 {
                entries.push(
                    EvidenceEntry::new(Status::Unknown, chart.title.clone()).detail(format!(
                        "{} more series omitted by Coroot",
                        chart.series_omitted
                    )),
                );
            }
        }
        for log in &report.log_patterns {
            entries.push(
                EvidenceEntry::new(
                    Status::LogError,
                    format!("Log pattern · {} · {} messages", log.severity, log.messages),
                )
                .detail(log.sample.clone())
                .metrics([Some(log.hash.clone())]),
            );
        }
    } else {
        entries.push(
            EvidenceEntry::new(Status::Absent, format!("{name} · Not reported"))
                .detail("This source has no report by that name."),
        );
    }
    entries.extend(health.vitals.iter().map(|v| series("Vital", v)));
    entries.extend(health.dependencies.iter().map(|d| {
        let connectivity = Status::from(d.connectivity);
        let mut entry = EvidenceEntry::new(d.status.into(), format!("Calls {}", d.id.short()))
            .detail(match d.connectivity_message.as_str() {
                "" => format!("Connectivity {}", connectivity.label().to_lowercase()),
                message => format!(
                    "Connectivity {} · {message}",
                    connectivity.label().to_lowercase()
                ),
            })
            .metrics([
                (!d.protocols.is_empty()).then(|| d.protocols.join(", ")),
                rps(d.rps),
                labelled("errors", d.errors_per_sec, |v| format!("{}/s", figure(v))),
                labelled("RTT", d.rtt_seconds, seconds),
                labelled(
                    "avg",
                    d.latency_seconds.as_ref().and_then(|l| l.avg),
                    seconds,
                ),
                labelled(
                    "p95",
                    d.latency_seconds.as_ref().and_then(|l| l.p95),
                    seconds,
                ),
            ]);
        entry.link = Some((
            d.id.clone(),
            format!("obs-{prefix}-dependency-{}", d.id).into(),
        ));
        entry
    }));
    entries.extend(health.clients.iter().map(|c| {
        let mut entry = EvidenceEntry::new(c.status.into(), format!("Called by {}", c.id.short()))
            .metrics([rps(c.rps), labelled("latency", c.latency_seconds, seconds)]);
        entry.link = Some((c.id.clone(), format!("obs-{prefix}-client-{}", c.id).into()));
        entry
    }));
    Evidence {
        pages: paginate(&entries),
        entries,
        page: 0,
    }
}
/// Retain every bounded entry, but lay out only a small page. Long Coroot
/// messages must not turn an otherwise small report into a huge frame.
fn paginate(entries: &[EvidenceEntry]) -> Vec<EvidencePage> {
    let mut pages = vec![];
    let (mut start, mut bytes) = (0, 0);
    for (ix, entry) in entries.iter().enumerate() {
        if ix > start && (ix - start == 24 || bytes + entry.bytes() > 16_384) {
            pages.push(start..ix);
            start = ix;
            bytes = 0;
        }
        bytes += entry.bytes();
    }
    pages.push(start..entries.len());
    pages
        .into_iter()
        .map(|range| EvidencePage {
            label: format!("{}–{} of {}", range.start + 1, range.end, entries.len()),
            entries: range,
        })
        .collect()
}
impl Evidence {
    /// Drops what the primary source already shows, so a second source adds
    /// only its own evidence.
    fn without(mut self, shown: Option<&Evidence>) -> Self {
        if let Some(shown) = shown {
            self.entries
                .retain(|entry| !shown.entries.iter().any(|e| e.same(entry)));
            self.pages = paginate(&self.entries);
        }
        self
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
        // A tab shows the primary source's verdict, or the other's when the
        // primary has no report by that name.
        let mut tabs = BTreeMap::new();
        for health in extended.into_iter().chain(rest) {
            for report in &health.reports {
                tabs.insert(report.name.clone(), Status::from(report.status));
            }
        }
        let rest = rest.map(|h| prepare(h, "rest", pages[0]));
        let extended = extended.map(|h| {
            let mut evidence = evidence(h, &self.report_name, "extended").without(rest.as_ref());
            evidence.page = pages[1].min(evidence.pages.len() - 1);
            evidence
        });
        self.report_snapshot = Some(ReportSnapshot {
            key,
            tabs: tabs
                .into_iter()
                .map(|(name, state)| {
                    let key = format!("obs-report-{}", name.to_lowercase());
                    (name, key.into(), state)
                })
                .collect(),
            rest,
            extended,
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
        let mut actions = line().flex_wrap();
        if self.fixture && *id == example::id(example::WORKER) {
            actions =
                actions
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
                    ));
        }
        let mut content = v_flex().gap(dp(14.));
        let Some(snapshot) = &self.report_snapshot else {
            return content
                .child(self.breadcrumbs("Applications", Destination::Applications, cx))
                .child(muted("Reading application reports…", cx))
                .into_any_element();
        };
        if let Some(subject) = &snapshot.subject {
            let subject = subject.clone();
            let source = self
                .live
                .source
                .clone()
                .expect("a subject has an associated source");
            let app = id.clone();
            actions = actions.child(
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
        }
        content = content
            .child(
                line()
                    .flex_wrap()
                    .justify_between()
                    .child(self.breadcrumbs("Applications", Destination::Applications, cx))
                    .child(actions),
            )
            .child(self.report_tabs(snapshot, cx));
        if snapshot.subject.is_none() && !self.fixture {
            content = content.child(muted(
                "No Kubernetes link: the application is external, of an unsupported kind, or its cluster isn't associated.",
                cx,
            ));
        }
        content
            .child(self.render_evidence(snapshot, cx))
            .into_any_element()
    }
    fn report_tabs(&self, snapshot: &ReportSnapshot, cx: &Context<Self>) -> Div {
        let p = palette(cx);
        line()
            .flex_wrap()
            .gap(dp(2.))
            .p(dp(3.))
            .rounded(px(8.))
            .bg(p.surface_2)
            .children(snapshot.tabs.iter().map(|(name, key, state)| {
                let report = name.clone();
                ui::segment(Button::new(key.clone()), self.report_name == *name, cx)
                    .group("fog-control")
                    .small()
                    .gap(dp(6.))
                    .child(status(*state, cx))
                    .child(text(name.clone()))
                    .tooltip(format!("{name} · {}", state.label()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.report_name = report.clone();
                        this.prepare_report();
                        cx.notify();
                    }))
            }))
    }
    fn render_evidence(&self, snapshot: &ReportSnapshot, cx: &Context<Self>) -> Div {
        let p = palette(cx);
        let mut card = card(format!("{} evidence", self.report_name), cx);
        let primary = snapshot.rest.as_ref();
        let added = snapshot.extended.as_ref().filter(|e| !e.entries.is_empty());
        if primary.is_none() && added.is_none() && !self.fixture {
            card = card.child(body().child(muted("No evidence has been read yet.", cx)));
        }
        if let Some(evidence) = primary {
            card = card.child(self.evidence_rows(evidence, false, cx));
        }
        if let Some(evidence) = added {
            card = card
                .child(
                    h_flex()
                        .h(dp(34.))
                        .px(dp(14.))
                        .border_t_1()
                        .border_color(p.line)
                        .bg(p.surface_2)
                        .child(ui::caption("Also from Coroot MCP", cx)),
                )
                .child(self.evidence_rows(evidence, true, cx));
        }
        card.child(self.evidence_sources(cx))
    }
    fn evidence_rows(&self, evidence: &Evidence, extended: bool, cx: &Context<Self>) -> Div {
        let page = &evidence.pages[evidence.page];
        let rows = v_flex().children(
            evidence.entries[page.entries.clone()]
                .iter()
                .map(|entry| self.evidence_row(entry, cx)),
        );
        if evidence.pages.len() < 2 {
            return rows;
        }
        let pager = line()
            .px(dp(14.))
            .py(dp(6.))
            .border_t_1()
            .border_color(palette(cx).line)
            .child(muted(page.label.clone(), cx))
            .child(div().flex_1())
            .children([(false, "Previous"), (true, "Next")].map(|(next, label)| {
                action(
                    SharedString::from(format!("obs-evidence-{extended}-{next}")),
                    label,
                )
                .disabled(if next {
                    evidence.page + 1 == evidence.pages.len()
                } else {
                    evidence.page == 0
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    let Some(snapshot) = &mut this.report_snapshot else {
                        return;
                    };
                    let evidence = if extended {
                        &mut snapshot.extended
                    } else {
                        &mut snapshot.rest
                    };
                    if let Some(evidence) = evidence {
                        evidence.page = if next {
                            (evidence.page + 1).min(evidence.pages.len() - 1)
                        } else {
                            evidence.page.saturating_sub(1)
                        };
                        cx.notify();
                    }
                }))
            }));
        v_flex().child(pager).child(rows)
    }
    fn evidence_row(&self, entry: &EvidenceEntry, cx: &Context<Self>) -> Div {
        let p = palette(cx);
        let title = line()
            .flex_wrap()
            .child(text(entry.title.clone()).font_weight(FontWeight::SEMIBOLD))
            .when_some(entry.link.clone(), |row, (id, key)| {
                row.child(
                    Button::new(key)
                        .link()
                        .xsmall()
                        .label("Open report")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_app(id.clone(), Report::Net, cx)
                        })),
                )
            });
        h_flex()
            .items_start()
            .gap(dp(10.))
            .px(dp(14.))
            .py(dp(10.))
            .border_t_1()
            .border_color(p.line)
            .child(
                h_flex()
                    .h(dp(20.))
                    .flex_none()
                    .child(status(entry.status, cx)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(dp(3.))
                    .child(title)
                    .when(!entry.detail.is_empty(), |this| {
                        this.child(text(entry.detail.clone()).text_color(p.ink_2))
                    })
                    .when(!entry.metrics.is_empty(), |this| {
                        this.child(
                            mono(entry.metrics.clone())
                                .text_size(dp(12.))
                                .text_color(p.muted),
                        )
                    }),
            )
            .when(entry.samples.len() > 1, |this| {
                this.child(
                    div()
                        .flex_none()
                        .w(dp(120.))
                        .h(dp(32.))
                        .child(ui::sparkline(entry.samples.clone(), None, cx).size_full()),
                )
            })
    }
    /// Where the evidence came from and how current each source is.
    fn evidence_sources(&self, cx: &Context<Self>) -> Div {
        let p = palette(cx);
        let mut footer = line()
            .flex_wrap()
            .gap(dp(16.))
            .px(dp(14.))
            .py(dp(10.))
            .border_t_1()
            .border_color(p.line);
        if self.fixture {
            return footer.child(muted(
                "Example data · Coroot supplies every status · charts are summaries",
                cx,
            ));
        }
        for (extended, name) in [(false, "Coroot API"), (true, "Coroot MCP")] {
            let source = if extended {
                &self.live.extended
            } else {
                &self.live.rest
            };
            let (state, label) = if let Some(error) = source.error() {
                (Status::Warning, error.to_string())
            } else if source.is_loading() {
                (Status::Unknown, "Reading…".into())
            } else if source.is_stale() {
                (Status::Unknown, "Last known · stale".into())
            } else if source.data().is_some() {
                (Status::Ok, "Current".into())
            } else {
                (Status::Absent, "Not read".into())
            };
            footer = footer.child(
                line()
                    .child(status(state, cx))
                    .child(muted(name, cx).font_weight(FontWeight::SEMIBOLD))
                    .child(muted(label, cx))
                    .when(source.error().is_some(), |this| {
                        this.child(
                            Button::new(if extended {
                                "obs-retry-extended"
                            } else {
                                "obs-retry-rest"
                            })
                            .link()
                            .xsmall()
                            .label("Retry")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                        )
                    }),
            );
        }
        footer
            .child(div().flex_1())
            .child(muted("Read-only · charts are summaries", cx))
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
            evidence.entries[1].detail.ends_with(
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
