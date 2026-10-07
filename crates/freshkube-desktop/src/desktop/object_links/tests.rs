use crate::{
    desktop::{Page, nodes::NodeTab, tests::fixture},
    resources::{Tab, example, model::ResourceIdentity},
};
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};

/// Draws until the drawer settles, then asserts that Details landed on
/// `section`: its top at the scroller's top, or, when what follows is too
/// short for that, scrolled to the end with the section's top in view.
fn assert_landed(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, section: &str) {
    for _ in 0..4 {
        window.render_frame(cx);
        if window.simulate_next_frame(cx) == 0 {
            break;
        }
    }
    let one = gpui_kit::px(1.);
    let scroller = window.find("detail-details").bounds();
    let top = window
        .find(gpui_kit::SharedString::from(format!(
            "detail-section-{section}"
        )))
        .bounds()
        .top()
        - scroller.top();
    // The last section ends the content, so its bottom shows the end.
    let end = window.find("detail-section-events").bounds().bottom();
    assert!(
        top >= -one && (top <= one || end <= scroller.bottom() + one),
        "{section} lands {top:?} below the top, the content ending at {end:?} of {scroller:?}"
    );
    assert_eq!(
        window
            .find(gpui_kit::SharedString::from(format!(
                "detail-jump-{section}"
            )))
            .selected(),
        Some(true)
    );
}
fn pod() -> (ResourceIdentity, freshkube_core::resources::ObjectDocument) {
    let identity = example::read("prod-fra", "pods", None, chrono::Utc::now().timestamp())
        .unwrap()
        .1
        .into_iter()
        .find(|row| {
            row.identity.namespace == "payments"
                && row.identity.name.starts_with("worker-")
                && row.cells[6] == "talos-wk-fra1-02"
        })
        .unwrap()
        .identity;
    let document = example::document(&identity, chrono::Utc::now().timestamp()).unwrap();
    (identity, document)
}
#[gpui_kit::test]
fn pod_links_reach_node_replica_set_deployment_and_service_ports(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1700., 1200.);
    let (identity, document) = pod();
    let open_pod = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                freshkube_core::resources::builtin("pods").unwrap(),
                identity.clone().into(),
                Tab::Overview,
                window,
                cx,
            )
        });
        window.render_frame(cx);
    };
    for (button, tab) in [
        ("pod-runs-on", NodeTab::Overview),
        ("pod-node-services", NodeTab::Services),
    ] {
        cx.update_window(handle, |_, window, cx| {
            open_pod(window, cx);
            assert!(window.find(button).visible());
            window.click(button, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(pilot.read(cx).page, Page::Nodes);
            assert_eq!(pilot.read(cx).node_workspace.tab, tab);
            assert_eq!(
                pilot
                    .read(cx)
                    .node_workspace
                    .selected
                    .as_ref()
                    .unwrap()
                    .kubernetes,
                document.overview.pod.as_ref().unwrap().node
            );
        });
    }
    let owner = &document.overview.owners[0];
    cx.update_window(handle, |_, window, cx| {
        open_pod(window, cx);
        window.within("pod-cross-links").click(
            format!("owner-{}-{}-{}", owner.kind, identity.namespace, owner.name),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    let deployment = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(pilot.read(cx).resource_kind.key(), "replicasets.apps");
            let rs = example::document(
                pilot
                    .read(cx)
                    .resources
                    .read(cx)
                    .detail_identity(cx)
                    .unwrap(),
                chrono::Utc::now().timestamp(),
            )
            .unwrap();
            let owner = &rs.overview.owners[0];
            window.within("detail-owners").click(
                format!("owner-{}-{}-{}", owner.kind, identity.namespace, owner.name),
                cx,
            );
            owner.name.clone()
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(pilot.read(cx).resource_kind.key(), "deployments.apps");
        assert_eq!(
            pilot
                .read(cx)
                .resources
                .read(cx)
                .detail_identity(cx)
                .unwrap()
                .name,
            deployment
        );
    });
    for (button, tab) in [
        ("pod-service-open", Tab::Overview),
        ("pod-service-forward", Tab::Ports),
    ] {
        cx.update_window(handle, |_, window, cx| {
            open_pod(window, cx);
            window
                .within("selected-service-payments-worker")
                .click(button, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(pilot.read(cx).resource_kind.key(), "services");
            assert_eq!(pilot.read(cx).resources.read(cx).detail_tab(cx), tab);
            if tab == Tab::Ports {
                assert_landed(window, cx, "ports");
            }
        })
        .unwrap();
    }
}
#[gpui_kit::test]
fn pod_node_link_goes_at_once_and_the_shell_runs_on(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1700., 1200.);
    let (identity, _) = pod();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                freshkube_core::resources::builtin("pods").unwrap(),
                identity.clone().into(),
                Tab::Overview,
                window,
                cx,
            )
        });
    })
    .unwrap();
    cx.run_until_parked();
    crate::desktop::tests::start_shell(handle, &pilot, &identity, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("pod-runs-on", cx);
    })
    .unwrap();
    assert!(!cx.has_pending_prompt());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).page, Page::Nodes);
        assert_eq!(
            crate::resources::shell::running_anywhere(cx),
            [gpui_kit::SharedString::from(identity.name.clone())]
        );
    })
    .unwrap();
}
#[gpui_kit::test]
fn kubelet_links_to_its_scoped_pods(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1700., 1200.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_node_by_name("talos-wk-fra1-02", NodeTab::Services, window, cx);
            pilot.selected_service = Some("kubelet".into());
            cx.notify();
        });
        window.render_frame(cx);
        assert!(
            window
                .find("kubelet-node-pods")
                .label()
                .unwrap()
                .contains("Pods on this node")
        );
        window.click("kubelet-node-pods", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.tab, NodeTab::Pods);
        assert_eq!(
            pilot.read(cx).node_pods.read(cx).field_selector_value(),
            Some("spec.nodeName=talos-wk-fra1-02")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn container_actions_choose_current_previous_and_events(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1700., 1400.);
    let identity = example::read("prod-fra", "pods", None, chrono::Utc::now().timestamp())
        .unwrap()
        .1
        .into_iter()
        .find(|row| row.cells[2] == "CrashLoopBackOff")
        .unwrap()
        .identity;
    let document = example::document(&identity, chrono::Utc::now().timestamp()).unwrap();
    let name = document
        .overview
        .pod
        .as_ref()
        .unwrap()
        .containers
        .iter()
        .find(|c| c.has_previous())
        .unwrap()
        .name
        .clone();
    // Logs open in the dock and close the drawer, so each action opens
    // the pod's Overview again first.
    let reopen = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                freshkube_core::resources::builtin("pods").unwrap(),
                identity.clone().into(),
                Tab::Overview,
                window,
                cx,
            )
        });
        window.render_frame(cx);
    };
    cx.update_window(handle, |_, window, cx| {
        reopen(window, cx);
        window
            .within(format!("pod-container-{name}"))
            .click("pod-container-previous", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The container's previous instance opens in the dock, and the
        // drawer steps aside.
        assert!(window.try_find("resource-drawer").is_none());
        assert_eq!(window.find("pod-logs-previous").checked(), Some(true));
        assert_eq!(
            pilot
                .read(cx)
                .dock
                .read(cx)
                .selected_container(cx)
                .as_deref(),
            Some(name.as_str())
        );
        reopen(window, cx);
        window
            .within(format!("pod-container-{name}"))
            .click("pod-container-logs", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The same tab turns to the current instance.
        assert_eq!(pilot.read(cx).dock.read(cx).tabs.len(), 1);
        assert_eq!(window.find("pod-logs-previous").checked(), Some(false));
        reopen(window, cx);
        window.click("detail-tab-details", cx);
        window.render_frame(cx);
        // Recent events sit below the history; scroll until the link is
        // whole, since a click lands on its centre.
        for _ in 0..20 {
            let (link, pane) = (
                window.find("pod-all-events").bounds(),
                window.find("detail-details").bounds(),
            );
            if link.bottom() <= pane.bottom() && link.top() >= pane.top() {
                break;
            }
            window.scroll(
                "detail-details",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(-60.),
                )),
                cx,
            );
            window.render_frame(cx);
        }
        window.click("pod-all-events", cx);
        // The link scrolls Details to its events.
        assert_landed(window, cx, "events");
        assert_eq!(
            pilot.read(cx).resources.read(cx).detail_tab(cx),
            Tab::Events
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn owner_and_service_links_go_at_once_while_a_shell_runs(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1700., 1400.);
    let (identity, document) = pod();
    let open_pod = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| {
                pilot.open_object(
                    freshkube_core::resources::builtin("pods").unwrap(),
                    identity.clone().into(),
                    Tab::Overview,
                    window,
                    cx,
                )
            });
        })
        .unwrap();
        cx.run_until_parked();
    };
    open_pod(cx);
    crate::desktop::tests::start_shell(handle, &pilot, &identity, cx);
    let running = vec![gpui_kit::SharedString::from(identity.name.clone())];
    let owner = &document.overview.owners[0];
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("pod-cross-links").click(
            format!("owner-{}-{}-{}", owner.kind, identity.namespace, owner.name),
            cx,
        );
    })
    .unwrap();
    assert!(!cx.has_pending_prompt());
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        let shown = pilot
            .read(cx)
            .resources
            .read(cx)
            .detail_identity(cx)
            .cloned();
        assert_eq!(shown.map(|shown| shown.name), Some(owner.name.clone()));
        assert_eq!(crate::resources::shell::running_anywhere(cx), running);
    })
    .unwrap();

    open_pod(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .within("selected-service-payments-worker")
            .click("pod-service-forward", cx);
    })
    .unwrap();
    assert!(!cx.has_pending_prompt());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).resource_kind.key(), "services");
        assert_eq!(pilot.read(cx).resources.read(cx).detail_tab(cx), Tab::Ports);
        assert_landed(window, cx, "ports");
        assert_eq!(crate::resources::shell::running_anywhere(cx), running);
    })
    .unwrap();
}

/// Opens a pod in the drawer, as a link does, and draws until the list's
/// measured width settles.
fn open_in_drawer(
    handle: gpui_kit::AnyWindowHandle,
    pilot: &gpui_kit::Entity<crate::desktop::Pilot>,
    cx: &mut TestAppContext,
) {
    let (identity, _) = pod();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                freshkube_core::resources::builtin("pods").unwrap(),
                identity.into(),
                Tab::Overview,
                window,
                cx,
            )
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        while window.simulate_next_frame(cx) > 0 {
            window.render_frame(cx);
        }
    })
    .unwrap();
}

/// In the app's frame at 1280×880, beside the rail and the column, the
/// page is about 1008 dp wide, and the default drawer leaves the list at
/// least 280 dp; at text size 20 the page is narrower in dp and the column
/// folds, and the list still keeps its 280.
#[gpui_kit::test]
fn the_drawer_leaves_the_list_its_room_in_the_frame(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    for text_size in [13., 20.] {
        cx.update(|cx| crate::text_size::set(text_size, cx));
        open_in_drawer(handle, &pilot, cx);
        cx.update_window(handle, |_, window, cx| {
            let dp = crate::ui::dp_px(1., window);
            let body = window.find("resource-split").bounds();
            let page = body.size.width / dp;
            // Beside the rail and the column; the column folds at text
            // size 20.
            let frame = if text_size == 13. { 64. + 208. } else { 64. };
            assert!(
                page <= 1280. * 13. / text_size - frame,
                "{text_size}: page {page}"
            );
            let drawn = crate::desktop::layout_check::assert_drawer(
                window,
                cx,
                "resource-split",
                "resource-drawer",
                "detail-inspector",
                "detail-title",
            );
            assert!(drawn < page, "{text_size}: the whole page");
        })
        .unwrap();
    }
}

/// At 760×560 and text size 20 the drawer takes the whole page, under the
/// toolbar, with its header and tabs in sight.
#[gpui_kit::test]
fn a_short_window_keeps_the_drawers_header_and_tabs_in_sight(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update(|cx| crate::text_size::set(20., cx));
    open_in_drawer(handle, &pilot, cx);
    cx.update_window(handle, |_, window, cx| {
        let drawn = crate::desktop::layout_check::assert_drawer(
            window,
            cx,
            "resource-split",
            "resource-drawer",
            "detail-inspector",
            "detail-title",
        );
        let body = window.find("resource-split").bounds();
        assert!((drawn - body.size.width / crate::ui::dp_px(1., window)).abs() <= 1.);
        let drawer = window.find("resource-drawer").bounds();
        let viewport = window.viewport_size();
        for id in ["detail-title", "detail-close", "detail-tabs"] {
            let bounds = window.find(id).bounds();
            assert!(window.find(id).visible(), "{id}");
            assert!(
                bounds.top() >= drawer.top() && bounds.bottom() <= viewport.height,
                "{id}: {bounds:?} out of sight"
            );
        }
        // The page's toolbar stays above it.
        assert!(window.find("resource-filter").bounds().bottom() <= drawer.top());
    })
    .unwrap();
}

/// Forward opens a pod's Service in a drawer that has never drawn, by
/// the same entry point as here, and lands its Ports at the top of
/// Details, not past it: a short window leaves room to overshoot (#322
/// review).
#[gpui_kit::test]
fn forward_lands_a_fresh_drawer_on_its_ports(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 440.);
    let service = example::read("prod-fra", "services", None, chrono::Utc::now().timestamp())
        .unwrap()
        .1
        .into_iter()
        .find(|row| row.identity.namespace == "payments" && row.identity.name == "worker")
        .unwrap()
        .identity;
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                freshkube_core::resources::builtin("services").unwrap(),
                service.into(),
                Tab::Ports,
                window,
                cx,
            )
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(pilot.read(cx).resource_kind.key(), "services");
        assert_landed(window, cx, "ports");
    })
    .unwrap();
}

/// A group row beside the drawer lays out within the room the drawer
/// leaves, its subject shrinking first, so its labels stay in sight
/// (#321).
#[gpui_kit::test]
fn group_labels_stay_clear_of_the_drawer(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1320., 880.);
    open_in_drawer(handle, &pilot, cx);
    cx.update_window(handle, |_, window, _| {
        let drawer = window.find("resource-drawer").bounds();
        let list = window.find("resource-list-area").bounds();
        assert!(
            drawer.left() - list.left() < gpui_kit::px(400.),
            "{list:?} {drawer:?}"
        );
        // The group's actions are its menu's (DESIGN.md change 10); what
        // it shows of itself lays out in the room the drawer leaves.
        for key in [
            "resource-group-failing",
            "resource-group-node:talos-wk-fra1-02",
        ] {
            let id = gpui_kit::ElementId::from(gpui_kit::SharedString::from(key));
            let label = window.find((id, "label")).bounds();
            assert!(
                label.right() <= drawer.left(),
                "{key}: {label:?} under the drawer at {:?}",
                drawer.left()
            );
        }
    })
    .unwrap();
}
