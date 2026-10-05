//! Bounded incident observations. The page owns identity, selection and prepared
//! strings; Coroot owns SLO/RCA interpretation. Render never invents evidence.
use super::*;
use freshkube_core::coroot as api;
mod detail;
mod table;
#[cfg(test)]
mod tests;
use super::tables::{ColumnKind, PageColumn, TableKey};
use detail::{Detail, detail};
pub(super) use table::IncidentCells;
const PAGE_SIZE: usize = 10;
/// Columns hidden until Columns shows them; the detail's summary has both.
pub(super) const HIDDEN_BY_DEFAULT: [ColumnKind; 2] = [ColumnKind::Duration, ColumnKind::Impact];

#[derive(Default)]
pub(super) struct Incidents {
    /// By key and application, never by position.
    selected: Option<TableKey>,
    rows: Vec<Row>,
    /// Indexes into `rows` the chips and the filter leave, in Coroot's order.
    shown: Vec<usize>,
    /// The chip chosen: open or resolved incidents only.
    state_filter: Option<api::IncidentState>,
    query: String,
    /// Open and resolved incidents in the sample.
    counts: [usize; 2],
    columns: Vec<PageColumn>,
    width: f32,
    detail: Option<Detail>,
    count: Option<String>,
    sample: String,
    related_page_ix: usize,
    related_page_label: String,
}
struct Row {
    key: TableKey,
    id: SharedString,
    incident: SharedString,
    app: api::AppId,
    app_label: SharedString,
    title: SharedString,
    /// The whole row in words, for its tooltip and accessibility label.
    label: SharedString,
    severity: Status,
    state: api::IncidentState,
    opened: SharedString,
    duration: SharedString,
    impact: SharedString,
    /// Lowercase key, title and application, for the filter.
    search: String,
}
impl Incidents {
    /// Nothing from the last connection, but the viewer's filters, which
    /// the filter field still shows.
    pub(super) fn cleared(&mut self) -> Self {
        Self {
            state_filter: self.state_filter,
            query: std::mem::take(&mut self.query),
            ..Default::default()
        }
    }
    pub(super) fn clear_evidence(&mut self) {
        self.rows.clear();
        self.shown.clear();
        self.counts = [0; 2];
        self.detail = None;
        self.count = None;
        self.sample.clear();
        self.related_page_ix = 0;
        self.related_page_label.clear();
    }
    fn prepare_list(&mut self, values: &[api::Incident]) -> bool {
        let previous = self.selected.clone();
        if !values
            .iter()
            .any(|i| self.selected.as_ref() == Some(&key(i)))
        {
            self.selected = values.first().map(key);
        }
        self.rows = values
            .iter()
            .map(|i| {
                let title = SharedString::from(i.description.clone());
                let app_label = SharedString::from(i.app.short());
                let duration = SharedString::from(format::duration(i.duration));
                let impact = SharedString::from(format::percent(Some(i.impact_percent)));
                Row {
                    key: key(i),
                    id: format!("obs-live-incident-{}", i.key).into(),
                    incident: i.key.clone().into(),
                    label: format!(
                        "{} · {app_label} · {title} · {} · {duration} · {impact} affected",
                        i.key,
                        state(i.state)
                    )
                    .into(),
                    search: format!("{} {} {}", i.key, i.description, app_label).to_lowercase(),
                    app: i.app.clone(),
                    app_label,
                    title,
                    severity: i.severity.into(),
                    state: i.state,
                    opened: i
                        .opened_at
                        .map_or_else(|| "—".to_owned(), format::local_time)
                        .into(),
                    duration,
                    impact,
                }
            })
            .collect();
        let open = values
            .iter()
            .filter(|i| i.state == api::IncidentState::Open)
            .count();
        self.counts = [open, values.len() - open];
        // '+' denotes a sample, not a complete project count. Even a short sample
        // can mean the server has no world yet, rather than no incident history.
        self.count = (open > 0).then(|| format!("{open} sampled"));
        self.sample = match values.len() {
            1 => "1 incident in the latest sample".into(),
            n => format!("{n} incidents in the latest sample"),
        };
        self.project();
        previous != self.selected
    }
    /// The rows the chips and the filter leave.
    fn project(&mut self) {
        self.shown = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| self.state_filter.is_none_or(|state| row.state == state))
            .filter(|(_, row)| row.search.contains(&self.query))
            .map(|(ix, _)| ix)
            .collect();
    }
    fn selected(&self) -> Option<(&String, &api::AppId)> {
        match &self.selected {
            Some(TableKey::Incident(key, app)) => Some((key, app)),
            _ => None,
        }
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
fn key(incident: &api::Incident) -> TableKey {
    TableKey::Incident(incident.key.clone(), incident.app.clone())
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
        self.incident_observations
            .prepare_list(&example::incidents(to));
        self.prepare_incident_columns();
        let state = &mut self.incident_observations;
        let view = state
            .selected()
            .and_then(|(key, _)| example::incident_view(key, to));
        state.detail = view.as_ref().map(detail);
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
                        this.prepare_incident_columns();
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
        let Some((key, app)) = self
            .incident_observations
            .selected()
            .map(|(key, app)| (key.clone(), app.clone()))
        else {
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
    pub(super) fn select_incident(&mut self, key: String, app: api::AppId, cx: &mut Context<Self>) {
        if !self.live.visible || self.incident_observations.selected() == Some((&key, &app)) {
            return;
        }
        if !self
            .incident_observations
            .rows
            .iter()
            .any(|r| r.incident == key && r.app == app)
        {
            return;
        }
        self.incident_observations.selected = Some(TableKey::Incident(key, app));
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
            || self.incident_observations.selected() != Some((&detail.key, &detail.app))
        {
            return;
        }
        if detail.app != app && !detail.related.iter().any(|p| p.app == app) {
            return;
        }
        self.open_app(app, Report::Errors, cx);
    }
    /// The table, with the selected incident beside it on a wide page and
    /// below it on a narrow one.
    pub(super) fn render_incidents(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let beside = crate::screens::beside(window);
        let table = self.render_incident_table(beside, window, cx);
        let detail = self.render_incident_detail(cx);
        crate::screens::split("obs-incidents-split", beside, table, detail)
    }
}
