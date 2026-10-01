use super::{GpuiOptions, NodeView, Page, Pilot};
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
fn ctrl_tab_cycles_through_every_screen(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("secondary-9", cx);
        window.render_frame(cx);
        for expected in [
            Page::Security,
            Page::Lifecycle,
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
        window.scroll(
            "sidebar-scroll",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-400.))),
            cx,
        );
        window.render_frame(cx);
        for page in [Page::Security, Page::Lifecycle, Page::Operations] {
            window.click(SharedString::from(format!("nav-{}", page.slug())), cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).page, page);
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn narrow_shell_log_catalog_and_multiline_errors_preserve_viewport(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        let events = (0..30)
            .map(|ix| {
                talos_pilot_core::logs::LogEvent::new(
                    format!("service-{ix:02}-long-catalog-name"),
                    format!("level=info fixture catalog service {ix}"),
                )
            })
            .collect();
        let failures = (0..30)
            .map(|ix| {
                (
                    talos_pilot_core::logs::ServiceId::new(format!(
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
        std::env::temp_dir().join(format!("talos-pilot-gpui-browse-{}", std::process::id()));
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
    use talos_pilot_core::cluster_overview::KubeconfigSelection;
    // Inspection runs on Tokio; let GPUI park for its completion.
    cx.executor().allow_parking();
    let directory = std::env::temp_dir().join(format!(
        "talos-pilot-gpui-kubeconfig-{}",
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
    use talos_pilot_core::cluster_overview::KubeconfigSelection;
    cx.executor().allow_parking();
    let directory = std::env::temp_dir().join(format!(
        "talos-pilot-gpui-kubeconfig-bad-{}",
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
    use talos_pilot_core::cluster_overview::KubeconfigSelection;
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
                talos_pilot_core::logs::LogEvent::new("apid", format!("level=info seed line {ix}"))
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
