//! Coroot's latency and error heatmap: rows by duration, failures first,
//! and a click, or the arrow keys and Enter, that list one cell's requests.
use super::*;

actions!(
    observability_heatmap,
    [
        EarlierBucket,
        LaterBucket,
        HigherBucket,
        LowerBucket,
        FirstBucket,
        LastBucket,
        SelectBucket,
        LeaveHeatmap
    ]
);

const CONTEXT: &str = "ObservabilityHeatmap";

/// The heatmap's keys. The arrows move a cursor and never read; Enter or
/// Space lists the cell under it.
pub(in crate::observability) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", EarlierBucket, Some(CONTEXT)),
        KeyBinding::new("right", LaterBucket, Some(CONTEXT)),
        KeyBinding::new("up", HigherBucket, Some(CONTEXT)),
        KeyBinding::new("down", LowerBucket, Some(CONTEXT)),
        KeyBinding::new("home", FirstBucket, Some(CONTEXT)),
        KeyBinding::new("end", LastBucket, Some(CONTEXT)),
        KeyBinding::new("enter", SelectBucket, Some(CONTEXT)),
        KeyBinding::new("space", SelectBucket, Some(CONTEXT)),
        KeyBinding::new("escape", LeaveHeatmap, Some(CONTEXT)),
    ]);
}

/// Heatmap columns drawn; Coroot's points are summed into them.
const COLUMNS: usize = 48;

pub(in crate::observability) struct Heat {
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
    /// Each cell in words: its bucket, time range and rate, for its
    /// tooltip, accessibility label and the keyboard cursor's caption.
    cells: Vec<SharedString>,
}

impl Heat {
    /// A cell kept inside the grid, or none when the grid is empty.
    fn clamp(&self, (row, column): (usize, usize)) -> Option<(usize, usize)> {
        let rows = self.lines.len();
        let columns = self.columns.len();
        (rows > 0 && columns > 0).then(|| (row.min(rows - 1), column.min(columns - 1)))
    }
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
    let time = |ms: i64| {
        chrono::DateTime::from_timestamp_millis(ms).map_or_else(String::new, |t| {
            t.with_timezone(&chrono::Local).format("%H:%M").to_string()
        })
    };
    let bounds: Vec<(i64, i64)> = (0..columns as i64)
        .map(|c| {
            let from = heatmap.from_ms + c * span;
            (from, (from + span).min(heatmap.to_ms))
        })
        .collect();
    let times: Vec<String> = bounds
        .iter()
        .map(|&(from, to)| format!("{}–{}", time(from), time(to)))
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
            let bucket = match (ix, row.name.strip_prefix('>')) {
                _ if errors => "Failed requests".to_string(),
                (_, Some(over)) => format!("Over {over}"),
                (0, None) => format!("Up to {}", row.name),
                _ => format!("{}–{}", heatmap.rows[ix - 1].name, row.name),
            };
            let cells = sums[ix]
                .iter()
                .zip(&times)
                .map(|(&sum, times)| {
                    // Coroot's points are requests per second; a cell averages
                    // the points it sums.
                    let rate = sum / group as f32;
                    let what = if errors { "failures" } else { "requests" };
                    let rate = match rate {
                        r if r <= 0. => format!("no {what}"),
                        r if r < 0.1 => format!("under 0.1 {what}/s"),
                        r if r < 10. => format!("{r:.1} {what}/s"),
                        r => format!("{r:.0} {what}/s"),
                    };
                    format!("{bucket} · {times} · {rate}").into()
                })
                .collect();
            HeatLine {
                cells,
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
    Heat {
        lines,
        columns: bounds,
        start: time(heatmap.from_ms),
        end: time(heatmap.to_ms),
    }
}

/// A heatmap from Coroot's application view, read as the traces heatmap
/// is: Coroot names each row by its bound and its last row `errors`.
pub(in crate::observability) fn app_heat(heatmap: &api::AppHeatmap) -> Heat {
    heat(&api::Heatmap {
        from_ms: heatmap.from_ms,
        to_ms: heatmap.to_ms,
        step_ms: heatmap.step_ms,
        rows: heatmap
            .rows
            .iter()
            .map(|row| api::HeatRow {
                name: if row.title.is_empty() {
                    row.name.clone()
                } else {
                    row.title.clone()
                },
                value: if row.name == "errors" {
                    "err".into()
                } else {
                    row.name.clone()
                },
                points: row.points.clone(),
            })
            .collect(),
    })
}

impl ObservabilityPage {
    /// A report's heatmap: the traces heatmap's cells, without its cursor,
    /// since a report's cells list nothing.
    pub(in crate::observability) fn static_heatmap(
        &self,
        id: SharedString,
        title: SharedString,
        heat: &Heat,
        cx: &Context<Self>,
    ) -> AnyElement {
        let grid = v_flex()
            .gap(dp(3.))
            .children(heat.lines.iter().map(|line_data| {
                line()
                    .gap(dp(3.))
                    .h(dp(16.))
                    .child(
                        muted(line_data.label.clone(), cx)
                            .w(dp(56.))
                            .flex_none()
                            .text_right(),
                    )
                    .children(
                        line_data
                            .levels
                            .iter()
                            .zip(&line_data.cells)
                            .enumerate()
                            .map(|(column, (&level, label))| {
                                let label = label.clone();
                                div()
                                    .id(column)
                                    .aria_label(label.clone())
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .rounded(px(3.))
                                    .bg(crate::palette::heat_color(level, line_data.errors))
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(label.clone()).build(window, cx)
                                    })
                            }),
                    )
            }));
        card(title, cx)
            .id(id)
            .test_support()
            .child(
                body().pt_0().child(grid).child(
                    line()
                        .justify_between()
                        .pl(dp(59.))
                        .child(muted(heat.start.clone(), cx))
                        .child(muted(heat.end.clone(), cx)),
                ),
            )
            .into_any_element()
    }
}

impl Traces {
    /// Keeps the keyboard cursor inside a rebuilt grid.
    pub(super) fn clamp_cursor(&mut self) {
        self.cursor = match (&self.heat, self.cursor) {
            (Some(heat), Some(cursor)) => heat.clamp(cursor),
            _ => None,
        };
    }

    /// Where the cursor is, or starts: the selected cell, else the latest
    /// column of the first row.
    fn heat_cursor(&self) -> Option<(usize, usize)> {
        let heat = self.heat.as_ref()?;
        let start = self
            .cursor
            .or(self.cell)
            .unwrap_or((0, heat.columns.len().saturating_sub(1)));
        heat.clamp(start)
    }

    fn move_cursor(&mut self, to: impl FnOnce((usize, usize), usize) -> (usize, usize)) {
        let (Some(heat), Some(cursor)) = (&self.heat, self.heat_cursor()) else {
            return;
        };
        self.cursor = heat.clamp(to(cursor, heat.columns.len()));
    }
}

impl ObservabilityPage {
    fn step_cursor(&mut self, rows: isize, columns: isize, cx: &mut Context<Self>) {
        self.live_traces.move_cursor(|(row, column), _| {
            (
                row.saturating_add_signed(rows),
                column.saturating_add_signed(columns),
            )
        });
        cx.notify();
    }

    fn select_cursor(&mut self, cx: &mut Context<Self>) {
        if let Some((row, column)) = self.live_traces.heat_cursor() {
            self.live_traces.cursor = Some((row, column));
            self.select_trace_cell(row, column, cx);
        }
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

    /// The heatmap card, drawn in every state so that it keeps the keyboard
    /// while Coroot answers: the grid, or why there is none.
    pub(super) fn live_heatmap(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let traces = &self.live_traces;
        let focused = self.heat_focus.is_focused(window);
        let cursor = focused.then(|| traces.heat_cursor()).flatten();
        let caption = match (cursor, &traces.heat) {
            (Some((row, column)), Some(heat)) => div()
                .id("obs-heatmap-cursor")
                .test_support()
                .aria_label(heat.lines[row].cells[column].clone())
                .child(muted(
                    format!(
                        "{} · Enter lists these requests",
                        heat.lines[row].cells[column]
                    ),
                    cx,
                ))
                .into_any_element(),
            _ => muted(
                "Requests per second by duration · brighter = more · click a cell to list its requests",
                cx,
            )
            .into_any_element(),
        };
        let mut body = body().pt_0().child(caption);
        body = match &traces.heat {
            Some(heat) => body.child(self.heat_grid(heat, cursor, cx)).child(
                line()
                    .justify_between()
                    .pl(dp(59.))
                    .child(muted(heat.start.clone(), cx))
                    .child(muted(heat.end.clone(), cx)),
            ),
            None => body.child(muted(
                if self.live.tracing.is_loading() {
                    "Reading the heatmap…"
                } else if self.live.tracing.error().is_some() {
                    "The heatmap didn't load."
                } else {
                    "Coroot sent no heatmap for this window."
                },
                cx,
            )),
        };
        card("Latency & errors", cx)
            .id("obs-heatmap")
            .test_support()
            .track_focus(&self.heat_focus)
            .key_context(CONTEXT)
            .aria_label(
                "Latency and error heatmap. Arrow keys move between cells; Enter lists a cell's requests.",
            )
            .on_action(cx.listener(|this, _: &EarlierBucket, _, cx| this.step_cursor(0, -1, cx)))
            .on_action(cx.listener(|this, _: &LaterBucket, _, cx| this.step_cursor(0, 1, cx)))
            .on_action(cx.listener(|this, _: &HigherBucket, _, cx| this.step_cursor(-1, 0, cx)))
            .on_action(cx.listener(|this, _: &LowerBucket, _, cx| this.step_cursor(1, 0, cx)))
            .on_action(cx.listener(|this, _: &FirstBucket, _, cx| {
                this.live_traces.move_cursor(|(row, _), _| (row, 0));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &LastBucket, _, cx| {
                this.live_traces
                    .move_cursor(|(row, _), columns| (row, columns.saturating_sub(1)));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectBucket, _, cx| this.select_cursor(cx)))
            .on_action(cx.listener(|this, _: &LeaveHeatmap, window, cx| {
                window.focus(&this.focus, cx);
                cx.notify();
            }))
            .child(body)
            .into_any_element()
    }

    fn heat_grid(
        &self,
        heat: &Heat,
        cursor: Option<(usize, usize)>,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let cell = self.live_traces.cell;
        v_flex()
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
                        // The keyboard cursor outlines more heavily than the
                        // selection; both are the accent, inside a ring of the
                        // card's colour that keeps them clear on the lightest
                        // cells.
                        let border = if cursor == Some((row, column)) {
                            Some(px(2.))
                        } else if cell == Some((row, column)) {
                            Some(px(1.))
                        } else {
                            None
                        };
                        let label = line_data.cells[column].clone();
                        div()
                            .id(SharedString::from(format!(
                                "obs-live-bucket-{row}-{column}"
                            )))
                            .test_support()
                            .aria_label(label.clone())
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .rounded(px(3.))
                            .bg(crate::palette::heat_color(level, line_data.errors))
                            .border(border.unwrap_or(px(1.)))
                            .border_color(if border.is_some() {
                                p.accent
                            } else {
                                gpui_kit::transparent_black()
                            })
                            .when(border.is_some(), |this| {
                                this.child(div().size_full().border_1().border_color(p.surface))
                            })
                            .cursor_pointer()
                            .tooltip(move |window, cx| {
                                Tooltip::new(format!(
                                    "{label} · Click or press Enter to list these requests"
                                ))
                                .build(window, cx)
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    window.focus(&this.heat_focus, cx);
                                    this.live_traces.cursor = Some((row, column));
                                    this.select_trace_cell(row, column, cx)
                                }),
                            )
                    }))
            }))
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
        // The axis reads in local time, as the request list does.
        let local = |ms| {
            chrono::DateTime::from_timestamp_millis(ms)
                .unwrap()
                .with_timezone(&chrono::Local)
                .format("%H:%M")
                .to_string()
        };
        assert_eq!(
            (heat.start.clone(), heat.end.clone()),
            (local(0), local(120 * 60_000))
        );
    }
}
