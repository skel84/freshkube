//! A row of closable tabs, shared by the dock and the view tabs under the
//! header. The caller owns the tabs and what each action does; this module
//! draws the row, wires a click, a close button, a middle-click and a
//! right-click menu to the caller's methods, and keeps the popup in the UI
//! font whatever the selected tab's label looks like.

use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AnyView, App, Context, FontWeight, IntoElement, MouseButton, SharedString, Window,
    div,
};

use crate::dock;
use crate::inspector::TabStrip;

/// Draws a tab's tooltip, when the pointer rests on it.
pub type Tip = Rc<dyn Fn(&mut Window, &mut App) -> AnyView>;

/// Builds a tab's context menu; called each time the menu opens.
pub type MenuBuilder =
    Rc<dyn Fn(PopupMenu, u64, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

/// One tab of the row.
pub struct Item {
    pub id: u64,
    pub title: SharedString,
    pub active: bool,
    pub tip: Option<Tip>,
}

/// What the owner does with a tab's gestures. `V` is the owning view.
pub struct Actions<V> {
    /// A click.
    pub select: fn(&mut V, u64, &mut Window, &mut Context<V>),
    /// The × button and a middle-click.
    pub close: fn(&mut V, u64, &mut Window, &mut Context<V>),
    /// A right-press, before the menu opens: select and focus the tab.
    pub press_for_menu: fn(&mut V, u64, &mut Window, &mut Context<V>),
    pub menu: MenuBuilder,
}

/// The row: `prefix-row` holding `prefix-<id>` tabs, each with a
/// `prefix-<id>-close` button and a `prefix-<id>-menu` wrapper.
pub fn row<V: 'static>(
    prefix: &str,
    strip: &TabStrip,
    items: impl IntoIterator<Item = Item>,
    active: usize,
    actions: &Actions<V>,
    cx: &mut Context<V>,
) -> AnyElement {
    let (select, close_tab, press) = (actions.select, actions.close, actions.press_for_menu);
    strip
        .row(format!("{prefix}-row"), active)
        .children(items.into_iter().map(|item| {
            let id = item.id;
            let close = dock::close_button(format!("{prefix}-{id}-close"), &item.title).on_click(
                cx.listener(move |owner, _, window, cx| {
                    cx.stop_propagation();
                    close_tab(owner, id, window, cx);
                }),
            );
            let tip = item.tip;
            let tab = dock::tab(format!("{prefix}-{id}"), item.title, item.active, close, cx)
                .when_some(tip, |this, tip| {
                    this.tooltip(move |window, cx| tip(window, cx))
                })
                .on_click(cx.listener(move |owner, _, window, cx| select(owner, id, window, cx)))
                // Select and focus before Kit records the focus to restore.
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |owner, _, window, cx| press(owner, id, window, cx)),
                )
                // A middle-click closes the tab, as in a browser.
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |owner, _, window, cx| {
                        cx.stop_propagation();
                        close_tab(owner, id, window, cx);
                    }),
                );
            let menu = actions.menu.clone();
            // The popup inherits its trigger's text style. Keep it in the
            // UI font, outside the selected tab's semibold label.
            div()
                .id(format!("{prefix}-{id}-menu"))
                .font_family(cx.theme().font_family.clone())
                .font_weight(FontWeight::NORMAL)
                .child(tab)
                .context_menu(move |popup, window, cx| menu(popup, id, window, cx))
                .into_any_element()
        }))
        .into_any_element()
}
