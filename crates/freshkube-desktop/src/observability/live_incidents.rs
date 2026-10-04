//! Bounded incident observations. The page owns identity, selection and prepared
//! strings; Coroot owns SLO/RCA interpretation. Render never invents evidence.
use super::*;
use freshkube_core::coroot as api;
const PAGE_SIZE: usize = 10;

#[derive(Default)]
pub(super) struct Incidents {
    selected: Option<(String, api::AppId)>,
    rows: Vec<Row>,
    detail: Option<Detail>,
    page_ix: usize,
    count: Option<String>,
    title: String,
    page_label: String,
    related_page_ix: usize,
    related_page_label: String,
}
struct Row {
    key: String,
    app: api::AppId,
    title: String,
    summary: String,
    severity: Status,
}
struct Objective {
    name: &'static str,
    objective: String,
    compliance: String,
    severity: Status,
    impact: String,
    rates: Vec<String>,
}
struct Evidence {
    title: &'static str,
    text: SharedString,
}
struct Related {
    app: api::AppId,
    label: String,
    severity: Status,
    issues: SharedString,
}
struct Detail {
    key: String,
    app: api::AppId,
    title: String,
    state: &'static str,
    severity: Status,
    summary: String,
    objectives: Vec<Objective>,
    rca_state: String,
    evidence: Vec<Evidence>,
    related: Vec<Related>,
}
impl Incidents {
    pub(super) fn clear_evidence(&mut self) {
        self.rows.clear();
        self.detail = None;
        self.count = None;
        self.title.clear();
        self.page_label.clear();
        self.page_ix = 0;
        self.related_page_ix = 0;
        self.related_page_label.clear();
    }
    fn prepare_list(&mut self, values: &[api::Incident]) -> bool {
        let previous = self.selected.clone();
        if !values
            .iter()
            .any(|i| self.selected.as_ref() == Some(&(i.key.clone(), i.app.clone())))
        {
            self.selected = values.first().map(|i| (i.key.clone(), i.app.clone()));
        }
        self.rows = values
            .iter()
            .map(|i| Row {
                key: i.key.clone(),
                app: i.app.clone(),
                title: format!("{} · {}", i.app.short(), i.description),
                summary: format!(
                    "{} · {} · {} affected",
                    state(i.state),
                    format::duration(i.duration),
                    format::percent(Some(i.impact_percent))
                ),
                severity: i.severity.into(),
            })
            .collect();
        let open = values
            .iter()
            .filter(|i| i.state == api::IncidentState::Open)
            .count();
        // '+' denotes a sample, not a complete project count. Even a short sample
        // can mean the server has no world yet, rather than no incident history.
        self.count = (open > 0).then(|| format!("{open} sampled"));
        self.title = format!("Incidents · {open} open in sample");
        self.page_ix = self
            .page_ix
            .min(self.rows.len().saturating_sub(1) / PAGE_SIZE);
        self.prepare_page();
        previous != self.selected
    }
    fn prepare_page(&mut self) {
        self.page_label = format!(
            "{} / {}",
            self.page_ix + 1,
            self.rows.len().div_ceil(PAGE_SIZE).max(1)
        );
    }
    fn prepare_related(&mut self) {
        let count = self.detail.as_ref().map_or(0, |d| d.related.len());
        self.related_page_ix = self
            .related_page_ix
            .min(count.saturating_sub(1) / PAGE_SIZE);
        self.related_page_label = format!(
            "{} / {}",
            self.related_page_ix + 1,
            count.div_ceil(PAGE_SIZE).max(1)
        );
    }
}
fn state(value: api::IncidentState) -> &'static str {
    match value {
        api::IncidentState::Open => "Open",
        api::IncidentState::Resolved => "Resolved",
    }
}
fn objective(
    name: &'static str,
    value: Option<&api::SloObjective>,
    impact: Option<f64>,
    rates: &[api::BurnRate],
    severity: Status,
) -> Objective {
    Objective {
        name,
        objective: value
            .map_or("Objective not reported", |v| v.objective())
            .into(),
        compliance: value.map_or("Not reported", |v| v.compliance()).into(),
        severity: value.map_or(Status::Unknown, |v| {
            if v.is_violated() {
                severity
            } else {
                Status::Ok
            }
        }),
        impact: format!("Affected requests: {}", format::percent(impact)),
        rates: rates
            .iter()
            .map(|r| {
                format!(
                    "{} / {}: {} / {} · threshold {} · {}",
                    format::duration(r.long_window),
                    format::duration(r.short_window),
                    format::multiplier(r.long_window_burn_rate),
                    format::multiplier(r.short_window_burn_rate),
                    format::multiplier(r.threshold),
                    Status::from(r.severity).label()
                )
            })
            .collect(),
    }
}
fn detail(value: &api::IncidentView) -> Detail {
    let i = value.incident();
    let started = i.opened_at.map_or("Not reported".into(), |t| {
        t.format("%d %b %H:%M UTC").to_string()
    });
    let ended = i
        .resolved_at
        .map(|t| format!(" · Ended {}", t.format("%d %b %H:%M UTC")))
        .unwrap_or_default();
    let slo = i.slo.as_ref();
    let objectives = vec![
        objective(
            "Availability",
            value.availability(),
            slo.and_then(|s| s.availability_impact_percent),
            slo.map_or(&[], |s| s.availability_burn_rates.as_slice()),
            i.severity.into(),
        ),
        objective(
            "Latency",
            value.latency(),
            slo.and_then(|s| s.latency_impact_percent),
            slo.map_or(&[], |s| s.latency_burn_rates.as_slice()),
            i.severity.into(),
        ),
    ];
    let mut evidence = vec![];
    let mut related = vec![];
    let rca_state = if let Some(rca) = &i.rca {
        for (title, value) in [
            ("Summary", &rca.summary),
            ("Root cause", &rca.root_cause),
            ("Suggested fixes", &rca.immediate_fixes),
            ("Detailed analysis", &rca.detailed_analysis),
            ("Analysis error", &rca.error),
        ] {
            if !value.is_empty() {
                evidence.push(Evidence {
                    title,
                    text: value.clone().into(),
                });
            }
        }
        related = rca
            .propagation
            .iter()
            .map(|p| Related {
                app: p.app_id.clone(),
                label: p.app_id.short(),
                severity: p.status.into(),
                issues: p.issues.join("\n").into(),
            })
            .collect();
        if rca.status.is_empty() {
            "Analysis state not reported".into()
        } else {
            format!("Analysis state: {}", rca.status)
        }
    } else {
        "Root cause analysis not reported".into()
    };
    Detail {
        key: i.key.clone(),
        app: i.app.clone(),
        title: i.description.clone(),
        state: state(i.state),
        severity: i.severity.into(),
        summary: format!(
            "Started {started}{ended} · {} · {} affected · {}",
            format::duration(i.duration),
            format::percent(Some(i.impact_percent)),
            i.cluster
        ),
        objectives,
        rca_state,
        evidence,
        related,
    }
}

impl ObservabilityPage {
    pub(crate) fn incident_count(&self) -> Option<&str> {
        if self.fixture {
            Some("2")
        } else {
            self.incident_observations.count.as_deref()
        }
    }
    pub(super) fn read_incidents(
        &mut self,
        provider: api::Provider,
        source: api::Source,
        range: api::TimeRange,
        cx: &mut Context<Self>,
    ) {
        let identity = self
            .live
            .identity(connection::Subject::Incidents)
            .expect("selected source");
        let request = self.live.incidents.begin(identity);
        self.read_incident_detail(cx);
        self.spawn_read(
            async move { provider.incidents(&source, range).await },
            move |this, result, cx| {
                let succeeded = result.is_ok();
                if this
                    .live
                    .incidents
                    .apply(&request, result.map_err(|e| e.to_string()))
                {
                    if succeeded {
                        let clusters = this
                            .live
                            .incidents
                            .data()
                            .expect("successful list")
                            .iter()
                            .map(|i| &i.app)
                            .filter(|id| !id.is_external())
                            .map(|id| id.cluster_id().to_string());
                        this.cluster_ids = this
                            .cluster_ids
                            .iter()
                            .cloned()
                            .chain(clusters)
                            .filter(|id| !id.is_empty())
                            .collect::<std::collections::BTreeSet<_>>()
                            .into_iter()
                            .collect();
                        let changed = this
                            .incident_observations
                            .prepare_list(this.live.incidents.data().expect("successful list"));
                        if changed {
                            this.live.incident_job = None;
                            this.live.incident = Default::default();
                            this.incident_observations.detail = None;
                            this.read_incident_detail(cx);
                        }
                        cx.emit(ObservabilityEvent::Navigation);
                    }
                    cx.notify();
                }
            },
            cx,
        );
    }
    fn read_incident_detail(&mut self, cx: &mut Context<Self>) {
        let Some((key, app)) = self.incident_observations.selected.clone() else {
            return;
        };
        let (Some(provider), Some(source)) = (self.live.provider.clone(), self.live.source.clone())
        else {
            return;
        };
        let identity = self
            .live
            .identity(connection::Subject::Incident(key.clone(), app.clone()))
            .expect("selected source");
        let request = self.live.incident.begin(identity);
        let range = self.live.range;
        self.live.incident_job = None;
        let job = self.spawn_owned_read(
            async move { provider.incident(&source, range, &key, &app).await },
            move |this, result, cx| {
                if this
                    .live
                    .incident
                    .apply(&request, result.map_err(|e| e.to_string()))
                {
                    this.incident_observations.detail = this.live.incident.data().map(detail);
                    this.incident_observations.prepare_related();
                    cx.notify();
                }
            },
            cx,
        );
        self.live.incident_job = Some(job);
    }
    fn select_incident(&mut self, key: String, app: api::AppId, cx: &mut Context<Self>) {
        if !self.live.visible
            || self.incident_observations.selected.as_ref() == Some(&(key.clone(), app.clone()))
        {
            return;
        }
        if !self
            .incident_observations
            .rows
            .iter()
            .any(|r| r.key == key && r.app == app)
        {
            return;
        }
        self.incident_observations.selected = Some((key, app));
        self.live.incident = Default::default();
        self.incident_observations.detail = None;
        self.incident_observations.related_page_ix = 0;
        // Replacement drops both read/delivery jobs. Retry the bounded list and
        // detail on the captured window, retaining the existing list as stale.
        self.refresh(cx);
    }
    fn incident_app(&mut self, key: &str, app: api::AppId, cx: &mut Context<Self>) {
        if !self.live.visible || self.live.incident.is_stale() {
            return;
        }
        let Some(detail) = &self.incident_observations.detail else {
            return;
        };
        if detail.key != key
            || self.incident_observations.selected.as_ref()
                != Some(&(detail.key.clone(), detail.app.clone()))
        {
            return;
        }
        if detail.app != app && !detail.related.iter().any(|p| p.app == app) {
            return;
        }
        self.open_app(app, Report::Errors, cx);
    }
    fn live_incident_list(&self, stacked: bool, cx: &Context<Self>) -> Div {
        let state = &self.incident_observations;
        card(state.title.clone(), cx)
            .when_else(stacked, |b| b.w_full(), |b| b.w(dp(260.)).flex_none())
            .children(
                state
                    .rows
                    .iter()
                    .skip(state.page_ix * PAGE_SIZE)
                    .take(PAGE_SIZE)
                    .map(|row| {
                        let (key, app) = (row.key.clone(), row.app.clone());
                        Button::new(SharedString::from(format!("obs-live-incident-{}", row.key)))
                            .ghost()
                            .group("fog-control")
                            .selected(state.selected.as_ref() == Some(&(key.clone(), app.clone())))
                            .w_full()
                            .h(dp(104.))
                            .justify_start()
                            .px(dp(14.))
                            .child(status(row.severity, cx))
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .flex_1()
                                    .items_start()
                                    .text_left()
                                    .gap(dp(4.))
                                    .child(mono(row.key.clone()).truncate())
                                    .child(
                                        text(row.title.clone()).whitespace_normal().line_clamp(2),
                                    )
                                    .child(muted(row.summary.clone(), cx).whitespace_normal()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_incident(key.clone(), app.clone(), cx)
                            }))
                    }),
            )
            .child(
                body().child(
                    line()
                        .child(
                            action("obs-incidents-previous", "Previous")
                                .disabled(state.page_ix == 0)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.incident_observations.page_ix =
                                        this.incident_observations.page_ix.saturating_sub(1);
                                    this.incident_observations.prepare_page();
                                    cx.notify();
                                })),
                        )
                        .child(muted(state.page_label.clone(), cx))
                        .child(
                            action("obs-incidents-next", "Next")
                                .disabled((state.page_ix + 1) * PAGE_SIZE >= state.rows.len())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let state = &mut this.incident_observations;
                                    state.page_ix = (state.page_ix + 1)
                                        .min(state.rows.len().saturating_sub(1) / PAGE_SIZE);
                                    state.prepare_page();
                                    cx.notify();
                                })),
                        ),
                ),
            )
    }
    fn live_incident_slo(&self, d: &Detail, cx: &Context<Self>) -> Div {
        card("Service level objectives", cx).children(d.objectives.iter().map(|o| {
            body()
                .child(
                    line()
                        .flex_wrap()
                        .child(status(o.severity, cx))
                        .child(text(o.name).font_weight(ui::HEADING_WEIGHT)),
                )
                .child(text(o.objective.clone()).whitespace_normal())
                .child(pair("Compliance", o.compliance.clone(), cx))
                .child(muted(o.impact.clone(), cx))
                .when(o.rates.is_empty(), |b| {
                    b.child(muted("Burn rates not reported", cx))
                })
                .children(o.rates.iter().map(|r| mono(r.clone()).whitespace_normal()))
        }))
    }
    fn live_incident_rca(&self, d: &Detail, cx: &Context<Self>) -> Div {
        let key = d.key.clone();
        let stale = self.live.incident.is_stale();
        card("Coroot analysis", cx).child(
            body()
                .child(muted(d.rca_state.clone(), cx))
                .when(d.evidence.is_empty(), |b| {
                    b.child(text("No analysis text was reported."))
                })
                .children(d.evidence.iter().map(|e| {
                    v_flex()
                        .gap(dp(6.))
                        .child(text(e.title).font_weight(ui::HEADING_WEIGHT))
                        .child(text(e.text.clone()).whitespace_normal())
                }))
                .child(muted(
                    "Analysis is reported by Coroot. Suggested fixes are read-only source text.",
                    cx,
                ))
                .when(!d.related.is_empty(), |b| {
                    b.child(
                        text("Applications with reported propagation problems")
                            .font_weight(ui::HEADING_WEIGHT),
                    )
                })
                .children(
                    d.related
                        .iter()
                        .skip(self.incident_observations.related_page_ix * PAGE_SIZE)
                        .take(PAGE_SIZE)
                        .map(|r| {
                            let (app, key) = (r.app.clone(), key.clone());
                            v_flex()
                                .gap(dp(6.))
                                .child(
                                    line().child(status(r.severity, cx)).child(
                                        action(
                                            SharedString::from(format!(
                                                "obs-incident-app-{}",
                                                r.app.as_str()
                                            )),
                                            r.label.clone(),
                                        )
                                        .disabled(stale)
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.incident_app(&key, app.clone(), cx)
                                            }),
                                        ),
                                    ),
                                )
                                .child(muted(r.issues.clone(), cx).whitespace_normal())
                        }),
                )
                .when(d.related.len() > PAGE_SIZE, |b| {
                    b.child(
                        line()
                            .child(
                                action("obs-propagation-previous", "Previous applications")
                                    .disabled(self.incident_observations.related_page_ix == 0)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.incident_observations.related_page_ix = this
                                            .incident_observations
                                            .related_page_ix
                                            .saturating_sub(1);
                                        this.incident_observations.prepare_related();
                                        cx.notify();
                                    })),
                            )
                            .child(muted(
                                self.incident_observations.related_page_label.clone(),
                                cx,
                            ))
                            .child(
                                action("obs-propagation-next", "Next applications")
                                    .disabled(
                                        (self.incident_observations.related_page_ix + 1)
                                            * PAGE_SIZE
                                            >= d.related.len(),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.incident_observations.related_page_ix += 1;
                                        this.incident_observations.prepare_related();
                                        cx.notify();
                                    })),
                            ),
                    )
                })
                .child(muted(
                    "Charts and ruled-out evidence are not available in this incident view.",
                    cx,
                )),
        )
    }
    pub(super) fn render_live_incidents(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let state = &self.incident_observations;
        if state.rows.is_empty() {
            return card("Incidents",cx).id("obs-incidents-empty").test_support().child(body()
                .child(text("No incidents were returned in the latest project sample."))
                .child(muted("Coroot can also return no data before its project world is ready. Refresh to check again.",cx))).into_any_element();
        }
        let stacked = crate::screens::content_width(window) < 880.;
        let mut content = v_flex()
            .id("obs-incident-detail")
            .test_support()
            .flex_1()
            .min_w_0()
            .gap(dp(12.))
            .when(stacked, |b| b.w_full())
            .when(self.live.incident.is_loading(), |b| {
                b.child(muted("Reading incident…", cx))
            })
            .when(self.live.incident.is_stale(), |b| {
                b.child(text("Last known incident · stale"))
            })
            .when_some(self.live.incident.error(), |b, e| {
                b.child(text(e.to_string()).text_color(palette(cx).crit_ink))
                    .child(
                        action("obs-incident-retry", "Retry incident")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )
            });
        if let Some(d) = &state.detail {
            let (key, app) = (d.key.clone(), d.app.clone());
            content = content
                .child(
                    line()
                        .flex_wrap()
                        .child(ui::page_title(d.title.clone()))
                        .child(status(d.severity, cx))
                        .child(text(d.state)),
                )
                .child(muted(d.summary.clone(), cx).whitespace_normal())
                .child(
                    action("obs-incident-primary-app", d.app.short())
                        .disabled(self.live.incident.is_stale())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.incident_app(&key, app.clone(), cx)
                        })),
                )
                .child(self.live_incident_slo(d, cx))
                .child(self.live_incident_rca(d, cx));
        } else if !self.live.incident.is_loading() && self.live.incident.error().is_none() {
            content = content.child(text("Select an incident to read its details."));
        }
        v_flex().gap(dp(12.))
            .child(muted("Latest project incidents · up to 100 · all states. The list is not filtered by the toolbar window; details use the incident's time context.",cx).whitespace_normal())
            .child(div().flex().gap(dp(12.)).items_start().when_else(stacked,|b|b.flex_col(),|b|b.flex_row())
                .child(self.live_incident_list(stacked,cx)).child(content)).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Incidents, detail};
    use freshkube_core::coroot::{Incident, IncidentView};
    fn incident(key: &str) -> Incident {
        serde_json::from_value(serde_json::json!({"key":key,"app":"c:ns:Deployment:api","cluster":"Production","severity":"warning","state":"open","opened_at":null,"duration_seconds":120,"impact_percent":0,"description":"SLO violation"})).unwrap()
    }
    #[test]
    fn selection_survives_reorder_and_clears_removed_identity() {
        let mut state = Incidents::default();
        assert!(state.prepare_list(&[incident("a"), incident("b")]));
        assert!(!state.prepare_list(&[incident("b"), incident("a")]));
        assert_eq!(state.selected.as_ref().unwrap().0, "a");
        assert!(state.prepare_list(&[incident("b")]));
        assert_eq!(state.selected.as_ref().unwrap().0, "b");
        assert!(state.prepare_list(&[]));
        assert!(state.selected.is_none());
    }
    #[test]
    fn missing_evidence_stays_unknown_and_reported_zero_is_preserved() {
        let view: IncidentView =
            serde_json::from_value(serde_json::json!({"incident":incident("a")})).unwrap();
        let projected = detail(&view);
        assert_eq!(projected.objectives[0].compliance, "Not reported");
        assert_eq!(projected.objectives[0].severity, super::Status::Unknown);
        assert_eq!(
            projected.objectives[0].impact,
            "Affected requests: Not reported"
        );
        assert!(projected.objectives[0].rates.is_empty());
        assert!(projected.evidence.is_empty());
        assert!(projected.related.is_empty());
        assert!(projected.summary.contains("0.0% affected"));
    }
}

#[cfg(test)]
mod ui_tests {
    use super::{PAGE_SIZE, detail};
    use crate::observability::{Destination, Status, connection::Subject, tests::mount_size};
    use freshkube_core::coroot as api;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, TestAppContext};
    #[gpui_kit::test]
    fn bounded_incidents_prepare_and_render_at_large_text(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = mount_size(cx, false, 760., 560.);
        let rows=(0..100).map(|ix|serde_json::from_value::<api::Incident>(serde_json::json!({"key":format!("k{ix}"),"app":"c:ns:Deployment:api","cluster":"Production","severity":"warning","state":"open","opened_at":null,"duration_seconds":120,"impact_percent":0,"description":"Source incident"})).unwrap()).collect::<Vec<_>>();
        let related=(0..100).map(|ix|serde_json::json!({"app_id":format!("c:ns:Deployment:related-{ix}"),"status":"warning","issues":["Source observation"]})).collect::<Vec<_>>();
        let mut i = serde_json::to_value(&rows[0]).unwrap();
        i["rca"] = serde_json::json!({"status":"OK","root_cause":"Source root cause ".repeat(512),"propagation":related});
        let view:api::IncidentView=serde_json::from_value(serde_json::json!({"incident":i,"availability":{"objective":"99% of requests should not fail","compliance":"97.2%","violated":true}})).unwrap();
        cx.update(|cx| {
            crate::text_size::set(20., cx);
            page.update(cx, |page, _| {
                let provider =
                    api::Provider::new("http://127.0.0.1:1", api::Credentials::None).unwrap();
                page.live.source = Some(provider.source(&api::ProjectInfo {
                    id: "p1".into(),
                    name: "Production".into(),
                }));
                page.live.provider = Some(provider);
                page.destination = Destination::Incidents;
                let start = std::time::Instant::now();
                page.incident_observations.prepare_list(&rows);
                page.incident_observations.detail = Some(detail(&view));
                page.incident_observations.prepare_related();
                eprintln!(
                    "Incidents: 100 rows/100 propagation preparation {:?}",
                    start.elapsed()
                );
                let request = page
                    .live
                    .incidents
                    .begin(page.live.identity(Subject::Incidents).unwrap());
                page.live.incidents.apply(&request, Ok(rows));
                let request = page.live.incident.begin(
                    page.live
                        .identity(Subject::Incident(
                            "k0".into(),
                            api::AppId::new("c:ns:Deployment:api"),
                        ))
                        .unwrap(),
                );
                page.live.incident.apply(&request, Ok(view));
                assert_eq!(
                    page.incident_observations
                        .detail
                        .as_ref()
                        .unwrap()
                        .objectives[0]
                        .severity,
                    Status::Warning
                );
            });
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("obs-live-incident-k0").is_some());
            assert!(
                window
                    .try_find(gpui_kit::SharedString::from(format!(
                        "obs-live-incident-k{PAGE_SIZE}"
                    )))
                    .is_none()
            );
            assert!(
                window
                    .try_find("obs-incident-app-c:ns:Deployment:related-10")
                    .is_none()
            );
            let bounds = window.find("obs-incident-detail").bounds();
            assert!(bounds.right() <= window.viewport_size().width);
            let start = std::time::Instant::now();
            for _ in 0..10 {
                window.render_frame(cx);
            }
            eprintln!(
                "Incidents: ten bounded headless frames {:?}",
                start.elapsed()
            );
        })
        .unwrap();
    }
}
