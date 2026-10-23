//! The strip driven by the pointer, as a person does: click, ×, middle-click
//! and the right-click menu. These update the shell from inside the strip's
//! own update, which a key never does.

use freshkube_nav::{LinkPage, ViewState};
use gpui_kit::InputEvent as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};

use super::OpenIn;
use crate::desktop::tests::fixture;
use crate::desktop::{Page, Pilot};

/// Four tabs: Overview, Etcd, Health, Nodes, the first shown.
fn four(
    cx: &mut TestAppContext,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<Pilot>,
    Vec<u64>,
) {
    let (runtime, handle, pilot) = fixture(cx, 1280., 880.);
    for page in [LinkPage::Nodes, LinkPage::Health, LinkPage::Etcd] {
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| {
                pilot.open_view_tab(ViewState::page(page), OpenIn::Background, window, cx)
            })
        })
        .unwrap();
        cx.run_until_parked();
    }
    let ids = cx.read(|cx| {
        pilot
            .read(cx)
            .tabs
            .set
            .tabs()
            .iter()
            .map(|tab| tab.id)
            .collect()
    });
    (runtime, handle, pilot, ids)
}

fn count(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> usize {
    cx.read(|cx| pilot.read(cx).tabs.set.len())
}

fn selected(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> usize {
    cx.read(|cx| pilot.read(cx).tabs.set.selected())
}

#[gpui_kit::test]
fn a_click_on_a_tab_shows_it(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot, ids) = four(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(format!("view-tab-{}", ids[2]), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(selected(cx, &pilot), 2);
    assert_eq!(cx.read(|cx| pilot.read(cx).navigation.page), Page::Health);
}

#[gpui_kit::test]
fn the_close_button_closes_its_tab(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot, ids) = four(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(format!("view-tab-{}-close", ids[1]), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(count(cx, &pilot), 3);
    assert!(cx.read(|cx| pilot.read(cx).tabs.set.position(ids[1]).is_none()));
}

#[gpui_kit::test]
fn a_middle_click_closes_its_tab(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot, ids) = four(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let position = window
            .find(format!("view-tab-{}", ids[3]))
            .bounds()
            .center();
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Middle,
                position,
                click_count: 1,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(count(cx, &pilot), 3);
    assert!(cx.read(|cx| pilot.read(cx).tabs.set.position(ids[3]).is_none()));
}

/// The menu's entries: Close, Close others, Close to the right, a
/// separator, Close all.
#[gpui_kit::test]
fn each_menu_entry_acts_on_the_tab_it_was_opened_on(cx: &mut TestAppContext) {
    // The tab right-clicked is the second, Etcd.
    for (ix, remaining) in [(0, 3), (1, 1), (2, 2), (4, 1)] {
        let (_runtime, handle, pilot, ids) = four(cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.right_click(format!("view-tab-{}", ids[1]), cx);
        })
        .unwrap();
        cx.run_until_parked();
        // Opening the menu shows the tab it is for.
        assert_eq!(selected(cx, &pilot), 1, "entry {ix}");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.within("popup-menu").click(ix, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        assert_eq!(count(cx, &pilot), remaining, "entry {ix}");
    }
}
