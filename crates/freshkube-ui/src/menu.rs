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
use gpui_kit::{
    Action, App, Context, DismissEvent, FocusHandle, Focusable as _, SharedString, Window,
};

/// One line of a menu that runs the page's commands.
pub enum MenuAction {
    /// An item that runs `action` on the page's list.
    Item {
        label: SharedString,
        action: Box<dyn Action>,
        enabled: bool,
        /// For a toggle, whether it is on.
        checked: Option<bool>,
    },
    Separator,
}

impl MenuAction {
    pub fn new(label: impl Into<SharedString>, action: impl Action) -> Self {
        Self::Item {
            label: label.into(),
            action: Box::new(action),
            enabled: true,
            checked: None,
        }
    }

    /// Shown, but greyed out and inert.
    pub fn enabled(mut self, enabled: bool) -> Self {
        if let Self::Item { enabled: on, .. } = &mut self {
            *on = enabled;
        }
        self
    }

    /// A toggle, checked while `checked`.
    pub fn checked(mut self, checked: bool) -> Self {
        if let Self::Item { checked: on, .. } = &mut self {
            *on = Some(checked);
        }
        self
    }
}

impl Clone for MenuAction {
    fn clone(&self) -> Self {
        match self {
            Self::Item {
                label,
                action,
                enabled,
                checked,
            } => Self::Item {
                label: label.clone(),
                action: action.boxed_clone(),
                enabled: *enabled,
                checked: *checked,
            },
            Self::Separator => Self::Separator,
        }
    }
}

/// Adds `actions` to `menu`, run on `focus`: the page's list, where their
/// keys are bound, so each item shows its key and runs the key's handler.
/// A menu opened from a button takes the same focus, not the button's.
/// The menu stays open while the list changes under it, so an item acts
/// only while `live` holds: the row it was opened for is still listed and
/// selected. Otherwise it does nothing, rather than act on whatever row
/// took its place. A separator at either end or next to another is dropped.
/// While the menu is open, no row tooltip draws over it, and once it
/// closes the keyboard goes back where it was ([`return_focus`]). An item
/// runs its action at once, while Kit holds the menu, so a handler must
/// not update the menu itself, such as by opening another.
pub fn actions(
    menu: PopupMenu,
    actions: Vec<MenuAction>,
    focus: &FocusHandle,
    live: impl Fn(&App) -> bool + 'static,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    crate::tooltip::hide_while_open(cx);
    return_focus(Some(focus), window, cx);
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
                checked,
            } => {
                if separate {
                    menu = menu.separator();
                    separate = false;
                }
                let (live, focus, run) = (live.clone(), focus.clone(), action.boxed_clone());
                let mut item = PopupMenuItem::new(label)
                    .action(action)
                    .disabled(!enabled)
                    .on_click(move |_, window, cx| {
                        if live(cx) {
                            // At once, on the list: the key's own path.
                            focus.focus(window, cx);
                            focus.dispatch_action(run.as_ref(), window, cx);
                        }
                    });
                if let Some(checked) = checked {
                    item = item.checked(checked);
                }
                menu = menu.item(item);
                any = true;
            }
        }
    }
    menu
}

/// Once `menu` closes, hands the keyboard back to what had it when the
/// menu opened, as Escape or a click beside it closes it and as an item
/// that ran on `focus` leaves it: Kit would leave it on `focus`, the
/// entries' list, or on the menu that's gone, where no page key works. An
/// item that moved it on, as one that opens a dialog does, keeps it there.
pub fn return_focus(focus: Option<&FocusHandle>, window: &mut Window, cx: &mut Context<PopupMenu>) {
    let Some(prior) = window.focused(cx) else {
        return;
    };
    // Weak, so a view the menu closed, such as a tab's log, isn't kept or
    // focused again.
    let prior = prior.downgrade();
    let focus = focus.cloned();
    let menu = cx.entity();
    cx.subscribe_in(
        &menu,
        window,
        move |menu, _, _: &DismissEvent, window, cx| {
            let Some(prior) = prior.upgrade() else {
                return;
            };
            let left = match window.focused(cx) {
                None => true,
                Some(now) if now == prior => false,
                Some(now) => {
                    Some(&now) == focus.as_ref()
                        || menu.focus_handle(cx).contains_focused(window, cx)
                }
            };
            if left {
                prior.focus(window, cx);
            }
        },
    )
    .detach();
}
