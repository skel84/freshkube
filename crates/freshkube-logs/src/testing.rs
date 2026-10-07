//! Read-outs for tests in crates that embed the view. The module builds
//! only with the `testing` feature, which those crates turn on from their
//! dev-dependencies, so no app code can call them.

use freshkube_core::logs::LogEntry;
use gpui_kit::{Pixels, Point};

use super::{LogSource, LogView};

impl<S: LogSource> LogView<S> {
    /// Lines applied to the review, and lines held back while hidden.
    pub fn applied_and_held(&self) -> (usize, usize) {
        (
            self.review.logs.buffer().entries().len(),
            self.backlog.len(),
        )
    }

    /// Every retained line, oldest first, whatever the filters.
    pub fn retained(&self) -> &[LogEntry] {
        self.review.logs.buffer().entries()
    }

    /// The rows the filters show, as indices into [`Self::retained`].
    pub fn visible_rows(&self) -> &[usize] {
        &self.review.visible
    }

    /// The line identity of visible row `ix`.
    pub fn row_id(&self, ix: usize) -> u64 {
        self.review.id(ix)
    }

    /// The current search match: its line identity, and its line within
    /// a message of several.
    pub fn current_match(&self) -> Option<(u64, Option<usize>)> {
        self.review.current_match.map(|hit| (hit.id, hit.line))
    }

    /// The level chips' counts: Error, Warn, Info, Debug, Unknown.
    pub fn level_counts(&self) -> [usize; 5] {
        self.review.level_counts()
    }

    pub fn following(&self) -> bool {
        self.following
    }

    pub fn anchor_evicted(&self) -> bool {
        self.anchor_evicted
    }

    pub fn scroll_offset(&self) -> Point<Pixels> {
        self.scroll.offset()
    }

    /// How far the whole panel can scroll: zero when its host gave it the
    /// least height it asked for.
    pub fn panel_max_offset(&self) -> Pixels {
        self.panel_scroll.max_offset().y
    }
}
