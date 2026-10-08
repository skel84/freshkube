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
//!
//! A flash fades in [`motion::FLASH_STEPS`] steps, read from its change's
//! time on the executor's clock, so it is right on any frame. One timer
//! steps every flash the layer can draw, redrawing the layer each
//! [`motion::FLASH_STEP`] and stopping when the last fade ends; the layer
//! never asks for animation frames, so a steady trickle of changes costs a
//! few frames a second, not sixty. The line each flashing row is on is
//! looked up when it flashes and when the rows change, never per frame.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

use gpui_kit::prelude::*;
use gpui_kit::{
    App, Bounds, ContentMask, Context, Hsla, Pixels, ScrollHandle, SharedString, Task,
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
    /// The line each flashing key's row was on when last looked up.
    lines: HashMap<K, usize>,
    /// The rows' revision the lines were looked up in.
    revision: Option<u64>,
    reduced: Reduced,
    /// Clears the flashes once the last has ended, with one redraw.
    clear: Option<Task<()>>,
    /// Steps the fades while one the layer draws runs with motion on.
    tick: Option<Task<()>>,
}

impl<K: Clone + Eq + Hash + 'static> FlashLayer<K> {
    /// A layer over the rows `rows` places, with the id `<prefix>-flash`.
    /// `line_of` finds the line a key's row is on; it runs when a row
    /// flashes and when [`rows_changed`](Self::rows_changed) says so.
    ///
    /// It runs inside [`changed`](Self::changed) and `rows_changed`, so
    /// call them outside any update of what `line_of` reads, such as the
    /// table's entity: update the table, then report to the layer.
    /// Reporting from inside the table's own update panics on a double
    /// lease.
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
            lines: HashMap::new(),
            revision: None,
            reduced: Reduced::Nothing,
            clear: None,
            tick: None,
        }
    }

    /// Flashes one batch of changed rows, unless it is a burst
    /// ([`motion::FLASH_BURST`]). Returns whether it flashed.
    pub fn changed(&mut self, keys: impl IntoIterator<Item = K>, cx: &mut Context<Self>) -> bool {
        let now = cx.background_executor().now();
        let flashed = self.flashes.changed(keys, now);
        if flashed {
            self.locate(now, cx);
            self.schedule_clear(now, cx);
            cx.notify();
        }
        flashed
    }

    /// Counts a batch of `count` changes too big to flash
    /// ([`burst`](Self::burst)) without its keys, as [`changed`](Self::changed)
    /// would count it.
    pub fn held_back(&mut self, count: usize, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        self.flashes.held_back(count, now);
    }

    /// The most changes one batch can bring and still flash.
    pub fn burst(&self) -> usize {
        self.flashes.burst()
    }

    /// Looks the flashing rows up again when the rows' `revision` is new:
    /// the page calls it whenever its rows are sorted, filtered or changed.
    pub fn rows_changed(&mut self, revision: u64, cx: &mut Context<Self>) {
        if self.revision == Some(revision) {
            return;
        }
        self.revision = Some(revision);
        let now = cx.background_executor().now();
        if self.flashes.live(now).next().is_some() {
            self.locate(now, cx);
            cx.notify();
        }
    }

    /// Forgets every flash, as when the list is read again.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.flashes.clear();
        self.lines.clear();
        self.clear = None;
        self.tick = None;
        cx.notify();
    }

    /// How strong a key's tint is now, from 1 at its change to 0, as the
    /// layer draws it; `None` while it draws none for the key.
    pub fn strength(&self, key: &K, cx: &App) -> Option<f32> {
        let now = cx.background_executor().now();
        self.lines.get(key)?;
        let flash = self.flashes.live(now).find(|flash| flash.key == *key)?;
        match (cx.reduce_motion(), self.reduced) {
            (false, _) => Some(motion::fade_step(now.saturating_duration_since(flash.at))),
            (true, Reduced::Held) => Some(1.),
            (true, Reduced::Nothing) => None,
        }
    }

    fn locate(&mut self, now: Instant, cx: &App) {
        self.lines.clear();
        for flash in self.flashes.live(now) {
            if let Some(line) = (self.line_of)(&flash.key, cx) {
                self.lines.insert(flash.key.clone(), line);
            }
        }
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

    /// The line the layer draws a key's tint on, as last looked up.
    pub fn line(&self, key: &K) -> Option<usize> {
        self.lines.get(key).copied()
    }

    /// Whether the timer that steps the fades runs.
    pub fn stepping(&self) -> bool {
        self.tick.is_some()
    }

    /// The rows flashing now, by key.
    pub fn flashing(&self, cx: &App) -> Vec<K> {
        let now = cx.background_executor().now();
        self.flashes
            .live(now)
            .map(|flash| flash.key.clone())
            .collect()
    }

    /// Whether a flash the layer can draw is fading at `now`: one whose
    /// row is filtered out or doesn't show steps nothing.
    fn drawing(&self, now: Instant) -> bool {
        self.flashes
            .live(now)
            .any(|flash| self.lines.contains_key(&flash.key))
    }

    /// Starts the timer that steps the fades, unless it runs. Each step
    /// redraws the layer while a fade it draws runs; the timer stops once
    /// none does or motion is reduced.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.tick.is_some() {
            return;
        }
        self.tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(motion::FLASH_STEP).await;
                let go = this
                    .update(cx, |this, cx| {
                        // The last fade's end clears itself, on time
                        // (`schedule_clear`); a step after it draws nothing.
                        let now = cx.background_executor().now();
                        let go = !cx.reduce_motion() && this.drawing(now);
                        if go {
                            cx.notify();
                        } else {
                            this.tick = None;
                        }
                        go
                    })
                    .unwrap_or(false);
                if !go {
                    break;
                }
            }
        }));
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
                let live: Vec<K> = this.flashes.live(now).map(|f| f.key.clone()).collect();
                this.lines.retain(|key, _| live.contains(key));
                this.clear = None;
                cx.notify();
            })
            .ok();
        }));
    }
}

impl<K: Clone + Eq + Hash + 'static> Render for FlashLayer<K> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("table.flash-layer");
        let accent = palette(cx).accent;
        let now = cx.background_executor().now();
        let reduce = cx.reduce_motion();
        let shown = !reduce || self.reduced == Reduced::Held;
        if !reduce && self.drawing(now) {
            self.tick(cx);
        }
        let tints = self
            .flashes
            .live(now)
            .filter(|_| shown)
            .filter_map(|flash| {
                let line = *self.lines.get(&flash.key)?;
                let strength = if reduce {
                    1.
                } else {
                    motion::fade_step(now.saturating_duration_since(flash.at))
                };
                let tint = accent.opacity(motion::FLASH_TINT * strength);
                Some(row_tint(self.rows.clone(), line, tint))
            });
        div()
            .id(self.id.clone())
            .absolute()
            .top_0()
            .left_0()
            .size_0()
            .children(tints)
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
