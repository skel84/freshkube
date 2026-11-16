//! Pane 1: a Resources list of its own beside or under the shell's page.
use freshkube_ui::pane::Pane;
use gpui_kit::Subscription;

use super::PaneId;
use crate::resources::ResourcesScreen;
use gpui_kit::Entity;

/// Where pane 1 sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::desktop) enum Split {
    /// Beside pane 0, on its right.
    Right,
    /// Under pane 0.
    Down,
}

/// Pane 1. It owns one Resources screen, made when the window is split and
/// dropped when the split closes, with its own room and the subscriptions
/// that carry the screen's events to the shell.
pub(in crate::desktop) struct Second {
    pub(in crate::desktop) id: PaneId,
    pub(in crate::desktop) split: Split,
    pub(in crate::desktop) room: Pane,
    pub(in crate::desktop) screen: Entity<ResourcesScreen>,
    pub(in crate::desktop) _events: Vec<Subscription>,
}
