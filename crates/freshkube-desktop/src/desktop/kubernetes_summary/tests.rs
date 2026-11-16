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
                let old = view
                    .switch
                    .registry
                    .active()
                    .summary_session
                    .as_ref()
                    .unwrap();
                let connection = old.core.identity().connection().to_owned();
                let mut target = old.target.clone();
                view.switch.stop_summary();
                target.epoch = view.epoch;
                let core = Session::new(SessionIdentity::new(connection, target.epoch));
                view.switch.registry.active_mut().summary_session = Some(SummarySession {
                    core: core.clone(),
                    target,
                    fixture_at: chrono::Utc::now(),
                    fixture_clock: cx.background_executor().now(),
                    applied_revision: 0,
                });
                view.switch.registry.active_mut().kubernetes_summary = Default::default();
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
            app.switch
                .registry
                .active()
                .kubernetes_summary
                .data()
                .unwrap()
                .pods
                .loaded()
                .is_none()
        );
        assert_eq!(
            app.overview_display(cx)
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
                .switch
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
            app.switch
                .registry
                .active()
                .summary_session
                .as_ref()
                .unwrap()
                .core
                .clone(),
            app.switch
                .registry
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
            .switch
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
            app.switch
                .registry
                .active()
                .kubernetes_summary
                .data()
                .unwrap()
                .events,
            Part::Refused(_)
        ));
        assert_eq!(
            app.switch
                .registry
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
        let display = app.overview_display(cx);
        let card = display
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
                .switch
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
            app.switch
                .registry
                .active()
                .summary_session
                .as_ref()
                .unwrap()
                .core
                .clone(),
            app.switch
                .registry
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
            .switch
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
            .switch
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
            .switch
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
                view.switch
                    .registry
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
                view.switch
                    .registry
                    .active()
                    .summary_session
                    .as_ref()
                    .unwrap()
                    .core
                    .identity(),
                old.identity()
            );
            let current = view
                .switch
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
                view.switch
                    .registry
                    .active()
                    .kubernetes_summary
                    .data()
                    .unwrap()
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
            app.switch
                .registry
                .active()
                .summary_session
                .as_ref()
                .unwrap()
                .core
                .identity()
                .clone(),
            app.switch
                .registry
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
                .target
                .nodes
                .iter()
                .find(|node| Some(&node.name) != view.target.selected_node.as_ref())
                .unwrap()
                .name
                .clone();
            view.select_node(Some(next), window, cx);
            assert_eq!(
                view.switch
                    .registry
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
                view.switch
                    .registry
                    .active()
                    .kubernetes_summary
                    .data()
                    .unwrap()
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
                .switch
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
                .switch
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
                .switch
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
                .switch
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
            view.mode.set_example(false);
            view.config_loading = true;
            view.apply_kubeconfig(selection, window, cx);
            assert!(view.switch.registry.active().summary_session.is_none());
            assert!(
                view.switch
                    .registry
                    .active()
                    .kubernetes_summary
                    .data()
                    .is_none()
            );
            view.apply_summary(
                late.clone(),
                WorkloadData::from_outcome(&late.summary.workloads),
                window,
                cx,
            );
            assert!(
                view.switch
                    .registry
                    .active()
                    .kubernetes_summary
                    .data()
                    .is_none()
            );
            view.mode.set_example(true);
            view.config_loading = false;
            view.ensure_summary(window, cx);
            assert_ne!(
                view.switch
                    .registry
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
                view.mode.set_kubernetes_only(
                    crate::desktop::kubernetes_only::KubernetesOnly::new(None, None),
                );
                view.navigate(Page::Health, window, cx);
                assert!(!view.countdown_state().1);
                view.switch
                    .registry
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
                .switch
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
                .switch
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

/// Hands out the next of its clients after each `forget`, as a cluster whose
/// login was renewed does, and counts the asks.
struct Renewing {
    clients: Vec<kube::Client>,
    asked: std::sync::atomic::AtomicUsize,
    forgotten: std::sync::atomic::AtomicUsize,
}

impl freshkube_core::cluster_source::KubeClientSource for Renewing {
    fn client(
        &self,
    ) -> futures::future::BoxFuture<'_, Result<kube::Client, freshkube_core::failure::Failure>>
    {
        use std::sync::atomic::Ordering::SeqCst;
        self.asked.fetch_add(1, SeqCst);
        let at = self.forgotten.load(SeqCst).min(self.clients.len() - 1);
        let client = self.clients[at].clone();
        Box::pin(async move { Ok(client) })
    }

    fn forget(&self) {
        self.forgotten
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn answering(code: u16) -> (kube::Client, freshkube_core::fake_api::Recorder) {
    freshkube_core::fake_api::client(move |request| {
        Some(if code == 200 {
            freshkube_core::fake_api::json(
                200,
                if request.path == "/version" {
                    r#"{"major":"1","minor":"32","gitVersion":"v1.32.3"}"#
                } else {
                    r#"{"metadata":{"resourceVersion":"1"},"items":[]}"#
                },
            )
        } else {
            freshkube_core::fake_api::json(
                code,
                json!({"kind":"Status","apiVersion":"v1","status":"Failure","code":code,
                    "reason":"Unauthorized","message":"token expired"}),
            )
        })
    })
}

async fn drive(
    clients: Vec<kube::Client>,
) -> (Arc<Renewing>, Session, tokio::task::JoinHandle<()>) {
    let source = Arc::new(Renewing {
        clients,
        asked: Default::default(),
        forgotten: Default::default(),
    });
    let session = Session::new(SessionIdentity::new("renewing", 1));
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    // The receiver stays open for the driver's life: it is the watch's.
    std::mem::forget(receiver);
    let task = tokio::spawn(run_summary_driver(
        session.clone(),
        ClusterAccess::Live(source.clone()),
        sender,
    ));
    (source, session, task)
}

#[tokio::test(start_paused = true)]
async fn an_expired_login_rebuilds_the_client_once_and_reads_through_the_new_one() {
    use std::sync::atomic::Ordering::SeqCst;
    let (expired, expired_log) = answering(401);
    let (renewed, renewed_log) = answering(200);
    let (source, session, task) = drive(vec![expired, renewed]).await;
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if renewed_log
            .lines()
            .iter()
            .any(|l| l.contains("/api/v1/nodes"))
        {
            break;
        }
    }
    assert!(
        renewed_log
            .lines()
            .iter()
            .any(|l| l.contains("/api/v1/nodes")),
        "the new client reads: {:?}",
        renewed_log.lines()
    );
    assert_eq!(source.forgotten.load(SeqCst), 1, "forgotten once");
    assert_eq!(source.asked.load(SeqCst), 2, "built again once");
    let before = expired_log.lines().len();
    tokio::time::sleep(Duration::from_secs(300)).await;
    assert_eq!(expired_log.lines().len(), before, "the old client is idle");
    assert_eq!(source.forgotten.load(SeqCst), 1);
    assert!(session.generation() >= 1);
    // The summary only reads.
    assert!(
        expired_log
            .requests()
            .iter()
            .chain(renewed_log.requests().iter())
            .all(|request| request.method == http::Method::GET)
    );
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn a_login_the_server_still_refuses_is_not_rebuilt_again() {
    use std::sync::atomic::Ordering::SeqCst;
    let (first, _) = answering(401);
    let (second, second_log) = answering(401);
    let (source, session, task) = drive(vec![first, second]).await;
    tokio::time::sleep(Duration::from_secs(900)).await;
    assert_eq!(source.asked.load(SeqCst), 2, "one rebuild, then none");
    assert_eq!(source.forgotten.load(SeqCst), 1);
    assert!(!second_log.lines().is_empty(), "the new client was tried");
    let derived = session.derive(chrono::Utc::now());
    assert_eq!(
        derived.summary.observations[&Source::Nodes].message(),
        Some("Kubernetes credentials were rejected"),
        "the refusal is what shows"
    );
    task.abort();
}
