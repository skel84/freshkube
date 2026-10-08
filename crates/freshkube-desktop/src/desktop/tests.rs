use super::{Area, ColumnReveal, GpuiOptions, NodeView, Page, Pilot, probe};
use crate::logs::TalosPanel;
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext,
    component::{ActiveTheme, Root, Theme, ThemeMode},
    point, px, size,
};
use std::cell::Cell;
use std::rc::Rc;

pub(crate) fn fixture(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    mount(cx, GpuiOptions::fixture(), width, height)
}

pub(super) fn mount(
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

/// Starts a shell in `pod`'s first running container, in a dock tab, as a
/// pick from the pane's Shell menu does.
pub(crate) fn start_shell(
    handle: AnyWindowHandle,
    pilot: &Entity<Pilot>,
    pod: &crate::resources::model::ResourceIdentity,
    cx: &mut TestAppContext,
) {
    use crate::resources::{ResourceLink, ShellRequest, detail::DetailTarget, example, live};
    let containers = example::document(pod, live::now())
        .unwrap()
        .overview
        .pod
        .unwrap();
    let container = crate::resources::shell::choices(&containers)
        .into_iter()
        .find(|choice| choice.enabled)
        .unwrap()
        .name;
    let request = ShellRequest {
        target: DetailTarget {
            identity: pod.clone(),
            kind: freshkube_core::resources::builtin("pods").unwrap(),
        },
        container,
        containers: None,
    };
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.resource_link(ResourceLink::Shell(request), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
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
    if nodes.healthy_collapsed() && nodes.view == NodeView::Table {
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

/// Opens a built-in kind on the Resources page by its kubectl key, as the
/// rail's column does.
pub(crate) fn open_kind(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, key: &str) {
    let view = root_pilot(window, cx);
    view.update(cx, |view, cx| view.open_builtin(key, window, cx));
}

/// Opens the node's Services tab and expands the inspector, so a selected
/// service's details sit beside the list rather than under it.
fn open_services_with_room(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    open_node_tab(window, cx, super::nodes::NodeTab::Services);
    window.render_frame(cx);
    if !root_pilot(window, cx).read(cx).node_workspace.expanded {
        window.click("node-expand", cx);
        window.render_frame(cx);
    }
}

/// Old page coverage now follows the actual node-pane tab controls.
pub(crate) fn open_node_tab(
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
        // The selection scrolled the stacked details into view; the
        // filter is back at the page's top.
        let scroll = view.read(cx).service_scroll.clone();
        scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
        window.render_frame(cx);
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

/// Stacked under the list, a newly selected service's details scroll into
/// view, by pointer and by keyboard; beside the list, nothing scrolls, and
/// a refresh never reveals again (#238).
#[gpui_kit::test]
fn a_selected_service_brings_its_stacked_details_into_view(cx: &mut TestAppContext) {
    // The node's inspector stacks them beside the table and in a narrow
    // window; expanded in a wide one, it sets them side by side, and a
    // short window lets that page scroll too.
    for (case, width, height, expanded, stacked) in [
        ("beside the table", 1280., 820., false, true),
        ("narrow window", 760., 820., false, true),
        ("expanded", 1280., 560., true, false),
    ] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        let scroll = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                open_node_tab(window, cx, super::nodes::NodeTab::Services);
                if expanded {
                    view.update(cx, |view, cx| view.toggle_node_expanded(cx));
                }
                let scroll = view.read(cx).service_scroll.clone();
                // Draws until no view asks for another frame, so the count
                // after an action is the reveal's alone.
                let settle = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
                    for _ in 0..10 {
                        window.render_frame(cx);
                        if window.simulate_next_frame(cx) == 0 {
                            return;
                        }
                    }
                    panic!("{case}: frames never settled");
                };
                let check = |window: &mut gpui_kit::Window,
                             cx: &mut gpui_kit::App,
                             step: &str,
                             before: gpui_kit::Pixels| {
                    // The frame that drew the selection laid the details out and
                    // moved the page; it asks for the next frame, which draws
                    // them there.
                    let asked = window.simulate_next_frame(cx);
                    assert_eq!(
                        asked > 0,
                        stacked,
                        "{case}, {step}: {asked} frames asked for"
                    );
                    window.render_frame(cx);
                    let page = window.find("services-page").bounds();
                    let details = window.find("selected-service").bounds();
                    let list = window.find("services-region").bounds();
                    assert_eq!(
                        details.top() >= list.bottom(),
                        stacked,
                        "{case}: the details {details:?} and the list {list:?}"
                    );
                    if !stacked {
                        assert_eq!(scroll.offset().y, before, "{case}, {step}: scrolled");
                        return;
                    }
                    assert!(
                        details.top() >= page.top() - px(0.5),
                        "{case}, {step}: the details' top {details:?} is above {page:?}"
                    );
                    for id in ["service-logs", "service-restart"] {
                        let action = window.find(id).bounds();
                        assert!(
                            action.bottom() <= page.bottom() + px(0.5),
                            "{case}, {step}: {id} at {action:?} is below {page:?}"
                        );
                    }
                };
                settle(window, cx);
                if !stacked {
                    // Side by side, a page already scrolled stays where it is.
                    let max = scroll.max_offset().y;
                    assert!(max > px(0.), "{case}: the page doesn't scroll");
                    scroll.set_offset(gpui_kit::point(px(0.), -max.min(px(40.))));
                    settle(window, cx);
                }
                let before = scroll.offset().y;
                window.within("services-region").click("apid", cx);
                check(window, cx, "pointer", before);
                // Stacked, back to the top first; then a key moves the selection.
                if stacked {
                    scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
                }
                settle(window, cx);
                let before = scroll.offset().y;
                window.press("down", cx);
                check(window, cx, "keyboard", before);
                assert_eq!(view.read(cx).selected_service.as_deref(), Some("auditd"));
                assert!(view.read(cx).service_focus.is_focused(window));
                // A refresh keeps the selection and reveals nothing.
                scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
                settle(window, cx);
                window.click("refresh-services", cx);
                assert_eq!(window.simulate_next_frame(cx), 0, "{case}: refresh");
                scroll
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).selected_service.as_deref(), Some("auditd"));
            assert_eq!(scroll.offset().y, px(0.), "{case}: the refresh revealed");
        })
        .unwrap();
    }
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
        open_services_with_room(window, cx);
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
fn embedded_services_start_with_their_toolbar(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Expanded, so the toolbar keeps one row.
        open_services_with_room(window, cx);
        window.render_frame(cx);
        // The tab names the screen and the heading the node, so no title
        // or node line sits above the toolbar: its controls are the first
        // row, one inset under the tab strip.
        let content = window.find("node-inspector-content").bounds();
        let inset = crate::ui::dp_px(freshkube_ui::page::PANE_PADDING, window);
        for id in ["service-filter", "health-all", "refresh-services"] {
            let control = window.find(id).bounds();
            let row = (control.top() - content.top()) - inset;
            assert!(
                row >= px(0.) && row <= crate::ui::dp_px(4., window),
                "{id} sits {row:?} below the inset"
            );
        }
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
    // The arrow selected a row; with the drawer closed, only Enter opens
    // its details.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("detail-close").is_none());
        window.press("enter", cx);
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
        assert_eq!(window.find("node-tab-processes").selected(), Some(true));
        assert!(window.find("processes-list").visible());
        // Rows are keyed by PID. Init leads the first sample, which has no
        // CPU deltas to sort by yet.
        assert!(window.find(("process", 1usize)).visible());
        // Navigating focuses the list, so arrow keys work without a click.
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(window.find(("process", 1usize)).selected(), Some(true));
        // A node switch reaches the visible screen as a new target.
        pick_target(window, cx, 4);
        assert_eq!(view.read(cx).selected_node.as_deref(), Some(DEGRADED_NODE));
        window.render_frame(cx);
        assert!(window.find("partial-notice").visible());
        // The retained screen scrolls under the node header in a short pane.
        for _ in 0..10 {
            if window.find("processes-list").visible() {
                break;
            }
            window.scroll(
                "processes-page",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-60.))),
                cx,
            );
            window.render_frame(cx);
        }
        assert!(window.find("processes-list").visible());
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
fn a_short_window_scrolls_the_frame_to_the_node_log_catalog_and_multiline_errors(
    cx: &mut TestAppContext,
) {
    short_window_node_log(cx, None);
}

#[gpui_kit::test]
fn a_short_window_at_the_largest_text_keeps_the_node_log_in_its_body(cx: &mut TestAppContext) {
    short_window_node_log(cx, Some(20.));
}

/// The node log's notices, viewport and status in a 760 × 560 window, at
/// the default text size or `text_size`.
fn short_window_node_log(cx: &mut TestAppContext, text_size: Option<f32>) {
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
        if let Some(text_size) = text_size {
            crate::text_size::set(text_size, cx);
        }
        // The log's width, then its toolbar and the body around it, settle.
        // A frame draws only when one was asked for, as on screen, so the
        // body's height reaches it only through its own redraw.
        window.render_frame(cx);
        let mut asked = 0;
        for _ in 0..8 {
            let frames = window.simulate_next_frame(cx);
            if frames == 0 {
                break;
            }
            asked += frames;
            window.render_frame(cx);
        }
        assert!(asked > 0, "the body never asked to draw again");
        assert_eq!(window.simulate_next_frame(cx), 0, "frames never settled");
        // The frame scrolled to the pane when it opened, at the text size
        // of the time.
        let frame = view.read(cx).node_workspace.page_scroll.clone();
        frame.scroll_to_bottom();
        window.render_frame(cx);
        // 560 px is short at the default text size, so the Logs body
        // scrolls inside the node's inspector. The log keeps its notices
        // above a usable viewport inside its panel, and the status bar's log
        // status stays below the pane.
        let panel = window.find("logs-panel").bounds();
        let viewport = window.find("logs-viewport").bounds();
        let notices = window.find("logs-notices").bounds();
        assert!(viewport.top() >= panel.top());
        assert!(viewport.left() >= panel.left());
        assert!(viewport.right() <= panel.right());
        assert!(viewport.bottom() <= panel.bottom());
        assert!(viewport.size.height >= window.rem_size() * 6.);
        assert!(viewport.right() <= px(760.));
        assert!(notices.size.height >= window.rem_size());
        assert!(notices.bottom() <= viewport.top());
        // At the largest text the toolbar alone outgrows the body's room,
        // and the body scrolls to the rest of it.
        if text_size.is_none() {
            assert!(window.find("logs-collection").visible());
        }
        // Scrolling the body brings the notices into view, then the whole
        // viewport, above the status bar's log status.
        let body = view.read(cx).node_workspace.logs_scroll.clone();
        let content = window.find("node-logs-scroll").bounds();
        let by = (content.bottom() - notices.bottom()).min(px(0.));
        body.set_offset(point(px(0.), body.offset().y + by));
        window.render_frame(cx);
        assert!(window.find("logs-notices").visible());
        body.set_offset(point(px(0.), -body.max_offset().y));
        window.render_frame(cx);
        let viewport = window.find("logs-viewport").bounds();
        let shown = window.find("node-logs-scroll").bounds();
        let status = window.find("logs-status");
        assert!(status.visible());
        assert!(status.bounds().top() >= viewport.bottom());
        assert!(status.bounds().bottom() <= px(560.));
        assert!(viewport.top() >= shown.top(), "{viewport:?} {shown:?}");
        assert!(viewport.bottom() <= shown.bottom(), "{viewport:?} {shown:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_talosconfig_that_fails_to_load_ends_the_services_wait(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    // Never the user's default talosconfig: a missing file.
    let options = GpuiOptions::new(Some(scratch("no-such-talosconfig-services")), None, 100);
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).config_error.is_some()
    })
    .await;
    cx.update(|cx| {
        let pilot = view.read(cx);
        // Neither page waits on an overview that will never be asked.
        let reading = pilot.system_services.read(cx).reading().clone();
        assert!(
            matches!(reading, super::system_services::Reading::Failed(_)),
            "{reading:?}"
        );
        assert!(!pilot.nodes_wait());
    });
}

#[gpui_kit::test]
fn a_cold_nodes_table_draws_its_header_over_the_loading_rows(cx: &mut TestAppContext) {
    // Nothing has answered: neither Talos nor the summary built the rows.
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture().holding_talos(), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-2", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, Page::Nodes);
        assert!(view.read(cx).nodes_wait());
        assert!(window.find("nodes-loading").visible());
        let header: Vec<_> = (0usize..12)
            .filter_map(|ix| window.try_find(("nodes-sort", ix)))
            .filter_map(|cell| cell.label().map(str::to_owned))
            .collect();
        assert!(
            header.iter().any(|label| label == "Name") && header.len() > 4,
            "{header:?}"
        );
    })
    .unwrap();
}

/// Lets the loading motion run, as the platform would: one 16 ms frame.
/// Returns how many next-frame callbacks the frame delivered.
fn motion_frame(cx: &mut TestAppContext, handle: AnyWindowHandle) -> usize {
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(16));
    let asked = cx
        .update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    cx.run_until_parked();
    asked
}

/// The shell's cached parts, by their probes.
const CHROME: [&str; 3] = ["chrome.header", "chrome.rail", "chrome.column"];

fn chrome_counts() -> [usize; 3] {
    CHROME.map(probe::count)
}

/// Runs 60 frames, about a second, and returns how many of them redrew
/// a part of the chrome or `page`. The header's countdown ring and a
/// page's ages tick once a second; nothing else should draw them.
fn frames_redrawing(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &'static str) -> usize {
    let counts = || (chrome_counts(), probe::count(page));
    (0..60)
        .filter(|_| {
            let before = counts();
            assert!(
                motion_frame(cx, handle) > 0,
                "the bars move while the page loads"
            );
            counts() != before
        })
        .count()
}

/// Opens Pods with example data that never answers, so it stays on its
/// loading rows, and lets the motion start.
fn loading_pods(
    cx: &mut TestAppContext,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update(|cx| cx.set_reduce_motion(false));
    cx.update_window(handle, |_, window, cx| {
        let resources = view.read(cx).resources.clone();
        resources.update(cx, |resources, _| resources.hold = true);
        open_kind(window, cx, "pods");
    })
    .unwrap();
    cx.run_until_parked();
    for _ in 0..3 {
        motion_frame(cx, handle);
    }
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("resource-loading").visible());
    })
    .unwrap();
    (runtime, handle, view)
}

/// The loading motion sits beside the page and the cached chrome: its
/// frames redraw the shell's frame, but neither the header, the rail, the
/// column, nor the page and its table.
#[gpui_kit::test]
fn the_loading_motion_redraws_neither_the_chrome_nor_the_page(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = loading_pods(cx);
    let motion = probe::count("table.loading-motion");
    assert!(frames_redrawing(cx, handle, "resources") <= 1);
    assert!(probe::count("table.loading-motion") >= motion + 60);
}

/// The first answer replaces the loading rows, and the motion asks no
/// more frames.
#[gpui_kit::test]
fn the_loading_motion_stops_on_the_first_answer(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = loading_pods(cx);
    cx.update_window(handle, |_, window, cx| {
        let resources = view.read(cx).resources.clone();
        resources.update(cx, |resources, cx| {
            resources.hold = false;
            resources.refresh(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    // At most the frame already asked, which draws the rows.
    motion_frame(cx, handle);
    let motion = probe::count("table.loading-motion");
    for _ in 0..3 {
        assert_eq!(motion_frame(cx, handle), 0, "no frames once Pods answers");
    }
    assert_eq!(probe::count("table.loading-motion"), motion);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("resource-loading").is_none());
    })
    .unwrap();
}

/// Runs a few frames and asserts that the loading motion asks for none
/// and draws no more: whatever replaced the table took it away.
fn assert_motion_stopped(cx: &mut TestAppContext, handle: AnyWindowHandle, why: &str) {
    // At most the frame already asked, which draws what replaced the rows.
    motion_frame(cx, handle);
    let motion = probe::count("table.loading-motion");
    for _ in 0..3 {
        assert_eq!(motion_frame(cx, handle), 0, "no frames after {why}");
    }
    assert_eq!(probe::count("table.loading-motion"), motion, "{why}");
}

/// A refusal or a failure replaces the loading rows with a state, and
/// the motion goes with the table: nothing asks for frames over it.
#[gpui_kit::test]
fn the_loading_motion_stops_when_the_list_is_refused_or_fails(cx: &mut TestAppContext) {
    use crate::resources::model::ReadState;
    let (_runtime, handle, view) = loading_pods(cx);
    let resources = cx.update(|cx| view.read(cx).resources.clone());
    for (state, shown) in [
        (
            ReadState::Refused("pods is forbidden".into()),
            "resource-refused",
        ),
        (ReadState::Failed("Timeout".into()), "resource-failed"),
    ] {
        // Listing again, held, brings the loading rows and their motion back.
        cx.update_window(handle, |_, window, cx| {
            resources.update(cx, |resources, cx| resources.refresh(window, cx))
        })
        .unwrap();
        cx.run_until_parked();
        motion_frame(cx, handle);
        assert!(motion_frame(cx, handle) > 0, "the rows move before {shown}");
        cx.update(|cx| resources.update(cx, |resources, cx| resources.deliver_read(state, cx)));
        cx.run_until_parked();
        assert_motion_stopped(cx, handle, shown);
        cx.update_window(handle, |_, window, cx| {
            assert!(window.find(shown).visible());
            assert!(window.try_find("resource-loading").is_none());
            assert!(view.read(cx).page_loading_motion(cx).is_none());
        })
        .unwrap();
    }
}

/// Nodes waiting on Talos that then fails show their failure, not the
/// table, and the motion stops; the cards view never mounts it.
#[gpui_kit::test]
fn the_nodes_loading_motion_stops_on_a_failure_or_the_cards(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture().holding_talos(), 1280., 880.);
    cx.update(|cx| cx.set_reduce_motion(false));
    cx.update_window(handle, |_, window, cx| window.press("secondary-2", cx))
        .unwrap();
    cx.run_until_parked();
    motion_frame(cx, handle);
    assert!(
        motion_frame(cx, handle) > 0,
        "the rows move while Talos loads"
    );
    cx.update_window(handle, |_, window, cx| window.click("nodes-view-cards", cx))
        .unwrap();
    cx.run_until_parked();
    // The cards wait under their own skeleton, which may ask frames of
    // its own; the table's motion neither draws nor is mounted.
    motion_frame(cx, handle);
    let motion = probe::count("table.loading-motion");
    for _ in 0..3 {
        motion_frame(cx, handle);
    }
    assert_eq!(probe::count("table.loading-motion"), motion, "the cards");
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("nodes-loading").visible());
        assert!(view.read(cx).page_loading_motion(cx).is_none());
        window.click("nodes-view-table", cx);
    })
    .unwrap();
    cx.run_until_parked();
    motion_frame(cx, handle);
    assert!(motion_frame(cx, handle) > 0, "the table's rows move again");
    cx.update(|cx| view.update(cx, |view, cx| view.simulate_failure(cx)));
    cx.run_until_parked();
    assert_motion_stopped(cx, handle, "a failure");
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("nodes-failed").visible());
        assert!(view.read(cx).page_loading_motion(cx).is_none());
    })
    .unwrap();
}

/// Lifecycle's roster pulses through the shell while it waits for its
/// first answer, and the motion stops when the rows or a failure arrive.
#[gpui_kit::test]
fn the_lifecycle_loading_motion_stops_on_its_answer_or_a_failure(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update(|cx| cx.set_reduce_motion(false));
    cx.update_window(handle, |_, window, cx| window.press("secondary-9", cx))
        .unwrap();
    cx.run_until_parked();
    let lifecycle = cx.update(|cx| view.read(cx).lifecycle.clone());
    for (ok, shown) in [(true, "lifecycle-list"), (false, "lifecycle-state")] {
        cx.update(|cx| lifecycle.update(cx, |lifecycle, cx| lifecycle.wait_again(cx)));
        cx.run_until_parked();
        motion_frame(cx, handle);
        assert!(motion_frame(cx, handle) > 0, "the rows move before {shown}");
        cx.update_window(handle, |_, window, cx| {
            assert!(window.find("lifecycle-loading").visible());
            assert!(view.read(cx).page_loading_motion(cx).is_some());
        })
        .unwrap();
        cx.update(|cx| lifecycle.update(cx, |lifecycle, cx| lifecycle.answer(ok, cx)));
        cx.run_until_parked();
        assert_motion_stopped(cx, handle, shown);
        cx.update_window(handle, |_, window, cx| {
            assert!(window.find(shown).visible());
            assert!(window.try_find("lifecycle-loading").is_none());
            assert!(view.read(cx).page_loading_motion(cx).is_none());
        })
        .unwrap();
    }
}

/// Every screen table shows the shared loading rows under its header, edge
/// to edge as its rows will be, while the overview is read; the shell
/// pulses them, and the first answer stops the motion.
#[gpui_kit::test]
fn screen_tables_show_the_shared_loading_rows_until_the_first_answer(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture().holding_talos(), 1280., 880.);
    cx.update(|cx| cx.set_reduce_motion(false));
    let pages = [
        ("secondary-5", "workloads", "Workloads"),
        ("secondary-6", "etcd", "etcd"),
        ("secondary-8", "security", "Security"),
        ("secondary-9", "lifecycle", "Lifecycle"),
    ];
    for (key, prefix, title) in pages {
        cx.update_window(handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
        motion_frame(cx, handle);
        assert!(motion_frame(cx, handle) > 0, "{prefix}'s rows move");
        cx.update_window(handle, |_, window, cx| {
            let loading: &'static str = format!("{prefix}-loading").leak();
            assert!(window.find(loading).visible());
            assert!(window.try_find("screen-waiting").is_none());
            super::layout_check::assert_edge_frame(
                window,
                cx,
                &super::layout_check::PageFrame {
                    page: format!("{prefix}-page").leak(),
                    title: format!("{prefix}-title").leak(),
                    title_text: title,
                    content: loading,
                },
            );
            assert!(view.read(cx).page_loading_motion(cx).is_some());
        })
        .unwrap();
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
    // The example answers through Tokio; draw until Lifecycle's rows are in.
    for _ in 0..50 {
        let waiting = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find("lifecycle-loading").is_some()
            })
            .unwrap();
        if !waiting {
            break;
        }
        motion_frame(cx, handle);
    }
    assert_motion_stopped(cx, handle, "the answer");
    cx.update_window(handle, |_, window, cx| {
        assert!(window.try_find("lifecycle-loading").is_none());
        window.find("lifecycle-list");
        assert!(view.read(cx).page_loading_motion(cx).is_none());
    })
    .unwrap();
}

/// A page that hides takes its motion with it: the shell no longer mounts
/// it, so nothing asks frames, and a shell redraw doesn't draw it.
#[gpui_kit::test]
fn a_hidden_pages_loading_motion_is_never_mounted(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = loading_pods(cx);
    cx.update_window(handle, |_, window, cx| window.press("secondary-1", cx))
        .unwrap();
    cx.run_until_parked();
    motion_frame(cx, handle);
    let motion = probe::count("table.loading-motion");
    for _ in 0..3 {
        assert_eq!(motion_frame(cx, handle), 0, "no frames from a hidden page");
    }
    let shell = probe::count("shell");
    cx.update(|cx| view.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    assert!(probe::count("shell") > shell);
    assert_eq!(probe::count("table.loading-motion"), motion);
    cx.update(|cx| assert!(view.read(cx).page_loading_motion(cx).is_none()));
}

/// Draws the next frame as the platform would, without `render_frame`,
/// which redraws every cached view: a part that missed its notify shows
/// what it drew before.
fn next_frame(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

/// Talos answering for the first time marks the rail through the real
/// path, from the example answer to the overview's cards, and the cached
/// rail draws the mark on the next frame.
#[gpui_kit::test]
fn the_cached_rail_draws_the_marks_the_first_answer_brings(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture().holding_talos(), 1280., 880.);
    next_frame(cx, handle);
    cx.update_window(handle, |_, window, _| {
        assert!(window.within("nav-nodes").try_find("rail-mark").is_none());
    })
    .unwrap();
    let rail = probe::count("chrome.rail");
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.fixture_hold = false;
            view.refresh_now(window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    next_frame(cx, handle);
    assert!(probe::count("chrome.rail") > rail, "the rail draws again");
    cx.update_window(handle, |_, window, _| {
        assert!(window.within("nav-nodes").find("rail-mark").visible());
    })
    .unwrap();
}

/// Applies `change` and returns which parts of the chrome drew again.
fn chrome_redrawn(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    change: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
) -> Vec<&'static str> {
    let before = chrome_counts();
    cx.update_window(handle, |_, window, cx| change(window, cx))
        .unwrap();
    cx.run_until_parked();
    let after = chrome_counts();
    CHROME
        .into_iter()
        .zip(before.into_iter().zip(after))
        .filter(|(_, (before, after))| after > before)
        .map(|(part, _)| part)
        .collect()
}

/// The cached chrome still draws for what it shows while the motion runs
/// beside it: a notify of the shell, the theme and the text size.
#[gpui_kit::test]
fn the_cached_chrome_redraws_for_what_it_shows(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = loading_pods(cx);
    let every = CHROME.to_vec();
    let shell = view.clone();
    assert_eq!(
        chrome_redrawn(cx, handle, |_, cx| shell.update(cx, |_, cx| cx.notify())),
        every
    );
    assert_eq!(
        chrome_redrawn(cx, handle, |window, cx| Theme::change(
            ThemeMode::Dark,
            Some(window),
            cx
        )),
        every
    );
    assert_eq!(
        chrome_redrawn(cx, handle, |_, cx| crate::text_size::set(20., cx)),
        every
    );
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.rem_size(), px(20.));
        assert!(window.find("refresh").visible());
        assert!(window.find("nav-rail").visible());
        assert!(window.find("nav-column").visible());
    })
    .unwrap();
}

/// The column draws again when what it reads from a page changes, and only
/// while it shows that page's area: System services' badge on Control
/// plane, and the namespace a Workloads column picks.
#[gpui_kit::test]
fn the_column_redraws_for_the_pages_it_reads(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    let (services, resources) = cx.update(|cx| {
        let pilot = view.read(cx);
        (pilot.system_services.clone(), pilot.resources.clone())
    });
    let services_notify =
        |_: &mut gpui_kit::Window, cx: &mut gpui_kit::App| services.update(cx, |_, cx| cx.notify());
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-control-plane", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        chrome_redrawn(cx, handle, services_notify),
        ["chrome.column"]
    );
    cx.update_window(handle, |_, window, cx| open_kind(window, cx, "pods"))
        .unwrap();
    cx.run_until_parked();
    assert!(chrome_redrawn(cx, handle, services_notify).is_empty());
    let resources_notify = resources.clone();
    assert!(
        chrome_redrawn(cx, handle, |_, cx| resources_notify
            .update(cx, |_, cx| cx.notify()))
        .is_empty(),
        "a watch event leaves the column"
    );
    assert_eq!(
        chrome_redrawn(cx, handle, |window, cx| resources
            .update(cx, |resources, cx| {
                resources.set_namespace(Some("payments".into()), window, cx)
            })),
        ["chrome.column"]
    );
}

/// Monitoring and Observability each draw the column in their own area,
/// and only there. Custom Resources draws it through the shell, which
/// observes it.
#[gpui_kit::test]
fn the_column_redraws_for_its_areas_page(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    type Notify = Box<dyn Fn(&mut gpui_kit::App)>;
    let (custom, pages): (Notify, Vec<(&str, Notify)>) = cx.update(|cx| {
        let pilot = view.read(cx);
        let (custom, monitoring) = (pilot.custom.clone(), pilot.monitoring.clone());
        let observability = pilot.observability.clone();
        (
            Box::new(move |cx: &mut gpui_kit::App| custom.update(cx, |_, cx| cx.notify()))
                as Notify,
            vec![
                (
                    "nav-monitoring",
                    Box::new(move |cx: &mut gpui_kit::App| {
                        monitoring.update(cx, |_, cx| cx.notify())
                    }) as Notify,
                ),
                (
                    "nav-observability",
                    Box::new(move |cx: &mut gpui_kit::App| {
                        observability.update(cx, |_, cx| cx.notify())
                    }),
                ),
            ],
        )
    });
    for (nav, _) in &pages {
        cx.update_window(handle, |_, window, cx| window.click(*nav, cx))
            .unwrap();
        cx.run_until_parked();
        for (other, notify) in &pages {
            let redrawn = chrome_redrawn(cx, handle, |_, cx| notify(cx));
            if other == nav {
                assert_eq!(redrawn, ["chrome.column"], "{other} on {nav}");
            } else {
                assert!(redrawn.is_empty(), "{other} on {nav}: {redrawn:?}");
            }
        }
    }
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-k8s-group-custom", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(chrome_redrawn(cx, handle, |_, cx| custom(cx)).contains(&"chrome.column"));
}

/// The parts draw with the shell's own controls: a rail button and a
/// header control act on the shell through them.
#[gpui_kit::test]
fn the_cached_chrome_acts_on_the_shell(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-control-plane", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(view.read(cx).area, Area::ControlPlane));
    let dark = cx.update(|cx| cx.theme().mode.is_dark());
    cx.update_window(handle, |_, window, cx| window.click("theme-toggle", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_ne!(cx.theme().mode.is_dark(), dark));
}

/// Talos holds: Nodes waits on its loading rows while the header's
/// Refresh shows the read in flight, and nothing in the chrome repeats.
#[gpui_kit::test]
fn nothing_in_the_chrome_moves_while_talos_loads(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture().holding_talos(), 1280., 880.);
    cx.update(|cx| cx.set_reduce_motion(false));
    cx.update_window(handle, |_, window, cx| window.press("secondary-2", cx))
        .unwrap();
    cx.run_until_parked();
    for _ in 0..3 {
        motion_frame(cx, handle);
    }
    cx.update(|cx| {
        assert!(view.read(cx).loading());
        assert!(view.read(cx).nodes_wait());
    });
    let motion = probe::count("table.loading-motion");
    assert!(frames_redrawing(cx, handle, "nodes") <= 1);
    assert!(probe::count("table.loading-motion") >= motion + 60);
}

#[gpui_kit::test]
fn kubernetes_only_never_waits_for_talos_services(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = kubernetes_only(cx, kubeconfig_file("services"), None);
    wait_until(cx, handle, "the kubeconfig to load", |_, cx| {
        !view.read(cx).contexts.is_empty()
    });
    cx.update(|cx| {
        let reading = view.read(cx).system_services.read(cx).reading().clone();
        assert_eq!(reading, super::system_services::Reading::Answered);
    });
}

#[gpui_kit::test]
async fn browse_picks_a_talosconfig_and_reloads_contexts(cx: &mut TestAppContext) {
    // Configuration loads on Tokio; let GPUI park for its completion.
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let directory = guard.path().to_path_buf();
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
    let guard = tempfile::tempdir().unwrap();
    let directory = guard.path().to_path_buf();
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
}

#[gpui_kit::test]
async fn kubeconfig_file_applies_its_current_context_then_a_chosen_one(cx: &mut TestAppContext) {
    use freshkube_core::cluster_overview::KubeconfigSelection;
    // Inspection runs on Tokio; let GPUI park for its completion.
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let directory = guard.path().to_path_buf();
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
}

#[gpui_kit::test]
async fn connection_failure_offers_a_picker_and_bad_files_do_not_replace_saved_selection(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let directory = guard.path().to_path_buf();
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
}

#[gpui_kit::test]
async fn kubeconfig_without_a_usable_context_is_not_applied(cx: &mut TestAppContext) {
    use freshkube_core::cluster_overview::KubeconfigSelection;
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let directory = guard.path().to_path_buf();
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
        open_services_with_room(window, cx);
        window.render_frame(cx);
        window.within("services-region").click(service, cx);
        window.render_frame(cx);
        window.click("service-restart", cx);
    })
    .unwrap();
    crate::mutation::settle_confirmation(cx, handle);
}

/// The inspector as a node first opens, beside the table: Services stacks its
/// detail under the list, and Restart scrolls into view there.
#[gpui_kit::test]
fn restart_is_reachable_in_the_unexpanded_inspector(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_node_tab(window, cx, super::nodes::NodeTab::Services);
        window.render_frame(cx);
        assert!(!view.read(cx).node_workspace.expanded);
        assert!(window.try_find("nodes-table").is_some());
        window.within("services-region").click("containerd", cx);
        window.render_frame(cx);
        let list = window.find("services-region").bounds();
        let detail = window.find("selected-service").bounds();
        assert!(
            detail.top() >= list.bottom() - px(0.5),
            "the detail {detail:?} isn't under the list {list:?}"
        );
        // The Services page fills the inspector's content and scrolls in it.
        let shown = window.find("node-inspector-content").bounds();
        for _ in 0..60 {
            let restart = window.find("service-restart").bounds();
            if restart.top() >= shown.top() && restart.bottom() <= shown.bottom() {
                assert_eq!(
                    window.find("service-restart").label(),
                    Some("Restart containerd")
                );
                return;
            }
            use gpui_kit::InputEvent as _;
            window.dispatch_event(
                gpui_kit::ScrollWheelEvent {
                    position: shown.center(),
                    delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-40.))),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        }
        panic!("Restart never scrolled into {shown:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn restart_is_offered_only_for_a_selected_service_on_an_answering_node(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_services_with_room(window, cx);
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
    assert!(crate::actions::critical_warning("etcd").is_some());
    assert!(crate::actions::critical_warning("machined").is_some());
    assert!(crate::actions::critical_warning("containerd").is_none());
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
    // Settings lists the files read, and the read runs on Tokio, which
    // `run_until_parked` doesn't wait for.
    let read = view.clone();
    wait_until(cx, handle, "the kubeconfig to be read", move |_, cx| {
        !read.read(cx).config_loading
    });
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

/// Kubernetes-only Health, after `what` failed: the reason, never "No node
/// selected". Returns the reason.
fn kubernetes_only_health_failure(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<Pilot>,
    what: &str,
) -> String {
    wait_until(cx, handle, what, |window, _| {
        window.try_find("k8s-unavailable").is_some()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("screen-no-node").is_none());
        assert!(window.try_find("screen-waiting").is_none());
        assert!(window.try_find("workloads-loading").is_none());
        match &view.read(cx).summary_health {
            Some(Err(reason)) => reason.clone(),
            _ => panic!("{what}: Health has no failure"),
        }
    })
    .unwrap()
}

fn show_health(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<Pilot>) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.navigate(Page::Health, window, cx))
    })
    .unwrap();
}

/// Staging, not the kubeconfig's current lab: Health's Retry connects to
/// the applied context again, never the current one.
#[gpui_kit::test]
fn kubernetes_only_health_says_why_its_context_cant_be_read(cx: &mut TestAppContext) {
    let (_runtime, handle, view) =
        kubernetes_only(cx, kubeconfig_file("health-refused"), Some("staging"));
    show_health(cx, handle, &view);
    let reason = kubernetes_only_health_failure(cx, handle, &view, "staging to refuse");
    cx.update_window(handle, |_, window, cx| {
        let pilot = view.read(cx);
        let kube = pilot.kubernetes_only.as_ref().unwrap();
        assert!(
            matches!(
                &kube.connection,
                super::kubernetes_only::KubeConnection::Failed(error) if *error == reason
            ),
            "{reason}"
        );
        window.click("screen-retry", cx);
    })
    .unwrap();
    // The shell takes Health's event once the click's update ends.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(kubernetes_status(window), "Connecting to staging…");
    })
    .unwrap();
    wait_until(cx, handle, "staging to refuse again", |window, _| {
        kubernetes_status(window).starts_with("Couldn't connect to staging: ")
    });
    kubernetes_only_health_failure(cx, handle, &view, "Health to say so again");
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(view.read(cx).applied.context.as_deref(), Some("staging"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubernetes_only_health_says_the_kubeconfig_is_missing_and_retries(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = kubernetes_only(cx, scratch("no-such-health-kubeconfig"), None);
    show_health(cx, handle, &view);
    let reason = kubernetes_only_health_failure(cx, handle, &view, "the kubeconfig to fail");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).config_error.as_deref(), Some(reason.as_str()));
        // Retry reads the kubeconfig again: Health's table shows its
        // loading rows while it does.
        window.click("screen-retry", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).config_loading);
        assert!(window.try_find("workloads-loading").is_some());
        assert!(window.try_find("k8s-unavailable").is_none());
    })
    .unwrap();
    kubernetes_only_health_failure(cx, handle, &view, "the kubeconfig to fail again");
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
fn the_resources_toolbar_folds_rather_than_clip_at_a_large_size(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        area(window, cx, "nav-k8s-group-workloads");
        let fit = |window: &mut gpui_kit::Window| {
            let ids = ["resource-filter", "resource-namespace", "resource-refresh"];
            let row = window.find("resource-toolbar").bounds();
            let height = crate::ui::dp_px(38., window);
            assert!((row.size.height - height).abs() < px(0.5), "{row:?}");
            for id in ids.into_iter().chain(["resource-more"]) {
                let Some(control) = window.try_find(id) else {
                    continue;
                };
                let bounds = control.bounds();
                assert!(
                    bounds.right() <= px(760.),
                    "{id} ends at {:?}",
                    bounds.right()
                );
                assert!(bounds.size.width > px(0.), "{id}");
                assert!(
                    bounds.top() >= row.top() && bounds.bottom() <= row.bottom(),
                    "{id}"
                );
            }
        };
        settle_header(window, cx);
        fit(window);
        for _ in 0..4 {
            window.press("secondary-=", cx);
        }
        settle_header(window, cx);
        fit(window);
        assert!(
            window.try_find("resource-more").is_some(),
            "a full row folds its controls"
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
    start_shell(handle, &view, &pod, cx);
    let running = |cx: &mut TestAppContext| cx.read(shell::running_anywhere);
    assert_eq!(running(cx), [SharedString::from(pod.name.clone())]);
    // The terminal hands the keyboard back with Command-Escape on macOS and
    // Ctrl-Shift-Q elsewhere.
    let leave = if cfg!(target_os = "macos") {
        "secondary-escape"
    } else {
        "ctrl-shift-q"
    };

    // Option-Down in the terminal is the shell's; from the list, another
    // connection asks, and Cancel keeps this one and the shell.
    step(cx, &|window, cx| window.press("alt-down", cx));
    assert!(!cx.has_pending_prompt());
    step(cx, &|window, cx| window.press(leave, cx));
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
    assert_eq!(running(cx).len(), 1);

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
    assert!(running(cx).is_empty());

    // Agreeing to another connection ends the shell and moves on.
    step(cx, &|window, cx| window.click("pod-shell-start", cx));
    assert_eq!(running(cx).len(), 1);
    step(cx, &|window, cx| window.press(leave, cx));
    step(cx, &|window, cx| window.press("alt-down", cx));
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    assert_ne!(
        cx.read(|cx| view.read(cx).applied.context.clone()),
        Some(context)
    );
    assert!(running(cx).is_empty());
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
        window.click("workloads-refresh", cx);
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
        window.click(row, cx);
        window.click("system-service-open", cx);
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
        // Back on the page, the row is still selected and the list has
        // the keyboard, so L opens its logs.
        window.press("secondary-7", cx);
        window.render_frame(cx);
        window.press("l", cx);
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
fn monitoring_breadcrumb_opens_a_collapsed_dashboards_column(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Monitoring, window, cx));
        window.render_frame(cx);
        assert!(pilot.read(cx).column_collapsed(window));
        window.click("monitoring-dashboards", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert!(!pilot.read(cx).column_collapsed(window));
        window.render_frame(cx);
        // An open column stays open.
        window.click("monitoring-dashboards", cx);
    })
    .unwrap();
    cx.run_until_parked();
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
            // Nor does a fade over a cut edge dim it (#406).
            let fade = crate::ui::dp_px(28., window);
            let (top, bottom) = super::shell::cut_edges(&pilot.read(cx).obs_column_scroll);
            assert!(
                (!top || item.top() >= list.top() + fade - px(0.5))
                    && (!bottom || item.bottom() <= list.bottom() - fade + px(0.5)),
                "{destination:?}: {item:?} under a fade of {list:?}"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn every_expanded_column_puts_its_caption_and_rows_in_one_place(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        // Each area's collapse button (beside its caption) and first row.
        let mut seen = Vec::new();
        for (page, first) in [
            (Page::Etcd, "nav-etcd"),
            (Page::Health, "nav-health"),
            (Page::Monitoring, "nav-collapse"),
            (Page::Observability, "nav-obs-applications"),
        ] {
            pilot.update(cx, |pilot, cx| pilot.navigate(page, window, cx));
            window.render_frame(cx);
            assert!(!pilot.read(cx).column_collapsed(window));
            let toggle = window.find("nav-collapse").bounds();
            let row = window.find(first).bounds();
            seen.push((page, toggle, row));
        }
        let (_, toggle, row) = seen[0];
        for (page, other, other_row) in &seen[1..] {
            assert_eq!(*other, toggle, "{page:?}'s caption moved");
            if *page != Page::Monitoring {
                assert_eq!(other_row.origin, row.origin, "{page:?}'s first row moved");
                assert_eq!(
                    other_row.size.width, row.size.width,
                    "{page:?}'s rows are wider"
                );
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_cut_rail_or_column_fades_at_the_edges_it_cuts(cx: &mut TestAppContext) {
    use super::shell::cut_edges;
    // At 20 px text a 560 high window shows neither the rail's areas nor
    // the collapsed column's destinations in full (#406).
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        window.render_frame(cx);
        let edges = |pilot: &Entity<Pilot>, cx: &gpui_kit::App| {
            let pilot = pilot.read(cx);
            (
                cut_edges(&pilot.rail_scroll),
                cut_edges(&pilot.obs_column_scroll),
            )
        };
        // Overview, at the top, leaves only the rail's bottom cut.
        assert_eq!(edges(&pilot, cx).0, (false, true));
        pilot.update(cx, |pilot, cx| {
            pilot.navigate(Page::Observability, window, cx)
        });
        window.render_frame(cx);
        window.render_frame(cx);
        // The column has scrolled to Applications, so it is cut at the
        // bottom; the rail shows Observability clear of its fades.
        let column = edges(&pilot, cx).1;
        assert!(column.1, "{column:?}");
        let rail = pilot.read(cx).rail_scroll.clone();
        assert_clear_of_fades(window, "nav-rail", "nav-observability", &rail);
        for _ in 0..30 {
            window.scroll(
                "nav-rail",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-40.))),
                cx,
            );
            window.render_frame(cx);
        }
        assert_eq!(edges(&pilot, cx).0, (true, false));
        window.scroll(
            "nav-rail",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(40.))),
            cx,
        );
        window.render_frame(cx);
        assert_eq!(edges(&pilot, cx).0, (true, true));
    })
    .unwrap();

    // With room for everything, nothing is cut.
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.navigate(Page::Observability, window, cx)
        });
        window.render_frame(cx);
        window.render_frame(cx);
        let pilot = pilot.read(cx);
        assert_eq!(cut_edges(&pilot.rail_scroll), (false, false));
        assert_eq!(cut_edges(&pilot.column_scroll), (false, false));
        assert_eq!(cut_edges(&pilot.obs_column_scroll), (false, false));
    })
    .unwrap();
}

/// Asserts that `item` shows inside the scrolling `list`, and that no fade
/// over a cut edge of it dims the item (#406).
fn assert_clear_of_fades(
    window: &mut gpui_kit::Window,
    list: &'static str,
    item: impl Into<SharedString>,
    scroll: &gpui_kit::ScrollHandle,
) {
    let item = item.into();
    let area = window.find(list).bounds();
    let bounds = window.find(item.clone()).bounds();
    let fade = crate::ui::dp_px(28., window);
    let (top, bottom) = super::shell::cut_edges(scroll);
    assert!(
        bounds.top() >= area.top() + if top { fade } else { px(0.) } - px(0.5)
            && bounds.bottom() <= area.bottom() - if bottom { fade } else { px(0.) } + px(0.5),
        "{item}: {bounds:?} under a fade or outside {area:?} (cut {top}, {bottom})"
    );
}

/// A reveal measures the layout it scrolls in, not the one before it: a
/// change of room or of the column's state between two reveals still
/// leaves the item in view and clear of the fades (#406).
#[gpui_kit::test]
fn a_reveal_follows_a_change_of_room_or_column(cx: &mut TestAppContext) {
    use crate::observability::Destination;
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    let settle = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        for _ in 0..3 {
            window.render_frame(cx);
        }
    };
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.navigate(Page::Observability, window, cx)
        });
        let page = pilot.read(cx).observability.clone();
        page.update(cx, |page, cx| page.open(Destination::Deployments, cx));
        settle(window, cx);
        let scroll = pilot.read(cx).obs_column_scroll.clone();
        assert_clear_of_fades(
            window,
            "obs-navigation-scroll",
            "nav-obs-deployments",
            &scroll,
        );
        // Larger text cuts the column the item was revealed in.
        crate::text_size::set(20., cx);
        settle(window, cx);
        assert!(pilot.read(cx).column_collapsed(window));
        assert_ne!(super::shell::cut_edges(&scroll), (false, false));
        assert_clear_of_fades(
            window,
            "obs-navigation-scroll",
            "nav-obs-deployments",
            &scroll,
        );
        // Expanded, its rows are taller.
        window.click("nav-collapse", cx);
        settle(window, cx);
        assert!(!pilot.read(cx).column_collapsed(window));
        assert_clear_of_fades(
            window,
            "obs-navigation-scroll",
            "nav-obs-deployments",
            &scroll,
        );
        // From a short column, a kind low in a long group; the column's
        // scroll is shared by every area.
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Etcd, window, cx));
        settle(window, cx);
        let key = "validatingadmissionpolicybindings.admissionregistration.k8s.io";
        pilot.update(cx, |pilot, cx| pilot.open_builtin(key, window, cx));
        settle(window, cx);
        let column = pilot.read(cx).column_scroll.clone();
        assert_clear_of_fades(window, "nav-column", format!("nav-k8s-{key}"), &column);
        // And the rail shows Control plane, low in it, as Command-9 opens.
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Lifecycle, window, cx));
        settle(window, cx);
        let rail = pilot.read(cx).rail_scroll.clone();
        assert_clear_of_fades(window, "nav-rail", "nav-control-plane", &rail);
    })
    .unwrap();
}

/// A wheel over the rail redraws the rail, so its fades follow the scroll,
/// and none of the other cached parts.
#[gpui_kit::test]
fn a_rail_wheel_redraws_the_rail_alone(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
    })
    .unwrap();
    for _ in 0..5 {
        cx.update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
    // Not `window.scroll`, which draws whole frames around the wheel and
    // so passes every cache.
    let rail = cx
        .update_window(handle, |_, window, _| window.find("nav-rail").bounds())
        .unwrap();
    let before = chrome_counts();
    cx.update_window(handle, |_, window, cx| {
        use gpui_kit::InputEvent as _;
        window.dispatch_event(
            gpui_kit::ScrollWheelEvent {
                position: rail.center(),
                delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-40.))),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.simulate_next_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let after = chrome_counts();
    let scrolled = cx.read(|cx| super::shell::cut_edges(&pilot.read(cx).rail_scroll).0);
    assert!(scrolled, "the wheel scrolled the rail");
    assert!(
        after[1] > before[1],
        "the rail drew again: {before:?} → {after:?}"
    );
    assert_eq!(after[0], before[0], "the header drew again");
    assert_eq!(after[2], before[2], "the column drew again");
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
fn coroot_links_open_without_asking_while_a_shell_runs(cx: &mut TestAppContext) {
    use crate::resources::{example, live, shell};
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
    start_shell(handle, &view, &pods[0], cx);
    let running = vec![SharedString::from(pods[0].name.clone())];
    assert_eq!(cx.read(shell::running_anywhere), running);
    step(cx, &|window, cx| {
        view.update(cx, |view, cx| {
            view.navigate(Page::Observability, window, cx);
            view.observability.update(cx, |page, cx| {
                page.fixture_link(access.clone(), &pods[1].namespace, &pods[1].name, cx);
            });
        })
    });
    // The shell lives in the dock, so following a link leaves it be.
    step(cx, &|window, cx| window.click("obs-open-object", cx));
    assert!(!cx.has_pending_prompt());
    cx.read(|cx| {
        assert_eq!(shell::running_anywhere(cx), running);
        assert_eq!(view.read(cx).page, Page::Resources);
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

/// Draws until the page's header stops asking for frames: it folds its
/// controls from what its parts measured on the frame before.
pub(crate) fn settle_header(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    for _ in 0..4 {
        window.render_frame(cx);
        if window.simulate_next_frame(cx) == 0 {
            return;
        }
    }
    panic!("the header keeps moving");
}

#[gpui_kit::test]
fn at_the_default_window_pods_folds_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1320., 860.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.open_builtin("pods", window, cx));
        settle_header(window, cx);
        assert!(window.try_find("resource-more").is_none());
        let row = window.find("resource-toolbar").bounds();
        assert_eq!(row.size.height, crate::ui::dp_px(38., window));
        // Namespace, Columns and Refresh, all on the toolbar's row.
        for ix in 0..3 {
            let slot = window.find(format!("resource-slot-{ix}")).bounds();
            assert!(
                slot.top() >= row.top() && slot.bottom() <= row.bottom(),
                "{ix}"
            );
        }
        let chips = window.find("resource-tally-healthy").bounds();
        assert!(chips.top() >= row.top() && chips.bottom() <= row.bottom());
    })
    .unwrap();
}

/// The status bar's page segment, as its id's accessibility label, if the
/// bar shows one.
fn segment(window: &mut gpui_kit::Window, id: &'static str) -> Option<String> {
    let element = window.try_find(id)?;
    assert!(
        element
            .path()
            .contains(&gpui_kit::ElementId::from("status-bar")),
        "{id} is in the status bar"
    );
    element.label().map(str::to_owned)
}

#[gpui_kit::test]
fn the_visible_page_fills_the_status_bar_and_its_reads_reach_it(cx: &mut TestAppContext) {
    use crate::resources::model::ReadState;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Overview has no segment.
        for id in [
            "resource-scope",
            "system-services-scope",
            "nodes-scope",
            "obs-scope",
        ] {
            assert!(window.try_find(id).is_none(), "{id} shows on Overview");
        }
        view.update(cx, |view, cx| view.navigate(Page::Resources, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let resources = cx.update(|cx| view.read(cx).resources.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let line = segment(window, "resource-scope").expect("Pods fill the status bar");
        assert!(line.starts_with("Example data"), "{line}");
        assert!(!line.contains("reconnecting"), "{line}");
    })
    .unwrap();

    // A read that goes stale, then recovers, reaches the bar on the next
    // frame with no other input: the page notifies, which asks for a frame,
    // and the shell reads the page's segment while the page's own view
    // stays cached.
    let notices = Rc::new(Cell::new(0usize));
    cx.update(|cx| {
        let seen = notices.clone();
        cx.observe(&resources, move |_, _| seen.set(seen.get() + 1))
            .detach();
    });
    for (state, stale) in [
        (ReadState::Stale("connection reset".into()), true),
        (ReadState::Loaded, false),
    ] {
        let before = notices.get();
        cx.update(|cx| resources.update(cx, |screen, cx| screen.deliver_read(state, cx)));
        assert!(notices.get() > before, "the read asks for a frame");
        cx.update_window(handle, |_, window, cx| {
            draw(window, cx);
            let line = segment(window, "resource-scope").unwrap();
            assert_eq!(line.contains("reconnecting"), stale, "{line}");
        })
        .unwrap();
    }

    // Another page replaces the segment.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.navigate(Page::SystemServices, window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("resource-scope").is_none());
        let line = segment(window, "system-services-scope").expect("System services' segment");
        assert!(line.contains("services on"), "{line}");
    })
    .unwrap();
}

/// A screen whose status bar segment a test sets.
struct SegmentScreen(Option<freshkube_ui::status::Segment>);

impl gpui_kit::EventEmitter<crate::screens::ScreenEvent> for SegmentScreen {}

impl gpui_kit::Render for SegmentScreen {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

impl crate::screens::ScreenPanel for SegmentScreen {
    fn new(
        _: tokio::runtime::Handle,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> Self {
        Self(None)
    }

    fn set_source(
        &mut self,
        _: Option<crate::screens::ScreenSource>,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) {
    }

    fn activate(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) {}

    fn refresh(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) {}

    fn status(&mut self) -> Option<&freshkube_ui::status::Segment> {
        self.0.as_ref()
    }
}

#[gpui_kit::test]
fn a_screens_segment_shows_only_while_its_page_is_visible(cx: &mut TestAppContext) {
    use super::pages::ScreenKind;
    use freshkube_ui::status::Segment;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    // etcd's page draws a screen that sets its segment as the test says.
    let screen = cx.new(|_| SegmentScreen(Some(Segment::new(None::<SharedString>, ["3 members"]))));
    cx.update(|cx| {
        view.update(cx, |view, _| {
            let slot = view
                .screens
                .iter_mut()
                .find(|(kind, _)| *kind == ScreenKind::Etcd)
                .expect("etcd has a screen");
            slot.1 = crate::screens::ScreenHandle::new(screen.clone());
        })
    });
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.navigate(Page::Etcd, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let line = segment(window, "etcd-scope").expect("etcd's segment");
        assert!(line.contains("3 members"), "{line}");
    })
    .unwrap();

    // A new segment reaches the bar on the next frame: the screen's notify
    // asks for one and marks the shell above it dirty, with no event of its
    // own.
    let notices = Rc::new(Cell::new(0usize));
    cx.update(|cx| {
        let seen = notices.clone();
        cx.observe(&screen, move |_, _| seen.set(seen.get() + 1))
            .detach();
        screen.update(cx, |screen, cx| {
            screen.0 = Some(Segment::new(None::<SharedString>, ["2 members"]));
            cx.notify();
        })
    });
    assert_eq!(notices.get(), 1, "the change asks for a frame");
    cx.update_window(handle, |_, window, cx| {
        draw(window, cx);
        let line = segment(window, "etcd-scope").unwrap();
        assert!(line.contains("2 members"), "{line}");
    })
    .unwrap();

    // Hidden, the screen's segment never shows, even when it changes: the
    // bar shows the visible page's own.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.navigate(Page::Security, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        screen.update(cx, |screen, cx| {
            screen.0 = Some(Segment::new(None::<SharedString>, ["1 member"]));
            cx.notify();
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("etcd-scope").is_none());
        let line = segment(window, "security-scope");
        assert!(
            line.as_ref().is_none_or(|line| !line.contains("member")),
            "{line:?}"
        );
    })
    .unwrap();
}

/// etcd and Security hand their line to the status bar, and their headers
/// keep none: the quorum and leader, then where the audit read from.
#[gpui_kit::test]
fn etcd_and_security_fill_the_status_bar(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    for (page, id, expect) in [
        (Page::Etcd, "etcd-scope", ["3 members", "Quorum", "leader "]),
        (
            Page::Security,
            "security-scope",
            ["endpoints ", "volume target ", "example data"],
        ),
    ] {
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.navigate(page, window, cx));
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let line = segment(window, id).unwrap_or_else(|| panic!("{id} in the bar"));
            for part in expect {
                assert!(line.contains(part), "{part:?} in {line}");
            }
        })
        .unwrap();
    }
}

/// In the narrowest window, a page's segment drops its minor parts and
/// then its other parts before its warnings. Example mode's bar is about
/// 200 px narrower than a live one, for Simulate failure and Example data:
/// at the largest text the first part goes there, and even the warnings
/// are cut, but the first one still leads.
#[gpui_kit::test]
fn a_narrow_status_bar_drops_minor_parts_and_keeps_the_toned_ones(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    for text_size in [13., 20.] {
        for (page, id, lead, warnings) in [
            (
                Page::Health,
                "health-scope",
                "deployments",
                &["degraded", "failing"][..],
            ),
            // No alerts in the example: nothing outranks the first part.
            (Page::Lifecycle, "lifecycle-scope", "Talos ", &[]),
        ] {
            cx.update_window(handle, |_, window, cx| {
                crate::text_size::set(text_size, cx);
                view.update(cx, |view, cx| view.navigate(page, window, cx));
                window.render_frame(cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let line = segment(window, id).unwrap_or_else(|| panic!("{id} in the bar"));
                let shown = freshkube_ui::status::shown(id).expect("a drawn line");
                let room = format!(
                    "{shown:?} of {line:?} in {:?} of a {:?} bar at {text_size}",
                    window.find(id).bounds().size.width,
                    window.find("status-bar").bounds().size.width,
                );
                // The label and tooltip keep the whole line.
                assert!(line.ends_with("example data"), "{line}");
                assert!(!shown.contains("example data"), "{room}");
                assert!(!shown.contains("updated"), "{room}");
                assert!(!shown.contains("no alerts"), "a count of none goes: {room}");
                let Some(first) = warnings.first() else {
                    assert!(shown.starts_with(lead), "{room}");
                    return;
                };
                assert!(shown.contains(first), "{room}");
                if text_size == 13. {
                    // Room for the first part beside the warnings.
                    assert!(shown.contains(lead), "{room}");
                    assert!(warnings.iter().all(|part| shown.contains(part)), "{room}");
                    assert!(!shown.ends_with('…'), "the warnings fit whole: {room}");
                } else {
                    assert!(
                        !shown.contains(lead),
                        "warnings outrank the first part: {room}"
                    );
                }
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn a_compact_status_bar_keeps_the_shells_glyph_and_gives_the_page_the_room(
    cx: &mut TestAppContext,
) {
    for (width, compact) in [(1280., false), (1000., true)] {
        let (_runtime, handle, view) = fixture(cx, width, 820.);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.navigate(Page::SystemServices, window, cx)
            });
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let status = window.find("overview-status");
            let label = status.label().unwrap_or_default().to_owned();
            assert!(!label.is_empty(), "the shell's status keeps its label");
            let glyph_only = status.bounds().size.width < crate::ui::dp_px(24., window);
            assert_eq!(glyph_only, compact, "at {width}: {:?}", status.bounds());
            let page = window.find("system-services-scope");
            assert!(page.visible());
            assert!(page.bounds().left() >= status.bounds().right());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_long_segment_truncates_and_leaves_the_right_side_whole(cx: &mut TestAppContext) {
    // At 760 and 20 px text Nodes' line is wider than the bar can give it.
    let (_runtime, handle, view) = fixture(cx, 760., 560.);
    let mut widths = Vec::new();
    for page in [Page::Overview, Page::Nodes] {
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(20., cx);
            view.update(cx, |view, cx| view.navigate(page, window, cx));
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // The right side's last item is the first a squeezed bar loses.
            let rate = window.find("frame-rate").bounds();
            assert!(rate.right() <= window.viewport_size().width, "{rate:?}");
            widths.push(rate.size.width);
            let fail = window.find("fixture-fail").bounds();
            if page == Page::Nodes {
                let scope = window.find("nodes-scope").bounds();
                assert!(scope.right() <= fail.left(), "{scope:?} {fail:?}");
            }
        })
        .unwrap();
    }
    assert_eq!(widths[0], widths[1], "the right side keeps its width");
}
