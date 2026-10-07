//! A row's context menu, built from the page's actions (DESIGN.md change 10).
//!
//! Each item dispatches the same action as the page's key and toolbar, so
//! the three never drift, and the menu draws each one's key as the
//! keymap binds it in the page's context, in the platform's form
//! (`⇧X` on macOS, `Shift+X` elsewhere).

use std::rc::Rc;

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{Action, App, FocusHandle, SharedString};

/// One line of a row's menu.
pub enum RowAction {
    /// An item that runs `action` on the page's list.
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

/// Adds `actions` to `menu`, run on `focus`: the page's list, where their
/// keys are bound, so each item shows its key and runs the key's handler.
/// The menu stays open while the list changes under it, so an item acts
/// only while `live` holds: the row it was opened for is still listed and
/// selected. Otherwise it does nothing, rather than act on whatever row
/// took its place. A separator at either end or next to another is dropped.
pub fn row_menu(
    menu: PopupMenu,
    actions: Vec<RowAction>,
    focus: &FocusHandle,
    live: impl Fn(&App) -> bool + 'static,
) -> PopupMenu {
    let live = Rc::new(live);
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
                let (live, focus, run) = (live.clone(), focus.clone(), action.boxed_clone());
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .action(action)
                        .disabled(!enabled)
                        .on_click(move |_, window, cx| {
                            if live(cx) {
                                // At once, on the list: the key's own path.
                                focus.focus(window, cx);
                                focus.dispatch_action(run.as_ref(), window, cx);
                            }
                        }),
                );
                any = true;
            }
        }
    }
    menu
}
