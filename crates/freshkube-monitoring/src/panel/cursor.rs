//! The cursor over a timeseries: a crosshair at the nearest sample with a
//! dot on each line, and the values at that time beside it. The readout is
//! formatted when the pointer moves, never in `render`.
//!
//! The chart's own cursor is a view of its own, [`CursorOverlay`], which
//! the view that lays the panel out draws beside the cached panel: a notify
//! reaches every ancestor view, so a cursor drawn inside the panel would
//! lay out its header and legend and paint its plot again on every move.
//! The panel keeps the pointer's handlers and tells the overlay what to show.
//!
//! That view also passes one chart's cursor to the others through
//! [`Linked`]: it keeps each other chart's [`Crosshair`] and draws it beside
//! the cached panel too.
use std::cell::Cell;
use std::rc::Rc;

use chrono::{Datelike, Local, TimeZone};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, Context, Entity, EntityId, Font, FontWeight, Hsla, IntoElement,
    MouseMoveEvent, Pixels, Point, Render, SharedString, TestSupportExt, TextRun, Window, canvas,
    div, fill, point, px, size,
};

use super::{Geometry, PanelEvent, PanelView, markers};
use crate::colors::Ink;
use crate::derive::{self, Chart};
use crate::palette::{Palette, palette};
use crate::ui::{self, dp, dp_px};

/// A dot on a line: its height in the plot, from 0 to 1, its series' ink
/// and whether that series has the focus. The colour is resolved as it
/// paints, so a theme change shows at once.
type Dot = (f32, Ink, bool);

/// Readout rows drawn at most; the rest are counted.
pub(super) const READOUT_ROWS: usize = 10;
/// One readout row's height, and the readout's top offset, padding and time
/// line around them, in dp.
const ROW: f32 = 18.;
const CHROME: f32 = 10. + 2. * 8. + ROW + 4.;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Cursor {
    pub index: usize,
    /// The crosshair, from the plot container's left edge.
    pub x: Pixels,
    /// The readout sits left of the crosshair when there is more room
    /// that side.
    pub flip: bool,
    /// The room between the crosshair and the plot's edge on the readout's
    /// side, which caps its width so it never leaves the plot.
    pub room: Pixels,
    /// The container's width when the cursor was placed.
    pub width: Pixels,
    /// The narrowest the readout may be: its time and every value whole,
    /// even where that crosses the plot's edge.
    pub least: Pixels,
    pub time: SharedString,
    pub rows: Vec<Row>,
    /// Shown series past `rows`; when there are any, `rows` holds the
    /// highest values.
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

/// Another chart's cursor on this one: its crosshair and dots, with no
/// readout.
#[derive(Clone)]
pub(crate) struct Crosshair {
    id: SharedString,
    geometry: Rc<Cell<Geometry>>,
    /// The sample's place across the plot, from 0 to 1.
    pub(super) x: f32,
    dots: Vec<Dot>,
}

impl Crosshair {
    /// Drawn over the panel's slot, where the plot last painted.
    fn element(self) -> AnyElement {
        div()
            .id(self.id.clone())
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, cx| {
                        paint(self.geometry.get(), self.x, &self.dots, 0.35, window, cx)
                    },
                )
                .size_full(),
            )
            .test_support()
            .into_any_element()
    }
}

/// A chart's own cursor: the crosshair with its dots, and the readout.
/// The panel sets what it shows when the pointer moves, the focus changes
/// or the markers move; the view that lays the panel out draws it over the
/// panel's slot.
pub struct CursorOverlay {
    id: SharedString,
    geometry: Rc<Cell<Geometry>>,
    /// Where this view was laid out last, in the window: the readout sits
    /// at the plot's place from there.
    origin: Rc<Cell<Point<Pixels>>>,
    shown: Option<Shown>,
}

/// What the overlay draws, taken from the panel when it changes.
struct Shown {
    cursor: Cursor,
    /// The sample's place across the plot, from 0 to 1.
    x: f32,
    dots: Vec<Dot>,
    /// Each readout row's ink, and whether its series has the focus.
    inks: Vec<(Ink, bool)>,
    /// The marker under the pointer.
    marker: Option<markers::Placed>,
}

impl Shown {
    /// Each readout row's colour in the theme `p`, and whether its series
    /// has the focus.
    fn colors(&self, p: &Palette) -> impl Iterator<Item = (Hsla, bool)> + '_ {
        let p = *p;
        (self.inks.iter()).map(move |&(ink, focused)| (ink.color(&p, focused), focused))
    }
}

impl CursorOverlay {
    /// The readout's swatch colours, as it draws them now.
    #[cfg(test)]
    pub(crate) fn swatches(&self, cx: &App) -> Option<Vec<Hsla>> {
        let colors = self.shown.as_ref()?.colors(&palette(cx));
        Some(colors.map(|(color, _)| color).collect())
    }

    /// The series the readout names, and the focused one among them.
    #[cfg(test)]
    pub(crate) fn named(&self) -> Option<(Vec<usize>, Option<usize>)> {
        let shown = self.shown.as_ref()?;
        let rows = shown.cursor.rows.iter().map(|row| row.series);
        let focused = rows.clone().zip(&shown.inks).find(|(_, (_, f))| *f);
        Some((rows.collect(), focused.map(|(series, _)| series)))
    }

    fn new(id: SharedString, geometry: Rc<Cell<Geometry>>) -> Self {
        Self {
            id,
            geometry,
            origin: Rc::default(),
            shown: None,
        }
    }
}

/// One chart's cursor shown on the others beside it, kept by the view that
/// lays them out.
#[derive(Default)]
pub(crate) struct Linked {
    /// The chart under the pointer, and the time there.
    from: Option<(EntityId, f64)>,
    /// Each panel's crosshair, in the view's order.
    crosshairs: Vec<Option<Crosshair>>,
}

impl Linked {
    /// `from`'s cursor moved to `time`, or left its plot.
    pub(crate) fn show<'a>(
        &mut self,
        from: EntityId,
        time: Option<f64>,
        panels: impl IntoIterator<Item = &'a Entity<PanelView>>,
        cx: &App,
    ) {
        self.from = time.map(|time| (from, time));
        self.refresh(panels, cx);
    }

    /// Places the crosshairs again, on the panels' latest answers. Whether
    /// any were placed before or now, so the view draws again.
    pub(crate) fn refresh<'a>(
        &mut self,
        panels: impl IntoIterator<Item = &'a Entity<PanelView>>,
        cx: &App,
    ) -> bool {
        let _span = crate::perf::span("monitoring.cursor");
        let had = !self.crosshairs.is_empty();
        self.crosshairs.clear();
        let Some((from, time)) = self.from else {
            return had;
        };
        self.crosshairs.extend(panels.into_iter().map(|panel| {
            (panel.entity_id() != from)
                .then(|| panel.read(cx).crosshair(time))
                .flatten()
        }));
        true
    }

    /// The crosshair over panel `index`, if another chart's cursor falls
    /// on its samples.
    pub(crate) fn element(&self, index: usize) -> Option<AnyElement> {
        Some(self.crosshairs.get(index)?.clone()?.element())
    }

    #[cfg(test)]
    pub(crate) fn crosshair(&self, index: usize) -> Option<&Crosshair> {
        self.crosshairs.get(index)?.as_ref()
    }
}

impl PanelView {
    /// Another chart's cursor at `time`, when it falls on this chart's
    /// samples.
    pub(crate) fn crosshair(&self, time: f64) -> Option<Crosshair> {
        let chart = self.chart()?;
        let index = nearest_time(&chart.times, time)?;
        let x = *chart.xs.get(index)?;
        let (named, _) = self.named(&chart, index, READOUT_ROWS);
        Some(Crosshair {
            id: self.element_id("crosshair"),
            geometry: self.geometry.clone(),
            x,
            dots: self.dots(&chart, index, &named),
        })
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
        // As many rows as the plot is tall, so a short panel never cuts the
        // readout off.
        let room = (geometry.size.height - dp_px(CHROME, window)) / dp_px(ROW, window);
        let reach = dp_px(markers::REACH, window) / geometry.width;
        let marker = markers::nearest(&self.placed, x / geometry.width, reach);
        let fit = (room.max(0.) as usize).saturating_sub(usize::from(marker.is_some()));
        let mut cursor =
            index.and_then(|index| self.cursor_at(&chart, index, fit.clamp(1, READOUT_ROWS)));
        if let Some(cursor) = &mut cursor {
            cursor.marker = marker;
            cursor.least = least_width(cursor, window);
        }
        let key = |c: &Cursor| (c.index, c.marker);
        if self.cursor.as_ref().map(key) == cursor.as_ref().map(key) {
            return;
        }
        let time = cursor.as_ref().map(|cursor| chart.times[cursor.index]);
        self.cursor = cursor;
        self.show_cursor(cx);
        cx.emit(PanelEvent::Cursor(time));
    }

    fn pointer_left(&mut self, cx: &mut Context<Self>) {
        if self.cursor.take().is_some() {
            self.show_cursor(cx);
            cx.emit(PanelEvent::Cursor(None));
        }
    }

    /// The chart's own cursor, for the view that lays the panel out to draw
    /// beside it.
    pub fn cursor_overlay(&self) -> Option<Entity<CursorOverlay>> {
        self.overlay.clone()
    }

    /// A chart's overlay, made with its plot.
    pub(super) fn new_overlay(&self, cx: &mut Context<Self>) -> Entity<CursorOverlay> {
        let (id, geometry) = (self.id.clone(), self.geometry.clone());
        cx.new(|_| CursorOverlay::new(id, geometry))
    }

    /// Hands the cursor, the focus and the marker under the pointer to the
    /// overlay, which alone draws again.
    pub(super) fn show_cursor(&self, cx: &mut Context<Self>) {
        let Some(overlay) = &self.overlay else {
            return;
        };
        let shown = self
            .cursor
            .clone()
            .zip(self.chart())
            .map(|(cursor, chart)| {
                let focus = self.focus();
                let named: Vec<usize> = cursor.rows.iter().map(|row| row.series).collect();
                Shown {
                    x: chart.xs[cursor.index],
                    dots: self.dots(&chart, cursor.index, &named),
                    inks: named
                        .iter()
                        .map(|series| (chart.series[*series].ink, focus == Some(*series)))
                        .collect(),
                    marker: cursor
                        .marker
                        .and_then(|marker| self.placed.get(marker))
                        .cloned(),
                    cursor,
                }
            });
        overlay.update(cx, |overlay, cx| {
            overlay.shown = shown;
            cx.notify();
        });
    }

    pub(super) fn chart(&self) -> Option<std::rc::Rc<Chart>> {
        match &self.data.as_ref()?.body {
            derive::Body::Chart(chart) => Some(chart.clone()),
            _ => None,
        }
    }

    /// The cursor at sample `index`, naming at most `fit` series.
    fn cursor_at(&self, chart: &Chart, index: usize, fit: usize) -> Option<Cursor> {
        let geometry = self.geometry.get();
        if geometry.width <= px(0.) {
            return None;
        }
        let x = geometry.left + geometry.width * *chart.xs.get(index)?;
        let (shown, more) = self.named(chart, index, fit);
        let rows = shown.into_iter().map(|series| Row {
            series,
            value: derive::format(&chart.series[series].field, value(chart, series, index)).into(),
            name: chart.series[series].name.clone(),
        });
        // Measured from the plot's own rectangle, so a flipped readout never
        // covers the value axis.
        let flip = x > geometry.left + geometry.width / 2.;
        Some(Cursor {
            index,
            x,
            flip,
            room: if flip {
                x - geometry.left
            } else {
                geometry.left + geometry.width - x
            },
            width: geometry.size.width,
            least: px(0.),
            time: when(chart.times[index]).into(),
            rows: rows.collect(),
            more,
            marker: None,
        })
    }

    /// The series named at sample `index`, at most `fit`: all of them in
    /// legend order when they fit, else the highest values and the focused
    /// series. Then how many are left out.
    fn named(&self, chart: &Chart, index: usize, fit: usize) -> (Vec<usize>, usize) {
        let value = |series: usize| value(chart, series, index);
        let mut shown: Vec<usize> = (0..chart.series.len())
            .filter(|series| !chart.series[*series].unlisted)
            .collect();
        let more = shown.len().saturating_sub(fit);
        if more > 0 {
            // Highest first, a missing value last; the focused series keeps
            // the last row when it falls below.
            let focus = self.focus().filter(|series| shown.contains(series));
            shown.sort_by(|a, b| {
                let (a, b) = (value(*a), value(*b));
                a.is_nan().cmp(&b.is_nan()).then(b.total_cmp(&a))
            });
            shown.truncate(fit);
            if let Some(focus) = focus.filter(|focus| !shown.contains(focus)) {
                shown[fit - 1] = focus;
            }
        }
        (shown, more)
    }

    /// A dot on each line, or past the readout's rows on the lines it
    /// names and the focused one: hundreds of dots stacked on one
    /// crosshair say nothing and cost every frame of the hover.
    fn dots(&self, chart: &Chart, index: usize, named: &[usize]) -> Vec<Dot> {
        let few = chart.series.len() <= READOUT_ROWS;
        let focus = self.focus();
        chart
            .series
            .iter()
            .enumerate()
            .filter(|(series, _)| few || focus == Some(*series) || named.contains(series))
            .filter_map(|(series, line)| {
                let y = *line.tops.get(index)?;
                y.is_finite().then(|| (y, line.ink, focus == Some(series)))
            })
            .collect()
    }

    /// The hover handlers over the plot's container. The cursor itself is
    /// the [`CursorOverlay`]'s, so a move never draws the panel again.
    pub(super) fn render_cursor(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(self.element_id("cursor"))
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, window, cx| {
                view.pointer_moved(event.position, window, cx)
            }))
            .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                if !hovered {
                    view.pointer_left(cx)
                }
            }))
            .into_any_element()
    }
}

/// The width of the readout's time line and of its widest swatch and value
/// row, with its padding and border, as `render_readout` lays them out.
fn least_width(cursor: &Cursor, window: &Window) -> Pixels {
    let font = Font {
        family: ui::MONO_FONT.into(),
        ..window.text_style().font()
    };
    let measure = |text: &SharedString, size: f32, weight: FontWeight| {
        let run = TextRun {
            len: text.len(),
            font: Font {
                weight,
                ..font.clone()
            },
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(text.clone(), dp_px(size, window), &[run], None)
            .width()
    };
    let time = measure(&cursor.time, 11., FontWeight::NORMAL);
    let value = cursor
        .rows
        .iter()
        .map(|row| measure(&row.value, 12., FontWeight::SEMIBOLD))
        .fold(px(0.), Pixels::max);
    // Swatch, then a gap either side of the values.
    let row = dp_px(12. + 8. + 8., window) + value;
    time.max(row) + dp_px(2. * 10., window) + px(2.)
}

impl Render for CursorOverlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let origin = self.origin.clone();
        let overlay = div().absolute().top_0().left_0().size_full().child(
            canvas(
                move |bounds, _, _| origin.set(bounds.origin),
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
        let Some(shown) = &self.shown else {
            return overlay;
        };
        let geometry = self.geometry.clone();
        let g = geometry.get();
        let at = g.origin - self.origin.get();
        let (x, dots) = (shown.x, shown.dots.clone());
        overlay.child(
            div()
                .absolute()
                .left(at.x)
                .top(at.y)
                .w(g.size.width)
                .h(g.size.height)
                .child(
                    canvas(
                        |_, _, _| {},
                        move |_, _, window, cx| paint(geometry.get(), x, &dots, 0.7, window, cx),
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
                .when(!shown.cursor.rows.is_empty(), |this| {
                    this.child(self.render_readout(shown, window, cx))
                }),
        )
    }
}

impl CursorOverlay {
    /// The time, then a row a series in three aligned columns: swatch,
    /// value and name. When rows are left out, the time line says how many
    /// it ranks from.
    fn render_readout(&self, shown: &Shown, window: &Window, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let cursor = &shown.cursor;
        let gap = dp(12.);
        // Within the room on its side, less its gap to the crosshair; names
        // and the count truncate to fit, but the time and values stay whole.
        let widest = (cursor.room - dp_px(12., window)).max(cursor.least);
        let cell = || h_flex().h(dp(ROW)).flex_none();
        let column = |rows: Vec<AnyElement>| v_flex().flex_none().children(rows);
        let mut swatches = Vec::with_capacity(cursor.rows.len());
        let mut values = Vec::with_capacity(cursor.rows.len());
        let mut names = Vec::with_capacity(cursor.rows.len());
        let rows = cursor.rows.iter().zip(shown.colors(&p));
        for (n, (row, (color, focused))) in rows.enumerate() {
            swatches.push(
                cell()
                    .child(div().w(dp(12.)).h(px(2.)).rounded(px(3.)).bg(color))
                    .into_any_element(),
            );
            values.push(
                cell()
                    .justify_end()
                    .text_size(dp(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.ink)
                    .whitespace_nowrap()
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-readout-value-{n}", self.id)))
                            .child(row.value.clone())
                            .test_support(),
                    )
                    .into_any_element(),
            );
            names.push(
                cell()
                    .max_w(dp(240.))
                    .text_size(dp(11.))
                    .text_color(if focused { p.ink } else { p.muted })
                    .child(div().min_w_0().truncate().child(row.name.clone()))
                    .into_any_element(),
            );
        }
        v_flex()
            .id(SharedString::from(format!("{}-readout", self.id)))
            .absolute()
            .top(dp(10.))
            .map(|this| {
                if cursor.flip {
                    this.right(cursor.width - cursor.x).mr(gap)
                } else {
                    this.left(cursor.x).ml(gap)
                }
            })
            .max_w(widest)
            .overflow_hidden()
            .px(dp(10.))
            .py(dp(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(p.line_strong)
            .bg(p.hover)
            .shadow_lg()
            .font_family(ui::MONO_FONT)
            .child(
                h_flex()
                    .h(dp(ROW))
                    .mb(dp(4.))
                    .gap(dp(16.))
                    .justify_between()
                    .min_w_0()
                    .text_size(dp(11.))
                    .text_color(p.muted)
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-readout-time", self.id)))
                            .flex_none()
                            .child(cursor.time.clone())
                            .test_support(),
                    )
                    .when(cursor.more > 0, |this| {
                        // The count gives way to the time in a narrow plot.
                        this.child(
                            div()
                                .id(SharedString::from(format!("{}-readout-more", self.id)))
                                .min_w_0()
                                .truncate()
                                .child(format!(
                                    "top {} of {}",
                                    cursor.rows.len(),
                                    cursor.rows.len() + cursor.more
                                ))
                                .test_support(),
                        )
                    }),
            )
            .children(
                shown
                    .marker
                    .as_ref()
                    .map(|marker| markers::readout_row(marker, &p)),
            )
            .child(
                h_flex()
                    .items_start()
                    .gap(dp(8.))
                    .child(column(swatches))
                    .child(column(values))
                    .child(column(names).flex_shrink(1.).min_w_0()),
            )
            .test_support()
            .into_any_element()
    }
}

/// A crosshair at `x` across the plot `g` last painted, its line at
/// `opacity`, with a dot on each line in the theme's inks.
fn paint(g: Geometry, x: f32, dots: &[Dot], opacity: f32, window: &mut Window, cx: &App) {
    if g.width <= px(0.) {
        return;
    }
    let p = palette(cx);
    let (line, ring) = (p.ink_2.opacity(opacity), p.surface);
    let x = g.origin.x + g.left + g.width * x;
    let top = g.origin.y + g.top;
    window.paint_quad(fill(
        Bounds::new(point(x, top), size(px(1.), g.height)),
        line,
    ));
    let (outer, inner) = (dp_px(10., window), dp_px(7., window));
    for &(y, ink, focused) in dots {
        let center = point(x + px(0.5), top + g.height * (1. - y));
        let ring_bounds = Bounds::centered_at(center, size(outer, outer));
        window.paint_quad(fill(ring_bounds, ring).corner_radii(outer / 2.));
        let dot = Bounds::centered_at(center, size(inner, inner));
        window.paint_quad(fill(dot, ink.color(&p, focused)).corner_radii(inner / 2.));
    }
}

fn value(chart: &Chart, series: usize, index: usize) -> f64 {
    let line = &chart.series[series];
    line.values.get(index).copied().unwrap_or(f64::NAN)
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
