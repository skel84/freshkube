//! Bounded incident observations. The page owns identity, selection and prepared
//! strings; Coroot owns SLO/RCA interpretation. Render never invents evidence.
use super::*;
use freshkube_core::coroot as api;
mod detail;
#[cfg(test)]
mod tests;
use detail::{Detail, detail};
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

impl ObservabilityPage {
    pub(crate) fn incident_count(&self) -> Option<&str> {
        self.incident_observations.count.as_deref()
    }
    /// Example mode answers the list and the selected incident at once,
    /// through the same preparation as Coroot's answers.
    pub(super) fn answer_example_incidents(&mut self) {
        let Some(to) = self.live.range.to else {
            return;
        };
        let state = &mut self.incident_observations;
        state.prepare_list(&example::incidents(to));
        state.detail = state
            .selected
            .as_ref()
            .and_then(|(key, _)| example::incident_view(key, to))
            .as_ref()
            .map(detail);
        state.prepare_related();
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
