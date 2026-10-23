//! What a tab never gets around (the cluster guard), and what keeps the
//! keys that belong to something else.

use freshkube_nav::{LinkPage, ViewState};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};

use super::OpenIn;
use crate::desktop::search::Destination;
use crate::desktop::session::SessionKey;
use crate::desktop::tests::{fixture, start_shell};
use crate::desktop::{Page, Pilot};
use crate::resources::model::ObjectRef;
use crate::resources::{self, example, live};
use freshkube_core::resources::builtin;

fn switch(cx: &mut TestAppContext, handle: AnyWindowHandle, pilot: &Entity<Pilot>, id: &str) {
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.switch_cluster(id.into(), window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn active(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> SessionKey {
    cx.read(|cx| pilot.read(cx).switch.registry.active_key().clone())
}

fn tab_count(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> usize {
    cx.read(|cx| pilot.read(cx).tabs.set.len())
}

/// A pod of `dev-fra`, as a link made in it names it.
fn other_clusters_pod() -> ObjectRef {
    let (_, rows) = example::read("dev-fra", "pods", None, live::now()).unwrap();
    let mut link = ObjectRef::from(rows[0].identity.clone());
    link.connection = Some(example::connection("dev-fra"));
    link
}

/// The window on `core-fra` with a running shell, and `dev-fra` an entry
/// that was open before: a link to it must ask first and say the shell ends.
fn pinned_by_a_shell(
    cx: &mut TestAppContext,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<Pilot>,
    String,
) {
    let (runtime, handle, pilot) = fixture(cx, 1280., 880.);
    switch(cx, handle, &pilot, "dev-fra");
    switch(cx, handle, &pilot, "core-fra");
    let context = cx.read(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let (_, rows) = example::read(&context, "pods", None, live::now()).unwrap();
    let pod = rows
        .iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity
        .clone();
    start_shell(handle, &pilot, &pod, cx);
    (runtime, handle, pilot, pod.name)
}

/// A link made in another cluster is sent there first, whatever the click
/// says, and no tab opens on the cluster it was not made in. The running
/// shell keeps its own connection, so nothing asks about it (#878).
#[gpui_kit::test]
fn a_tab_link_from_another_cluster_goes_there_and_ends_no_shell(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot, _shell) = pinned_by_a_shell(cx);
    let other = other_clusters_pod();
    for open_in in [OpenIn::Here, OpenIn::Background, OpenIn::Front] {
        switch(cx, handle, &pilot, "core-fra");
        let object = other.clone();
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| {
                pilot.open_object_in(
                    open_in,
                    builtin("pods").unwrap(),
                    object,
                    resources::Tab::Overview,
                    window,
                    cx,
                )
            })
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!cx.has_pending_prompt(), "{open_in:?}");
        assert_eq!(
            active(cx, &pilot),
            SessionKey::Entry("dev-fra".into()),
            "{open_in:?}"
        );
        assert_eq!(tab_count(cx, &pilot), 1, "{open_in:?}");
        assert_eq!(cx.read(crate::resources::shell::running_anywhere).len(), 1);
    }
}

/// Command-Return in the palette takes an object through the same guard.
#[gpui_kit::test]
fn command_return_on_an_object_of_another_cluster_goes_there_and_opens_no_tab(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, pilot, _shell) = pinned_by_a_shell(cx);
    let object = other_clusters_pod();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.search_destination(
                Destination::Object(builtin("pods").unwrap(), object),
                OpenIn::Front,
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(active(cx, &pilot), SessionKey::Entry("dev-fra".into()));
    assert_eq!(tab_count(cx, &pilot), 1, "no tab opened on this cluster");
}

/// A link made in a cluster that is no entry's is refused with a notice,
/// and a tab is no way round it.
#[gpui_kit::test]
fn a_tab_link_from_a_cluster_that_is_not_open_is_refused(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let heard = crate::desktop::testing::record_notices(cx, &pilot);
    let (_, rows) = example::read("prod-lon", "pods", None, live::now()).unwrap();
    let mut object = ObjectRef::from(rows[0].identity.clone());
    object.connection = Some(example::connection("prod-lon"));
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object_in(
                OpenIn::Front,
                builtin("pods").unwrap(),
                object,
                resources::Tab::Overview,
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        heard
            .borrow()
            .last()
            .is_some_and(|message| message.contains("isn’t open")),
        "{:?}",
        heard.borrow()
    );
    assert_eq!(tab_count(cx, &pilot), 1);
}

/// The dock's Command-W closes the dock's tab while two view tabs exist.
#[gpui_kit::test]
fn the_docks_close_key_wins_in_the_dock_with_two_view_tabs(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let context = cx.read(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let (_, rows) = example::read(&context, "pods", None, live::now()).unwrap();
    let pod = rows[0].identity.clone();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_view_tab(
                ViewState::page(LinkPage::Health),
                OpenIn::Background,
                window,
                cx,
            );
            pilot.open_object(
                builtin("pods").unwrap(),
                pod.clone().into(),
                resources::Tab::Logs,
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    let dock = cx.read(|cx| pilot.read(cx).pages.dock.clone());
    assert_eq!(cx.read(|cx| dock.read(cx).tab_count()), 1);
    assert_eq!(tab_count(cx, &pilot), 2);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The dock has the keyboard, as it does after Open logs.
        assert!(dock.read(cx).has_focus(window, cx));
        window.press(
            freshkube_ui::platform::Platform::current().close_tab_key(),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| dock.read(cx).tab_count()),
        0,
        "the dock's tab closed"
    );
    assert_eq!(tab_count(cx, &pilot), 2, "the view tabs stayed");

    // With the keyboard on the page, the same key closes a view tab.
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| window.focus(&pilot.focus.clone(), cx));
        window.press(
            freshkube_ui::platform::Platform::current().close_tab_key(),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(tab_count(cx, &pilot), 1);
}

/// Control-Tab goes to a focused terminal, not to the tabs (Control-Shift
/// combinations are the terminal's shortcuts on Linux and Windows, which
/// reach the keymap by design): the terminal's
/// interceptor takes it before any binding runs
/// (`freshkube_terminal`'s own tests prove the bytes it sends).
#[gpui_kit::test]
fn control_tab_in_a_focused_terminal_does_not_switch_tabs(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot, _shell) = {
        let (runtime, handle, pilot) = fixture(cx, 1280., 880.);
        let context = cx.read(|cx| pilot.read(cx).applied.context.clone().unwrap());
        let (_, rows) = example::read(&context, "pods", None, live::now()).unwrap();
        let pod = rows
            .iter()
            .find(|row| row.cells[2] == "Running")
            .unwrap()
            .identity
            .clone();
        start_shell(handle, &pilot, &pod, cx);
        (runtime, handle, pilot, pod.name)
    };
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_view_tab(
                ViewState::page(LinkPage::Health),
                OpenIn::Background,
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    let page = cx.read(|cx| pilot.read(cx).navigation.page);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("terminal").focused(), Some(true));
        window.press("ctrl-tab", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("terminal").focused(), Some(true));
    })
    .unwrap();
    assert_eq!(cx.read(|cx| pilot.read(cx).tabs.set.selected()), 0);
    assert_eq!(cx.read(|cx| pilot.read(cx).navigation.page), page);
    // Out of the terminal, the same key steps them.
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| window.focus(&pilot.focus.clone(), cx));
        window.press("ctrl-tab", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| pilot.read(cx).tabs.set.selected()), 1);
    assert_eq!(cx.read(|cx| pilot.read(cx).navigation.page), Page::Health);
}

/// Opening a pod's logs or starting its shell opens the dock and leaves the
/// pod's pane open in the headless window.
#[gpui_kit::test]
fn opening_the_dock_keeps_the_pod_pane_open(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let context = cx.read(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let (_, rows) = example::read(&context, "pods", None, live::now()).unwrap();
    let pod = rows
        .iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity
        .clone();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                builtin("pods").unwrap(),
                pod.clone().into(),
                resources::Tab::Overview,
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    let open = |cx: &mut TestAppContext| cx.read(|cx| pilot.read(cx).opened_object(cx).is_some());
    assert!(open(cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("l", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(open(cx), "logs");
    start_shell(handle, &pilot, &pod, cx);
    assert!(open(cx), "shell");
}
