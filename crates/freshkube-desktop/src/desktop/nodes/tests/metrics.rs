use super::*;

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
        let cells = super::super::resource::RowResources::new(
            &row,
            Some(&sample),
            stale,
            "read failed",
            true,
        );
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
    let cells = super::super::resource::RowResources::new(
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
    let stale = super::super::resource::RowResources::new(&row, None, false, "denied", false);
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
        (849, super::super::projection::Status::Healthy, vec![]),
        (
            850,
            super::super::projection::Status::Warning,
            vec!["Memory 85%"],
        ),
        (
            949,
            super::super::projection::Status::Warning,
            vec!["Memory 94%"],
        ),
        (
            950,
            super::super::projection::Status::Failing,
            vec!["Memory 95%"],
        ),
    ] {
        talos.memory = Some(presentation::Memory { used, total: 1000 });
        let row = join::join(&[talos.clone()], &[kube.clone()], true, true).remove(0);
        assert_eq!(
            super::super::projection::Status::of(&row),
            status,
            "used {used}"
        );
        let shown: Vec<&str> = row
            .problems
            .iter()
            .map(|problem| problem.as_ref())
            .collect();
        assert_eq!(shown, problems, "used {used}");
    }
}

#[gpui_kit::test]
fn hidden_columns_survive_summary_rebuild_and_metric_widths_stay_fixed(cx: &mut TestAppContext) {
    use super::super::table::Field;
    use freshkube_ui::table::TableColumn;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, _, cx| {
        pilot.update(cx, |pilot, cx| {
            let widths = |pilot: &super::super::Pilot| {
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
            pilot.rebuild_joined_nodes(cx);
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
