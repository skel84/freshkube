//! An incident's detail: its objectives, Coroot's analysis and the
//! applications it reached, prepared when the incident view arrives.
use super::*;

pub(super) struct Objective {
    pub(super) name: &'static str,
    pub(super) objective: String,
    pub(super) compliance: String,
    pub(super) severity: Status,
    pub(super) state: &'static str,
    pub(super) impact: String,
    pub(super) rates: Vec<String>,
}
pub(super) struct Evidence {
    pub(super) title: &'static str,
    pub(super) text: SharedString,
}
pub(super) struct Related {
    pub(super) app: api::AppId,
    pub(super) label: String,
    pub(super) severity: Status,
    pub(super) issues: SharedString,
}
pub(super) struct Detail {
    pub(super) key: String,
    pub(super) app: api::AppId,
    pub(super) title: String,
    pub(super) state: &'static str,
    pub(super) severity: Status,
    pub(super) summary: String,
    pub(super) objectives: Vec<Objective>,
    pub(super) rca_state: String,
    pub(super) evidence: Vec<Evidence>,
    pub(super) related: Vec<Related>,
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
        state: value.map_or("Not reported", |v| {
            if v.is_violated() {
                "Violated"
            } else {
                "Within objective"
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
pub(super) fn detail(value: &api::IncidentView) -> Detail {
    let i = value.incident();
    let started = i
        .opened_at
        .map_or("Not reported".into(), format::local_time);
    let ended = i
        .resolved_at
        .map(|t| format!(" · Ended {}", format::local_time(t)))
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

/// One part of the pane, under a hairline.
fn section(title: &str, cx: &App) -> Div {
    v_flex()
        .min_w_0()
        .border_t_1()
        .border_color(palette(cx).line)
        .child(
            h_flex()
                .px(dp(14.))
                .pt(dp(12.))
                .child(ui::caption(title, cx)),
        )
}

impl ObservabilityPage {
    /// The selected incident in a detail pane: its heading, then Coroot's
    /// objectives and analysis. Nothing when the list is empty.
    pub(super) fn render_incident_detail(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = &self.incident_observations;
        if state.rows.is_empty() {
            return None;
        }
        let read = &self.live.incident;
        let read_tag = if read.is_loading() {
            Some((Tone::Unknown, "Reading"))
        } else if read.is_stale() {
            Some((Tone::Warn, "Stale"))
        } else if read.error().is_some() {
            Some((Tone::Crit, "Failed"))
        } else {
            None
        };
        let pane = freshkube_ui::page::card(cx)
            .id("obs-incident-detail")
            .test_support()
            .when_some(read.error(), |pane, e| {
                pane.child(
                    body()
                        .child(text(e.to_string()).text_color(palette(cx).crit_ink))
                        .child(
                            action("obs-incident-retry", "Retry incident")
                                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                        ),
                )
            });
        let Some(d) = &state.detail else {
            let note = if read.is_loading() || read.error().is_some() {
                None
            } else {
                Some("Select an incident to read its details.")
            };
            return Some(
                pane.child(self.incident_heading(None, read_tag, cx))
                    .children(note.map(|note| body().child(muted(note, cx))))
                    .into_any_element(),
            );
        };
        let (key, app) = (d.key.clone(), d.app.clone());
        Some(
            pane.child(self.incident_heading(Some(d), read_tag, cx))
                .child(
                    body()
                        .pt_0()
                        .child(text(d.title.clone()).whitespace_normal())
                        .child(muted(d.summary.clone(), cx).whitespace_normal())
                        .child(
                            line().child(
                                Button::new("obs-incident-primary-app")
                                    .outline()
                                    .xsmall()
                                    .label(d.app.short())
                                    .tooltip("Open this application's errors report")
                                    .disabled(read.is_stale())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.incident_app(&key, app.clone(), cx)
                                    })),
                            ),
                        ),
                )
                .child(self.live_incident_slo(d, cx))
                .child(self.live_incident_rca(d, cx))
                .into_any_element(),
        )
    }
    fn incident_heading(
        &self,
        d: Option<&Detail>,
        read: Option<(Tone, &'static str)>,
        cx: &Context<Self>,
    ) -> Div {
        let title = d.map_or_else(
            || {
                self.incident_observations
                    .selected()
                    .map_or_else(String::new, |(key, _)| key.clone())
            },
            |d| d.key.clone(),
        );
        h_flex()
            .items_start()
            .gap(dp(8.))
            .px(dp(14.))
            .pt(dp(12.))
            .pb(dp(8.))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(dp(2.))
                    .child(ui::caption("Incident", cx))
                    .child(
                        div()
                            .id("obs-incident-title")
                            .test_support()
                            .font_family(MONO_FONT)
                            .text_size(dp(13.5))
                            .truncate()
                            .child(title),
                    ),
            )
            .children(d.map(|d| {
                let tone = if d.state == "Open" {
                    d.severity.tone()
                } else {
                    Tone::Good
                };
                div()
                    .id("obs-incident-state")
                    .test_support()
                    .mt(dp(14.))
                    .child(ui::tag(tone, None, d.state, cx))
            }))
            .children(read.map(|(tone, text)| {
                div()
                    .id("obs-incident-read")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(text)
                    .mt(dp(14.))
                    .child(ui::tag(tone, None, text, cx))
            }))
    }
    pub(super) fn live_incident_slo(&self, d: &Detail, cx: &Context<Self>) -> Div {
        section("Service level objectives", cx).children(d.objectives.iter().map(|o| {
            body()
                .child(
                    line()
                        .flex_wrap()
                        .child(status(o.severity, cx))
                        .child(text(o.name).font_weight(ui::HEADING_WEIGHT))
                        .child(muted(o.state, cx)),
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
    pub(super) fn live_incident_rca(&self, d: &Detail, cx: &Context<Self>) -> Div {
        let key = d.key.clone();
        let stale = self.live.incident.is_stale();
        section("Coroot analysis", cx).child(
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
}
