//! View tabs (#877): several places of one cluster open at once under the
//! header, each a `ViewState` with a history of its own.
//!
//! `set.rs` is the data and its rules, with no GPUI types. `strip.rs` is the
//! strip: a cached view of its own beside the page, derived when a tab
//! changes, so a page redraws without it and a hover over it redraws only
//! it. `commands.rs` is what `Pilot` does with the set: open, select, close,
//! reopen, step, keep the shown tab's view current, and the keys.
//! `restore.rs` saves the views in `navigation.json` and reads them back.
//! Frozen rows are in `frozen.rs`.
//!
//! The strip shows from two tabs; one tab is the window as it always was.
use super::*;
use freshkube_nav::ViewState;
use std::time::Duration;

mod choose;
#[cfg(test)]
mod click_tests;
mod commands;
#[cfg(test)]
mod guard_tests;
mod restore;
mod set;
#[cfg(test)]
mod set_tests;
mod strip;
#[cfg(test)]
mod tests;

pub(super) use commands::bind_keys;
pub(super) use commands::{destination_view, node_view, object_view};
pub(crate) use set::OpenIn;
pub(super) use set::TabSet;
pub(super) use strip::ViewTabs;

gpui_kit::actions!(
    tabs,
    [
        /// ⌘T: a new tab at the view that is shown.
        DuplicateTab,
        /// ⌘W: closes the shown tab, unless the dock has the keyboard.
        CloseViewTab,
        /// ⇧⌘T: brings back the tab closed last.
        ReopenViewTab,
        /// ⌘⇧] and ⌃⇥: the next tab.
        NextViewTab,
        /// ⌘⇧[ and ⌃⇧⇥: the previous tab.
        PreviousViewTab,
        /// ⌘[: the shown tab's previous view.
        TabBack,
        /// ⌘]: the view the tab left by going back.
        TabForward,
    ]
);

/// How long a click's modifiers stay the answer to the link it makes. A
/// link arrives after the click's own update ends; a key that opens a link
/// later is not that click's.
const GESTURE_LIFE: Duration = Duration::from_millis(500);

/// How long after a tab is shown its page may take to arrive before the
/// shell stops waiting for it and follows the page again.
const SETTLE: Duration = Duration::from_secs(2);

/// What the shell keeps for tabs.
pub(super) struct Tabs {
    pub(super) set: TabSet,
    /// The strip's view, drawn beside the page.
    pub(super) strip: Entity<ViewTabs>,
    /// How the last click on the window was modified, until its link.
    gesture: Option<(OpenIn, std::time::Instant)>,
    /// The view a tab was just shown at, until the page reaches it.
    applying: Option<(ViewState, std::time::Instant)>,
    /// The entry a tab just shown is being opened on, until the window shows
    /// it: the tab's own, never a fallback (#878).
    activating: Option<String>,
    /// A person picked a cluster for a tab whose entry is gone.
    chosen: bool,
    /// The cluster the set belongs to: its identity once one has opened.
    cluster: Option<String>,
    /// The tabs of the last run, waiting for a cluster to open them in.
    restore: Option<restore::Saved>,
    /// Saves the views shortly after the last change.
    save: Option<Task<()>>,
    /// While a tab is shown: the namespace to list, when the object it opens
    /// is in another (`Tab::list_ns`). `apply_kind` takes it.
    pub(super) list_override: Option<Option<String>>,
    /// The rows each Resources tab kept when it stopped showing.
    pub(super) frozen: freshkube_resources::frozen::FrozenSet<set::TabId>,
    /// What the shell last drew of the tabs (`answers`).
    answered: std::cell::Cell<Option<[bool; 4]>>,
}

impl Tabs {
    pub(super) fn new(
        resources: &Entity<crate::resources::ResourcesScreen>,
        nodes: &Entity<crate::desktop::nodes::Nodes>,
        cx: &mut Context<Pilot>,
    ) -> Self {
        let restore = restore::Saved::read(cx);
        let pilot = cx.entity();
        let strip = cx.new(|cx| ViewTabs::new(&pilot, resources, nodes, cx));
        Self {
            set: TabSet::new(ViewState::page(freshkube_nav::LinkPage::Overview)),
            strip,
            gesture: None,
            applying: None,
            activating: None,
            chosen: false,
            cluster: None,
            restore,
            save: None,
            list_override: None,
            frozen: Default::default(),
            answered: Default::default(),
        }
    }

    /// What the shell draws and answers for the tabs: whether the strip
    /// shows and whether reopening, going back and going forward can act.
    pub(super) fn answers(&self) -> [bool; 4] {
        let tab = self.set.current();
        [
            self.shows(),
            self.set.can_reopen(),
            tab.can_go_back(),
            tab.can_go_forward(),
        ]
    }

    /// Whether the strip shows: from two tabs.
    pub(super) fn shows(&self) -> bool {
        self.set.len() >= 2
    }
}
