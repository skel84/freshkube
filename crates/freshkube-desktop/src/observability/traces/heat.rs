//! Coroot's latency and error heatmap: rows by duration, failures first,
//! and a click that lists one cell's requests.
use super::*;

/// Heatmap columns drawn; Coroot's points are summed into them.
const COLUMNS: usize = 48;

pub(super) struct Heat {
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

pub(super) fn heat(heatmap: &api::Heatmap) -> Heat {
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

impl ObservabilityPage {
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

    pub(super) fn live_heatmap(&self, heat: &Heat, cx: &Context<Self>) -> Div {
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
}

#[cfg(test)]
mod tests {
    use super::{api, heat};

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
