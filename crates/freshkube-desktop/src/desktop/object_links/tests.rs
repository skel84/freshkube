use crate::{
    desktop::{Page, nodes::NodeTab, tests::fixture},
    resources::{Tab, example, model::ResourceIdentity},
};
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};
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
                assert!(window.find("ports").visible());
            }
        })
        .unwrap();
    }
}
#[gpui_kit::test]
fn pod_node_link_asks_and_cancel_keeps_the_pane_and_target(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1700., 1200.);
    let (identity, _) = pod();
    let target = cx.update(|cx| pilot.read(cx).selected_node.clone());
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                freshkube_core::resources::builtin("pods").unwrap(),
                identity.clone().into(),
                Tab::Shell,
                window,
                cx,
            )
        });
        window.render_frame(cx);
        window.click("pod-shell-start", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("detail-tab-overview", cx);
        window.render_frame(cx);
        window.click("pod-runs-on", cx);
    })
    .unwrap();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).page, Page::Resources);
        assert_eq!(pilot.read(cx).selected_node, target);
        assert_eq!(
            pilot.read(cx).resources.read(cx).detail_identity(cx),
            Some(&identity)
        );
        assert!(window.find("detail-shell-running").visible());
        window.click("pod-runs-on", cx);
    })
    .unwrap();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).page, Page::Nodes);
        assert!(window.try_find("detail-shell-running").is_none());
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
        window.render_frame(cx);
        window
            .within(format!("pod-container-{name}"))
            .click("pod-container-previous", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).resources.read(cx).detail_tab(cx), Tab::Logs);
        assert_eq!(window.find("pod-logs-previous").checked(), Some(true));
        assert_eq!(
            pilot.read(cx).resources.read(cx).detail_log_container(cx),
            Some(name.as_str())
        );
        window.click("detail-tab-overview", cx);
        window.render_frame(cx);
        window
            .within(format!("pod-container-{name}"))
            .click("pod-container-logs", cx);
        window.render_frame(cx);
        assert_eq!(window.find("pod-logs-previous").checked(), Some(false));
        window.click("detail-tab-overview", cx);
        window.render_frame(cx);
        window.click("pod-all-events", cx);
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).resources.read(cx).detail_tab(cx),
            Tab::Events
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn owner_and_service_links_respect_a_running_shell(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1700., 1400.);
    let (identity, document) = pod();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                freshkube_core::resources::builtin("pods").unwrap(),
                identity.clone().into(),
                Tab::Shell,
                window,
                cx,
            )
        });
        window.render_frame(cx);
        window.click("pod-shell-start", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("detail-shell-running").visible());
        window.click("detail-tab-overview", cx);
        window.render_frame(cx);
        let owner = &document.overview.owners[0];
        window.within("pod-cross-links").click(
            format!("owner-{}-{}-{}", owner.kind, identity.namespace, owner.name),
            cx,
        );
    })
    .unwrap();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).resources.read(cx).detail_identity(cx),
            Some(&identity)
        );
        assert!(window.find("detail-shell-running").visible());
        window
            .within("selected-service-payments-worker")
            .click("pod-service-forward", cx);
    })
    .unwrap();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).resource_kind.key(), "services");
        assert_eq!(pilot.read(cx).resources.read(cx).detail_tab(cx), Tab::Ports);
        assert!(window.find("ports").visible());
        assert!(window.try_find("detail-shell-running").is_none());
    })
    .unwrap();
}
