//! A timeseries plot, painted from a derived [`Chart`]. It is its own view
//! so the panel can draw it cached: a moving cursor redraws only the
//! overlay above it, and a new answer or a focused series redraws the plot.
//!
//! Paths are built at a zero origin and kept in [`PathCaches`] by the
//! chart's revision and the plot's size, so a repaint at the same size only
//! moves them.
use std::cell::Cell;
use std::rc::Rc;

use freshkube_core::monitoring::model::spec::{Curve, DrawStyle};
use gpui_kit::component::plot::{PathCaches, ShapeKey};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Bounds, Context, Hsla, IntoElement, Path, PathBuilder, Pixels, Point, Render,
    SharedString, Size, TextAlign, TextRun, Window, canvas, div, fill, point, px, size,
};

use crate::monitoring::colors::{FADED_OPACITY, Tier};
use crate::monitoring::derive::{Axis, Chart, ChartSeries, fitting};
use crate::palette::{Palette, palette};
use crate::ui::{self, dp_px};

/// Where the plot's inner rectangle sits in its container, shared with the
/// panel's cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Geometry {
    /// The container's origin in the window and its size.
    pub origin: Point<Pixels>,
    pub size: Size<Pixels>,
    /// The inner rectangle, from the container's origin.
    pub left: Pixels,
    pub top: Pixels,
    pub width: Pixels,
    pub height: Pixels,
}

pub(crate) struct PlotView {
    id: SharedString,
    chart: Rc<Chart>,
    /// Bumped with each chart, so cached paths are built again.
    revision: u64,
    pub(super) focus: Option<usize>,
    geometry: Rc<Cell<Geometry>>,
}

impl PlotView {
    pub(super) fn new(id: SharedString, chart: Rc<Chart>, geometry: Rc<Cell<Geometry>>) -> Self {
        Self {
            id: format!("{id}-plot").into(),
            chart,
            revision: 0,
            focus: None,
            geometry,
        }
    }

    pub(super) fn set_chart(&mut self, chart: Rc<Chart>, cx: &mut Context<Self>) {
        self.chart = chart;
        self.revision += 1;
        cx.notify();
    }

    pub(super) fn set_focus(&mut self, focus: Option<usize>, cx: &mut Context<Self>) {
        if self.focus != focus {
            self.focus = focus;
            cx.notify();
        }
    }
}

impl Render for PlotView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        crate::desktop::probe::hit("monitoring-plot");
        let paint = Paint {
            id: self.id.clone(),
            chart: self.chart.clone(),
            revision: self.revision,
            focus: self.focus,
            geometry: self.geometry.clone(),
            palette: palette(cx),
        };
        div()
            .id(self.id.clone())
            .size_full()
            .font_family(ui::MONO_FONT)
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| paint.paint(bounds, window, cx),
                )
                .size_full(),
            )
    }
}

/// Everything the canvas needs, copied out of the view.
struct Paint {
    id: SharedString,
    chart: Rc<Chart>,
    revision: u64,
    focus: Option<usize>,
    geometry: Rc<Cell<Geometry>>,
    palette: Palette,
}

/// The inner rectangle at a zero origin, and what projects onto it.
struct Frame {
    left: Pixels,
    top: Pixels,
    width: Pixels,
    height: Pixels,
}

impl Frame {
    fn x(&self, fraction: f32) -> Pixels {
        self.left + self.width * fraction
    }

    fn y(&self, fraction: f32) -> Pixels {
        self.top + self.height * (1. - fraction)
    }

    fn bottom(&self) -> Pixels {
        self.top + self.height
    }

    fn right(&self) -> Pixels {
        self.left + self.width
    }
}

impl Paint {
    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let label = dp_px(10.5, window);
        let gap = dp_px(6., window);
        let gutter = |axis: &Option<Axis>, window: &mut Window| {
            axis.as_ref().map(|axis| {
                let widest = axis
                    .ticks
                    .iter()
                    .map(|tick| text_width(&tick.label, label, window))
                    .fold(px(0.), Pixels::max);
                (widest + gap).max(dp_px(24., window))
            })
        };
        let left = gutter(&self.chart.axes[0], window).unwrap_or(dp_px(8., window));
        let right = gutter(&self.chart.axes[1], window).unwrap_or(dp_px(8., window));
        let (top, bottom) = (dp_px(8., window), dp_px(20., window));
        let frame = Frame {
            left,
            top,
            width: (bounds.size.width - left - right).max(px(1.)),
            height: (bounds.size.height - top - bottom).max(px(1.)),
        };
        self.geometry.set(Geometry {
            origin: bounds.origin,
            size: bounds.size,
            left: frame.left,
            top: frame.top,
            width: frame.width,
            height: frame.height,
        });

        self.paint_grid(&frame, bounds.origin, label, gap, window, cx);
        self.paint_bands(&frame, bounds.origin, window);
        let inner = Bounds::new(
            bounds.origin + point(frame.left, frame.top - px(2.)),
            size(frame.width, frame.height + px(4.)),
        );
        let caches =
            PathCaches::for_paint(SharedString::from(format!("{}-paths", self.id)), window, cx);
        window.with_content_mask(Some(gpui_kit::ContentMask { bounds: inner }), |window| {
            caches.update(cx, |caches, _| {
                self.paint_series(&frame, bounds, caches, window);
            })
        });
        self.paint_thresholds(&frame, bounds.origin, label, window, cx);
    }

    /// Gridlines and labels for the value axes, and the clock along the
    /// bottom.
    fn paint_grid(
        &self,
        frame: &Frame,
        origin: Point<Pixels>,
        label: Pixels,
        gap: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) {
        let p = &self.palette;
        let line_height = label * 1.2;
        let grid_axis = self.chart.axes[0].as_ref().or(self.chart.axes[1].as_ref());
        for (side, axis) in self.chart.axes.iter().enumerate() {
            let Some(axis) = axis else { continue };
            for tick in &axis.ticks {
                let y = frame.y(tick.at);
                if grid_axis.is_some_and(|grid| std::ptr::eq(grid, axis)) {
                    window.paint_quad(fill(
                        Bounds::new(
                            origin + point(frame.left, y.floor()),
                            size(frame.width, px(1.)),
                        ),
                        p.line,
                    ));
                }
                let (x, align) = if side == 0 {
                    (frame.left - gap, TextAlign::Right)
                } else {
                    (frame.right() + gap, TextAlign::Left)
                };
                paint_text(
                    &tick.label,
                    origin + point(x, y - line_height / 2.),
                    align,
                    label,
                    p.faint,
                    window,
                    cx,
                );
            }
        }
        let width = f32::from(frame.width);
        let Some(set) = fitting(&self.chart.time_ticks, width, f32::from(label) * 6.) else {
            return;
        };
        for tick in &set.ticks {
            if !(0.03..=0.97).contains(&tick.at) {
                continue;
            }
            paint_text(
                &tick.label,
                origin + point(frame.x(tick.at), frame.bottom() + label * 0.6),
                TextAlign::Center,
                label,
                p.faint,
                window,
                cx,
            );
        }
    }

    fn paint_bands(&self, frame: &Frame, origin: Point<Pixels>, window: &mut Window) {
        for band in &self.chart.bands {
            let (low, high) = (band.from.clamp(0., 1.), band.to.clamp(0., 1.));
            if high <= low {
                continue;
            }
            let top = frame.y(high);
            window.paint_quad(fill(
                Bounds::new(
                    origin + point(frame.left, top),
                    size(frame.width, frame.y(low) - top),
                ),
                self.tier_color(band.tier).opacity(0.08),
            ));
        }
    }

    fn paint_thresholds(
        &self,
        frame: &Frame,
        origin: Point<Pixels>,
        label: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) {
        let p = &self.palette;
        for threshold in &self.chart.thresholds {
            if !(0.0..=1.0).contains(&threshold.at) {
                continue;
            }
            let y = frame.y(threshold.at).floor() + px(0.5);
            let mut line = PathBuilder::stroke(px(1.)).dash_array(&[px(4.), px(3.)]);
            line.move_to(origin + point(frame.left, y));
            line.line_to(origin + point(frame.right(), y));
            if let Ok(path) = line.build() {
                window.paint_path(path, self.tier_color(threshold.tier).opacity(0.8));
            }
            let (x, align) = if threshold.right {
                (frame.right() - px(4.), TextAlign::Right)
            } else {
                (frame.left + px(4.), TextAlign::Left)
            };
            paint_text(
                &threshold.label,
                origin + point(x, y - label * 1.4),
                align,
                label,
                p.ink_2,
                window,
                cx,
            );
        }
    }

    fn paint_series(
        &self,
        frame: &Frame,
        bounds: Bounds<Pixels>,
        caches: &mut PathCaches,
        window: &mut Window,
    ) {
        let surface = self.palette.surface;
        let bars = self
            .chart
            .series
            .iter()
            .filter(|s| s.draw == DrawStyle::Bars)
            .count();
        let mut bar_slot = 0;
        // The first series draws last, on top, as the legend lists it.
        for (index, series) in self.chart.series.iter().enumerate().rev() {
            let focused = self.focus == Some(index);
            let fade = if self.focus.is_some() && !focused {
                FADED_OPACITY
            } else {
                1.
            };
            let color = series.ink.color(focused);
            let key = |part: u8| {
                let mut key = ShapeKey::new((self.revision, index, part));
                key.f32(frame.width.into())
                    .f32(frame.height.into())
                    .f32(frame.left.into());
                key.finish()
            };
            if series.draw == DrawStyle::Bars {
                let slot = bar_slot;
                bar_slot += 1;
                let path = caches.slot(index * 4).get(
                    key(0),
                    bounds.origin,
                    built(|| bars_path(series, &self.chart.xs, frame, slot, bars)),
                );
                if let Some(path) = path {
                    window.paint_path(path, color.opacity(0.7 * fade));
                }
                continue;
            }
            if series.fill > 0. {
                let path = caches.slot(index * 4 + 1).get(
                    key(1),
                    bounds.origin,
                    built(|| area_path(series, &self.chart.xs, self.chart.curve, frame)),
                );
                if let Some(path) = path {
                    window.paint_path(path, color.opacity(series.fill * fade));
                }
            }
            if series.draw == DrawStyle::Line {
                let path = caches.slot(index * 4 + 2).get(
                    key(2),
                    bounds.origin,
                    built(|| line_path(series, &self.chart.xs, self.chart.curve, frame)),
                );
                if let Some(path) = path {
                    window.paint_path(path, color.opacity(fade));
                }
            }
            if series.points || series.draw == DrawStyle::Points || self.chart.xs.len() == 1 {
                let r = px(if focused { 3. } else { 2.5 });
                for (x, y) in self.chart.xs.iter().zip(&series.tops) {
                    if y.is_finite() {
                        let center = bounds.origin + point(frame.x(*x), frame.y(*y));
                        window.paint_quad(
                            fill(
                                Bounds::centered_at(center, size(r * 2., r * 2.)),
                                color.opacity(fade),
                            )
                            .corner_radii(r),
                        );
                    }
                }
            } else if let Some(last) = early_end(&series.tops) {
                // A line that stops before the window's end ends in a dot.
                let center =
                    bounds.origin + point(frame.x(self.chart.xs[last]), frame.y(series.tops[last]));
                let (outer, inner) = (dp_px(10., window), dp_px(7., window));
                window.paint_quad(
                    fill(Bounds::centered_at(center, size(outer, outer)), surface)
                        .corner_radii(outer / 2.),
                );
                window.paint_quad(
                    fill(
                        Bounds::centered_at(center, size(inner, inner)),
                        color.opacity(fade),
                    )
                    .corner_radii(inner / 2.),
                );
            }
        }
    }

    fn tier_color(&self, tier: Tier) -> Hsla {
        match tier {
            Tier::Warn => self.palette.warn,
            Tier::Crit => self.palette.crit,
        }
    }
}

/// The last drawn sample of a line that ends before the final sample.
fn early_end(tops: &[f32]) -> Option<usize> {
    let last = tops.iter().rposition(|y| y.is_finite())?;
    (last + 1 < tops.len() && last > 0).then_some(last)
}

/// A shape's builder, counted in tests: a cached shape isn't built again
/// when only the cursor or the focus changes.
fn built<T>(build: impl FnOnce() -> T) -> impl FnOnce() -> T {
    move || {
        crate::desktop::probe::hit("monitoring-path");
        build()
    }
}

/// The runs of a series between gaps, as points at a zero origin.
fn runs<'a>(
    xs: &'a [f32],
    ys: &'a [f32],
    frame: &'a Frame,
) -> impl Iterator<Item = Vec<(usize, Point<Pixels>)>> + 'a {
    let mut index = 0;
    std::iter::from_fn(move || {
        while index < ys.len() && !ys[index].is_finite() {
            index += 1;
        }
        if index >= ys.len() {
            return None;
        }
        let mut run = Vec::new();
        while index < ys.len() && ys[index].is_finite() {
            run.push((index, point(frame.x(xs[index]), frame.y(ys[index]))));
            index += 1;
        }
        Some(run)
    })
}

/// Lays `run` into `path` with the panel's curve. `start` begins a new
/// subpath; otherwise the run continues the current one.
fn trace(path: &mut PathBuilder, run: &[(usize, Point<Pixels>)], curve: Curve, start: bool) {
    let points: Vec<Point<Pixels>> = run.iter().map(|(_, point)| *point).collect();
    let Some(first) = points.first() else { return };
    if start {
        path.move_to(*first);
    } else {
        path.line_to(*first);
    }
    match curve {
        Curve::Linear => points[1..].iter().for_each(|point| path.line_to(*point)),
        // A step holds each value until the next sample. Traced backwards,
        // as an area's lower edge is, it turns first.
        Curve::Step => {
            for pair in points.windows(2) {
                let corner = if pair[1].x >= pair[0].x {
                    point(pair[1].x, pair[0].y)
                } else {
                    point(pair[0].x, pair[1].y)
                };
                path.line_to(corner);
                path.line_to(pair[1]);
            }
        }
        Curve::Smooth => {
            // Catmull-Rom as cubic Béziers, which passes through every sample.
            for i in 0..points.len().saturating_sub(1) {
                let p0 = points[i.saturating_sub(1)];
                let (p1, p2) = (points[i], points[i + 1]);
                let p3 = points[(i + 2).min(points.len() - 1)];
                let c1 = point(p1.x + (p2.x - p0.x) / 6., p1.y + (p2.y - p0.y) / 6.);
                let c2 = point(p2.x - (p3.x - p1.x) / 6., p2.y - (p3.y - p1.y) / 6.);
                path.cubic_bezier_to(p2, c1, c2);
            }
        }
    }
}

fn line_path(
    series: &ChartSeries,
    xs: &[f32],
    curve: Curve,
    frame: &Frame,
) -> Option<Path<Pixels>> {
    let mut path = PathBuilder::stroke(px(if series.dashes.is_some() { 1.5 } else { 2. }));
    if let Some(dashes) = &series.dashes {
        let dashes: Vec<Pixels> = dashes.iter().map(|d| px(*d)).collect();
        path = path.dash_array(&dashes);
    }
    let mut any = false;
    for run in runs(xs, &series.tops, frame) {
        if run.len() < 2 {
            continue;
        }
        trace(&mut path, &run, curve, true);
        any = true;
    }
    any.then(|| path.build().ok()).flatten()
}

/// The area under a line, down to its baseline or, stacked, to the series
/// below.
fn area_path(
    series: &ChartSeries,
    xs: &[f32],
    curve: Curve,
    frame: &Frame,
) -> Option<Path<Pixels>> {
    let mut path = PathBuilder::fill();
    let mut any = false;
    for run in runs(xs, &series.tops, frame) {
        if run.len() < 2 {
            continue;
        }
        trace(&mut path, &run, curve, true);
        let lower: Vec<(usize, Point<Pixels>)> = run
            .iter()
            .rev()
            .map(|(index, top)| {
                let base = series
                    .bases
                    .as_ref()
                    .and_then(|bases| bases.get(*index).copied())
                    .filter(|base| base.is_finite())
                    .unwrap_or(series.baseline);
                (*index, point(top.x, frame.y(base.clamp(-0.05, 1.05))))
            })
            .collect();
        let edge = if series.bases.is_some() {
            curve
        } else {
            Curve::Linear
        };
        trace(&mut path, &lower, edge, false);
        path.close();
        any = true;
    }
    any.then(|| path.build().ok()).flatten()
}

/// One bar per sample, side by side with the panel's other bar series.
fn bars_path(
    series: &ChartSeries,
    xs: &[f32],
    frame: &Frame,
    slot: usize,
    count: usize,
) -> Option<Path<Pixels>> {
    let step = xs
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|step| *step > 0.)
        .fold(f32::INFINITY, f32::min);
    let step = if step.is_finite() { step } else { 0.05 };
    let group = frame.width * step * 0.8;
    let width = (group / count.max(1) as f32).max(px(1.));
    let mut path = PathBuilder::fill();
    let mut any = false;
    for (index, (x, top)) in xs.iter().zip(&series.tops).enumerate() {
        if !top.is_finite() {
            continue;
        }
        let base = series
            .bases
            .as_ref()
            .and_then(|bases| bases.get(index).copied())
            .filter(|base| base.is_finite())
            .unwrap_or(series.baseline);
        let left = frame.x(*x) - group / 2. + width * slot as f32;
        let (y1, y2) = (frame.y(*top), frame.y(base));
        let (y1, y2) = (y1.min(y2), y1.max(y2).max(y1.min(y2) + px(1.)));
        path.add_polygon(
            &[
                point(left, y1),
                point(left + width - px(1.), y1),
                point(left + width - px(1.), y2),
                point(left, y2),
            ],
            true,
        );
        any = true;
    }
    any.then(|| path.build().ok()).flatten()
}

fn text_width(text: &SharedString, font_size: Pixels, window: &mut Window) -> Pixels {
    px(gpui_kit::component::plot::label::measure_text_width(
        text, font_size, window,
    ))
}

/// A one-line label: `origin` is its top edge, at the left, centre or right
/// as `align` says.
fn paint_text(
    text: &SharedString,
    origin: Point<Pixels>,
    align: TextAlign,
    font_size: Pixels,
    color: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    if text.is_empty() {
        return;
    }
    let run = TextRun {
        len: text.len(),
        font: window.text_style().font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(text.clone(), font_size, &[run], None);
    let x = match align {
        TextAlign::Left => origin.x,
        TextAlign::Center => origin.x - line.width() / 2.,
        TextAlign::Right => origin.x - line.width(),
    };
    let _ = line.paint(
        point(x, origin.y),
        font_size * 1.2,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}
