use super::{NodeTab, join};
use crate::{
    desktop::{Page, tests::fixture},
    fixture as examples, presentation,
    resources::example,
    ui::Tone,
};
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};

#[test]
fn names_are_reserved_before_address_fallback_and_each_side_joins_once() {
    let mut talos = presentation::node_summaries(&examples::cluster("prod-fra", 0));
    let summary = example::summary("prod-fra", chrono::Utc::now().timestamp());
    let kube = summary.nodes.loaded().unwrap();
    talos[0].name = "different-hostname".into();
    talos[0].address = talos[1].address.clone();
    let rows = join::join(&talos, kube, true, true);
    assert_eq!(rows.len(), 7);
    let reserved = rows
        .iter()
        .find(|row| row.key.talos.as_deref() == Some(&talos[1].name))
        .unwrap();
    assert_eq!(
        reserved.key.kubernetes.as_deref(),
        Some(talos[1].name.as_str())
    );
    let unmatched = rows
        .iter()
        .find(|row| row.key.talos.as_deref() == Some("different-hostname"))
        .unwrap();
    assert_eq!(unmatched.ready, "Not in Kubernetes");
    assert_eq!(
        rows.iter()
            .filter(|row| row.key.kubernetes.is_some())
            .count(),
        6
    );
    let rows = join::join(&[], kube, false, true);
    assert!(
        rows.iter()
            .all(|row| row.talos_state == "Talos unavailable")
    );
    let rows = join::join(&talos, &[], true, false);
    assert!(rows.iter().all(|row| row.ready == "Kubernetes unavailable"));
}

#[gpui_kit::test]
fn joined_fixture_pane_preserves_selection_tab_and_target(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let rows = pilot.read(cx).node_workspace.rows.clone();
        assert_eq!(rows.len(), 6);
        let bad = rows
            .iter()
            .find(|row| row.name == "talos-wk-fra1-02")
            .unwrap();
        assert_eq!(bad.ready, "NotReady");
        assert_eq!(bad.tone, Tone::Crit);
        assert!(
            bad.problems
                .iter()
                .any(|problem| problem == "kubelet unhealthy")
        );
        let silent = rows
            .iter()
            .find(|row| row.name == "talos-wk-fra1-03")
            .unwrap();
        assert_eq!(silent.talos_state, "No response");
        window.click(bad.id.clone(), cx);
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).selected_node.as_deref(),
            Some("talos-wk-fra1-02")
        );
        let selected = pilot.read(cx).node_workspace.selected.clone();
        window.click("node-tab-processes", cx);
        window.render_frame(cx);
        window.click("node-expand", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.expanded);
        assert!(window.try_find("joined-nodes-list").is_none());
        window.click("node-expand", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.selected, selected);
        window.click("node-talos-cp-fra1-01", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.tab, NodeTab::Processes);
        assert_eq!(
            pilot.read(cx).selected_node.as_deref(),
            Some("talos-cp-fra1-01")
        );
        window.click("node-close", cx);
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).selected_node.as_deref(),
            Some("talos-cp-fra1-01")
        );
        assert_eq!(pilot.read(cx).page, Page::Nodes);
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubernetes_node_has_only_its_supported_tabs_and_narrow_back(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.nodes.clear();
            pilot.rebuild_joined_nodes();
            let key = pilot.node_workspace.rows[0].key.clone();
            pilot.open_node(key, window, cx);
        });
        window.render_frame(cx);
        assert!(window.find("node-back").visible());
        assert!(window.try_find("joined-nodes-list").is_none());
        assert!(window.find("node-tab-pods").visible());
        assert!(window.try_find("node-tab-processes").is_none());
        window.click("node-tab-yaml", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.tab, NodeTab::Yaml);
        window.click("node-back", cx);
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.open);
    })
    .unwrap();
}

#[test]
fn an_unmatched_hostname_uses_its_address_once() {
    let mut talos = presentation::node_summaries(&examples::cluster("prod-fra", 0));
    let summary = example::summary("prod-fra", chrono::Utc::now().timestamp());
    let kube = summary.nodes.loaded().unwrap();
    let name = talos[0].name.clone();
    talos[0].name = "changed-hostname".into();
    let rows = join::join(&talos, kube, true, true);
    assert_eq!(rows.len(), 6);
    assert_eq!(
        rows.iter()
            .find(|row| row.key.talos.as_deref() == Some("changed-hostname"))
            .unwrap()
            .key
            .kubernetes
            .as_deref(),
        Some(name.as_str())
    );
}

#[gpui_kit::test]
fn node_keys_expand_switch_tabs_and_step_back(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        window.click("node-talos-cp-fra1-01", cx);
        window.render_frame(cx);
        window.press("secondary-shift-enter", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.expanded);
        window.press("secondary-shift-enter", cx);
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.expanded);
        window.click("node-tab-processes", cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| {
            window.focus(&pilot.node_workspace.tab_focus, cx)
        });
        window.press("right", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.tab, NodeTab::Storage);
        window.press("secondary-{", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.tab, NodeTab::Processes);
        pilot.update(cx, |pilot, cx| {
            pilot.active_screen().unwrap().focus(window, cx)
        });
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.tab, NodeTab::Overview);
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.open);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_context_change_closes_the_old_node_document_and_replaces_rows(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            let key = pilot.node_workspace.rows[0].key.clone();
            pilot.open_node(key, window, cx);
            pilot.show_node_tab(NodeTab::Yaml, window, cx);
        });
        window.render_frame(cx);
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .document
                .read(cx)
                .target_identity()
                .is_some()
        );
        pilot.update(cx, |pilot, cx| {
            pilot.select_context("homelab".into(), window, cx)
        });
        window.render_frame(cx);
        let read = pilot.read(cx);
        assert!(!read.node_workspace.open);
        assert!(
            read.node_workspace
                .document
                .read(cx)
                .target_identity()
                .is_none()
        );
        assert_eq!(read.node_workspace.rows.len(), 1);
        assert_eq!(read.node_workspace.rows[0].name, "talos-home");
    })
    .unwrap();
}
