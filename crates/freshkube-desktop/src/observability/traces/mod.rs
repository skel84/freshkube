//! Live traces for one application: Coroot's latency and error heatmap, the
//! spans of the whole window or one selected cell, and one trace as a
//! waterfall. Display data is prepared when an answer arrives; render reads it.
use super::*;
use freshkube_core::coroot as api;
mod heat;
mod waterfall;
use heat::{Heat, heat};
use waterfall::{Waterfall, waterfall};

#[derive(Default)]
pub(super) struct Traces {
    /// `otel`, `agent`, or empty for Coroot's choice.
    pub(super) source: String,
    pub(super) selection: api::TraceSelection,
    /// The selected heatmap cell, in display rows and columns.
    cell: Option<(usize, usize)>,
    pub(super) trace: Option<String>,
    app: Option<api::AppId>,
    span: usize,
    heat: Option<Heat>,
    sources: Vec<(String, String, bool)>,
    note: String,
    rows: Vec<SpanRow>,
    count: String,
    waterfall: Option<Waterfall>,
}

struct SpanRow {
    trace_id: String,
    service: String,
    name: String,
    when: String,
    duration: String,
    error: bool,
    message: String,
}

impl Traces {
    /// A new application or window: start from its latest spans.
    pub(super) fn reset(&mut self) {
        *self = Self {
            source: std::mem::take(&mut self.source),
            app: self.app.take(),
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
        self.heat = tracing.heatmap.as_ref().map(heat);
        self.rows = tracing
            .spans
            .iter()
            .map(|span| SpanRow {
                trace_id: span.trace_id.clone(),
                service: span.service.clone(),
                name: span.name.clone(),
                when: span
                    .started_at()
                    .map_or_else(String::new, |t| t.format("%H:%M:%S").to_string()),
                duration: millis(span.duration),
                error: span.status.error,
                message: span.status.message.clone(),
            })
            .collect();
        self.count = match (self.rows.len(), tracing.limited) {
            (0, _) => "No spans match".into(),
            (n, true) => format!("Latest {n} spans · more match, select a cell to narrow"),
            (1, false) => "1 span".into(),
            (n, false) => format!("{n} spans"),
        };
        if self
            .trace
            .as_ref()
            .is_some_and(|id| self.rows.iter().any(|r| &r.trace_id == id))
        {
            return None;
        }
        self.waterfall = None;
        self.trace = self
            .rows
            .iter()
            .find(|r| r.error)
            .or(self.rows.first())
            .map(|r| r.trace_id.clone());
        self.trace.clone()
    }

    fn prepare_trace(&mut self, trace_id: &str, spans: &[api::Span]) {
        self.waterfall = Some(waterfall(trace_id, spans));
        self.span = 0;
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

    fn choose_trace_source(&mut self, kind: String, cx: &mut Context<Self>) {
        if self.live_traces.source == kind {
            return;
        }
        self.live_traces.source = kind;
        self.live_traces.reset();
        self.read_traces(cx);
        cx.notify();
    }

    fn open_trace(&mut self, trace_id: String, cx: &mut Context<Self>) {
        if self.live_traces.trace.as_ref() == Some(&trace_id) {
            return;
        }
        self.live_traces.trace = Some(trace_id);
        self.live_traces.waterfall = None;
        self.read_trace(cx);
        cx.notify();
    }

    pub(super) fn render_live_traces(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
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
        if !self.fixture && self.live.tracing.data().is_none() {
            return page.into_any_element();
        }
        page = page.child(self.trace_controls(cx));
        if !traces.note.is_empty() {
            page = page.child(muted(traces.note.clone(), cx).whitespace_normal());
        }
        if let Some(heat) = &traces.heat {
            page = page.child(self.live_heatmap(heat, cx));
        }
        let stacked = crate::screens::content_width(window) < 900.;
        page.child(
            h_flex()
                .items_start()
                .flex_wrap()
                .gap(dp(12.))
                .child(
                    div()
                        .when_else(
                            stacked,
                            |this| this.w_full(),
                            |this| this.flex_1().min_w(dp(300.)),
                        )
                        .child(self.span_list(cx)),
                )
                .child(
                    div()
                        .when_else(
                            stacked,
                            |this| this.w_full(),
                            |this| this.flex_1().min_w(dp(420.)),
                        )
                        .child(self.live_waterfall(cx)),
                ),
        )
        .into_any_element()
    }

    fn trace_controls(&self, cx: &Context<Self>) -> Div {
        let traces = &self.live_traces;
        let errors =
            matches!(traces.selection, api::TraceSelection::Errors { .. }) && traces.cell.is_none();
        let all = traces.selection == api::TraceSelection::Recent;
        line()
            .flex_wrap()
            .children(traces.sources.iter().map(|(kind, name, selected)| {
                let kind = kind.clone();
                action(
                    SharedString::from(format!("obs-trace-source-{kind}")),
                    name.clone(),
                )
                .selected(*selected)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.choose_trace_source(kind.clone(), cx)),
                )
            }))
            .child(div().flex_1())
            .child(
                action("obs-trace-all", "All requests")
                    .selected(all)
                    .on_click(cx.listener(|this, _, _, cx| this.show_all_requests(cx))),
            )
            .child(
                action("obs-trace-failed", "Failed requests")
                    .selected(errors)
                    .on_click(cx.listener(|this, _, _, cx| this.show_failed_requests(cx))),
            )
    }

    fn span_list(&self, cx: &Context<Self>) -> Div {
        let p = palette(cx);
        let traces = &self.live_traces;
        card("Requests", cx).child(
            body()
                .pt_0()
                .child(muted(traces.count.clone(), cx))
                .children(traces.rows.iter().enumerate().take(100).map(|(ix, row)| {
                    let trace_id = row.trace_id.clone();
                    Button::new(SharedString::from(format!("obs-live-span-{ix}")))
                        .ghost()
                        .group("fog-control")
                        .selected(traces.trace.as_ref() == Some(&row.trace_id))
                        .w_full()
                        .h(dp(44.))
                        .justify_start()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .items_start()
                                .gap(dp(3.))
                                .child(
                                    line()
                                        .w_full()
                                        .child(mono(row.name.clone()).truncate().flex_1())
                                        .child(mono(row.duration.clone()).flex_none()),
                                )
                                .child(
                                    line()
                                        .w_full()
                                        .child(
                                            muted(format!("{} · {}", row.when, row.service), cx)
                                                .truncate()
                                                .flex_none(),
                                        )
                                        .when(row.error, |this| {
                                            this.child(
                                                text(if row.message.is_empty() {
                                                    "failed".to_string()
                                                } else {
                                                    row.message.clone()
                                                })
                                                .text_size(dp(12.))
                                                .text_color(p.crit_ink)
                                                .truncate(),
                                            )
                                        }),
                                ),
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.open_trace(trace_id.clone(), cx)
                            }),
                        )
                })),
        )
    }
}
