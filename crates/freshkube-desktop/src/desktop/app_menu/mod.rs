//! The app's menus ([docs/DESIGN.md](../../../../../docs/DESIGN.md#the-platform-module-and-the-app-menu)):
//! one list, built from the app's actions, that macOS draws as its menu bar
//! and elsewhere the header's menu button opens. Each entry runs the action
//! its key runs, on the focused view, and is greyed where nothing on the
//! focus path handles it.

use std::sync::Arc;

use freshkube_ui::menu::{self as shared, Find, FindNext, FindPrevious, MenuAction};
use freshkube_ui::platform::{self, About, AppMenu, OpenSettings, Platform, SystemMenu};
use freshkube_ui::text_size::{DefaultText, LargerText, SmallerText};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, AnyElement, App, Context, DismissEvent, FocusHandle, Focusable as _, FontWeight, Image,
    ImageFormat, Menu, MenuItem, MouseButton, OsAction, Role, TestSupportExt as _, Window, div,
    img,
};

use super::{
    Appearance, GoToKind, NextContext, NextScreen, Pilot, PreviousContext, PreviousScreen, Quit,
    Refresh, ShowEtcd, ShowEvents, ShowHealth, ShowLifecycle, ShowNamespaces, ShowNodes,
    ShowOverview, ShowSecurity, ShowSystemServices, ToggleColumn, dock,
};
use crate::ui::{self, dp};

gpui_kit::actions!(
    pilot,
    [
        /// View ▸ Appearance ▸ System.
        UseSystemAppearance,
        /// View ▸ Appearance ▸ Light.
        UseLightAppearance,
        /// View ▸ Appearance ▸ Dark.
        UseDarkAppearance,
    ]
);

/// What the menus' labels and checks state: they follow it, so the shell
/// sets the menus again when it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MenuState {
    /// Whether the sidebar's column shows.
    pub(crate) sidebar: bool,
    /// Whether the dock is open, not minimized to its bar.
    pub(crate) dock: bool,
    pub(crate) appearance: Appearance,
}

impl Default for MenuState {
    fn default() -> Self {
        Self {
            sidebar: true,
            dock: true,
            appearance: Appearance::System,
        }
    }
}

/// The app's menus on `platform`: Freshkube, Edit, View, Go and Window.
/// Off macOS the first holds Settings, About and Quit alone, which the
/// menu button puts after the others.
pub(crate) fn menus(platform: Platform, state: &MenuState) -> Vec<Menu> {
    vec![
        app(platform),
        menu(
            "Edit",
            [
                MenuItem::os_action("Undo", Undo, OsAction::Undo),
                MenuItem::os_action("Redo", Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", Cut, OsAction::Cut),
                MenuItem::os_action("Copy", Copy, OsAction::Copy),
                MenuItem::os_action("Paste", Paste, OsAction::Paste),
                MenuItem::os_action("Select all", SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action("Find", Find),
                MenuItem::action("Find next", FindNext),
                MenuItem::action("Find previous", FindPrevious),
            ],
        ),
        view(state),
        menu(
            "Go",
            [
                MenuItem::action("Search everything…", GoToKind),
                MenuItem::separator(),
                MenuItem::action("Overview", ShowOverview),
                MenuItem::action("Nodes", ShowNodes),
                MenuItem::action("Namespaces", ShowNamespaces),
                MenuItem::action("Events", ShowEvents),
                MenuItem::action("Health", ShowHealth),
                MenuItem::action("etcd", ShowEtcd),
                MenuItem::action("System services", ShowSystemServices),
                MenuItem::action("Security", ShowSecurity),
                MenuItem::action("Lifecycle", ShowLifecycle),
                MenuItem::separator(),
                MenuItem::action("Next screen", NextScreen),
                MenuItem::action("Previous screen", PreviousScreen),
                MenuItem::separator(),
                MenuItem::action("Next context", NextContext),
                MenuItem::action("Previous context", PreviousContext),
            ],
        ),
        menu(
            "Window",
            platform
                .system_items(SystemMenu::Window)
                .into_iter()
                .chain([
                    MenuItem::separator(),
                    MenuItem::action("Next tab", dock::NextDockTab),
                    MenuItem::action("Previous tab", dock::PreviousDockTab),
                    MenuItem::action("Close tab", dock::CloseDockTab),
                    MenuItem::action(
                        if state.dock {
                            "Minimize dock"
                        } else {
                            "Open dock"
                        },
                        dock::MinimizeDock,
                    ),
                ]),
        ),
    ]
}

/// The menu named after the app. macOS's has its system entries; off
/// macOS it holds what the button lists last.
fn app(platform: Platform) -> Menu {
    let system = platform.system_items(SystemMenu::App);
    if system.is_empty() {
        return menu(
            "Freshkube",
            [
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::action("About Freshkube", About),
                MenuItem::separator(),
                MenuItem::action("Quit Freshkube", Quit),
            ],
        );
    }
    menu(
        "Freshkube",
        [
            MenuItem::action("About Freshkube", About),
            MenuItem::separator(),
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
        ]
        .into_iter()
        .chain(system)
        .chain([
            MenuItem::separator(),
            MenuItem::action("Quit Freshkube", Quit),
        ]),
    )
}

fn view(state: &MenuState) -> Menu {
    let appearance = |label, action: Box<dyn Action>, appearance| MenuItem::Action {
        name: label,
        action,
        os_action: None,
        checked: state.appearance == appearance,
        disabled: false,
    };
    menu(
        "View",
        [
            MenuItem::action(
                if state.sidebar {
                    "Hide sidebar"
                } else {
                    "Show sidebar"
                },
                ToggleColumn,
            ),
            MenuItem::separator(),
            MenuItem::submenu(Menu::new("Appearance").items([
                appearance(
                    "System".into(),
                    Box::new(UseSystemAppearance),
                    Appearance::System,
                ),
                appearance(
                    "Light".into(),
                    Box::new(UseLightAppearance),
                    Appearance::Light,
                ),
                appearance("Dark".into(), Box::new(UseDarkAppearance), Appearance::Dark),
            ])),
            MenuItem::separator(),
            MenuItem::action("Bigger text", LargerText),
            MenuItem::action("Smaller text", SmallerText),
            MenuItem::action("Default text size", DefaultText),
            MenuItem::separator(),
            MenuItem::action("Refresh", Refresh),
        ],
    )
}

/// A menu of `items`, with no separator at either end or beside another,
/// as where a platform has no system entries.
fn menu(name: &'static str, items: impl IntoIterator<Item = MenuItem>) -> Menu {
    let mut tidy: Vec<MenuItem> = Vec::new();
    for item in items {
        let separator = matches!(item, MenuItem::Separator);
        if separator
            && tidy
                .last()
                .is_none_or(|last| matches!(last, MenuItem::Separator))
        {
            continue;
        }
        tidy.push(item);
    }
    if tidy
        .last()
        .is_some_and(|last| matches!(last, MenuItem::Separator))
    {
        tidy.pop();
    }
    Menu::new(name).items(tidy)
}

/// The menu button's entries: each menu but the app's as a submenu, then
/// the app's own entries. An entry is greyed unless `available` says its
/// action is handled, as macOS asks when it opens a menu.
pub(crate) fn button_entries(
    menus: Vec<Menu>,
    available: &mut dyn FnMut(&dyn Action) -> bool,
) -> Vec<MenuAction> {
    let mut menus = menus.into_iter();
    let app = menus.next();
    let mut out: Vec<MenuAction> = menus
        .map(|menu| MenuAction::submenu(menu.name, entries(menu.items, available)))
        .collect();
    if let Some(app) = app {
        out.push(MenuAction::Separator);
        out.extend(entries(app.items, available));
    }
    out
}

fn entries(
    items: Vec<MenuItem>,
    available: &mut dyn FnMut(&dyn Action) -> bool,
) -> Vec<MenuAction> {
    items
        .into_iter()
        .filter_map(|item| match item {
            MenuItem::Separator => Some(MenuAction::Separator),
            MenuItem::Submenu(menu) => Some(MenuAction::submenu(
                menu.name,
                entries(menu.items, available),
            )),
            MenuItem::SystemMenu(_) => None,
            MenuItem::Action {
                name,
                action,
                checked,
                ..
            } => Some(MenuAction::Item {
                enabled: available(action.as_ref()),
                label: name,
                action,
                checked: checked.then_some(true),
            }),
        })
        .collect()
}

/// The platform whose menu the window has ([`Pilot`]'s `menu_platform`).
pub(super) fn menu_platform() -> Platform {
    #[cfg(debug_assertions)]
    if std::env::var("FRESHKUBE_APP_MENU").as_deref() == Ok("button") {
        return Platform::Linux;
    }
    Platform::current()
}

impl Pilot {
    /// Sets the menus, and again whenever what they state changes: the
    /// shell notifies when the sidebar, the appearance or the dock it
    /// watches does, and a narrower window folds the column by itself.
    pub(super) fn watch_menus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_menus(window, cx);
        let shell = cx.entity();
        self._subscriptions.extend([
            cx.observe_in(&shell, window, |view, _, window, cx| {
                view.sync_menus(window, cx)
            }),
            cx.observe_window_bounds(window, |view, window, cx| view.sync_menus(window, cx)),
        ]);
    }

    fn sync_menus(&mut self, window: &Window, cx: &mut Context<Self>) {
        let state = MenuState {
            sidebar: self.area.has_column() && !self.column_collapsed(window),
            dock: self.dock.read(cx).is_open(),
            appearance: self.appearance,
        };
        if self.menu_state != Some(state) {
            self.menu_state = Some(state);
            cx.set_menus(menus(Platform::current(), &state));
        }
    }

    /// Opens the menu button's menu on what has the keyboard, each entry
    /// greyed as macOS greys its bar: asked of the app once this action is
    /// done with the window, so the app's own handlers count too.
    pub(super) fn open_app_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.app_menu_popup.is_some() {
            return;
        }
        // The button's popover has the keyboard by now, so the view is the one
        // its press recorded; F10 opens the menu on the view itself.
        let focus = self
            .app_menu_focus
            .take()
            .or_else(|| window.focused(cx))
            .unwrap_or_else(|| self.focus.clone());
        let menus = menus(self.menu_platform, &self.menu_state.unwrap_or_default());
        // The window answers for that view; the app's `is_action_available`
        // can't see the window it is called from, so it adds only the global
        // actions.
        let entries = button_entries(menus, &mut |action| {
            window.is_action_available_in(action, &focus) || cx.is_action_available(action)
        });
        self.show_app_menu(entries, focus, window, cx);
    }

    fn show_app_menu(
        &mut self,
        entries: Vec<MenuAction>,
        focus: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let popup = PopupMenu::build(window, cx, move |menu, window, cx| {
            shared::actions(menu, entries.clone(), &focus, |_| true, window, cx)
        });
        popup.focus_handle(cx).focus(window, cx);
        let closed = cx.subscribe_in(&popup, window, |view, _, _: &DismissEvent, _, cx| {
            view.app_menu_popup = None;
            cx.notify();
        });
        self.app_menu_popup = Some((popup, closed));
        cx.notify();
    }

    /// The menu button and its menu, where there is no menu bar.
    pub(super) fn render_menu_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.menu_platform.app_menu() != AppMenu::Button {
            return None;
        }
        let popup = self.app_menu_popup.as_ref().map(|(popup, _)| popup.clone());
        let shell = cx.entity().downgrade();
        Some(
            Popover::new("app-menu-popover")
                .appearance(false)
                .overlay_closable(false)
                .open(popup.is_some())
                .on_open_change({
                    let popup = popup.clone();
                    move |open, window, cx| match (*open, &popup) {
                        (true, _) => {
                            _ = shell.update(cx, |view, cx| view.open_app_menu(window, cx));
                        }
                        (false, Some(popup)) => popup.update(cx, |_, cx| cx.emit(DismissEvent)),
                        (false, None) => {}
                    }
                })
                .trigger(platform::menu_button("app-menu").on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|view, _, window, cx| view.app_menu_focus = window.focused(cx)),
                ))
                .content(move |_, _, _| match &popup {
                    Some(popup) => popup.clone().into_any_element(),
                    None => div().into_any_element(),
                })
                .into_any_element(),
        )
    }
}

/// About Freshkube: the mark, the name, the version and where it began.
pub(crate) fn open_about(window: &mut Window, cx: &mut App) {
    let mark = Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        include_bytes!("../../../../../packaging/icon/freshkube.svg").to_vec(),
    ));
    window.open_dialog(cx, move |dialog, window, cx| {
        let muted = cx.theme().muted_foreground;
        dialog
            .w(ui::dp_px(340., window).min(window.viewport_size().width - ui::dp_px(32., window)))
            .child(
                v_flex()
                    .id("about")
                    .test_support()
                    .role(Role::Dialog)
                    .aria_label("About Freshkube")
                    .items_center()
                    .gap(dp(6.))
                    .pb(dp(8.))
                    .child(img(mark.clone()).size(dp(64.)).mb(dp(6.)))
                    .child(
                        div()
                            .text_size(dp(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Freshkube"),
                    )
                    .child(
                        div()
                            .text_color(muted)
                            .child(concat!("Version ", env!("CARGO_PKG_VERSION"))),
                    )
                    .child(
                        h_flex()
                            .text_color(muted)
                            .text_size(dp(12.))
                            .child("Began as a fork of talos-pilot by Ken Udovic"),
                    ),
            )
    });
}

#[cfg(test)]
mod tests;
