use super::{NodeTab, join};
use crate::{
    desktop::{Page, tests::fixture},
    fixture as examples, presentation,
    resources::example,
    ui::Tone,
};
use gpui_kit::{AppContext, Focusable, TestAppContext, test::TestWindowExt};

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
fn resource_cells_keep_request_use_allocatable_and_stale_inputs_for_shared_meters() {
    use freshkube_core::resources::{Amounts, NodeUsage};
    let (talos, mut kube) = projection_sources();
    kube.allocatable =
        serde_json::from_value(serde_json::json!({"cpu":"4","memory":"8Gi"})).unwrap();
    kube.requests = Amounts {
        cpu_millis: Some(1000.),
        memory_bytes: Some(1024. * 1024. * 1024.),
    };
    let row = join::join(&[talos], &[kube], true, true).remove(0);
    for (used, stale) in [(500., false), (1500., false), (3400., false), (3500., true)] {
        let sample = NodeUsage {
            name: row.name.to_string(),
            usage: Amounts {
                cpu_millis: Some(used),
                memory_bytes: Some(2. * 1024. * 1024. * 1024.),
            },
            sampled_at: None,
        };
        let cells =
            super::resource::RowResources::new(&row, Some(&sample), stale, "read failed", true);
        assert_eq!(cells.cpu.used, Some(used));
        assert_eq!(cells.cpu.request, Some(1000.));
        assert_eq!(cells.cpu.allocatable, Some(4000.));
        assert_eq!(cells.cpu.stale, stale);
        assert_eq!(
            cells.cpu.text,
            if used < 1000. {
                "500m"
            } else if used == 1500. {
                "1.5"
            } else if used == 3400. {
                "3.4"
            } else {
                "3.5"
            }
        );
        assert!(
            cells
                .cpu
                .tooltip
                .contains("requested 1.0 (25%) · allocatable 4.0")
        );
        assert!(
            cells
                .cpu
                .tooltip
                .contains("load averages: 1.00 · 2.00 · 3.00 (not CPU use)")
        );
        assert_eq!(
            cells.cpu.tooltip.contains("last known metrics.k8s.io"),
            stale
        );
        assert_eq!(
            cells
                .cpu
                .tooltip
                .contains("· at least 85% of allocatable ·"),
            used == 3400.
        );
        assert_eq!(
            cells.cpu.tooltip.contains("· above request ·"),
            used == 1500.
        );
        assert_eq!(cells.memory.text, "2.0Gi");
    }
}

#[test]
fn missing_node_metrics_fall_back_to_talos_memory_without_inventing_allocatable() {
    let (mut talos, mut kube) = projection_sources();
    talos.memory = Some(presentation::Memory {
        used: 3 * 1024 * 1024 * 1024,
        total: 4 * 1024 * 1024 * 1024,
    });
    kube.allocatable.clear();
    let row = join::join(&[talos], &[kube], true, true).remove(0);
    let cells = super::resource::RowResources::new(
        &row,
        None,
        false,
        "metrics-server isn't installed",
        true,
    );
    assert_eq!(cells.cpu.used, None);
    assert_eq!(cells.cpu.text, "—");
    assert!(cells.cpu.tooltip.contains("metrics-server isn't installed"));
    assert_eq!(cells.memory.used, Some(3. * 1024. * 1024. * 1024.));
    assert_eq!(cells.memory.text, "3.0Gi");
    assert_eq!(cells.memory.allocatable, None);
    assert!(cells.memory.tooltip.contains("Talos memory fallback"));
    assert!(cells.memory.tooltip.contains("physical total"));
    assert!(cells.memory.tooltip.contains("allocatable unknown"));
    let stale = super::resource::RowResources::new(&row, None, false, "denied", false);
    assert!(stale.memory.stale);
    assert!(stale.memory.tooltip.contains("last known Talos memory"));
}

#[gpui_kit::test]
fn node_metrics_are_hidden_owned_cancelled_and_retain_failed_answers(cx: &mut TestAppContext) {
    use freshkube_core::resources::{Amounts, NodeUsage};
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            assert!(!pilot.node_workspace.metrics.visible);
            assert!(pilot.node_workspace.metrics.delivery.is_none());
            assert!(pilot.node_workspace.metrics.job.is_none());
            pilot.navigate(Page::Nodes, window, cx);
            let metrics = &mut pilot.node_workspace.metrics;
            assert!(metrics.visible);
            assert!(metrics.delivery.is_some());
            let generation = metrics.generation;
            let request = metrics.snapshot.begin(metrics.source.clone().unwrap());
            assert!(metrics.apply(
                generation,
                &request,
                Ok(vec![
                    NodeUsage {
                        name: "talos-wk-fra1-02".into(),
                        usage: Amounts {
                            cpu_millis: Some(250.),
                            memory_bytes: Some(1048576.)
                        },
                        sampled_at: None
                    },
                    NodeUsage {
                        name: "old-source".into(),
                        usage: Amounts::default(),
                        sampled_at: None
                    }
                ])
            ));
            assert!(metrics.snapshot.data().unwrap().contains_key("old-source"));
            let request = metrics.snapshot.begin(metrics.source.clone().unwrap());
            assert!(metrics.apply(generation, &request, Err("denied".into())));
            assert_eq!(
                metrics.snapshot.data().unwrap()["talos-wk-fra1-02"]
                    .usage
                    .cpu_millis,
                Some(250.)
            );
            pilot.node_workspace.rebuild_resource_cells();
            let row = pilot
                .node_workspace
                .rows
                .iter()
                .find(|row| row.name == "talos-wk-fra1-02")
                .unwrap();
            let cells = &pilot.node_workspace.resource_cells[&row.key];
            assert!(cells.cpu.stale);
            assert_eq!(cells.memory.used, Some(1048576.)); // Last metrics outrank a new fallback.
            assert!(cells.memory.tooltip.contains("last known metrics.k8s.io"));
            let request = pilot
                .node_workspace
                .metrics
                .snapshot
                .begin(pilot.node_workspace.metrics.source.clone().unwrap());
            pilot.navigate(Page::Overview, window, cx);
            assert!(pilot.node_workspace.metrics.delivery.is_none());
            assert!(pilot.node_workspace.metrics.job.is_none());
            assert!(
                !pilot
                    .node_workspace
                    .metrics
                    .apply(generation, &request, Ok(vec![]))
            );
            pilot.navigate(Page::Nodes, window, cx);
            let metrics = &mut pilot.node_workspace.metrics;
            let generation = metrics.generation;
            let token = metrics.snapshot.begin(metrics.source.clone().unwrap());
            assert!(metrics.apply(
                generation,
                &token,
                Ok(vec![NodeUsage {
                    name: "old-source".into(),
                    usage: Amounts::default(),
                    sampled_at: None
                }])
            ));
            assert!(metrics.snapshot.data().unwrap().contains_key("old-source"));
            let old_id = metrics.source.clone();
            let request = metrics.snapshot.begin(metrics.source.clone().unwrap());
            pilot.applied.context = Some("different".into());
            pilot.push_source(window, cx);
            assert!(
                !pilot
                    .node_workspace
                    .metrics
                    .apply(generation, &request, Ok(vec![]))
            );
            assert_ne!(pilot.node_workspace.metrics.source, old_id);
            assert!(
                !pilot
                    .node_workspace
                    .metrics
                    .snapshot
                    .data()
                    .unwrap()
                    .contains_key("old-source")
            );
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn example_nodes_show_use_as_soon_as_the_page_shows(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Nodes, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    pilot.read_with(cx, |pilot, _| {
        let nodes = &pilot.node_workspace;
        let joined: Vec<_> = nodes
            .rows
            .iter()
            .filter(|row| row.kubernetes.is_some())
            .collect();
        assert!(!joined.is_empty());
        for row in joined {
            let cells = &nodes.resource_cells[&row.key];
            assert!(cells.cpu.used.is_some(), "{} has no CPU use", row.name);
            assert!(
                cells.memory.used.is_some(),
                "{} has no memory use",
                row.name
            );
            assert!(cells.cpu.allocatable.is_some(), "{}", row.name);
            let ready = row.kubernetes.as_ref().unwrap().is_ready();
            assert_eq!(cells.cpu.stale, !ready, "{}", row.name);
            if !ready {
                assert!(
                    cells.cpu.tooltip.contains("node is NotReady"),
                    "{}",
                    row.name
                );
            }
            for cell in [&cells.cpu, &cells.memory] {
                assert!(
                    cell.used <= cell.allocatable,
                    "{} uses more than it has",
                    row.name
                );
            }
        }
    });
}

#[gpui_kit::test]
fn hiding_or_replacing_nodes_source_drops_its_pending_job(cx: &mut TestAppContext) {
    let (runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.executor().allow_parking();
    for replace_context in [false, true] {
        let (job, receiver) = crate::backend::spawn_job(
            &runtime.handle().clone(),
            std::time::Duration::from_secs(60),
            "timeout".into(),
            std::future::pending::<Result<(), String>>(),
        );
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| {
                pilot.navigate(Page::Nodes, window, cx);
                pilot.node_workspace.metrics.job = Some(job);
                if replace_context {
                    pilot.applied.context = Some("replacement".into());
                    pilot.push_source(window, cx);
                } else {
                    pilot.navigate(Page::Overview, window, cx);
                }
                assert!(pilot.node_workspace.metrics.job.is_none());
                if !replace_context {
                    assert!(pilot.node_workspace.metrics.delivery.is_none());
                }
            });
        })
        .unwrap();
        assert!(
            runtime.block_on(receiver).is_err(),
            "the workspace must abort the pending job"
        );
    }
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Overview, window, cx));
    })
    .unwrap();
    let before = pilot.read_with(cx, |pilot, _| pilot.node_workspace.metrics.generation);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(45));
    cx.run_until_parked();
    pilot.read_with(cx, |pilot, _| {
        let metrics = &pilot.node_workspace.metrics;
        assert_eq!(metrics.generation, before);
        assert!(metrics.job.is_none());
        assert!(metrics.delivery.is_none());
    });
}

#[test]
fn a_nodes_memory_level_picks_its_group() {
    let (mut talos, mut kube) = projection_sources();
    kube.conditions[0].status = "True".into();
    talos.services.clear();
    for (used, status, problems) in [
        (849, super::projection::Status::Healthy, vec![]),
        (850, super::projection::Status::Warning, vec!["Memory 85%"]),
        (949, super::projection::Status::Warning, vec!["Memory 94%"]),
        (950, super::projection::Status::Failing, vec!["Memory 95%"]),
    ] {
        talos.memory = Some(presentation::Memory { used, total: 1000 });
        let row = join::join(&[talos.clone()], &[kube.clone()], true, true).remove(0);
        assert_eq!(super::projection::Status::of(&row), status, "used {used}");
        let shown: Vec<&str> = row
            .problems
            .iter()
            .map(|problem| problem.as_ref())
            .collect();
        assert_eq!(shown, problems, "used {used}");
    }
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
    // Just under the memory level's High.
    let memory = talos.memory.as_mut().unwrap();
    memory.used = memory.total * 85 / 100 - 1;
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
        assert!(window.try_find("nodes-cards").is_none());
        window.click("node-expand", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.selected, selected);
        // Beside the pane, the table folds its healthy rows as it does
        // without one (#339).
        crate::desktop::tests::expand_healthy_nodes(window, cx);
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
fn kubernetes_node_has_only_its_supported_tabs_and_stacks_under_the_table_when_narrow(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{point, px};
    let (_runtime, handle, pilot) = fixture(cx, 760., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.nodes.clear();
            pilot.rebuild_joined_nodes();
            pilot.navigate(Page::Nodes, window, cx);
        });
        window.render_frame(cx);
        crate::desktop::tests::expand_healthy_nodes(window, cx);
        pilot.update(cx, |pilot, cx| {
            let key = pilot.node_workspace.rows[0].key.clone();
            pilot.open_node(key, window, cx);
        });
        window.render_frame(cx);
        // Narrow, the inspector opens under the table, which stays.
        assert!(window.try_find("node-back").is_none());
        let table = window.find("nodes-table-scroll").bounds();
        let pane = window.find("node-inspector").bounds();
        assert!(
            pane.top() >= table.bottom() - px(1.),
            "{pane:?} under {table:?}"
        );
        assert!(window.find("node-tab-pods").visible());
        assert!(window.try_find("node-tab-processes").is_none());
        window.click("node-tab-yaml", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.tab, NodeTab::Yaml);

        // Another row above opens its node on the same tab.
        pilot
            .read(cx)
            .node_workspace
            .page_scroll
            .set_offset(point(px(0.), px(0.)));
        window.render_frame(cx);
        let rows: Vec<_> = pilot.read(cx).node_workspace.rows.iter().cloned().collect();
        let other = rows[1..]
            .iter()
            .find(|row| {
                window
                    .try_find(row.id.clone())
                    .is_some_and(|row| row.visible())
            })
            .expect("another row shows above the inspector");
        window.click(other.id.clone(), cx);
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        assert_eq!(nodes.selected.as_ref(), Some(&other.key));
        assert!(nodes.open);
        assert_eq!(nodes.tab, NodeTab::Yaml);

        window.click("node-close", cx);
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

/// Enter on a node, a click on a pane tab, or a click in the pane moves
/// the keyboard into the node's pane, as Enter does on Resources (#340).
/// Escape then hands it back to the table with the pane open, and a second
/// Escape closes the pane.
#[gpui_kit::test]
fn enter_and_pane_clicks_move_the_keyboard_into_the_node_pane(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    let in_pane = |pilot: &gpui_kit::Entity<crate::desktop::Pilot>,
                   window: &gpui_kit::Window,
                   cx: &gpui_kit::App| {
        pilot
            .read(cx)
            .node_workspace
            .pane_focus
            .contains_focused(window, cx)
    };
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| window.focus(&pilot.node_focus, cx));
        window.press("down", cx);
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.open);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.open);
        assert_eq!(pilot.read(cx).node_workspace.tab, NodeTab::Overview);
        assert!(in_pane(&pilot, window, cx));
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.open);
        assert!(pilot.read(cx).node_focus.is_focused(window));
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.open);

        // A click on a row opens the pane and leaves the keyboard on the
        // table; a click on a tab moves it into the tab's list.
        crate::desktop::tests::expand_healthy_nodes(window, cx);
        window.click("node-talos-cp-fra1-01", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_focus.is_focused(window));
        window.click("node-tab-pods", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.tab, NodeTab::Pods);
        assert!(in_pane(&pilot, window, cx));

        // A click on the pane's content takes it from the table too.
        window.click("node-tab-overview", cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| window.focus(&pilot.node_focus, cx));
        window.click("node-pane", cx);
        window.render_frame(cx);
        assert!(in_pane(&pilot, window, cx));

        // From the filter, a click on a tab whose screen takes no keys
        // moves the keyboard to the tab strip, not leaving it in the filter.
        let query = pilot.read(cx).node_workspace.query.clone();
        window.focus(&query.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
        window.click("node-tab-overview", cx);
        window.render_frame(cx);
        assert!(in_pane(&pilot, window, cx));
        assert!(!query.read(cx).focus_handle(cx).is_focused(window));
        // The strip's mouse-down took it there; asked straight from the
        // filter, the pane takes it too, rather than leaving it in the
        // filter.
        window.focus(&query.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| pilot.focus_node_pane(window, cx));
        window.render_frame(cx);
        assert!(in_pane(&pilot, window, cx));
    })
    .unwrap();
}

/// Leaving a tab whose list or log holds the keyboard for one whose screen
/// takes no keys gives the keyboard to the tab strip, not to the previous
/// tab's handle, which isn't drawn any more; Escape then still reaches the
/// pane and hands the keyboard to the table (#343 review). A headless click
/// on a tab focuses the strip on its mouse-down first, so the switch is
/// asked straight, as More's items and Overview's links ask it.
#[gpui_kit::test]
fn a_tab_without_keys_takes_the_keyboard_from_the_previous_tab(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        crate::desktop::tests::expand_healthy_nodes(window, cx);
        window.click("node-talos-cp-fra1-01", cx);
        window.render_frame(cx);
    })
    .unwrap();
    for from in [NodeTab::Pods, NodeTab::Logs] {
        for to in [NodeTab::Overview, NodeTab::Services] {
            cx.update_window(handle, |_, window, cx| {
                pilot.update(cx, |pilot, cx| {
                    pilot.show_node_tab(from, window, cx);
                    pilot.focus_node_pane(window, cx);
                });
                window.render_frame(cx);
                let tab_strip = pilot.read(cx).node_workspace.tab_focus.clone();
                let pane = pilot.read(cx).node_workspace.pane_focus.clone();
                assert!(pane.contains_focused(window, cx), "{from:?}");
                assert!(!tab_strip.is_focused(window), "{from:?}");
                pilot.update(cx, |pilot, cx| {
                    pilot.show_node_tab(to, window, cx);
                    pilot.focus_node_pane(window, cx);
                });
                window.render_frame(cx);
                assert_eq!(pilot.read(cx).node_workspace.tab, to);
                assert!(tab_strip.is_focused(window), "{from:?} → {to:?}");
                window.press("escape", cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(pilot.read(cx).node_workspace.open, "{from:?} → {to:?}");
                assert!(
                    pilot.read(cx).node_focus.is_focused(window),
                    "{from:?} → {to:?}"
                );
            })
            .unwrap();
        }
    }
}

/// With the open node hidden by the filter, Down and Up move the pane to the
/// first and last rows the table shows (#339's selection rule).
#[gpui_kit::test]
fn arrows_from_a_hidden_open_node_move_to_the_visible_rows(cx: &mut TestAppContext) {
    use freshkube_ui::table::{Line, TableSource};
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    let hidden = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("nav-nodes", cx);
            window.render_frame(cx);
            crate::desktop::tests::expand_healthy_nodes(window, cx);
            window.click("node-talos-cp-fra1-01", cx);
            window.render_frame(cx);
            pilot.read(cx).node_workspace.selected.clone().unwrap()
        })
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        let query = pilot.read(cx).node_workspace.query.clone();
        window.focus(&query.read(cx).focus_handle(cx), cx);
        window.input("wk", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let visible: Vec<_> = {
            let pilot = pilot.read(cx);
            (0..pilot.line_count())
                .filter_map(|line| match pilot.line(line, cx)? {
                    Line::Row(row) => Some(row.key),
                    Line::Group(_) => None,
                })
                .collect()
        };
        assert!(
            visible.len() > 1 && !visible.contains(&hidden),
            "{visible:?}"
        );
        for (key, first) in [("down", true), ("up", false)] {
            pilot.update(cx, |pilot, cx| {
                pilot.open_node(hidden.clone(), window, cx);
                window.focus(&pilot.node_focus, cx);
            });
            window.render_frame(cx);
            assert_eq!(
                pilot.read(cx).node_workspace.selected.as_ref(),
                Some(&hidden)
            );
            window.press(key, cx);
            window.render_frame(cx);
            let expected = if first {
                visible.first()
            } else {
                visible.last()
            };
            let nodes = &pilot.read(cx).node_workspace;
            assert!(nodes.open, "{key}");
            assert_eq!(nodes.selected.as_ref(), expected, "{key}");
        }
    })
    .unwrap();
}

/// The Processes find in the node pane (#347): Escape clears it, then
/// hands the keyboard to the list, then steps back to the table with the
/// pane left open.
#[gpui_kit::test]
fn escape_steps_back_from_the_processes_find_in_the_node_pane(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        crate::desktop::tests::open_node_tab(window, cx, NodeTab::Processes);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| {
            pilot.active_screen().unwrap().focus(window, cx)
        });
        window.press("/", cx);
        for key in ["e", "t", "c", "d"] {
            window.press(key, cx);
        }
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    // Init, PID 1, is listed unless the find hides it.
    let init_shown = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        window.render_frame(cx);
        window.try_find(("process", 1usize)).is_some()
    };
    let escape = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| window.press("escape", cx))
            .unwrap();
        cx.run_until_parked();
    };
    cx.update_window(handle, |_, window, cx| {
        assert!(!init_shown(window, cx), "etcd hides init");
    })
    .unwrap();
    escape(cx);
    cx.update_window(handle, |_, window, cx| {
        assert!(init_shown(window, cx), "the find is clear");
        let nodes = &pilot.read(cx).node_workspace;
        assert!(nodes.open && nodes.tab == NodeTab::Processes);
        assert!(!pilot.read(cx).node_focus.is_focused(window));
    })
    .unwrap();
    escape(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .pane_focus
                .contains_focused(window, cx)
        );
        assert!(!pilot.read(cx).node_focus.is_focused(window));
    })
    .unwrap();
    escape(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.open);
        assert_eq!(pilot.read(cx).node_workspace.tab, NodeTab::Processes);
        assert!(pilot.read(cx).node_focus.is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn node_keys_expand_switch_tabs_and_step_back(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        crate::desktop::tests::expand_healthy_nodes(window, cx);
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
        // Escape steps back as on Resources (#334): the pane hands the
        // keyboard to the table and stays open on its tab.
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        assert!(nodes.open);
        assert_eq!(nodes.tab, NodeTab::Processes);
        assert!(pilot.read(cx).node_focus.is_focused(window));
        // Expanded from the table, the pane first gives the table its room
        // back.
        window.press("secondary-shift-enter", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.expanded);
        assert!(pilot.read(cx).node_focus.is_focused(window));
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        assert!(nodes.open && !nodes.expanded);
        assert!(pilot.read(cx).node_focus.is_focused(window));
        window.focus(
            &pilot
                .read(cx)
                .node_workspace
                .query
                .read(cx)
                .focus_handle(cx),
            cx,
        );
        window.input("fra1", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.query_text, "fra1");
        pilot.update(cx, |pilot, cx| window.focus(&pilot.node_focus, cx));
        // The table clears its filter, then closes the pane.
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.query_text.is_empty());
        assert!(pilot.read(cx).node_workspace.open);
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.open);
    })
    .unwrap();
}

/// Escape steps back from each kind of tab as on Resources (#334): the
/// tab's own levels first, then the keyboard to the table with the pane
/// left open, then the pane closes, keeping its node selected.
#[gpui_kit::test]
fn escape_steps_back_from_the_pods_yaml_and_logs_tabs(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    let press = |cx: &mut TestAppContext, key: &str| {
        cx.update_window(handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let pilot = pilot.read(cx);
            (
                pilot.node_workspace.open,
                pilot.node_focus.is_focused(window),
            )
        })
        .unwrap()
    };
    let show = |cx: &mut TestAppContext, tab: NodeTab| {
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| pilot.show_node_tab(tab, window, cx));
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let key = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("nav-nodes", cx);
            window.render_frame(cx);
            crate::desktop::tests::expand_healthy_nodes(window, cx);
            window.click("node-talos-cp-fra1-01", cx);
            window.render_frame(cx);
            pilot.read(cx).node_workspace.selected.clone().unwrap()
        })
        .unwrap();

    // Pods: the filter clears, then leaves; the list hands the keyboard
    // to the table.
    show(cx, NodeTab::Pods);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot
                .node_pods
                .update(cx, |pods, cx| pods.focus(window, cx))
        });
        window.press("/", cx);
        window.input("zz", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(press(cx, "escape"), (true, false), "the filter clears");
    assert_eq!(press(cx, "escape"), (true, false), "the filter leaves");
    assert_eq!(press(cx, "escape"), (true, true), "the list steps back");
    assert_eq!(
        pilot.read_with(cx, |p, _| p.node_workspace.tab),
        NodeTab::Pods
    );

    // YAML: find clears, then leaves; the document hands the keyboard to
    // the table.
    show(cx, NodeTab::Yaml);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot
                .node_workspace
                .document
                .update(cx, |pane, cx| pane.focus(window, cx))
        });
        window.press("secondary-f", cx);
        window.input("kind", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(press(cx, "escape"), (true, false), "find clears");
    assert_eq!(press(cx, "escape"), (true, false), "find leaves");
    assert_eq!(press(cx, "escape"), (true, true), "the document steps back");
    assert_eq!(
        pilot.read_with(cx, |p, _| p.node_workspace.tab),
        NodeTab::Yaml
    );

    // Logs: the log hands the keyboard to the table.
    show(cx, NodeTab::Logs);
    assert!(
        !cx.update_window(handle, |_, window, cx| {
            pilot.read(cx).node_focus.is_focused(window)
        })
        .unwrap()
    );
    assert_eq!(press(cx, "escape"), (true, true), "the log steps back");

    // The table closes the pane and keeps the node selected.
    assert_eq!(press(cx, "escape"), (false, true));
    pilot.read_with(cx, |pilot, _| {
        assert_eq!(pilot.node_workspace.selected.as_ref(), Some(&key));
        assert_eq!(pilot.selected_node.as_deref(), Some("talos-cp-fra1-01"));
    });
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

#[gpui_kit::test]
fn header_chips_filter_health_without_changing_the_talos_target(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let target = pilot.read(cx).selected_node.clone();
        let nodes = &pilot.read(cx).node_workspace;
        let not_ready = nodes
            .rows
            .iter()
            .filter(|row| row.ready == "NotReady")
            .count();
        let ready = nodes
            .rows
            .iter()
            .filter(|row| row.tone == Tone::Good)
            .count();
        let unknown = nodes
            .rows
            .iter()
            .filter(|row| row.tone == Tone::Unknown)
            .count();
        assert_eq!(
            window.find("nodes-tally-failing").label(),
            Some(format!("{not_ready} failing nodes").as_str())
        );
        assert_eq!(
            window.find("nodes-tally-healthy").label(),
            Some(format!("{ready} healthy nodes").as_str())
        );
        assert_eq!(
            window.find("nodes-tally-unknown").label(),
            Some(format!("{unknown} unknown nodes").as_str())
        );
        window.click("nodes-tally-failing", cx);
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        assert_eq!(nodes.lines.len(), not_ready);
        assert!(
            nodes
                .lines
                .iter()
                .all(|ix| nodes.rows[*ix].ready == "NotReady")
        );
        assert_eq!(pilot.read(cx).selected_node, target);
        assert!(!nodes.open);
        window.click("nodes-tally-failing", cx);
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        assert_eq!(nodes.lines.len(), nodes.rows.len());
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.view, super::NodeView::Cards);
        assert!(window.find("nodes-title").visible());
        assert_eq!(pilot.read(cx).selected_node, target);
    })
    .unwrap();
}

#[gpui_kit::test]
fn nodes_matches_the_shared_table_layout_at_default_and_large_text(cx: &mut TestAppContext) {
    use crate::desktop::layout_check::{self, TablePage};
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let before = pilot.read(cx).selected_node.clone();
        for text in [14., 20.] {
            crate::text_size::set(text, cx);
            window.render_frame(cx);
            layout_check::assert_table_page(
                window,
                cx,
                &TablePage {
                    page: "nodes-page",
                    title: "nodes-title",
                    title_text: "Nodes",
                    table: "nodes-table-scroll",
                    list: "nodes-list",
                },
            );
            assert_eq!(pilot.read(cx).selected_node, before);
            assert!(!pilot.read(cx).node_workspace.open);
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn nodes_columns_preserve_row_identity(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        crate::desktop::tests::expand_healthy_nodes(window, cx);
        assert_eq!(
            window.find("node-talos-cp-fra1-01").bounds().size.height,
            crate::ui::dp_px(26., window)
        );
        window.click("nodes-columns", cx);
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        let before_columns = nodes.columns.len();
        let services_menu = nodes
            .menu_columns
            .iter()
            .position(|(field, _)| *field == super::table::Field::Services)
            .unwrap();
        assert!(window.try_find(("nodes-sort", 8usize)).is_some());
        window.within("popup-menu").click(services_menu, cx);
        window.render_frame(cx);
        // The menu stays open for another column; the toolbar stays while a
        // node is open, so close it before clicking past it.
        window.click("nodes-columns", cx);
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
        assert!(window.try_find(("nodes-sort", 8usize)).is_none());
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .hidden_columns
                .contains(&super::table::Field::Services)
        );
        assert_eq!(
            pilot.read(cx).node_workspace.columns.len(),
            before_columns - 1
        );
        window.click("node-talos-cp-fra1-01", cx);
        window.render_frame(cx);
        let key = pilot.read(cx).node_workspace.selected.clone();
        assert_eq!(
            pilot.read(cx).selected_node.as_deref(),
            Some("talos-cp-fra1-01")
        );
        window.click("node-close", cx);
        window.render_frame(cx);
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        assert_eq!(window.find("node-talos-cp-fra1-01").selected(), Some(true));
        window.click("nodes-view-table", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.selected, key);
        assert_eq!(window.find("node-talos-cp-fra1-01").selected(), Some(true));
        assert!(window.try_find(("nodes-sort", 8usize)).is_none());
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .hidden_columns
                .contains(&super::table::Field::Services)
        );
        assert_eq!(
            pilot.read(cx).node_workspace.columns.len(),
            before_columns - 1
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn filtered_table_keys_select_visible_nodes_and_keep_the_pane_tabs(cx: &mut TestAppContext) {
    use freshkube_ui::table::TableSource;
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let target = pilot.read(cx).selected_node.clone();
        window.click("nodes-tally-failing", cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| window.focus(&pilot.node_focus, cx));
        window.press("down", cx);
        window.render_frame(cx);
        let row = pilot.read(cx).node_workspace.row().unwrap();
        assert_eq!(row.ready, "NotReady");
        assert_eq!(pilot.read(cx).selected_node, target);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.open);
        // The table beside the pane keeps its filter (#339): the failing
        // group's heading and its two rows.
        assert_eq!(pilot.read(cx).line_count(), 3);
        assert_eq!(pilot.read(cx).node_workspace.lines.len(), 2);
        assert_eq!(
            pilot.read(cx).selected_node.as_deref(),
            Some("talos-wk-fra1-02")
        );
        window.click("node-tab-processes", cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| pilot.step_joined_node(1, window, cx));
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.tab, NodeTab::Processes);
        assert_eq!(
            pilot.read(cx).selected_node.as_deref(),
            Some("talos-wk-fra1-03")
        );
        window.click("node-close", cx);
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).node_workspace.filter,
            Some(super::projection::Status::Failing)
        );
        assert_eq!(pilot.read(cx).node_workspace.lines.len(), 2);
        assert!(window.find("node-talos-wk-fra1-03").visible());
    })
    .unwrap();
}

/// With a node's pane open, the table follows the filter as it does
/// without one, and its tallies count the rows it shows (#339). A filter
/// that hides the open node keeps its pane and its selection.
#[gpui_kit::test]
fn the_filter_narrows_the_table_beside_an_open_pane(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    let type_filter = |cx: &mut TestAppContext, text: &str| {
        cx.update_window(handle, |_, window, cx| {
            let query = pilot.read(cx).node_workspace.query.clone();
            query.update(cx, |input, cx| input.set_value("", window, cx));
            window.focus(&query.read(cx).focus_handle(cx), cx);
            window.input(text, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    // The rows drawn, by name, and the tallies' total.
    let shown = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let nodes = &pilot.read(cx).node_workspace;
            let names: Vec<String> = nodes
                .rows
                .iter()
                .filter(|row| window.try_find(row.id.clone()).is_some())
                .map(|row| row.name.to_string())
                .collect();
            (names, nodes.counts.iter().sum::<usize>())
        })
        .unwrap()
    };
    let key = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("nav-nodes", cx);
            window.render_frame(cx);
            window.click("node-talos-wk-fra1-01", cx);
            window.render_frame(cx);
            assert!(pilot.read(cx).node_workspace.open);
            pilot.read(cx).node_workspace.selected.clone().unwrap()
        })
        .unwrap();

    type_filter(cx, "wk");
    let (names, total) = shown(cx);
    assert!(names.iter().all(|name| name.contains("wk")), "{names:?}");
    assert_eq!(names.len(), 3);
    assert_eq!(total, 3);

    // A filter that hides the open node keeps its pane and selection.
    type_filter(cx, "cp");
    let (names, total) = shown(cx);
    assert!(names.iter().all(|name| name.contains("cp")), "{names:?}");
    assert_eq!(names.len(), 3);
    assert_eq!(total, 3);
    let nodes = pilot.read_with(cx, |pilot, _| {
        (
            pilot.node_workspace.open,
            pilot.node_workspace.selected.clone(),
        )
    });
    assert_eq!(nodes, (true, Some(key)));
}

#[test]
fn health_tally_treats_stale_and_absent_sources_as_unknown() {
    use super::projection::Status;
    let (talos, kube) = projection_sources();
    let mut stale = join::join(
        std::slice::from_ref(&talos),
        std::slice::from_ref(&kube),
        true,
        false,
    );
    stale[0].tone = super::projection::assessed_tone(&stale[0], true);
    assert_eq!(Status::of(&stale[0]), Status::Unknown);
    let mut absent = join::join(std::slice::from_ref(&talos), &[], true, true);
    absent[0].tone = super::projection::assessed_tone(&absent[0], true);
    assert_eq!(Status::of(&absent[0]), Status::Unknown);
    let current = join::join(&[talos], &[kube], true, true);
    assert_eq!(Status::of(&current[0]), Status::Failing);
}

#[gpui_kit::test]
fn nodes_empty_states_keep_the_shared_header_and_keyboard_context(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        pilot.update(cx, |pilot, cx| {
            pilot.node_workspace.rows = std::sync::Arc::new(Vec::new());
            pilot.node_workspace.empty = Some(super::Empty::Loading);
            pilot.node_workspace.rebuild_lines();
            window.focus(&pilot.node_focus, cx);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("nodes-title").visible());
        assert!(window.find("nodes-loading").visible());
        assert!(window.try_find("nodes-table-scroll").is_none());
        window.press("down", cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.open);
        for (state, id, title) in [
            (super::Empty::Loaded, "nodes-empty", "No nodes reported"),
            (
                super::Empty::Failed("Read result".into()),
                "nodes-failed",
                "Nodes unavailable · Read result",
            ),
        ] {
            pilot.update(cx, |pilot, cx| {
                pilot.node_workspace.empty = Some(state.clone());
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.find(id).visible());
            assert_eq!(window.find(id).label(), Some(title));
            assert!(window.try_find("nodes-table-scroll").is_none());
            assert!(window.find("nodes-refresh").visible());
            if id == "nodes-failed" {
                assert!(window.find("nodes-retry").visible());
            }
        }
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        assert!(window.find("nodes-title").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn minimum_nodes_table_keeps_shared_padding_and_scaled_rows(cx: &mut TestAppContext) {
    use crate::desktop::layout_check::{self, TablePage};
    let (_runtime, handle, _pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        crate::text_size::set(20., cx);
        crate::desktop::tests::settle_header(window, cx);
        layout_check::assert_table_page(
            window,
            cx,
            &TablePage {
                page: "nodes-page",
                title: "nodes-title",
                title_text: "Nodes",
                table: "nodes-table-scroll",
                list: "nodes-list",
            },
        );
        for id in [
            "nodes-view-cards",
            "nodes-view-table",
            "nodes-filter",
            "nodes-tally-failing",
            "nodes-tally-warning",
            "nodes-tally-unknown",
            "nodes-tally-healthy",
            "nodes-columns",
            "nodes-refresh",
        ] {
            let control = window.find(id);
            assert!(control.visible(), "{id} is clipped");
            assert!(
                control.bounds().right() <= window.find("nodes-page").bounds().right(),
                "{id} extends beyond the page"
            );
        }
        // The chips take their own row, which leaves room for every control.
        assert!(window.try_find("nodes-more").is_none());
        assert!(
            window.find("nodes-chips").bounds().top()
                >= window.find("nodes-toolbar").bounds().bottom()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_folded_nodes_controls_do_what_the_controls_do(cx: &mut TestAppContext) {
    // At 760 the chips' own row leaves room for every control; at 600 the
    // last two fold.
    let (_runtime, handle, pilot) = fixture(cx, 600., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        crate::text_size::set(20., cx);
        crate::desktop::tests::settle_header(window, cx);
        // Columns at the page's default, and Refresh never marks the menu.
        assert!(window.try_find("nodes-more-dot").is_none());
        // Refresh reads again, as its button does.
        let tick = pilot.read(cx).fixture_tick;
        window.click("nodes-more", cx);
        window.render_frame(cx);
        // Columns, Refresh.
        window.within("popup-menu").click(1usize, cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).fixture_tick, tick + 1);
    })
    .unwrap();
    // The first menu finishes closing.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        // A column from the menu hides as the Columns menu hides it.
        let first = pilot.read(cx).node_workspace.menu_columns[0].0;
        window.click("nodes-more", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
        window.render_frame(cx);
        window
            .within("submenu")
            .within("popup-menu")
            .click(0usize, cx);
        window.render_frame(cx);
        assert!(
            pilot
                .read(cx)
                .node_workspace
                .hidden_columns
                .contains(&first)
        );
        // Off the page's default, the "…" counts the hidden columns.
        window.find("nodes-more-dot");
        let hidden = pilot.read(cx).node_workspace.hidden_columns.len();
        assert_eq!(hidden, 2);
        assert_eq!(
            window.find("nodes-more").label(),
            Some("More · 2 columns hidden")
        );
    })
    .unwrap();
}

/// Scrolled sideways, a node's glyph and name stay at the table's left edge
/// while the other columns pass under them, and each id still finds one
/// element: the name is moved, never copied. A pinned name still opens its
/// node, and the selection keeps its key.
#[gpui_kit::test]
fn a_sideways_scroll_keeps_each_node_name_in_view_once(cx: &mut TestAppContext) {
    use gpui_kit::{ElementId, ScrollDelta, point, px};
    // Narrow enough to scroll sideways, tall enough for several rows.
    let (_runtime, handle, pilot) = fixture(cx, 760., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        // At text size 20 the healthy group's toggle is below the fold.
        crate::desktop::tests::expand_healthy_nodes(window, cx);
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        let rows: Vec<(ElementId, String)> = (pilot.read(cx).node_workspace.rows.iter())
            .map(|row| (row.id.clone().into(), row.name.to_string()))
            .collect();
        let shown: Vec<_> = (rows.into_iter())
            .filter(|(id, _)| window.try_find(id.clone()).is_some())
            .collect();
        assert!(shown.len() >= 2, "{shown:?}");
        let left = |window: &mut gpui_kit::Window, row: &ElementId| {
            window
                .within(row.clone())
                .find("node-row-name")
                .bounds()
                .left()
        };
        let names: Vec<_> = shown.iter().map(|(row, _)| left(window, row)).collect();
        let header = window.find(("nodes-sort", 1usize)).bounds().left();
        // Column 2 starts at the pinned run's edge, where its label stays
        // once scrolled; column 3 starts clear of the scroll's 120.
        let edge = window.find(("nodes-sort", 2usize)).bounds().left();
        let role = window.find(("nodes-sort", 3usize)).bounds().left();
        assert!(role - edge > px(120.), "{:?}", role - edge);
        window.scroll(
            "nodes-table-scroll",
            ScrollDelta::Pixels(point(px(-120.), px(0.))),
            cx,
        );
        window.render_frame(cx);
        let moved = role - window.find(("nodes-sort", 3usize)).bounds().left();
        assert!((f32::from(moved) - 120.).abs() <= 1.5, "{moved:?}");
        // `find` fails on an id that resolves twice.
        assert!((window.find(("nodes-sort", 1usize)).bounds().left() - header).abs() <= px(1.5));
        for ((row, _), name) in shown.iter().zip(names) {
            assert!((left(window, row) - name).abs() <= px(1.5));
        }
        let (row, name) = shown[0].clone();
        window.within(row).click("node-row-name", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.open);
        assert_eq!(pilot.read(cx).selected_node.as_deref(), Some(name.as_str()));
    })
    .unwrap();
}

#[test]
fn incomplete_assessments_are_unknown_and_ready_service_or_memory_problems_warn() {
    let (mut talos, mut kube) = projection_sources();
    kube.conditions[0].status = "True".into();
    let warning = join::join(
        std::slice::from_ref(&talos),
        std::slice::from_ref(&kube),
        true,
        true,
    )
    .remove(0);
    assert_eq!(warning.ready, "Ready");
    assert_eq!(super::projection::assessed_tone(&warning, true), Tone::Warn);
    assert_eq!(
        super::projection::assessed_tone(&warning, false),
        Tone::Unknown
    );
    let kube_only = join::join(&[], std::slice::from_ref(&kube), false, true).remove(0);
    assert_eq!(
        super::projection::assessed_tone(&kube_only, false),
        Tone::Unknown
    );
    talos.services.clear();
    talos.memory.as_mut().unwrap().used = 100;
    let healthy = join::join(&[talos], std::slice::from_ref(&kube), true, true).remove(0);
    assert_eq!(super::projection::assessed_tone(&healthy, true), Tone::Good);
    kube.conditions[0].status = "False".into();
    let failing = join::join(&[], &[kube], false, true).remove(0);
    assert_eq!(
        super::projection::assessed_tone(&failing, false),
        Tone::Crit
    );
}

#[gpui_kit::test]
fn groups_tallies_and_glyphs_agree_and_arrows_skip_folded_healthy_rows(cx: &mut TestAppContext) {
    use super::projection::{Item, Status};
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        assert_eq!(nodes.counts, [2, 1, 0, 3]);
        assert_eq!(nodes.group_counts, nodes.counts);
        assert_eq!(nodes.items.first(), Some(&Item::Group(Status::Failing)));
        assert!(nodes.healthy_collapsed());
        assert_eq!(
            window.find("nodes-tally-warning").label(),
            Some("1 warning nodes")
        );
        assert_eq!(
            window.find("nodes-group-warning").label(),
            Some("Warning · 1 node")
        );
        assert_eq!(
            window.find("nodes-group-failing").label(),
            Some("Failing · 2 nodes")
        );
        assert!(window.try_find("node-talos-cp-fra1-01").is_none());
        pilot.update(cx, |pilot, cx| window.focus(&pilot.node_focus, cx));
        for name in ["talos-wk-fra1-02", "talos-wk-fra1-03", "talos-wk-fra1-01"] {
            window.press("down", cx);
            window.render_frame(cx);
            let nodes = &pilot.read(cx).node_workspace;
            assert_eq!(nodes.row().unwrap().name, name);
            assert!(nodes.row().unwrap().tone != Tone::Good);
            assert!(window.find(nodes.row().unwrap().id.clone()).visible());
        }
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).node_workspace.row().unwrap().name,
            "talos-wk-fra1-01"
        );
        window.press("up", cx);
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).node_workspace.row().unwrap().name,
            "talos-wk-fra1-03"
        );
        window.click("nodes-tally-warning", cx);
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        assert_eq!(nodes.lines.len(), 1);
        assert_eq!(nodes.rows[nodes.lines[0]].tone, Tone::Warn);
        assert_eq!(nodes.group_counts, [0, 1, 0, 0]);
        assert_eq!(nodes.counts, [2, 1, 0, 3]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn text_filter_and_cards_empty_state_preserve_selection_and_target(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let target = pilot.read_with(cx, |pilot, _| pilot.selected_node.clone());
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        assert!(window.find("nodes-filter").visible());
        let scope = window.find("nodes-scope");
        assert!(scope.visible());
        assert!(
            scope
                .path()
                .contains(&gpui_kit::ElementId::from("status-bar")),
            "Nodes' line is in the status bar"
        );
        let status = scope.label().unwrap_or_default().to_owned();
        for part in ["Example data", "Talos", "Kubernetes"] {
            assert!(status.contains(part), "{part} is missing from {status}");
        }
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        window.focus(
            &pilot
                .read(cx)
                .node_workspace
                .query
                .read(cx)
                .focus_handle(cx),
            cx,
        );
        window.input("not-an-observed-node", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.lines.len(), 0);
        assert_eq!(pilot.read(cx).node_workspace.counts, [0; 4]);
        assert_eq!(
            window.find("nodes-empty").label(),
            Some("No matching nodes")
        );
        assert!(window.try_find("nodes-cards").is_none());
        assert_eq!(pilot.read(cx).selected_node, target);
        assert!(pilot.read(cx).node_workspace.selected.is_none());
        // Escape clears the filter, keeping the keyboard there; a second
        // hands it to the cards.
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.query_text.is_empty());
        assert!(window.find("nodes-cards").visible());
        assert!(!pilot.read(cx).node_focus.is_focused(window));
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_focus.is_focused(window));
        window.focus(
            &pilot
                .read(cx)
                .node_workspace
                .query
                .read(cx)
                .focus_handle(cx),
            cx,
        );
        window.input("WoRkEr", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        assert_eq!(nodes.lines.len(), 3);
        assert_eq!(nodes.counts, [2, 1, 0, 0]);
        assert!(
            nodes
                .lines
                .iter()
                .all(|ix| nodes.rows[*ix].role == presentation::Role::Worker)
        );
        assert_eq!(pilot.read(cx).selected_node, target);
    })
    .unwrap();
}

#[gpui_kit::test]
fn hidden_columns_survive_summary_rebuild_and_metric_widths_stay_fixed(cx: &mut TestAppContext) {
    use super::table::Field;
    use freshkube_ui::table::TableColumn;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, _, cx| {
        pilot.update(cx, |pilot, _| {
            let widths = |pilot: &super::Pilot| {
                pilot
                    .node_workspace
                    .all_columns
                    .iter()
                    .filter(|column| matches!(column.label().as_ref(), "CPU" | "Memory" | "Load"))
                    .map(TableColumn::width)
                    .collect::<Vec<_>>()
            };
            let before = widths(pilot);
            pilot.node_workspace.hidden_columns.insert(Field::Services);
            pilot.node_workspace.show_columns();
            for node in &mut pilot.nodes {
                node.load = Some([12345.67; 3]);
                if let Some(memory) = node.memory.as_mut() {
                    memory.used = memory.total;
                }
            }
            pilot.rebuild_joined_nodes();
            assert_eq!(widths(pilot), before);
            assert!(
                pilot
                    .node_workspace
                    .hidden_columns
                    .contains(&Field::Services)
            );
            assert_eq!(
                pilot.node_workspace.columns.len(),
                pilot.node_workspace.all_columns.len() - 2
            );
            assert!(
                !pilot
                    .node_workspace
                    .columns
                    .iter()
                    .any(|column| column.label() == "Services")
            );
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn leaving_kubernetes_only_restores_talos_columns_without_waiting_for_a_summary(
    cx: &mut TestAppContext,
) {
    use freshkube_ui::table::TableColumn;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.kubernetes_only = Some(crate::desktop::kubernetes_only::KubernetesOnly::new(
                None, None,
            ));
            pilot.rebuild_joined_nodes();
            let labels = |pilot: &super::Pilot| {
                pilot
                    .node_workspace
                    .columns
                    .iter()
                    .map(|column| column.label().to_string())
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                labels(pilot),
                ["", "Name", "Role", "Kubernetes", "CPU", "Memory", "Pods"]
            );
            pilot.leave_kubernetes_only(window, cx);
            assert_eq!(
                labels(pilot),
                [
                    "",
                    "Name",
                    "Role",
                    "Kubernetes",
                    "Talos",
                    "CPU",
                    "Memory",
                    "Pods",
                    "Services"
                ]
            );
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn opening_a_folded_healthy_node_from_overview_expands_and_reveals_it_on_return(
    cx: &mut TestAppContext,
) {
    use freshkube_ui::table::TableSource;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 560.);
    cx.update_window(handle, |_, window, cx| {
        let key = pilot
            .read(cx)
            .node_workspace
            .rows
            .iter()
            .find(|row| row.name == "talos-cp-fra1-01")
            .unwrap()
            .key
            .clone();
        assert!(pilot.read(cx).node_workspace.healthy_collapsed());
        assert!(pilot.read(cx).line_of(&key).is_none());
        pilot.update(cx, |pilot, cx| {
            pilot.open_destination(
                crate::presentation::attention::Destination::Node(key.clone(), NodeTab::Overview),
                window,
                cx,
            )
        });
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.healthy_collapsed());
        assert_eq!(pilot.read(cx).node_workspace.selected.as_ref(), Some(&key));
        // Simulate a newer summary folding its now-healthy selected row while the pane is open.
        pilot.update(cx, |pilot, _| {
            pilot.node_workspace.healthy_open = false;
            pilot.node_workspace.rebuild_lines();
            pilot
                .node_workspace
                .table
                .reveal(0, gpui_kit::ScrollStrategy::Top);
        });
        assert!(pilot.read(cx).node_workspace.healthy_collapsed());
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.open);
        assert!(!pilot.read(cx).node_workspace.healthy_collapsed());
        assert_eq!(pilot.read(cx).node_workspace.selected.as_ref(), Some(&key));
        assert!(pilot.read(cx).line_of(&key).is_some());
        assert!(window.find("node-talos-cp-fra1-01").visible());
        assert_eq!(
            pilot.read(cx).selected_node.as_deref(),
            Some("talos-cp-fra1-01")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn node_logs_startup_opens_a_folded_node_without_a_table_click(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        assert!(pilot.read(cx).node_workspace.healthy_collapsed());
        pilot.update(cx, |pilot, cx| {
            pilot.startup_selection(Some("node-logs"), None, None, window, cx);
        });
        window.render_frame(cx);
        let view = pilot.read(cx);
        assert_eq!(view.page, Page::Nodes);
        assert!(view.node_workspace.open);
        assert_eq!(view.node_workspace.tab, NodeTab::Logs);
        assert_eq!(view.node_workspace.row().unwrap().name, "talos-cp-fra1-01");
        assert!(!view.node_workspace.healthy_collapsed());
        assert!(window.find("logs-viewport").visible());
        window.click("node-close", cx);
        window.render_frame(cx);
        assert!(window.find("node-talos-cp-fra1-01").visible());
    })
    .unwrap();
}

#[test]
fn service_status_uses_health_facts_and_keeps_unknown_and_stale_honest() {
    let (mut talos, kube) = projection_sources();
    let row = join::join(&[talos.clone()], std::slice::from_ref(&kube), true, true).remove(0);
    // Kubernetes NotReady makes the node failing; a service problem is a warning.
    assert_eq!(row.tone, Tone::Crit);
    assert_eq!(row.service_status.tone, Tone::Warn);
    assert_eq!(row.service_status.count, "1");
    assert_eq!(
        row.service_status.label,
        "System services: 0 healthy, 1 unhealthy, 0 unknown"
    );
    talos.services[0].health.as_mut().unwrap().healthy = true;
    let mut row = join::join(&[talos.clone()], &[kube], true, true).remove(0);
    // Display text is not a health source.
    row.services = "99 unhealthy".into();
    assert_eq!(row.service_status.tone, Tone::Good);
    assert_eq!(row.service_status.count, "1");
    assert_eq!(
        row.service_status.label,
        "System services: 1 healthy, 0 unhealthy, 0 unknown"
    );
    let stale = join::ServiceStatus::new(Some(&talos), false);
    assert_eq!(stale.tone, Tone::Unknown);
    assert_eq!(stale.count, "—");
    assert_eq!(
        stale.label,
        "System services: 1 healthy, 0 unhealthy, 0 unknown · last known"
    );
    talos.services[0].health = None;
    let unknown = join::ServiceStatus::new(Some(&talos), true);
    assert_eq!(unknown.tone, Tone::Unknown);
    assert_eq!(unknown.count, "1");
    assert_eq!(
        unknown.label,
        "System services: 0 healthy, 0 unhealthy, 1 unknown"
    );
    talos.services.clear();
    let empty = join::ServiceStatus::new(Some(&talos), true);
    assert_eq!(empty.tone, Tone::Unknown);
    assert_eq!(empty.count, "—");
    talos.responding = false;
    for source in [Some(&talos), None] {
        let absent = join::ServiceStatus::new(source, true);
        assert_eq!(absent.tone, Tone::Unknown);
        assert_eq!(absent.count, "—");
        assert_eq!(absent.label, "System services unavailable");
    }
}

#[gpui_kit::test]
fn default_columns_fit_without_sideways_scroll_when_healthy_is_folded_or_expanded(
    cx: &mut TestAppContext,
) {
    use freshkube_ui::table::TableColumn;
    use gpui_kit::{SharedString, TextRun, font, px};
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-nodes", cx);
        for text in [13., 14.] {
            crate::text_size::set(text, cx);
            window.render_frame(cx);
            for expanded in [false, true] {
                if pilot.read(cx).node_workspace.healthy_collapsed() == expanded {
                    window.click("nodes-healthy-toggle", cx);
                    window.render_frame(cx);
                }
                let viewport = window.find("nodes-table-scroll").bounds();
                let last = window.find(("nodes-sort", 8usize)).bounds();
                assert!(last.right() <= viewport.right() + px(1.),
                    "Services overflows at text {text}, expanded={expanded}: {last:?} in {viewport:?}");
                let nodes = &pilot.read(cx).node_workspace;
                assert!(crate::ui::dp_px(nodes.table_width, window) <= viewport.size.width,
                    "column width {} exceeds viewport {:?}", nodes.table_width, viewport);
                let name = window.find(("nodes-sort", 1usize)).bounds();
                for row in nodes.rows.iter() {
                    let resources = nodes.resource_cells.get(&row.key).unwrap();
                    for (label, value) in [("CPU", resources.cpu.text.as_ref()), ("Memory", resources.memory.text.as_ref())] {
                        let column = nodes.columns.iter().find(|column| column.label() == label).unwrap();
                        assert_eq!(column.width(), 124.);
                        let run = TextRun {len: value.len(), font: font(crate::ui::MONO_FONT),
                            color: Default::default(), background_color: None, underline: None, strikethrough: None};
                        let shaped = window.text_system().shape_line(SharedString::from(value.to_owned()),
                            crate::ui::dp_px(12.5, window), &[run], None).width;
                        assert!(shaped <= crate::ui::dp_px(48., window),
                            "{label} truncates {value} at text {text}");
                    }
                    if row.name.len() <= 16 {
                        assert!(crate::ui::dp_px(row.name.len() as f32 * 7.5 + 24., window) <= name.size.width);
                    }
                }
                let row = nodes.rows.iter().find(|row| row.name == "talos-wk-fra1-02").unwrap();
                let status = window.find((row.id.clone(), super::table::Field::Services as usize));
                assert!(status.visible());
                assert_eq!(status.role(), Some(gpui_kit::Role::Status));
                assert_eq!(status.label(), Some(row.service_status.label.as_ref()));
                assert!(status.bounds().right() <= viewport.right() + px(1.));
                assert_eq!(row.table_load.split('·').count(), 3);
            }
        }
    }).unwrap();
}

#[gpui_kit::test]
fn a_short_window_scrolls_the_frame_and_keeps_the_list_usable(cx: &mut TestAppContext) {
    use freshkube_ui::page::{PANE_PADDING_Y, SHORT_LIST_HEIGHT};
    use gpui_kit::{point, px};
    // At 560 high the header and a usable list fit; at 480 they don't.
    let (_runtime, handle, pilot) = fixture(cx, 760., 480.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        crate::text_size::set(20., cx);
        crate::desktop::tests::settle_header(window, cx);
        // The table's frame, header and legend run to the end of the
        // scrolled page; its hairlines sit outside the measured scroll.
        let least = crate::ui::dp_px(SHORT_LIST_HEIGHT, window) - px(2.);
        let scroll = pilot.read(cx).node_workspace.page_scroll.clone();
        assert!(scroll.max_offset().y > px(0.), "the frame doesn't scroll");
        let end = window.find("nodes-page").bounds().bottom() + scroll.max_offset().y;
        let table = window.find("nodes-table-scroll").bounds();
        assert!(
            end - table.top() >= least,
            "the table is squeezed to {table:?}"
        );
        assert!(window.find("nodes-list").bounds().size.height > px(0.));

        // Opening a node scrolls the frame down to its inspector, and
        // closing it starts the frame at the top again.
        scroll.set_offset(point(px(0.), px(0.)));
        pilot.update(cx, |pilot, cx| {
            let key = pilot.node_workspace.rows[0].key.clone();
            pilot.open_node(key, window, cx);
        });
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(
            scroll.max_offset().y > px(0.),
            "the open frame doesn't scroll"
        );
        assert_eq!(scroll.offset().y, -scroll.max_offset().y);
        scroll.set_offset(point(px(0.), -scroll.max_offset().y));
        pilot.update(cx, |pilot, cx| pilot.close_node(window, cx));
        window.render_frame(cx);
        assert_eq!(scroll.offset().y, px(0.));

        // The cards and their legend keep it too, inside their inset.
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        let end = window.find("nodes-page").bounds().bottom() + scroll.max_offset().y;
        let cards = window.find("nodes-cards").bounds();
        let inset = crate::ui::dp_px(2. * PANE_PADDING_Y, window);
        assert!(
            end - cards.top() >= least - inset,
            "the cards are squeezed to {cards:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_tall_window_keeps_the_frame_still_and_the_table_filling_it(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let scroll = pilot.read(cx).node_workspace.page_scroll.clone();
        assert_eq!(scroll.max_offset().y, px(0.));
        // The legend sits under the rows, at the page's foot.
        let page = window.find("nodes-page").bounds();
        let table = window.find("nodes-table-scroll").bounds();
        let list = window.find("nodes-list").bounds();
        assert!(
            list.bottom() <= table.bottom() + px(0.5),
            "{list:?} in {table:?}"
        );
        assert!(
            page.bottom() - table.bottom() < crate::ui::dp_px(60., window),
            "{table:?} in {page:?}"
        );
    })
    .unwrap();
}

/// The log rows drawn at least in part inside `bounds`, by their ids: the
/// view's generation, then each shown row's id.
fn log_rows_within(
    window: &mut gpui_kit::Window,
    cx: &gpui_kit::App,
    logs: &gpui_kit::Entity<crate::logs::LogPanel>,
    bounds: gpui_kit::Bounds<gpui_kit::Pixels>,
) -> usize {
    use gpui_kit::SharedString;
    let logs = logs.read(cx);
    let ids: Vec<u64> = (logs.visible_rows().iter())
        .map(|&ix| logs.row_id(ix))
        .collect();
    let mut rows = 0;
    for generation in 0..16 {
        for id in &ids {
            let id = SharedString::from(format!("log-line-{generation}-{id}"));
            if window
                .try_find(id)
                .is_some_and(|row| row.bounds().intersects(&bounds))
            {
                rows += 1;
            }
        }
    }
    rows
}

/// Stacked under the table at 760 and text size 20, Talos' toolbar takes
/// most of the inspector (#233), so the Logs body scrolls inside it: first
/// the controls, then at least three log lines. Expand is the way to more.
#[gpui_kit::test]
fn stacked_node_logs_scroll_their_body_to_the_controls_then_the_lines(cx: &mut TestAppContext) {
    use gpui_kit::{point, px};
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        pilot.update(cx, |pilot, cx| {
            pilot.startup_selection(Some("node-logs"), None, None, window, cx);
        });
        // The log view learns its width while drawing, then lays out again.
        for _ in 0..3 {
            window.render_frame(cx);
        }
        let table = window.find("nodes-table-scroll").bounds();
        let pane = window.find("node-inspector").bounds();
        assert!(
            pane.top() >= table.bottom() - px(1.),
            "{pane:?} under {table:?}"
        );
        let content = window.find("node-inspector-content").bounds();
        let body = pilot.read(cx).node_workspace.logs_scroll.clone();
        assert!(
            body.max_offset().y > px(0.),
            "the stacked log body doesn't scroll"
        );

        // At the top, the toolbar's controls show whole in the inspector.
        let toolbar = window.find("logs-toolbar").bounds();
        for id in ["logs-collection", "logs-search", "logs-follow"] {
            let control = window.find(id).bounds();
            assert!(
                control.top() >= toolbar.top() && control.bottom() <= toolbar.bottom(),
                "{id} at {control:?} is clipped by the toolbar at {toolbar:?}"
            );
            let shown = window.find(id).bounds();
            if shown.bottom() > content.bottom() {
                // Lower controls come into view as the body scrolls.
                let by = content.bottom() - shown.bottom();
                body.set_offset(point(px(0.), body.offset().y + by));
                window.render_frame(cx);
            }
            let shown = window.find(id).bounds();
            assert!(
                shown.top() >= content.top() - px(0.5)
                    && shown.bottom() <= content.bottom() + px(0.5),
                "{id} at {shown:?} can't be scrolled into {content:?}"
            );
        }

        // Scrolled on to the log, at least three of its lines show.
        let viewport = window.find("logs-viewport").bounds();
        let by = (content.top() - viewport.top()).max(-body.max_offset().y - body.offset().y);
        body.set_offset(point(px(0.), body.offset().y + by));
        window.render_frame(cx);
        let logs = pilot.read(cx).logs.clone();
        // Its entries wrap, so count lines of text, 1.5 rem each.
        let shown = window.find("logs-viewport").bounds().intersect(&content);
        let lines = shown.size.height / (window.rem_size() * 1.5);
        assert!(lines >= 3., "{lines} log lines show in {content:?}");
        assert!(
            log_rows_within(window, cx, &logs, shown) > 0,
            "no log rows in {shown:?}"
        );

        // A wheel over the log scrolls the log, not the body or the frame.
        let frame = pilot.read(cx).node_workspace.page_scroll.clone();
        let (scrolled, framed) = (body.offset().y, frame.offset().y);
        // At the part of the log that shows; its centre is below the fold.
        wheel(window, shown.center(), 120., cx);
        assert_eq!(
            body.offset().y,
            scrolled,
            "the log's wheel scrolled the body"
        );
        assert_eq!(
            frame.offset().y,
            framed,
            "the log's wheel scrolled the frame"
        );
        assert!(!logs.read(cx).following(), "the wheel didn't reach the log");

        // Another tab starts the body at the top again.
        pilot.update(cx, |pilot, cx| {
            pilot.show_node_tab(NodeTab::Overview, window, cx);
            pilot.show_node_tab(NodeTab::Logs, window, cx);
        });
        window.render_frame(cx);
        assert_eq!(body.offset().y, px(0.));

        // Expanded, the inspector fills the page and the log has more room.
        let stacked = content.size.height;
        assert!(frame.offset().y < px(0.), "the frame isn't scrolled down");
        window.click("node-expand", cx);
        for _ in 0..3 {
            window.render_frame(cx);
        }
        assert!(window.try_find("nodes-table-scroll").is_none());
        let expanded = window.find("node-inspector-content").bounds().size.height;
        assert!(
            expanded > stacked,
            "expanded {expanded:?}, stacked {stacked:?}"
        );
        // From the top of the frame, so its heading and Collapse show.
        assert_eq!(frame.offset().y, px(0.));
        let page = window.find("nodes-page").bounds();
        let title = window.find("node-pane-title").bounds();
        assert!(title.top() >= page.top(), "{title:?} above {page:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn node_logs_in_a_tall_window_keep_the_frame_still(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.startup_selection(Some("node-logs"), None, None, window, cx);
        });
        for _ in 0..3 {
            window.render_frame(cx);
        }
        let frame = pilot.read(cx).node_workspace.page_scroll.clone();
        assert_eq!(frame.max_offset().y, px(0.));
        // Nor does the log's body scroll inside the inspector.
        let body = pilot.read(cx).node_workspace.logs_scroll.clone();
        assert_eq!(body.max_offset().y, px(0.));
        let pane_bounds = window.find("node-pane").bounds();
        let viewport = window.find("logs-viewport").bounds();
        assert!(
            viewport.bottom() <= pane_bounds.bottom() + px(0.5),
            "{viewport:?} in {pane_bounds:?}"
        );
    })
    .unwrap();
}

/// The arrows walk the cards in order; one in a row out of view scrolls the
/// grid to that row. At 400 high the list is shorter than a row of cards.
#[gpui_kit::test]
fn arrows_scroll_the_card_grid_to_the_selected_cards_row(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 400.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        crate::desktop::tests::settle_header(window, cx);
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        assert_eq!(super::cards::card_columns(window), 3);
        let lines = pilot.read(cx).node_workspace.lines.len();
        assert!(lines > 3, "only {lines} cards: no second row");
        // The second row starts below the list, so isn't drawn.
        let fourth = pilot.read(cx).node_workspace.lines[3];
        let fourth = pilot.read(cx).node_workspace.rows[fourth].id.clone();
        assert!(window.try_find(fourth.clone()).is_none());
        pilot.update(cx, |pilot, cx| window.focus(&pilot.node_focus, cx));
        for _ in 0..4 {
            window.press("down", cx);
            window.render_frame(cx);
        }
        let row = pilot.read(cx).node_workspace.row().unwrap().id.clone();
        assert_eq!(row, fourth);
        assert!(window.find(fourth).visible());
    })
    .unwrap();
}

/// At 760 wide and the largest text, the cards are one to a row, as wide as
/// the list.
#[gpui_kit::test]
fn a_narrow_window_at_large_text_shows_one_card_to_a_row(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        crate::text_size::set(20., cx);
        crate::desktop::tests::settle_header(window, cx);
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        assert_eq!(super::cards::card_columns(window), 1);
        let list = window.find("nodes-cards").bounds();
        let first = pilot.read(cx).node_workspace.lines[0];
        let first = pilot.read(cx).node_workspace.rows[first].id.clone();
        let card = window.find(first).bounds();
        assert_eq!(card.left(), list.left());
        assert_eq!(card.right(), list.right());
    })
    .unwrap();
}

/// Hovers the point `at` picks from the first card's bounds and the list's, in
/// the Nodes cards (fixture 1280×560, text size 20), and
/// returns the first card's id and name and the pointer's position.
fn hover_first_card(
    cx: &mut TestAppContext,
    at: impl FnOnce(
        gpui_kit::Bounds<gpui_kit::Pixels>,
        gpui_kit::Bounds<gpui_kit::Pixels>,
    ) -> gpui_kit::Point<gpui_kit::Pixels>,
) -> (
    tokio::runtime::Runtime,
    gpui_kit::AnyWindowHandle,
    gpui_kit::SharedString,
    gpui_kit::SharedString,
    gpui_kit::Point<gpui_kit::Pixels>,
) {
    use gpui_kit::{InputEvent, MouseMoveEvent};
    let (runtime, handle, pilot) = fixture(cx, 1280., 560.);
    let (first, name, position) = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("nav-nodes", cx);
            crate::text_size::set(20., cx);
            crate::desktop::tests::settle_header(window, cx);
            window.click("nodes-view-cards", cx);
            window.render_frame(cx);
            let first = pilot.read(cx).node_workspace.lines[0];
            let row = &pilot.read(cx).node_workspace.rows[first];
            let (first, name) = (row.id.clone(), row.name.clone());
            let card = window.find(first.clone()).bounds();
            let position = at(card, window.find("nodes-cards").bounds());
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            (first, name, position)
        })
        .unwrap();
    (runtime, handle, first, name, position)
}

/// A wheel scroll of `y` at `position`, in ten steps with a frame after
/// each, as a trackpad sends it.
fn wheel(
    window: &mut gpui_kit::Window,
    position: gpui_kit::Point<gpui_kit::Pixels>,
    y: f32,
    cx: &mut gpui_kit::App,
) {
    use gpui_kit::{InputEvent, ScrollDelta, ScrollWheelEvent, TouchPhase, point, px};
    for step in 0..10 {
        window.dispatch_event(
            ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(0.), px(y / 10.))),
                touch_phase: if step == 0 {
                    TouchPhase::Started
                } else {
                    TouchPhase::Moved
                },
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    }
}

fn drawn(tooltip: &str) -> usize {
    freshkube_ui::tooltip::drawn(tooltip)
}

/// A card's tooltip hides when a wheel scroll moves another card under a
/// pointer that stays still.
#[gpui_kit::test]
fn a_wheel_scroll_hides_a_shown_card_tooltip(cx: &mut TestAppContext) {
    use gpui_kit::{point, px};
    let (_runtime, handle, _, name, position) =
        hover_first_card(cx, |card, _| card.origin + point(px(24.), px(24.)));
    // Past the tooltip's show delay.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let before = drawn(&name);
        window.render_frame(cx);
        assert!(drawn(&name) > before, "the card's tooltip never showed");
        // A row and its gap, so the next row's card is under the pointer.
        wheel(window, position, -400., cx);
        let before = drawn(&name);
        window.render_frame(cx);
        assert_eq!(drawn(&name), before, "the tooltip stayed after the scroll");
    })
    .unwrap();
}

/// A tooltip still waiting to show when a wheel scroll moves its card away
/// from a still pointer never shows over the card now there (#192).
#[gpui_kit::test]
fn a_card_tooltip_waiting_to_show_does_not_show_after_its_card_scrolls_away(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{point, px};
    // Near the bottom of what shows of the card: the list is shorter than
    // a card at this size, so a short scroll brings the next row's card
    // under the pointer while this one is still drawn.
    let (_runtime, handle, first, name, position) = hover_first_card(cx, |card, list| {
        point(
            card.origin.x + px(24.),
            card.bottom().min(list.bottom()) - px(10.),
        )
    });
    // Before the show delay ends.
    cx.update_window(handle, |_, window, cx| {
        wheel(window, position, -120., cx);
        let card = window.find(first).bounds();
        assert!(
            !card.contains(&position),
            "the card is still under the pointer: {card:?} {position:?}"
        );
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let before = drawn(&name);
        window.render_frame(cx);
        assert_eq!(
            drawn(&name),
            before,
            "the first card's tooltip showed over another card"
        );
    })
    .unwrap();
}

/// Opens the first control-plane node from the table.
fn open_first_node(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    window.render_frame(cx);
    window.click("nav-nodes", cx);
    window.render_frame(cx);
    crate::desktop::tests::expand_healthy_nodes(window, cx);
    window.click("node-talos-cp-fra1-01", cx);
    window.render_frame(cx);
}

/// DESIGN.md's inspector: beside the table at the default text size, under
/// it at 20, where the page is narrower than 900 dp; the table's toolbar
/// stays, and the tabs are the inspector's, 28 high.
#[gpui_kit::test]
fn a_node_opens_in_the_inspector_beside_the_table_or_under_it(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first_node(window, cx);
        assert!(pilot.read(cx).node_workspace.open);
        for text in [13., 20.] {
            crate::text_size::set(text, cx);
            window.render_frame(cx);
            crate::desktop::layout_check::assert_inspector(
                window,
                cx,
                "nodes-split",
                "nodes-table",
                "node-inspector",
                "node-pane-title",
            );
            let pane = window.find("node-inspector").bounds();
            let table = window.find("nodes-table-scroll").bounds();
            assert_eq!(
                pane.left() > table.left(),
                text == 13.,
                "{pane:?} by {table:?}"
            );
            assert!(window.find("nodes-title").visible());
            let tall = crate::ui::dp_px(freshkube_ui::inspector::TAB_HEIGHT, window);
            for tab in ["node-tab-overview", "node-tab-pods", "node-tab-logs"] {
                let height = window.find(tab).bounds().size.height;
                assert!((height - tall).abs() <= px(1.), "{tab} is {height:?} high");
            }
        }
    })
    .unwrap();
}

/// At the least window the inspector stacks under the table and shrinks it;
/// the opened row is revealed in the shrunk table, not the table it left.
#[gpui_kit::test]
fn the_opened_row_shows_whole_above_the_stacked_inspector(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        // Each node in turn, from the table as it was before: the rows
        // under the stacked inspector's least table must scroll to show.
        let rows: Vec<_> = (pilot.read(cx).node_workspace.rows.iter())
            .map(|row| (row.key.clone(), row.id.clone()))
            .collect();
        for (key, id) in rows {
            pilot.update(cx, |pilot, cx| pilot.open_node(key, window, cx));
            // As the app's frames run: the next-frame callbacks, then the
            // draw.
            for _ in 0..8 {
                let ran = window.simulate_next_frame(cx);
                window.render_frame(cx);
                if ran == 0 {
                    break;
                }
            }
            let pane = window.find("node-inspector").bounds();
            let list = window.find("nodes-list").bounds();
            assert!(pane.top() >= list.bottom() - px(0.5), "not stacked");
            let row = window.find(id.clone()).bounds();
            assert!(
                row.top() >= list.top() - px(0.5) && row.bottom() <= list.bottom() + px(0.5),
                "{id} at {row:?} in {list:?}"
            );
            pilot.update(cx, |pilot, cx| pilot.close_node(window, cx));
            window.render_frame(cx);
        }
    })
    .unwrap();
}

/// Beside the table the inspector is too narrow for a node's chips on one
/// line, so they wrap inside it rather than cut one mid-text.
#[gpui_kit::test]
fn the_node_chips_wrap_inside_a_narrow_inspector(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (_runtime, handle, _pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first_node(window, cx);
        window.render_frame(cx);
        let banner = window.find("node-inspector-banner").bounds();
        let chips = window.find("node-chips").bounds();
        assert!(
            chips.right() <= banner.right() + px(0.5),
            "{chips:?} past {banner:?}"
        );
        // More than one line of 20 dp chips.
        let line = crate::ui::dp_px(20., window);
        assert!(chips.size.height > line * 1.5, "one line: {chips:?}");
        let tabs = window.find("node-inspector-tabs").bounds();
        assert!(tabs.top() >= chips.bottom(), "{tabs:?} over {chips:?}");
    })
    .unwrap();
}

/// The inspector's width is Nodes' own and comes back when the app opens
/// again.
#[gpui_kit::test]
fn the_node_inspector_width_survives_reopening(cx: &mut TestAppContext) {
    use crate::navigation_file::NavigationFile;
    use gpui_kit::px;
    let directory = std::env::temp_dir().join(format!(
        "freshkube-nodes-width-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
    ));
    let preferences = directory.join("preferences.json");
    let options = || crate::GpuiOptions::fixture().with_preferences(Some(preferences.clone()));
    let (_runtime, handle, pilot) = crate::desktop::tests::mount(cx, options(), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first_node(window, cx);
        let state = pilot.read(cx).node_workspace.split.beside_state().clone();
        state.update(cx, |state, cx| {
            state.resize_panel(1, crate::ui::dp_px(560., window), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let reopened = NavigationFile::open(Some(&preferences));
    assert_eq!(reopened.inspector_width("nodes"), Some(560.));
    assert_eq!(reopened.inspector_width("resources"), None);

    let (_runtime, handle, _pilot) = crate::desktop::tests::mount(cx, options(), 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first_node(window, cx);
        let width = window.find("node-inspector").bounds().size.width;
        let expected = crate::ui::dp_px(560., window);
        assert!(
            (width - expected).abs() <= px(1.),
            "{width:?}, not {expected:?}"
        );
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&directory);
}

/// Beside the inspector at 1280 the table is narrower than its columns: it
/// scrolls sideways with its header, and each row's glyph and name stay
/// at its left edge, drawn once.
#[gpui_kit::test]
fn the_table_beside_the_inspector_scrolls_sideways_with_its_names_pinned(cx: &mut TestAppContext) {
    use gpui_kit::{ElementId, ScrollDelta, point, px};
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first_node(window, cx);
        let table = window.find("nodes-table-scroll").bounds();
        let columns = pilot.read(cx).node_workspace.table_width;
        assert!(
            crate::ui::dp_px(columns, window) > table.size.width,
            "the table fits its columns in {table:?}"
        );
        let rows: Vec<ElementId> = (pilot.read(cx).node_workspace.rows.iter())
            .map(|row| row.id.clone().into())
            .filter(|id: &ElementId| window.try_find(id.clone()).is_some())
            .collect();
        assert!(rows.len() >= 2, "{rows:?}");
        let name = |window: &mut gpui_kit::Window, row: &ElementId| {
            window
                .within(row.clone())
                .find("node-row-name")
                .bounds()
                .left()
        };
        let names: Vec<_> = rows.iter().map(|row| name(window, row)).collect();
        let header = window.find(("nodes-sort", 1usize)).bounds().left();
        let role = window.find(("nodes-sort", 3usize)).bounds().left();
        window.scroll(
            "nodes-table-scroll",
            ScrollDelta::Pixels(point(px(-120.), px(0.))),
            cx,
        );
        window.render_frame(cx);
        let moved = role - window.find(("nodes-sort", 3usize)).bounds().left();
        assert!((f32::from(moved) - 120.).abs() <= 1.5, "{moved:?}");
        // `find` fails on an id that resolves twice.
        assert!((window.find(("nodes-sort", 1usize)).bounds().left() - header).abs() <= px(1.5));
        for (row, left) in rows.iter().zip(names) {
            assert!((name(window, row) - left).abs() <= px(1.5));
            assert!(name(window, row) >= table.left());
        }
        assert!(window.find("node-inspector").visible());
    })
    .unwrap();
}

/// From the cards, a node opens with the roster, one to a row, beside it.
#[gpui_kit::test]
fn a_node_opened_from_the_cards_keeps_the_roster_beside_its_inspector(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        let key = pilot.read(cx).node_workspace.rows[0].key.clone();
        pilot.update(cx, |pilot, cx| pilot.open_node(key, window, cx));
        window.render_frame(cx);
        assert!(window.try_find("nodes-table-scroll").is_none());
        let roster = window.find("nodes-cards").bounds();
        let pane = window.find("node-inspector").bounds();
        assert!(pane.left() >= roster.right(), "{pane:?} beside {roster:?}");
        assert!(
            window
                .find("nodes-split")
                .bounds()
                .contains(&roster.center())
        );

        // The opened node's row is marked, and the mark follows another.
        let rows = pilot.read(cx).node_workspace.rows.clone();
        let marked = |window: &mut gpui_kit::Window| {
            let mark = window.find("nodes-roster-selected").bounds();
            rows.iter()
                .filter(|row| {
                    let element = window.find(row.id.clone());
                    // Inside the row's bottom hairline.
                    element.selected() == Some(true) && element.bounds().contains(&mark.center())
                })
                .map(|row| row.key.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(marked(window), vec![rows[0].key.clone()]);
        window.click(rows[1].id.clone(), cx);
        window.render_frame(cx);
        assert_eq!(marked(window), vec![rows[1].key.clone()]);
        assert_eq!(window.find(rows[0].id.clone()).selected(), Some(false));
    })
    .unwrap();
}

/// The node's Events and YAML draw in the inspector's content, in no card
/// of their own.
#[gpui_kit::test]
fn node_events_and_yaml_draw_without_a_card(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first_node(window, cx);
    })
    .unwrap();
    for tab in [NodeTab::Yaml, NodeTab::Events] {
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| pilot.show_node_tab(tab, window, cx));
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(pilot.read(cx).node_workspace.tab, tab);
            let content = window.find("node-inspector-content").bounds();
            let detail = window.find("resource-detail").bounds();
            assert!(
                content.contains(&detail.center()),
                "{detail:?} in {content:?}"
            );
            let content = content.scale(window.scale_factor());
            let card = window.painted_quads().into_iter().find(|quad| {
                let (b, w) = (quad.bounds, quad.border_widths);
                w.left.0 > 0.
                    && w.right.0 > 0.
                    && quad.corner_radii.top_left.0 > 0.
                    && b.size.width.0 >= content.size.width.0 * 0.9
                    && b.size.height.0 >= content.size.height.0 * 0.5
            });
            assert!(card.is_none(), "{tab:?} draws in a card: {card:#?}");
        })
        .unwrap();
    }
}

/// Kit rescales the inspector when the text size or the window changes and
/// says nothing, so the node's screens lay out by the width it has, not the
/// width it was last dragged to.
#[gpui_kit::test]
fn the_node_screens_follow_the_inspector_after_a_text_size_change(cx: &mut TestAppContext) {
    use crate::ui::dp_px;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        open_first_node(window, cx);
        let state = pilot.read(cx).node_workspace.split.beside_state().clone();
        state.update(cx, |state, cx| {
            state.resize_panel(1, dp_px(800., window), window, cx)
        });
        let check = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, what: &str| {
            for _ in 0..4 {
                window.render_frame(cx);
                if window.simulate_next_frame(cx) == 0 {
                    break;
                }
            }
            window.render_frame(cx);
            let pane = window.find("node-inspector").bounds().size.width / dp_px(1., window);
            let seen =
                crate::screens::embedded_width(window) + freshkube_ui::page::PANE_PADDING * 2.;
            assert!(
                (pane - seen).abs() <= 1.,
                "{what}: the inspector is {pane} dp, its screens lay out by {seen}"
            );
        };
        check(window, cx, "after the drag");
        crate::text_size::set(20., cx);
        check(window, cx, "at 20 px");
        crate::text_size::set(13., cx);
        check(window, cx, "back at 13 px");
    })
    .unwrap();
}

/// Expanded, stepping to another node keeps the frame at the top, so the
/// node's name and Collapse stay in view in a short window.
#[gpui_kit::test]
fn stepping_nodes_while_expanded_keeps_the_heading_in_view(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        let key = pilot.read(cx).node_workspace.rows[0].key.clone();
        pilot.update(cx, |pilot, cx| pilot.open_node(key, window, cx));
        window.render_frame(cx);
        window.click("node-expand", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.expanded);
        let frame = pilot.read(cx).node_workspace.page_scroll.clone();
        let first = pilot.read(cx).node_workspace.selected.clone();
        pilot.update(cx, |pilot, cx| pilot.step_joined_node(1, window, cx));
        for _ in 0..3 {
            window.render_frame(cx);
        }
        assert_ne!(pilot.read(cx).node_workspace.selected, first, "no step");
        assert_eq!(frame.offset().y, px(0.));
        let page = window.find("nodes-page").bounds();
        let title = window.find("node-pane-title").bounds();
        assert!(title.top() >= page.top(), "{title:?} above {page:?}");
        assert!(window.try_find("node-expand").is_some());
    })
    .unwrap();
}

/// Enter in a node log's search steps to the next match and Shift-Enter
/// to the previous one, with the keyboard kept in the search (#316): the
/// node workspace's own Enter doesn't take it.
#[gpui_kit::test]
fn enter_in_a_node_logs_search_steps_through_its_matches(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.startup_selection(Some("node-logs"), None, None, window, cx);
        });
        for _ in 0..3 {
            window.render_frame(cx);
        }
        window.click("logs-search", cx);
        window.input("cilium", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let mut seen = Vec::new();
    for key in ["enter", "enter", "shift-enter"] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let current = pilot.read(cx).logs.read(cx).current_match();
            assert_eq!(
                window.find("logs-search").focused(),
                Some(true),
                "{key} keeps the keyboard in the search"
            );
            seen.push(current.expect("a current match"));
        })
        .unwrap();
    }
    // Enter moves on, and Shift-Enter comes back.
    assert_ne!(seen[0], seen[1], "{seen:?}");
    assert_eq!(seen[2], seen[0], "{seen:?}");
}

#[gpui_kit::test]
fn enter_opens_a_node_from_the_list_and_presses_a_focused_button(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        // On the list, Enter opens the selected node.
        pilot.update(cx, |pilot, cx| window.focus(&pilot.node_focus, cx));
        window.press("down", cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.open);
        let node = pilot.read(cx).selected_node.clone();
        assert!(node.is_some());
        pilot.update(cx, |pilot, cx| {
            pilot.show_node_tab(NodeTab::Logs, window, cx)
        });
        window.render_frame(cx);
        // On a focused button in the inspector, Enter presses the button.
        window.click("logs-search", cx);
        window.press("tab", cx);
        window.render_frame(cx);
        assert_eq!(window.find("logs-wrap").focused(), Some(true));
        let wrapped = window.find("logs-wrap").checked();
        assert!(wrapped.is_some());
        window.press("enter", cx);
        window.render_frame(cx);
        assert_ne!(window.find("logs-wrap").checked(), wrapped);
        assert_eq!(window.find("logs-wrap").focused(), Some(true));
        assert_eq!(pilot.read(cx).selected_node, node);
        assert!(pilot.read(cx).node_workspace.open);
    })
    .unwrap();
}

/// Enter in the filter hands the keyboard to the list without opening a
/// node, as the Resources filter does; Enter on the list then opens one.
#[gpui_kit::test]
fn enter_in_the_filter_moves_to_the_list_and_opens_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        window.click("nodes-filter", cx);
        window.input("wk", cx);
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(pilot.read(cx).node_focus.is_focused(window));
        assert!(!pilot.read(cx).node_workspace.open);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.open);
        let node = pilot.read(cx).selected_node.clone().unwrap_or_default();
        assert!(node.contains("-wk-"), "{node} isn't a filtered node");
    })
    .unwrap();
}

#[gpui_kit::test]
fn h_folds_the_healthy_nodes_from_the_list_and_the_filter_types_it(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.healthy_collapsed());
        pilot.update(cx, |pilot, cx| window.focus(&pilot.node_focus, cx));
        window.press("h", cx);
        window.render_frame(cx);
        assert!(!pilot.read(cx).node_workspace.healthy_collapsed());
        assert!(window.find("node-talos-cp-fra1-01").visible());
        window.press("h", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.healthy_collapsed());

        window.click("nodes-filter", cx);
        window.render_frame(cx);
        window.press("h", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(pilot.read(cx).node_workspace.query_text, "h"));
}

#[gpui_kit::test]
fn h_leaves_the_fold_alone_on_the_cards(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        window.click("nodes-view-cards", cx);
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).node_workspace.view, super::NodeView::Cards);
        assert!(pilot.read(cx).node_workspace.healthy_collapsed());
        pilot.update(cx, |pilot, cx| window.focus(&pilot.node_focus, cx));
        window.press("h", cx);
        window.render_frame(cx);
        assert!(pilot.read(cx).node_workspace.healthy_collapsed());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_right_click_selects_its_node_and_its_menu_folds_the_healthy_ones(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-nodes", cx);
        window.render_frame(cx);
        window.right_click("node-talos-wk-fra1-03", cx);
    })
    .unwrap();
    // The menu builds on the next frame.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let nodes = &pilot.read(cx).node_workspace;
        assert_eq!(nodes.row().unwrap().name, "talos-wk-fra1-03");
        // Selected as an arrow selects: the closed pane stays closed.
        assert!(!nodes.open);
        let menu = window.within("popup-menu");
        let items: Vec<_> = (0usize..)
            .map_while(|ix| menu.try_find(ix))
            .map(|item| item.label().map(str::to_owned))
            .collect();
        assert_eq!(
            items,
            [
                Some("Open".to_owned()),
                None,
                Some("Expand healthy nodes".to_owned())
            ]
        );
        window.within("popup-menu").click(2usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert!(!pilot.read(cx).node_workspace.healthy_collapsed()));
}
