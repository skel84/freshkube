//! Menus show their keys (docs/DESIGN.md "Menus show their keys"): the
//! table's "After" column, page by page, and where the keyboard goes when
//! a menu opened from a button closes.
use super::tests::{fixture, open_kind, open_node_tab};
use super::{Page, Pilot};
use crate::resources::{Tab, example, live};
use freshkube_core::resources::builtin;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, Pixels, TestAppContext, VisualTestContext,
    px, size,
};

/// Each key a menu here may show, as Kit marks it when it draws one.
#[cfg(target_os = "macos")]
mod platform {
    pub const REFRESH: &str = "kbd:cmd-r";
    pub const MOVE_UP: &str = "kbd:alt-cmd-up";
    pub const MOVE_DOWN: &str = "kbd:alt-cmd-down";
    pub const CLOSE: &str = "kbd:cmd-w";
}
#[cfg(not(target_os = "macos"))]
mod platform {
    pub const REFRESH: &str = "kbd:ctrl-r";
    pub const MOVE_UP: &str = "kbd:ctrl-alt-up";
    pub const MOVE_DOWN: &str = "kbd:ctrl-alt-down";
    pub const CLOSE: &str = "kbd:ctrl-shift-w";
}
use platform::{CLOSE, MOVE_DOWN, MOVE_UP, REFRESH};
const TREE: &str = "kbd:shift-t";
const LOGS: &str = "kbd:l";
const OPEN: &str = "kbd:o";
const ADD: &str = "kbd:a";
const EDIT: &str = "kbd:e";
const REMOVE: &str = "kbd:backspace";
const RELOAD: &str = "kbd:r";
/// Keys a page binds that no folded entry should show.
const OTHERS: [&str; 3] = ["kbd:enter", "kbd:escape", "kbd:x"];

const KEYS: [&str; 11] = [
    REFRESH, MOVE_UP, MOVE_DOWN, CLOSE, TREE, LOGS, OPEN, ADD, EDIT, REMOVE, RELOAD,
];

/// A window narrow enough, at text size 20, that every page folds every
/// control it can.
fn narrow(cx: &mut TestAppContext) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let mounted = fixture(cx, 760., 560.);
    cx.update(|cx| crate::text_size::set(20., cx));
    cx.run_until_parked();
    mounted
}

fn draw(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn on(
    handle: AnyWindowHandle,
    cx: &mut TestAppContext,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
) {
    cx.update_window(handle, |_, window, cx| {
        // The header folds from what it measured on the frame before, and a
        // resize settles over a few such frames.
        for _ in 0..12 {
            window.render_frame(cx);
            if window.simulate_next_frame(cx) == 0 {
                break;
            }
        }
        f(window, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    draw(handle, cx);
}

/// Clicks `button`, a "…" or the dock's menu, and reads the menu it opens:
/// each entry's label and the key drawn on its row, or `None`.
fn open_menu(
    handle: AnyWindowHandle,
    button: &'static str,
    cx: &mut TestAppContext,
) -> Vec<(String, Option<&'static str>)> {
    on(handle, cx, |window, cx| window.click(button, cx));
    let items: Vec<(String, Bounds<Pixels>)> = cx
        .update_window(handle, |_, window, _| {
            let menu = window.within("popup-menu");
            (0usize..)
                .map_while(|ix| menu.try_find(ix))
                .filter_map(|item| Some((item.label()?.to_owned(), item.bounds())))
                .collect()
        })
        .unwrap();
    assert!(!items.is_empty(), "{button} opened no menu");
    let mut window = VisualTestContext::from_window(handle, cx);
    let drawn: Vec<(&'static str, Bounds<Pixels>)> = KEYS
        .iter()
        .chain(&OTHERS)
        .filter_map(|key| Some((*key, window.debug_bounds(key)?)))
        .collect();
    items
        .into_iter()
        .map(|(label, row)| {
            let key = drawn
                .iter()
                .find(|(_, at)| row.contains(&at.center()))
                .map(|(key, _)| *key);
            (label, key)
        })
        .collect()
}

fn close_menu(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    on(handle, cx, |window, cx| window.press("escape", cx));
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("popup-menu").is_none(), "the menu closed");
    })
    .unwrap();
}

/// `expected`'s entries show their keys and no others; an entry missing
/// from the menu is a control that didn't fold, and fails too.
#[track_caller]
fn assert_keys(menu: &[(String, Option<&'static str>)], expected: &[(&str, Option<&'static str>)]) {
    for (label, key) in expected {
        let shown = menu
            .iter()
            .find(|(entry, _)| entry == label)
            .unwrap_or_else(|| panic!("no {label:?} in {menu:?}"));
        assert_eq!(shown.1, *key, "{label:?} in {menu:?}");
    }
}

/// Narrows the window, at each text size, until each `expected` entry has folded into `more`'s
/// menu, and checks its key at the first width that folds it. The header
/// folds a control only between the widths where it fits and where the
/// controls take a row of their own, and those bands move with the page's
/// parts, so the test looks for them rather than naming widths.
#[track_caller]
fn assert_folded(
    handle: AnyWindowHandle,
    more: &'static str,
    expected: &[(&str, Option<&'static str>)],
    cx: &mut TestAppContext,
) {
    let mut left = expected.to_vec();
    let sizes = [13., 16., 20.].into_iter().flat_map(|text| {
        (260..=1600)
            .rev()
            .step_by(20)
            .map(move |width| (text, width as f32))
    });
    for (text, width) in sizes {
        cx.update(|cx| crate::text_size::set(text, cx));
        cx.simulate_window_resize(handle, size(px(width), px(720.)));
        on(handle, cx, |_, _| {});
        let shown = cx
            .update_window(handle, |_, window, _| window.try_find(more).is_some())
            .unwrap();
        if !shown {
            continue;
        }
        let menu = open_menu(handle, more, cx);
        close_menu(handle, cx);
        left.retain(|entry| {
            let folded = menu.iter().any(|(label, _)| label == entry.0);
            if folded {
                assert_keys(&menu, &[*entry]);
            }
            !folded
        });
        if left.is_empty() {
            break;
        }
    }
    cx.update(|cx| crate::text_size::set(crate::ui::BASE_TEXT, cx));
    assert!(left.is_empty(), "no size folds {left:?} into {more}");
}

fn navigate(handle: AnyWindowHandle, pilot: &Entity<Pilot>, page: Page, cx: &mut TestAppContext) {
    on(handle, cx, |window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(page, window, cx))
    });
}

/// Every folded control with a key shows it, and the dock tab menu's Close
/// shows ⌘W: DESIGN.md's table, "After" column. A control that no longer
/// folds, a key rebound, or an entry that names its key itself fails here.
#[gpui_kit::test]
fn every_menu_entry_shows_the_key_its_context_binds(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 720.);
    let refresh = [("Refresh", Some(REFRESH))];

    on(handle, cx, |window, cx| open_kind(window, cx, "pods"));
    assert_folded(handle, "resource-more", &refresh, cx);
    // The pickers that fold beside it show no key.
    let menu = open_menu(handle, "resource-more", cx);
    close_menu(handle, cx);
    let keyed: Vec<_> = menu.iter().filter(|(_, key)| key.is_some()).collect();
    assert_eq!(keyed, [&("Refresh".to_owned(), Some(REFRESH))], "{menu:?}");

    navigate(handle, &pilot, Page::Nodes, cx);
    assert_folded(handle, "nodes-more", &refresh, cx);

    on(handle, cx, |window, cx| {
        open_node_tab(window, cx, super::nodes::NodeTab::Processes)
    });
    assert_folded(handle, "processes-more", &[("Tree", Some(TREE))], cx);

    navigate(handle, &pilot, Page::SystemServices, cx);
    assert_folded(
        handle,
        "system-services-more",
        &[("Logs", Some(LOGS)), ("Open node", Some(OPEN))],
        cx,
    );

    navigate(handle, &pilot, Page::Settings, cx);
    assert_folded(
        handle,
        "settings-more",
        &[
            ("Add", Some(ADD)),
            ("Edit", Some(EDIT)),
            ("Remove", Some(REMOVE)),
            ("Move up", Some(MOVE_UP)),
            ("Move down", Some(MOVE_DOWN)),
            ("Reload", Some(RELOAD)),
        ],
        cx,
    );

    navigate(handle, &pilot, Page::Monitoring, cx);
    assert_folded(handle, "monitoring-more", &refresh, cx);

    navigate(handle, &pilot, Page::Observability, cx);
    assert_folded(handle, "obs-more", &refresh, cx);

    // The dock tab menu: Close is ⌘W's; the others have no key.
    cx.simulate_window_resize(handle, size(px(1280.), px(720.)));
    let context = cx.update(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let (_, rows) = example::read(&context, "pods", None, live::now()).unwrap();
    let pod = rows[0].identity.clone();
    on(handle, cx, |window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(builtin("pods").unwrap(), pod.into(), Tab::Logs, window, cx)
        })
    });
    assert_keys(
        &open_menu(handle, "dock-menu", cx),
        &[
            ("Close", Some(CLOSE)),
            ("Close others", None),
            ("Close to the right", None),
            ("Close all", None),
        ],
    );
    close_menu(handle, cx);
}

/// A folded Refresh runs on the list's wrapper, which draws in every state,
/// so it works and shows ⌘R while the list shows a placeholder; and the
/// keyboard goes back where it was when the menu closes.
#[gpui_kit::test]
fn a_folded_refresh_works_on_a_placeholder_and_escape_gives_the_keyboard_back(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, pilot) = narrow(cx);
    let focused = |id: &'static str, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, _| window.find(id).focused())
            .unwrap()
    };
    // The example data has no config maps, so a message replaces the rows.
    on(handle, cx, |window, cx| open_kind(window, cx, "configmaps"));
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("resource-not-in-example").visible());
    })
    .unwrap();
    assert_eq!(focused("resource-body", cx), Some(true));

    // Escape closes the menu and gives the keyboard back to the list.
    let menu = open_menu(handle, "resource-more", cx);
    assert_keys(&menu, &[("Refresh", Some(REFRESH))]);
    close_menu(handle, cx);
    assert_eq!(focused("resource-body", cx), Some(true));

    // Refresh runs ⌘R's refresh, and the keyboard stays on the list.
    let ticks = |cx: &mut TestAppContext| cx.update(|cx| pilot.read(cx).fixture_tick);
    let before = ticks(cx);
    let menu = open_menu(handle, "resource-more", cx);
    let ix = menu
        .iter()
        .position(|(label, _)| label == "Refresh")
        .unwrap();
    on(handle, cx, |window, cx| {
        window.within("popup-menu").click(ix, cx)
    });
    assert_eq!(ticks(cx), before + 1);
    assert_eq!(focused("resource-body", cx), Some(true));

    // From the filter, both give the keyboard back to the filter.
    on(handle, cx, |window, cx| window.click("resource-filter", cx));
    let filter = "resource-filter";
    assert_eq!(focused(filter, cx), Some(true));
    open_menu(handle, "resource-more", cx);
    close_menu(handle, cx);
    assert_eq!(focused(filter, cx), Some(true));
    let menu = open_menu(handle, "resource-more", cx);
    let ix = menu
        .iter()
        .position(|(label, _)| label == "Refresh")
        .unwrap();
    on(handle, cx, |window, cx| {
        window.within("popup-menu").click(ix, cx)
    });
    assert_eq!(ticks(cx), before + 2);
    assert_eq!(focused(filter, cx), Some(true));
}
