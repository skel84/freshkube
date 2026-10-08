//! Menus that run the page's commands: a row's context menu (DESIGN.md
//! change 10) and, from the same entries, the others that show keys
//! ([docs/DESIGN.md](../../docs/DESIGN.md#menus-show-their-keys)).
//!
//! Each item dispatches the same action as the page's key and toolbar, so
//! the three never drift, and the menu draws each one's key as the
//! keymap binds it in the page's context, in the platform's form
//! (`⇧X` on macOS, `Shift+X` elsewhere).

use std::rc::Rc;

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{Action, App, Context, FocusHandle, SharedString};

/// One line of a row's menu.
pub enum MenuAction {
    /// An item that runs `action` on the page's list.
    Item {
        label: SharedString,
        action: Box<dyn Action>,
        enabled: bool,
    },
    Separator,
}

impl MenuAction {
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
/// While the menu is open, no row tooltip draws over it. An item runs its
/// action at once, while Kit holds the menu, so a handler must not update
/// the menu itself, such as by opening another.
pub fn actions(
    menu: PopupMenu,
    actions: Vec<MenuAction>,
    focus: &FocusHandle,
    live: impl Fn(&App) -> bool + 'static,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    crate::tooltip::hide_while_open(cx);
    let live = Rc::new(live);
    let mut menu = menu.action_context(focus.clone());
    let mut separate = false;
    let mut any = false;
    for action in actions {
        match action {
            MenuAction::Separator => separate = any,
            MenuAction::Item {
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
