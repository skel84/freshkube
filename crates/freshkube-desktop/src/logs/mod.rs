//! The Logs page's panel: a bounded, virtualized review of streamed log
//! lines with search, selection, copy, level and service filters.
//!
//! This file holds the panel's state, key actions and review navigation
//! (follow, scroll anchor, search, selection, copy). The rest is split by
//! concern into child modules, which share the panel's private fields:
//!
//! - `review`: the line model (retention, visible rows, search, selection).
//! - `talos`: collection from Talos services and stream delivery.
//! - `measure`: measured row heights and the wrapped-resize handling.
//! - `view`: rendering.

mod measure;
mod review;
mod talos;
mod view;

#[cfg(test)]
mod tests;

use std::{
    cell::Cell,
    collections::BTreeSet,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui_kit::{
    AppContext, Bounds, ClipboardItem, Context, Entity, FocusHandle, KeyBinding, Pixels, Point,
    SharedString, Size, Subscription, Task, Window, actions,
    component::{
        VirtualListScrollHandle,
        input::{InputEvent, InputState},
        scroll::ScrollbarHandle,
    },
    point, px,
};
use talos_rs::TalosClient;
use tokio::runtime::Handle;

use freshkube_core::logs::ServiceId;

use crate::backend::{OwnedJob, StreamEvent, Target};
use review::LogReview;

actions!(
    talos_logs,
    [
        CopySelected,
        NextLine,
        PreviousLine,
        ExtendNext,
        ExtendPrevious,
        FirstLine,
        LastLine,
        PageNext,
        PagePrevious,
        ClearSelection,
        FindNext,
        FindPrevious
    ]
);

#[derive(Clone, PartialEq)]
struct MeasurementKey {
    width: Pixels,
    rem: Pixels,
    font: SharedString,
    wrapped: bool,
    revision: u64,
}

#[derive(Clone, Copy)]
struct ReviewAnchor {
    id: u64,
    within_row: Pixels,
}

/// A row's size and the wrap width it was measured at; `None` when
/// unwrapped, where rows take their natural width whatever the pane's.
#[derive(Clone, Copy)]
struct RowMeasurement {
    wrap_width: Option<Pixels>,
    size: Size<Pixels>,
}

/// How long a wrapped pane's width must hold before rows off screen are
/// remeasured. Until then a live resize lays out only the rows it shows.
const RESIZE_SETTLE: Duration = Duration::from_millis(150);
/// Main-thread time per frame for remeasuring rows off screen.
const REMEASURE_BUDGET: Duration = Duration::from_millis(8);

/// Only the visible scrollbar gets this adapter. VirtualList and application
/// requests retain the raw handle, so their offsets are not manual review.
/// This scalar intent flag owns no entity or async task.
#[derive(Clone)]
struct ManualReviewScroll {
    base: VirtualListScrollHandle,
    requested: Rc<Cell<bool>>,
}

impl ScrollbarHandle for ManualReviewScroll {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        ScrollbarHandle::viewport_bounds(&self.base)
    }
    fn offset(&self) -> Point<Pixels> {
        ScrollbarHandle::offset(&self.base)
    }
    fn set_offset(&self, offset: Point<Pixels>) {
        self.requested.set(true);
        ScrollbarHandle::set_offset(&self.base, offset);
    }
    fn content_size(&self) -> Size<Pixels> {
        ScrollbarHandle::content_size(&self.base)
    }
    fn start_drag(&self) {
        self.requested.set(true);
    }
}

/// How often stream batches are applied while the page is hidden.
const HIDDEN_APPLY_INTERVAL: Duration = Duration::from_millis(250);

pub(crate) struct LogPanel {
    runtime: Handle,
    tail: i32,
    target: Option<(Target, TalosClient)>,
    fixture_target: Option<Target>,
    services: Vec<ServiceId>,
    collecting: BTreeSet<ServiceId>,
    /// Whether the default collection was already offered for this target.
    defaults_applied: bool,
    showing: BTreeSet<ServiceId>,
    review: LogReview,
    generation: u64,
    stream_revision: u64,
    job: Option<OwnedJob>,
    delivery: Option<Task<()>>,
    collection_active: bool,
    following: bool,
    wrapped: bool,
    query: Entity<InputState>,
    _query_subscription: Subscription,
    focus: FocusHandle,
    scroll: VirtualListScrollHandle,
    manual_review: Rc<Cell<bool>>,
    width: Option<Pixels>,
    panel_height: Option<Pixels>,
    measured: Option<MeasurementKey>,
    sizes: Rc<Vec<Size<Pixels>>>,
    /// Natural width of each row in `sizes`, for the unwrapped content width.
    row_widths: Vec<Pixels>,
    row_measurements: std::collections::BTreeMap<u64, RowMeasurement>,
    /// Whether each row in `sizes` is measured at the current geometry.
    /// The others keep their last height as an estimate.
    row_exact: Vec<bool>,
    /// Whether estimated rows may be remeasured: no resize is in progress.
    settled: bool,
    settle: Option<Task<()>>,
    unwrapped_width: Pixels,
    pending_reveal: Option<u64>,
    review_anchor: Option<ReviewAnchor>,
    anchor_evicted: bool,
    feedback: Option<String>,
    errors: std::collections::BTreeMap<ServiceId, String>,
    /// Whether the Logs page is on screen. Collection continues either way,
    /// but a hidden panel applies batches in coalesced groups and never
    /// asks the window to redraw for them.
    visible: bool,
    backlog: Vec<StreamEvent>,
    last_applied: Instant,
}

impl LogPanel {
    pub(crate) fn new(
        runtime: Handle,
        tail: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.bind_keys([
            KeyBinding::new("secondary-c", CopySelected, Some("TalosLogs")),
            KeyBinding::new("down", NextLine, Some("TalosLogs")),
            KeyBinding::new("up", PreviousLine, Some("TalosLogs")),
            KeyBinding::new("shift-down", ExtendNext, Some("TalosLogs")),
            KeyBinding::new("shift-up", ExtendPrevious, Some("TalosLogs")),
            KeyBinding::new("home", FirstLine, Some("TalosLogs")),
            KeyBinding::new("end", LastLine, Some("TalosLogs")),
            KeyBinding::new("pagedown", PageNext, Some("TalosLogs")),
            KeyBinding::new("pageup", PagePrevious, Some("TalosLogs")),
            KeyBinding::new("escape", ClearSelection, Some("TalosLogs")),
            KeyBinding::new("f3", FindNext, Some("TalosLogs")),
            KeyBinding::new("shift-f3", FindPrevious, Some("TalosLogs")),
        ]);
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search retained lines"));
        let subscription = cx.subscribe_in(&query, window, |this, _, event, _, cx| match event {
            InputEvent::Change => {
                this.review.query = this.query.read(cx).value().to_string();
                this.review.current_match = None;
                this.feedback = None;
                cx.notify();
            }
            InputEvent::PressEnter { shift, .. } => this.search(!shift, cx),
            _ => {}
        });
        Self {
            runtime,
            tail,
            target: None,
            fixture_target: None,
            services: Vec::new(),
            collecting: BTreeSet::new(),
            defaults_applied: false,
            showing: BTreeSet::new(),
            review: LogReview::new(""),
            generation: 0,
            stream_revision: 0,
            job: None,
            delivery: None,
            collection_active: false,
            following: true,
            wrapped: true,
            query,
            _query_subscription: subscription,
            focus: cx.focus_handle().tab_stop(true),
            scroll: VirtualListScrollHandle::new(),
            width: None,
            panel_height: None,
            measured: None,
            sizes: Rc::new(Vec::new()),
            row_widths: Vec::new(),
            pending_reveal: None,
            manual_review: Rc::new(Cell::new(false)),
            row_measurements: std::collections::BTreeMap::new(),
            row_exact: Vec::new(),
            settled: true,
            settle: None,
            unwrapped_width: px(0.),
            review_anchor: None,
            anchor_evicted: false,
            feedback: None,
            errors: std::collections::BTreeMap::new(),
            visible: true,
            backlog: Vec::new(),
            last_applied: Instant::now(),
        }
    }

    fn capture_anchor(&mut self) {
        if self.following
            || self.review.visible.is_empty()
            || self.sizes.len() != self.review.visible.len()
        {
            return;
        }
        let mut offset = -self.scroll.offset().y;
        for (ix, row) in self.sizes.iter().enumerate() {
            if offset < row.height || ix + 1 == self.sizes.len() {
                self.review_anchor = Some(ReviewAnchor {
                    id: self.review.id(ix),
                    within_row: offset.max(px(0.)),
                });
                return;
            }
            offset -= row.height;
        }
    }

    fn restore_anchor(&mut self) {
        let Some(anchor) = self.review_anchor else {
            return;
        };
        let row_ix = self.review.row_for_id(anchor.id);
        if let Some(row_ix) = row_ix {
            let before: Pixels = self.sizes[..row_ix].iter().map(|row| row.height).sum();
            let within = anchor.within_row.min(self.sizes[row_ix].height);
            self.scroll
                .set_offset(point(self.scroll.offset().x, -(before + within)));
        } else {
            self.anchor_evicted = true;
            self.scroll
                .set_offset(point(self.scroll.offset().x, px(0.)));
            self.review_anchor = self.review.visible.first().map(|_| ReviewAnchor {
                id: self.review.id(0),
                within_row: px(0.),
            });
        }
    }

    fn set_following(&mut self, following: bool, cx: &mut Context<Self>) {
        self.following = following;
        self.review.logs.buffer_mut().set_following(following);
        self.review.logs.buffer_mut().set_paused(!following);
        if following {
            self.review_anchor = None;
            self.anchor_evicted = false;
            self.pending_reveal = self
                .review
                .visible
                .len()
                .checked_sub(1)
                .map(|ix| self.review.id(ix));
        } else {
            self.capture_anchor();
        }
        cx.notify();
    }

    fn apply_manual_review(&mut self, cx: &mut Context<Self>) {
        if self.manual_review.replace(false) {
            self.set_following(false, cx);
        }
    }

    fn search(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.set_following(false, cx);
        self.pending_reveal = self.review.search(forward);
        self.review_anchor = None;
        self.feedback = if self.pending_reveal.is_none() && !self.review.query.is_empty() {
            Some("No matching retained lines in the current filters".into())
        } else {
            None
        };
        cx.notify();
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        match self.review.copy_text() {
            Ok(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                self.feedback = Some("Copied selected complete visible lines".into());
            }
            Err(error) => self.feedback = Some(error.into()),
        }
        cx.notify();
    }

    fn navigate(&mut self, delta: isize, extend: bool, cx: &mut Context<Self>) {
        self.set_following(false, cx);
        self.pending_reveal = self.review.move_selection(delta, extend);
        self.review_anchor = None;
        cx.notify();
    }
}
