//! The split's other pane: a page map, its own room and the screen it shows.
use std::collections::BTreeMap;

use freshkube_ui::pane::Pane;
use gpui_kit::Subscription;

use super::PaneId;
use crate::desktop::PageId;
use crate::screens::ScreenHandle;

/// Where the other pane sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::desktop) enum Split {
    /// Beside pane 0, on its right.
    Right,
    /// Under pane 0.
    Down,
}

/// Pane 1, the general pane the split grows into: a map of the pages it
/// owns (only Resources today), which one shows, its own room and the
/// subscriptions that carry its pages' events to the shell. It is made when
/// the window is split and dropped when the split closes, and its pages'
/// reads go with it.
pub(in crate::desktop) struct Second {
    pub(in crate::desktop) id: PaneId,
    pub(in crate::desktop) split: Split,
    pub(in crate::desktop) room: Pane,
    pub(in crate::desktop) pages: BTreeMap<PageId, ScreenHandle>,
    pub(in crate::desktop) shown: PageId,
    pub(in crate::desktop) _events: Vec<Subscription>,
}

impl Second {
    /// The page the pane shows.
    pub(in crate::desktop) fn screen(&self) -> &ScreenHandle {
        &self.pages[&self.shown]
    }
}
