use std::sync::Arc;

use freshkube_core::workloads::{HealthState, WorkloadKind, WorkloadSource, WorkloadSourceError};
use freshkube_ui::table::{self, TableSource};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext, Window, WindowHandle, px,
    size,
};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{ItemKey, RowRef, ScreenPanel, ScreenSource, WorkloadData, WorkloadsScreen};
use crate::backend::Target;
use crate::desktop::layout_check;
use crate::desktop::tests::fixture as app;
use crate::{fixture, presentation, ui::Tone};

const WORKLOADS_FRAME: layout_check::PageFrame = layout_check::PageFrame {
    page: "workloads-page",
    title: "workloads-title",
    title_text: "Workloads",
    content: "workloads-split",
};

fn source(node: &str) -> ScreenSource {
    let nodes = presentation::node_summaries(&fixture::cluster("prod-fra", 1));
    let summary = nodes.iter().find(|summary| summary.name == node).unwrap();
    ScreenSource {
        target: Target {
            epoch: 1,
            context: "prod-fra".into(),
            node: summary.name.clone(),
            address: summary.address.clone(),
        },
        nodes: Arc::new(nodes),
        live: None,
    }
}

fn mount(
    cx: &mut TestAppContext,
    node: &str,
) -> (Runtime, Entity<WorkloadsScreen>, WindowHandle<Root>) {
    mount_sized(cx, node, 1100.)
}

fn mount_sized(
    cx: &mut TestAppContext,
    node: &str,
    width: f32,
) -> (Runtime, Entity<WorkloadsScreen>, WindowHandle<Root>) {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        crate::theme::install(cx);
    });
    let source = source(node);
    let mut screen = None;
    let handle = cx.open_window(size(px(width), px(760.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = WorkloadsScreen::new(runtime.handle().clone(), window, cx);
            view.set_source(Some(source), window, cx);
            view.activate(window, cx);
            view
        });
        screen = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, screen.unwrap(), handle)
}

/// The element id of the visible row at `ix`.
fn row_id(screen: &WorkloadsScreen, ix: usize) -> SharedString {
    screen.row_element(ix)
}

fn row_key(screen: &WorkloadsScreen, ix: usize) -> ItemKey {
    let rows = screen.rows();
    rows[ix].key(&screen.loader.data().unwrap().snapshot)
}

/// The Inspector shows only with a selection: beside the table on a wide
/// page and under it on a narrow one, edge to edge and in no card.
#[gpui_kit::test]
fn the_inspector_opens_with_a_selection_beside_or_under_the_table(cx: &mut TestAppContext) {
    for width in [1500., 900.] {
        let (_runtime, screen, handle) = mount_sized(cx, "talos-cp-fra1-01", width);
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("workload-detail").is_none(), "{width}");
            window.click(row_id(screen.read(cx), 0), cx);
            window.render_frame(cx);
            layout_check::assert_inspector(
                window,
                cx,
                "workloads-split",
                "workloads-table",
                "workload-detail",
                "workload-detail-title",
            );
            let table = window.find("workloads-table").bounds();
            let detail = window.find("workload-detail").bounds();
            let beside = crate::screens::page_width(window) >= freshkube_ui::inspector::SPLIT_WIDTH;
            if beside {
                assert!(detail.left() >= table.right(), "{width}: {detail:?}");
            } else {
                assert!(detail.top() >= table.bottom(), "{width}: {detail:?}");
            }
            assert_eq!(width > 1200., beside, "{width}");
        })
        .unwrap();
    }
}

/// Escape steps back one level at a time: it clears the filters first,
/// keeping the selection, then clears the selection, which closes the
/// Inspector.
#[gpui_kit::test]
fn escape_clears_the_filter_then_closes_the_inspector(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_sized(cx, "talos-cp-fra1-01", 1500.);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(row_id(screen.read(cx), 0), cx);
        window.press("u", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).only_unhealthy);
        window.find("workload-detail");
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(!screen.read(cx).only_unhealthy);
        assert!(screen.read(cx).selected.is_some());
        window.find("workload-detail");
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).selected.is_none());
        assert!(window.try_find("workload-detail").is_none());
        // The table keeps the keyboard and the arrows select again.
        window.press("down", cx);
        window.render_frame(cx);
        window.find("workload-detail-title");
    })
    .unwrap();
}

/// A selected item the cluster no longer reports keeps the Inspector open
/// and says so, rather than showing stale details.
#[gpui_kit::test]
fn a_selection_the_cluster_no_longer_reports_says_so(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_sized(cx, "talos-cp-fra1-01", 1500.);
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.select(ItemKey::Namespace("removed-namespace".into()), cx)
        });
        window.render_frame(cx);
        assert!(window.find("workload-detail-gone").visible());
        assert_eq!(
            window.find("workload-detail-title").label(),
            Some("removed-namespace")
        );
    })
    .unwrap();
}

/// A selected pod that leaves the cluster's answer, as one that recovers
/// does, keeps the Inspector open under its name and says it is no longer
/// listed, not that the cluster stopped reporting it.
#[gpui_kit::test]
fn a_selected_pod_the_next_answer_drops_keeps_its_name(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount_sized(cx, "talos-cp-fra1-01", 1500.);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let (namespace, pod) = {
            let data = screen.read(cx).loader.data().unwrap();
            let namespace = (data.snapshot.namespaces.iter())
                .find(|ns| !ns.problem_pods.is_empty())
                .unwrap();
            (
                namespace.name.clone(),
                namespace.problem_pods[0].name.clone(),
            )
        };
        let key = ItemKey::Pod {
            namespace: namespace.clone(),
            name: pod.clone(),
        };
        screen.update(cx, |screen, cx| screen.select(key.clone(), cx));
        window.render_frame(cx);
        assert!(window.try_find("workload-detail-gone").is_none());
        let title = window
            .find("workload-detail-title")
            .label()
            .unwrap()
            .to_owned();
        assert!(title.contains(&pod), "{title}");
        screen.update(cx, |screen, cx| {
            let mut snapshot = screen.loader.data().unwrap().snapshot.clone();
            for ns in &mut snapshot.namespaces {
                if ns.name == namespace {
                    ns.problem_pods.retain(|p| p.name != pod);
                }
            }
            let outcome = freshkube_core::workloads::WorkloadCollectionOutcome::Complete(snapshot);
            screen.apply_summary("prod-fra", WorkloadData::from_outcome(&outcome), cx);
        });
        window.render_frame(cx);
        assert_eq!(screen.read(cx).selected, Some(key));
        let gone = window.find("workload-detail-gone");
        assert!(gone.visible());
        assert_eq!(
            window.find("workload-detail-title").label(),
            Some(pod.as_str())
        );
    })
    .unwrap();
}

/// A width dragged to is saved under `health` in `navigation.json`, and
/// the next page opens its Inspector at it.
#[gpui_kit::test]
fn the_inspector_width_survives_reopening(cx: &mut TestAppContext) {
    use crate::navigation_file::NavigationFile;
    let directory = std::env::temp_dir().join(format!(
        "freshkube-health-inspector-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let preferences = directory.join("preferences.json");
    cx.update(|cx| cx.set_global(NavigationFile::open(Some(&preferences))));
    let (_runtime, screen, handle) = mount_sized(cx, "talos-cp-fra1-01", 1500.);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(row_id(screen.read(cx), 0), cx);
        window.render_frame(cx);
        let state = screen.read(cx).split.beside_state().clone();
        state.update(cx, |state, cx| {
            state.resize_panel(1, crate::ui::dp_px(520., window), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let reopened = NavigationFile::open(Some(&preferences));
    assert_eq!(reopened.inspector_width("health"), Some(520.));
    cx.update(|cx| cx.set_global(reopened));
    let (_runtime, screen, _handle) = mount_sized(cx, "talos-cp-fra1-01", 1500.);
    cx.read(|cx| assert_eq!(screen.read(cx).split.width(), 520.));
    let _ = std::fs::remove_dir_all(&directory);
}

#[gpui_kit::test]
fn keyboard_selects_rows_and_updates_details(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.read(cx).loader.data().is_some());
        window.click(row_id(screen.read(cx), 0), cx);
        assert_eq!(screen.read(cx).selected, Some(row_key(screen.read(cx), 0)));
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(row_id(screen.read(cx), 1)).selected(),
            Some(true)
        );
        assert_eq!(screen.read(cx).selected, Some(row_key(screen.read(cx), 1)));
        window.find("workload-detail-title");
        window.press("end", cx);
        window.render_frame(cx);
        let last = screen.read(cx).rows().len() - 1;
        assert_eq!(
            window.find(row_id(screen.read(cx), last)).selected(),
            Some(true)
        );
        window.press("home", cx);
        assert_eq!(screen.read(cx).selected, Some(row_key(screen.read(cx), 0)));
    })
    .unwrap();
}

/// A row's id names its item, not its place: a filter that moves the row
/// keeps its id, and the id still selects that item.
/// A row's ids carry their role in the prefix, so a namespace whose name
/// ends like a role, such as `a-status`, shares no id with namespace `a`.
#[test]
fn no_two_rows_share_an_element_id() {
    use super::source::element_ids;
    let pod = |namespace: &str, name: &str| ItemKey::Pod {
        namespace: namespace.into(),
        name: name.into(),
    };
    let keys = [
        ItemKey::Namespace("a".into()),
        ItemKey::Namespace("a-status".into()),
        ItemKey::Namespace("a-name".into()),
        ItemKey::Namespace("a-row".into()),
        ItemKey::Workload {
            namespace: "a".into(),
            name: "x".into(),
            kind: WorkloadKind::Deployment,
        },
        pod("a", "x"),
    ];
    let ids: Vec<String> = keys.iter().flat_map(element_ids).collect();
    let unique: std::collections::BTreeSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "{ids:?}");
}

#[gpui_kit::test]
fn a_row_id_follows_its_item_through_a_filter(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    let worker = ItemKey::Workload {
        namespace: "shop".into(),
        name: "worker".into(),
        kind: WorkloadKind::Deployment,
    };
    let line_of = |screen: &WorkloadsScreen, key: &ItemKey| {
        let snapshot = &screen.loader.data().unwrap().snapshot;
        screen
            .rows()
            .iter()
            .position(|row| row.key(snapshot) == *key)
    };
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let before = line_of(screen.read(cx), &worker).unwrap();
        let id = row_id(screen.read(cx), before);
        assert_eq!(id.as_ref(), "workload-row-shop/deployment/worker");
        let shop = line_of(screen.read(cx), &ItemKey::Namespace("shop".into())).unwrap();
        assert_eq!(row_id(screen.read(cx), shop).as_ref(), "workload-row-shop");
        let ids: std::collections::HashSet<_> = (0..screen.read(cx).line_count())
            .map(|line| row_id(screen.read(cx), line))
            .collect();
        assert_eq!(ids.len(), screen.read(cx).line_count(), "ids are unique");

        window.click("only-unhealthy", cx);
        window.render_frame(cx);
        let after = line_of(screen.read(cx), &worker).unwrap();
        assert_ne!(after, before, "the filter moves the row");
        assert_eq!(row_id(screen.read(cx), after), id);
        window.click(id, cx);
        assert_eq!(screen.read(cx).selected, Some(worker.clone()));
    })
    .unwrap();
}

#[gpui_kit::test]
fn unhealthy_filter_narrows_the_list(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let all = screen.read(cx).rows();
        assert!(
            all.iter().any(|row| matches!(
                row.key(&screen.read(cx).loader.data().unwrap().snapshot),
                ItemKey::Namespace(name) if name == "cert-manager"
            )),
            "the healthy namespace is listed"
        );
        window.click("only-unhealthy", cx);
        window.render_frame(cx);
        let screen_ref = screen.read(cx);
        let narrowed = screen_ref.rows();
        assert!(narrowed.len() < all.len());
        let snapshot = &screen_ref.loader.data().unwrap().snapshot;
        for row in narrowed.iter() {
            match *row {
                RowRef::Namespace(ns) => {
                    assert_ne!(snapshot.namespaces[ns].health, HealthState::Healthy)
                }
                RowRef::Workload(ns, ix) => assert_ne!(
                    snapshot.namespaces[ns].workloads[ix].health,
                    HealthState::Healthy
                ),
                RowRef::Pod(..) => {}
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn enter_collapses_a_namespace(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let before = screen.read(cx).rows().len();
        window.click(row_id(screen.read(cx), 0), cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).rows().len() < before);
        window.press("enter", cx);
        assert_eq!(screen.read(cx).rows().len(), before);
    })
    .unwrap();
}

#[gpui_kit::test]
fn example_reflects_the_degraded_worker(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-03");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        // Cluster-scoped: a silent target node still shows workloads.
        let data = screen.read(cx).loader.data().expect("workloads load");
        let flannel = data
            .snapshot
            .namespaces
            .iter()
            .flat_map(|ns| &ns.problem_pods)
            .find(|pod| pod.name.starts_with("kube-flannel"))
            .expect("flannel pod on the degraded worker");
        assert_eq!(flannel.node.as_deref(), Some("talos-wk-fra1-02"));
        // The counts go to the status bar's line; the header keeps none.
        let line = screen.update(cx, |screen, _| {
            screen.status().expect("a segment").text(None).clone()
        });
        assert!(line.contains(" deployments · "), "{line}");
        assert!(line.contains("pods "), "{line}");
        assert!(line.ends_with("example data"), "{line}");
        assert!(window.try_find("screen-retry").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn pods_whose_container_died_draw_the_skull(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-03");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let screen = screen.read(cx);
        let data = screen.loader.data().expect("workloads load");
        let tones: Vec<_> = screen
            .rows()
            .iter()
            .filter(|row| matches!(row, RowRef::Pod(..)))
            .map(|row| {
                let view = screen.describe(*row, data);
                (view.name, view.tone)
            })
            .collect();
        let tone = |prefix: &str| {
            tones
                .iter()
                .find(|(name, _)| name.starts_with(prefix))
                .unwrap_or_else(|| panic!("no {prefix} row in {tones:?}"))
                .1
        };
        // CrashLoopBackOff and OOMKilled ran and stopped.
        assert_eq!(tone("kube-flannel-"), Tone::Died);
        assert_eq!(tone("api-"), Tone::Died);
        // An image it can't pull is still critical: nothing ran.
        assert_eq!(tone("worker-5c7b8d6f4-hq8r2"), Tone::Crit);
        assert_eq!(tone("worker-5c7b8d6f4-zl4vn"), Tone::Crit);
        assert_eq!(tone("kube-proxy-"), Tone::Warn);
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubernetes_unavailable_offers_retry_not_failure(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            let target = screen.source.as_ref().unwrap().target.clone();
            screen.loader.reset();
            screen
                .loader
                .resolve(target, Err("no kubeconfig selected".into()));
            cx.notify();
        });
        window.render_frame(cx);
        window.find("k8s-unavailable");
        window.find("screen-retry");
        assert!(window.try_find("workload-list").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn partial_results_name_what_is_missing(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            let snapshot = screen.loader.data().unwrap().snapshot.clone();
            let outcome = freshkube_core::workloads::WorkloadCollectionOutcome::Partial {
                snapshot,
                unavailable: vec![WorkloadSourceError {
                    source: WorkloadSource::Pods,
                    message: "forbidden".into(),
                }],
            };
            let data = WorkloadData::from_outcome(&outcome);
            screen.apply_summary("prod-fra", data, cx);
            cx.notify();
        });
        window.render_frame(cx);
        window.find("partial-notice");
        window.find("workload-list");
    })
    .unwrap();
}

#[gpui_kit::test]
fn changing_target_drops_old_data(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.selected = Some(ItemKey::Namespace("shop".into()));
            // Same target, fresh handles: data and selection stay.
            screen.set_source(Some(source("talos-cp-fra1-01")), window, cx);
            assert!(screen.loader.data().is_some());
            assert!(screen.selected.is_some());
            screen.set_source(Some(source("talos-wk-fra1-02")), window, cx);
            assert!(screen.loader.data().is_none());
            assert!(screen.selected.is_none());
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn rows_are_cached_until_the_data_or_the_filters_change(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let computed = || crate::desktop::probe::count("workloads.rows");
        let first = screen.read(cx).rows();
        let base = computed();
        window.render_frame(cx);
        window.press("down", cx);
        let again = screen.read(cx).rows();
        assert!(std::rc::Rc::ptr_eq(&first, &again));
        assert_eq!(computed(), base, "nothing it depends on changed");
        // Unhealthy only.
        window.click("only-unhealthy", cx);
        let narrowed = screen.read(cx).rows();
        assert_eq!(computed(), base + 1);
        assert!(narrowed.len() < first.len());
        window.click("only-unhealthy", cx);
        // Collapsing a namespace.
        let namespace = screen.read(cx).loader.data().unwrap().snapshot.namespaces[0]
            .name
            .clone();
        screen.update(cx, |screen, cx| {
            screen.collapsed.insert(namespace);
            screen.sync(cx);
            cx.notify();
        });
        let collapsed = screen.read(cx).rows();
        assert!(computed() > base + 1);
        assert!(collapsed.len() < first.len());
        let settled = computed();
        screen.read(cx).rows();
        assert_eq!(computed(), settled);
        // The filter text.
        screen.update(cx, |screen, cx| {
            screen.collapsed.clear();
            screen
                .query
                .update(cx, |input, cx| input.set_value("cert", window, cx));
            screen.sync(cx);
        });
        screen.read(cx).rows();
        assert!(computed() > settled);
        let settled = computed();
        // A new snapshot.
        screen.update(cx, |screen, cx| screen.refresh(window, cx));
        screen.read(cx).rows();
        assert_eq!(computed(), settled + 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn workloads_is_an_edge_page_at_both_text_sizes(cx: &mut TestAppContext) {
    for text in [None, Some(20.)] {
        let (_runtime, handle, _view) = app(cx, 1280., 880.);
        cx.update_window(handle, |_, window, cx| {
            if let Some(text) = text {
                crate::text_size::set(text, cx);
            }
            window.press("secondary-5", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            layout_check::assert_edge_frame(window, cx, &WORKLOADS_FRAME);
            let rows = layout_check::assert_table(
                window,
                cx,
                &layout_check::Table {
                    table: Some("workload-table-scroll"),
                    list: "workload-list",
                },
            );
            assert!(rows.header.is_some(), "{rows:#?}");
            let line = window.find("health-scope");
            assert!(
                line.path()
                    .contains(&gpui_kit::ElementId::from("status-bar")),
                "the counts are in the status bar"
            );
        })
        .unwrap();
    }
}

/// Narrow and at a large text size the table scrolls sideways, as Nodes'
/// does: its glyph and name stay at the left edge, and the last column can
/// be brought fully into view (#150's follow-up).
#[gpui_kit::test]
fn a_narrow_table_scrolls_sideways_with_its_name_pinned(cx: &mut TestAppContext) {
    use gpui_kit::{ScrollDelta, point};
    // Window, text size, and whether the table runs past its list.
    for (width, height, text, overflows) in [
        (1280., 880., None, false),
        (760., 560., Some(20.), true),
        (760., 560., Some(14.), true),
        (1024., 700., Some(18.), true),
        (900., 600., Some(16.), true),
    ] {
        let (_runtime, handle, app_view) = app(cx, width, height);
        let screen = cx.update(|cx| app_view.read(cx).workloads());
        cx.update_window(handle, |_, window, cx| {
            if let Some(text) = text {
                crate::text_size::set(text, cx);
            }
            window.press("secondary-5", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let case = format!("{width}/{text:?}");
            let rows = layout_check::assert_table(
                window,
                cx,
                &layout_check::Table {
                    table: Some("workload-table-scroll"),
                    list: "workload-list",
                },
            );
            assert!(rows.header.is_some(), "{case}: {rows:#?}");
            let view = window.find("workload-table-scroll").bounds();
            let header =
                |window: &Window, column: usize| window.find(("workload-sort", column)).bounds();
            // A workload's row, not a namespace's, whose line always stays.
            let line = (screen.read(cx).rows().iter())
                .position(|row| !matches!(row, RowRef::Namespace(_)))
                .expect("a workload row");
            let cell = screen.read(cx).name_element(line);
            let name = header(window, 1).left();
            let row_name = window.find(cell.clone()).bounds().left();
            // The last column, which never passes under the pinned run; a
            // cell that does reports its bounds clipped at the pinned edge.
            let last = header(window, 4).right();
            let overflow = last - view.right();
            assert_eq!(
                overflow > px(1.5),
                overflows,
                "{case}: the table {view:?}, its last column ends at {last:?}"
            );
            window.scroll(
                "workload-table-scroll",
                ScrollDelta::Pixels(point(px(-10_000.), px(0.))),
                cx,
            );
            window.render_frame(cx);
            let moved = last - header(window, 4).right();
            assert!(
                (moved - overflow.max(px(0.))).abs() <= px(1.5),
                "{case}: the last column moved {moved:?}, not {overflow:?}"
            );
            // `find` fails on an id that resolves twice, so the pinned
            // name is found once, at its place.
            let pinned = header(window, 1);
            assert!(
                (pinned.left() - name).abs() <= px(1.5),
                "{case}: the name moved from {name:?} to {:?}",
                pinned.left()
            );
            let kept = window.find(cell).bounds().left();
            assert!(
                (kept - row_name).abs() <= px(1.5),
                "{case}: the row's name moved from {row_name:?} to {kept:?}"
            );
            // The last column shows whole, clear of the pinned glyph and name.
            let last = header(window, 4);
            assert!(
                last.right() <= view.right() + px(1.5) && last.left() >= pinned.right() - px(1.5),
                "{case}: the last column {last:?} isn't clear of the pinned run, \
                 which ends at {:?}, inside {view:?}",
                pinned.right()
            );
        })
        .unwrap();
    }
}

/// The Name column narrows only where pinning needs it: a list wide enough
/// keeps any name up to the table's widest column, and a narrow one never
/// goes below its least width.
#[test]
fn the_name_narrows_only_in_a_narrow_list() {
    use super::source::name_most;
    // 1280 at text 13, with the details beside the list.
    assert!(name_most(654.) > table::WIDEST + 20.);
    // 760 at text 20, with the details below.
    let narrow = name_most(378.);
    assert!(narrow < 220., "{narrow}");
    assert!(
        narrow + table::GLYPH_WIDTH <= table::widest_pinned_run(378.),
        "{narrow}"
    );
    assert_eq!(name_most(100.), 120.);
}

/// The state in the table's place sits under the toolbar, which keeps the
/// title and Refresh.
fn under_the_toolbar(window: &Window, id: &'static str) {
    let toolbar = window.find("workloads-toolbar").bounds();
    window.find("workloads-title");
    window.find("workloads-refresh");
    let state = window.find(id).bounds();
    assert!(
        state.top() >= toolbar.bottom(),
        "{id} {state:?} isn't under the toolbar {toolbar:?}"
    );
    assert!(window.try_find("workload-list").is_none());
}

#[gpui_kit::test]
fn every_state_sits_under_the_toolbar(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        // The Kubernetes API couldn't be reached.
        screen.update(cx, |screen, cx| {
            let target = screen.source.as_ref().unwrap().target.clone();
            screen.loader.reset();
            screen.loader.resolve(target, Err("no kubeconfig".into()));
            cx.notify();
        });
        window.render_frame(cx);
        under_the_toolbar(window, "k8s-unavailable");
        under_the_toolbar(window, "workloads-state");
        // No target.
        screen.update(cx, |screen, cx| screen.set_source(None, window, cx));
        window.render_frame(cx);
        under_the_toolbar(window, "workloads-state");
        assert!(window.try_find("k8s-unavailable").is_none());
    })
    .unwrap();
}

fn settle(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    for _ in 0..4 {
        cx.run_until_parked();
        let asked = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.simulate_next_frame(cx)
            })
            .unwrap();
        if asked == 0 {
            return;
        }
    }
    panic!("the header keeps moving");
}

#[gpui_kit::test]
fn only_unhealthy_folded_acts_as_its_button(cx: &mut TestAppContext) {
    // The title, the filter, Refresh and "…" fit; Only unhealthy doesn't.
    let (_runtime, screen, handle) = mount_sized(cx, "talos-cp-fra1-01", 340.);
    let handle: AnyWindowHandle = handle.into();
    settle(handle, cx);
    let pick = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            assert!(window.try_find("only-unhealthy").is_none(), "it's folded");
            window.find("workload-filter");
            window.click("workloads-more", cx);
            window.render_frame(cx);
            window.within("popup-menu").click(0, cx);
            window.render_frame(cx);
        })
        .unwrap();
        settle(handle, cx);
    };
    pick(cx);
    assert!(screen.read_with(cx, |screen, _| screen.only_unhealthy));
    cx.update_window(handle, |_, window, _| {
        window.find("workloads-more-dot");
    })
    .unwrap();
    pick(cx);
    assert!(!screen.read_with(cx, |screen, _| screen.only_unhealthy));
}

/// A namespace row is a table row with a group's look: its tinted cells
/// fill the row inside its border, and the row keeps the table's 26 dp.
#[gpui_kit::test]
fn a_namespace_row_is_tinted_at_the_row_height(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(matches!(screen.read(cx).rows()[0], RowRef::Namespace(_)));
        let row = window.find(row_id(screen.read(cx), 0)).bounds();
        let name = window.find(screen.read(cx).name_element(0)).bounds();
        let height = f32::from(row.size.height);
        assert!((height - 26.).abs() < 0.5, "row {row:?}");
        // Inside the row's 1 px border, above and below.
        assert!(
            (f32::from(name.size.height) - (height - 2.)).abs() < 0.5,
            "the tinted name cell {name:?} doesn't fill its row {row:?}"
        );
    })
    .unwrap();
}

/// Beside the Inspector the table stops at its left edge: rows narrower
/// than the table's columns scroll sideways inside it, as Nodes' do, and
/// never run under the Inspector.
#[gpui_kit::test]
fn the_table_stops_at_the_inspector(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = app(cx, 1280., 880.);
    let screen = cx.update(|cx| view.read(cx).workloads());
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-5", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(row_id(screen.read(cx), 1), cx);
        window.render_frame(cx);
        let scroll = window.find("workload-table-scroll").bounds();
        let details = window.find("workload-detail").bounds();
        assert!(
            (details.left() - scroll.right()).abs() <= px(0.5),
            "{scroll:?} {details:?}"
        );
        let mut rows = 0usize;
        for line in 0..screen.read(cx).line_count() {
            let Some(row) = window.try_find(row_id(screen.read(cx), line)) else {
                continue;
            };
            let row = row.bounds();
            assert!(
                (row.left() - scroll.left()).abs() <= px(0.5),
                "row {line} {row:?} starts outside the table {scroll:?}"
            );
            rows += 1;
        }
        assert!(rows > 0);
        // The table's columns are wider than its room, so it scrolls
        // sideways rather than squeezing the Issue column below its least.
        let width = crate::ui::dp_px(screen.read(cx).width(), window);
        assert!(width > scroll.size.width, "{width:?} {scroll:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_row_tooltip_holds_its_issue(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let screen = screen.read(cx);
        let tips: Vec<String> = (0..screen.line_count())
            .filter_map(|line| match screen.line(line, cx)? {
                table::Line::Row(row) => row.tooltip.map(|tip| tip.to_string()),
                _ => None,
            })
            .collect();
        assert!(
            tips.iter().any(|tip| tip.ends_with(" · CrashLoopBackOff")),
            "{tips:?}"
        );
        assert!(tips.iter().any(|tip| tip == "coredns"), "{tips:?}");
    })
    .unwrap();
}

/// Stacked, the Inspector a click opens shrinks the table; the clicked row,
/// low in the full table, comes back into view once the table settles, at
/// the default text size and at 20.
#[gpui_kit::test]
fn a_stacked_inspector_keeps_the_clicked_row_in_view(cx: &mut TestAppContext) {
    for text in [None, Some(20.)] {
        let (_runtime, screen, handle) = mount_sized(cx, "talos-cp-fra1-01", 900.);
        cx.update_window(handle.into(), |_, window, cx| {
            if let Some(text) = text {
                crate::text_size::set(text, cx);
            }
            window.render_frame(cx);
            assert!(crate::screens::page_width(window) < freshkube_ui::inspector::SPLIT_WIDTH);
            let scroll = window.find("workload-table-scroll").bounds();
            let low = (0..screen.read(cx).line_count())
                .filter(|&line| {
                    window
                        .try_find(row_id(screen.read(cx), line))
                        .is_some_and(|row| row.bounds().bottom() <= scroll.bottom())
                })
                .max()
                .unwrap();
            assert!(low > 4, "only {low} rows in the full table");
            let id = row_id(screen.read(cx), low);
            window.click(id.clone(), cx);
            window.render_frame(cx);
            for _ in 0..10 {
                if window.simulate_next_frame(cx) == 0 {
                    break;
                }
            }
            window.render_frame(cx);
            let scroll = window.find("workload-table-scroll").bounds();
            let detail = window.find("workload-detail").bounds();
            assert!(
                detail.top() >= scroll.bottom() - px(0.5),
                "{detail:?} {scroll:?}"
            );
            let row = window.find(id).bounds();
            assert!(
                row.top() >= scroll.top() && row.bottom() <= scroll.bottom() + px(0.5),
                "row {low} {row:?} out of the table {scroll:?}"
            );
        })
        .unwrap();
    }
}
