//! One trace as a waterfall: spans in call order under their parents,
//! and the selected span's attributes.
use super::*;

/// Waterfall rows drawn; a larger trace says how many it leaves out.
const WATERFALL_ROWS: usize = 200;
/// Attributes shown for the selected span.
const ATTRIBUTES: usize = 16;

pub(super) struct Waterfall {
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

/// Spans in call order, each under its parent, sized against the whole trace.
pub(super) fn waterfall(trace_id: &str, spans: &[api::Span]) -> Waterfall {
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
    pub(super) fn live_waterfall(&self, cx: &Context<Self>) -> AnyElement {
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
    use super::{api, waterfall};

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
}
