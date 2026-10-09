use super::*;
use crate::desktop::tests::fixture;
use crate::resources::{Tab, example, live};
use freshkube_core::resources::builtin;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, Entity, OwnedMenu, OwnedMenuItem, TestAppContext};

/// A menu's entries as text: a label, "✓" after a checked one, "—" for a
/// separator, "▸" after a submenu's name.
fn shown(menu: &OwnedMenu) -> Vec<String> {
    menu.items
        .iter()
        .map(|item| match item {
            OwnedMenuItem::Separator => "—".to_owned(),
            OwnedMenuItem::Submenu(menu) => format!("{} ▸", menu.name),
            OwnedMenuItem::SystemMenu(menu) => format!("{} ▸", menu.name),
            OwnedMenuItem::Action { name, checked, .. } if *checked => format!("{name} ✓"),
            OwnedMenuItem::Action { name, .. } => name.clone(),
        })
        .collect()
}

fn owned(platform: Platform, state: &MenuState) -> Vec<OwnedMenu> {
    menus(platform, state)
        .into_iter()
        .map(Menu::owned)
        .collect()
}

fn named<'a>(menus: &'a [OwnedMenu], name: &str) -> &'a OwnedMenu {
    menus.iter().find(|menu| menu.name == name).unwrap()
}

/// The button's entries as text, as [`shown`] writes a menu's, greyed ones
/// in brackets.
fn listed(entries: &[MenuAction]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| match entry {
            MenuAction::Separator => "—".to_owned(),
            MenuAction::Submenu { label, .. } => format!("{label} ▸"),
            MenuAction::Item {
                label,
                enabled: false,
                ..
            } => format!("[{label}]"),
            MenuAction::Item { label, .. } => label.to_string(),
        })
        .collect()
}

/// Every platform has the same menus; macOS's alone hold its system
/// entries, and elsewhere the app's menu holds Settings, About and Quit,
/// which the button lists after the others. No menu starts or ends with a
/// separator, or has two together, where the system entries are missing.
#[test]
fn the_menus_differ_only_by_the_systems_entries() {
    let state = MenuState::default();
    let mac = owned(Platform::MacOs, &state);
    let names: Vec<_> = mac.iter().map(|menu| menu.name.to_string()).collect();
    assert_eq!(names, ["Freshkube", "Edit", "View", "Go", "Window"]);
    assert_eq!(
        shown(named(&mac, "Freshkube")),
        [
            "About Freshkube",
            "—",
            "Settings…",
            "—",
            "Services ▸",
            "—",
            "Hide Freshkube",
            "Hide others",
            "Show all",
            "—",
            "Quit Freshkube"
        ]
    );
    assert_eq!(
        shown(named(&mac, "Window")),
        [
            "Minimize",
            "Zoom",
            "—",
            "Next tab",
            "Previous tab",
            "Close tab",
            "Minimize dock"
        ]
    );
    assert_eq!(
        shown(named(&mac, "Edit")),
        [
            "Undo",
            "Redo",
            "—",
            "Cut",
            "Copy",
            "Paste",
            "Select all",
            "—",
            "Find",
            "Find next",
            "Find previous"
        ]
    );
    assert_eq!(
        shown(named(&mac, "View")),
        [
            "Hide sidebar",
            "—",
            "Appearance ▸",
            "—",
            "Bigger text",
            "Smaller text",
            "Default text size",
            "—",
            "Refresh"
        ]
    );
    for platform in [Platform::Windows, Platform::Linux] {
        let menus = owned(platform, &state);
        assert_eq!(menus.len(), 5, "{platform:?}");
        assert_eq!(
            shown(named(&menus, "Freshkube")),
            ["Settings…", "About Freshkube", "—", "Quit Freshkube"]
        );
        assert_eq!(
            shown(named(&menus, "Window")),
            ["Next tab", "Previous tab", "Close tab", "Minimize dock"]
        );
        for name in ["Edit", "View", "Go"] {
            assert_eq!(
                shown(named(&menus, name)),
                shown(named(&mac, name)),
                "{platform:?}"
            );
        }
    }
}

/// The button lists each menu but the app's as a submenu, then the app's
/// entries, and greys what isn't handled.
#[test]
fn the_button_lists_the_menus_then_the_apps_entries() {
    let menus = menus(Platform::Linux, &MenuState::default());
    let entries = button_entries(menus, &mut |action| !action.as_any().is::<Paste>());
    assert_eq!(
        listed(&entries),
        [
            "Edit ▸",
            "View ▸",
            "Go ▸",
            "Window ▸",
            "—",
            "Settings…",
            "About Freshkube",
            "—",
            "Quit Freshkube"
        ]
    );
    let MenuAction::Submenu { items, .. } = &entries[0] else {
        panic!("Edit is a submenu");
    };
    assert!(listed(items).contains(&"[Paste]".to_owned()));
    assert!(listed(items).contains(&"Copy".to_owned()));
}

/// The example shell on Pods, its window active: macOS asks the active
/// window what an entry would run on.
fn on_pods(cx: &mut TestAppContext) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        crate::desktop::tests::open_kind(window, cx, "pods")
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert!(cx.update(|cx| cx.active_window()) == Some(handle));
    (runtime, handle, pilot)
}

fn draw(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// The menus as the app set them last.
fn installed(cx: &mut TestAppContext) -> Vec<OwnedMenu> {
    cx.update(|cx| cx.get_menus()).expect("the menus are set")
}

fn dispatch(handle: AnyWindowHandle, action: impl Action, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(action), cx)
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
}

fn running_pod(
    pilot: &Entity<Pilot>,
    cx: &mut TestAppContext,
) -> crate::resources::model::ResourceIdentity {
    let context = cx.update(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let (_, rows) = example::read(&context, "pods", None, live::now()).unwrap();
    rows.into_iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity
}

/// The sidebar's entry says what it does next, the dock's too, and the
/// appearance shown is checked: each follows its state as it changes, by
/// key, by menu or by a click.
#[gpui_kit::test]
fn the_labels_follow_the_sidebar_the_dock_and_the_appearance(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = on_pods(cx);
    let view = |cx: &mut TestAppContext| shown(named(&installed(cx), "View"));
    let window = |cx: &mut TestAppContext| shown(named(&installed(cx), "Window"));
    assert_eq!(view(cx)[0], "Hide sidebar");

    dispatch(handle, ToggleColumn, cx);
    assert_eq!(view(cx)[0], "Show sidebar");
    dispatch(handle, ToggleColumn, cx);
    assert_eq!(view(cx)[0], "Hide sidebar");

    let appearance = |cx: &mut TestAppContext| {
        let menus = installed(cx);
        let view = named(&menus, "View");
        let submenu = view
            .items
            .iter()
            .find_map(|item| match item {
                OwnedMenuItem::Submenu(menu) if menu.name == "Appearance" => Some(menu),
                _ => None,
            })
            .unwrap();
        shown(submenu)
    };
    assert_eq!(appearance(cx), ["System ✓", "Light", "Dark"]);
    dispatch(handle, UseDarkAppearance, cx);
    assert_eq!(appearance(cx), ["System", "Light", "Dark ✓"]);
    assert!(cx.update(|cx| cx.theme().mode.is_dark()));

    let pod = running_pod(&pilot, cx);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(builtin("pods").unwrap(), pod.into(), Tab::Logs, window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert_eq!(window(cx).last().unwrap(), "Minimize dock");
    // Shift-Escape minimizes the dock and opens it again.
    dispatch(handle, dock::MinimizeDock, cx);
    assert!(!cx.update(|cx| pilot.read(cx).dock.read(cx).is_open()));
    assert_eq!(window(cx).last().unwrap(), "Open dock");
    dispatch(handle, dock::MinimizeDock, cx);
    assert!(cx.update(|cx| pilot.read(cx).dock.read(cx).is_open()));
    assert_eq!(window(cx).last().unwrap(), "Minimize dock");
}

/// Whether an entry for `action` is live: what macOS asks before it opens a
/// menu, and the menu button when it opens.
fn live(action: impl Action, cx: &mut TestAppContext) -> bool {
    cx.update(|cx| cx.is_action_available(&action))
}

/// Edit's entries are live only where the focused view handles them: on
/// a table nothing pastes, cuts or selects all, and in its filter Kit's
/// input does. The check sees the page's handlers through Kit's Root, not
/// only Root's own.
#[gpui_kit::test]
fn edit_greys_what_the_focused_view_does_not_handle(cx: &mut TestAppContext) {
    let (_runtime, handle, _pilot) = on_pods(cx);
    let focused = |id: &'static str, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, _| window.find(id).focused())
            .unwrap()
    };
    assert_eq!(focused("resource-body", cx), Some(true));
    assert!(!live(Paste, cx));
    assert!(!live(Cut, cx));
    assert!(!live(SelectAll, cx));
    assert!(!live(Undo, cx));
    // Kit's Root answers Copy in every window, copying the window's
    // selected text, so it stays live on a table (DESIGN.md).
    assert!(live(Copy, cx));
    // Find is the next step's.
    assert!(!live(Find, cx));
    // The shell's own entries are live here.
    assert!(live(Refresh, cx));
    assert!(live(ToggleColumn, cx));

    cx.update_window(handle, |_, window, cx| window.click("resource-filter", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert_eq!(focused("resource-filter", cx), Some(true));
    assert!(live(Paste, cx));
    assert!(live(Cut, cx));
    assert!(live(SelectAll, cx));
    assert!(live(Undo, cx));
    assert!(live(Copy, cx));
    assert!(!live(Find, cx));
}

/// Where there is no menu bar, the header's menu button opens the menus
/// on what has the keyboard, F10's action too; Escape closes them and hands
/// the keyboard back. On macOS there is no button.
#[gpui_kit::test]
fn the_menu_button_opens_the_menus_where_there_is_no_bar(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = on_pods(cx);
    cx.update(|cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.menu_platform = Platform::MacOs;
            cx.notify();
        })
    });
    cx.run_until_parked();
    draw(handle, cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("app-menu").is_none(), "macOS has its bar");
    })
    .unwrap();

    cx.update(|cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.menu_platform = Platform::Linux;
            cx.notify();
        })
    });
    cx.run_until_parked();
    draw(handle, cx);
    let labels = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, _| {
            let menu = window.within("popup-menu");
            (0usize..)
                .map_while(|ix| menu.try_find(ix))
                .filter_map(|item| item.label().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap()
    };
    cx.update_window(handle, |_, window, cx| window.click("app-menu", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert!(cx.update(|cx| pilot.read(cx).app_menu_popup.is_some()));
    assert_eq!(
        labels(cx),
        [
            "Edit",
            "View",
            "Go",
            "Window",
            "Settings…",
            "About Freshkube",
            "Quit Freshkube"
        ]
    );
    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert!(cx.update(|cx| pilot.read(cx).app_menu_popup.is_none()));
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("resource-body").focused(), Some(true));
    })
    .unwrap();

    // An entry the shell answers is live: a snapshot can't say so, so pick
    // it and see Settings open.
    cx.update_window(handle, |_, window, cx| window.click("app-menu", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    cx.update_window(handle, |_, window, cx| {
        let mut menu = window.within("popup-menu");
        let settings = (0usize..)
            .map_while(|ix| menu.try_find(ix).map(|item| (ix, item)))
            .find(|(_, item)| item.label() == Some("Settings…"))
            .map(|(ix, _)| ix)
            .expect("Settings… in the menu");
        menu.click(settings, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.update(|cx| pilot.read(cx).settings_open));
    draw(handle, cx);
    assert!(cx.update(|cx| pilot.read(cx).app_menu_popup.is_none()));

    dispatch(handle, freshkube_ui::platform::OpenAppMenu, cx);
    assert!(cx.update(|cx| pilot.read(cx).app_menu_popup.is_some()));
}

/// Settings… opens the header's Settings and About its dialog.
#[gpui_kit::test]
fn settings_and_about_open_from_their_actions(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    draw(handle, cx);
    dispatch(handle, OpenSettings, cx);
    assert!(cx.update(|cx| pilot.read(cx).settings_open));
    cx.update(|cx| pilot.update(cx, |pilot, _| pilot.settings_open = false));
    dispatch(handle, About, cx);
    cx.update_window(handle, |_, window, cx| {
        assert!(window.has_active_dialog(cx));
        assert!(window.try_find("about").is_some());
    })
    .unwrap();
}

/// Pods with the menu button, as on Windows and Linux.
fn with_button(
    cx: &mut TestAppContext,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, pilot) = on_pods(cx);
    cx.update(|cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.menu_platform = Platform::Linux;
            cx.notify();
        })
    });
    cx.run_until_parked();
    draw(handle, cx);
    (runtime, handle, pilot)
}

/// Picks Edit ▸ `label` from the menu button's menu.
fn pick_from_edit(handle: AnyWindowHandle, label: &str, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| window.click("app-menu", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    cx.update_window(handle, |_, window, cx| {
        window.within("popup-menu").hover(0usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    cx.update_window(handle, |_, window, cx| {
        let mut edit = window.within("submenu");
        let entry = (0usize..)
            .map_while(|ix| edit.try_find(ix).map(|item| (ix, item)))
            .find(|(_, item)| item.label() == Some(label))
            .map(|(ix, _)| ix)
            .unwrap_or_else(|| panic!("{label} in Edit"));
        edit.click(entry, cx);
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
}

/// The button's menu greys what the view that had the keyboard doesn't
/// handle, as the bar does. A snapshot can't say an entry is grey, so pick
/// Paste: grey over the table, it does nothing and the menu stays open;
/// live over the filter, it pastes and the menu closes.
#[gpui_kit::test]
fn the_buttons_menu_greys_what_the_view_does_not_handle(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = with_button(cx);
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("web".to_owned()));
    let filter = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, _| {
            window.find("resource-filter").value().map(str::to_owned)
        })
        .unwrap()
    };
    let open = |cx: &mut TestAppContext| cx.update(|cx| pilot.read(cx).app_menu_popup.is_some());

    pick_from_edit(handle, "Paste", cx);
    assert!(open(cx), "Paste is grey over the table");
    assert_ne!(filter(cx).as_deref(), Some("web"));
    for _ in 0..2 {
        cx.update_window(handle, |_, window, cx| window.press("escape", cx))
            .unwrap();
        cx.run_until_parked();
        draw(handle, cx);
    }
    assert!(!open(cx));

    cx.update_window(handle, |_, window, cx| window.click("resource-filter", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    pick_from_edit(handle, "Paste", cx);
    assert!(!open(cx), "Paste is live over the filter");
    assert_eq!(filter(cx).as_deref(), Some("web"));
}

/// F10 opens the menu with the keyboard on it, not on the button's
/// popover, which would otherwise take it as it opens, and Escape gives
/// it back.
#[gpui_kit::test]
fn f10_puts_the_keyboard_on_the_menu(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = with_button(cx);
    dispatch(handle, freshkube_ui::platform::OpenAppMenu, cx);
    cx.run_until_parked();
    draw(handle, cx);
    let popup = cx.update(|cx| {
        pilot
            .read(cx)
            .app_menu_popup
            .as_ref()
            .map(|(popup, _)| popup.clone())
    });
    let popup = popup.expect("F10 opens the menu");
    cx.update_window(handle, |_, window, cx| {
        assert!(popup.focus_handle(cx).is_focused(window));
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
    assert!(cx.update(|cx| pilot.read(cx).app_menu_popup.is_none()));
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("resource-body").focused(), Some(true));
    })
    .unwrap();
}

/// macOS's bar shows, and binds, an entry's first binding that no context
/// of its own picks out, and GPUI hands AppKit the key's name unless it
/// maps it: AppKit keeps the first character, so `ctrl-tab` would show
/// and bind ⌃T. Every entry's key is one character or a name GPUI maps.
/// The screens' Tab is the character, which AppKit shows as no key.
#[gpui_kit::test]
fn the_bar_shows_keys_appkit_reads(cx: &mut TestAppContext) {
    // As GPUI's macOS backend chooses (`gpui-pre-macos` `platform.rs`).
    let mut defaults = gpui_kit::KeyContext::new_with_defaults();
    for name in ["Workspace", "Pane", "Editor"] {
        defaults.add(name);
    }
    // The names `key_to_native` maps; it passes any other on as it is.
    const MAPPED: &[&str] = &[
        "space",
        "backspace",
        "escape",
        "up",
        "down",
        "left",
        "right",
        "pageup",
        "pagedown",
        "home",
        "end",
        "delete",
        "insert",
    ];
    // F1 to F35, which GPUI maps too.
    fn function_key(key: &str) -> bool {
        key.strip_prefix('f')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    }
    fn actions(items: &[OwnedMenuItem], out: &mut Vec<(String, Box<dyn Action>)>) {
        for item in items {
            match item {
                OwnedMenuItem::Submenu(menu) => actions(&menu.items, out),
                OwnedMenuItem::Action { name, action, .. } => {
                    out.push((name.clone(), action.boxed_clone()))
                }
                _ => {}
            }
        }
    }
    let _app = on_pods(cx);
    let mut entries = Vec::new();
    for menu in owned(Platform::MacOs, &MenuState::default()) {
        actions(&menu.items, &mut entries);
    }
    let unread: Vec<String> = cx.update(|cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        entries
            .iter()
            .filter_map(|(name, action)| {
                let mut bindings = keymap.bindings_for_action(action.as_ref()).peekable();
                let first = bindings.peek().copied()?;
                let shown = bindings
                    .find(|binding| {
                        binding
                            .predicate()
                            .is_none_or(|predicate| predicate.eval(&[defaults.clone()]))
                    })
                    .unwrap_or(first);
                let [stroke] = shown.keystrokes() else {
                    return None;
                };
                let key = stroke.key();
                (key.chars().count() > 1 && !MAPPED.contains(&key) && !function_key(key))
                    .then(|| format!("{name}: {key}"))
            })
            .collect()
    });
    assert!(unread.is_empty(), "AppKit can't read {unread:?}");
}
