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
use std::collections::BTreeSet;
use std::sync::Mutex;

/// An entry a menu should show: its label, and the action whose key it
/// shows, by its registered name, or `None` for an entry with no key.
type Entry = (&'static str, Option<&'static str>);
/// An entry as a menu shows it: its label and the key drawn on its row.
type Shown = (String, Option<&'static str>);

/// `text` for as long as the tests run: Kit's debug selectors are
/// `&'static str`. Each text is kept once.
fn interned(text: String) -> &'static str {
    static SEEN: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    let mut seen = SEEN.lock().unwrap();
    if let Some(known) = seen.get(text.as_str()) {
        return known;
    }
    let text: &'static str = Box::leak(text.into_boxed_str());
    seen.insert(text);
    text
}

/// How Kit marks a key it draws: `kbd:` and the binding's first stroke.
fn selector(binding: &gpui_kit::KeyBinding) -> Option<&'static str> {
    let stroke = gpui_kit::AsKeystroke::as_keystroke(binding.keystrokes().first()?);
    Some(interned(format!("kbd:{}", stroke.unparse())))
}

/// Every key the keymap binds, as Kit would mark it: what a menu row may
/// show.
fn every_key(cx: &mut TestAppContext) -> Vec<&'static str> {
    cx.update(|cx| {
        let keymap = cx.key_bindings();
        let keys: BTreeSet<_> = keymap.borrow().bindings().filter_map(selector).collect();
        keys.into_iter().collect()
    })
}

/// The keys `entries` should show: each action's binding where the
/// keyboard is, the page's list, as the keymap resolves it there. A page's
/// entries come from its own context, so a rebound key moves with them.
fn keys_of(handle: AnyWindowHandle, entries: &[Entry], cx: &mut TestAppContext) -> Vec<Shown> {
    cx.update_window(handle, |_, window, cx| {
        let focus = window.focused(cx).expect("the page has the keyboard");
        entries
            .iter()
            .map(|(label, name)| {
                let key = name.map(|name| {
                    let action = cx.build_action(name, None).unwrap();
                    let binding = window
                        .highest_precedence_binding_for_action_in(action.as_ref(), &focus)
                        .unwrap_or_else(|| panic!("nothing binds {name} where {label} runs"));
                    selector(&binding).unwrap()
                });
                ((*label).to_owned(), key)
            })
            .collect()
    })
    .unwrap()
}

const REFRESH: &str = "pilot::Refresh";

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
fn open_menu(handle: AnyWindowHandle, button: &'static str, cx: &mut TestAppContext) -> Vec<Shown> {
    let keys = every_key(cx);
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
    let drawn: Vec<(&'static str, Bounds<Pixels>)> = keys
        .into_iter()
        .filter_map(|key| Some((key, window.debug_bounds(key)?)))
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
fn assert_keys(menu: &[Shown], expected: &[Shown]) {
    for (label, key) in expected {
        let shown = menu
            .iter()
            .find(|(entry, _)| entry == label)
            .unwrap_or_else(|| panic!("no {label:?} in {menu:?}"));
        assert_eq!(shown.1, *key, "{label:?} in {menu:?}");
    }
}

/// Narrows the window, at each text size, until each `expected` entry has
/// folded into `more`'s menu, and checks its key at the first width that
/// folds it. The header folds a control only between the widths where it
/// fits and where the controls take a row of their own, and those bands
/// move with the page's parts, so the test looks for them rather than
/// naming widths. Once all have folded, the entries that showed a key are
/// `expected`'s keyed ones, no more: a picker folded beside them shows none.
#[track_caller]
fn assert_folded(
    handle: AnyWindowHandle,
    more: &'static str,
    expected: &[Entry],
    cx: &mut TestAppContext,
) {
    let expected = keys_of(handle, expected, cx);
    let mut left = expected.clone();
    let mut keyed = BTreeSet::new();
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
        keyed.extend(menu.iter().filter(|(_, key)| key.is_some()).cloned());
        left.retain(|entry| {
            let folded = menu.iter().any(|(label, _)| *label == entry.0);
            if folded {
                assert_keys(&menu, std::slice::from_ref(entry));
            }
            !folded
        });
        if left.is_empty() {
            break;
        }
    }
    cx.update(|cx| crate::text_size::set(crate::ui::BASE_TEXT, cx));
    assert!(left.is_empty(), "no size folds {left:?} into {more}");
    let want: BTreeSet<Shown> = expected
        .into_iter()
        .filter(|(_, key)| key.is_some())
        .collect();
    assert_eq!(keyed, want, "the entries of {more} that show a key");
}

fn navigate(handle: AnyWindowHandle, pilot: &Entity<Pilot>, page: Page, cx: &mut TestAppContext) {
    on(handle, cx, |window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(page, window, cx))
    });
}

/// Every folded control with a key shows it, and the dock tab menu's Close
/// shows ⌘W: DESIGN.md's table, "After" column. A control that no longer
/// folds, an entry that shows a key its page doesn't bind there, or one
/// that names its key itself fails here.
#[gpui_kit::test]
fn every_menu_entry_shows_the_key_its_context_binds(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 720.);
    let refresh = [("Refresh", Some(REFRESH))];

    on(handle, cx, |window, cx| open_kind(window, cx, "pods"));
    assert_folded(handle, "resource-more", &refresh, cx);

    navigate(handle, &pilot, Page::Nodes, cx);
    assert_folded(handle, "nodes-more", &refresh, cx);

    on(handle, cx, |window, cx| {
        open_node_tab(window, cx, super::nodes::NodeTab::Processes)
    });
    let tree = [("Tree", Some("talos_processes::ToggleTree"))];
    assert_folded(handle, "processes-more", &tree, cx);

    navigate(handle, &pilot, Page::SystemServices, cx);
    let services = [
        ("Logs", Some("system_services::ServiceLogs")),
        ("Open node", Some("system_services::OpenServiceNode")),
    ];
    // A menu shows an action's last binding, so O, the key the menu
    // should show, is bound after Enter (DESIGN.md).
    assert_eq!(keys_of(handle, &services, cx)[1].1, Some("kbd:o"));
    assert_folded(handle, "system-services-more", &services, cx);

    navigate(handle, &pilot, Page::Settings, cx);
    let settings = [
        ("Add", Some("settings::AddCluster")),
        ("Edit", Some("settings::EditCluster")),
        ("Remove", Some("settings::RemoveCluster")),
        ("Move up", Some("settings::MoveClusterUp")),
        ("Move down", Some("settings::MoveClusterDown")),
        ("Reload", Some("settings::ReloadWorkspace")),
    ];
    // E, bound after Enter, as O is above.
    assert_eq!(keys_of(handle, &settings, cx)[1].1, Some("kbd:e"));
    assert_folded(handle, "settings-more", &settings, cx);

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
    let tabs = keys_of(
        handle,
        &[
            ("Close", Some("dock::CloseDockTab")),
            ("Close others", None),
            ("Close to the right", None),
            ("Close all", None),
        ],
        cx,
    );
    assert_keys(&open_menu(handle, "dock-menu", cx), &tabs);
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
    let refresh = keys_of(handle, &[("Refresh", Some(REFRESH))], cx);

    // Escape closes the menu and gives the keyboard back to the list.
    let menu = open_menu(handle, "resource-more", cx);
    assert_keys(&menu, &refresh);
    close_menu(handle, cx);
    assert_eq!(focused("resource-body", cx), Some(true));

    // Refresh runs ⌘R's refresh, which reads the list again, and the
    // keyboard stays on the list.
    let reads = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let pilot = pilot.read(cx);
            (pilot.fixture_tick, pilot.resources.read(cx).read_epoch())
        })
    };
    let refresh_from_menu = |cx: &mut TestAppContext| {
        let menu = open_menu(handle, "resource-more", cx);
        let ix = menu
            .iter()
            .position(|(label, _)| label == "Refresh")
            .unwrap();
        on(handle, cx, |window, cx| {
            window.within("popup-menu").click(ix, cx)
        });
    };
    let (ticks, epoch) = reads(cx);
    refresh_from_menu(cx);
    let (after, read) = reads(cx);
    assert_eq!(after, ticks + 1);
    assert!(read > epoch, "the list read again");
    assert_eq!(focused("resource-body", cx), Some(true));

    // From the filter, both give the keyboard back to the filter.
    on(handle, cx, |window, cx| window.click("resource-filter", cx));
    let filter = "resource-filter";
    assert_eq!(focused(filter, cx), Some(true));
    open_menu(handle, "resource-more", cx);
    close_menu(handle, cx);
    assert_eq!(focused(filter, cx), Some(true));
    refresh_from_menu(cx);
    let (last, again) = reads(cx);
    assert_eq!(last, ticks + 2);
    assert!(again > read, "the list read again");
    assert_eq!(focused(filter, cx), Some(true));
}

/// Edit and Remove from Settings' "…" leave the keyboard in the dialog they
/// open, not on the list the menu ran them on: the form's field takes what
/// is typed, and Escape closes the confirmation.
#[gpui_kit::test]
fn settings_dialogs_from_the_menu_keep_the_keyboard(cx: &mut TestAppContext) {
    // Tokio wakes GPUI from its own threads.
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    std::fs::write(
        guard.path().join("workspace.json"),
        r#"{"version":1,"clusters":[
            {"id":"core","role":"core","context":"acme-core"},
            {"id":"edge","role":"environment","context":"acme-edge"}]}"#,
    )
    .unwrap();
    let (_runtime, handle, pilot) = super::settings::tests::launch(cx, guard.path());
    navigate(handle, &pilot, Page::Settings, cx);
    on(handle, cx, |window, cx| {
        window.click("settings-cluster-edge", cx)
    });
    // The narrowest windows fold Edit and Remove.
    cx.update(|cx| crate::text_size::set(20., cx));
    cx.simulate_window_resize(handle, size(px(300.), px(720.)));
    let pick = |label: &'static str, cx: &mut TestAppContext| {
        let menu = open_menu(handle, "settings-more", cx);
        let ix = menu
            .iter()
            .position(|(entry, _)| entry == label)
            .unwrap_or_else(|| panic!("no {label} in {menu:?}"));
        on(handle, cx, |window, cx| {
            window.within("popup-menu").click(ix, cx)
        });
    };

    pick("Edit", cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("popup-menu").is_none(), "the menu closed");
        assert_eq!(window.find("settings-form-context").focused(), Some(true));
    })
    .unwrap();
    on(handle, cx, |window, cx| {
        window.click("settings-form-cancel", cx)
    });

    pick("Remove", cx);
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("settings-remove-dialog").visible());
    })
    .unwrap();
    on(handle, cx, |window, cx| window.press("escape", cx));
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("settings-remove-dialog").is_none());
    })
    .unwrap();
}
