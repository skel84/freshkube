//! History-driven recovery through the page's existing connection owner.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU16, AtomicUsize, Ordering},
};

use freshkube_core::monitoring::{Prometheus, history::Subject};
use freshkube_ui::real_io::until_io;
use tokio::sync::{oneshot, watch};

use super::*;
use crate::history::HistoryView;

/// A loopback proxy whose query answers can be failed or held. No credentials.
struct Replies {
    status: AtomicU16,
    asked: AtomicUsize,
    answered: AtomicUsize,
    gate: Mutex<Option<watch::Receiver<bool>>>,
}

impl Replies {
    fn new(status: u16) -> Arc<Self> {
        Arc::new(Self {
            status: AtomicU16::new(status),
            asked: AtomicUsize::new(0),
            answered: AtomicUsize::new(0),
            gate: Mutex::new(None),
        })
    }

    fn hold(&self) -> watch::Sender<bool> {
        let (send, receive) = watch::channel(false);
        *self.gate.lock().unwrap() = Some(receive);
        send
    }
}

fn history_api(runtime: &tokio::runtime::Runtime, replies: &Arc<Replies>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let address = listener.local_addr().unwrap();
    let replies = replies.clone();
    runtime.spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let replies = replies.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0; 16 * 1024];
                let mut read = 0;
                while !buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buffer[read..]).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => read += n,
                    }
                }
                let head = String::from_utf8_lossy(&buffer[..read]);
                let path = head.split_whitespace().nth(1).unwrap_or("");
                let query = path.contains("/api/v1/query_range");
                let (status, body) = if query {
                    replies.asked.fetch_add(1, Ordering::SeqCst);
                    let gate = replies.gate.lock().unwrap().clone();
                    if let Some(mut gate) = gate {
                        while !*gate.borrow_and_update() {
                            if gate.changed().await.is_err() { break; }
                        }
                    }
                    let status = replies.status.load(Ordering::SeqCst);
                    let body = if status == 200 {
                        r#"{"status":"success","data":{"resultType":"matrix","result":[{"metric":{"container":"sample"},"values":[[1700003600,"0.5"]]}]}}"#
                    } else { "the proxy says no" };
                    (status, body)
                } else if path.contains("/api/v1/label/") {
                    (200, r#"{"status":"success","data":["sample"]}"#)
                } else if path.contains("/api/v1/series") {
                    (200, r#"{"status":"success","data":[{"node":"sample","namespace":"sample"}]}"#)
                } else {
                    (200, r#"{"apiVersion":"v1","kind":"List","metadata":{},"items":[]}"#)
                };
                let answer = format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                _ = socket.write_all(answer.as_bytes()).await;
                if query { replies.answered.fetch_add(1, Ordering::SeqCst); }
            });
        }
    });
    format!("http://{address}")
}

fn ready_history(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    first: &Arc<Replies>,
    replacement: &Arc<Replies>,
) -> (Entity<MonitoringPage>, Arc<FakeCluster>) {
    let cluster = FakeCluster::new(history_api(runtime, first));
    *cluster.replacement.lock().unwrap() = Some(history_api(runtime, replacement));
    let source = ClusterSource {
        id: "access-a".into(),
        context: "sample".into(),
        access: ClusterAccess::Live(cluster.clone()),
    };
    let (_, page) = mount_with(cx, runtime, Some(source), None, None);
    let client = runtime.block_on(cluster.client()).unwrap();
    let prometheus = Prometheus::new(client, PrometheusService::new("metrics", "sample", 9090));
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.discovered(
                Ok(Discovery::Found {
                    prometheus,
                    version: None,
                    tried: Vec::new(),
                }),
                cx,
            )
        })
    });
    (page, cluster)
}

fn pod() -> Subject {
    Subject::Pod {
        namespace: "sample".into(),
        name: "sample".into(),
    }
}

fn history(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    page: &Entity<MonitoringPage>,
    subject: Subject,
) -> Entity<HistoryView> {
    let source = cx.read(|cx| page.read(cx).history());
    let history = cx.new(|_| {
        let mut view = HistoryView::new(runtime.handle().clone(), "sample");
        view.set_now(|| NOW);
        view
    });
    cx.update(|cx| {
        history.update(cx, |view, cx| {
            view.set_source(source, cx);
            view.set_subject(Some(subject), cx);
            view.set_visible(true, cx);
        })
    });
    history
}

fn answered(cx: &mut TestAppContext, history: &Entity<HistoryView>) -> usize {
    cx.read(|cx| {
        history
            .read(cx)
            .panels()
            .iter()
            .filter(|panel| panel.read(cx).is_ready())
            .count()
    })
}

fn first_failure(
    cx: &mut TestAppContext,
    history: &Entity<HistoryView>,
    cluster: &Arc<FakeCluster>,
) {
    until_io(cx, "the first history read or its rebuild", |cx| {
        cx.read(|cx| !history.read(cx).reading()) || cluster.forgot() > 0
    });
    assert_eq!(
        cluster.forgot(),
        1,
        "history must request the bounded rebuild"
    );
}

fn recovered(
    cx: &mut TestAppContext,
    page: &Entity<MonitoringPage>,
    history: &Entity<HistoryView>,
) {
    until_io(cx, "history through the replacement client", |cx| {
        cx.read(|cx| page.read(cx).history().unwrap().client == 1 && !history.read(cx).reading())
            && answered(cx, history) == 2
    });
}

fn hold_client(cluster: &FakeCluster) -> oneshot::Sender<()> {
    let (send, receive) = oneshot::channel();
    *cluster.gate.lock().unwrap() = Some(receive);
    send
}

fn client_started(cx: &mut TestAppContext, cluster: &FakeCluster) {
    until_io(cx, "the gated replacement client", |_| {
        cluster.asked.load(Ordering::SeqCst) == 2
    });
}

fn hidden_recovers(cx: &mut TestAppContext, status: u16, subject: Subject) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (page, cluster) = ready_history(cx, &runtime, &Replies::new(status), &Replies::new(200));
    let history = history(cx, &runtime, &page, subject);
    first_failure(cx, &history, &cluster);
    recovered(cx, &page, &history);
    cx.read(|cx| {
        assert!(!page.read(cx).visible);
        assert!(
            page.read(cx).board.is_none(),
            "hidden Monitoring must not start dashboard reads"
        );
        assert_eq!(page.read(cx).rebuild_name(), "used");
    });
    assert_eq!(cluster.asked.load(Ordering::SeqCst), 2);
}

#[gpui_kit::test]
fn hidden_monitoring_recovers_a_pod_history_401(cx: &mut TestAppContext) {
    hidden_recovers(cx, 401, pod());
}

#[gpui_kit::test]
fn hidden_monitoring_recovers_a_node_history_unavailable(cx: &mut TestAppContext) {
    hidden_recovers(
        cx,
        503,
        Subject::Node {
            name: "sample".into(),
        },
    );
}

#[gpui_kit::test]
fn concurrent_history_failures_share_one_rebuild(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (page, cluster) = ready_history(cx, &runtime, &Replies::new(401), &Replies::new(200));
    let release = hold_client(&cluster);
    let pod = history(cx, &runtime, &page, pod());
    let node = history(
        cx,
        &runtime,
        &page,
        Subject::Node {
            name: "sample".into(),
        },
    );
    until_io(cx, "both histories' failures", |cx| {
        cx.read(|cx| !pod.read(cx).reading() && !node.read(cx).reading())
    });
    assert_eq!(cluster.forgot(), 1);
    client_started(cx, &cluster);
    release.send(()).unwrap();
    recovered(cx, &page, &pod);
    recovered(cx, &page, &node);
    assert_eq!(cluster.asked.load(Ordering::SeqCst), 2);
}

#[gpui_kit::test]
fn history_timers_do_not_loop_after_a_failed_rebuild(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (page, cluster) = ready_history(cx, &runtime, &Replies::new(401), &Replies::new(401));
    let history = history(cx, &runtime, &page, pod());
    first_failure(cx, &history, &cluster);
    until_io(cx, "replacement history failures", |cx| {
        cx.read(|cx| {
            page.read(cx).rebuild_name() == "used"
                && history.read(cx).asks() == 2
                && !history.read(cx).reading()
        })
    });
    for asks in 3..=5 {
        cx.executor().advance_clock(Duration::from_secs(60));
        until_io(cx, "the history timer's failed read", |cx| {
            cx.read(|cx| history.read(cx).asks() == asks && !history.read(cx).reading())
        });
        assert_eq!(cluster.forgot(), 1);
        assert_eq!(cluster.asked.load(Ordering::SeqCst), 2);
    }
}

#[gpui_kit::test]
fn explicit_history_refresh_resets_the_same_rebuild_bound(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let replacement = Replies::new(401);
    let (page, cluster) = ready_history(cx, &runtime, &Replies::new(401), &replacement);
    let history = history(cx, &runtime, &page, pod());
    first_failure(cx, &history, &cluster);
    until_io(cx, "bounded persistent failure", |cx| {
        cx.read(|cx| {
            page.read(cx).rebuild_name() == "used"
                && history.read(cx).asks() == 2
                && !history.read(cx).reading()
        })
    });
    let release = hold_client(&cluster);
    cx.update(|cx| history.update(cx, |view, cx| view.refresh(cx)));
    until_io(cx, "the explicit retry's rebuild", |cx| {
        cluster.forgot() == 2 || cx.read(|cx| !history.read(cx).reading())
    });
    assert_eq!(cluster.forgot(), 2);
    cx.update(|cx| history.update(cx, |view, cx| view.refresh(cx)));
    until_io(cx, "the coalesced Refresh read", |cx| {
        cx.read(|cx| !history.read(cx).reading())
    });
    assert_eq!(cluster.forgot(), 2);
    replacement.status.store(200, Ordering::SeqCst);
    release.send(()).unwrap();
    until_io(cx, "the explicit retry's answer", |cx| {
        cx.read(|cx| page.read(cx).history().unwrap().client == 2 && !history.read(cx).reading())
            && answered(cx, &history) == 2
    });
    assert_eq!(cluster.asked.load(Ordering::SeqCst), 3);
}

#[gpui_kit::test]
fn hiding_monitoring_keeps_a_rebuild_requested_by_history(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (page, cluster) = ready_history(cx, &runtime, &Replies::new(401), &Replies::new(200));
    let release = hold_client(&cluster);
    let history = history(cx, &runtime, &page, pod());
    first_failure(cx, &history, &cluster);
    client_started(cx, &cluster);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_visible(true, cx);
            page.set_visible(false, cx);
        })
    });
    assert_eq!(cx.read(|cx| page.read(cx).rebuild_name()), "building");
    release.send(()).unwrap();
    recovered(cx, &page, &history);
    cx.read(|cx| {
        let page = page.read(cx);
        let board = page.board.as_ref().unwrap();
        assert!(board.resolving.is_none());
        assert!(board.slots.iter().all(|slot| slot.request.is_none()));
    });
}

#[gpui_kit::test]
fn history_403_keeps_the_service_client(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (page, cluster) = ready_history(cx, &runtime, &Replies::new(403), &Replies::new(200));
    let history = history(cx, &runtime, &page, pod());
    until_io(cx, "history's refusal", |cx| {
        cx.read(|cx| !history.read(cx).reading())
    });
    assert_eq!(answered(cx, &history), 0);
    assert_eq!(cluster.forgot(), 0);
    assert_eq!(cluster.asked.load(Ordering::SeqCst), 1);
    assert_eq!(cx.read(|cx| page.read(cx).rebuild_name()), "unused");
}

#[gpui_kit::test]
fn direct_url_history_does_not_rebuild_cluster_access(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let replies = Replies::new(401);
    let (page, cluster) = ready_history(cx, &runtime, &replies, &Replies::new(200));
    let url = history_api(&runtime, &replies);
    let prometheus = Prometheus::direct(url, None).unwrap();
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.discovered(
                Ok(Discovery::Found {
                    prometheus,
                    version: None,
                    tried: Vec::new(),
                }),
                cx,
            )
        })
    });
    let history = history(cx, &runtime, &page, pod());
    until_io(cx, "direct history's failure", |cx| {
        cx.read(|cx| !history.read(cx).reading())
    });
    assert_eq!(answered(cx, &history), 0);
    assert_eq!(cluster.forgot(), 0);
    assert_eq!(cluster.asked.load(Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn fixture_history_never_requests_cluster_access(cx: &mut TestAppContext) {
    let (runtime, _, page) = mount(cx, Some(example_source()));
    let history = history(cx, &runtime, &page, pod());
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 2);
    assert_eq!(cx.read(|cx| page.read(cx).history().unwrap().client), 0);
    cx.update(|cx| history.update(cx, |view, cx| view.refresh(cx)));
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 2);
    assert_eq!(cx.read(|cx| page.read(cx).history().unwrap().client), 0);
}

#[derive(Clone, Copy)]
enum Cancel {
    Access,
    Subject,
    Hide,
}

fn cancel(
    cx: &mut TestAppContext,
    page: &Entity<MonitoringPage>,
    history: &Entity<HistoryView>,
    how: Cancel,
) {
    cx.update(|cx| match how {
        Cancel::Access => page.update(cx, |page, cx| {
            page.set_source(
                Some(ClusterSource {
                    id: "access-b".into(),
                    context: "sample".into(),
                    access: ClusterAccess::Live(Arc::new(NoClient)),
                }),
                cx,
            )
        }),
        Cancel::Subject => history.update(cx, |view, cx| view.set_subject(None, cx)),
        Cancel::Hide => history.update(cx, |view, cx| view.set_visible(false, cx)),
    });
    cx.run_until_parked();
}

fn late_rebuild(cx: &mut TestAppContext, how: Cancel) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let replacement = Replies::new(200);
    let (page, cluster) = ready_history(cx, &runtime, &Replies::new(401), &replacement);
    let release = hold_client(&cluster);
    let history = history(cx, &runtime, &page, pod());
    first_failure(cx, &history, &cluster);
    client_started(cx, &cluster);
    cancel(cx, &page, &history, how);
    until_io(cx, "the cancelled rebuild", |cx| {
        cx.read(|cx| page.read(cx).rebuild_name() != "building")
    });
    // Request cancellation drops the gated provider future before it can publish.
    until_io(cx, "the dropped client provider future", |_| {
        release.is_closed()
    });
    assert!(
        release.send(()).is_err(),
        "the cancelled client build still owns its delivery"
    );
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| page.read(cx).history().unwrap().client), 0);
    assert_eq!(replacement.asked.load(Ordering::SeqCst), 0);
    assert_eq!(answered(cx, &history), 0);
}

#[gpui_kit::test]
fn replacing_access_rejects_a_late_history_rebuild(cx: &mut TestAppContext) {
    late_rebuild(cx, Cancel::Access);
}
#[gpui_kit::test]
fn cancelling_history_rejects_a_late_rebuild(cx: &mut TestAppContext) {
    late_rebuild(cx, Cancel::Subject);
}
#[gpui_kit::test]
fn hiding_history_rejects_a_late_rebuild(cx: &mut TestAppContext) {
    late_rebuild(cx, Cancel::Hide);
}

fn late_read(cx: &mut TestAppContext, how: Cancel) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let replies = Replies::new(200);
    let release = replies.hold();
    let (page, cluster) = ready_history(cx, &runtime, &replies, &Replies::new(200));
    let history = history(cx, &runtime, &page, pod());
    until_io(cx, "both gated history reads", |_| {
        replies.asked.load(Ordering::SeqCst) == 2
    });
    cancel(cx, &page, &history, how);
    release.send(true).unwrap();
    until_io(cx, "the late proxy answers", |_| {
        replies.answered.load(Ordering::SeqCst) == 2
    });
    cx.run_until_parked();
    assert_eq!(
        answered(cx, &history),
        0,
        "a cancelled read reached a history panel"
    );
    assert_eq!(cluster.forgot(), 0);
    assert_eq!(cx.read(|cx| page.read(cx).history().unwrap().client), 0);
}

#[gpui_kit::test]
fn replacing_access_rejects_late_history_reads(cx: &mut TestAppContext) {
    late_read(cx, Cancel::Access);
}
#[gpui_kit::test]
fn cancelling_history_rejects_late_reads(cx: &mut TestAppContext) {
    late_read(cx, Cancel::Subject);
}
#[gpui_kit::test]
fn hiding_history_rejects_late_reads(cx: &mut TestAppContext) {
    late_read(cx, Cancel::Hide);
}

#[gpui_kit::test]
fn a_failed_history_client_build_is_bounded_until_explicit_retry(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (page, cluster) = ready_history(cx, &runtime, &Replies::new(401), &Replies::new(200));
    cluster.fail_rebuild.store(true, Ordering::SeqCst);
    let release = hold_client(&cluster);
    let history = history(cx, &runtime, &page, pod());
    first_failure(cx, &history, &cluster);
    client_started(cx, &cluster);
    release.send(()).unwrap();
    until_io(cx, "the failed client build", |cx| {
        cx.read(|cx| page.read(cx).rebuild_name() == "used" && !history.read(cx).reading())
    });
    assert_eq!(cx.read(|cx| page.read(cx).history().unwrap().client), 0);
    for asks in 2..=4 {
        cx.executor().advance_clock(Duration::from_secs(60));
        until_io(cx, "another failed history read", |cx| {
            cx.read(|cx| history.read(cx).asks() == asks && !history.read(cx).reading())
        });
        // The failed build also evicts its client through core's existing policy.
        assert_eq!(cluster.forgot(), 2);
        assert_eq!(cluster.asked.load(Ordering::SeqCst), 2);
    }
    cluster.fail_rebuild.store(false, Ordering::SeqCst);
    cx.update(|cx| history.update(cx, |view, cx| view.refresh(cx)));
    recovered(cx, &page, &history);
    assert_eq!(cluster.forgot(), 3);
    assert_eq!(cluster.asked.load(Ordering::SeqCst), 3);
}

#[gpui_kit::test]
fn cancelling_one_history_keeps_the_other_historys_rebuild(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (page, cluster) = ready_history(cx, &runtime, &Replies::new(401), &Replies::new(200));
    let release = hold_client(&cluster);
    let pod = history(cx, &runtime, &page, pod());
    let node = history(
        cx,
        &runtime,
        &page,
        Subject::Node {
            name: "sample".into(),
        },
    );
    first_failure(cx, &pod, &cluster);
    client_started(cx, &cluster);
    cancel(cx, &page, &pod, Cancel::Subject);
    assert_eq!(cx.read(|cx| page.read(cx).rebuild_name()), "building");
    release.send(()).unwrap();
    recovered(cx, &page, &node);
    assert_eq!(answered(cx, &pod), 0);
    assert_eq!(cluster.forgot(), 1);
}

#[gpui_kit::test]
fn dropping_the_last_history_cancels_its_rebuild(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (page, cluster) = ready_history(cx, &runtime, &Replies::new(401), &Replies::new(200));
    let release = hold_client(&cluster);
    let history = history(cx, &runtime, &page, pod());
    first_failure(cx, &history, &cluster);
    client_started(cx, &cluster);
    drop(history);
    cx.update(|_| {});
    until_io(cx, "the dropped history's client build", |_| {
        release.is_closed()
    });
    assert_eq!(cx.read(|cx| page.read(cx).rebuild_name()), "unused");
    assert_eq!(cx.read(|cx| page.read(cx).history().unwrap().client), 0);
}

#[gpui_kit::test]
fn hiding_monitoring_cancels_a_rebuild_only_the_page_requested(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (page, cluster) = ready_history(cx, &runtime, &Replies::new(401), &Replies::new(200));
    let release = hold_client(&cluster);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_visible(true, cx);
            page.read_failed(&QueryError::unauthorized(), cx);
        })
    });
    client_started(cx, &cluster);
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(false, cx)));
    until_io(cx, "the hidden dashboard's client build", |_| {
        release.is_closed()
    });
    assert_eq!(cx.read(|cx| page.read(cx).rebuild_name()), "unused");
    assert_eq!(cx.read(|cx| page.read(cx).history().unwrap().client), 0);
}
