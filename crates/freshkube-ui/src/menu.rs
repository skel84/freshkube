//! Menus that run the page's commands: a row's context menu (DESIGN.md
//! change 10) and, from the same entries, the others that show keys
//! ([docs/DESIGN.md](../../docs/DESIGN.md#menus-show-their-keys)).
//!
//! Each item dispatches the same action as the page's key and toolbar, so
//! the three never drift, and the menu draws each one's key as the
//! keymap binds it in the page's context, in the platform's form
//! (`⇧X` on macOS, `Shift+X` elsewhere).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{
    Action, App, Context, DismissEvent, EntityId, FocusHandle, Focusable as _, SharedString, Window,
};

gpui_kit::actions!(
    menu,
    [
        /// Edit ▸ Find: searches what has the keyboard.
        Find,
        /// Edit ▸ Find next.
        FindNext,
        /// Edit ▸ Find previous.
        FindPrevious,
    ]
);

thread_local! {
    /// The open menus [`return_focus`] watches, each with the lists its
    /// items run on: a "…" menu gathers one from each folded control.
    static WATCHED: RefCell<HashMap<EntityId, Rc<RefCell<Vec<FocusHandle>>>>> =
        RefCell::default();
}

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
    /// A submenu of more entries, run on the same list.
    Submenu {
        label: SharedString,
        items: Vec<MenuAction>,
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

    /// A submenu holding `items`.
    pub fn submenu(label: impl Into<SharedString>, items: Vec<MenuAction>) -> Self {
        Self::Submenu {
            label: label.into(),
            items,
        }
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
            Self::Submenu { label, items } => Self::Submenu {
                label: label.clone(),
                items: items.clone(),
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
    return_focus(Some(focus), window, cx);
    entries(menu, actions, focus, Rc::new(live), window, cx)
}

/// Adds `actions` to `menu` as [`actions`] does, a submenu's to a menu of
/// its own. Only the outer menu hands the keyboard back: a submenu closes
/// with it.
fn entries(
    menu: PopupMenu,
    actions: Vec<MenuAction>,
    focus: &FocusHandle,
    live: Rc<dyn Fn(&App) -> bool>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let mut menu = menu.action_context(focus.clone());
    let mut separate = false;
    let mut any = false;
    for action in actions {
        if separate && !matches!(action, MenuAction::Separator) {
            menu = menu.separator();
            separate = false;
        }
        match action {
            MenuAction::Separator => separate = any,
            MenuAction::Submenu { label, items } => {
                let (focus, live) = (focus.clone(), live.clone());
                menu = menu.submenu(label, window, cx, move |menu, window, cx| {
                    entries(menu, items.clone(), &focus, live.clone(), window, cx)
                });
                any = true;
            }
            MenuAction::Item {
                label,
                action,
                enabled,
                checked,
            } => {
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
/// While it's open, no row tooltip draws over it. A menu is watched once
/// while it's open, however many of its parts call this, and each call adds
/// its `focus`.
pub fn return_focus(focus: Option<&FocusHandle>, window: &mut Window, cx: &mut Context<PopupMenu>) {
    let id = cx.entity_id();
    if let Some(lists) = WATCHED.with_borrow(|watched| watched.get(&id).cloned()) {
        lists.borrow_mut().extend(focus.cloned());
        return;
    }
    let lists = Rc::new(RefCell::new(Vec::from_iter(focus.cloned())));
    WATCHED.with_borrow_mut(|watched| watched.insert(id, lists.clone()));
    cx.on_release(move |_, _| {
        WATCHED.with_borrow_mut(|watched| watched.remove(&id));
    })
    .detach();
    crate::tooltip::hide_while_open(cx);
    // Weak, so a view the menu closed, such as a tab's log, isn't kept or
    // focused again.
    let prior = window.focused(cx).map(|prior| prior.downgrade());
    let menu = cx.entity();
    cx.subscribe_in(
        &menu,
        window,
        move |menu, _, _: &DismissEvent, window, cx| {
            // Closed, the menu is watched no more: a part added after this
            // watches it again.
            WATCHED.with_borrow_mut(|watched| watched.remove(&id));
            let Some(prior) = prior.as_ref().and_then(|prior| prior.upgrade()) else {
                return;
            };
            let left = match window.focused(cx) {
                None => true,
                Some(now) if now == prior => false,
                Some(now) => {
                    lists.borrow().contains(&now)
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::Root;
    use gpui_kit::{AppContext as _, IntoElement, Render, TestAppContext, div, px, size};

    struct Empty;

    impl Render for Empty {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    fn watched(id: EntityId) -> bool {
        WATCHED.with_borrow(|watched| watched.contains_key(&id))
    }

    /// A menu is watched while it's open, and no longer once it closes,
    /// though something still holds it.
    #[gpui_kit::test]
    fn a_closed_menu_is_watched_no_more(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            cx.set_reduce_motion(true);
        });
        let handle = cx.open_window(size(px(400.), px(300.)), |window, cx| {
            let view = cx.new(|_| Empty);
            Root::new(view, window, cx)
        });
        let menu = cx
            .update_window(handle.into(), |_, window, cx| {
                PopupMenu::build(window, cx, |menu, window, cx| {
                    return_focus(None, window, cx);
                    menu
                })
            })
            .unwrap();
        let id = menu.entity_id();
        assert!(watched(id));
        cx.update(|cx| menu.update(cx, |_, cx| cx.emit(DismissEvent)));
        cx.run_until_parked();
        assert!(!watched(id));
    }
}
