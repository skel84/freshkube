use super::{Area, ColumnReveal, GpuiOptions, NodeView, Page, Pilot};
use crate::logs::TalosPanel;
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext,
    component::{ActiveTheme, Root, Theme, ThemeMode},
    px, size,
};
use std::cell::Cell;
use std::rc::Rc;

pub(super) fn fixture(
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
        crate::text_size::install(None, cx);
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

fn root_pilot(window: &gpui_kit::Window, cx: &gpui_kit::App) -> Entity<Pilot> {
    window
        .root::<Root>()
        .unwrap()
        .unwrap()
        .read(cx)
        .view()
        .clone()
        .downcast::<Pilot>()
        .unwrap()
}

/// Test setup that needs a healthy table row follows the actual group control.
/// Folding itself is exercised separately by Nodes' projection/entry-point tests.
pub(super) fn expand_healthy_nodes(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    let pilot = root_pilot(window, cx);
    let nodes = &pilot.read(cx).node_workspace;
    if nodes.healthy_collapsed() && nodes.view == NodeView::Table && !nodes.open {
        window.click("nodes-healthy-toggle", cx);
        window.render_frame(cx);
    }
}

/// Choose through the Nodes page, then return to the previously visible view.
fn pick_target(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, ix: usize) {
    let view = root_pilot(window, cx);
    let page = view.read(cx).page;
    let tab = view.read(cx).node_workspace.tab;
    window.click("nav-nodes", cx);
    window.render_frame(cx);
    if view.read(cx).node_workspace.open {
        window.click("node-close", cx);
        window.render_frame(cx);
    }
    expand_healthy_nodes(window, cx);
    let id = view.read(cx).node_workspace.rows[ix].id.clone();
    window.click(id, cx);
    window.render_frame(cx);
    if page == Page::Nodes {
        view.update(cx, |view, cx| view.show_node_tab(tab, window, cx));
    } else {
        view.update(cx, |view, cx| view.navigate(page, window, cx));
    }
    window.render_frame(cx);
}

/// Old page coverage now follows the actual node-pane tab controls.
fn open_node_tab(
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
    tab: super::nodes::NodeTab,
) {
    let view = root_pilot(window, cx);
    let name = view.read(cx).selected_node.clone().unwrap();
    area(window, cx, "nav-nodes");
    if !view.read(cx).node_workspace.open {
        expand_healthy_nodes(window, cx);
        let id = view
            .read(cx)
            .node_workspace
            .rows
            .iter()
            .find(|row| row.key.talos.as_ref() == Some(&name))
            .unwrap()
            .id
            .clone();
        window.click(id, cx);
        window.render_frame(cx);
    }
    if window.try_find(tab.id()).is_some_and(|button| {
        let pane = window.find("node-pane").bounds();
        button.visible()
            && button.bounds().left() >= pane.left()
            && button.bounds().right() <= pane.right()
    }) {
        window.click(tab.id(), cx);
    } else {
        for _ in 0..10 {
            if view.read(cx).node_workspace.tab == tab {
                break;
            }
            window.press("secondary-}", cx);
            window.render_frame(cx);
        }
    }
    window.render_frame(cx);
}

fn contexts(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    if !root_pilot(window, cx).read(cx).context_display.open {
        window.click("context-switcher", cx);
        window.render_frame(cx);
    }
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
        open_node_tab(window, cx, super::nodes::NodeTab::Services);
        window.render_frame(cx);
        window.within("services-region").click("containerd", cx);
    })
    .unwrap();
    cx.run_until_parked();
    for (nav, page) in [
        ("nav-overview", Page::Overview),
        ("node-tab-services", Page::Nodes),
        ("node-tab-logs", Page::Nodes),
    ] {
        cx.update_window(handle, |_, window, cx| {
            match nav {
                "node-tab-services" => open_node_tab(window, cx, super::nodes::NodeTab::Services),
                "node-tab-logs" => open_node_tab(window, cx, super::nodes::NodeTab::Logs),
                _ => window.click(nav, cx),
            };
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
            if nav == "node-tab-logs" {
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
            if nav == "node-tab-logs" {
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
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        expand_healthy_nodes(window, cx);
        assert_eq!(view.read(cx).selected_node.as_deref(), Some(FIRST_NODE));
        window
            .within("nodes-page")
            .click("node-talos-cp-fra1-01", cx);
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
                .within("nodes-page")
                .find("node-talos-cp-fra1-02")
                .selected(),
            Some(true)
        );
        assert_eq!(
            window
                .within("nodes-page")
                .find("node-talos-cp-fra1-01")
                .selected(),
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
        open_node_tab(window, cx, super::nodes::NodeTab::Services);
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
        contexts(window, cx);
        window.click(("context", 1usize), cx);
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
        contexts(window, cx);
        assert_eq!(window.find(("context", 1usize)).selected(), Some(true));
        // Nothing is selected, so there is no logs action to trigger.
        assert!(window.try_find("service-logs").is_none());
        assert_eq!(view.read(cx).page, Page::Nodes);
    })
    .unwrap();
}

#[gpui_kit::test]
fn service_keyboard_selection_filter_retains_domain_id(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_node_tab(window, cx, super::nodes::NodeTab::Services);
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
        open_node_tab(window, cx, super::nodes::NodeTab::Services);
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
        open_node_tab(window, cx, super::nodes::NodeTab::Services);
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
        assert_eq!(view.read(cx).page, Page::Nodes);
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
        open_node_tab(window, cx, super::nodes::NodeTab::Logs);
        window.render_frame(cx);
        window.click("logs-collection", cx);
        window.render_frame(cx);
        assert!(view.read(cx).logs.read(cx).is_collecting());
        window.click("nav-overview", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Overview);
        assert!(view.read(cx).logs.read(cx).is_collecting());
        assert!(view.read(cx).logs.read(cx).collecting_count() > 0);
        open_node_tab(window, cx, super::nodes::NodeTab::Logs);
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
        assert!(window.find("tile-services").visible());
        assert!(
            view.read(cx)
                .attention
                .rows
                .iter()
                .any(|row| row.name.contains(DEGRADED_NODE))
        );
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
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        assert!(window.try_find("target-node").is_none());
        pick_target(window, cx, 3);
        assert_eq!(
            view.read(cx).selected_node.as_deref(),
            Some("talos-wk-fra1-01")
        );
        window.click("nav-overview", cx);
        window.render_frame(cx);
        window.click("tile-services", cx);
        window.render_frame(cx);
        let pilot = view.read(cx);
        assert_eq!(pilot.page, Page::SystemServices);
        assert_eq!(pilot.selected_node.as_deref(), Some("talos-wk-fra1-01"));
        assert!(
            window
                .find("system-service-talos-wk-fra1-02-kubelet")
                .visible()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn silent_node_is_unknown_not_failed(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let card = window.find("node-talos-wk-fra1-03");
        assert!(card.label().unwrap().contains(SILENT_NODE));
        pick_target(window, cx, 5);
        open_node_tab(window, cx, super::nodes::NodeTab::Services);
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
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        expand_healthy_nodes(window, cx);
        window.click("nodes-view-table", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).node_workspace.view, NodeView::Table);
        assert_eq!(
            window
                .within("nodes-page")
                .find("node-talos-cp-fra1-01")
                .selected(),
            Some(false)
        );
        window
            .within("nodes-page")
            .click("node-talos-wk-fra1-01", cx);
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).selected_node.as_deref(),
            Some("talos-wk-fra1-01")
        );
        window.click("node-close", cx);
        window.render_frame(cx);
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).node_workspace.view, NodeView::Cards);
        assert_eq!(
            window
                .within("nodes-page")
                .find("node-talos-wk-fra1-01")
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
        assert!(window.find("context-switcher").bounds().right() <= px(760.));
        assert!(window.find("overview-cards").visible());
        assert!(window.find("overview-cards").bounds().right() <= px(760.));
        open_node_tab(window, cx, super::nodes::NodeTab::Services);
        window.render_frame(cx);
        assert!(window.find("theme-toggle").visible());
        assert!(window.find("service-filter").visible());
        assert!(window.find("services-region").bounds().size.height > px(0.));
        assert!(window.find("services-region").bounds().right() <= px(760.));
        assert!(window.find("services-region").bounds().top() < px(560.));
        open_node_tab(window, cx, super::nodes::NodeTab::Logs);
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
        for (key, page, kind) in [
            ("secondary-1", Page::Overview, None),
            ("secondary-2", Page::Nodes, None),
            ("secondary-3", Page::Resources, Some("namespaces")),
            ("secondary-4", Page::Resources, Some("events")),
            ("secondary-5", Page::Health, None),
            ("secondary-6", Page::Etcd, None),
            ("secondary-7", Page::SystemServices, None),
            ("secondary-8", Page::Security, None),
            ("secondary-9", Page::Lifecycle, None),
        ] {
            window.press(key, cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).page, page);
            if let Some(kind) = kind {
                assert_eq!(view.read(cx).resource_kind.key(), kind);
            }
        }
        window.press("secondary-1", cx);
        window.render_frame(cx);
        window.press("alt-down", cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("staging-eu"));
        window.press("alt-up", cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("prod-fra"));
        window.press("alt-up", cx);
        assert_eq!(
            view.read(cx).applied.context.as_deref(),
            Some("talos-production-frankfurt-equinix-fr5-baremetal-b7")
        );
        window.render_frame(cx);
        contexts(window, cx);
        assert!(window.find(("context", 0usize)).visible());
        assert_eq!(window.find(("context", 3usize)).selected(), Some(true));
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
        assert_eq!(view.read(cx).page, Page::Nodes);
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
        contexts(window, cx);
        window.click(("context", 2usize), cx);
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
        open_node_tab(window, cx, super::nodes::NodeTab::Logs);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Nodes);
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
            Page::Operations,
            Page::Overview,
            Page::Nodes,
            Page::Resources,
            Page::Resources,
            Page::Health,
            Page::Resources,
            Page::Etcd,
            Page::SystemServices,
            Page::Security,
            Page::Lifecycle,
        ] {
            window.press("ctrl-tab", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).page, expected);
        }
        window.press("ctrl-shift-tab", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Security);
    })
    .unwrap();
}

#[gpui_kit::test]
fn every_screen_is_reachable_and_loads_only_when_shown(cx: &mut TestAppContext) {
    // Minimum window size: the rail and column must scroll, not clip.
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for (area_id, pages) in [
            (
                "nav-control-plane",
                &[
                    Page::Etcd,
                    Page::Security,
                    Page::Lifecycle,
                    Page::Operations,
                ][..],
            ),
            ("nav-k8s-group-workloads", &[Page::Health][..]),
        ] {
            area(window, cx, area_id);
            for page in pages {
                reveal(window, cx, &format!("nav-{}", page.slug()));
            }
        }
        window.press("secondary-1", cx);
        window.render_frame(cx);
        // Hidden screens never ask for data.
        assert!(window.try_find("processes-page").is_none());
        open_node_tab(window, cx, super::nodes::NodeTab::Processes);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Nodes);
        assert_eq!(window.find("node-tab-processes").checked(), Some(true));
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
        // The retained screen scrolls under the node header in a short pane.
        for _ in 0..10 {
            if window.find("process-list").visible() {
                break;
            }
            window.scroll(
                "processes-page",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-60.))),
                cx,
            );
            window.render_frame(cx);
        }
        assert!(window.find("process-list").visible());
        for tab in [
            super::nodes::NodeTab::Storage,
            super::nodes::NodeTab::Network,
            super::nodes::NodeTab::Diagnostics,
            super::nodes::NodeTab::Logs,
            super::nodes::NodeTab::Services,
        ] {
            open_node_tab(window, cx, tab);
            assert_eq!(view.read(cx).node_workspace.tab, tab);
        }
        for (key, page) in [("secondary-6", Page::Etcd), ("secondary-5", Page::Health)] {
            window.press(key, cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).page, page);
        }
        // Control plane's pages are in its column.
        area(window, cx, "nav-control-plane");
        assert_eq!(view.read(cx).page, Page::Etcd);
        for page in [Page::Security, Page::Lifecycle] {
            let nav = format!("nav-{}", page.slug());
            reveal(window, cx, &nav);
            window.click(SharedString::from(nav), cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).page, page);
        }
        // Kubernetes kinds are in their group's column.
        assert!(window.try_find("resources-page").is_none());
        area(window, cx, "nav-k8s-group-workloads");
        reveal(window, cx, "nav-k8s-pods");
        window.click("nav-k8s-pods", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(window.find("nav-k8s-pods").checked(), Some(true));
        assert!(window.find("resource-list").visible());
        contexts(window, cx);
        assert!(window.find(("context", 0usize)).visible());
        area(window, cx, "nav-control-plane");
        reveal(window, cx, "nav-operations");
        window.click("nav-operations", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Operations);
        assert!(window.try_find("resources-page").is_none());
    })
    .unwrap();
}

/// Clicks the rail button `id`, scrolling the rail to it first.
fn area(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, id: &str) {
    let id = SharedString::from(id.to_owned());
    for _ in 0..30 {
        let rail = window.find("nav-rail").bounds();
        let button = window.find(id.clone()).bounds();
        if button.top() >= rail.top() && button.bottom() <= rail.bottom() {
            window.click(id, cx);
            window.render_frame(cx);
            return;
        }
        let above = button.top() < rail.top();
        window.scroll(
            "nav-rail",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                px(0.),
                px(if above { 40. } else { -40. }),
            )),
            cx,
        );
        window.render_frame(cx);
    }
    panic!("{id} never scrolled into the rail");
}

/// Scrolls the navigation column until `id` shows in full.
fn reveal(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, id: &str) {
    for _ in 0..60 {
        if shown_in_column(window, id) {
            return;
        }
        let area = window.find("nav-column").bounds();
        let above = window
            .try_find(SharedString::from(id.to_owned()))
            .is_some_and(|element| element.bounds().top() < area.top());
        window.scroll(
            "nav-column",
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

/// Whether `id` shows in full inside the scrolled navigation column.
fn shown_in_column(window: &mut gpui_kit::Window, id: &str) -> bool {
    let area = window.find("nav-column").bounds();
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
        assert!(shown_in_column(window, &nav));
        assert_eq!(window.find(SharedString::from(nav)).checked(), Some(true));
        assert_eq!(
            window.find("page-title").label(),
            Some("Validating Admission Policy Bindings")
        );

        // A group's rail button opens its first kind, with the group's kinds.
        view.update(cx, |view, cx| view.open_builtin("pods", window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(shown_in_column(window, "nav-k8s-pods"));
        area(window, cx, "nav-k8s-group-storage");
        window.render_frame(cx);
        assert_eq!(view.read(cx).resource_kind.key(), "persistentvolumeclaims");
        reveal(window, cx, "nav-k8s-csinodes.storage.k8s.io");
    })
    .unwrap();
}

#[gpui_kit::test]
fn rail_areas_list_their_kinds_and_open_the_last_one(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 1000.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Overview has no column; a group's rail button shows its kinds.
        assert_eq!(window.find("nav-overview").selected(), Some(true));
        assert!(window.try_find("nav-column").is_none());
        let workloads = window.find("nav-k8s-group-workloads");
        assert_eq!(workloads.role(), Some(gpui_kit::Role::Tab));
        assert_eq!(workloads.selected(), Some(false));
        area(window, cx, "nav-k8s-group-workloads");
        assert_eq!(view.read(cx).resource_kind.key(), "pods");
        assert_eq!(
            window.find("nav-k8s-group-workloads").selected(),
            Some(true)
        );
        assert!(window.try_find("nav-k8s-deployments.apps").is_some());
        assert!(window.try_find("nav-k8s-services").is_none());

        area(window, cx, "nav-k8s-group-networking");
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(view.read(cx).resource_kind.key(), "services");
        assert_eq!(window.find("nav-k8s-services").selected(), Some(true));
        assert_eq!(
            window.find("nav-k8s-group-workloads").selected(),
            Some(false)
        );
        assert!(window.find("resource-list").visible());
        assert_eq!(window.find("page-title").label(), Some("Services"));

        // Each group opens the kind it showed last.
        let ingresses = "nav-k8s-ingresses.networking.k8s.io";
        window.click(ingresses, cx);
        window.render_frame(cx);
        area(window, cx, "nav-k8s-group-workloads");
        assert_eq!(view.read(cx).resource_kind.key(), "pods");
        area(window, cx, "nav-k8s-group-networking");
        assert_eq!(
            view.read(cx).resource_kind.key(),
            "ingresses.networking.k8s.io"
        );
        assert_eq!(window.find(ingresses).selected(), Some(true));

        // Namespaces and Events are kinds of their own, with no column.
        window.click("nav-k8s-events", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).resource_kind.key(), "events");
        assert_eq!(window.find("nav-k8s-events").selected(), Some(true));
        assert!(window.try_find("nav-column").is_none());

        // Other pages hide the table, and coming back shows the last kind.
        window.press("secondary-1", cx);
        window.render_frame(cx);
        assert!(window.try_find("resources-page").is_none());
        for _ in 0..5 {
            window.press("ctrl-tab", cx);
            window.render_frame(cx);
        }
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(
            view.read(cx).resource_kind.key(),
            "ingresses.networking.k8s.io"
        );
        assert!(window.find("resources-page").visible());
        assert_eq!(
            window.find("nav-k8s-group-networking").selected(),
            Some(true)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_rail_marks_areas_the_overview_finds_problems_in(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 1000.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let marks = &view.read(cx).rail_marks;
        // The example cluster has NotReady nodes and a node at 92% memory,
        // High: both warn.
        assert_eq!(marks.tone(Area::Nodes), Some(crate::ui::Tone::Warn));
        assert_eq!(marks.tone(Area::ControlPlane), Some(crate::ui::Tone::Warn));
        assert_eq!(
            marks.tone(Area::Group("workloads")),
            Some(crate::ui::Tone::Warn)
        );
        assert_eq!(marks.tone(Area::Overview), None);
        assert_eq!(marks.tone(Area::Group("networking")), None);
    })
    .unwrap();
}

#[gpui_kit::test]
fn custom_resources_are_discovered_on_expand_and_open_their_kinds(cx: &mut TestAppContext) {
    // The short window: opening the section or a group scrolls its rows in.
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("nav-k8s-group-custom").selected(), Some(false));
        assert!(view.read(cx).custom.read(cx).groups().is_none());

        // With no custom kind shown yet, the page stays while the column
        // discovers the groups.
        area(window, cx, "nav-k8s-group-custom");
        window.click("nav-collapse", cx);
        window.render_frame(cx);
        assert_eq!(window.find("nav-k8s-group-custom").selected(), Some(true));
        assert_eq!(view.read(cx).page, Page::Overview);
        assert!(shown_in_column(window, "nav-k8s-api-velero.io"));
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
        assert!(shown_in_column(window, "nav-k8s-issuers.cert-manager.io"));

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

        // Another area hides the column; Custom Resources then opens the
        // custom kind shown last. Opening a kind, as the keyboard or
        // FRESHKUBE_KIND would, shows its group.
        area(window, cx, "nav-overview");
        assert!(window.try_find(certificates).is_none());
        area(window, cx, "nav-k8s-group-custom");
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(window.find(certificates).selected(), Some(true));
        let kind = crate::resources::example::kind("certificaterequests.cert-manager.io").unwrap();
        view.update(cx, |view, cx| view.open_kind(kind, window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
        let requests = "nav-k8s-certificaterequests.cert-manager.io";
        assert!(shown_in_column(window, requests));
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
            view.show_custom(cx);
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
        window.click("nav-collapse", cx);
        window.render_frame(cx);
        assert!(shown_in_column(window, certificates));
        // Another connection discovers again.
        window.press("alt-down", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("staging-eu"));
        // The section and the group stay open, with the kinds found again.
        assert_eq!(window.find("nav-k8s-group-custom").selected(), Some(true));
        assert_eq!(
            window.find("nav-k8s-api-cert-manager.io").expanded(),
            Some(true)
        );
        assert!(window.try_find(certificates).is_some());

        // While the groups are read, the column says so and the reveal
        // waits for them, unless the column is scrolled by hand.
        let reopen = |view: &mut Pilot, cx: &mut gpui_kit::Context<Pilot>| {
            view.set_area(Area::Overview, cx);
            view.show_custom(cx);
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
        assert_eq!(view.read(cx).column_reveal, Some(ColumnReveal::Custom));
        window.scroll(
            "nav-column",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(40.))),
            cx,
        );
        assert_eq!(view.read(cx).column_reveal, None);

        view.update(cx, reopen);
        window.render_frame(cx);
        assert_eq!(view.read(cx).column_reveal, Some(ColumnReveal::Custom));
        view.update(cx, |view, cx| {
            view.custom.update(cx, |custom, cx| custom.retry(cx))
        });
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).column_reveal, None);
        assert!(shown_in_column(window, "nav-k8s-api-velero.io"));
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
        // Reach the same retained log view through the node pane.
        open_node_tab(window, cx, super::nodes::NodeTab::Logs);
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
fn settings_shows_the_metrics_source_read_only_for_example_data(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        assert!(window.try_find("monitoring-source-settings").is_some());
        // Example data has nothing to choose.
        assert!(window.try_find("monitoring-source-mode").is_none());
        assert!(window.try_find("monitoring-source-test").is_none());
    })
    .unwrap();
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
async fn chosen_talosconfig_and_context_survive_a_launch_without_arguments(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let directory = std::env::temp_dir().join(format!(
        "freshkube-connection-restore-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let preferences = directory.join("preferences.json");
    let chosen = directory.join("talosconfig");
    // Invalid synthetic certificates stop before any network connection.
    std::fs::write(&chosen, "context: alpha\ncontexts:\n  alpha: &entry\n    endpoints: [192.0.2.1]\n    ca: YQ==\n    crt: Yg==\n    key: Yw==\n  beta: *entry\n").unwrap();
    let options = GpuiOptions::new(Some(directory.join("missing")), None, 100)
        .with_preferences(Some(preferences.clone()));
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).config_error.is_some()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("browse-config-empty", cx);
    })
    .unwrap();
    cx.simulate_path_prompt_response(|_| Some(vec![chosen.clone()]));
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).contexts == ["alpha", "beta"]
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        contexts(window, cx);
        window.click(("context", 1usize), cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("beta"));
    })
    .unwrap();
    let saved_path = directory.join("connection.json");
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, _| {
        std::fs::read_to_string(&saved_path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|saved| saved["context"] == "beta")
    })
    .await;
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&saved_path).unwrap()).unwrap();
    assert_eq!(saved["talosconfig"], chosen.display().to_string());
    assert!(saved.get("ca").is_none() && saved.get("crt").is_none() && saved.get("key").is_none());

    let restored = GpuiOptions::new(None, None, 100).with_preferences(Some(preferences));
    let (_runtime2, handle2, view2) = mount(cx, restored, 1280., 820.);
    cx.wait_for(handle2, std::time::Duration::from_secs(2), |_, cx| {
        view2.read(cx).contexts == ["alpha", "beta"]
    })
    .await;
    cx.update_window(handle2, |_, window, cx| {
        window.render_frame(cx);
        let pilot = view2.read(cx);
        assert_eq!(pilot.applied.path.as_deref(), Some(chosen.as_path()));
        assert_eq!(pilot.applied.context.as_deref(), Some("beta"));
        contexts(window, cx);
        assert_eq!(window.find(("context", 1usize)).selected(), Some(true));
    })
    .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
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
async fn connection_failure_offers_a_picker_and_bad_files_do_not_replace_saved_selection(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let directory = std::env::temp_dir().join(format!(
        "freshkube-connection-picker-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let preferences = directory.join("preferences.json");
    let first = directory.join("first");
    let second = directory.join("second");
    let invalid = directory.join("invalid");
    // These synthetic certificates fail parsing before a network dial.
    for (path, context) in [(&first, "alpha"), (&second, "beta")] {
        std::fs::write(path, format!("context: {context}\ncontexts:\n  {context}:\n    endpoints: [192.0.2.1]\n    ca: YQ==\n    crt: Yg==\n    key: Yw==\n")).unwrap();
    }
    std::fs::write(&invalid, "not a talosconfig").unwrap();
    let options = GpuiOptions::new(Some(first.clone()), None, 100)
        .with_preferences(Some(preferences.clone()));
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.wait_for(handle, std::time::Duration::from_secs(2), |window, _| {
        window.try_find("browse-config-unreachable").is_some()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        window.click("browse-config-unreachable", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(view.read(cx).applied.path.as_deref(), Some(first.as_path())));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("browse-config-unreachable", cx);
    })
    .unwrap();
    cx.simulate_path_prompt_response(|_| Some(vec![invalid.clone()]));
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).config_error.is_some()
    })
    .await;
    let saved_path = directory.join("connection.json");
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, _| {
        std::fs::read_to_string(&saved_path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|saved| saved["context"] == "alpha")
    })
    .await;
    let restored = GpuiOptions::new(None, None, 100).with_preferences(Some(preferences.clone()));
    assert_eq!(restored.config_path(), Some(first.as_path()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("browse-config-empty", cx);
    })
    .unwrap();
    cx.simulate_path_prompt_response(|_| Some(vec![second.clone()]));
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).contexts == ["beta"]
    })
    .await;
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, _| {
        std::fs::read_to_string(&saved_path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|saved| saved["context"] == "beta")
    })
    .await;
    let restored = GpuiOptions::new(None, None, 100).with_preferences(Some(preferences));
    assert_eq!(restored.config_path(), Some(second.as_path()));
    assert_eq!(restored.context(), Some("beta"));
    cx.run_until_parked();
    std::fs::remove_dir_all(directory).unwrap();
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
        open_node_tab(window, cx, super::nodes::NodeTab::Services);
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
        open_node_tab(window, cx, super::nodes::NodeTab::Services);
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
        cx.background_executor()
            .advance_clock(std::time::Duration::from_millis(300));
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
        open_node_tab(window, cx, super::nodes::NodeTab::Logs);
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
        open_node_tab(window, cx, super::nodes::NodeTab::Processes);
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
        open_node_tab(window, cx, super::nodes::NodeTab::Processes);
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
        assert_eq!(view.read(cx).page, Page::Overview);
        for (page, kind) in [
            (Page::Nodes, None),
            (Page::Resources, Some("namespaces")),
            (Page::Resources, Some("events")),
            (Page::Health, None),
            (Page::Resources, Some("pods")),
            (Page::Overview, None),
        ] {
            window.press("ctrl-tab", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).page, page);
            if let Some(kind) = kind {
                assert_eq!(view.read(cx).resource_kind.key(), kind);
            }
        }
        for key in ["secondary-6", "secondary-7", "secondary-8", "secondary-9"] {
            window.press(key, cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).page, Page::Overview);
        }
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
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).page, Page::Overview);
        view.update(cx, |view, cx| view.open_builtin("pods", window, cx));
    })
    .unwrap();
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
        assert_eq!(pilot.context_display.detail.as_ref(), "Couldn't connect");
        contexts(window, cx);
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
        contexts(window, cx);
        window.click(("context", 0usize), cx);
        window.render_frame(cx);
        assert_eq!(kubernetes_status(window), "Connecting to lab…");
    })
    .unwrap();
    wait_until(cx, handle, "lab to refuse again", |window, _| {
        kubernetes_status(window).starts_with("Couldn't connect to lab: ")
    });

    cx.update_window(handle, |_, window, cx| {
        contexts(window, cx);
        window.click(("context", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("staging"));
        contexts(window, cx);
        assert_eq!(window.find(("context", 1usize)).selected(), Some(true));
        contexts(window, cx);
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
        for page in Page::ALL.into_iter().filter(|page| {
            !matches!(
                page,
                Page::Overview
                    | Page::Resources
                    | Page::Health
                    | Page::Nodes
                    | Page::Monitoring
                    | Page::Observability
            )
        }) {
            view.update(cx, |view, cx| view.navigate(page, window, cx));
            window.render_frame(cx);
            assert_eq!(view.read(cx).page, page);
            assert!(window.find("needs-talosconfig").visible(), "{page:?}");
        }
        // Monitoring reads the cluster's Prometheus, which needs no Talos.
        view.update(cx, |view, cx| view.navigate(Page::Monitoring, window, cx));
        window.render_frame(cx);
        assert!(window.find("monitoring-page").visible());
        assert!(window.try_find("needs-talosconfig").is_none());
        view.update(cx, |view, cx| view.navigate(Page::Etcd, window, cx));
        window.render_frame(cx);
        window.click("browse-kubernetes", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Resources);
        assert!(window.try_find("needs-talosconfig").is_none());

        view.update(cx, |view, cx| view.navigate(Page::Etcd, window, cx));
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
fn kubernetes_only_control_plane_offers_a_talosconfig(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = kubernetes_only(cx, kubeconfig_file("rail"), None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        area(window, cx, "nav-control-plane");
        // The area shows why it is empty and keeps the page.
        assert_eq!(view.read(cx).page, Page::Overview);
        assert!(window.try_find("nav-etcd").is_none());
        window.click("add-talosconfig", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("talosconfig-path").visible());
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
        contexts(window, cx);
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
        view.update(cx, |view, cx| view.open_builtin("pods", window, cx));
        window.render_frame(cx);
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
        assert!(window.find("context-switcher").visible());
        assert!(window.try_find("kubernetes-status").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn command_k_goes_to_any_kind_from_any_page(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    step(cx, &|window, cx| {
        window.press("secondary-2", cx);
        window.press("secondary-k", cx);
    });
    step(cx, &|window, cx| {
        assert!(window.has_active_dialog(cx));
        // Kinds match by name or kubectl key.
        window.input("statefulsets", cx);
    });
    step(cx, &|window, cx| window.press("enter", cx));
    step(cx, &|window, cx| {
        assert!(!window.has_active_dialog(cx));
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(view.read(cx).resource_kind.key(), "statefulsets.apps");
        assert_eq!(window.find("page-title").label(), Some("StatefulSets"));
        assert_eq!(window.find("resource-body").focused(), Some(true));
        // Escape leaves the palette and hands the keyboard back.
        window.press("secondary-k", cx);
    });
    step(cx, &|window, cx| {
        assert!(window.has_active_dialog(cx));
        window.press("escape", cx);
    });
    step(cx, &|window, cx| {
        assert!(!window.has_active_dialog(cx));
        assert_eq!(window.find("resource-body").focused(), Some(true));
        assert_eq!(view.read(cx).resource_kind.key(), "statefulsets.apps");
        // Custom kinds join once discovered.
        let kind = crate::resources::example::kind("certificates.cert-manager.io").unwrap();
        view.update(cx, |view, cx| view.open_kind(kind, window, cx));
    });
    step(cx, &|window, cx| {
        window.press("secondary-1", cx);
        window.press("secondary-k", cx);
    });
    step(cx, &|window, cx| window.input("certificates.cert", cx));
    step(cx, &|window, cx| window.press("enter", cx));
    step(cx, &|_, cx| {
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(
            view.read(cx).resource_kind.key(),
            "certificates.cert-manager.io"
        );
    });
}

#[gpui_kit::test]
fn the_smallest_window_leaves_room_for_a_pods_log_lines(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    step(cx, &|window, cx| {
        view.update(cx, |view, cx| view.open_builtin("pods", window, cx));
    });
    step(cx, &|window, cx| {
        window.press("down", cx);
        window.press("enter", cx);
    });
    step(cx, &|window, cx| {
        // Overview, YAML, Events, then Logs.
        for _ in 0..3 {
            window.press("secondary-}", cx);
        }
    });
    step(cx, &|window, _| {
        let list = window.find("resource-list").bounds();
        let pane = window.find("resource-detail").bounds();
        let lines = window
            .within("resource-detail")
            .find("logs-viewport")
            .bounds();
        // The pane stacks under the list and takes the larger share.
        assert!(pane.top() >= list.bottom(), "{list:?} {pane:?}");
        assert!(pane.size.height > list.size.height, "{list:?} {pane:?}");
        assert!(pane.bottom() <= px(560.), "{pane:?}");
        // At least three log lines show.
        assert!(lines.size.height >= px(54.), "{lines:?}");
        assert!(lines.bottom() <= pane.bottom(), "{lines:?} {pane:?}");
    });
}

#[gpui_kit::test]
fn text_size_steps_by_shortcut_and_settings_and_survives_appearance(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let rem = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        window.render_frame(cx);
        window.rem_size()
    };
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(rem(window, cx), px(13.));
        window.press("secondary-=", cx);
        assert_eq!(rem(window, cx), px(14.));
        for _ in 0..3 {
            window.press("secondary-=", cx);
        }
        // The largest step holds.
        assert_eq!(rem(window, cx), px(20.));
        assert!((Theme::global(cx).mono_font_size - px(20. * 12.5 / 13.)).abs() < px(0.001));
        window.press("secondary-0", cx);
        assert_eq!(rem(window, cx), px(13.));
        window.press("secondary--", cx);
        window.press("secondary--", cx);
        assert_eq!(rem(window, cx), px(12.));

        window.click("settings", cx);
        window.render_frame(cx);
        window.click(("text-size", 18usize), cx);
        assert_eq!(rem(window, cx), px(18.));
        // Kit reloads the theme's font size with its colors.
        view.update(cx, |view, cx| {
            view.set_appearance(super::Appearance::Dark, window, cx)
        });
        assert_eq!(Theme::global(cx).mode, ThemeMode::Dark);
        assert_eq!(rem(window, cx), px(18.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_shell_scales_with_the_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        let measure = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            window.render_frame(cx);
            let sidebar = window.find("nav-rail").bounds().size.width;
            let nav = window.find("nav-overview").bounds().size.height;
            (sidebar, nav)
        };
        let (sidebar, nav) = measure(window, cx);
        for _ in 0..4 {
            window.press("secondary-=", cx);
        }
        let (larger_sidebar, larger_nav) = measure(window, cx);
        let ratio = 20. / 13.;
        assert!(
            (larger_sidebar / sidebar - ratio).abs() < 0.01,
            "{larger_sidebar:?}"
        );
        assert!(
            (larger_nav / nav - ratio).abs() < 0.1,
            "{nav:?} → {larger_nav:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_resources_toolbar_wraps_rather_than_clip_at_a_large_size(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        area(window, cx, "nav-k8s-group-workloads");
        let controls = ["resource-namespace", "resource-filter", "resource-refresh"];
        let fit = |window: &mut gpui_kit::Window| {
            for id in controls {
                let bounds = window.find(id).bounds();
                assert!(
                    bounds.right() <= px(760.),
                    "{id} ends at {:?}",
                    bounds.right()
                );
                assert!(bounds.size.width > px(0.), "{id}");
            }
        };
        // Fog wraps its status filters and controls as the space changes.
        fit(window);
        for _ in 0..4 {
            window.press("secondary-=", cx);
        }
        window.render_frame(cx);
        fit(window);
        let filter = window.find("resource-filter").bounds();
        let refresh = window.find("resource-refresh").bounds();
        assert!(
            filter.top() != refresh.top(),
            "narrow controls should wrap: {filter:?} {refresh:?}"
        );
        assert!(
            window.find("resource-list").bounds().size.height >= crate::ui::dp_px(68., window),
            "The toolbar and legend must leave at least two rows of list space"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_resources_page_scales_with_the_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        area(window, cx, "nav-k8s-group-workloads");
        let namespace = window.find("resource-namespace").bounds().size.width;
        assert_eq!(namespace, px(132.));
        window.press("secondary--", cx);
        window.render_frame(cx);
        let smaller = window.find("resource-namespace").bounds().size.width;
        assert!(
            (smaller / namespace - 12. / 13.).abs() < 0.01,
            "{smaller:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn another_connection_or_closing_the_window_asks_to_end_a_running_shell(cx: &mut TestAppContext) {
    use crate::resources::{example, live, shell};
    let (_runtime, handle, view) = fixture(cx, 1280., 800.);
    let context = cx.read(|cx| view.read(cx).applied.context.clone().unwrap());
    let (_, rows) = example::read(&context, "pods", None, live::now()).unwrap();
    let pod = rows
        .iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity
        .clone();
    let row: SharedString =
        format!("resource-row:{}/{}/{}", pod.namespace, pod.name, pod.uid).into();
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    step(cx, &|window, cx| {
        let kind = example::kind("pods").unwrap();
        view.update(cx, |view, cx| view.open_kind(kind, window, cx));
    });
    // A healthy pod is folded under the problems; All lists it.
    step(cx, &|window, cx| window.click("resource-view-all", cx));
    step(cx, &|window, cx| window.click(row.clone(), cx));
    step(cx, &|window, cx| window.click("detail-tab-shell", cx));
    step(cx, &|window, cx| window.click("pod-shell-start", cx));
    let running = |cx: &mut TestAppContext| cx.read(shell::running_anywhere);
    assert_eq!(running(cx).as_deref(), Some(pod.name.as_str()));

    // Option-Down in the terminal is the shell's; from the list, another
    // connection asks, and Cancel keeps this one and the shell.
    step(cx, &|window, cx| window.press("alt-down", cx));
    assert!(!cx.has_pending_prompt());
    step(cx, &|window, cx| window.press("secondary-escape", cx));
    step(cx, &|window, cx| window.press("alt-down", cx));
    let (message, detail) = cx.pending_prompt().unwrap();
    assert_eq!(message, format!("End the shell in {}?", pod.name));
    // Closing the connection alone would leave the shell running in the
    // pod, so ending it sends keys first, and says what they can't stop.
    assert!(detail.contains("Control-C, then Control-D"), "{detail}");
    assert!(detail.contains("keeps running in the pod"), "{detail}");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| view.read(cx).applied.context.clone()),
        Some(context.clone())
    );
    assert!(running(cx).is_some());

    // Closing the window asks too, and closes only once the user agrees.
    let closed = Rc::new(Cell::new(false));
    for (answer, closes) in [("Cancel", false), ("End the shell", true)] {
        let flag = closed.clone();
        let close = cx
            .update_window(handle, |_, window, cx| {
                shell::may_close(window, cx, move |_, _| flag.set(true))
            })
            .unwrap();
        assert!(!close);
        cx.simulate_prompt_answer(answer);
        cx.run_until_parked();
        assert_eq!(closed.get(), closes);
    }
    // Agreeing ended the shell before the window went.
    assert_eq!(running(cx), None);

    // Agreeing to another connection ends the shell and moves on.
    step(cx, &|window, cx| window.click("pod-shell-start", cx));
    assert!(running(cx).is_some());
    step(cx, &|window, cx| window.press("secondary-escape", cx));
    step(cx, &|window, cx| window.press("alt-down", cx));
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    assert_ne!(
        cx.read(|cx| view.read(cx).applied.context.clone()),
        Some(context)
    );
    assert_eq!(running(cx), None);
    // With no shell running, nothing asks.
    let close = cx
        .update_window(handle, |_, window, cx| {
            shell::may_close(window, cx, |_, _| {})
        })
        .unwrap();
    assert!(close && !cx.has_pending_prompt());
}

#[gpui_kit::test]
fn refused_summary_events_leave_health_and_other_parts_loaded(cx: &mut TestAppContext) {
    use freshkube_core::kubernetes_summary::Part;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            let mut summary = (**view.kubernetes_summary.data().unwrap()).clone();
            summary.events = Part::Refused("Can't list events: forbidden".into());
            let health = crate::screens::WorkloadData::from_outcome(&summary.workloads);
            let request = view.kubernetes_summary.begin(view.applied.clone());
            view.kubernetes_summary
                .apply(&request, Ok(std::sync::Arc::new(summary)));
            view.summary_health = Some(health.clone());
            view.deliver_workloads(health, cx);
            cx.notify();
        });
        window.render_frame(cx);
        area(window, cx, "nav-k8s-group-workloads");
        window.click("nav-health", cx);
        window.find("workload-list");
        let summary = view.read(cx).kubernetes_summary.data().unwrap();
        assert!(summary.pods.loaded().is_some());
        assert!(summary.nodes.loaded().is_some());
        assert_eq!(summary.events.error(), Some("Can't list events: forbidden"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn health_refreshes_the_shared_summary_and_old_context_answers_are_ignored(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let previous = cx.update(|cx| view.read(cx).kubernetes_summary.data().unwrap().clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        area(window, cx, "nav-k8s-group-workloads");
        window.click("nav-health", cx);
        window.click("screen-refresh", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(!std::sync::Arc::ptr_eq(
            &previous,
            view.read(cx).kubernetes_summary.data().unwrap()
        ));
        view.update(cx, |view, cx| {
            let request = view.kubernetes_summary.begin(view.applied.clone());
            view.select_context("staging-eu".into(), window, cx);
            assert!(
                !view
                    .kubernetes_summary
                    .apply(&request, Ok(previous.clone()))
            );
            assert_eq!(view.applied.context.as_deref(), Some("staging-eu"));
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn long_context_names_keep_both_ends_and_wrap_in_the_popover(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let full = "talos-production-frankfurt-equinix-fr5-baremetal-b7";
    for text_size in [14., 20.] {
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(text_size, cx);
            view.update(cx, |view, cx| view.select_context(full.into(), window, cx));
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let short = window
                .find("context-short-name")
                .label()
                .unwrap()
                .to_owned();
            assert!(short.contains('…'), "{short}");
            assert!(short.starts_with("talos-"), "{short}");
            assert!(short.ends_with("-b7"), "{short}");
            assert_eq!(window.find("context-switcher").label(), Some(full));
            contexts(window, cx);
            let row = window.find(("context", 3usize));
            assert_eq!(row.label(), Some(full));
            assert!(row.bounds().size.height >= crate::ui::dp_px(32., window));
            window.press("escape", cx);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn cluster_services_route_actions_to_the_retained_node_pane(cx: &mut TestAppContext) {
    // Logs starts the synthetic producer, which wakes GPUI from a Tokio
    // thread; let the scheduler park for those external wakes.
    cx.executor().allow_parking();
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let row = "system-service-talos-wk-fra1-02-kubelet";
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("secondary-7", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::SystemServices);
        assert!(window.find(row).visible());
        assert!(window.try_find("restart-service").is_none());
        window.within(row).click("open", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Nodes);
        assert_eq!(view.read(cx).selected_node.as_deref(), Some(DEGRADED_NODE));
        assert_eq!(view.read(cx).selected_service.as_deref(), Some("kubelet"));
        assert_eq!(
            view.read(cx).node_workspace.tab,
            super::nodes::NodeTab::Services
        );
        window.press("secondary-7", cx);
        window.render_frame(cx);
        window.within(row).click("logs", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).node_workspace.tab,
            super::nodes::NodeTab::Logs
        );
        assert_eq!(
            window.find("logs-collection").label(),
            Some("Stop collecting")
        );
        assert!(window.find("logs-status").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn fog_observability_navigation_range_and_sidebar_shortcut(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("section-observability", cx);
        assert_eq!(pilot.read(cx).page, Page::Observability);
        window.render_frame(cx);
        assert!(window.find("refresh").visible());
        window.click("nav-collapse", cx);
        assert!(pilot.read(cx).column_collapsed(window));
        window.click("nav-obs-traces", cx);
        assert_eq!(
            pilot.read(cx).observability.read(cx).destination(),
            crate::observability::Destination::Traces
        );
        window.click("obs-time", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert_eq!(pilot.read(cx).observability.read(cx).hours(), 24);
        window.press("secondary-b", cx);
        assert!(!pilot.read(cx).column_collapsed(window));
    })
    .unwrap();
    // An expanded preference still yields to a narrow window, then returns.
    cx.simulate_window_resize(handle, size(px(760.), px(560.)));
    cx.update_window(handle, |_, window, cx| {
        assert!(pilot.read(cx).column_collapsed(window));
    })
    .unwrap();
    cx.simulate_window_resize(handle, size(px(1280.), px(880.)));
    cx.update_window(handle, |_, window, cx| {
        assert!(!pilot.read(cx).column_collapsed(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn collapsed_observability_column_scrolls_its_active_item_into_view(cx: &mut TestAppContext) {
    use crate::observability::Destination;
    // At 20 px text a 560 high window can't show every destination.
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| {
            pilot.navigate(Page::Observability, window, cx)
        });
        window.render_frame(cx);
        assert!(pilot.read(cx).column_collapsed(window));
    })
    .unwrap();
    for destination in [
        Destination::Deployments,
        Destination::Applications,
        Destination::Profiling,
    ] {
        cx.update_window(handle, |_, window, cx| {
            let page = pilot.read(cx).observability.clone();
            page.update(cx, |page, cx| page.open(destination, cx));
            // The reveal is asked for while drawing and applied by the next layout.
            window.render_frame(cx);
            window.render_frame(cx);
            let list = window.find("obs-navigation-scroll").bounds();
            let item = window
                .find(format!("nav-obs-{}", destination.slug()))
                .bounds();
            assert!(
                item.top() >= list.top() - px(0.5) && item.bottom() <= list.bottom() + px(0.5),
                "{destination:?}: {item:?} outside {list:?}"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn fog_narrow_sidebar_can_be_expanded_manually(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.navigate(Page::Observability, window, cx)
        });
        window.render_frame(cx);
        assert!(pilot.read(cx).column_collapsed(window));
        window.click("nav-collapse", cx);
        assert!(!pilot.read(cx).column_collapsed(window));
        window.click("nav-collapse", cx);
        assert!(pilot.read(cx).column_collapsed(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn coroot_links_resolve_real_subjects_and_reject_old_providers(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    let (pod, access) = cx.read(|cx| {
        let view = view.read(cx);
        let source = view.kube_source().unwrap();
        let pod = crate::resources::example::read(
            &source.context,
            "pods",
            None,
            crate::resources::live::now(),
        )
        .unwrap()
        .1
        .into_iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity;
        (pod, source.id)
    });
    let mut old = None;
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.navigate(Page::Observability, window, cx);
            old = Some(view.observability.update(cx, |page, cx| {
                page.fixture_link(access.clone(), &pod.namespace, &pod.name, cx)
            }));
        });
        window.render_frame(cx);
        window.click("obs-open-object", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(
            view.read(cx).resources.read(cx).detail_identity(cx),
            Some(&pod)
        );
        view.update(cx, |view, cx| {
            view.navigate(Page::Observability, window, cx);
            view.observability.update(cx, |page, cx| {
                page.fixture_link(access.clone(), &pod.namespace, &pod.name, cx);
                cx.emit(old.take().unwrap());
            });
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(view.read(cx).page, Page::Observability));
}

#[gpui_kit::test]
fn coroot_links_honor_shell_confirmation_and_recheck_access(cx: &mut TestAppContext) {
    use crate::resources::{Tab, example, live, model::ObjectRef, shell};
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    let (pods, access) = cx.read(|cx| {
        let source = view.read(cx).kube_source().unwrap();
        let pods: Vec<_> = example::read(&source.context, "pods", None, live::now())
            .unwrap()
            .1
            .into_iter()
            .filter(|row| row.cells[2] == "Running")
            .take(2)
            .map(|row| row.identity)
            .collect();
        (pods, source.id)
    });
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    step(cx, &|window, cx| {
        view.update(cx, |view, cx| {
            view.open_object(
                example::kind("pods").unwrap(),
                ObjectRef {
                    namespace: pods[0].namespace.clone(),
                    name: pods[0].name.clone(),
                    uid: pods[0].uid.clone(),
                },
                Tab::Shell,
                window,
                cx,
            );
        })
    });
    step(cx, &|window, cx| window.click("pod-shell-start", cx));
    assert_eq!(
        cx.read(shell::running_anywhere).as_deref(),
        Some(pods[0].name.as_str())
    );
    step(cx, &|window, cx| {
        view.update(cx, |view, cx| {
            view.navigate(Page::Observability, window, cx);
            view.observability.update(cx, |page, cx| {
                page.fixture_link(access.clone(), &pods[1].namespace, &pods[1].name, cx);
            });
        })
    });
    step(cx, &|window, cx| window.click("obs-open-object", cx));
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(view.read(cx).page, Page::Observability);
        assert_eq!(
            shell::running_anywhere(cx).as_deref(),
            Some(pods[0].name.as_str())
        );
    });
    step(cx, &|window, cx| window.click("obs-open-object", cx));
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(shell::running_anywhere(cx), None);
        assert_eq!(view.read(cx).page, Page::Resources);
        assert_eq!(
            view.read(cx).resources.read(cx).detail_identity(cx),
            Some(&pods[1])
        );
    });
    step(cx, &|window, cx| window.click("detail-tab-shell", cx));
    step(cx, &|window, cx| window.click("pod-shell-start", cx));
    step(cx, &|window, cx| {
        view.update(cx, |view, cx| {
            view.navigate(Page::Observability, window, cx);
            view.observability.update(cx, |page, cx| {
                page.fixture_link(access.clone(), &pods[0].namespace, &pods[0].name, cx);
            });
        })
    });
    step(cx, &|window, cx| window.click("obs-open-object", cx));
    assert!(cx.has_pending_prompt());
    // Isolate the access check from normal epoch/job cancellation: a
    // confirmation captured the old access even if the epoch is unchanged.
    cx.update(|cx| {
        view.update(cx, |view, _| {
            view.applied.context = Some("staging-eu".into())
        })
    });
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    cx.read(|cx| {
        // An obsolete confirmation must have no side effects, including ending
        // the shell captured by a different access identity.
        assert_eq!(
            shell::running_anywhere(cx).as_deref(),
            Some(pods[1].name.as_str())
        );
        assert_eq!(view.read(cx).page, Page::Observability);
        assert_eq!(
            view.read(cx).resources.read(cx).detail_identity(cx),
            Some(&pods[1])
        );
    });
}

#[gpui_kit::test]
async fn object_metadata_completion_rechecks_access_even_without_an_epoch_change(
    cx: &mut TestAppContext,
) {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let server_runtime = tokio::runtime::Runtime::new().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let received = Arc::new(AtomicBool::new(false));
    let release = Arc::new(tokio::sync::Notify::new());
    let (seen, gate) = (received.clone(), release.clone());
    let server = server_runtime.spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        while let Ok((mut socket, _)) = listener.accept().await {
            let (seen, gate) = (seen.clone(), gate.clone());
            tokio::spawn(async move {
                let mut request = vec![0u8; 16384];
                let n = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..n]);
                let (status, body) = if request.starts_with("GET /api/v1/namespaces/prod/pods/later ") {
                    assert!(request.contains("PartialObjectMetadata"));
                    seen.store(true, Ordering::SeqCst);
                    gate.notified().await;
                    (200, serde_json::json!({"apiVersion":"meta.k8s.io/v1","kind":"PartialObjectMetadata","metadata":{"namespace":"prod","name":"later","uid":"old-access-uid"}}))
                } else if request.starts_with("GET /version ") {
                    (200, serde_json::json!({"major":"1","minor":"32","gitVersion":"v1.32.0","gitCommit":"fixture","gitTreeState":"clean","buildDate":"2026-01-01T00:00:00Z","goVersion":"go1.24","compiler":"gc","platform":"test"}))
                } else {
                    (404, serde_json::json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"NotFound","code":404,"message":"No fixture for this endpoint"}))
                };
                let body = body.to_string();
                let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    std::fs::write(
        &config,
        KUBECONFIG.replace("https://127.0.0.1:1", &format!("http://{address}")),
    )
    .unwrap();
    let (_runtime, handle, view) = kubernetes_only(cx, config, Some("lab"));
    let observed = view.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        observed.read(cx).kube_source().is_some()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.open_object(
                crate::resources::example::kind("pods").unwrap(),
                crate::resources::model::ObjectRef {
                    namespace: "prod".into(),
                    name: "later".into(),
                    uid: String::new(),
                },
                crate::resources::Tab::Overview,
                window,
                cx,
            );
        })
    })
    .unwrap();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, _| {
        received.load(Ordering::SeqCst)
    })
    .await;
    let completion = cx.update(|cx| {
        view.update(cx, |view, _| {
            // Simulate a replacement access independently of the existing
            // epoch/sequence guards, leaving the delayed worker alive.
            view.fixture = true;
            view.applied.context = Some("replacement".into());
            view.object_open_task.take().unwrap()
        })
    });
    release.notify_one();
    completion.await;
    cx.read(|cx| {
        assert!(
            view.read(cx)
                .resources
                .read(cx)
                .detail_identity(cx)
                .is_none()
        )
    });
    server.abort();
}

/// The table page every other table page is measured against.
const PODS: super::layout_check::TablePage = super::layout_check::TablePage {
    page: "resources-page",
    title: "resource-title",
    title_text: "Pods",
    table: "resource-table-scroll",
    list: "resource-list",
};

#[gpui_kit::test]
fn pods_is_a_table_page_at_every_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            view.update(cx, |view, cx| view.open_builtin("pods", window, cx));
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            // Example pods include a failing one, so Problems shows groups.
            let layout = super::layout_check::assert_table_page(window, cx, &PODS);
            assert!(layout.group.is_some(), "{layout:#?}");
        })
        .unwrap();
    }
}

/// A page whose table isn't the whole page (a dashboard's table panel, a list
/// without column captions) checks its frame and its table apart.
#[gpui_kit::test]
fn the_frame_and_table_checks_measure_apart(cx: &mut TestAppContext) {
    use super::layout_check::{PageFrame, Table, assert_edge_frame, assert_table};
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.open_builtin("pods", window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let frame = PageFrame {
            page: PODS.page,
            title: PODS.title,
            title_text: PODS.title_text,
            content: PODS.table,
        };
        assert_edge_frame(window, cx, &frame);
        let rows = assert_table(
            window,
            cx,
            &Table {
                table: Some(PODS.table),
                list: PODS.list,
            },
        );
        assert!(rows.header.is_some(), "{rows:#?}");
        // A list without column captions skips the header.
        let rows = assert_table(
            window,
            cx,
            &Table {
                table: None,
                list: PODS.list,
            },
        );
        assert!(rows.header.is_none(), "{rows:#?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn search_everything_keeps_one_width_on_every_page(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    let mut widths = Vec::new();
    for page in [Page::Resources, Page::Observability, Page::Overview] {
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.navigate(page, window, cx));
            window.render_frame(cx);
            let width = window.find("search-everything").bounds().size.width;
            widths.push((page, width / crate::ui::dp_px(1., window)));
        })
        .unwrap();
    }
    for (page, width) in &widths {
        assert!(
            (*width - 240.).abs() < 0.5,
            "{page:?} shows Search everything as a {width} dp field, not the full one"
        );
    }
}

#[gpui_kit::test]
fn pods_toolbar_fits_one_row_from_where_the_header_stops_stacking(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 2400., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.open_builtin("pods", window, cx));
        window.render_frame(cx);
        let bounds = |id: String| window.find(id).bounds();
        let title = bounds("resource-title".into());
        // The example pods show the failing and healthy chips.
        let chips = bounds("resource-tally-healthy".into());
        let first = bounds("resource-slot-0".into());
        let last = (1..)
            .map_while(|ix| window.try_find(format!("resource-slot-{ix}")))
            .last()
            .map_or(first, |slot| slot.bounds());
        let gap = crate::ui::dp_px(8., window);
        let one_row = (chips.right() - title.left()) + gap + (last.right() - first.left());
        let one_row = one_row / crate::ui::dp_px(1., window);
        // The other two chips, not ready and waiting, take about 150 more.
        assert!(
            one_row + 150. <= freshkube_ui::page::HEADER_NARROW,
            "Pods' toolbar needs {one_row} dp on one row"
        );
        assert!(
            freshkube_ui::page::HEADER_NARROW - one_row <= 200.,
            "HEADER_NARROW stacks a header that fits: Pods' needs {one_row} dp"
        );
    })
    .unwrap();
}
