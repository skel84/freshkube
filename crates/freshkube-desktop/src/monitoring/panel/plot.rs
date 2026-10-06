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
    App, Bounds, Context, FillOptions, FillRule, Hsla, IntoElement, Path, PathBuilder, PathStyle,
    Pixels, Point, Render, SharedString, Size, TextAlign, TextRun, Window, canvas, div, fill,
    point, px, size,
};

use super::markers::{self, Placed};
use crate::monitoring::colors::{FADED_OPACITY, Tier};
use crate::monitoring::derive::{Axis, BarSlot, Chart, ChartSeries, fitting};
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
    markers: Rc<[Placed]>,
    /// Whether a path failed to build, so it is reported once.
    failed: Rc<Cell<bool>>,
}

impl PlotView {
    pub(super) fn new(id: SharedString, chart: Rc<Chart>, geometry: Rc<Cell<Geometry>>) -> Self {
        Self {
            id: format!("{id}-plot").into(),
            chart,
            revision: 0,
            focus: None,
            geometry,
            markers: Rc::from([]),
            failed: Rc::default(),
        }
    }

    pub(super) fn set_markers(&mut self, markers: Rc<[Placed]>, cx: &mut Context<Self>) {
        self.markers = markers;
        cx.notify();
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
            markers: self.markers.clone(),
            palette: palette(cx),
            failed: self.failed.clone(),
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
    markers: Rc<[Placed]>,
    palette: Palette,
    failed: Rc<Cell<bool>>,
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

/// How a group's line and area draw (`Chart::groups`).
struct Look<'a> {
    color: Hsla,
    fill: f32,
    line: bool,
    dashes: Option<&'a [f32]>,
}

impl<'a> Look<'a> {
    fn of(series: &'a ChartSeries, focused: bool) -> Self {
        Self {
            color: series.ink.color(focused),
            fill: series.fill,
            line: series.draw == DrawStyle::Line,
            dashes: series.dashes.as_deref(),
        }
    }
}

/// Where a shape keeps its path in the chart's [`PathCaches`].
#[derive(Clone, Copy)]
enum Slot {
    /// A bar series, by its index.
    Bar(usize),
    /// A group's area, and its line in the slot after.
    Group(usize),
    /// The focused series' area and line, drawn again on top.
    Focus(usize),
}

impl Slot {
    /// Bars by series, then two per group, then two per series for focus;
    /// `series` is how many the chart has, so no two shapes share a slot.
    fn index(self, series: usize) -> usize {
        match self {
            Slot::Bar(index) => index,
            Slot::Group(number) => series + number * 2,
            Slot::Focus(index) => series * 3 + index * 2,
        }
    }
}

impl Paint {
    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let _span = crate::perf::span("monitoring.plot_paint");
        #[cfg(test)]
        crate::desktop::probe::hit("monitoring-plot-paint");
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
        markers::paint(
            &self.markers,
            bounds.origin,
            (frame.left, frame.top, frame.width, frame.height),
            &self.palette,
            window,
        );
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
                    p.muted,
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
                p.muted,
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

    /// Bars first, then the lines and areas, then the dots on top.
    fn paint_series(
        &self,
        frame: &Frame,
        bounds: Bounds<Pixels>,
        caches: &mut PathCaches,
        window: &mut Window,
    ) {
        self.paint_bars(frame, bounds, caches, window);
        self.paint_groups(frame, bounds, caches, window);
        self.paint_dots(frame, bounds, window);
    }

    /// A cached shape's key: the chart's revision, its slot and its kind,
    /// and the plot's size.
    fn shape_key(&self, frame: &Frame, slot: usize, kind: u8) -> u64 {
        let mut key = ShapeKey::new((self.revision, slot, kind));
        key.f32(frame.width.into())
            .f32(frame.height.into())
            .f32(frame.left.into());
        key.finish()
    }

    /// Whether series `index` is focused, and how faded it draws.
    fn emphasis(&self, index: usize) -> (bool, f32) {
        let focused = self.focus == Some(index);
        let fade = if self.focus.is_some() && !focused {
            FADED_OPACITY
        } else {
            1.
        };
        (focused, fade)
    }

    /// Each bar series as one path, in its place in the column.
    fn paint_bars(
        &self,
        frame: &Frame,
        bounds: Bounds<Pixels>,
        caches: &mut PathCaches,
        window: &mut Window,
    ) {
        for (index, series) in self.chart.series.iter().enumerate().rev() {
            let Some(place) = series.bar else {
                continue;
            };
            let (focused, fade) = self.emphasis(index);
            let slot = Slot::Bar(index).index(self.chart.series.len());
            let path = caches.slot(slot).get(
                self.shape_key(frame, slot, 0),
                bounds.origin,
                built(|| self.reported(bars_path(series, &self.chart.xs, frame, place))),
            );
            if let Some(path) = path {
                crate::desktop::probe::hit("monitoring-path-painted");
                window.paint_path(path, series.ink.color(focused).opacity(0.7 * fade));
            }
        }
    }

    /// The groups of lines and areas, as one path each. While one series is
    /// focused every group fades and that series draws again on top, so
    /// the groups never change with focus and their paths are built once
    /// per answer.
    fn paint_groups(
        &self,
        frame: &Frame,
        bounds: Bounds<Pixels>,
        caches: &mut PathCaches,
        window: &mut Window,
    ) {
        let fade = if self.focus.is_some() {
            FADED_OPACITY
        } else {
            1.
        };
        for (number, members) in self.chart.groups.iter().enumerate() {
            let look = Look::of(&self.chart.series[members[0]], false);
            self.paint_shape(
                Slot::Group(number),
                &look,
                members,
                fade,
                frame,
                bounds,
                caches,
                window,
            );
        }
        let focused = self.focus.filter(|index| {
            self.chart
                .series
                .get(*index)
                .is_some_and(|series| series.bar.is_none())
        });
        if let Some(index) = focused {
            let look = Look::of(&self.chart.series[index], true);
            self.paint_shape(
                Slot::Focus(index),
                &look,
                &[index],
                1.,
                frame,
                bounds,
                caches,
                window,
            );
        }
    }

    /// One look's area and line over `members`, built only when its slot
    /// holds no path for this answer and size.
    #[allow(clippy::too_many_arguments)]
    fn paint_shape(
        &self,
        slot: Slot,
        look: &Look,
        members: &[usize],
        fade: f32,
        frame: &Frame,
        bounds: Bounds<Pixels>,
        caches: &mut PathCaches,
        window: &mut Window,
    ) {
        let slot = slot.index(self.chart.series.len());
        let members =
            || -> Vec<&ChartSeries> { members.iter().map(|&i| &self.chart.series[i]).collect() };
        if look.fill > 0. {
            let path = caches.slot(slot).get(
                self.shape_key(frame, slot, 1),
                bounds.origin,
                built(|| {
                    self.reported(area_path(
                        &members(),
                        &self.chart.xs,
                        self.chart.curve,
                        frame,
                    ))
                }),
            );
            if let Some(path) = path {
                crate::desktop::probe::hit("monitoring-path-painted");
                window.paint_path(path, look.color.opacity(look.fill * fade));
            }
        }
        if look.line {
            let path = caches.slot(slot + 1).get(
                self.shape_key(frame, slot + 1, 2),
                bounds.origin,
                built(|| {
                    self.reported(line_path(
                        &members(),
                        look.dashes,
                        &self.chart.xs,
                        self.chart.curve,
                        frame,
                    ))
                }),
            );
            if let Some(path) = path {
                crate::desktop::probe::hit("monitoring-path-painted");
                window.paint_path(path, look.color.opacity(fade));
            }
        }
    }

    /// A series' points, or the dot that ends a line short of the window's
    /// end; after every path, so they don't split the paths' batches.
    fn paint_dots(&self, frame: &Frame, bounds: Bounds<Pixels>, window: &mut Window) {
        let surface = self.palette.surface;
        let mut dots = Vec::new();
        for (index, series) in self.chart.series.iter().enumerate().rev() {
            if series.bar.is_some() {
                continue;
            }
            let (focused, fade) = self.emphasis(index);
            let color = series.ink.color(focused);
            if series.points || series.draw == DrawStyle::Points || self.chart.xs.len() == 1 {
                let r = px(if focused { 3. } else { 2.5 });
                for (x, y) in self.chart.xs.iter().zip(&series.tops) {
                    if y.is_finite() {
                        let center = bounds.origin + point(frame.x(*x), frame.y(*y));
                        dots.push((center, r * 2., None, color.opacity(fade)));
                    }
                }
            } else if let Some(last) = early_end(&series.tops) {
                // A line that stops before the window's end ends in a dot.
                let center =
                    bounds.origin + point(frame.x(self.chart.xs[last]), frame.y(series.tops[last]));
                dots.push((
                    center,
                    dp_px(7., window),
                    Some(dp_px(10., window)),
                    color.opacity(fade),
                ));
            }
        }
        for (center, diameter, ring, color) in dots {
            if let Some(ring) = ring {
                window.paint_quad(
                    fill(Bounds::centered_at(center, size(ring, ring)), surface)
                        .corner_radii(ring / 2.),
                );
            }
            window.paint_quad(
                fill(Bounds::centered_at(center, size(diameter, diameter)), color)
                    .corner_radii(diameter / 2.),
            );
        }
    }

    /// A path, or nothing when it couldn't be built: said once per chart on
    /// stderr, and a failure in debug builds, so a limit never hides lines
    /// silently again.
    fn reported(&self, built: Built) -> Option<Path<Pixels>> {
        built.unwrap_or_else(|error| {
            if !self.failed.replace(true) {
                eprintln!("{}: a path failed to build: {error}", self.id);
            }
            debug_assert!(false, "{}: a path failed to build: {error}", self.id);
            None
        })
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
    members: &[&ChartSeries],
    dashes: Option<&[f32]>,
    xs: &[f32],
    curve: Curve,
    frame: &Frame,
) -> Built {
    joined(members, &|members| {
        line_chunk(members, dashes, xs, curve, frame)
    })
}

fn line_chunk(
    members: &[&ChartSeries],
    dashes: Option<&[f32]>,
    xs: &[f32],
    curve: Curve,
    frame: &Frame,
) -> Built {
    let mut path = PathBuilder::stroke(px(if dashes.is_some() { 1.5 } else { 2. }));
    if let Some(dashes) = dashes {
        let dashes: Vec<Pixels> = dashes.iter().map(|d| px(*d)).collect();
        path = path.dash_array(&dashes);
    }
    let mut any = false;
    for series in members {
        for run in runs(xs, &series.tops, frame) {
            if run.len() < 2 {
                continue;
            }
            trace(&mut path, &run, curve, true);
            any = true;
        }
    }
    finished(any, path)
}

/// The area under a line, down to its baseline or, stacked, to the series
/// below.
/// Built whole where it can be: GPUI draws a path's triangles over each
/// other, so where two chunks' areas overlap they would fill twice.
fn area_path(members: &[&ChartSeries], xs: &[f32], curve: Curve, frame: &Frame) -> Built {
    halving(members, &|members| area_chunk(members, xs, curve, frame))
}

fn area_chunk(members: &[&ChartSeries], xs: &[f32], curve: Curve, frame: &Frame) -> Built {
    let mut path = area_builder();
    let mut any = false;
    for series in members {
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
    }
    finished(any, path)
}

/// A path, none when there was nothing to draw, or why it couldn't be built.
type Built = Result<Option<Path<Pixels>>, String>;

fn finished(any: bool, path: PathBuilder) -> Built {
    if !any {
        return Ok(None);
    }
    path.build().map(Some).map_err(|error| error.to_string())
}

/// About how many samples one path traces first. GPUI tessellates a path
/// with 16-bit indices, so one holds at most 65,536 vertices (K25 in
/// docs/GPUI_FRICTION.md). A sample takes from 2 on a flat straight line to
/// about 56 on a smooth one that jumps 100 px at every sample; ordinary
/// charts fit this many, and a chunk that doesn't is halved.
const SAMPLES_PER_PATH: usize = 4_000;

/// A group's lines built a chunk of series at a time, each under GPUI's
/// vertex limit, and joined into one path, so the group still draws in one
/// pass. A chunk that still fails is split in half until it builds.
fn joined(members: &[&ChartSeries], build: &impl Fn(&[&ChartSeries]) -> Built) -> Built {
    let mut path = None;
    let mut start = 0;
    let mut samples = 0;
    for (index, series) in members.iter().enumerate() {
        let count = series.tops.iter().filter(|y| y.is_finite()).count();
        if index > start && samples + count > SAMPLES_PER_PATH {
            join(&mut path, halving(&members[start..index], build)?);
            (start, samples) = (index, 0);
        }
        samples += count;
    }
    join(&mut path, halving(&members[start..], build)?);
    Ok(path)
}

/// `members`' shape, split in half until each half builds.
fn halving(members: &[&ChartSeries], build: &impl Fn(&[&ChartSeries]) -> Built) -> Built {
    match build(members) {
        Err(_) if members.len() > 1 => {
            let (first, rest) = members.split_at(members.len() / 2);
            let mut path = halving(first, build)?;
            join(&mut path, halving(rest, build)?);
            Ok(path)
        }
        built => built,
    }
}

/// Adds `more`'s triangles to `path`: a built path is a list of triangles,
/// so two drawn as one fill the same pixels.
fn join(path: &mut Option<Path<Pixels>>, more: Option<Path<Pixels>>) {
    let Some(more) = more.filter(|more| !more.vertices.is_empty()) else {
        return;
    };
    match path {
        Some(path) => {
            path.bounds = path.bounds.union(&more.bounds);
            path.vertices.extend(more.vertices);
        }
        None => *path = Some(more),
    }
}

/// A fill that draws overlapping areas once. Even-odd, the default, would cut
/// a hole wherever two of a group's areas overlap.
fn area_builder() -> PathBuilder {
    PathBuilder::fill().with_style(PathStyle::Fill(
        FillOptions::default().with_fill_rule(FillRule::NonZero),
    ))
}

/// One bar per sample, in the series' place in its column.
fn bars_path(series: &ChartSeries, xs: &[f32], frame: &Frame, place: BarSlot) -> Built {
    let bases = series.bases.as_deref();
    let mut path = PathBuilder::fill();
    let mut any = false;
    for bar in bars(&series.tops, bases, series.baseline, xs, frame, place) {
        let (min, max) = (bar.origin, bar.bottom_right());
        path.add_polygon(&[min, point(max.x, min.y), max, point(min.x, max.y)], true);
        any = true;
    }
    finished(any, path)
}

/// Where each sample's bar stands: a slice of its column, a pixel short of
/// the next one's when there is room, and never under a pixel wide or tall.
/// A sample that reaches no height from its base has no bar, so a zero in a
/// stack doesn't paint the edge of the bar below it.
fn bars(
    tops: &[f32],
    bases: Option<&[f32]>,
    baseline: f32,
    xs: &[f32],
    frame: &Frame,
    place: BarSlot,
) -> Vec<Bounds<Pixels>> {
    let step = xs
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|step| *step > 0.)
        .fold(f32::INFINITY, f32::min);
    let step = if step.is_finite() { step } else { 0.05 };
    let group = frame.width * step * 0.8;
    let slice = group / place.count.max(1) as f32;
    let width = (slice - px(1.)).max(px(1.));
    xs.iter()
        .zip(tops)
        .enumerate()
        .filter_map(|(index, (x, top))| {
            let base = bases
                .and_then(|bases| bases.get(index).copied())
                .filter(|base| base.is_finite())
                .unwrap_or(baseline);
            if !top.is_finite() || *top == base {
                return None;
            }
            let left = frame.x(*x) - group / 2. + slice * place.index as f32;
            let (y1, y2) = (frame.y(*top), frame.y(base));
            let (y1, y2) = (y1.min(y2), y1.max(y2));
            let height = (y2 - y1).max(px(1.));
            Some(Bounds::new(point(left, y1), size(width, height)))
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Two overlapping areas in one path cover their union, with no hole
    /// where they overlap.
    #[test]
    fn areas_in_one_path_fill_their_union() {
        let mut path = area_builder();
        for (left, right) in [(0., 60.), (40., 100.)] {
            path.move_to(point(px(left), px(0.)));
            path.line_to(point(px(right), px(0.)));
            path.line_to(point(px(right), px(10.)));
            path.line_to(point(px(left), px(10.)));
            path.close();
        }
        let path = path.build().unwrap();
        let covered: f32 = path
            .vertices
            .chunks(3)
            .map(|triangle| {
                let [a, b, c] = [0, 1, 2].map(|i| triangle[i].xy_position);
                (f32::from(b.x - a.x) * f32::from(c.y - a.y)
                    - f32::from(c.x - a.x) * f32::from(b.y - a.y))
                .abs()
                    / 2.
            })
            .sum();
        assert!((covered - 1000.).abs() < 1., "covered {covered}");
    }

    fn frame(width: f32) -> Frame {
        Frame {
            left: px(0.),
            top: px(0.),
            width: px(width),
            height: px(100.),
        }
    }

    /// A zero in a stack has no bar, so it can't tint the top of the bar
    /// below; a small value still shows as a pixel.
    #[test]
    fn a_zero_in_a_stack_draws_no_bar() {
        let xs = [0.25, 0.75];
        let place = BarSlot { index: 0, count: 1 };
        let below = [0.5, 0.5];
        let zero = bars(&below, Some(&below), 0., &xs, &frame(200.), place);
        assert!(zero.is_empty(), "{zero:?}");
        let small = bars(&[0.501, 0.5], Some(&below), 0., &xs, &frame(200.), place);
        assert_eq!(small.len(), 1);
        assert_eq!(small[0].size.height, px(1.));
        assert!((f32::from(small[0].origin.y) - 49.9).abs() < 0.01);
        let unstacked = bars(&[0., 0.3], None, 0., &xs, &frame(200.), place);
        assert_eq!(unstacked.len(), 1);
        assert!((f32::from(unstacked[0].size.height) - 30.).abs() < 0.01);
    }

    /// Bars a pixel short of the next slice, but never narrower than a pixel,
    /// so a dense chart keeps every bar.
    #[test]
    fn bars_stay_a_pixel_wide_however_dense() {
        let xs: Vec<f32> = (0..100).map(|i| i as f32 / 100.).collect();
        let tops = vec![0.5; xs.len()];
        let roomy = bars(
            &tops,
            None,
            0.,
            &xs,
            &frame(1000.),
            BarSlot { index: 0, count: 1 },
        );
        assert!((f32::from(roomy[0].size.width) - 7.).abs() < 0.01);
        for count in [1, 3] {
            let place = BarSlot {
                index: count - 1,
                count,
            };
            let dense = bars(&tops, None, 0., &xs, &frame(100.), place);
            assert_eq!(dense.len(), xs.len());
            assert!(dense.iter().all(|bar| bar.size.width == px(1.)), "{count}");
        }
    }
}
