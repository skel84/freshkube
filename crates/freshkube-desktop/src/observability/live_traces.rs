//! Live traces for one application: Coroot's latency and error heatmap, the
//! spans of the whole window or one selected cell, and one trace as a
//! waterfall. Display data is prepared when an answer arrives; render reads it.
use super::*;
use freshkube_core::coroot as api;

/// Heatmap columns drawn; Coroot's points are summed into them.
const COLUMNS: usize = 48;
/// Waterfall rows drawn; a larger trace says how many it leaves out.
const WATERFALL_ROWS: usize = 200;
/// Attributes shown for the selected span.
const ATTRIBUTES: usize = 16;

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

struct Heat {
    /// Failed requests first, then slowest to fastest.
    lines: Vec<HeatLine>,
    /// Each column's start and end, in epoch milliseconds.
    columns: Vec<(i64, i64)>,
    start: String,
    end: String,
}

struct HeatLine {
    label: String,
    above: String,
    up_to: String,
    errors: bool,
    levels: Vec<usize>,
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

struct Waterfall {
    trace_id: String,
    title: String,
    summary: String,
    total: String,
    rows: Vec<WaterRow>,
}

struct WaterRow {
    service: String,
    name: String,
    depth: usize,
    start: f32,
    width: f32,
    error: bool,
    duration: String,
    detail: Vec<String>,
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

fn heat(heatmap: &api::Heatmap) -> Heat {
    let points = heatmap
        .rows
        .iter()
        .map(|r| r.points.len())
        .max()
        .unwrap_or(0)
        .max(1);
    let group = points.div_ceil(COLUMNS);
    let columns = points.div_ceil(group);
    let span = group as i64 * heatmap.step_ms;
    let sums: Vec<Vec<f32>> = heatmap
        .rows
        .iter()
        .map(|row| {
            (0..columns)
                .map(|c| {
                    row.points
                        .iter()
                        .skip(c * group)
                        .take(group)
                        .flatten()
                        .sum()
                })
                .collect()
        })
        .collect();
    let most = |errors: bool| {
        heatmap
            .rows
            .iter()
            .zip(&sums)
            .filter(|(row, _)| row.is_errors() == errors)
            .flat_map(|(_, sums)| sums.iter().copied())
            .fold(0f32, f32::max)
    };
    let (requests, failures) = (most(false), most(true));
    let level = |value: f32, most: f32| {
        if value <= 0. || most <= 0. {
            0
        } else {
            1 + ((value.ln_1p() / most.ln_1p()) * 4.).round().clamp(0., 4.) as usize
        }
    };
    let mut lines: Vec<HeatLine> = heatmap
        .rows
        .iter()
        .enumerate()
        .map(|(ix, row)| {
            let errors = row.is_errors();
            HeatLine {
                label: if errors {
                    "errors".into()
                } else {
                    row.name.clone()
                },
                above: match ix {
                    0 => "0".into(),
                    _ if errors => String::new(),
                    _ => heatmap.rows[ix - 1].value.clone(),
                },
                up_to: row.value.clone(),
                errors,
                levels: sums[ix]
                    .iter()
                    .map(|&v| level(v, if errors { failures } else { requests }))
                    .collect(),
            }
        })
        .collect();
    // Coroot lists the failed requests last and the fastest first.
    lines.sort_by_key(|line| !line.errors);
    let errors = lines.iter().take_while(|l| l.errors).count();
    lines[errors..].reverse();
    let time = |ms: i64| {
        chrono::DateTime::from_timestamp_millis(ms)
            .map_or_else(String::new, |t| t.format("%H:%M").to_string())
    };
    Heat {
        lines,
        columns: (0..columns as i64)
            .map(|c| {
                let from = heatmap.from_ms + c * span;
                (from, (from + span).min(heatmap.to_ms))
            })
            .collect(),
        start: time(heatmap.from_ms),
        end: time(heatmap.to_ms),
    }
}

/// Spans in call order, each under its parent, sized against the whole trace.
fn waterfall(trace_id: &str, spans: &[api::Span]) -> Waterfall {
    let start = spans.iter().map(|s| s.timestamp).min().unwrap_or(0);
    let end = spans
        .iter()
        .map(|s| s.timestamp as f64 + s.duration.max(0.))
        .fold(start as f64, f64::max);
    let total = (end - start as f64).max(0.001);
    let ids: std::collections::HashSet<&str> = spans.iter().map(|s| s.id.as_str()).collect();
    let mut children: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut roots = vec![];
    for (ix, span) in spans.iter().enumerate() {
        if span.parent_id.is_empty() || !ids.contains(span.parent_id.as_str()) {
            roots.push(ix);
        } else {
            children
                .entry(span.parent_id.as_str())
                .or_default()
                .push(ix);
        }
    }
    let by_start = |list: &mut Vec<usize>| list.sort_by_key(|&ix| spans[ix].timestamp);
    by_start(&mut roots);
    children.values_mut().for_each(by_start);
    let mut order = vec![];
    let mut seen = vec![false; spans.len()];
    let mut stack: Vec<(usize, usize)> = roots.iter().rev().map(|&ix| (ix, 0)).collect();
    while let Some((ix, depth)) = stack.pop() {
        if std::mem::replace(&mut seen[ix], true) {
            continue;
        }
        order.push((ix, depth));
        if let Some(list) = children.get(spans[ix].id.as_str()) {
            stack.extend(list.iter().rev().map(|&child| (child, depth + 1)));
        }
    }
    let failed = spans.iter().filter(|s| s.status.error).count();
    let root = order.first().map(|&(ix, _)| &spans[ix]);
    let rows = order
        .iter()
        .take(WATERFALL_ROWS)
        .map(|&(ix, depth)| {
            let span = &spans[ix];
            let mut detail = vec![];
            if !span.status.message.is_empty() {
                detail.push(span.status.message.clone());
            }
            if !span.details.text.is_empty() {
                detail.push(span.details.text.clone());
            }
            detail.extend(
                span.attributes
                    .iter()
                    .take(ATTRIBUTES)
                    .map(|(key, value)| format!("{key} = {value}")),
            );
            detail.extend(span.events.iter().take(4).map(|event| {
                let message = event
                    .attributes
                    .get("exception.message")
                    .map_or(String::new(), |m| format!(": {m}"));
                format!("event {}{message}", event.name)
            }));
            WaterRow {
                service: span.service.clone(),
                name: span.name.clone(),
                depth,
                start: ((span.timestamp - start) as f64 / total) as f32,
                width: ((span.duration.max(0.) / total) as f32).max(0.004),
                error: span.status.error,
                duration: millis(span.duration),
                detail,
            }
        })
        .collect();
    let hidden = order.len().saturating_sub(WATERFALL_ROWS);
    Waterfall {
        trace_id: trace_id.to_owned(),
        title: root.map_or_else(
            || "Trace".into(),
            |span| format!("{} · {}", span.service, span.name),
        ),
        summary: format!(
            "{} spans{}{}",
            spans.len(),
            match failed {
                0 => String::new(),
                n => format!(" · {n} failed"),
            },
            match hidden {
                0 => String::new(),
                n => format!(" · first {WATERFALL_ROWS} shown, {n} more"),
            }
        ),
        total: millis(total),
        rows,
    }
}

impl ObservabilityPage {
    pub(super) fn read_traces(&mut self, cx: &mut Context<Self>) {
        let (Some(provider), Some(source), Some(app)) = (
            self.live.provider.clone(),
            self.live.source.clone(),
            self.selected_app.clone(),
        ) else {
            return;
        };
        self.live_traces.reset_for(&app);
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

    fn read_trace(&mut self, cx: &mut Context<Self>) {
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

    fn select_trace_cell(&mut self, row: usize, column: usize, cx: &mut Context<Self>) {
        let Some(heat) = &self.live_traces.heat else {
            return;
        };
        let (Some(line), Some(&(from_ms, to_ms))) = (heat.lines.get(row), heat.columns.get(column))
        else {
            return;
        };
        let selection = if line.errors {
            api::TraceSelection::Errors { from_ms, to_ms }
        } else {
            api::TraceSelection::Latency {
                from_ms,
                to_ms,
                above: line.above.clone(),
                up_to: line.up_to.clone(),
            }
        };
        let traces = &mut self.live_traces;
        traces.cell = Some((row, column));
        traces.selection = selection;
        traces.trace = None;
        traces.waterfall = None;
        self.read_traces(cx);
        cx.notify();
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
        if self.live.tracing.data().is_none() {
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

    fn live_heatmap(&self, heat: &Heat, cx: &Context<Self>) -> Div {
        let p = palette(cx);
        let cell = self.live_traces.cell;
        let grid = v_flex()
            .id("obs-live-heatmap")
            .test_support()
            .gap(dp(3.))
            .children(heat.lines.iter().enumerate().map(|(row, line_data)| {
                line()
                    .gap(dp(3.))
                    .h(dp(16.))
                    .child(
                        muted(line_data.label.clone(), cx)
                            .w(dp(56.))
                            .flex_none()
                            .text_right(),
                    )
                    .children(line_data.levels.iter().enumerate().map(|(column, &level)| {
                        let selected = cell == Some((row, column));
                        let label = line_data.label.clone();
                        div()
                            .id(SharedString::from(format!(
                                "obs-live-bucket-{row}-{column}"
                            )))
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .rounded(px(2.))
                            .bg(crate::palette::heat_color(level, line_data.errors))
                            .border_1()
                            .border_color(if selected {
                                p.accent
                            } else {
                                gpui_kit::transparent_black()
                            })
                            .cursor_pointer()
                            .tooltip(move |window, cx| {
                                Tooltip::new(format!("{label} · Click to list these requests"))
                                    .build(window, cx)
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.select_trace_cell(row, column, cx)
                                }),
                            )
                    }))
            }));
        card("Latency & errors", cx).child(
            body()
                .pt_0()
                .child(muted(
                    "Requests per second by duration · brighter = more · click a cell to list its requests",
                    cx,
                ))
                .child(grid)
                .child(
                    line()
                        .justify_between()
                        .pl(dp(59.))
                        .child(muted(heat.start.clone(), cx))
                        .child(muted(heat.end.clone(), cx)),
                ),
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

    fn live_waterfall(&self, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let traces = &self.live_traces;
        let Some(fall) = &traces.waterfall else {
            let message = if traces.trace.is_none() {
                "Select a request to see its trace.".to_string()
            } else if let Some(error) = self.live.trace.error() {
                error.to_string()
            } else {
                "Reading the trace…".to_string()
            };
            return card("Trace", cx)
                .child(body().pt_0().child(muted(message, cx)))
                .into_any_element();
        };
        let selected = fall.rows.get(traces.span);
        card(fall.title.clone(), cx)
            .id("obs-live-waterfall")
            .test_support()
            .child(
                body()
                    .pt_0()
                    .child(
                        line()
                            .flex_wrap()
                            .child(muted(fall.summary.clone(), cx))
                            .child(div().flex_1())
                            .child(muted(format!("trace {}", fall.trace_id), cx)),
                    )
                    .child(
                        line()
                            .justify_between()
                            .child(muted("0 ms", cx))
                            .child(muted(fall.total.clone(), cx)),
                    )
                    .children(fall.rows.iter().enumerate().map(|(ix, row)| {
                        Button::new(SharedString::from(format!("obs-live-trace-span-{ix}")))
                            .ghost()
                            .group("fog-control")
                            .selected(traces.span == ix)
                            .w_full()
                            .h(dp(28.))
                            .justify_start()
                            .px_0()
                            .gap(dp(8.))
                            .child(
                                mono(row.name.clone())
                                    .pl(dp(row.depth.min(12) as f32 * 10.))
                                    .w(dp(220.))
                                    .flex_none()
                                    .truncate(),
                            )
                            .child(
                                div().flex_1().relative().h(dp(10.)).child(
                                    div()
                                        .absolute()
                                        .left(relative(row.start.min(0.996)))
                                        .w(relative(row.width.min(1. - row.start).max(0.004)))
                                        .h_full()
                                        .rounded(px(3.))
                                        .bg(if row.error { p.crit } else { p.accent }),
                                ),
                            )
                            .child(
                                mono(row.duration.clone())
                                    .w(dp(72.))
                                    .flex_none()
                                    .text_right(),
                            )
                            .tooltip(format!("{} · {}", row.service, row.name))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.live_traces.span = ix;
                                cx.notify();
                            }))
                    }))
                    .when_some(selected, |this, row| {
                        this.child(
                            v_flex()
                                .id("obs-live-span-detail")
                                .test_support()
                                .mt(dp(8.))
                                .pt(dp(12.))
                                .border_t_1()
                                .border_color(p.line)
                                .gap(dp(6.))
                                .child(
                                    line()
                                        .child(mono(row.service.clone()))
                                        .child(muted(row.duration.clone(), cx)),
                                )
                                .child(text(row.name.clone()).whitespace_normal())
                                .children(row.detail.iter().map(|detail| {
                                    mono(detail.clone()).text_color(p.muted).whitespace_normal()
                                })),
                        )
                    }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{api, heat, waterfall};

    fn span(id: &str, parent: &str, at: i64, ms: f64, error: bool) -> api::Span {
        serde_json::from_value(serde_json::json!({
            "service":"api","trace_id":"t1","id":id,"parent_id":parent,"name":format!("op {id}"),
            "timestamp":at,"duration":ms,"status":{"error":error,"message":""},
            "details":{"text":"","lang":""},"attributes":{},"events":null
        }))
        .unwrap()
    }

    #[test]
    fn a_waterfall_nests_children_under_parents_in_start_order() {
        let spans = [
            span("c", "a", 1_050, 10., false),
            span("a", "", 1_000, 100., false),
            span("b", "a", 1_010, 20., true),
            span("d", "missing", 1_020, 5., false),
        ];
        let fall = waterfall("t1", &spans);
        let order: Vec<_> = fall
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.depth))
            .collect();
        assert_eq!(
            order,
            vec![("op a", 0), ("op b", 1), ("op c", 1), ("op d", 0)]
        );
        assert_eq!(fall.summary, "4 spans · 1 failed");
        assert!((fall.rows[2].start - 0.5).abs() < 1e-6);
    }

    #[test]
    fn heatmap_rows_put_failures_first_then_slowest_and_cells_know_their_bounds() {
        let heatmap = api::Heatmap {
            from_ms: 0,
            to_ms: 120 * 60_000,
            step_ms: 60_000,
            rows: vec![
                api::HeatRow {
                    name: "5ms".into(),
                    value: "0.005".into(),
                    points: vec![Some(1.); 120],
                },
                api::HeatRow {
                    name: ">5s".into(),
                    value: "inf".into(),
                    points: vec![None; 120],
                },
                api::HeatRow {
                    name: "errors".into(),
                    value: "err".into(),
                    points: vec![Some(0.5); 120],
                },
            ],
        };
        let heat = heat(&heatmap);
        let labels: Vec<_> = heat.lines.iter().map(|l| l.label.as_str()).collect();
        assert_eq!(labels, vec!["errors", ">5s", "5ms"]);
        assert_eq!(heat.columns.len(), 40);
        assert_eq!(heat.columns[1], (180_000, 360_000));
        assert_eq!(
            (heat.lines[1].above.as_str(), heat.lines[1].up_to.as_str()),
            ("0.005", "inf")
        );
        assert_eq!(heat.lines[1].levels[0], 0);
        assert_eq!(heat.lines[2].levels[0], 5);
    }
}
