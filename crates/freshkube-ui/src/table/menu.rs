//! A row's context menu, built from the page's actions (DESIGN.md change 10).
//!
//! Each item dispatches the same action as the page's key and toolbar, so
//! the three never drift, and the menu draws each one's key as the
//! keymap binds it in the page's context, in the platform's form
//! (`⇧X` on macOS, `Shift+X` elsewhere).

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::{Action, FocusHandle, SharedString};

/// One line of a row's menu.
pub enum RowAction {
    /// An item that dispatches `action` on the page's focus.
    Item {
        label: SharedString,
        action: Box<dyn Action>,
        enabled: bool,
    },
    Separator,
}

impl RowAction {
    pub fn new(label: impl Into<SharedString>, action: impl Action) -> Self {
        Self::Item {
            label: label.into(),
            action: Box::new(action),
            enabled: true,
        }
    }

    /// Shown, but greyed out and inert.
    pub fn enabled(self, enabled: bool) -> Self {
        match self {
            Self::Item { label, action, .. } => Self::Item {
                label,
                action,
                enabled,
            },
            Self::Separator => Self::Separator,
        }
    }
}

/// Adds `actions` to `menu`, dispatched to `focus`: the page's list, where
/// their keys are bound, so each item shows its key and runs its handler.
/// A separator at either end or next to another is dropped.
pub fn row_menu(menu: PopupMenu, actions: Vec<RowAction>, focus: &FocusHandle) -> PopupMenu {
    let mut menu = menu.action_context(focus.clone());
    let mut separate = false;
    let mut any = false;
    for action in actions {
        match action {
            RowAction::Separator => separate = any,
            RowAction::Item {
                label,
                action,
                enabled,
            } => {
                if separate {
                    menu = menu.separator();
                    separate = false;
                }
                menu = menu.menu_with_disabled(label, action, !enabled);
                any = true;
            }
        }
    }
    menu
}
