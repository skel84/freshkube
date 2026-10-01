//! A bounded, virtualized review of streamed log lines with search,
//! selection, copy, level and source filters, shared by every log source.
//!
//! [`LogView`] owns everything about reviewing lines: retention, search,
//! selection, copy, the level and source filters, follow and wrap, measured
//! rows, and coalescing batches while hidden. A [`LogSource`] owns where
//! the lines come from: its stream, its catalog of sources, its failures
//! and its own toolbar controls. The Talos Logs page is
//! `LogView<TalosLogs>`, named [`LogPanel`].
//!
//! This file holds the view's state, key actions and review navigation
//! (follow, scroll anchor, search, selection, copy). The rest is split by
//! concern into child modules, which share the view's private fields:
//!
//! - `review`: the line model (retention, visible rows, search, selection).
//! - `measure`: measured row heights and the wrapped-resize handling.
//! - `view`: rendering.
//! - `talos`: the Talos source: service catalog, collection and delivery.

mod measure;
mod pod;
mod review;
mod talos;
mod view;

#[cfg(test)]
mod tests;

use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    time::{Duration, Instant},
};

use gpui_kit::{
    AnyElement, App, AppContext, Bounds, ClipboardItem, Context, Entity, FocusHandle, Global,
    KeyBinding, Pixels, Point, SharedString, Size, Subscription, Task, Window, actions,
    component::{
        VirtualListScrollHandle,
        input::{InputEvent, InputState},
        scroll::ScrollbarHandle,
    },
    point, px,
};

use freshkube_core::logs::{LogEvent, ServiceId};

pub(crate) use pod::PodLogView;
use review::{LogReview, MAX_SELECTED_LINES};
pub(crate) use talos::TalosLogs;

/// The Talos Logs page.
pub(crate) type LogPanel = LogView<TalosLogs>;

/// The key context of every log view's line list.
const CONTEXT: &str = "LogView";
/// The key context around the search field.
const SEARCH_CONTEXT: &str = "LogSearch";
/// The key context of the whole panel: toolbar, search and lines.
const PANEL_CONTEXT: &str = "LogPanel";

actions!(
    log_view,
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
        FindPrevious,
        LeaveSearch,
        FocusSearch,
        SelectAll
    ]
);

/// Marks the log view's key bindings as registered, so views made later,
/// one per pane, don't add them again.
struct KeysBound;

impl Global for KeysBound {}

#[derive(Clone, PartialEq)]
struct MeasurementKey {
    width: Pixels,
    rem: Pixels,
    font: SharedString,
    wrapped: bool,
    columns: Columns,
    revision: u64,
}

/// Which of the optional columns rows show. A source with one stream has
/// no use for the source column; pod logs may hide their timestamps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Columns {
    pub(super) time: bool,
    pub(super) source: bool,
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

/// Where a log view's lines come from. The view calls these while it
/// renders; the source feeds lines through [`LogView::ingest`] and keeps
/// its stream, catalog and failures to itself.
pub(crate) trait LogSource: Sized + 'static {
    /// Lays out whatever `controls` needs measured, once per frame before
    /// the toolbar is laid out. `width` is the panel's.
    fn prepare_controls(
        _view: &mut LogView<Self>,
        _width: Pixels,
        _window: &mut Window,
        _cx: &mut Context<LogView<Self>>,
    ) {
    }

    /// Toolbar rows above the shared filters, search and buttons.
    fn controls(view: &LogView<Self>, cx: &mut Context<LogView<Self>>) -> Vec<AnyElement>;

    /// What the list says while no line is visible.
    fn empty_message(view: &LogView<Self>) -> SharedString;

    /// Whether new lines may still arrive, so following them means
    /// something. A log read to its end turns Follow off.
    fn live(_view: &LogView<Self>) -> bool {
        true
    }

    /// Stream failures by source, shown above the lines.
    fn errors(&self) -> &BTreeMap<ServiceId, String>;
}

pub(crate) struct LogView<S: LogSource> {
    source: S,
    /// Sources whose lines are shown; the others stay retained but hidden.
    showing: BTreeSet<ServiceId>,
    review: LogReview,
    /// Bumped on every reset, so rows of an earlier review never share an
    /// element id with the current one.
    generation: u64,
    following: bool,
    wrapped: bool,
    columns: Columns,
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
    row_measurements: BTreeMap<u64, RowMeasurement>,
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
    /// Whether the view is on screen. Its source keeps streaming either
    /// way, but a hidden view applies lines in coalesced groups and never
    /// asks the window to redraw for them.
    visible: bool,
    backlog: Vec<LogEvent>,
    last_applied: Instant,
}

impl<S: LogSource> LogView<S> {
    pub(crate) fn with_source(source: S, window: &mut Window, cx: &mut Context<Self>) -> Self {
        if !cx.has_global::<KeysBound>() {
            cx.set_global(KeysBound);
            cx.bind_keys([
                KeyBinding::new("secondary-c", CopySelected, Some(CONTEXT)),
                KeyBinding::new("down", NextLine, Some(CONTEXT)),
                KeyBinding::new("up", PreviousLine, Some(CONTEXT)),
                KeyBinding::new("shift-down", ExtendNext, Some(CONTEXT)),
                KeyBinding::new("shift-up", ExtendPrevious, Some(CONTEXT)),
                KeyBinding::new("home", FirstLine, Some(CONTEXT)),
                KeyBinding::new("end", LastLine, Some(CONTEXT)),
                KeyBinding::new("pagedown", PageNext, Some(CONTEXT)),
                KeyBinding::new("pageup", PagePrevious, Some(CONTEXT)),
                KeyBinding::new("escape", ClearSelection, Some(CONTEXT)),
                KeyBinding::new("secondary-a", SelectAll, Some(CONTEXT)),
                KeyBinding::new("escape", LeaveSearch, Some(SEARCH_CONTEXT)),
                KeyBinding::new("secondary-f", FocusSearch, Some(PANEL_CONTEXT)),
                KeyBinding::new("secondary-g", FindNext, Some(PANEL_CONTEXT)),
                KeyBinding::new("secondary-shift-g", FindPrevious, Some(PANEL_CONTEXT)),
                KeyBinding::new("f3", FindNext, Some(PANEL_CONTEXT)),
                KeyBinding::new("shift-f3", FindPrevious, Some(PANEL_CONTEXT)),
            ]);
        }
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
            source,
            showing: BTreeSet::new(),
            review: LogReview::new(""),
            generation: 0,
            following: true,
            wrapped: true,
            columns: Columns {
                time: true,
                source: true,
            },
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
            row_measurements: BTreeMap::new(),
            row_exact: Vec::new(),
            settled: true,
            settle: None,
            unwrapped_width: px(0.),
            review_anchor: None,
            anchor_evicted: false,
            feedback: None,
            visible: true,
            backlog: Vec::new(),
            last_applied: Instant::now(),
        }
    }

    /// Tells the view whether it is on screen. Showing it applies
    /// everything received meanwhile.
    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if visible {
            self.flush_backlog(cx);
            cx.notify();
        }
    }

    /// Applies the lines held while hidden.
    fn flush_backlog(&mut self, cx: &mut Context<Self>) {
        if self.backlog.is_empty() {
            return;
        }
        let lines = std::mem::take(&mut self.backlog);
        self.apply_lines(lines, cx);
    }

    /// Takes lines the source accepted for its current stream. A hidden
    /// view doesn't spend main-thread time per batch: it holds lines and
    /// applies them in coalesced groups.
    fn ingest(&mut self, lines: Vec<LogEvent>, cx: &mut Context<Self>) {
        if !self.visible {
            self.backlog.extend(lines);
            if self.last_applied.elapsed() < HIDDEN_APPLY_INTERVAL {
                return;
            }
            let lines = std::mem::take(&mut self.backlog);
            self.apply_lines(lines, cx);
            return;
        }
        self.apply_lines(lines, cx);
    }

    fn apply_lines(&mut self, lines: Vec<LogEvent>, cx: &mut Context<Self>) {
        self.last_applied = Instant::now();
        self.apply_manual_review(cx);
        self.capture_anchor();
        self.review.append(lines);
        if self.following {
            self.pending_reveal = self.last_row_id();
        }
        // A hidden view isn't drawn and nothing else shows what a batch
        // changes, so only a visible one asks for a redraw.
        if self.visible {
            cx.notify();
        }
    }

    /// Starts an empty review for a new source identity, such as another
    /// node: lines, filters, scroll position and search all start over.
    /// Measurements are keyed by line identity and stay cached.
    fn reset(&mut self, address: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.reset_lines(address);
        // Setting the input's value from code emits no change event.
        self.review.query.clear();
        self.query
            .update(cx, |query, cx| query.set_value("", window, cx));
    }

    /// Like [`Self::reset`], but the search carries over to the new lines.
    fn reset_lines(&mut self, address: &str) {
        let query = std::mem::take(&mut self.review.query);
        self.backlog.clear();
        self.generation += 1;
        self.review = LogReview::new(address);
        self.showing.clear();
        self.review_anchor = None;
        self.anchor_evicted = false;
        self.feedback = None;
        self.following = true;
        self.measured = None;
        self.sizes = Rc::new(Vec::new());
        self.row_widths.clear();
        self.row_exact.clear();
        self.scroll = VirtualListScrollHandle::new();
        self.manual_review = Rc::new(Cell::new(false));
        self.review.query = query;
    }

    /// Shows or hides the optional columns, keeping the review in place.
    fn set_columns(&mut self, columns: Columns, cx: &mut Context<Self>) {
        if self.columns != columns {
            self.capture_anchor();
            self.columns = columns;
            cx.notify();
        }
    }

    /// Shows or hides one source's lines without touching its stream.
    fn toggle_shown(&mut self, source: &ServiceId, cx: &mut Context<Self>) {
        self.capture_anchor();
        if !self.showing.remove(source) {
            self.showing.insert(source.clone());
        }
        self.review.set_service_filter(self.showing.clone());
        cx.notify();
    }

    /// Identity of the last visible row, which following keeps in view.
    fn last_row_id(&self) -> Option<u64> {
        self.review
            .visible
            .len()
            .checked_sub(1)
            .map(|ix| self.review.id(ix))
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
            self.pending_reveal = self.last_row_id();
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

    /// Puts the keyboard on the lines.
    pub(crate) fn focus_lines(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    /// Puts the keyboard in the search.
    pub(crate) fn focus_search(&self, window: &mut Window, cx: &mut App) {
        let focus = gpui_kit::Focusable::focus_handle(self.query.read(cx), cx);
        window.focus(&focus, cx);
    }

    /// The next or previous retained line that matches the search.
    pub(crate) fn find_next(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.search(forward, cx);
    }

    /// Selects every visible line, up to the selection limit, newest
    /// first.
    pub(crate) fn select_all(&mut self, cx: &mut Context<Self>) {
        let count = self.review.select_all();
        self.feedback = self
            .review
            .selection_limited
            .then(|| format!("Selected the newest {MAX_SELECTED_LINES} of {count} lines"));
        cx.notify();
    }

    /// Escape in the search clears it; in an empty search it hands the
    /// keyboard to the lines.
    fn leave_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.query.read(cx).value().is_empty() {
            window.focus(&self.focus, cx);
            return;
        }
        // Setting the value from code emits no change event.
        self.query
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.review.query.clear();
        self.review.current_match = None;
        self.feedback = None;
        cx.notify();
    }

    /// Escape drops the selection; with none, it goes to the page around.
    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.review.selected.is_empty() {
            cx.propagate();
            return;
        }
        self.review.selected.clear();
        self.review.selection_anchor = None;
        cx.notify();
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
        match self.review.copy_text(self.columns.time) {
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
