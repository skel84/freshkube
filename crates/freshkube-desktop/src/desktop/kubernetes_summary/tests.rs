use std::{sync::Arc, time::Duration};

use freshkube_core::{
    kubernetes_summary::{ObservationFailure, Part, ReadStatus},
    resources::FailureKind,
};
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Event as KubeEvent, Pod};
use kube::runtime::watcher::Event;
use serde_json::json;

use super::*;
use crate::desktop::{Page, tests::fixture};

fn pod(name: &str) -> Pod {
    serde_json::from_value(
        json!({"metadata":{"namespace":"watch-test","name":name,"uid":name,"resourceVersion":"2"},
        "status":{"phase":"Pending"}}),
    )
    .unwrap()
}
fn flush(cx: &mut TestAppContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(DEBOUNCE);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn initial_sync_does_not_show_partial_pages_as_a_complete_count(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let session = cx
        .update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                let old = view.registry.active().summary_session.as_ref().unwrap();
                let connection = old.core.identity().connection().to_owned();
                let mut target = old.target.clone();
                view.stop_summary();
                target.epoch = view.registry.active().summary_epoch;
                let core = Session::new(SessionIdentity::new(connection, target.epoch));
                view.registry.active_mut().summary_session = Some(SummarySession {
                    core: core.clone(),
                    target,
                    fixture_at: chrono::Utc::now(),
                    fixture_clock: cx.background_executor().now(),
                    applied_revision: 0,
                });
                view.registry.active_mut().kubernetes_summary = Default::default();
                view.watch_fixture_summary(window, cx);
                core
            })
        })
        .unwrap();
    session
        .apply::<Pod>(0, Event::Init, chrono::Utc::now())
        .unwrap();
    session
        .apply(0, Event::InitApply(pod("first-page")), chrono::Utc::now())
        .unwrap();
    flush(cx);
    cx.update_window(handle, |_, window, cx| {
        let app = view.read(cx);
        assert!(
            app.registry
                .active()
                .kubernetes_summary
                .data()
                .unwrap()
                .pods
                .loaded()
                .is_none()
        );
        assert_eq!(
            app.overview_display
                .cards
                .iter()
                .find(|card| card.id == "tile-pods")
                .unwrap()
                .figure
                .as_ref(),
            "Unavailable"
        );
        window.render_frame(cx);
        window.find("tile-pods");
    })
    .unwrap();
    session
        .apply::<Pod>(0, Event::InitDone, chrono::Utc::now())
        .unwrap();
    flush(cx);
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            view.read(cx)
                .registry
                .active()
                .kubernetes_summary
                .data()
                .unwrap()
                .pods
                .loaded()
                .unwrap()
                .total,
            1
        );
        window.render_frame(cx);
        window.find("tile-pods");
    })
    .unwrap();
}

#[gpui_kit::test]
fn events_timeout_must_not_reject_successful_changed_pods(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let previous_health = cx
        .update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.navigate(Page::Health, window, cx));
            window.render_frame(cx);
            window.find("health-scope").label().unwrap().to_owned()
        })
        .unwrap();
    let (session, count) = cx.update(|cx| {
        let app = view.read(cx);
        (
            app.registry
                .active()
                .summary_session
                .as_ref()
                .unwrap()
                .core
                .clone(),
            app.registry
                .active()
                .kubernetes_summary
                .data()
                .unwrap()
                .pods
                .loaded()
                .unwrap()
                .total,
        )
    });
    session.fail(
        0,
        Source::Events,
        ObservationFailure::Read(FailureKind::Timeout),
    );
    session
        .apply(0, Event::Apply(pod("without-refresh")), chrono::Utc::now())
        .unwrap();
    let deployment: Deployment = serde_json::from_value(json!({
        "metadata": {"namespace":"watch-test", "name":"new-controller", "uid":"controller-uid", "resourceVersion":"3"},
        "spec": {"replicas":3, "selector":{}}, "status":{"readyReplicas":0}
    })).unwrap();
    session
        .apply(0, Event::Apply(deployment), chrono::Utc::now())
        .unwrap();
    flush(cx);
    cx.update_window(handle, |_, window, cx| {
        let summary = view
            .read(cx)
            .registry
            .active()
            .kubernetes_summary
            .data()
            .unwrap();
        assert_eq!(summary.pods.loaded().unwrap().total, count + 1);
        assert!(summary.events.loaded().is_some());
        assert!(!summary.events.is_current());
        view.update(cx, |view, cx| view.navigate(Page::Health, window, cx));
        window.render_frame(cx);
        window.find("workload-list");
        assert_ne!(
            window.find("health-scope").label().unwrap(),
            previous_health
        );
    })
    .unwrap();
    session.fail(
        0,
        Source::Events,
        ObservationFailure::Read(FailureKind::Forbidden),
    );
    flush(cx);
    cx.update_window(handle, |_, window, cx| {
        let app = view.read(cx);
        assert!(matches!(
            app.registry
                .active()
                .kubernetes_summary
                .data()
                .unwrap()
                .events,
            Part::Refused(_)
        ));
        assert_eq!(
            app.registry
                .active()
                .kubernetes_summary
                .data()
                .unwrap()
                .pods
                .loaded()
                .unwrap()
                .total,
            count + 1
        );
        let card = app
            .overview_display
            .cards
            .iter()
            .find(|card| card.id == "tile-events")
            .unwrap();
        assert!(card.detail.contains("Last known"));
        window.render_frame(cx);
        window.find("workload-list");
    })
    .unwrap();
    session
        .apply::<KubeEvent>(0, Event::Init, chrono::Utc::now())
        .unwrap();
    session
        .apply::<KubeEvent>(0, Event::InitDone, chrono::Utc::now())
        .unwrap();
    flush(cx);
    cx.update(|cx| {
        assert!(
            view.read(cx)
                .registry
                .active()
                .kubernetes_summary
                .data()
                .unwrap()
                .events
                .is_current()
        )
    });
}

#[gpui_kit::test]
fn expired_watch_relist_keeps_old_rows_until_the_replacement_commits(cx: &mut TestAppContext) {
    // The HTTP and streamed 410 tests exercise kube's wire transitions; feed
    // the resulting Init protocol through the real desktop receiver here.
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let (session, previous) = cx.update(|cx| {
        let app = view.read(cx);
        (
            app.registry
                .active()
                .summary_session
                .as_ref()
                .unwrap()
                .core
                .clone(),
            app.registry
                .active()
                .kubernetes_summary
                .data()
                .unwrap()
                .pods
                .loaded()
                .unwrap()
                .total,
        )
    });
    session
        .apply::<Pod>(0, Event::Init, chrono::Utc::now())
        .unwrap();
    session
        .apply(0, Event::InitApply(pod("replacement")), chrono::Utc::now())
        .unwrap();
    flush(cx);
    cx.update_window(handle, |_, window, cx| {
        let summary = view
            .read(cx)
            .registry
            .active()
            .kubernetes_summary
            .data()
            .unwrap();
        assert_eq!(summary.pods.loaded().unwrap().total, previous);
        assert_eq!(
            summary.observations[&Source::Pods].status(),
            ReadStatus::Resyncing
        );
        window.render_frame(cx);
        window.find("tile-pods");
    })
    .unwrap();
    session
        .apply::<Pod>(0, Event::InitDone, chrono::Utc::now())
        .unwrap();
    flush(cx);
    cx.update(|cx| {
        let summary = view
            .read(cx)
            .registry
            .active()
            .kubernetes_summary
            .data()
            .unwrap();
        assert!(summary.pods.is_current());
        assert_eq!(summary.pods.loaded().unwrap().total, 1);
        assert_eq!(summary.pods.loaded().unwrap().issues[0].name, "replacement");
    });
}

#[gpui_kit::test]
fn context_replacement_rejects_late_publications_even_when_returning_to_same_context(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let old = cx.update(|cx| {
        view.read(cx)
            .registry
            .active()
            .summary_session
            .as_ref()
            .unwrap()
            .core
            .clone()
    });
    let context = cx.update(|cx| view.read(cx).applied.context.clone().unwrap());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.select_context("staging-eu".into(), window, cx)
        })
    })
    .unwrap();
    old.apply(0, Event::Apply(pod("late-old-context")), chrono::Utc::now())
        .unwrap();
    let late = old.derive(chrono::Utc::now());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.apply_summary(
                late.clone(),
                WorkloadData::from_outcome(&late.summary.workloads),
                window,
                cx,
            );
            assert_ne!(
                view.registry
                    .active()
                    .summary_session
                    .as_ref()
                    .unwrap()
                    .core
                    .identity(),
                old.identity()
            );
            view.select_context(context, window, cx);
            assert_ne!(
                view.registry
                    .active()
                    .summary_session
                    .as_ref()
                    .unwrap()
                    .core
                    .identity(),
                old.identity()
            );
            let current = view
                .registry
                .active()
                .kubernetes_summary
                .data()
                .unwrap()
                .clone();
            view.apply_summary(
                late.clone(),
                WorkloadData::from_outcome(&late.summary.workloads),
                window,
                cx,
            );
            assert!(Arc::ptr_eq(
                &current,
                view.registry.active().kubernetes_summary.data().unwrap()
            ));
        })
    })
    .unwrap();
}

#[gpui_kit::test]
fn talos_cycle_and_node_selection_keep_the_session_but_refresh_relists(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let (identity, previous) = cx.update(|cx| {
        let app = view.read(cx);
        (
            app.registry
                .active()
                .summary_session
                .as_ref()
                .unwrap()
                .core
                .identity()
                .clone(),
            app.registry
                .active()
                .kubernetes_summary
                .data()
                .unwrap()
                .clone(),
        )
    });
    for _ in 0..16 {
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
    }
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            let next = view
                .nodes
                .iter()
                .find(|node| Some(&node.name) != view.selected_node.as_ref())
                .unwrap()
                .name
                .clone();
            view.select_node(Some(next), window, cx);
            assert_eq!(
                view.registry
                    .active()
                    .summary_session
                    .as_ref()
                    .unwrap()
                    .core
                    .identity(),
                &identity
            );
            assert!(Arc::ptr_eq(
                &previous,
                view.registry.active().kubernetes_summary.data().unwrap()
            ));
            view.navigate(Page::Health, window, cx);
        });
        window.render_frame(cx);
        window.click("workloads-refresh", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            view.read(cx)
                .registry
                .active()
                .summary_session
                .as_ref()
                .unwrap()
                .core
                .generation(),
            1
        );
        assert!(!Arc::ptr_eq(
            &previous,
            view.read(cx)
                .registry
                .active()
                .kubernetes_summary
                .data()
                .unwrap()
        ));
    });
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.navigate(Page::Lifecycle, window, cx));
        window.render_frame(cx);
        window.click("lifecycle-refresh", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            view.read(cx)
                .registry
                .active()
                .summary_session
                .as_ref()
                .unwrap()
                .core
                .generation(),
            2
        );
    });
}

#[gpui_kit::test]
fn reapplying_a_kubeconfig_at_the_same_path_discards_the_old_session(cx: &mut TestAppContext) {
    use freshkube_core::cluster_overview::KubeconfigSelection;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config");
    std::fs::write(&path, "apiVersion: v1\nkind: Config\n").unwrap();
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            let old = view
                .registry
                .active()
                .summary_session
                .as_ref()
                .unwrap()
                .core
                .clone();
            let late = old.derive(chrono::Utc::now());
            let selection = KubeconfigSelection::File {
                path: path.clone(),
                context: None,
            };
            view.kubeconfig = selection.clone();
            std::fs::write(
                &path,
                "apiVersion: v1\nkind: Config\ncurrent-context: replacement\n",
            )
            .unwrap();
            // Exercise the production reload entry point while the local
            // configuration read is pending; no cluster request can start.
            view.fixture = false;
            view.config_loading = true;
            view.apply_kubeconfig(selection, window, cx);
            assert!(view.registry.active().summary_session.is_none());
            assert!(view.registry.active().kubernetes_summary.data().is_none());
            view.apply_summary(
                late.clone(),
                WorkloadData::from_outcome(&late.summary.workloads),
                window,
                cx,
            );
            assert!(view.registry.active().kubernetes_summary.data().is_none());
            view.fixture = true;
            view.config_loading = false;
            view.ensure_summary(window, cx);
            assert_ne!(
                view.registry
                    .active()
                    .summary_session
                    .as_ref()
                    .unwrap()
                    .core
                    .identity(),
                old.identity()
            );
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubernetes_only_has_no_countdown_or_periodic_summary_apply(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let previous = cx
        .update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.kubernetes_only = Some(crate::desktop::kubernetes_only::KubernetesOnly::new(
                    None, None,
                ));
                view.navigate(Page::Health, window, cx);
                assert!(!view.countdown_state().1);
                view.registry
                    .active()
                    .kubernetes_summary
                    .data()
                    .unwrap()
                    .clone()
            })
        })
        .unwrap();
    for _ in 0..31 {
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
    }
    cx.update_window(handle, |_, window, cx| {
        assert!(Arc::ptr_eq(
            &previous,
            view.read(cx)
                .registry
                .active()
                .kubernetes_summary
                .data()
                .unwrap()
        ));
        assert!(!view.read(cx).countdown_state().1);
        window.render_frame(cx);
        window.click("refresh", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            view.read(cx)
                .registry
                .active()
                .summary_session
                .as_ref()
                .unwrap()
                .core
                .generation(),
            1
        )
    });
}
