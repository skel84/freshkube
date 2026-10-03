use std::sync::Arc;

use freshkube_core::workloads::{HealthState, WorkloadSource, WorkloadSourceError};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{ItemKey, RowRef, ScreenPanel, ScreenSource, WorkloadData, WorkloadsScreen};
use crate::backend::Target;
use crate::{fixture, presentation};

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

fn row_key(screen: &WorkloadsScreen, ix: usize, cx: &gpui_kit::App) -> ItemKey {
    let rows = screen.rows(cx);
    rows[ix].key(&screen.loader.data().unwrap().snapshot)
}

#[gpui_kit::test]
fn details_sit_beside_the_list_only_when_names_keep_their_room(cx: &mut TestAppContext) {
    for (width, beside) in [(1700., true), (1320., false)] {
        let (_runtime, _screen, handle) = mount_sized(cx, "talos-cp-fra1-01", width);
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let list = window.find("workload-list").bounds();
            let details = window.find("workload-details").bounds();
            if beside {
                assert!(details.left() >= list.right(), "{width}: {details:?}");
            } else {
                assert!(details.top() >= list.bottom(), "{width}: {details:?}");
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn keyboard_selects_rows_and_updates_details(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.read(cx).loader.data().is_some());
        window.click(("workload-row", 0usize), cx);
        assert_eq!(
            screen.read(cx).selected,
            Some(row_key(screen.read(cx), 0, cx))
        );
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(window.find(("workload-row", 1usize)).selected(), Some(true));
        assert_eq!(
            screen.read(cx).selected,
            Some(row_key(screen.read(cx), 1, cx))
        );
        window.find("workload-detail-title");
        window.press("end", cx);
        window.render_frame(cx);
        let last = screen.read(cx).rows(cx).len() - 1;
        assert_eq!(window.find(("workload-row", last)).selected(), Some(true));
        window.press("home", cx);
        assert_eq!(
            screen.read(cx).selected,
            Some(row_key(screen.read(cx), 0, cx))
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn unhealthy_filter_narrows_the_list(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let all = screen.read(cx).rows(cx);
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
        let narrowed = screen_ref.rows(cx);
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
        let before = screen.read(cx).rows(cx).len();
        window.click(("workload-row", 0usize), cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).rows(cx).len() < before);
        window.press("enter", cx);
        assert_eq!(screen.read(cx).rows(cx).len(), before);
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
        window.find("workload-summary");
        assert!(window.try_find("screen-retry").is_none());
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
        let first = screen.read(cx).rows(cx);
        let base = computed();
        window.render_frame(cx);
        window.press("down", cx);
        let again = screen.read(cx).rows(cx);
        assert!(std::rc::Rc::ptr_eq(&first, &again));
        assert_eq!(computed(), base, "nothing it depends on changed");
        // Unhealthy only.
        window.click("only-unhealthy", cx);
        let narrowed = screen.read(cx).rows(cx);
        assert_eq!(computed(), base + 1);
        assert!(narrowed.len() < first.len());
        window.click("only-unhealthy", cx);
        // Collapsing a namespace.
        let namespace = screen.read(cx).loader.data().unwrap().snapshot.namespaces[0]
            .name
            .clone();
        screen.update(cx, |screen, cx| {
            screen.collapsed.insert(namespace);
            cx.notify();
        });
        let collapsed = screen.read(cx).rows(cx);
        assert!(computed() > base + 1);
        assert!(collapsed.len() < first.len());
        let settled = computed();
        screen.read(cx).rows(cx);
        assert_eq!(computed(), settled);
        // The filter text.
        screen.update(cx, |screen, cx| {
            screen.collapsed.clear();
            screen
                .query
                .update(cx, |input, cx| input.set_value("cert", window, cx));
        });
        screen.read(cx).rows(cx);
        assert!(computed() > settled);
        let settled = computed();
        // A new snapshot.
        screen.update(cx, |screen, cx| screen.refresh(window, cx));
        screen.read(cx).rows(cx);
        assert_eq!(computed(), settled + 1);
    })
    .unwrap();
}
