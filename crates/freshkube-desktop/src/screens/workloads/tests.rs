use std::sync::Arc;

use freshkube_core::workloads::{HealthState, WorkloadSource, WorkloadSourceError};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, TestAppContext, Window, WindowHandle, px, size,
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

fn row_key(screen: &WorkloadsScreen, ix: usize) -> ItemKey {
    let rows = screen.rows();
    rows[ix].key(&screen.loader.data().unwrap().snapshot)
}

#[gpui_kit::test]
fn details_sit_beside_a_wide_list_and_below_a_narrow_one(cx: &mut TestAppContext) {
    for (width, beside) in [(1500., true), (900., false)] {
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
        assert_eq!(screen.read(cx).selected, Some(row_key(screen.read(cx), 0)));
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(window.find(("workload-row", 1usize)).selected(), Some(true));
        assert_eq!(screen.read(cx).selected, Some(row_key(screen.read(cx), 1)));
        window.find("workload-detail-title");
        window.press("end", cx);
        window.render_frame(cx);
        let last = screen.read(cx).rows().len() - 1;
        assert_eq!(window.find(("workload-row", last)).selected(), Some(true));
        window.press("home", cx);
        assert_eq!(screen.read(cx).selected, Some(row_key(screen.read(cx), 0)));
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
        window.click(("workload-row", 0usize), cx);
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
        window.find("workload-summary");
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
            window.find("workload-summary");
        })
        .unwrap();
    }
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
        let row = window.find(("workload-row", 0usize)).bounds();
        let name = window.find(("workload-name", 0usize)).bounds();
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
