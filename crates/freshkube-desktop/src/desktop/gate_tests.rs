//! Node-scoped pages while the Talos overview hasn't answered: they wait or
//! show the failure, and only an answered overview can say "No node selected".

use std::time::Duration;

use super::pages::Page;
use super::tests::mount;
use crate::GpuiOptions;
use crate::screens::{
    DiagnosticsScreen, NetworkScreen, ProcessesScreen, Reading, ScreenPanel, StorageScreen,
    set_reading,
};
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::px;
use gpui_kit::size;
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, Window};

const PATIENCE: Duration = Duration::from_secs(5);
const WAITING: &str = "screen-waiting";
const UNREACHABLE: &str = "screen-unreachable";
const NO_NODE: &str = "screen-no-node";

/// The pages that need a node, as the shell shows them.
const PAGES: [Page; 5] = [
    Page::Etcd,
    Page::Security,
    Page::Lifecycle,
    Page::Health,
    Page::Operations,
];
/// Something only a page that has its data draws.
fn data_of(page: Page) -> &'static str {
    match page {
        Page::Etcd => "etcd-split",
        Page::Security => "security-list",
        Page::Lifecycle => "lifecycle-etcd",
        Page::Health => "workloads-table",
        _ => "ops-kinds",
    }
}
/// Health reads the Kubernetes summary and keeps its own source.
const NEEDING_A_NODE: [Page; 4] = [
    Page::Etcd,
    Page::Security,
    Page::Lifecycle,
    Page::Operations,
];
fn show_page(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<super::Pilot>,
    page: Page,
) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.navigate(page, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
}

fn state(cx: &mut TestAppContext, handle: AnyWindowHandle, what: &str) -> [bool; 3] {
    cx.update_window(handle, |_, window: &mut Window, cx| {
        window.render_frame(cx);
        [WAITING, UNREACHABLE, NO_NODE].map(|id| window.try_find(id).is_some())
    })
    .unwrap_or_else(|_| panic!("{what}"))
}

#[gpui_kit::test]
async fn pages_wait_while_the_overview_loads_and_show_data_once_it_answers(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture().holding_talos(), 1280., 880.);
    for page in PAGES {
        show_page(cx, handle, &view, page);
        assert_eq!(
            state(cx, handle, "loading"),
            [true, false, false],
            "{page:?} while loading"
        );
    }

    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.fixture_hold = false;
            // The held read still counts as in flight; let the refresh start one.
            view.overview = crate::state::Snapshot::default();
            view.refresh_now(window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    for page in PAGES {
        show_page(cx, handle, &view, page);
        cx.wait_for(handle, PATIENCE, |window, _| {
            settled(window) && window.try_find(data_of(page)).is_some()
        })
        .await;
    }
}

/// No waiting, failure or no-node state: the page shows its data.
fn settled(window: &mut Window) -> bool {
    [WAITING, UNREACHABLE, NO_NODE]
        .iter()
        .all(|id| window.try_find(*id).is_none())
}

#[gpui_kit::test]
fn pages_show_the_failure_when_the_first_overview_read_fails(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture().holding_talos(), 1280., 880.);
    cx.update_window(handle, |_, _, cx| {
        view.update(cx, |pilot, cx| pilot.simulate_failure(cx))
    })
    .unwrap();
    for page in PAGES {
        show_page(cx, handle, &view, page);
        assert_eq!(
            state(cx, handle, "failure"),
            [false, true, false],
            "{page:?} after a failed read"
        );
    }
}

#[gpui_kit::test]
fn only_an_answered_overview_without_a_node_says_no_node_selected(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture(), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.selected_node = None;
            pilot.nodes.clear();
            pilot.push_source(window, cx);
        })
    })
    .unwrap();
    for page in NEEDING_A_NODE {
        show_page(cx, handle, &view, page);
        assert_eq!(
            state(cx, handle, "no node"),
            [false, false, true],
            "{page:?} with no node"
        );
    }
}

/// The node pane's tabs can't open before a node's row exists, so each embedded
/// screen is drawn alone: the same rule answers for it.
fn pane_screen<T: ScreenPanel>(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        Theme::change(ThemeMode::Light, None, cx);
        cx.set_reduce_motion(true);
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let handle = cx.open_window(size(px(900.), px(700.)), |window, cx| {
        let screen = cx.new(|cx| T::new(runtime.handle().clone(), window, cx));
        Root::new(screen, window, cx)
    });
    let handle: AnyWindowHandle = handle.into();
    for (reading, expected) in [
        (Reading::Waiting, [true, false, false]),
        (
            Reading::Failed("The Talos API didn't answer".into()),
            [false, true, false],
        ),
        (Reading::Answered, [false, false, true]),
    ] {
        cx.update(|cx| set_reading(reading.clone(), cx));
        assert_eq!(state(cx, handle, "pane"), expected, "{reading:?}");
    }
}

#[gpui_kit::test]
fn the_node_panes_screens_follow_the_same_rule(cx: &mut TestAppContext) {
    pane_screen::<ProcessesScreen>(cx);
    pane_screen::<StorageScreen>(cx);
    pane_screen::<NetworkScreen>(cx);
    pane_screen::<DiagnosticsScreen>(cx);
}

/// Retry after a failed first read shows the loading state while the new
/// read is in flight, then the data.
#[gpui_kit::test]
async fn retry_after_a_failed_first_read_waits_and_then_shows_data(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture().holding_talos(), 1280., 880.);
    cx.update_window(handle, |_, _, cx| {
        view.update(cx, |pilot, cx| pilot.simulate_failure(cx))
    })
    .unwrap();
    show_page(cx, handle, &view, Page::Etcd);
    assert_eq!(state(cx, handle, "failure"), [false, true, false]);

    // The hold keeps the new read in flight.
    cx.update_window(handle, |_, window, cx| window.click("screen-retry", cx))
        .unwrap();
    assert_eq!(state(cx, handle, "retrying"), [true, false, false]);

    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.fixture_hold = false;
            view.overview = crate::state::Snapshot::default();
            view.refresh_now(window, cx);
        })
    })
    .unwrap();
    cx.wait_for(handle, PATIENCE, |window, _| {
        settled(window) && window.try_find(data_of(Page::Etcd)).is_some()
    })
    .await;
}

/// A new talosconfig path doesn't show the old path's error while it loads.
#[gpui_kit::test]
async fn a_new_talosconfig_path_does_not_show_the_old_error(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    // Never the user's default talosconfig: a missing file.
    let missing = std::env::temp_dir().join(format!("freshkube-tests-{}-gate", std::process::id()));
    let (_runtime, handle, view) =
        mount(cx, GpuiOptions::new(Some(missing), None, 100), 1280., 820.);
    cx.wait_for(handle, Duration::from_secs(2), |_, cx| {
        view.read(cx).config_error.is_some()
    })
    .await;
    cx.update(|cx| assert!(matches!(crate::screens::reading(cx), Reading::Failed(_))));
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.load_configuration(window, cx));
        assert_eq!(crate::screens::reading(cx), Reading::Waiting);
    })
    .unwrap();
}

/// The example window in Kubernetes-only mode, on Health.
fn kubernetes_only_health(
    cx: &mut TestAppContext,
    options: GpuiOptions,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<super::Pilot>,
) {
    let (runtime, handle, view) = mount(cx, options, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.startup_selection(Some("kubernetes-only"), None, None, window, cx)
        })
    })
    .unwrap();
    show_page(cx, handle, &view, Page::Health);
    (runtime, handle, view)
}

/// Health needs no node: in Kubernetes-only mode it waits for the summary's
/// first answer, however long that takes, then shows it.
#[gpui_kit::test]
async fn kubernetes_only_health_waits_for_the_summary_then_shows_it(cx: &mut TestAppContext) {
    let (_runtime, handle, view) =
        kubernetes_only_health(cx, GpuiOptions::fixture().holding_talos());
    for _ in 0..3 {
        assert_eq!(state(cx, handle, "held"), [true, false, false]);
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
    }

    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.fixture_hold = false;
            view.refresh_now(window, cx);
        })
    })
    .unwrap();
    cx.wait_for(handle, PATIENCE, |window, _| {
        settled(window) && window.try_find(data_of(Page::Health)).is_some()
    })
    .await;
}

/// With no context applied nothing reads, so Health says so rather than
/// waiting; choosing one shows its workloads.
#[gpui_kit::test]
async fn kubernetes_only_health_without_a_context_asks_for_one(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = kubernetes_only_health(cx, GpuiOptions::fixture());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.use_kube_context(None, window, cx))
    })
    .unwrap();
    assert_eq!(state(cx, handle, "no context"), [false, false, false]);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("health-no-context").visible());
        view.update(cx, |pilot, cx| {
            pilot.select_context("staging-eu".into(), window, cx)
        });
    })
    .unwrap();
    cx.wait_for(handle, PATIENCE, |window, _| {
        settled(window)
            && window.try_find("health-no-context").is_none()
            && window.try_find(data_of(Page::Health)).is_some()
    })
    .await;
}

/// Health waits only while a summary session reads. A Talos overview that
/// answered without one, as one with no client does, keeps the gate's state
/// instead of an endless wait.
#[gpui_kit::test]
fn health_waits_only_while_a_summary_session_reads(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture(), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            // The example overview has no client, as an unreachable Talos
            // endpoint gives: outside example mode nothing can read the
            // summary.
            pilot.fixture = false;
            pilot.stop_summary();
            pilot.summary_health = None;
            pilot.selected_node = None;
            pilot.nodes.clear();
            pilot.push_source(window, cx);
        })
    })
    .unwrap();
    show_page(cx, handle, &view, Page::Health);
    assert_eq!(state(cx, handle, "no session"), [false, false, true]);

    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.fixture = true;
            pilot.fixture_hold = true;
            pilot.ensure_summary(window, cx);
        })
    })
    .unwrap();
    assert_eq!(state(cx, handle, "a held session"), [true, false, false]);
}
