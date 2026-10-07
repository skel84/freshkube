//! The contract a [`LogSource`]'s panel uses: everything a source may ask
//! of the view it feeds. A source keeps its stream, catalog and failures to
//! itself and changes the review only through these methods; retention,
//! measurement, scroll and selection stay private to the view.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;

use gpui_kit::{Context, Window, component::VirtualListScrollHandle, point, px};

use freshkube_core::logs::{LogEvent, ServiceId};

use super::review::{LogReview, MAX_SELECTED_LINES};
use super::{Columns, HIDDEN_APPLY_INTERVAL, LogSource, LogView, ReviewAnchor};

impl<S: LogSource> LogView<S> {
    pub fn source(&self) -> &S {
        &self.source
    }

    pub fn source_mut(&mut self) -> &mut S {
        &mut self.source
    }

    /// The panel's height when it last drew, for a source that fits its
    /// controls to the room the log leaves them.
    pub fn panel_height(&self) -> Option<gpui_kit::Pixels> {
        self.panel_height
    }

    /// Applies the lines held while hidden.
    pub fn flush_backlog(&mut self, cx: &mut Context<Self>) {
        if self.backlog.is_empty() {
            return;
        }
        let lines = std::mem::take(&mut self.backlog);
        self.apply_lines(lines, cx);
    }

    /// Takes lines the source accepted for its current stream. A hidden
    /// view doesn't spend main-thread time per batch: it holds lines and
    /// applies them in coalesced groups.
    pub fn ingest(&mut self, lines: Vec<LogEvent>, cx: &mut Context<Self>) {
        if !self.visible {
            self.backlog.extend(lines);
            // The executor's clock, so tests can step it.
            let now = cx.background_executor().now();
            if now.saturating_duration_since(self.last_applied) < HIDDEN_APPLY_INTERVAL {
                return;
            }
            let lines = std::mem::take(&mut self.backlog);
            self.apply_lines(lines, cx);
            return;
        }
        self.apply_lines(lines, cx);
    }

    /// Starts an empty review for a new source identity, such as another
    /// node: lines, filters, scroll position and search all start over.
    /// Measurements are keyed by line identity and stay cached.
    pub fn reset(&mut self, address: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.reset_lines(address);
        // Setting the input's value from code emits no change event.
        self.review.query.clear();
        self.query
            .update(cx, |query, cx| query.set_value("", window, cx));
    }

    /// Like [`Self::reset`], but the search carries over to the new lines.
    pub fn reset_lines(&mut self, address: &str) {
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
        self.sizes_height = 0.;
        self.row_widths.clear();
        self.row_exact.clear();
        self.scroll = VirtualListScrollHandle::new();
        self.panel_scroll.set_offset(point(px(0.), px(0.)));
        self.manual_review = Rc::new(Cell::new(false));
        self.review.query = query;
    }

    /// Shows or hides the optional columns, keeping the review in place.
    pub fn set_columns(&mut self, columns: Columns, cx: &mut Context<Self>) {
        if self.columns != columns {
            self.capture_anchor();
            self.columns = columns;
            cx.notify();
        }
    }

    /// Shows or hides one source's lines without touching its stream.
    pub fn toggle_shown(&mut self, source: &ServiceId, cx: &mut Context<Self>) {
        self.capture_anchor();
        if !self.showing.remove(source) {
            self.showing.insert(source.clone());
        }
        self.review.set_service_filter(self.showing.clone());
        cx.notify();
    }

    /// Holds the review position, by line, across a change the source
    /// makes next: Talos captures it before a new catalog changes which
    /// services are shown.
    pub fn capture_anchor(&mut self) {
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

    /// Bumped on every reset; row element ids carry it, and the Talos
    /// example target takes it as its epoch.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The toolbar is narrow, as the last frame laid it out: a source's
    /// tools show their icons alone too.
    pub fn compact(&self) -> bool {
        self.compact
    }

    pub fn columns(&self) -> Columns {
        self.columns
    }

    /// The columns a new view starts with.
    pub fn with_columns(mut self, columns: Columns) -> Self {
        self.columns = columns;
        self
    }

    /// Sources whose lines are shown; the others stay retained but hidden.
    pub fn shown(&self) -> &BTreeSet<ServiceId> {
        &self.showing
    }

    /// Shows exactly these sources' lines. It doesn't notify; the caller
    /// does, once its own change is made.
    pub fn set_shown(&mut self, shown: BTreeSet<ServiceId>) {
        self.showing = shown;
        self.review.set_service_filter(self.showing.clone());
    }

    /// The notice under the toolbar, such as why an action did nothing.
    /// It doesn't notify; the caller does.
    pub fn set_feedback(&mut self, feedback: Option<String>) {
        self.feedback = feedback;
    }

    /// Adds lines read before any stream, such as example history, at once
    /// and whether or not the view is on screen.
    pub fn preload(&mut self, events: Vec<LogEvent>, cx: &mut Context<Self>) {
        self.review.append(events);
        self.last_applied = cx.background_executor().now();
    }

    /// Scrolls to the last visible line on the next frame.
    pub fn reveal_last(&mut self) {
        self.pending_reveal = self.last_row_id();
        self.reveal_matched_line = None;
    }

    /// Whether any line is retained, whatever the filters.
    pub fn has_lines(&self) -> bool {
        !self.review.logs.buffer().entries().is_empty()
    }

    /// Retained lines from `service`.
    pub fn service_count(&self, service: &ServiceId) -> usize {
        self.review.service_count(service)
    }

    /// The identity the next line will get. It only grows, so it counts
    /// arrivals whatever retention dropped; the Talos example stream
    /// numbers its lines from it.
    pub fn next_line_id(&self) -> u64 {
        self.review.next_id
    }

    /// The review's part of a status line: counts, matching lines, selection,
    /// follow, and what retention dropped. `streaming` says whether lines
    /// still arrive while paused.
    pub fn review_status(&self, streaming: bool) -> Vec<String> {
        let mut parts = vec![format!(
            "{} visible / {} retained",
            self.review.visible.len(),
            self.review.logs.buffer().entries().len()
        )];
        if !self.review.query.is_empty() {
            let count = self.review.match_count();
            parts.push(if count == 1 {
                "1 matching line".into()
            } else {
                format!("{count} matching lines")
            });
        }
        parts.push(format!("{} selected", self.review.selected.len()));
        parts.push(if self.following {
            "Following".into()
        } else if streaming {
            "Paused, collection continues".into()
        } else {
            "Paused".into()
        });
        if self.review.evicted > 0 || self.review.omitted > 0 {
            parts.push(format!(
                "{} oldest lines evicted, {} over 64 KiB omitted",
                self.review.evicted, self.review.omitted
            ));
        }
        if self.anchor_evicted {
            parts.push("Review position was evicted; showing the earliest line".into());
        }
        if self.review.selection_limited {
            parts.push(format!("Selection limited to {MAX_SELECTED_LINES} lines"));
        }
        parts
    }
}
