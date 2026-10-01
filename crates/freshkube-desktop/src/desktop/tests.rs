use super::{GpuiOptions, NodeView, Page, Pilot, SidebarReveal};
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext,
    component::{ActiveTheme, Root, Theme, ThemeMode},
    px, size,
};
use std::cell::Cell;
use std::rc::Rc;

fn fixture(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    mount(cx, GpuiOptions::fixture(), width, height)
}

fn mount(
    cx: &mut TestAppContext,
    options: GpuiOptions,
    width: f32,
    height: f32,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        Theme::change(ThemeMode::Light, None, cx);
        // Dialogs animate on the real clock, which the test clock can't
        // advance; reduced motion opens and closes them immediately.
        cx.set_reduce_motion(true);
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut view = None;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let pilot = cx.new(|cx| Pilot::new(options, runtime.handle().clone(), window, cx));
        view = Some(pilot.clone());
        Root::new(pilot, window, cx)
    });
    cx.run_until_parked();
    (runtime, window.into(), view.unwrap())
}

/// Chooses a target through the title bar picker, which is always on screen.
fn pick_target(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, ix: usize) {
    window.click("target-node", cx);
    window.render_frame(cx);
    window.click(("target-option", ix), cx);
    window.render_frame(cx);
}

const FIRST_NODE: &str = "talos-cp-fra1-01";
const DEGRADED_NODE: &str = "talos-wk-fra1-02";
const SILENT_NODE: &str = "talos-wk-fra1-03";

#[gpui_kit::test]
fn theme_toggle_preserves_fixture_state_on_every_screen(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(Theme::global(cx).mode, ThemeMode::Light);
        pick_target(window, cx, 4);
        window.click("nav-services", cx);
        window.render_frame(cx);
        window.within("services-region").click("containerd", cx);
    })
    .unwrap();
    cx.run_until_parked();
    for (nav, page) in [
        ("nav-overview", Page::Overview),
        ("nav-services", Page::Services),
        ("nav-logs", Page::Logs),
    ] {
        cx.update_window(handle, |_, window, cx| {
            window.click(nav, cx);
            window.render_frame(cx);
            let pilot = view.read(cx);
            assert_eq!(pilot.selected_node.as_deref(), Some(DEGRADED_NODE));
            assert_eq!(pilot.selected_service.as_deref(), Some("containerd"));
            let context = pilot.applied.context.clone();
            let node = pilot.selected_node.clone();
            let service = pilot.selected_service.clone();
            let epoch = pilot.epoch;
            let config_generation = pilot.config_generation;
            let overview_success = pilot.overview.last_successful();
            let services_success = pilot.services.last_successful();
            let overview_name = pilot.overview.data().unwrap().name.clone();
            let node_count = pilot.nodes.len();
            let service_count = pilot.services.data().unwrap().len();
            let logs = pilot.logs.clone();
            let light_background = cx.theme().background;
            assert_eq!(window.find("theme-toggle").label(), Some("Dark mode"));
            if page == Page::Logs {
                assert_eq!(
                    window.find("logs-collection").label(),
                    Some("Start collecting")
                );
            }
            window.click("theme-toggle", cx);
            window.render_frame(cx);
            assert_eq!(Theme::global(cx).mode, ThemeMode::Dark);
            assert_ne!(cx.theme().background, light_background);
            assert_eq!(window.find("theme-toggle").label(), Some("Light mode"));
            // Kit buttons deliberately do not steal focus on pointer down.
            // Reach this control through the production keyboard tab order.
            for _ in 0..96 {
                if window.find("theme-toggle").focused() == Some(true) {
                    break;
                }
                window.press("tab", cx);
            }
            assert_eq!(
                window.find("theme-toggle").focused(),
                Some(true),
                "theme toggle must be keyboard reachable on {page:?}"
            );
            window.press("space", cx);
            window.render_frame(cx);
            assert_eq!(Theme::global(cx).mode, ThemeMode::Light);
            assert_eq!(cx.theme().background, light_background);
            assert_eq!(window.find("theme-toggle").label(), Some("Dark mode"));
            assert_eq!(window.find("theme-toggle").focused(), Some(true));
            let pilot = view.read(cx);
            assert_eq!(pilot.page, page);
            assert_eq!(pilot.applied.context, context);
            assert_eq!(pilot.selected_node, node);
            assert_eq!(pilot.selected_service, service);
            assert_eq!(pilot.epoch, epoch);
            assert_eq!(pilot.config_generation, config_generation);
            assert_eq!(pilot.overview.last_successful(), overview_success);
            assert_eq!(pilot.services.last_successful(), services_success);
            assert_eq!(pilot.overview.data().unwrap().name, overview_name);
            assert_eq!(pilot.nodes.len(), node_count);
            assert_eq!(pilot.services.data().unwrap().len(), service_count);
            assert_eq!(pilot.logs, logs);
            if page == Page::Logs {
                assert!(window.find("logs-viewport").visible());
                assert_eq!(
                    window.find("logs-collection").label(),
                    Some("Start collecting")
                );
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn overview_keyboard_selection_updates_target_node(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_node.as_deref(), Some(FIRST_NODE));
        window.within("nodes-region").click(FIRST_NODE, cx);
        window.press("down", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).selected_node.as_deref(),
            Some("talos-cp-fra1-02")
        );
        assert_eq!(
            window
                .within("nodes-region")
                .find("talos-cp-fra1-02")
                .selected(),
            Some(true)
        );
        assert_eq!(
            window.within("nodes-region").find(FIRST_NODE).selected(),
            Some(false)
        );
        assert!(
            window
                .find("applied-config")
                .label()
                .unwrap()
                .contains("talos-cp-fra1-02")
        );
        assert!(view.read(cx).node_focus.is_focused(window));
        window.press("tab", cx);
        assert!(!view.read(cx).node_focus.is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn context_switch_invalidates_service_target(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-services", cx);
        window.render_frame(cx);
        window.within("services-region").click("kubelet", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let old_epoch = cx.update(|cx| {
        assert_eq!(view.read(cx).selected_service.as_deref(), Some("kubelet"));
        view.read(cx).epoch
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("sidebar").click(("context", 1usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let pilot = view.read(cx);
        assert!(pilot.epoch > old_epoch);
        assert_eq!(pilot.applied.context.as_deref(), Some("staging-eu"));
        assert_eq!(pilot.selected_node.as_deref(), Some("stg-cp-01"));
        assert!(pilot.selected_service.is_none());
        assert!(
            window
                .find("applied-config")
                .label()
                .unwrap()
                .contains("staging-eu")
        );
        assert_eq!(
            window
                .within("sidebar")
                .find(("context", 1usize))
                .selected(),
            Some(true)
        );
        // Nothing is selected, so there is no logs action to trigger.
        assert!(window.try_find("service-logs").is_none());
        assert_eq!(view.read(cx).page, Page::Services);
    })
    .unwrap();
}

#[gpui_kit::test]
fn service_keyboard_selection_filter_retains_domain_id(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-services", cx);
        window.render_frame(cx);
        window.within("services-region").click("apid", cx);
        window.press("down", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_service.as_deref(), Some("auditd"));
        assert!(
            window
                .find("selected-service")
                .label()
                .unwrap()
                .contains("Started task auditd")
        );
        window.click("service-filter", cx);
        window.input("apid", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_service.as_deref(), Some("auditd"));
        assert!(
            window
                .within("services-region")
                .try_find("auditd")
                .is_none()
        );
        assert!(window.within("services-region").find("apid").visible());
        window.press("secondary-a", cx);
        window.press("backspace", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.within("services-region").find("auditd").selected(),
            Some(true)
        );
        window.click("refresh-services", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_service.as_deref(), Some("auditd"));
        assert_eq!(
            window.within("services-region").find("auditd").selected(),
            Some(true)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn health_filter_narrows_the_list_and_shows_the_unhealthy_message(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        pick_target(window, cx, 4);
        assert_eq!(view.read(cx).selected_node.as_deref(), Some(DEGRADED_NODE));
        window.click("nav-services", cx);
        window.render_frame(cx);
        window.click("health-unhealthy", cx);
        window.render_frame(cx);
        let region = window.within("services-region");
        assert!(region.find("kubelet").visible());
        assert!(region.try_find("apid").is_none());
        window.within("services-region").click("kubelet", cx);
        window.render_frame(cx);
        assert!(
            window
                .find("selected-service")
                .label()
                .unwrap()
                .contains("connection refused")
        );
        window.click("health-all", cx);
        window.render_frame(cx);
        assert!(window.within("services-region").find("apid").visible());
        assert_eq!(view.read(cx).selected_service.as_deref(), Some("kubelet"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn selected_service_opens_matching_log_collection(cx: &mut TestAppContext) {
    // Opening logs starts the synthetic producer, which wakes GPUI from a
    // Tokio thread; let the scheduler park for those external wakes.
    cx.executor().allow_parking();
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-services", cx);
        window.render_frame(cx);
        window.within("services-region").click("containerd", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("service-logs", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Logs);
        assert!(window.find("logs-viewport").visible());
        assert_eq!(
            window
                .find(SharedString::from("collect-containerd"))
                .checked(),
            Some(true)
        );
        assert_eq!(
            window.find(SharedString::from("collect-apid")).checked(),
            Some(false)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn log_collection_keeps_running_on_other_screens(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-logs", cx);
        window.render_frame(cx);
        window.click("logs-collection", cx);
        window.render_frame(cx);
        assert!(view.read(cx).logs.read(cx).is_collecting());
        window.click("nav-overview", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Overview);
        assert!(view.read(cx).logs.read(cx).is_collecting());
        assert!(view.read(cx).logs.read(cx).collecting_count() > 0);
        window.click("nav-logs", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("logs-collection").label(),
            Some("Stop collecting")
        );
        assert!(
            window
                .find("logs-status")
                .label()
                .unwrap()
                .starts_with("Collecting")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn fixture_stale_failure_retains_cards_until_refresh(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("fixture-fail", cx);
        window.render_frame(cx);
        assert!(view.read(cx).overview.is_stale());
        assert!(
            window
                .find("overview-status")
                .label()
                .unwrap()
                .contains("Stale")
        );
        assert!(window.find("retry").visible());
        assert!(window.within("nodes-region").find(DEGRADED_NODE).visible());
        window.click("refresh", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).overview.is_stale());
        assert!(
            window
                .find("overview-status")
                .label()
                .unwrap()
                .contains("Current snapshot")
        );
        assert!(window.try_find("retry").is_none());
        window.click("settings", cx);
        window.render_frame(cx);
        assert_eq!(window.find("auto-refresh").checked(), Some(true));
        window.click("auto-refresh", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).automatic);
        assert_eq!(window.find("auto-refresh").checked(), Some(false));
    })
    .unwrap();
}

#[gpui_kit::test]
fn target_picker_and_tiles_change_the_target_node(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("target-node", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("target-option", 0usize)).selected(),
            Some(true)
        );
        window.click(("target-option", 3usize), cx);
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).selected_node.as_deref(),
            Some("talos-wk-fra1-01")
        );
        assert!(window.try_find("target-options").is_none());
        window.click("tile-services", cx);
        window.render_frame(cx);
        let pilot = view.read(cx);
        assert_eq!(pilot.page, Page::Services);
        assert_eq!(pilot.selected_node.as_deref(), Some(DEGRADED_NODE));
        assert_eq!(pilot.selected_service.as_deref(), Some("kubelet"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn silent_node_is_unknown_not_failed(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let card = window.within("nodes-region").find(SILENT_NODE);
        assert!(card.label().unwrap().contains(SILENT_NODE));
        pick_target(window, cx, 5);
        window.click("nav-services", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_node.as_deref(), Some(SILENT_NODE));
        assert!(window.find("back-to-overview").visible());
        assert!(window.try_find("services-region").is_none());
        window.click("back-to-overview", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Overview);
    })
    .unwrap();
}

#[gpui_kit::test]
fn table_view_keeps_node_identity_and_selection(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("view-table", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).node_view, NodeView::Table);
        assert_eq!(
            window.within("nodes-region").find(FIRST_NODE).selected(),
            Some(true)
        );
        window.within("nodes-region").click("talos-wk-fra1-01", cx);
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).selected_node.as_deref(),
            Some("talos-wk-fra1-01")
        );
        window.click("view-cards", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).node_view, NodeView::Cards);
        assert_eq!(
            window
                .within("nodes-region")
                .find("talos-wk-fra1-01")
                .selected(),
            Some(true)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn narrow_window_keeps_screens_and_actions_reachable(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("refresh").visible());
        assert!(window.find("theme-toggle").visible());
        assert!(window.find("theme-toggle").bounds().right() <= px(760.));
        assert!(window.find("target-node").bounds().right() <= px(760.));
        assert!(window.find("nodes-region").visible());
        assert!(window.find("nodes-region").bounds().right() <= px(760.));
        window.click("nav-services", cx);
        window.render_frame(cx);
        assert!(window.find("theme-toggle").visible());
        assert!(window.find("service-filter").visible());
        assert!(window.find("services-region").bounds().size.height > px(0.));
        assert!(window.find("services-region").bounds().right() <= px(760.));
        assert!(window.find("services-region").bounds().top() < px(560.));
        window.click("nav-logs", cx);
        window.render_frame(cx);
        assert!(window.find("theme-toggle").visible());
        assert!(window.find("theme-toggle").bounds().right() <= px(760.));
        assert!(window.find("theme-toggle").bounds().bottom() <= px(560.));
        window.click("theme-toggle", cx);
        assert_eq!(Theme::global(cx).mode, ThemeMode::Dark);
        assert_eq!(window.find("theme-toggle").label(), Some("Light mode"));
        for _ in 0..96 {
            if window.find("theme-toggle").focused() == Some(true) {
                break;
            }
            window.press("tab", cx);
        }
        assert_eq!(window.find("theme-toggle").focused(), Some(true));
        window.press("space", cx);
        assert_eq!(Theme::global(cx).mode, ThemeMode::Light);
    })
    .unwrap();
}

#[gpui_kit::test]
fn named_keys_navigate_all_screens_and_configured_contexts(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("secondary-2", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Services);
        assert!(window.find("service-filter").visible());
        assert_eq!(window.find("nav-services").selected(), Some(true));
        window.press("secondary-3", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Logs);
        assert!(window.find("logs-viewport").visible());
        window.press("secondary-1", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Overview);
        assert!(window.find("nodes-region").visible());
        window.press("alt-down", cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("staging-eu"));
        window.press("alt-up", cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("prod-fra"));
        window.press("alt-up", cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("homelab"));
        window.render_frame(cx);
        assert!(window.within("sidebar").find(("context", 0usize)).visible());
        assert_eq!(
            window
                .within("sidebar")
                .find(("context", 2usize))
                .selected(),
            Some(true)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn shell_keys_work_while_a_kind_shows_no_rows(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The example data has no config maps, so a message replaces the rows.
        let kind = crate::resources::example::kind("configmaps").unwrap();
        view.update(cx, |view, cx| view.open_kind(kind, window, cx));
        window.render_frame(cx);
        assert!(window.find("resource-not-in-example").visible());
        assert_eq!(window.find("resource-body").focused(), Some(true));
        window.press("secondary-2", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Services);
    })
    .unwrap();
}

#[gpui_kit::test]
fn another_context_hands_the_keyboard_back_to_the_list(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let kind = crate::resources::example::kind("pods").unwrap();
        view.update(cx, |view, cx| view.open_kind(kind, window, cx));
        window.render_frame(cx);
        window.press("alt-down", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("staging-eu"));
        assert_eq!(window.find("resource-body").focused(), Some(true));
        assert!(window.try_find("detail-close").is_none());
        window.press("down", cx);
    })
    .unwrap();
    // The arrow selected a row, whose details open once the keys pause.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("detail-close").visible());
        // Clicking a context in the sidebar hands the keyboard back too.
        window.within("sidebar").click(("context", 2usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("homelab"));
        assert_eq!(window.find("resource-body").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_logs_page_puts_the_keyboard_on_its_lines(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("secondary-3", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Logs);
        assert_eq!(window.find("logs-viewport").focused(), Some(true));
        // Command-F finds in the logs, and the shell's keys still work.
        window.press("secondary-f", cx);
        window.render_frame(cx);
        assert_eq!(window.find("logs-viewport").focused(), Some(false));
        window.press("secondary-1", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Overview);
    })
    .unwrap();
}

#[gpui_kit::test]
fn ctrl_tab_cycles_through_every_screen(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("secondary-9", cx);
        window.render_frame(cx);
        for expected in [
            Page::Security,
            Page::Lifecycle,
            Page::Resources,
            Page::Operations,
            Page::Overview,
        ] {
            window.press("ctrl-tab", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).page, expected);
        }
        window.press("ctrl-shift-tab", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Operations);
    })
    .unwrap();
}

#[gpui_kit::test]
fn every_screen_is_reachable_and_loads_only_when_shown(cx: &mut TestAppContext) {
    // Minimum window size: the sectioned sidebar must scroll, not clip.
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for page in Page::SCREENS {
            let nav = SharedString::from(format!("nav-{}", page.slug()));
            assert!(window.try_find(nav.clone()).is_some(), "{nav} is missing");
        }
        // Hidden screens never ask for data.
        assert!(window.try_find("processes-page").is_none());
        window.press("secondary-4", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Processes);
        assert_eq!(window.find("nav-processes").selected(), Some(true));
        assert!(window.find("process-list").visible());
        assert!(window.find(("process", 0usize)).visible());
        // Navigating focuses the list, so arrow keys work without a click.
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(window.find(("process", 0usize)).selected(), Some(true));
        // A node switch reaches the visible screen as a new target.
        pick_target(window, cx, 4);
        assert_eq!(view.read(cx).selected_node.as_deref(), Some(DEGRADED_NODE));
        window.render_frame(cx);
        assert!(window.find("partial-notice").visible());
        assert!(window.find("process-list").visible());
        for (key, page) in [
            ("secondary-5", Page::Storage),
            ("secondary-6", Page::Network),
            ("secondary-7", Page::Diagnostics),
            ("secondary-8", Page::Etcd),
            ("secondary-9", Page::Workloads),
        ] {
            window.press(key, cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).page, page);
        }
        // The short window scrolls the sidebar to reach the last sections.
        for page in [Page::Security, Page::Lifecycle] {
            let nav = format!("nav-{}", page.slug());
            reveal(window, cx, &nav);
            window.click(SharedString::from(nav), cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).page, page);
        }
        // Kubernetes kinds sit in collapsible groups below the cluster pages.
        assert!(window.try_find("resources-page").is_none());
        reveal(window, cx, "nav-k8s-pods");
        window.click("nav-k8s-pods", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(window.find("nav-k8s-pods").selected(), Some(true));
        assert!(window.find("resource-list").visible());
        // Contexts stay in view below the scrolled navigation.
        assert!(window.within("sidebar").find(("context", 0usize)).visible());
        reveal(window, cx, "nav-operations");
        window.click("nav-operations", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Operations);
        assert!(window.try_find("resources-page").is_none());
    })
    .unwrap();
}

/// Scrolls the sidebar navigation until `id` shows in full.
fn reveal(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, id: &str) {
    for _ in 0..60 {
        if shown_in_sidebar(window, id) {
            return;
        }
        let area = window.find("sidebar-scroll").bounds();
        let above = window
            .try_find(SharedString::from(id.to_owned()))
            .is_some_and(|element| element.bounds().top() < area.top());
        window.scroll(
            "sidebar-scroll",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                px(0.),
                px(if above { 40. } else { -40. }),
            )),
            cx,
        );
        window.render_frame(cx);
    }
    panic!("{id} never scrolled into view");
}

/// Whether `id` shows in full inside the scrolled sidebar navigation.
fn shown_in_sidebar(window: &mut gpui_kit::Window, id: &str) -> bool {
    let area = window.find("sidebar-scroll").bounds();
    window
        .try_find(SharedString::from(id.to_owned()))
        .is_some_and(|element| {
            let bounds = element.bounds();
            element.visible() && bounds.top() >= area.top() && bounds.bottom() <= area.bottom()
        })
}

#[gpui_kit::test]
fn opening_a_kind_scrolls_its_group_into_the_short_sidebar(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let key = "validatingadmissionpolicybindings.admissionregistration.k8s.io";
        let nav = format!("nav-k8s-{key}");
        assert!(window.try_find(SharedString::from(nav.clone())).is_none());
        // As the keyboard or FRESHKUBE_KIND would open it.
        view.update(cx, |view, cx| view.open_builtin(key, window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(shown_in_sidebar(window, &nav));
        assert_eq!(window.find(SharedString::from(nav)).selected(), Some(true));
        assert_eq!(
            window.find("page-title").label(),
            Some("Validating Admission Policy Bindings")
        );

        // Opening a group from its header shows its kinds too.
        view.update(cx, |view, cx| view.open_builtin("pods", window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(shown_in_sidebar(window, "nav-k8s-pods"));
        reveal(window, cx, "nav-k8s-group-storage");
        window.click("nav-k8s-group-storage", cx);
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(shown_in_sidebar(window, "nav-k8s-csinodes.storage.k8s.io"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubernetes_groups_collapse_and_kinds_open_the_resources_page(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 1000.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Workloads starts open; the other groups start closed.
        let workloads = window.find("nav-k8s-group-workloads");
        assert_eq!(workloads.role(), Some(gpui_kit::Role::Button));
        assert_eq!(workloads.expanded(), Some(true));
        assert!(window.try_find("nav-k8s-deployments.apps").is_some());
        assert_eq!(
            window.find("nav-k8s-group-networking").expanded(),
            Some(false)
        );
        assert!(window.try_find("nav-k8s-services").is_none());

        reveal(window, cx, "nav-k8s-group-networking");
        window.click("nav-k8s-group-networking", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("nav-k8s-group-networking").expanded(),
            Some(true)
        );
        reveal(window, cx, "nav-k8s-services");
        window.click("nav-k8s-services", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(view.read(cx).resource_kind.key(), "services");
        assert_eq!(window.find("nav-k8s-services").selected(), Some(true));
        assert_eq!(window.find("nav-k8s-group-workloads").selected(), None);
        assert!(window.find("resource-list").visible());
        assert_eq!(window.find("page-title").label(), Some("Services"));

        // Collapsing the group of the open kind keeps the page.
        reveal(window, cx, "nav-k8s-group-networking");
        window.click("nav-k8s-group-networking", cx);
        window.render_frame(cx);
        assert!(window.try_find("nav-k8s-services").is_none());
        assert_eq!(view.read(cx).page, Page::Resources);

        // Other pages hide the table, and coming back shows the same kind.
        window.press("secondary-1", cx);
        window.render_frame(cx);
        assert!(window.try_find("resources-page").is_none());
        for _ in 0..11 {
            window.press("ctrl-tab", cx);
            window.render_frame(cx);
        }
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(window.find("page-title").label(), Some("Services"));
        assert!(window.find("resource-list").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn custom_resources_are_discovered_on_expand_and_open_their_kinds(cx: &mut TestAppContext) {
    // The short window: opening the section or a group scrolls its rows in.
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("nav-k8s-group-custom").expanded(), Some(false));
        assert!(view.read(cx).custom.read(cx).groups().is_none());

        reveal(window, cx, "nav-k8s-group-custom");
        window.click("nav-k8s-group-custom", cx);
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(window.find("nav-k8s-group-custom").expanded(), Some(true));
        assert!(shown_in_sidebar(window, "nav-k8s-api-velero.io"));
        let group = "nav-k8s-api-cert-manager.io";
        assert_eq!(window.find(group).expanded(), Some(false));
        assert!(
            window
                .try_find("nav-k8s-api-cert-manager.io-status")
                .is_none()
        );

        reveal(window, cx, group);
        window.click(group, cx);
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(window.find(group).expanded(), Some(true));
        assert!(shown_in_sidebar(window, "nav-k8s-issuers.cert-manager.io"));

        let certificates = "nav-k8s-certificates.cert-manager.io";
        reveal(window, cx, certificates);
        window.click(certificates, cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(
            view.read(cx).resource_kind.key(),
            "certificates.cert-manager.io"
        );
        assert_eq!(window.find(certificates).selected(), Some(true));
        assert_eq!(window.find("page-title").label(), Some("Certificate"));
        assert!(window.find("resource-list").visible());

        // Collapsing the section keeps the page. Opening a kind again, as
        // the keyboard or FRESHKUBE_KIND would, opens the section and group.
        reveal(window, cx, "nav-k8s-group-custom");
        window.click("nav-k8s-group-custom", cx);
        window.render_frame(cx);
        assert!(window.try_find(certificates).is_none());
        assert_eq!(view.read(cx).page, Page::Resources);
        let kind = crate::resources::example::kind("certificaterequests.cert-manager.io").unwrap();
        view.update(cx, |view, cx| view.open_kind(kind, window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
        let requests = "nav-k8s-certificaterequests.cert-manager.io";
        assert!(shown_in_sidebar(window, requests));
        assert_eq!(window.find(requests).selected(), Some(true));
        assert_eq!(window.find(certificates).selected(), Some(false));
        assert_eq!(
            window.find("page-title").label(),
            Some("CertificateRequest")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn custom_groups_say_why_they_show_no_kinds(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 1000.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        view.update(cx, |view, cx| {
            view.toggle_custom_resources(cx);
            for group in [
                "external.metrics.k8s.io",
                "metrics.k8s.io",
                "monitoring.coreos.com",
                "traefik.containo.us",
                "velero.io",
            ] {
                view.toggle_api_group(group, cx);
            }
        });
        window.render_frame(cx);
        for (id, label, retry) in [
            ("nav-k8s-api-velero.io-status", "Not permitted", true),
            // A group whose definition was removed after /apis listed it.
            (
                "nav-k8s-api-traefik.containo.us-status",
                "No longer served",
                true,
            ),
            (
                "nav-k8s-api-external.metrics.k8s.io-status",
                "Couldn't discover",
                true,
            ),
            (
                "nav-k8s-api-metrics.k8s.io-status",
                "Nothing to list",
                false,
            ),
            // Versions that failed while another was read.
            (
                "nav-k8s-api-monitoring.coreos.com-partial",
                "v1beta1 not served · v1alpha1 unreadable",
                true,
            ),
        ] {
            reveal(window, cx, id);
            let row = window.find(SharedString::from(id));
            assert_eq!(row.role(), Some(gpui_kit::Role::Status), "{id}");
            assert_eq!(row.label(), Some(label), "{id}");
            assert_eq!(
                window
                    .try_find(SharedString::from(format!("{id}-retry")))
                    .is_some(),
                retry,
                "{id}"
            );
        }
        assert!(
            window
                .try_find("nav-k8s-servicemonitors.monitoring.coreos.com")
                .is_some()
        );
        assert!(
            window
                .try_find("nav-k8s-api-cert-manager.io-status")
                .is_none()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn custom_resources_follow_the_connection_and_wait_for_discovery(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    let certificates = "nav-k8s-certificates.cert-manager.io";
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let kind = crate::resources::example::kind("certificates.cert-manager.io").unwrap();
        view.update(cx, |view, cx| view.open_kind(kind, window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(shown_in_sidebar(window, certificates));
        // Another connection discovers again.
        window.press("alt-down", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("staging-eu"));
        // The section and the group stay open, with the kinds found again.
        assert_eq!(window.find("nav-k8s-group-custom").expanded(), Some(true));
        assert_eq!(
            window.find("nav-k8s-api-cert-manager.io").expanded(),
            Some(true)
        );
        assert!(window.try_find(certificates).is_some());

        // While the groups are read, the section says so and the reveal
        // waits for them, unless the sidebar is scrolled by hand.
        let reopen = |view: &mut Pilot, cx: &mut gpui_kit::Context<Pilot>| {
            view.toggle_custom_resources(cx);
            view.toggle_custom_resources(cx);
        };
        view.update(cx, |view, cx| {
            view.custom
                .update(cx, |custom, _| custom.hold_reading(None));
            reopen(view, cx);
        });
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("nav-k8s-custom-status").label(),
            Some("Discovering…")
        );
        assert_eq!(view.read(cx).sidebar_reveal, Some(SidebarReveal::Custom));
        window.scroll(
            "sidebar-scroll",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(40.))),
            cx,
        );
        assert_eq!(view.read(cx).sidebar_reveal, None);

        view.update(cx, reopen);
        window.render_frame(cx);
        assert_eq!(view.read(cx).sidebar_reveal, Some(SidebarReveal::Custom));
        view.update(cx, |view, cx| {
            view.custom.update(cx, |custom, cx| custom.retry(cx))
        });
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).sidebar_reveal, None);
        assert!(shown_in_sidebar(window, "nav-k8s-api-velero.io"));
        assert!(window.try_find(certificates).is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn narrow_shell_log_catalog_and_multiline_errors_preserve_viewport(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        let events = (0..30)
            .map(|ix| {
                freshkube_core::logs::LogEvent::new(
                    format!("service-{ix:02}-long-catalog-name"),
                    format!("level=info fixture catalog service {ix}"),
                )
            })
            .collect();
        let failures = (0..30)
            .map(|ix| {
                (
                    freshkube_core::logs::ServiceId::new(format!(
                        "service-{ix:02}-long-catalog-name"
                    )),
                    format!(
                        "Service {ix}: connection unavailable\n{}\nRetry on the next manual refresh",
                        "Multiline diagnostic context 中文 ".repeat(8)
                    ),
                )
            })
            .collect();
        let logs = view.read(cx).logs.clone();
        logs.update(cx, |logs, cx| {
            logs.set_fixture(events, window, cx);
            logs.set_fixture_failures(failures, cx);
        });
        window.render_frame(cx);
        // Navigate through the same production action as a keyboard user.
        window.press("secondary-3", cx);
        window.render_frame(cx);
        let viewport = window.find("logs-viewport");
        let panel = window.find("logs-panel");
        assert!(viewport.bounds().top() >= panel.bounds().top());
        assert!(viewport.bounds().left() >= panel.bounds().left());
        assert!(viewport.bounds().right() <= panel.bounds().right());
        assert!(viewport.bounds().bottom() <= panel.bounds().bottom());
        assert!(viewport.visible());
        assert!(viewport.bounds().size.height >= window.rem_size() * 6.);
        assert!(viewport.bounds().right() <= px(760.));
        assert!(viewport.bounds().bottom() <= px(560.));
        let notices = window.find("logs-notices");
        assert!(notices.visible());
        assert!(notices.bounds().size.height >= window.rem_size());
        assert!(notices.bounds().bottom() <= viewport.bounds().top());
        let status = window.find("logs-status");
        assert!(status.visible());
        assert!(status.bounds().top() >= viewport.bounds().bottom());
        assert!(status.bounds().bottom() <= px(560.));
        assert!(window.find("logs-collection").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn browse_picks_a_talosconfig_and_reloads_contexts(cx: &mut TestAppContext) {
    // Configuration loads on Tokio; let GPUI park for its completion.
    cx.executor().allow_parking();
    let directory =
        std::env::temp_dir().join(format!("freshkube-desktop-browse-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let chosen = directory.join("config");
    std::fs::write(
        &chosen,
        "context: alpha\ncontexts:\n  alpha: &entry\n    endpoints: [192.0.2.1]\n    ca: YQ==\n    crt: Yg==\n    key: Yw==\n  beta: *entry\n",
    )
    .unwrap();
    // Never the user's default talosconfig: start from a missing file.
    let options = GpuiOptions::new(Some(directory.join("missing")), None, 100);
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).config_error.is_some()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The error screen offers Browse directly; cancelling changes nothing.
        window.click("browse-config-empty", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    cx.update(|cx| assert!(view.read(cx).config_error.is_some()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        window.click("browse-config", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.directories && !options.multiple);
        Some(vec![chosen.clone()])
    });
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).contexts == ["alpha", "beta"]
    })
    .await;
    cx.update(|cx| {
        let pilot = view.read(cx);
        assert_eq!(pilot.applied.path.as_deref(), Some(chosen.as_path()));
        assert_eq!(pilot.path.read(cx).value(), chosen.display().to_string());
        assert!(pilot.config_error.is_none());
    });
    let _ = std::fs::remove_dir_all(&directory);
}

#[gpui_kit::test]
fn browse_is_unavailable_for_example_data(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        window.click("browse-config", cx);
    })
    .unwrap();
    assert!(!cx.did_prompt_for_paths());
}

#[gpui_kit::test]
async fn kubeconfig_file_applies_its_current_context_then_a_chosen_one(cx: &mut TestAppContext) {
    use freshkube_core::cluster_overview::KubeconfigSelection;
    // Inspection runs on Tokio; let GPUI park for its completion.
    cx.executor().allow_parking();
    let directory = std::env::temp_dir().join(format!(
        "freshkube-desktop-kubeconfig-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let chosen = directory.join("kubeconfig");
    std::fs::write(
        &chosen,
        "apiVersion: v1\nkind: Config\ncurrent-context: beta\nclusters:\n- name: c\n  cluster:\n    server: https://192.0.2.1:6443\ncontexts:\n- name: alpha\n  context: {cluster: c, user: u}\n- name: beta\n  context: {cluster: c, user: u}\nusers:\n- name: u\n  user: {}\n",
    )
    .unwrap();
    // Never the user's default talosconfig: start from a missing file.
    let options = GpuiOptions::new(Some(directory.join("missing")), None, 100);
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        // File mode waits for a file; nothing changes yet.
        window.click("kubeconfig-file", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).kubeconfig, KubeconfigSelection::Automatic);
        assert!(window.find("kubeconfig-path").visible());
        window.click("browse-kubeconfig", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.directories && !options.multiple);
        Some(vec![chosen.clone()])
    });
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).kubeconfig_draft.inspection.is_some()
    })
    .await;
    // A valid current context is used right away.
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).kubeconfig,
            KubeconfigSelection::File {
                path: chosen.clone(),
                context: None
            }
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        assert_eq!(window.find(("kube-context", 1usize)).selected(), Some(true));
        window.click(("kube-context", 0usize), cx);
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).kubeconfig,
            KubeconfigSelection::File {
                path: chosen.clone(),
                context: Some("alpha".into())
            }
        );
        assert_eq!(window.find(("kube-context", 0usize)).selected(), Some(true));
        window.click("kubeconfig-talos", cx);
        assert_eq!(
            view.read(cx).kubeconfig,
            KubeconfigSelection::TalosControlPlane
        );
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&directory);
}

#[gpui_kit::test]
async fn kubeconfig_without_a_usable_context_is_not_applied(cx: &mut TestAppContext) {
    use freshkube_core::cluster_overview::KubeconfigSelection;
    cx.executor().allow_parking();
    let directory = std::env::temp_dir().join(format!(
        "freshkube-desktop-kubeconfig-bad-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let unreadable = directory.join("not-yaml");
    std::fs::write(&unreadable, "{{{ not a kubeconfig").unwrap();
    let options = GpuiOptions::new(Some(directory.join("missing")), None, 100);
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        window.click("kubeconfig-file", cx);
        window.render_frame(cx);
        window.click("browse-kubeconfig", cx);
    })
    .unwrap();
    cx.simulate_path_prompt_response(|_| Some(vec![unreadable.clone()]));
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).kubeconfig_draft.inspection.is_some()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).kubeconfig, KubeconfigSelection::Automatic);
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        assert!(window.find("kubeconfig-error").visible());
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&directory);
}

#[gpui_kit::test]
fn kubeconfig_choice_is_unavailable_for_example_data(cx: &mut TestAppContext) {
    use freshkube_core::cluster_overview::KubeconfigSelection;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        window.click("kubeconfig-talos", cx);
        assert_eq!(view.read(cx).kubeconfig, KubeconfigSelection::Automatic);
    })
    .unwrap();
}

fn confirmation(
    current: Rc<Cell<u64>>,
    confirmed: Rc<Cell<usize>>,
) -> crate::mutation::Confirmation {
    crate::mutation::Confirmation {
        title: "Reboot talos-wk-fra1-02".into(),
        summary: "Drains the node, then reboots it.".into(),
        facts: vec![("Pods to evict".into(), "12".into())],
        warnings: Vec::new(),
        confirm_label: "Reboot".into(),
        destructive: true,
        fingerprint: 1,
        current: Rc::new(move |_| Some(current.get())),
        on_confirm: Rc::new(move |_, _| confirmed.set(confirmed.get() + 1)),
    }
}

/// Opens a confirmation and lets its open animation settle, so clicks land
/// where the dialog finally is.
fn open_confirmation(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    current: &Rc<Cell<u64>>,
    confirmed: &Rc<Cell<usize>>,
) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        crate::mutation::confirm(confirmation(current.clone(), confirmed.clone()), window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
fn confirmation_runs_only_against_the_preview_shown(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    let current = Rc::new(Cell::new(1));
    let confirmed = Rc::new(Cell::new(0));
    open_confirmation(cx, handle, &current, &confirmed);
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("confirm-dialog").visible());
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert_eq!(confirmed.get(), 1);
        assert!(window.try_find("confirm-dialog").is_none());
    })
    .unwrap();

    // The cluster changed while the dialog was open: refuse, explain, and
    // don't let a second click through either.
    open_confirmation(cx, handle, &current, &confirmed);
    current.set(2);
    cx.update_window(handle, |_, window, cx| {
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert_eq!(confirmed.get(), 1);
        assert!(window.find("confirm-blocked").visible());
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert_eq!(confirmed.get(), 1);
        window.click("confirm-cancel", cx);
        window.render_frame(cx);
        assert!(window.try_find("confirm-dialog").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn one_operation_at_a_time_with_status_cancel_and_close_guard(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    let operations = cx.update(crate::mutation::Operations::global);
    let ticket = operations
        .update(cx, |operations, cx| {
            operations.begin("Reboot talos-wk-fra1-02", cx)
        })
        .ok()
        .unwrap();
    assert!(
        operations
            .update(cx, |operations, cx| operations
                .begin("Drain talos-wk-fra1-01", cx))
            .is_err()
    );
    let cancelled = ticket.cancellation();
    let current = Rc::new(Cell::new(1));
    let confirmed = Rc::new(Cell::new(0));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("operation-status")
                .label()
                .unwrap()
                .starts_with("Reboot talos-wk-fra1-02")
        );
    })
    .unwrap();
    // A second mutation can't be confirmed while one runs.
    open_confirmation(cx, handle, &current, &confirmed);
    cx.update_window(handle, |_, window, cx| {
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert_eq!(confirmed.get(), 0);
        assert!(window.find("confirm-blocked").visible());
        window.click("confirm-cancel", cx);
        window.render_frame(cx);
        // Closing is refused with an explanation.
        assert!(!crate::mutation::may_close(window, cx));
    })
    .unwrap();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Keep running");
    cx.update_window(handle, |_, window, cx| {
        window.click("operation-cancel", cx);
        window.render_frame(cx);
        assert!(cancelled());
        assert!(
            window
                .find("operation-status")
                .label()
                .unwrap()
                .contains("stopping")
        );
    })
    .unwrap();
    operations.update(cx, |operations, cx| operations.finish(ticket, cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("operation-status").is_none());
        assert!(crate::mutation::may_close(window, cx));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_dropped_ticket_frees_the_slot(cx: &mut TestAppContext) {
    let operations = cx.update(crate::mutation::Operations::global);
    let ticket = operations
        .update(cx, |operations, cx| operations.begin("Drain", cx))
        .ok()
        .unwrap();
    drop(ticket);
    assert!(cx.read(crate::mutation::Operations::current).is_none());
    assert!(
        operations
            .update(cx, |operations, cx| operations.begin("Drain", cx))
            .is_ok()
    );
}

/// Selects `service` on the Services page and opens its restart confirmation.
fn open_restart(cx: &mut TestAppContext, handle: AnyWindowHandle, service: &'static str) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-services", cx);
        window.render_frame(cx);
        window.within("services-region").click(service, cx);
        window.render_frame(cx);
        window.click("service-restart", cx);
    })
    .unwrap();
    crate::mutation::settle_confirmation(cx, handle);
}

#[gpui_kit::test]
fn restart_is_offered_only_for_a_selected_service_on_an_answering_node(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-services", cx);
        window.render_frame(cx);
        // Nothing selected yet: there is nothing to restart.
        assert!(window.try_find("service-restart").is_none());
        window.within("services-region").click("containerd", cx);
        window.render_frame(cx);
        assert!(window.find("service-restart").visible());
        assert_eq!(
            window.find("service-restart").label(),
            Some("Restart containerd")
        );
    })
    .unwrap();
    // While another operation holds the slot, the button is off.
    let operations = cx.update(crate::mutation::Operations::global);
    let ticket = operations
        .update(cx, |operations, cx| {
            operations.begin("Drain talos-wk-fra1-01", cx)
        })
        .ok()
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Pressing it does nothing: no dialog opens.
        window.click("service-restart", cx);
        window.render_frame(cx);
        assert!(window.try_find("confirm-dialog").is_none());
    })
    .unwrap();
    cx.run_until_parked();
    operations.update(cx, |operations, cx| operations.finish(ticket, cx));
    // A node that doesn't answer has no services to restart.
    cx.update_window(handle, |_, window, cx| {
        pick_target(window, cx, 5);
        window.render_frame(cx);
        assert!(window.try_find("service-restart").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn confirming_a_restart_in_example_mode_simulates_it_and_frees_the_slot(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    open_restart(cx, handle, "containerd");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            window.find("confirm-dialog").label(),
            Some("Restart containerd?")
        );
        assert!(window.try_find("service-restart-result").is_none());
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert!(window.try_find("confirm-dialog").is_none());
        assert!(window.try_find("confirm-blocked").is_none());
        // Run and finished within the click: the slot is free again.
        assert!(crate::mutation::Operations::current(cx).is_none());
        let result = window
            .find("service-restart-result")
            .label()
            .map(str::to_owned)
            .unwrap();
        assert!(result.contains("Example data"), "{result}");
        assert!(result.contains("containerd"), "{result}");
        assert_eq!(
            view.read(cx).selected_service.as_deref(),
            Some("containerd")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn dangerous_services_are_confirmed_by_name(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    for service in ["etcd", "apid"] {
        open_restart(cx, handle, service);
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(
                window.find("confirm-dialog").label().map(str::to_owned),
                Some(format!("Restart {service}?"))
            );
            window.click("confirm-cancel", cx);
            window.render_frame(cx);
            assert!(window.try_find("confirm-dialog").is_none());
            assert!(crate::mutation::Operations::current(cx).is_none());
            // Cancelling ran nothing.
            assert!(window.try_find("service-restart-result").is_none());
        })
        .unwrap();
        cx.run_until_parked();
    }
    assert!(crate::actions::is_critical_service("etcd"));
    assert!(crate::actions::is_critical_service("machined"));
    assert!(!crate::actions::is_critical_service("containerd"));
}

#[gpui_kit::test]
fn a_stale_restart_preview_is_refused_and_runs_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    open_restart(cx, handle, "containerd");
    // The target moved on while the dialog was open.
    view.update(cx, |view, _| view.epoch = view.epoch.wrapping_add(1));
    cx.update_window(handle, |_, window, cx| {
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert!(window.find("confirm-blocked").visible());
        assert!(window.find("confirm-dialog").visible());
        assert!(crate::mutation::Operations::current(cx).is_none());
        assert!(window.try_find("service-restart-result").is_none());
        window.click("confirm-cancel", cx);
    })
    .unwrap();
    // Switching to another node also invalidates a preview.
    cx.run_until_parked();
    open_restart(cx, handle, "containerd");
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.select_node(Some(DEGRADED_NODE.to_owned()), window, cx)
        });
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert!(window.find("confirm-blocked").visible());
        assert!(window.try_find("service-restart-result").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_restart_is_refused_while_another_operation_runs(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    open_restart(cx, handle, "containerd");
    let operations = cx.update(crate::mutation::Operations::global);
    let ticket = operations
        .update(cx, |operations, cx| {
            operations.begin("Reboot talos-wk-fra1-02", cx)
        })
        .ok()
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.click("confirm-ok", cx);
        window.render_frame(cx);
        assert!(window.find("confirm-blocked").visible());
        assert!(window.try_find("service-restart-result").is_none());
        // The operation that holds the slot is untouched.
        assert!(
            crate::mutation::Operations::current(cx)
                .is_some_and(|running| running.label.starts_with("Reboot"))
        );
    })
    .unwrap();
    operations.update(cx, |operations, cx| operations.finish(ticket, cx));
}

/// Draws what changed since the last frame. Unlike `render_frame` this keeps
/// GPUI's view cache, which is what these tests are about.
fn draw(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    window.draw(cx).clear(cx);
}

fn count(name: &'static str) -> usize {
    super::probe::count(name)
}

#[gpui_kit::test]
fn hidden_log_batches_do_not_redraw_the_window(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let shell_notices = Rc::new(Cell::new(0usize));
    let log_notices = Rc::new(Cell::new(0usize));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let logs = view.read(cx).logs.clone();
        let events = (0..20)
            .map(|ix| {
                freshkube_core::logs::LogEvent::new("apid", format!("level=info seed line {ix}"))
            })
            .collect();
        logs.update(cx, |logs, cx| logs.set_fixture(events, window, cx));
        window.render_frame(cx);
        draw(window, cx);
        let (counter, seen) = (shell_notices.clone(), log_notices.clone());
        cx.observe(&view, move |_, _| counter.set(counter.get() + 1))
            .detach();
        cx.observe(&logs, move |_, _| seen.set(seen.get() + 1))
            .detach();
        let (overview, panel) = (count("overview"), count("logs"));
        let (applied, _) = logs.read(cx).applied_and_held();
        for batch in 0..5 {
            let lines = (0..10).map(|ix| format!("level=info hidden {batch}-{ix}"));
            assert!(logs.update(cx, |logs, cx| logs.push_fixture_batch(lines.collect(), cx)));
            draw(window, cx);
        }
        // Held back, not applied one by one, and nothing asked for a redraw.
        let (now, held) = logs.read(cx).applied_and_held();
        assert_eq!(now, applied);
        assert_eq!(held, 50);
        // Past the coalescing interval the whole backlog lands at once.
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(logs.update(cx, |logs, cx| {
            logs.push_fixture_batch(vec!["level=info late".into()], cx)
        }));
        draw(window, cx);
        let (now, held) = logs.read(cx).applied_and_held();
        assert_eq!((now, held), (applied + 51, 0));
        assert_eq!(log_notices.get(), 0, "hidden batches must not notify");
        assert_eq!(shell_notices.get(), 0, "the shell must not hear of them");
        assert_eq!(count("overview"), overview);
        assert_eq!(count("logs"), panel);
    })
    .unwrap();
    // Showing Logs applies anything still held and draws it.
    cx.update_window(handle, |_, window, cx| {
        let logs = view.read(cx).logs.clone();
        assert!(logs.update(cx, |logs, cx| {
            logs.push_fixture_batch(vec!["level=info held".into()], cx)
        }));
        window.click("nav-logs", cx);
        draw(window, cx);
        assert_eq!(logs.read(cx).applied_and_held().1, 0);
        assert!(count("logs") > 0);
        let visible = count("logs");
        assert!(logs.update(cx, |logs, cx| {
            logs.push_fixture_batch(vec!["level=info shown".into()], cx)
        }));
        draw(window, cx);
        assert!(count("logs") > visible, "a visible panel redraws");
    })
    .unwrap();
}

/// One countdown tick as the shell's timer applies it.
fn tick(view: &Entity<Pilot>, cx: &mut TestAppContext) {
    cx.update(|cx| {
        view.update(cx, |pilot, cx| {
            pilot.elapsed += std::time::Duration::from_secs(1);
            pilot.tick_countdown(cx);
        });
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn tick_does_not_redraw_the_active_screen(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-processes", cx);
        window.render_frame(cx);
        draw(window, cx);
        draw(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let (screen, ring) = cx.update(|cx| {
        (
            count("processes"),
            view.read(cx).countdown.read(cx).remaining,
        )
    });
    assert!(screen > 0);
    for _ in 0..3 {
        tick(&view, cx);
        cx.update_window(handle, |_, window, cx| draw(window, cx))
            .unwrap();
    }
    cx.update(|cx| {
        assert_ne!(
            view.read(cx).countdown.read(cx).remaining,
            ring,
            "ring moves"
        );
    });
    assert_eq!(count("processes"), screen, "the screen stays as drawn");
}

#[gpui_kit::test]
fn tick_does_not_redraw_overview(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        draw(window, cx);
        draw(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let overview = count("overview");
    assert!(overview > 0);
    tick(&view, cx);
    cx.update_window(handle, |_, window, cx| draw(window, cx))
        .unwrap();
    assert_eq!(count("overview"), overview);
}

#[gpui_kit::test]
fn cached_screen_redraws_after_data_and_theme_change(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-processes", cx);
        window.render_frame(cx);
        draw(window, cx);
        draw(window, cx);
        let drawn = count("processes");
        draw(window, cx);
        assert_eq!(count("processes"), drawn, "nothing changed, nothing drawn");
        // New data.
        let screen = view.read(cx).active_screen().unwrap();
        screen.refresh(window, cx);
        draw(window, cx);
        let after_data = count("processes");
        assert!(after_data > drawn, "data change must redraw the screen");
        // Theme.
        let light = cx.theme().background;
        Theme::change(ThemeMode::Dark, Some(window), cx);
        draw(window, cx);
        assert_ne!(cx.theme().background, light);
        let after_theme = count("processes");
        assert!(after_theme > after_data, "theme change must redraw it");
    })
    .unwrap();
    // Font size: observers run when the update ends, before the next frame.
    let before = count("processes");
    cx.update(|cx| Theme::update(cx, |theme| theme.font_size = px(20.)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| draw(window, cx))
        .unwrap();
    assert!(count("processes") > before, "font size must redraw it");
}

#[gpui_kit::test]
fn cached_overview_redraws_after_data_and_theme_change(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        draw(window, cx);
        draw(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let drawn = count("overview");
    cx.update(|cx| view.update(cx, |pilot, cx| pilot.simulate_failure(cx)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| draw(window, cx))
        .unwrap();
    let after_data = count("overview");
    assert!(after_data > drawn, "data change must redraw the page");
    cx.update_window(handle, |_, window, cx| {
        Theme::change(ThemeMode::Dark, Some(window), cx);
        draw(window, cx);
    })
    .unwrap();
    assert!(
        count("overview") > after_data,
        "theme change must redraw it"
    );
}

/// A kubeconfig whose server refuses at once, so nothing leaves this
/// machine, with a fake token.
const KUBECONFIG: &str = "apiVersion: v1
kind: Config
current-context: lab
clusters:
- name: lab
  cluster:
    server: https://127.0.0.1:1
contexts:
- name: lab
  context:
    cluster: lab
    user: lab
- name: staging
  context:
    cluster: lab
    user: lab
    namespace: web
users:
- name: lab
  user:
    token: not-a-real-token
";

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("freshkube-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn kubeconfig_file(name: &str) -> std::path::PathBuf {
    let path = scratch(name);
    std::fs::write(&path, KUBECONFIG).unwrap();
    path
}

/// The window `--kubernetes-only --kubeconfig <path>` opens. Never the
/// ambient kubeconfig: the path is always explicit.
fn kubernetes_only(
    cx: &mut TestAppContext,
    kubeconfig: std::path::PathBuf,
    context: Option<&str>,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    // Reading and connecting run on Tokio and wake GPUI from there.
    cx.executor().allow_parking();
    let options = GpuiOptions::kubernetes_only(Some(kubeconfig), context.map(Into::into), 100);
    mount(cx, options, 1280., 860.)
}

/// Renders until `ready` holds, letting Tokio work land between frames.
fn wait_until(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    what: &str,
    ready: impl Fn(&mut gpui_kit::Window, &mut gpui_kit::App) -> bool,
) {
    for _ in 0..1000 {
        cx.run_until_parked();
        let done = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                ready(window, cx)
            })
            .unwrap();
        if done {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

fn kubernetes_status(window: &gpui_kit::Window) -> String {
    window
        .find("kubernetes-status")
        .label()
        .unwrap_or_default()
        .to_owned()
}

#[gpui_kit::test]
fn kubernetes_only_ctrl_tab_skips_pages_that_need_talos(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = kubernetes_only(cx, kubeconfig_file("ctrl-tab"), None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Resources);
        // Every other page would only say it needs a talosconfig.
        window.press("ctrl-tab", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Resources);
        window.press("secondary-2", cx);
        window.render_frame(cx);
        assert!(window.find("needs-talosconfig").visible());
        window.press("ctrl-shift-tab", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(window.find("resource-body").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubernetes_only_lists_kubeconfig_contexts_and_connects_to_the_current_one(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = kubernetes_only(cx, kubeconfig_file("contexts"), None);
    wait_until(cx, handle, "lab to refuse", |window, _| {
        kubernetes_status(window).starts_with("Couldn't connect to lab: ")
    });
    // The page read through that same connection, and failed with it.
    wait_until(cx, handle, "the page to fail", |window, _| {
        window.try_find("resource-failed").is_some()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let pilot = view.read(cx);
        assert_eq!(pilot.contexts, ["lab", "staging"]);
        assert_eq!(pilot.applied.context.as_deref(), Some("lab"));
        assert_eq!(pilot.page, Page::Resources);
        assert_eq!(window.find(("context", 0usize)).selected(), Some(true));
        assert_eq!(window.find("page-title").label(), Some("Pods"));
        assert!(
            window
                .find("applied-config")
                .label()
                .unwrap()
                .starts_with("Kubeconfig: ")
        );
        // No nodes to target without Talos.
        assert!(window.try_find("target-node").is_none());

        // Choosing the failed context again retries it.
        window.click(("context", 0usize), cx);
        window.render_frame(cx);
        assert_eq!(kubernetes_status(window), "Connecting to lab…");
    })
    .unwrap();
    wait_until(cx, handle, "lab to refuse again", |window, _| {
        kubernetes_status(window).starts_with("Couldn't connect to lab: ")
    });

    cx.update_window(handle, |_, window, cx| {
        window.click(("context", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("staging"));
        assert_eq!(window.find(("context", 1usize)).selected(), Some(true));
        assert_eq!(window.find(("context", 0usize)).selected(), Some(false));
    })
    .unwrap();
    wait_until(cx, handle, "staging to refuse", |window, _| {
        kubernetes_status(window).starts_with("Couldn't connect to staging: ")
    });
}

#[gpui_kit::test]
fn kubernetes_only_talos_pages_ask_for_a_talosconfig(cx: &mut TestAppContext) {
    let path = kubeconfig_file("talos-pages");
    let (_runtime, handle, view) = kubernetes_only(cx, path.clone(), None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for page in Page::ALL
            .into_iter()
            .filter(|page| *page != Page::Resources)
        {
            let nav = format!("nav-{}", page.slug());
            reveal(window, cx, &nav);
            window.click(SharedString::from(nav), cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).page, page);
            assert!(window.find("needs-talosconfig").visible(), "{page:?}");
        }
        window.click("browse-kubernetes", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Resources);
        assert!(window.try_find("needs-talosconfig").is_none());

        reveal(window, cx, "nav-etcd");
        window.click("nav-etcd", cx);
        window.render_frame(cx);
        window.click("open-settings", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("talosconfig-path").visible());
        let files = format!("Kubeconfig files: {}", path.display());
        assert_eq!(window.find("kubeconfig-path").label(), Some(files.as_str()));
        // The Talos kubeconfig choices don't apply without Talos.
        assert!(window.try_find("kubeconfig-automatic").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubernetes_only_never_swaps_in_another_context(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = kubernetes_only(cx, kubeconfig_file("asked-for"), Some("prod"));
    wait_until(cx, handle, "prod to be missing", |window, _| {
        kubernetes_status(window).starts_with("Couldn't connect to prod: ")
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let status = kubernetes_status(window);
        assert!(status.contains("Context 'prod' was not found"), "{status}");
        let pilot = view.read(cx);
        assert_eq!(pilot.applied.context.as_deref(), Some("prod"));
        assert_eq!(pilot.contexts, ["lab", "staging"]);
        assert_eq!(window.find(("context", 0usize)).selected(), Some(false));
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubernetes_only_reports_a_missing_kubeconfig_and_switches_to_talos(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = kubernetes_only(cx, scratch("no-such-kubeconfig"), None);
    wait_until(cx, handle, "the kubeconfig to fail", |window, _| {
        kubernetes_status(window).starts_with("No kubeconfig loaded: ")
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let status = kubernetes_status(window);
        assert!(
            status.contains("file is unreadable or does not exist"),
            "{status}"
        );
        assert!(view.read(cx).contexts.is_empty());
        assert!(window.find("resource-disconnected").visible());

        // Applying no talosconfig keeps the window as it is: the default
        // one is what's missing.
        view.update(cx, |view, cx| view.apply_config_path(window, cx));
        assert!(view.read(cx).kubernetes_only.is_some());
        // A talosconfig switches the window to Talos.
        view.update(cx, |view, cx| {
            let path = scratch("no-such-talosconfig").display().to_string();
            view.path
                .update(cx, |input, cx| input.set_value(path, window, cx));
            view.apply_config_path(window, cx);
        });
        window.render_frame(cx);
        assert!(view.read(cx).kubernetes_only.is_none());
        assert!(window.find("target-node").visible());
        assert!(window.try_find("kubernetes-status").is_none());
    })
    .unwrap();
}
