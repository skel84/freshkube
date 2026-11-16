//! The panes of the window (#879). One pane for now: pane 0, which draws
//! the page the shell shows.
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
use gpui_kit::{AnyElement, ElementId, IntoElement, TestSupportExt, div};

/// The key context of a pane's frame.
pub(super) const PANE_CONTEXT: &str = "Pane";

/// Which pane: 0 is the first, and until there is a split the only one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PaneId(u8);

impl PaneId {
    pub(super) const FIRST: PaneId = PaneId(0);

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

/// One pane: its id and its room.
struct Slot {
    id: PaneId,
    room: Pane,
}

/// The window's panes and which has the focus.
pub(super) struct Panes {
    slots: Vec<Slot>,
    focused: PaneId,
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
        }
    }

    /// The room of the focused pane, which the shell's chrome sets.
    pub(super) fn room(&self) -> Pane {
        self.slots
            .iter()
            .find(|slot| slot.id == self.focused)
            .map(|slot| slot.room.clone())
            .expect("the focused pane is in the list")
    }

    /// The frame of the first pane around `page`, drawn in every state.
    pub(super) fn frame(&self, page: AnyElement) -> impl IntoElement {
        div()
            .id(PaneId::FIRST.id("pane"))
            .test_support()
            .key_context(PANE_CONTEXT)
            .size_full()
            .child(page)
    }
}

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
    fn the_first_pane_is_focused_and_in_the_main_room() {
        let panes = Panes::new();
        assert!(panes.room().is(&Pane::new()));
    }
}
