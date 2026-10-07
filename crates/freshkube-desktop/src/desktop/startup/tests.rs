use crate::{
    desktop::{Area, Page, tests::fixture},
    resources::{KubeAccess, Tab},
};
use gpui_kit::{AppContext, TestAppContext, component::WindowExt, test::TestWindowExt};

#[test]
fn screenshot_sizes_are_bounded_and_invalid_values_keep_the_default() {
    for value in [
        None,
        Some("NaNx560"),
        Some("759x560"),
        Some("760x559"),
        Some("9000x600"),
        Some("broken"),
    ] {
        assert_eq!(
            super::window_size(value),
            gpui_kit::size(gpui_kit::px(1320.), gpui_kit::px(860.))
        );
    }
    assert_eq!(
        super::window_size(Some("760x560")),
        gpui_kit::size(gpui_kit::px(760.), gpui_kit::px(560.))
    );
}

#[gpui_kit::test]
fn fixture_pages_remain_reachable_at_minimum_size_in_both_themes(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    for text_size in [12., 14., 20.] {
        for theme in ["light", "dark"] {
            for (page, expected, id) in [
                ("overview", Page::Overview, "tile-pods"),
                ("nodes", Page::Nodes, "nodes-page"),
                ("node-overview", Page::Nodes, "node-pane"),
                ("pod-overview", Page::Resources, "resource-detail"),
                ("workload-logs", Page::Resources, "workload-logs-status"),
                ("search", Page::Resources, "command"),
            ] {
                cx.update_window(handle, |_, window, cx| {
                    crate::text_size::set(text_size, cx);
                    pilot.update(cx, |pilot, cx| {
                        pilot.startup_selection(Some(page), None, Some(theme), window, cx)
                    });
                })
                .unwrap();
                cx.run_until_parked();
                cx.update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    assert_eq!(pilot.read(cx).page, expected);
                    let element = window.find(id);
                    if page == "overview" && !element.visible() {
                        for _ in 0..10 {
                            window.scroll(
                                "overview-page",
                                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                                    gpui_kit::px(0.),
                                    gpui_kit::px(-90.),
                                )),
                                cx,
                            );
                            window.render_frame(cx);
                            if window.find(id).visible() {
                                break;
                            }
                        }
                    }
                    let element = window.find(id);
                    assert!(element.visible(), "{page} {theme} {text_size}: {id}");
                    let viewport = window.viewport_size();
                    let bounds = element.bounds();
                    assert!(bounds.size.width > gpui_kit::px(0.));
                    assert!(bounds.left() >= gpui_kit::px(0.));
                    assert!(bounds.right() <= viewport.width + gpui_kit::px(1.));
                    if page == "pod-overview" {
                        assert_eq!(
                            pilot.read(cx).resources.read(cx).detail_tab(cx),
                            Tab::Overview
                        );
                        // The pane opens on why the pod fails.
                        assert!(
                            window.find("pod-cause").visible(),
                            "{page} {theme} {text_size}: cause={:?} pane={:?} viewport={:?}",
                            window.find("pod-cause").bounds(),
                            window.find("resource-detail").bounds(),
                            window.viewport_size()
                        );
                    }
                    if page == "search" {
                        assert!(window.has_active_dialog(cx));
                        assert!(bounds.top() >= gpui_kit::px(0.));
                        assert!(bounds.bottom() <= viewport.height);
                        window.close_dialog(cx);
                    }
                })
                .unwrap();
            }
        }
    }
}

#[gpui_kit::test]
fn fixture_monitoring_opens_its_dashboard_from_example_data_and_hides_on_navigation(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.startup_selection(Some("monitoring"), None, Some("dark"), window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).area, Area::Monitoring);
        assert!(window.find("monitoring-grid").visible());
        assert!(window.find("monitoring-variable-node").visible());
        assert!(window.find("monitoring-panel-0-title").visible());
        assert!(window.try_find("monitoring-panel-0-failed").is_none());
        assert!(
            window
                .find("monitoring-dashboard-freshkube-cluster")
                .visible()
        );
        window.click("monitoring-dashboard-freshkube-workloads", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            pilot.read(cx).monitoring.read(cx).chosen(),
            &crate::monitoring::page::EntryId::Builtin("freshkube-workloads")
        );
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Overview, window, cx));
        window.render_frame(cx);
        assert!(window.try_find("monitoring-grid").is_none());
        assert!(window.find("tile-pods").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn fixture_kubernetes_only_stays_offline_across_contexts_and_refresh(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.startup_selection(Some("kubernetes-only"), None, Some("dark"), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(pilot.read(cx).overview_display.cards.len(), 4);
        assert!(window.try_find("nav-etcd").is_none());
        assert!(pilot.read(cx).nodes.is_empty());
        assert!(matches!(
            pilot.read(cx).kube_source().unwrap().access,
            KubeAccess::Example
        ));
        pilot.update(cx, |pilot, cx| {
            pilot.select_context("staging-eu".into(), window, cx);
            pilot.refresh(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = pilot.read(cx);
        assert_eq!(view.overview_display.cards.len(), 4);
        assert!(view.nodes.is_empty());
        assert_eq!(view.applied.context.as_deref(), Some("staging-eu"));
        assert!(matches!(
            view.kube_source().unwrap().access,
            KubeAccess::Example
        ));
        assert!(view.kubernetes_only.as_ref().unwrap().access().is_none());
        assert!(
            view.kubernetes_summary
                .data()
                .unwrap()
                .pods
                .loaded()
                .is_some()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn short_resource_navigation_reveals_the_pane_and_returns_to_the_list(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        pilot.update(cx, |pilot, cx| {
            pilot.startup_selection(Some("pod-overview"), None, Some("light"), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("pod-cause").visible());
        assert_eq!(window.find("resource-detail").focused(), Some(true));
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("resource-body").focused(), Some(true));
        assert!(window.find("resource-filter").visible());
        window.press("secondary-f", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("resource-filter").visible());
        assert_eq!(window.find("resource-filter").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn card_contents_align_and_long_node_names_keep_to_one_row(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let top = window
            .within("tile-nodes")
            .find("tile-card-content")
            .bounds()
            .top();
        for id in ["tile-etcd", "tile-workloads", "tile-pods"] {
            assert!(
                (window.within(id).find("tile-card-content").bounds().top() - top).abs()
                    <= gpui_kit::px(1.)
            );
        }
        pilot.update(cx, |pilot, cx| {
            pilot.startup_selection(Some("nodes"), None, None, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        crate::desktop::tests::expand_healthy_nodes(window, cx);
        let name = "talos-cp-fra1-03-baremetal-rack-b7";
        let id = format!("node-{name}");
        assert_eq!(window.find(id.clone()).label(), Some(name));
        assert!(
            window.within(id).find("node-row-name").bounds().size.height
                <= crate::ui::dp_px(26., window)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn minimum_node_table_can_reveal_its_rightmost_column(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        pilot.update(cx, |pilot, cx| {
            pilot.startup_selection(Some("nodes"), None, None, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let before = window.find(("nodes-sort", 8usize)).bounds().left();
        window.scroll(
            "nodes-table-scroll",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(-1800.), gpui_kit::px(0.))),
            cx,
        );
        window.render_frame(cx);
        let column = window.find(("nodes-sort", 8usize));
        assert!(column.bounds().left() < before);
        assert!(column.visible());
        // The table runs to the window's edge, and layout rounding can put
        // the last cell's padding a pixel past it.
        assert!(column.bounds().right() <= window.viewport_size().width + gpui_kit::px(1.5));
    })
    .unwrap();
}

#[gpui_kit::test]
fn example_pods_and_nodes_show_their_cpu_and_memory_history(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    for (page, id) in [
        ("pod-overview", "pod-history"),
        ("node-overview", "node-history"),
    ] {
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| {
                pilot.startup_selection(Some(page), None, None, window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find(id).is_some(), "{page}: {id}");
            let prefix = id.trim_end_matches("-history");
            for panel in ["cpu", "memory"] {
                let panel = format!("monitoring-panel-{prefix}-{panel}");
                let found = window.try_find(gpui_kit::SharedString::from(panel.clone()));
                assert!(found.is_some(), "{page}: {panel}");
                // A pod's history is among the first things its Overview
                // shows, on screen without scrolling.
                if prefix == "pod" {
                    assert!(found.unwrap().visible(), "{page}: {panel} visible");
                }
            }
        })
        .unwrap();
    }
    // Another tab on the node pane hides the history and stops its reads.
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.show_node_tab(crate::desktop::nodes::NodeTab::Pods, window, cx);
            assert!(!pilot.node_history.read(cx).reading());
        });
        window.render_frame(cx);
        assert!(window.try_find("node-history").is_none());
    })
    .unwrap();
}

/// At the smallest window a workload's logs keep every control above the
/// log and inside the dock: the chips give their rows up first, and
/// "+N" lists every container.
#[gpui_kit::test]
fn workload_logs_controls_stay_above_the_log_at_minimum_size(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.startup_selection(Some("workload-logs"), None, None, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        // The panel learns its height while drawing; the chips fit it on
        // the next frame.
        for _ in 0..3 {
            window.render_frame(cx);
        }
        let pane = window.find("dock").bounds();
        let toolbar = window.find("logs-toolbar").bounds();
        let viewport = window.find("logs-viewport").bounds();
        assert!(
            toolbar.bottom() <= viewport.top(),
            "{toolbar:?} {viewport:?}"
        );
        assert!(toolbar.top() >= pane.top() && toolbar.bottom() <= pane.bottom());
        assert!(toolbar.left() >= pane.left() && toolbar.right() <= pane.right());
        // Nothing in the toolbar is cut off at its bottom.
        for id in ["workload-logs-status", "logs-search", "logs-follow"] {
            let bounds = window.find(id).bounds();
            assert!(
                bounds.bottom() <= toolbar.bottom() + gpui_kit::px(0.5),
                "{id}: {bounds:?} below {toolbar:?}"
            );
        }
        // Every container stays in reach.
        assert!(window.try_find("workload-logs-more").is_some());
    })
    .unwrap();
}
