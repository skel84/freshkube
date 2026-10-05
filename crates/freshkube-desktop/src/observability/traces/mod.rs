//! Live traces for one application: Coroot's latency and error heatmap, the
//! spans of the whole window or one selected cell, and one trace as a
//! waterfall. Display data is prepared when an answer arrives; render reads it.
use super::*;
use freshkube_core::coroot as api;
mod heat;
mod table;
#[cfg(test)]
mod tests;
mod waterfall;
use super::tables::{PageColumn, TableKey};
pub(super) use heat::bind_keys as bind_heatmap_keys;
use heat::{Heat, heat};
pub(super) use table::SpanCells;
use waterfall::{Waterfall, waterfall};

#[derive(Default)]
pub(super) struct Traces {
    /// `otel`, `agent`, or empty for Coroot's choice.
    pub(super) source: String,
    pub(super) selection: api::TraceSelection,
    /// The selected heatmap cell, in display rows and columns.
    cell: Option<(usize, usize)>,
    /// The heatmap's keyboard cursor, which moves without reading.
    cursor: Option<(usize, usize)>,
    /// The trace the waterfall shows.
    pub(super) trace: Option<String>,
    /// The listed span chosen, by trace and span id; its trace is `trace`.
    selected: Option<TableKey>,
    app: Option<api::AppId>,
    span: usize,
    heat: Option<Heat>,
    sources: Vec<(String, String, bool)>,
    /// The source Coroot chose before the user picked one.
    default_source: Option<String>,
    note: String,
    rows: Vec<SpanRow>,
    /// Indexes into `rows` the filter leaves, in Coroot's order.
    shown: Vec<usize>,
    /// Lowercase filter text.
    query: String,
    count: String,
    /// Coroot stopped at its limit, so more spans match.
    limited: bool,
    columns: Vec<PageColumn>,
    width: f32,
    waterfall: Option<Waterfall>,
}

struct SpanRow {
    key: TableKey,
    id: SharedString,
    trace_id: String,
    service: SharedString,
    name: SharedString,
    started: SharedString,
    duration: SharedString,
    error: bool,
    /// The whole row in words, with a failure's message, for its tooltip
    /// and accessibility label.
    label: SharedString,
    /// Lowercase name, service and trace id, for the filter.
    search: String,
}

impl Traces {
    /// A new application or window: start from its latest spans. The filter
    /// stays, since its field still shows it, and so does the heatmap's
    /// cursor, clamped when the new heatmap arrives.
    pub(super) fn reset(&mut self) {
        *self = Self {
            source: std::mem::take(&mut self.source),
            default_source: self.default_source.take(),
            app: self.app.take(),
            query: std::mem::take(&mut self.query),
            cursor: self.cursor.take(),
            ..Self::default()
        };
    }

    /// Another application: start again from its latest spans.
    fn reset_for(&mut self, app: &api::AppId) {
        if self.app.as_ref() != Some(app) {
            self.reset();
            self.app = Some(app.clone());
        }
    }

    /// Returns the trace to open when none is selected: the first failed
    /// span's, else the first span's.
    fn prepare_list(&mut self, tracing: &api::Tracing) -> Option<String> {
        self.note = tracing.message.clone();
        self.sources = tracing
            .sources
            .iter()
            .map(|s| (s.kind.clone(), s.name.clone(), s.selected))
            .collect();
        if self.source.is_empty() {
            self.default_source = self.sources.iter().find(|s| s.2).map(|s| s.0.clone());
        }
        self.heat = tracing.heatmap.as_ref().map(heat);
        self.clamp_cursor();
        self.rows = tracing.spans.iter().map(span_row).collect();
        self.limited = tracing.limited;
        self.count = match (self.rows.len(), tracing.limited) {
            (0, _) => "No spans match".into(),
            (n, true) => format!("Latest {n} spans · more match"),
            (1, false) => "1 span".into(),
            (n, false) => format!("{n} spans"),
        };
        self.project();
        if !self
            .selected
            .as_ref()
            .is_some_and(|key| self.rows.iter().any(|r| &r.key == key))
        {
            self.selected = None;
        }
        if let Some(id) = &self.trace
            && let Some(row) = self.rows.iter().find(|r| &r.trace_id == id)
        {
            self.selected.get_or_insert_with(|| row.key.clone());
            return None;
        }
        self.waterfall = None;
        let row = self.rows.iter().find(|r| r.error).or(self.rows.first());
        self.selected = row.map(|r| r.key.clone());
        self.trace = row.map(|r| r.trace_id.clone());
        self.trace.clone()
    }

    /// The rows the filter leaves.
    fn project(&mut self) {
        self.shown = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.search.contains(&self.query))
            .map(|(ix, _)| ix)
            .collect();
    }

    fn prepare_trace(&mut self, trace_id: &str, spans: &[api::Span]) {
        self.waterfall = Some(waterfall(trace_id, spans));
        self.span = 0;
    }
}

fn span_row(span: &api::Span) -> SpanRow {
    let started = span.started_at().map_or_else(String::new, |t| {
        t.with_timezone(&chrono::Local)
            .format("%H:%M:%S")
            .to_string()
    });
    let duration = millis(span.duration);
    let failed = match (span.status.error, span.status.message.as_str()) {
        (false, _) => String::new(),
        (true, "") => " · failed".into(),
        (true, message) => format!(" · failed: {message}"),
    };
    SpanRow {
        key: TableKey::Span(span.trace_id.clone(), span.id.clone()),
        id: format!("obs-live-span-{}-{}", span.trace_id, span.id).into(),
        label: format!(
            "{} · {} · {started} · {duration}{failed}",
            span.name, span.service
        )
        .into(),
        search: format!("{} {} {}", span.name, span.service, span.trace_id).to_lowercase(),
        trace_id: span.trace_id.clone(),
        service: span.service.clone().into(),
        name: span.name.clone().into(),
        started: started.into(),
        duration: duration.into(),
        error: span.status.error,
    }
}

fn millis(ms: f64) -> String {
    match ms {
        ms if ms >= 1_000. => format!("{:.2} s", ms / 1_000.),
        ms if ms >= 10. => format!("{ms:.0} ms"),
        ms => format!("{ms:.1} ms"),
    }
}

impl ObservabilityPage {
    pub(super) fn read_traces(&mut self, cx: &mut Context<Self>) {
        let Some(app) = self.selected_app.clone() else {
            return;
        };
        self.live_traces.reset_for(&app);
        if self.fixture {
            self.answer_example_traces();
            return;
        }
        let (Some(provider), Some(source)) = (self.live.provider.clone(), self.live.source.clone())
        else {
            return;
        };
        let (trace_source, selection) = (
            self.live_traces.source.clone(),
            self.live_traces.selection.clone(),
        );
        let Some(identity) = self.live.identity(connection::Subject::Tracing(
            app.clone(),
            trace_source.clone(),
            selection.clone(),
        )) else {
            return;
        };
        let request = self.live.tracing.begin(identity);
        let range = self.live.range;
        self.live.traces_job = Some(self.spawn_owned_read(
            async move {
                provider
                    .tracing(&source, range, &app, &trace_source, &selection)
                    .await
            },
            move |this, result, cx| {
                if this
                    .live
                    .tracing
                    .apply(&request, result.map_err(|e| e.to_string()))
                {
                    let opened = this
                        .live
                        .tracing
                        .data()
                        .and_then(|tracing| this.live_traces.prepare_list(tracing));
                    this.prepare_trace_columns();
                    if opened.is_some() {
                        this.read_trace(cx);
                    }
                    cx.notify();
                }
            },
            cx,
        ));
        self.read_trace(cx);
    }

    /// Example mode answers the list and its trace at once, through the
    /// same preparation as Coroot's answers.
    fn answer_example_traces(&mut self) {
        let (Some(from), Some(to)) = (self.live.range.from, self.live.range.to) else {
            return;
        };
        let traces = &mut self.live_traces;
        let tracing = example::tracing(&traces.source, &traces.selection, from, to);
        traces.prepare_list(&tracing);
        self.prepare_trace_columns();
        let traces = &mut self.live_traces;
        if let Some(trace) = traces.trace.clone()
            && traces.waterfall.is_none()
        {
            traces.prepare_trace(&trace, &example::trace(&trace));
        }
    }

    fn read_trace(&mut self, cx: &mut Context<Self>) {
        if self.fixture {
            self.answer_example_traces();
            return;
        }
        let (Some(provider), Some(source), Some(app), Some(trace)) = (
            self.live.provider.clone(),
            self.live.source.clone(),
            self.selected_app.clone(),
            self.live_traces.trace.clone(),
        ) else {
            return;
        };
        let trace_source = self.live_traces.source.clone();
        let Some(identity) = self.live.identity(connection::Subject::Trace(
            app.clone(),
            trace_source.clone(),
            trace.clone(),
        )) else {
            return;
        };
        let request = self.live.trace.begin(identity);
        let range = self.live.range;
        let selection = api::TraceSelection::Trace(trace.clone());
        self.live.trace_job = Some(self.spawn_owned_read(
            async move {
                provider
                    .tracing(&source, range, &app, &trace_source, &selection)
                    .await
            },
            move |this, result, cx| {
                if this
                    .live
                    .trace
                    .apply(&request, result.map_err(|e| e.to_string()))
                {
                    if let Some(tracing) = this.live.trace.data() {
                        this.live_traces.prepare_trace(&trace, &tracing.spans);
                    }
                    cx.notify();
                }
            },
            cx,
        ));
    }

    fn show_failed_requests(&mut self, cx: &mut Context<Self>) {
        let (Some(from), Some(to)) = (self.live.range.from, self.live.range.to) else {
            return;
        };
        let traces = &mut self.live_traces;
        traces.cell = None;
        traces.selection = api::TraceSelection::Errors {
            from_ms: from.timestamp_millis(),
            to_ms: to.timestamp_millis(),
        };
        traces.trace = None;
        traces.waterfall = None;
        self.read_traces(cx);
        cx.notify();
    }

    fn show_all_requests(&mut self, cx: &mut Context<Self>) {
        let traces = &mut self.live_traces;
        traces.cell = None;
        traces.selection = api::TraceSelection::Recent;
        traces.trace = None;
        traces.waterfall = None;
        self.read_traces(cx);
        cx.notify();
    }

    /// Whether the user picked a source other than the one Coroot chose.
    fn trace_source_changed(&self) -> bool {
        let traces = &self.live_traces;
        !traces.source.is_empty() && traces.default_source.as_ref() != Some(&traces.source)
    }

    fn choose_trace_source(&mut self, kind: String, cx: &mut Context<Self>) {
        if self.live_traces.source == kind {
            return;
        }
        self.live_traces.source = kind;
        self.live_traces.reset();
        self.read_traces(cx);
        cx.notify();
    }

    /// A listed span: select it, and open its trace unless it shows.
    pub(super) fn open_span(&mut self, key: TableKey, cx: &mut Context<Self>) {
        let traces = &mut self.live_traces;
        let TableKey::Span(trace_id, _) = &key else {
            return;
        };
        if traces.selected.as_ref() == Some(&key) || !traces.rows.iter().any(|r| r.key == key) {
            return;
        }
        let trace_id = trace_id.clone();
        traces.selected = Some(key);
        if traces.trace.as_ref() != Some(&trace_id) {
            traces.trace = Some(trace_id);
            traces.waterfall = None;
            self.read_trace(cx);
        }
        cx.notify();
    }

    /// The application, Coroot's note and heatmap, then the requests with
    /// their trace beside them on a wide page and below on a narrow one.
    pub(super) fn render_live_traces(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let traces = &self.live_traces;
        let mut page = v_flex()
            .id("obs-live-traces")
            .test_support()
            .gap(dp(12.))
            .child(self.evidence_header("obs-trace-app", cx));
        if self.selected_app.is_none() {
            return page
                .child(muted(
                    "Choose an application to read its traces from Coroot.",
                    cx,
                ))
                .into_any_element();
        }
        // The heatmap stays drawn while Coroot answers, so it keeps the keyboard.
        let waiting = !self.fixture && self.live.tracing.data().is_none();
        if !waiting && !traces.note.is_empty() {
            page = page.child(muted(traces.note.clone(), cx).whitespace_normal());
        }
        page = page.child(self.live_heatmap(window, cx));
        if waiting {
            return page.into_any_element();
        }
        let beside = crate::screens::beside(window);
        let table = self.render_trace_table(beside, window, cx);
        let pane = self.live_waterfall(cx);
        page.child(crate::screens::split(
            "obs-traces-split",
            beside,
            table,
            Some(pane),
        ))
        .into_any_element()
    }
}
