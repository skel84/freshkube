//! The cursor over a timeseries: a crosshair at the nearest sample with a
//! dot on each line, and the values at that time beside it. The readout is
//! formatted when the pointer moves, never in `render`. The page passes a
//! cursor from one chart to the others with [`PanelView::show_cursor`].
use chrono::{Datelike, Local, TimeZone};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Bounds, Context, FontWeight, MouseMoveEvent, Pixels, SharedString, TestSupportExt,
    Window, canvas, div, fill, point, px, size,
};

use super::{PanelEvent, PanelView, markers};
use crate::monitoring::derive::{self, Chart};
use crate::palette::palette;
use crate::ui::{self, dp, dp_px};

/// Readout rows drawn at most; the rest are counted.
const READOUT_ROWS: usize = 10;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Cursor {
    pub index: usize,
    /// The crosshair, from the plot container's left edge.
    pub x: Pixels,
    /// The readout sits left of the crosshair near the right edge.
    pub flip: bool,
    /// The container's width when the cursor was placed.
    pub width: Pixels,
    /// Whether the pointer is over this panel; only that one shows the
    /// readout.
    pub own: bool,
    pub time: SharedString,
    pub rows: Vec<Row>,
    pub more: usize,
    /// The marker under the pointer, named first in the readout.
    pub marker: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub series: usize,
    pub value: SharedString,
    pub name: SharedString,
}

impl PanelView {
    /// The cursor another chart on the page is showing, at `time`.
    pub(crate) fn show_cursor(&mut self, time: Option<f64>, cx: &mut Context<Self>) {
        let _span = crate::perf::span("monitoring.cursor");
        let cursor = time.and_then(|time| {
            let chart = self.chart()?;
            let index = nearest_time(&chart.times, time)?;
            self.cursor_at(&chart, index, false)
        });
        if self.cursor != cursor {
            self.cursor = cursor;
            cx.notify();
        }
    }

    /// Where the pointer is over the plot, in window coordinates.
    fn pointer_moved(
        &mut self,
        position: gpui_kit::Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(chart) = self.chart() else {
            return;
        };
        let geometry = self.geometry.get();
        let x = position.x - geometry.origin.x - geometry.left;
        let inside = geometry.width > px(0.) && x >= px(0.) && x <= geometry.width;
        let index = inside
            .then(|| nearest_x(&chart.xs, x / geometry.width))
            .flatten();
        let mut cursor = index.and_then(|index| self.cursor_at(&chart, index, true));
        if let Some(cursor) = &mut cursor {
            let reach = dp_px(markers::REACH, window) / geometry.width;
            cursor.marker = markers::nearest(&self.placed, x / geometry.width, reach);
        }
        let key = |c: &Cursor| (c.index, c.own, c.marker);
        if self.cursor.as_ref().map(key) == cursor.as_ref().map(key) {
            return;
        }
        let time = cursor.as_ref().map(|cursor| chart.times[cursor.index]);
        self.cursor = cursor;
        cx.emit(PanelEvent::Cursor(time));
        cx.notify();
    }

    fn pointer_left(&mut self, cx: &mut Context<Self>) {
        if self.cursor.take().is_some() {
            cx.emit(PanelEvent::Cursor(None));
            cx.notify();
        }
    }

    pub(super) fn chart(&self) -> Option<std::rc::Rc<Chart>> {
        match &self.data.as_ref()?.body {
            derive::Body::Chart(chart) => Some(chart.clone()),
            _ => None,
        }
    }

    fn cursor_at(&self, chart: &Chart, index: usize, own: bool) -> Option<Cursor> {
        let geometry = self.geometry.get();
        if geometry.width <= px(0.) {
            return None;
        }
        let x = geometry.left + geometry.width * *chart.xs.get(index)?;
        let shown: Vec<Row> = chart
            .series
            .iter()
            .enumerate()
            .filter(|(series, _)| chart.legend.rows.iter().any(|row| row.series == *series))
            .map(|(series, line)| Row {
                series,
                value: derive::format(
                    &line.field,
                    line.values.get(index).copied().unwrap_or(f64::NAN),
                )
                .into(),
                name: line.name.clone(),
            })
            .collect();
        let more = shown.len().saturating_sub(READOUT_ROWS);
        Some(Cursor {
            index,
            x,
            flip: x > geometry.size.width * 0.6,
            width: geometry.size.width,
            own,
            time: when(chart.times[index]).into(),
            rows: shown.into_iter().take(READOUT_ROWS).collect(),
            more,
            marker: None,
        })
    }

    /// The hover handlers for the plot's container, and the overlay drawn
    /// above the cached plot.
    pub(super) fn render_cursor(
        &mut self,
        chart: &std::rc::Rc<Chart>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let mut overlay = div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, window, cx| {
                view.pointer_moved(event.position, window, cx)
            }));
        let Some(cursor) = self.cursor.clone() else {
            return overlay
                .id(self.element_id("cursor"))
                .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                    if !hovered {
                        view.pointer_left(cx)
                    }
                }))
                .into_any_element();
        };
        let geometry = self.geometry.clone();
        let (line, ring) = (
            p.ink_2.opacity(if cursor.own { 0.7 } else { 0.35 }),
            p.surface,
        );
        let dots: Vec<(f32, gpui_kit::Hsla)> = chart
            .series
            .iter()
            .enumerate()
            .filter_map(|(series, line)| {
                let y = *line.tops.get(cursor.index)?;
                let focused = self.focus() == Some(series);
                y.is_finite().then(|| (y, line.ink.color(focused)))
            })
            .collect();
        let at = cursor.x;
        overlay = overlay.child(
            canvas(
                |_, _, _| {},
                move |bounds: Bounds<Pixels>, _, window, _| {
                    let g = geometry.get();
                    let x = bounds.origin.x + at;
                    let top = bounds.origin.y + g.top;
                    window.paint_quad(fill(
                        Bounds::new(point(x, top), size(px(1.), g.height)),
                        line,
                    ));
                    let (outer, inner) = (dp_px(10., window), dp_px(7., window));
                    for (y, color) in &dots {
                        let center = point(x + px(0.5), top + g.height * (1. - *y));
                        let ring_bounds = Bounds::centered_at(center, size(outer, outer));
                        window.paint_quad(fill(ring_bounds, ring).corner_radii(outer / 2.));
                        let dot = Bounds::centered_at(center, size(inner, inner));
                        window.paint_quad(fill(dot, *color).corner_radii(inner / 2.));
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
        if cursor.own && !cursor.rows.is_empty() {
            overlay = overlay.child(self.render_readout(chart, &cursor, cx));
        }
        overlay
            .id(self.element_id("cursor"))
            .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                if !hovered {
                    view.pointer_left(cx)
                }
            }))
            .into_any_element()
    }

    fn render_readout(&self, chart: &Chart, cursor: &Cursor, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let focus = self.focus();
        let gap = dp(12.);
        v_flex()
            .id(self.element_id("readout"))
            .absolute()
            .top(dp(10.))
            .map(|this| {
                if cursor.flip {
                    this.right(cursor.width - cursor.x).mr(gap)
                } else {
                    this.left(cursor.x).ml(gap)
                }
            })
            .gap(dp(3.))
            .px(dp(10.))
            .py(dp(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(p.line_strong)
            .bg(p.hover)
            .shadow_lg()
            .child(
                div()
                    .font_family(ui::MONO_FONT)
                    .text_size(dp(11.))
                    .text_color(p.muted)
                    .child(cursor.time.clone()),
            )
            .children(
                cursor
                    .marker
                    .and_then(|marker| self.placed.get(marker))
                    .map(|marker| markers::readout_row(marker, &p)),
            )
            .children(cursor.rows.iter().map(|row| {
                let color = chart.series[row.series]
                    .ink
                    .color(focus == Some(row.series));
                h_flex()
                    .gap(dp(8.))
                    .child(
                        div()
                            .flex_none()
                            .w(dp(12.))
                            .h(px(2.))
                            .rounded(px(1.))
                            .bg(color),
                    )
                    .child(
                        div()
                            .min_w(dp(40.))
                            .font_family(ui::MONO_FONT)
                            .text_size(dp(12.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.ink)
                            .whitespace_nowrap()
                            .child(row.value.clone()),
                    )
                    .child(
                        div()
                            .max_w(dp(220.))
                            .truncate()
                            .font_family(ui::MONO_FONT)
                            .text_size(dp(11.))
                            .text_color(p.muted)
                            .child(row.name.clone()),
                    )
            }))
            .when(cursor.more > 0, |this| {
                this.child(
                    div()
                        .text_size(dp(11.))
                        .text_color(p.faint)
                        .child(format!("and {} more", cursor.more)),
                )
            })
            .test_support()
            .into_any_element()
    }
}

/// The sample nearest `fraction` across the window.
pub(super) fn nearest_x(xs: &[f32], fraction: f32) -> Option<usize> {
    if xs.is_empty() {
        return None;
    }
    let after = xs.partition_point(|x| *x < fraction);
    let candidates = [after.checked_sub(1), (after < xs.len()).then_some(after)];
    candidates.into_iter().flatten().min_by(|a, b| {
        (xs[*a] - fraction)
            .abs()
            .total_cmp(&(xs[*b] - fraction).abs())
    })
}

/// The sample nearest `time`, when it falls within the samples.
pub(super) fn nearest_time(times: &[f64], time: f64) -> Option<usize> {
    let (first, last) = (*times.first()?, *times.last()?);
    if time < first || time > last {
        return None;
    }
    let after = times.partition_point(|t| *t < time);
    [after.checked_sub(1), (after < times.len()).then_some(after)]
        .into_iter()
        .flatten()
        .min_by(|a, b| {
            (times[*a] - time)
                .abs()
                .total_cmp(&(times[*b] - time).abs())
        })
}

/// "Today 13:40", or the date for another day.
pub(super) fn when(time: f64) -> String {
    let Some(at) = Local.timestamp_opt(time as i64, 0).single() else {
        return String::new();
    };
    if at.date_naive() == Local::now().date_naive() {
        at.format("Today %H:%M").to_string()
    } else if at.year() == Local::now().year() {
        at.format("%b %-d %H:%M").to_string()
    } else {
        at.format("%Y-%m-%d %H:%M").to_string()
    }
}
