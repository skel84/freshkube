use super::{NodeTab, join};
use crate::{
    desktop::{Page, tests::fixture},
    fixture as examples, presentation,
    resources::example,
    ui::Tone,
};
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};

fn projection_sources() -> (
    presentation::NodeSummary,
    freshkube_core::kubernetes_summary::NodeSummary,
) {
    let talos = presentation::NodeSummary {
        name: "joined".into(),
        address: "10.0.0.1".into(),
        role: presentation::Role::Worker,
        etcd_member: true,
        responding: true,
        version: Some("v1.12.0".into()),
        cores: Some(8),
        memory: Some(presentation::Memory {
            used: 9216,
            total: 10240,
        }),
        load: Some([1., 2., 3.]),
        services: vec![talos_rs::ServiceInfo {
            id: "kubelet".into(),
            state: "Running".into(),
            health: Some(talos_rs::ServiceHealth {
                unknown: false,
                healthy: false,
                last_message: String::new(),
            }),
        }],
    };
    let summary = example::summary("prod-fra", 0);
    let mut kube = summary.nodes.loaded().unwrap()[0].clone();
    kube.name = talos.name.clone();
    kube.roles = vec!["master".into()];
    kube.conditions.truncate(1);
    kube.conditions[0].kind = "Ready".into();
    kube.conditions[0].status = "False".into();
    kube.conditions[0].reason = "KubeletNotReady".into();
    kube.conditions[0].message = "runtime unavailable".into();
    kube.addresses = vec![
        ("Hostname".into(), "joined.example".into()),
        ("InternalIP".into(), "10.0.0.2".into()),
    ];
    kube.capacity =
        serde_json::from_value(serde_json::json!({"cpu": "16", "memory": "16Gi"})).unwrap();
    kube.kubelet_version = "v1.35.0".into();
    kube.unschedulable = true;
    kube.taints = vec!["dedicated=infra:NoSchedule".into()];
    kube.pods = 7;
    (talos, kube)
}

#[test]
fn joined_row_preserves_display_contract_and_problem_order() {
    let (mut talos, kube) = projection_sources();
    talos.responding = false;
    let row = join::join(&[talos], &[kube], true, true).remove(0);
    assert_eq!(row.key.talos.as_deref(), Some("joined"));
    assert_eq!(row.key.kubernetes.as_deref(), Some("joined"));
    assert_eq!(row.id, "node-joined");
    assert_eq!(row.open_id, "node-joined-open");
    assert_eq!(row.name, "joined");
    assert_eq!(row.role, presentation::Role::ControlPlane);
    assert_eq!(row.address, "10.0.0.1");
    assert_eq!(row.tone, Tone::Crit);
    assert_eq!(row.note, "NotReady");
    assert_eq!(row.ready, "NotReady");
    assert_eq!(row.talos_state, "No response");
    assert_eq!(row.load, "1.00 · 2.00 · 3.00");
    assert_eq!(row.memory, "90% · 10.0 KB");
    assert_eq!(row.services, "—");
    assert!(row.service_problem);
    assert_eq!(row.pods, "7");
    assert_eq!(row.pod_label, "Pods 7");
    assert_eq!(row.kubelet_pods, "Pods on this node (7)");
    assert_eq!(
        row.problems,
        [
            "Kubernetes NotReady",
            "Talos API not answering",
            "kubelet unhealthy",
            "Memory 90%"
        ]
    );
    assert_eq!(
        row.chips,
        [
            "Control plane",
            "10.0.0.1",
            "Talos v1.12.0",
            "8 cores",
            "10.0 KB",
            "kubelet v1.35.0",
            "Cordoned"
        ]
    );
    let facts: Vec<_> = row
        .facts
        .iter()
        .map(|(key, value)| (key.as_ref(), value.as_ref()))
        .collect();
    assert_eq!(
        facts,
        [
            ("Kubernetes", "NotReady"),
            ("Talos", "No response"),
            ("Load", "1.00 · 2.00 · 3.00"),
            ("Memory", "90% · 10.0 KB"),
            ("System services", "—"),
            ("etcd member", "Yes"),
            ("Ready", "False · KubeletNotReady · runtime unavailable"),
            ("Hostname", "joined.example"),
            ("InternalIP", "10.0.0.2"),
            ("Capacity cpu", "16"),
            ("Capacity memory", "16Gi"),
            ("Taint", "dedicated=infra:NoSchedule"),
        ]
    );
}

#[test]
fn partial_rows_keep_source_labels_and_display_fallbacks() {
    let (mut talos, mut kube) = projection_sources();
    talos.version = None;
    talos.load = None;
    talos.memory = None;
    talos.services.clear();
    talos.role = presentation::Role::Unknown;
    for (available, ready) in [
        (false, "Kubernetes unavailable"),
        (true, "Not in Kubernetes"),
    ] {
        let row = join::join(&[talos.clone()], &[], true, available).remove(0);
        assert_eq!(row.ready, ready);
        assert_eq!(row.talos_state, "Version not reported");
        assert_eq!(row.role, presentation::Role::Unknown);
        assert_eq!(row.note, "worker");
        assert_eq!(row.tone, Tone::Good);
        assert_eq!(row.services, "0 healthy · 0 unhealthy");
        assert_eq!(row.load, "—");
        assert_eq!(row.memory, "—");
        assert_eq!(row.pods, "—");
        assert_eq!(row.pod_label, "Pods");
        assert_eq!(row.kubelet_pods, "Pods on this node");
        assert!(row.problems.is_empty());
        assert!(row.key.kubernetes.is_none());
    }
    kube.roles.clear();
    kube.conditions[0].status = "True".into();
    for (available, state) in [(false, "Talos unavailable"), (true, "No Talos data")] {
        let row = join::join(&[], &[kube.clone()], available, true).remove(0);
        assert_eq!(row.talos_state, state);
        assert_eq!(row.ready, "Ready");
        assert_eq!(row.role, presentation::Role::Worker);
        assert_eq!(row.address, "10.0.0.2");
        assert_eq!(row.tone, Tone::Good);
        assert_eq!(row.services, "—");
        assert_eq!(
            row.chips,
            [
                "Worker",
                "10.0.0.2",
                "16 cores",
                "16Gi memory",
                "kubelet v1.35.0",
                "Cordoned"
            ]
        );
        assert!(!row.service_problem);
        assert!(row.key.talos.is_none());
    }
    kube.addresses.pop();
    assert_eq!(
        join::join(&[], &[kube.clone()], false, true)[0].address,
        "joined.example"
    );
    kube.addresses.clear();
    assert_eq!(join::join(&[], &[kube.clone()], false, true)[0].address, "");
    talos.address.clear();
    kube.addresses
        .push(("InternalIP".into(), "10.0.0.2".into()));
    assert_eq!(join::join(&[talos], &[kube], true, true)[0].address, "");
}

#[test]
fn compact_note_tracks_health_priority_then_role() {
    let (mut talos, mut kube) = projection_sources();
    kube.conditions[0].status = "True".into();
    talos.responding = false;
    let project = |talos: &presentation::NodeSummary| {
        join::join(
            std::slice::from_ref(talos),
            std::slice::from_ref(&kube),
            true,
            true,
        )
        .remove(0)
    };
    assert_eq!(project(&talos).note, "No response");
    talos.responding = true;
    assert_eq!(project(&talos).note, "1 svc");
    assert_eq!(project(&talos).tone, Tone::Warn);
    assert_eq!(project(&talos).services, "0 healthy · 1 unhealthy");
    talos.services[0].health.as_mut().unwrap().unknown = true;
    assert_eq!(project(&talos).note, "90% · 10.0 KB");
    assert!(!project(&talos).service_problem);
    talos.memory.as_mut().unwrap().used -= 1;
    assert_eq!(project(&talos).note, "cp");
    assert_eq!(project(&talos).tone, Tone::Good);
}

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
        // Its unhealthy kubelet marks the Services tab with the warning glyph.
        assert!(window.try_find("node-services-problem").is_some());
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
        assert!(window.try_find("node-services-problem").is_none());
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
