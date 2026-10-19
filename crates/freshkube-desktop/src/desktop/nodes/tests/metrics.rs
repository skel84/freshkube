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
            let nodes = pilot.pages.node_workspace.clone();
            nodes.read_with(cx, |nodes, _| {
                assert!(!nodes.metrics.visible);
                assert!(!nodes.metrics.snapshot.is_visible());
                assert!(!nodes.metrics.snapshot.is_loading());
            });
            pilot.navigate(Page::Nodes, window, cx);
            let (generation, request) = nodes.update(cx, |nodes, cx| {
                let metrics = &mut nodes.metrics;
                assert!(metrics.visible);
                assert!(metrics.snapshot.is_visible());
                let generation = metrics.snapshot.revision();
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
                    ]),
                    cx
                ));
                assert!(metrics.snapshot.data().unwrap().contains_key("old-source"));
                let request = metrics.snapshot.begin(metrics.source.clone().unwrap());
                assert!(metrics.apply(
                    metrics.snapshot.revision(),
                    &request,
                    Err("denied".into()),
                    cx
                ));
                assert_eq!(
                    metrics.snapshot.data().unwrap()["talos-wk-fra1-02"]
                        .usage
                        .cpu_millis,
                    Some(250.)
                );
                nodes.rebuild_resource_cells();
                let row = nodes
                    .rows
                    .iter()
                    .find(|row| row.name == "talos-wk-fra1-02")
                    .unwrap();
                let cells = &nodes.resource_cells[&row.key];
                assert!(cells.cpu.stale);
                assert_eq!(cells.memory.used, Some(1048576.)); // Last metrics outrank a new fallback.
                assert!(cells.memory.tooltip.contains("last known metrics.k8s.io"));
                let request = nodes
                    .metrics
                    .snapshot
                    .begin(nodes.metrics.source.clone().unwrap());
                (nodes.metrics.snapshot.revision(), request)
            });
            pilot.navigate(Page::Overview, window, cx);
            nodes.update(cx, |nodes, cx| {
                assert!(!nodes.metrics.snapshot.is_visible());
                assert!(!nodes.metrics.snapshot.is_loading());
                assert!(!nodes.metrics.apply(generation, &request, Ok(vec![]), cx));
            });
            pilot.navigate(Page::Nodes, window, cx);
            let (generation, old_id, request) = nodes.update(cx, |nodes, cx| {
                let metrics = &mut nodes.metrics;
                let generation = metrics.snapshot.revision();
                let token = metrics.snapshot.begin(metrics.source.clone().unwrap());
                assert!(metrics.apply(
                    generation,
                    &token,
                    Ok(vec![NodeUsage {
                        name: "old-source".into(),
                        usage: Amounts::default(),
                        sampled_at: None
                    }]),
                    cx
                ));
                assert!(metrics.snapshot.data().unwrap().contains_key("old-source"));
                let old_id = metrics.source.clone();
                let request = metrics.snapshot.begin(metrics.source.clone().unwrap());
                (metrics.snapshot.revision(), old_id, request)
            });
            pilot.applied.context = Some("different".into());
            pilot.push_source(window, cx);
            nodes.update(cx, |nodes, cx| {
                assert!(!nodes.metrics.apply(generation, &request, Ok(vec![]), cx));
                assert_ne!(nodes.metrics.source, old_id);
                assert!(
                    !nodes
                        .metrics
                        .snapshot
                        .data()
                        .unwrap()
                        .contains_key("old-source")
                );
            });
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
    pilot.read_with(cx, |pilot, cx| {
        let nodes = &pilot.pages.node_workspace.read(cx);
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
        let (delivery, mut delivered) = tokio::sync::oneshot::channel::<()>();
        let (job, receiver) = crate::backend::spawn_job(
            &runtime.handle().clone(),
            std::time::Duration::from_secs(60),
            "timeout".into(),
            std::future::pending::<Result<(), String>>(),
        );
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| {
                pilot.navigate(Page::Nodes, window, cx);
                pilot.pages.node_workspace.update(cx, |nodes, cx| {
                    let source = nodes.metrics.source.clone().unwrap();
                    nodes.metrics.snapshot.start(source);
                    let task = cx.spawn(async move |_, _| {
                        let _delivery = delivery;
                        std::future::pending().await
                    });
                    nodes.metrics.snapshot.run(Some(job), task);
                });
                if replace_context {
                    pilot.applied.context = Some("replacement".into());
                    pilot.push_source(window, cx);
                } else {
                    pilot.navigate(Page::Overview, window, cx);
                }
                assert!(
                    !pilot
                        .pages
                        .node_workspace
                        .read(cx)
                        .metrics
                        .snapshot
                        .is_loading()
                );
                if !replace_context {
                    assert!(
                        !pilot
                            .pages
                            .node_workspace
                            .read(cx)
                            .metrics
                            .snapshot
                            .is_visible()
                    );
                }
            });
        })
        .unwrap();
        cx.run_until_parked();
        assert!(
            matches!(
                delivered.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Closed)
            ),
            "the workspace must drop the delivery task"
        );
        assert!(
            runtime.block_on(receiver).is_err(),
            "the workspace must abort the pending job"
        );
    }
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Overview, window, cx));
    })
    .unwrap();
    let before = pilot.read_with(cx, |pilot, cx| {
        pilot
            .pages
            .node_workspace
            .read(cx)
            .metrics
            .snapshot
            .revision()
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(45));
    cx.run_until_parked();
    pilot.read_with(cx, |pilot, cx| {
        let metrics = &pilot.pages.node_workspace.read(cx).metrics;
        assert_eq!(metrics.snapshot.revision(), before);
        assert!(!metrics.snapshot.is_loading());
        assert!(!metrics.snapshot.is_visible());
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
            let widths = |pilot: &super::super::Pilot, cx: &gpui_kit::App| {
                pilot
                    .pages
                    .node_workspace
                    .read(cx)
                    .all_columns
                    .iter()
                    .filter(|column| matches!(column.label().as_ref(), "CPU" | "Memory" | "Load"))
                    .map(TableColumn::width)
                    .collect::<Vec<_>>()
            };
            let before = widths(pilot, cx);
            pilot.pages.node_workspace.update(cx, |nodes, _| {
                nodes.hidden_columns.insert(Field::Services);
            });
            pilot.pages.node_workspace.update(cx, |nodes, _| {
                nodes.show_columns();
            });
            for node in &mut pilot.nodes {
                node.load = Some([12345.67; 3]);
                if let Some(memory) = node.memory.as_mut() {
                    memory.used = memory.total;
                }
            }
            pilot.rebuild_joined_nodes(cx);
            assert_eq!(widths(pilot, cx), before);
            assert!(
                pilot
                    .pages
                    .node_workspace
                    .read(cx)
                    .hidden_columns
                    .contains(&Field::Services)
            );
            assert_eq!(
                pilot.pages.node_workspace.read(cx).columns.len(),
                pilot.pages.node_workspace.read(cx).all_columns.len() - 2
            );
            assert!(
                !pilot
                    .pages
                    .node_workspace
                    .read(cx)
                    .columns
                    .iter()
                    .any(|column| column.label() == "Services")
            );
        });
    })
    .unwrap();
}

/// The node-usage read follows Nodes' page, not the pane: opening a node
/// and moving between its tabs, a Talos screen's included, neither stops
/// nor restarts it, and leaving Nodes stops it. The read runs from Nodes'
/// `set_visible` and `set_page_source`; this pins that schedule.
#[gpui_kit::test]
fn node_metrics_keep_reading_while_the_pane_shows_a_screen(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let read = |cx: &mut TestAppContext| {
        pilot.read_with(cx, |pilot, cx| {
            let metrics = &pilot.pages.node_workspace.read(cx).metrics;
            (
                metrics.visible,
                (metrics.snapshot.revision(), metrics.snapshot.is_visible()),
            )
        })
    };
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Nodes, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let (visible, generation) = read(cx);
    assert!(visible);
    for tab in [NodeTab::Processes, NodeTab::Overview, NodeTab::Storage] {
        cx.update_window(handle, |_, window, cx| {
            crate::desktop::tests::open_node_tab(window, cx, tab)
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(read(cx), (true, generation), "on {tab:?}");
    }
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Etcd, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let (visible, after) = read(cx);
    assert!(!visible);
    assert_ne!(after, generation, "leaving Nodes stops the read");
}

/// Node usage reads while Nodes shows, whatever its pane does, and drops
/// its read the moment another page shows (#617's review, item 2).
#[gpui_kit::test]
fn node_usage_follows_the_nodes_page_through_its_pane(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            let reading = |pilot: &super::super::Pilot, cx: &gpui_kit::App| {
                let metrics = &pilot.pages.node_workspace.read(cx).metrics;
                // Hidden, it holds no read and no delivery.
                if !metrics.visible {
                    assert!(!metrics.snapshot.is_visible() && !metrics.snapshot.is_loading());
                }
                metrics.visible
            };
            let rows = pilot.pages.node_workspace.read(cx).rows.clone();
            let (first, second) = (rows[0].key.clone(), rows[1].key.clone());
            let mut steps = Vec::new();
            pilot.navigate(Page::Nodes, window, cx);
            pilot.open_node(first, window, cx);
            pilot.show_node_tab(NodeTab::Processes, window, cx);
            steps.push(("processes", reading(pilot, cx)));
            pilot.open_node(second, window, cx);
            assert_eq!(pilot.pages.node_workspace.read(cx).tab, NodeTab::Processes);
            steps.push(("another node on processes", reading(pilot, cx)));
            pilot.show_node_tab(NodeTab::Logs, window, cx);
            steps.push(("logs", reading(pilot, cx)));
            pilot.close_node(window, cx);
            steps.push(("closed", reading(pilot, cx)));
            pilot.navigate(Page::Etcd, window, cx);
            steps.push(("etcd", reading(pilot, cx)));
            pilot.navigate(Page::Nodes, window, cx);
            steps.push(("back after etcd", reading(pilot, cx)));
            pilot.navigate(Page::Overview, window, cx);
            steps.push(("overview", reading(pilot, cx)));
            let name = rows[0].name.to_string();
            pilot.open_node_by_name(&name, NodeTab::Storage, window, cx);
            steps.push(("opened from overview", reading(pilot, cx)));
            assert_eq!(
                steps,
                [
                    ("processes", true),
                    ("another node on processes", true),
                    ("logs", true),
                    ("closed", true),
                    ("etcd", false),
                    ("back after etcd", true),
                    ("overview", false),
                    ("opened from overview", true),
                ]
            );
        });
    })
    .unwrap();
}

/// The loader owns the poll's wait as well as its delivery. Arriving fixture
/// rows update the samples without moving the next poll; repeated polls keep
/// running, and hiding drops the wait before showing reads immediately again.
#[gpui_kit::test]
fn node_metrics_keep_the_poll_interval_when_example_rows_arrive(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Nodes, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let revision = |cx: &mut TestAppContext| {
        pilot.read_with(cx, |pilot, cx| {
            pilot
                .pages
                .node_workspace
                .read(cx)
                .metrics
                .snapshot
                .revision()
        })
    };
    let first = revision(cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(14));
    cx.run_until_parked();
    assert_eq!(revision(cx), first);
    cx.update_window(handle, |_, _, cx| {
        pilot
            .read(cx)
            .pages
            .node_workspace
            .clone()
            .update(cx, |nodes, _| {
                let row = std::sync::Arc::make_mut(&mut nodes.rows)
                    .iter_mut()
                    .find(|row| row.kubernetes.is_some())
                    .unwrap();
                let node = row.kubernetes.as_mut().unwrap();
                node.requests.cpu_millis = Some(1.);
                let name = node.name.clone();
                nodes.refresh_example_metrics();
                let sample = &nodes.metrics.snapshot.data().unwrap()[&name];
                assert_eq!(sample.usage.cpu_millis, Some(75.));
            });
    })
    .unwrap();
    assert_eq!(revision(cx), first);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    let second = revision(cx);
    assert!(second > first);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(15));
    cx.run_until_parked();
    assert!(revision(cx) > second);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Overview, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let hidden = revision(cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(45));
    cx.run_until_parked();
    assert_eq!(revision(cx), hidden);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Nodes, window, cx));
    })
    .unwrap();
    assert!(revision(cx) > hidden);
}

#[gpui_kit::test]
fn a_late_node_metrics_answer_is_rejected_after_returning_to_the_same_source(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.navigate(Page::Nodes, window, cx);
            let nodes = pilot.pages.node_workspace.clone();
            let (revision, request, source) = nodes.update(cx, |nodes, _| {
                let metrics = &mut nodes.metrics;
                let request = metrics.snapshot.begin(metrics.source.clone().unwrap());
                (metrics.snapshot.revision(), request, metrics.source.clone())
            });
            let original = pilot.applied.context.clone();
            pilot.applied.context = Some("replacement".into());
            pilot.push_source(window, cx);
            pilot.applied.context = original;
            pilot.push_source(window, cx);
            nodes.update(cx, |nodes, cx| {
                let metrics = &mut nodes.metrics;
                assert_eq!(metrics.source, source);
                let current = metrics.snapshot.begin(metrics.source.clone().unwrap());
                assert!(!metrics.apply(revision, &request, Ok(vec![]), cx));
                assert!(metrics.apply(metrics.snapshot.revision(), &current, Ok(vec![]), cx));
            });
        });
    })
    .unwrap();
}
