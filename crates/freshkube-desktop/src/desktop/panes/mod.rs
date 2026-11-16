//! The panes of the window (#879). Pane 0 draws the page the shell shows;
//! pane 1, when the window is split, draws a Resources list of its own
//! (`second.rs`, `split.rs`).
//!
//! A pane has an id, the room it draws in (a [`Pane`] handle that its pages
//! ask for their width) and a frame drawn around its page in every state.
//! The frame carries the `Pane` key context, so a binding for the pane
//! works whether the page shows rows, a loading state or an error.
//!
//! **Element ids.** Pane 0 keeps every id it has always had. Every id drawn
//! in pane 1 is prefixed `p1:`, and only [`PaneId::id`] makes one, so no id
//! is shared between two panes.
use freshkube_ui::pane::Pane;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, ElementId, IntoElement, TestSupportExt, div};

/// The key context of a pane's frame. Not `Pane`: that is one of GPUI's
/// menu-bar default contexts on macOS, so a binding under it could fire from
/// anywhere.
pub(super) const PANE_CONTEXT: &str = "PaneFrame";

/// Which pane: 0 is the first, and until there is a split the only one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PaneId(u8);

impl PaneId {
    pub(super) const FIRST: PaneId = PaneId(0);
    pub(super) const SECOND: PaneId = PaneId(1);

    /// The id of an element `name` in this pane: the name itself in pane 0,
    /// `p1:name` in pane 1.
    pub(super) fn id(self, name: &'static str) -> ElementId {
        if self.0 == 0 {
            ElementId::from(name)
        } else {
            ElementId::from(format!("p{}:{name}", self.0))
        }
    }
}

mod second;
mod split;

pub(super) use second::{Second, Split};
pub(super) use split::{FocusOtherPane, SplitDown, SplitRight, bind_keys};

/// One pane: its id and its room.
struct Slot {
    id: PaneId,
    room: Pane,
}

/// The window's panes and which has the focus.
pub(super) struct Panes {
    slots: Vec<Slot>,
    focused: PaneId,
    /// Pane 1, while the window is split.
    pub(super) second: Option<Second>,
}

impl Panes {
    /// One pane, in the window's main pane's room.
    pub(super) fn new() -> Self {
        Self {
            slots: vec![Slot {
                id: PaneId::FIRST,
                room: Pane::new(),
            }],
            focused: PaneId::FIRST,
            second: None,
        }
    }

    /// The room of the shell's pane, pane 0, which the shell's chrome sets:
    /// the rail, column, header and status bar always follow it, split or
    /// not. The dock's height is the window's, so the shell sets it on every
    /// pane (`Pilot::render_page_cell`).
    pub(super) fn room(&self) -> Pane {
        self.slots
            .iter()
            .find(|slot| slot.id == PaneId::FIRST)
            .map(|slot| slot.room.clone())
            .expect("pane 0 is in the list")
    }

    pub(super) fn focus_first(&mut self) {
        self.focused = PaneId::FIRST;
    }

    pub(super) fn focus_second(&mut self) {
        if self.second.is_some() {
            self.focused = PaneId::SECOND;
        }
    }

    /// Marks pane 1's screen dirty, for state it reads from outside itself
    /// (the theme), as `Pages::notify_cached` does the shell's.
    pub(super) fn notify_second(&self, cx: &mut App) {
        if let Some(second) = &self.second {
            App::notify(cx, second.screen().view().entity_id());
        }
    }

    /// Whether pane 1 has the focus.
    pub(super) fn second_focused(&self) -> bool {
        self.focused == PaneId(1)
    }

    /// The frame of the first pane around `page`, drawn in every state.
    pub(super) fn frame(&self, page: AnyElement) -> impl IntoElement {
        div()
            .id(PaneId::FIRST.id("pane-frame"))
            .test_support()
            .key_context(PANE_CONTEXT)
            .size_full()
            .child(page)
    }
}

#[cfg(test)]
mod split_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_zero_keeps_its_ids_and_pane_one_prefixes_them() {
        assert_eq!(PaneId::FIRST.id("page-cell"), ElementId::from("page-cell"));
        assert_eq!(
            PaneId(1).id("page-cell"),
            ElementId::from("p1:page-cell".to_string())
        );
    }

    #[test]
    fn the_first_pane_is_focused_and_each_window_has_its_own_room() {
        let (one, other) = (Panes::new(), Panes::new());
        assert!(one.room().is(&one.room()), "the room is one shared handle");
        assert!(!one.room().is(&other.room()), "windows do not share it");
    }
}
