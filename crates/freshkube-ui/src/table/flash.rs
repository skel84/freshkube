//! The change flash: a changed row is tinted and fades over
//! [`motion::FADE`], drawn by a small view over the table's rows rather than
//! by the table.
//!
//! GPUI marks a view's ancestors dirty when it asks for a frame, so a flash
//! drawn by a cell would redraw the table and its page on every frame. The
//! layer is the table's sibling instead: keep the table in a cached view
//! beside it, and the layer's frames draw only the layer and the views
//! above both. It paints where the table's last frame put each row, read
//! from the table's scroll handles, clipped to the rows' viewport, and takes
//! no pointer events.

use std::time::Instant;

use gpui_kit::prelude::*;
use gpui_kit::{
    App, Bounds, ContentMask, Context, ElementId, Hsla, Pixels, ScrollHandle, SharedString, Task,
    UniformListScrollHandle, Window, canvas, div, fill, point, size,
};

use super::ROW_HEIGHT;
use crate::motion::{self, Flashes};
use crate::palette::palette;
use crate::ui;

/// Where a table's rows are in the window, from its scroll handles:
/// [`TableState::rows_at`](super::TableState::rows_at).
#[derive(Clone)]
pub struct RowsAt {
    pub(super) list: UniformListScrollHandle,
    pub(super) sideways: ScrollHandle,
}

impl RowsAt {
    /// The line's bounds and the rows' viewport they show in, as the
    /// table's last frame laid them out; `None` before it drew rows.
    pub fn line(&self, line: usize, window: &Window) -> Option<(Bounds<Pixels>, Bounds<Pixels>)> {
        let state = self.list.0.borrow();
        let list = state.base_handle.bounds();
        if list.size.height <= Pixels::ZERO {
            return None;
        }
        let viewport = list.intersect(&self.sideways.bounds());
        let height = ui::dp_px(ROW_HEIGHT, window);
        let top = list.top() + state.base_handle.offset().y + height * line as f32;
        let row = Bounds::new(
            point(viewport.left(), top),
            size(viewport.size.width, height),
        );
        Some((row, viewport))
    }
}

/// What a flash shows while motion is reduced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reduced {
    /// Nothing: GPUI draws a fade's end, which is clear.
    Nothing,
    /// The tint, held for the fade's length and then cleared at once.
    Held,
}

/// Finds the line a key's row is on now, if it shows.
type LineOf<K> = Box<dyn Fn(&K, &App) -> Option<usize>>;

/// The flashes over one table's rows, keyed as its rows are.
pub struct FlashLayer<K> {
    id: SharedString,
    rows: RowsAt,
    /// The line a key's row is on now, if it shows.
    line_of: LineOf<K>,
    flashes: Flashes<K>,
    reduced: Reduced,
    /// Clears the flashes once the last has ended, with one redraw.
    clear: Option<Task<()>>,
}

impl<K: Clone + Eq + 'static> FlashLayer<K> {
    /// A layer over the rows `rows` places, with the id `<prefix>-flash`.
    pub fn new(
        prefix: &str,
        rows: RowsAt,
        line_of: impl Fn(&K, &App) -> Option<usize> + 'static,
    ) -> Self {
        Self {
            id: format!("{prefix}-flash").into(),
            rows,
            line_of: Box::new(line_of),
            flashes: Flashes::new(),
            reduced: Reduced::Nothing,
            clear: None,
        }
    }

    /// Flashes one batch of changed rows, unless it is a burst
    /// ([`motion::FLASH_BURST`]). Returns whether it flashed.
    pub fn changed(&mut self, keys: impl IntoIterator<Item = K>, cx: &mut Context<Self>) -> bool {
        let now = cx.background_executor().now();
        let flashed = self.flashes.changed(keys, now);
        if flashed {
            self.schedule_clear(now, cx);
            cx.notify();
        }
        flashed
    }

    /// Forgets every flash, as when the list is read again.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.flashes.clear();
        self.clear = None;
        cx.notify();
    }

    pub fn reduced(&self) -> Reduced {
        self.reduced
    }

    pub fn set_reduced(&mut self, reduced: Reduced, cx: &mut Context<Self>) {
        if reduced != self.reduced {
            self.reduced = reduced;
            cx.notify();
        }
    }

    /// The rows flashing now, by key.
    pub fn flashing(&self, cx: &App) -> Vec<K> {
        let now = cx.background_executor().now();
        self.flashes
            .live(now)
            .map(|flash| flash.key.clone())
            .collect()
    }

    fn schedule_clear(&mut self, now: Instant, cx: &mut Context<Self>) {
        let Some(ends) = self.flashes.ends() else {
            return;
        };
        let wait = ends.saturating_duration_since(now);
        self.clear = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                let now = cx.background_executor().now();
                this.flashes.prune(now);
                this.clear = None;
                cx.notify();
            })
            .ok();
        }));
    }
}

impl<K: Clone + Eq + 'static> Render for FlashLayer<K> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("table.flash-layer");
        let reduce = cx.reduce_motion();
        let tint = palette(cx).accent.opacity(motion::FLASH_TINT);
        let now = cx.background_executor().now();
        let shown = !reduce || self.reduced == Reduced::Held;
        let flashes: Vec<_> = if shown {
            self.flashes
                .live(now)
                .filter_map(|flash| Some((flash.seq, (self.line_of)(&flash.key, cx)?)))
                .collect()
        } else {
            Vec::new()
        };
        div()
            .id(self.id.clone())
            .absolute()
            .top_0()
            .left_0()
            .size_0()
            .children(flashes.into_iter().map(|(seq, line)| {
                let tinted = div()
                    .size_0()
                    .child(row_tint(self.rows.clone(), line, tint));
                let id = ElementId::from((self.id.clone(), seq));
                if reduce {
                    tinted.into_any_element()
                } else {
                    motion::fade_out(id, tinted).into_any_element()
                }
            }))
    }
}

/// Paints one line's tint where the table put it, inside its viewport.
fn row_tint(rows: RowsAt, line: usize, tint: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |_, _, window, _| {
            let Some((row, viewport)) = rows.line(line, window) else {
                return;
            };
            window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
                window.paint_quad(fill(row, tint));
            });
        },
    )
    .size_0()
}
