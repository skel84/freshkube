//! The loading state every table shares: skeleton rows at the row height,
//! one bar per column, under the table's real header, in two halves.
//!
//! The table draws the bars still: a source returns its [`LoadingRows`]
//! from [`TableSource::loading`](super::TableSource::loading) until its
//! first answer, and the table paints them and records where. Their motion
//! is a [`LoadingMotion`], the table's sibling drawn over it, as the change
//! flash is: a frame redraws the view that asked for it and every view
//! around it, so motion inside the table would redraw the table on every
//! frame. The motion dims the bars (Pulse) or sweeps a light band across
//! them (Shimmer) where the table's last frame put them, on the executor's
//! clock, and under reduced motion draws nothing and asks no frames. It
//! paints where the table recorded, in window coordinates, so it may sit
//! anywhere after the table: the app's shell draws the shown page's motion
//! beside the page, which stays cached.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, AppContext, Bounds, ContentMask, Context, Corners, Entity, EntityId, Hsla,
    Pixels, Role, SharedString, TestSupportExt, Window, canvas, div, fill, linear_color_stop,
    linear_gradient, point, px, size, white,
};

use super::{CELL_PAD, GLYPH_WIDTH, ROW_HEIGHT, TableColumn};
use crate::motion;
use crate::ui::dp_px;

/// How the bars move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// Every bar dims and comes back together.
    Pulse,
    /// A light band crosses the rows from the left.
    Shimmer,
}

/// The rows a loading table shows; the list clips what doesn't fit.
pub const LOADING_ROWS: usize = 14;
/// A bar's height in a row.
const BAR: f32 = 8.;
/// A glyph column's dot.
const DOT: f32 = BAR + 2.;
/// A bar's corners, in px as every radius is.
const RADIUS: f32 = 5.;
/// The shimmer's band, in dp.
const BAND: f32 = 120.;
/// How far each row's band trails the one above, so the sweep leans.
const SLANT: f32 = 6.;
/// How much of a cell each row's bar fills, by row and column, so the
/// rows read as text rather than a grid.
const FILL: [f32; 7] = [0.72, 0.48, 0.86, 0.6, 0.78, 0.54, 0.66];

/// One bar as the table last painted it.
#[derive(Clone, Copy)]
struct Bar {
    bounds: Bounds<Pixels>,
    radius: Pixels,
    row: usize,
}

/// Where the table last painted its loading bars, which the motion reads.
#[derive(Default)]
struct Painted {
    /// Whether a table draws these rows now: set when it renders them and
    /// cleared, with the bars, when it renders without them.
    showing: bool,
    /// The motion's view, which the table wakes when it starts showing the
    /// rows: a cached table renders after the views beside it, so the
    /// motion's first render can come before the rows show.
    motion: Option<EntityId>,
    bars: Vec<Bar>,
    /// The rows' region and the part of it that shows.
    region: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
    /// What the bars sit on, which the pulse dims them towards.
    fill: Hsla,
}

/// The still half: the bars a table draws while it loads, with the id
/// `<prefix>-loading`. A glyph column (unlabelled, [`GLYPH_WIDTH`] wide)
/// shows a dot, the others a bar.
#[derive(Clone)]
pub struct LoadingRows {
    id: SharedString,
    painted: Rc<RefCell<Painted>>,
}

impl LoadingRows {
    pub fn new(prefix: &str) -> Self {
        Self {
            id: format!("{prefix}-loading").into(),
            painted: Rc::default(),
        }
    }

    /// The moving half, to draw after the table over it. Mount it while
    /// the table shows these rows: it asks for every frame while it shows.
    pub fn motion(&self, look: Look) -> LoadingMotion {
        LoadingMotion {
            id: format!("{}-motion", self.id).into(),
            painted: self.painted.clone(),
            look,
        }
    }

    /// Whether `other` is these rows.
    pub(super) fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.painted, &other.painted)
    }

    /// The table no longer draws these rows: forget their bars, so the
    /// motion stops and veils nothing over the rows that replaced them.
    pub(super) fn hide(&self) {
        let mut painted = self.painted.borrow_mut();
        *painted = Painted {
            motion: painted.motion,
            ..Painted::default()
        };
    }

    /// The rows in `columns`, on `surface`, filling the list's room.
    pub(super) fn render<C: TableColumn>(
        &self,
        columns: &[C],
        surface: Hsla,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let wake = {
            let mut painted = self.painted.borrow_mut();
            let started = !painted.showing;
            painted.showing = true;
            painted.motion.filter(|_| started)
        };
        if let Some(motion) = wake {
            window.on_next_frame(move |_, cx| cx.notify(motion));
        }
        let slots: Vec<Slot> = columns
            .iter()
            .map(|column| Slot {
                width: column.width(),
                glyph: column.label().is_empty() && column.width() == GLYPH_WIDTH,
                flexible: column.flexible(),
            })
            .collect();
        let bar = cx.theme().skeleton;
        let painted = self.painted.clone();
        div()
            .id(self.id.clone())
            .test_support()
            .role(Role::Status)
            .aria_label("Loading")
            .size_full()
            .overflow_hidden()
            .child(
                canvas(
                    |_, _, _| (),
                    move |region, _, window, _| {
                        let bars = place(&slots, region, window);
                        for bar_at in &bars {
                            window.paint_quad(
                                fill(bar_at.bounds, bar).corner_radii(Corners::all(bar_at.radius)),
                            );
                        }
                        let viewport = window.content_mask().bounds.intersect(&region);
                        let mut painted = painted.borrow_mut();
                        *painted = Painted {
                            showing: true,
                            motion: painted.motion,
                            bars,
                            region,
                            viewport,
                            fill: surface,
                        };
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

/// One column's cell, in dp.
struct Slot {
    width: f32,
    glyph: bool,
    flexible: bool,
}

/// Where each bar goes in `region`, row by row: the rows the region has
/// room for, at most [`LOADING_ROWS`]. Flexible columns share the room the
/// others leave, as the real rows' cells do.
fn place(slots: &[Slot], region: Bounds<Pixels>, window: &Window) -> Vec<Bar> {
    let unit = dp_px(1., window);
    let total: f32 = slots.iter().map(|slot| slot.width).sum();
    let flexible = slots.iter().filter(|slot| slot.flexible).count().max(1);
    let spare = (region.size.width / unit - total).max(0.) / flexible as f32;
    let row_height = dp_px(ROW_HEIGHT, window);
    let rows = ((region.size.height / row_height).ceil() as usize).min(LOADING_ROWS);
    let mut bars = Vec::with_capacity(rows * slots.len());
    for row in 0..rows {
        let top = region.top() + row_height * row as f32;
        let mut x = 0.;
        for (column, slot) in slots.iter().enumerate() {
            let (offset, width, height) = if slot.glyph {
                ((slot.width - DOT) / 2., DOT, DOT)
            } else {
                let share = FILL[(row * 3 + column) % FILL.len()];
                let width = ((slot.width - 2. * CELL_PAD) * share).max(BAR);
                (CELL_PAD, width, BAR)
            };
            let origin = point(
                region.left() + unit * (x + offset),
                top + (row_height - unit * height) / 2.,
            );
            bars.push(Bar {
                bounds: Bounds::new(origin, size(unit * width, unit * height)),
                radius: if slot.glyph {
                    unit * DOT / 2.
                } else {
                    px(RADIUS)
                },
                row,
            });
            x += slot.width + if slot.flexible { spare } else { 0. };
        }
    }
    bars
}

/// The moving half of the loading rows: a small view over the table that
/// moves the bars the table painted, so its frames never redraw the table.
/// Once the table draws its rows instead, it draws nothing and asks no
/// frames. Only a drawn table stills it, so whoever mounts it must stop
/// once something else replaces the table. Mounted beside a page, it
/// paints after everything drawn before it, the status bar too; that is
/// safe, since it paints only within the table's viewport.
pub struct LoadingMotion {
    id: SharedString,
    painted: Rc<RefCell<Painted>>,
    look: Look,
}

impl LoadingMotion {
    pub fn look(&self) -> Look {
        self.look
    }

    pub fn set_look(&mut self, look: Look, cx: &mut Context<Self>) {
        if look != self.look {
            self.look = look;
            cx.notify();
        }
    }
}

impl Render for LoadingMotion {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("table.loading-motion");
        let _span = freshkube_probe::perf::span("table.loading-motion");
        let showing = {
            let mut painted = self.painted.borrow_mut();
            painted.motion = Some(cx.entity_id());
            painted.showing
        };
        let look = self.look;
        // Where the motion is now; nothing moves under reduced motion.
        let at = (showing && !cx.reduce_motion()).then(|| match look {
            Look::Pulse => motion::pulse_dim(cx),
            Look::Shimmer => motion::shimmer_at(cx),
        });
        if showing {
            motion::next_frame(window, cx);
        }
        let band = white().opacity(if cx.theme().mode.is_dark() { 0.09 } else { 0.6 });
        let painted = self.painted.clone();
        div()
            .id(self.id.clone())
            .absolute()
            .top_0()
            .left_0()
            .size_0()
            .children(at.map(|at| {
                canvas(
                    |_, _, _| (),
                    move |_, _, window, _| {
                        let painted = painted.borrow();
                        match look {
                            Look::Pulse => paint_pulse(&painted, at, window),
                            Look::Shimmer => paint_shimmer(&painted, at, band, window),
                        }
                    },
                )
                .size_0()
            }))
    }
}

/// Dims every bar by `dim` towards what it sits on: the same as drawing it
/// at `1 - dim` opacity.
fn paint_pulse(painted: &Painted, dim: f32, window: &mut Window) {
    let veil = painted.fill.opacity(dim);
    let mask = ContentMask {
        bounds: painted.viewport,
    };
    window.with_content_mask(Some(mask), |window| {
        for bar in &painted.bars {
            window.paint_quad(fill(bar.bounds, veil).corner_radii(Corners::all(bar.radius)));
        }
    });
}

/// The band at `at` of its sweep, on each bar it crosses. Each half is a
/// quad the bar's shape, its gradient's stops set where the half lies in
/// the bar, cut at the band's middle; so the bar's round ends stay round.
fn paint_shimmer(painted: &Painted, at: f32, band: Hsla, window: &mut Window) {
    let unit = dp_px(1., window);
    let region = painted.region;
    let travel = region.size.width / unit + BAND + LOADING_ROWS as f32 * SLANT;
    let clear = band.opacity(0.);
    for bar in &painted.bars {
        let start = region.left() + unit * (at * travel - BAND - bar.row as f32 * SLANT);
        // On a whole device pixel, so the halves' masks meet without
        // painting the pixel between them twice.
        let middle = snap(start + unit * (BAND / 2.), window);
        let end = start + unit * BAND;
        for (from, to, colors) in [(start, middle, (clear, band)), (middle, end, (band, clear))] {
            let left = from.max(bar.bounds.left());
            let right = to.min(bar.bounds.right());
            if left >= right {
                continue;
            }
            let width = bar.bounds.size.width;
            let stop = |x: Pixels| (x - bar.bounds.left()) / width;
            let gradient = linear_gradient(
                90.,
                linear_color_stop(colors.0, stop(from)),
                linear_color_stop(colors.1, stop(to)),
            );
            let cut = Bounds::new(
                point(left, bar.bounds.top()),
                size(right - left, bar.bounds.size.height),
            )
            .intersect(&painted.viewport);
            window.with_content_mask(Some(ContentMask { bounds: cut }), |window| {
                window
                    .paint_quad(fill(bar.bounds, gradient).corner_radii(Corners::all(bar.radius)));
            });
        }
    }
}

/// `x` on the nearest device pixel.
fn snap(x: Pixels, window: &Window) -> Pixels {
    let scale = window.scale_factor();
    px((f32::from(x) * scale).round() / scale)
}

/// Both halves for a page whose table waits for its first answer: the
/// [`LoadingRows`] its `TableSource::loading` returns while
/// [`showing`](Self::show), and a [`LoadingMotion`] (Pulse) to mount beside
/// the page. The page says each frame whether the rows show; it holds no
/// other state.
pub struct TableLoading {
    rows: LoadingRows,
    motion: Entity<LoadingMotion>,
    showing: std::cell::Cell<bool>,
}

impl TableLoading {
    pub fn new(prefix: &str, cx: &mut App) -> Self {
        let rows = LoadingRows::new(prefix);
        let motion = cx.new(|_| rows.motion(Look::Pulse));
        Self {
            rows,
            motion,
            showing: std::cell::Cell::new(false),
        }
    }

    /// Says whether the table shows the rows this frame.
    pub fn show(&self, showing: bool) {
        self.showing.set(showing);
    }

    /// For `TableSource::loading`.
    pub fn rows(&self) -> Option<&LoadingRows> {
        self.showing.get().then_some(&self.rows)
    }

    /// The motion to mount beside the page while the rows show.
    pub fn motion(&self, showing: bool) -> Option<Entity<LoadingMotion>> {
        showing.then(|| self.motion.clone())
    }
}
