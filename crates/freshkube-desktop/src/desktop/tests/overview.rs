use super::*;

const FIRST_NODE: &str = "talos-cp-fra1-01";

#[gpui_kit::test]
fn overview_keyboard_selection_updates_target_node(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        expand_healthy_nodes(window, cx);
        assert_eq!(
            view.read(cx).target.selected_node.as_deref(),
            Some(FIRST_NODE)
        );
        window
            .within("nodes-page")
            .click("node-talos-cp-fra1-01", cx);
    })
    .unwrap();
    // The click reaches the shell once the update ends.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.press("down", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).target.selected_node.as_deref(),
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
        assert!(
            view.read(cx)
                .pages
                .node_workspace
                .read(cx)
                .focus
                .is_focused(window)
        );
        window.press("tab", cx);
        assert!(
            !view
                .read(cx)
                .pages
                .node_workspace
                .read(cx)
                .focus
                .is_focused(window)
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
    })
    .unwrap();
    // Overview hears where things stand once the shell's effects run.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
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
                .pages
                .node_workspace
                .read(cx)
                .attention
                .rows
                .iter()
                .any(|row| row.name.contains(DEGRADED_NODE))
        );
        window.click("refresh", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
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
        assert!(!view.read(cx).chrome.automatic);
        assert_eq!(window.find("auto-refresh").checked(), Some(false));
    })
    .unwrap();
}

#[gpui_kit::test]
fn refused_summary_events_leave_health_and_other_parts_loaded(cx: &mut TestAppContext) {
    use freshkube_core::kubernetes_summary::Part;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            let mut summary = (**view
                .switch
                .registry
                .active()
                .kubernetes_summary
                .data()
                .unwrap())
            .clone();
            summary.events = Part::Refused("Can't list events: forbidden".into());
            let health = crate::screens::WorkloadData::from_outcome(&summary.workloads);
            let request = view
                .switch
                .registry
                .active_mut()
                .kubernetes_summary
                .begin(view.applied.clone());
            view.switch
                .registry
                .active_mut()
                .kubernetes_summary
                .apply(&request, Ok(std::sync::Arc::new(summary)));
            view.switch.registry.active_mut().summary_health = Some(health.clone());
            view.deliver_workloads(health, cx);
            cx.notify();
        });
        window.render_frame(cx);
        area(window, cx, "nav-k8s-group-workloads");
        window.click("nav-health", cx);
        window.find("workload-list");
        let summary = view
            .read(cx)
            .switch
            .registry
            .active()
            .kubernetes_summary
            .data()
            .unwrap();
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
    let previous = cx.update(|cx| {
        view.read(cx)
            .switch
            .registry
            .active()
            .kubernetes_summary
            .data()
            .unwrap()
            .clone()
    });
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
            view.read(cx)
                .switch
                .registry
                .active()
                .kubernetes_summary
                .data()
                .unwrap()
        ));
        view.update(cx, |view, cx| {
            let request = view
                .switch
                .registry
                .active_mut()
                .kubernetes_summary
                .begin(view.applied.clone());
            view.select_context("staging-eu".into(), window, cx);
            assert!(
                !view
                    .switch
                    .registry
                    .active_mut()
                    .kubernetes_summary
                    .apply(&request, Ok(previous.clone()))
            );
            assert_eq!(view.applied.context.as_deref(), Some("staging-eu"));
        });
    })
    .unwrap();
}
