use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::{Arc, Mutex};

use freshkube_core::monitoring::scope::{Place, Resolved};
use freshkube_core::monitoring::{Matcher, Prometheus};

use super::*;
use crate::history::HistoryKind;

/// Every request path and query a recording API saw.
type Seen = Arc<Mutex<Vec<String>>>;

/// A fake API server on loopback that answers every read through the proxy
/// as a Prometheus with no series, records what it was asked, and holds its
/// answers while `hold` is set.
fn recording_api(runtime: &tokio::runtime::Runtime, hold: Arc<AtomicBool>) -> (String, Seen) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let address = listener.local_addr().unwrap();
    let seen: Seen = Default::default();
    let log = seen.clone();
    runtime.spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let (log, hold) = (log.clone(), hold.clone());
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
                let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                log.lock().unwrap().push(path.clone());
                while hold.load(SeqCst) {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                let body = if path.contains("/proxy/api/v1/label/") {
                    r#"{"status":"success","data":["a"]}"#
                } else if path.contains("/proxy/api/v1/status/buildinfo") {
                    r#"{"status":"success","data":{"version":"2.53.0"}}"#
                } else if path.contains("/proxy/api/v1/targets") {
                    r#"{"status":"success","data":{"activeTargets":[]}}"#
                } else {
                    r#"{"status":"success","data":{"resultType":"matrix","result":[]}}"#
                };
                let answer = format!(
                    "HTTP/1.1 200 X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(answer.as_bytes()).await;
            });
        }
    });
    (format!("http://{address}"), seen)
}

fn central(value: &str) -> Resolved {
    Resolved::Central {
        place: Place::Service(PrometheusService::new("monitoring", "central", 9090)),
        host: Some("cluster-h".into()),
        matcher: Matcher::new("cluster", value),
    }
}

fn scoped(resolved: Resolved, host: &Arc<FakeCluster>) -> Scope {
    Scope {
        resolved,
        host: Some(ClusterAccess::Live(host.clone())),
    }
}

fn open_source(cluster: &Arc<FakeCluster>) -> ClusterSource {
    ClusterSource {
        id: "open".into(),
        context: "prod-ams".into(),
        access: ClusterAccess::Live(cluster.clone()),
    }
}

fn proxied(seen: &Seen) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|path| path.contains("/proxy/"))
        .cloned()
        .collect()
}

fn matcher_of(cx: &mut TestAppContext, page: &Entity<MonitoringPage>) -> Option<Matcher> {
    cx.read(|cx| match &page.read(cx).connection {
        Connection::Ready { prometheus, .. } => prometheus.matcher().cloned(),
        _ => None,
    })
}

#[gpui_kit::test]
async fn a_central_store_is_read_through_its_host_and_scoped_to_the_entry(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (open_url, open_seen) = recording_api(&runtime, Default::default());
    let (host_url, host_seen) = recording_api(&runtime, Default::default());
    let (open, host) = (FakeCluster::new(open_url), FakeCluster::new(host_url));
    let (handle, page) = mount_with(cx, &runtime, Some(open_source(&open)), None, None);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_scope(scoped(central("cluster-b"), &host), cx)
        })
    });
    show(cx, handle, &page);
    settled(cx, handle, &page).await;
    let observed = host_seen.clone();
    cx.wait_for_io(handle, move |_, _| {
        proxied(&observed)
            .iter()
            .any(|path| path.contains("/query_range"))
    })
    .await;

    assert_eq!(
        matcher_of(cx, &page),
        Some(Matcher::new("cluster", "cluster-b"))
    );
    let asked = proxied(&host_seen);
    assert!(
        asked
            .iter()
            .all(|path| path.contains("/services/central:9090/proxy/")),
        "{asked:?}"
    );
    // Every PromQL read carries the matcher; the probe and build info don't.
    for path in asked.iter().filter(|path| {
        path.contains("/query_range") || path.contains("/label/") || path.contains("/series")
    }) {
        assert!(path.contains("cluster%3D%22cluster-b%22"), "{path}");
    }
    // The open cluster was never asked for metrics, and nothing is
    // remembered for its context.
    assert!(proxied(&open_seen).is_empty(), "{:?}", proxied(&open_seen));
    cx.read(|cx| {
        let page = page.read(cx);
        assert!(page.saved.services.is_empty());
        // History reads the same scoped store.
        let history = page.history().unwrap();
        let HistoryKind::Ready(prometheus) = &history.kind else {
            panic!("history isn't on the confirmed store");
        };
        assert_eq!(
            prometheus.matcher(),
            Some(&Matcher::new("cluster", "cluster-b"))
        );
    });
}

#[gpui_kit::test]
async fn an_entry_with_no_source_under_per_entry_says_so_and_asks_nothing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (open_url, open_seen) = recording_api(&runtime, Default::default());
    let open = FakeCluster::new(open_url);
    let (handle, page) = mount_with(cx, &runtime, Some(open_source(&open)), None, None);
    let unknown = Resolved::Unknown("cluster-b names no metrics source".into());
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_scope(
                Scope {
                    resolved: unknown,
                    host: None,
                },
                cx,
            )
        })
    });
    show(cx, handle, &page);
    cx.read(|cx| {
        let page = page.read(cx);
        // A setup that names no store is not a failed connection.
        let Connection::Unknown(why) = &page.connection else {
            panic!("{}", page.connection_name());
        };
        assert_eq!(why.as_ref(), "cluster-b names no metrics source");
        // Not even the Service remembered for the context is guessed.
        assert!(page.history().is_none());
    });
    assert!(open_seen.lock().unwrap().is_empty());
    // It is the not-set-up state: no failure, no grid, nothing to retry.
    assert!(shown(cx, handle, "monitoring-unknown"));
    assert!(!shown(cx, handle, "monitoring-retry"));
    assert!(!shown(cx, handle, "monitoring-grid"));
}

#[gpui_kit::test]
async fn a_central_store_that_cannot_be_reached_names_the_entry_that_hosts_it(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (open_url, _) = recording_api(&runtime, Default::default());
    let open = FakeCluster::new(open_url);
    let (handle, page) = mount_with(cx, &runtime, Some(open_source(&open)), None, None);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_scope(
                Scope {
                    resolved: central("cluster-a"),
                    // The hosting entry's cluster cannot connect.
                    host: Some(ClusterAccess::Live(std::sync::Arc::new(NoClient))),
                },
                cx,
            )
        })
    });
    show(cx, handle, &page);
    settled(cx, handle, &page).await;
    cx.read(|cx| {
        let Connection::Failed(why) = &page.read(cx).connection else {
            panic!("{}", page.read(cx).connection_name());
        };
        // Not read as the open entry's own cluster failing.
        assert!(
            why.starts_with("Couldn't reach the metrics store through cluster-h: "),
            "{why}"
        );
        assert!(why.contains("No kubeconfig"), "{why}");
    });
}

#[gpui_kit::test]
async fn a_scope_change_mid_read_never_draws_the_previous_entrys_answer(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let hold = Arc::new(AtomicBool::new(true));
    let (open_url, _) = recording_api(&runtime, Default::default());
    let (host_url, host_seen) = recording_api(&runtime, hold.clone());
    let (open, host) = (FakeCluster::new(open_url), FakeCluster::new(host_url));
    let (handle, page) = mount_with(cx, &runtime, Some(open_source(&open)), None, None);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_scope(scoped(central("cluster-b"), &host), cx)
        })
    });
    show(cx, handle, &page);
    // The store's probe is asked and held.
    let observed = host_seen.clone();
    cx.wait_for_io(handle, move |_, _| !proxied(&observed).is_empty())
        .await;
    assert_eq!(cx.read(|cx| page.read(cx).connection_name()), "looking");
    let (clients, board) = cx.read(|cx| {
        let page = page.read(cx);
        (page.clients, page.board.as_ref().map(|board| board.id))
    });

    // Another entry opens before the first answered.
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_scope(scoped(central("cluster-c"), &host), cx)
        })
    });
    cx.read(|cx| {
        let page = page.read(cx);
        assert_eq!(page.connection_name(), "looking");
        assert_eq!(page.clients, clients + 1);
        assert_ne!(page.board.as_ref().map(|board| board.id), board);
    });
    hold.store(false, SeqCst);
    settled(cx, handle, &page).await;
    let observed = host_seen.clone();
    cx.wait_for_io(handle, move |_, _| {
        proxied(&observed)
            .iter()
            .any(|path| path.contains("/query_range"))
    })
    .await;

    assert_eq!(
        matcher_of(cx, &page),
        Some(Matcher::new("cluster", "cluster-c"))
    );
    let asked = proxied(&host_seen);
    assert!(
        !asked
            .iter()
            .any(|path| path.contains("cluster%3D%22cluster-b%22")),
        "the previous entry's scope was read: {asked:?}"
    );
}

#[gpui_kit::test]
async fn an_answer_for_the_previous_scope_is_dropped_when_it_lands_late(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let hold = Arc::new(AtomicBool::new(true));
    let (open_url, _) = recording_api(&runtime, Default::default());
    let (host_url, host_seen) = recording_api(&runtime, hold.clone());
    let (open, host) = (FakeCluster::new(open_url), FakeCluster::new(host_url));
    let (handle, page) = mount_with(cx, &runtime, Some(open_source(&open)), None, None);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_scope(scoped(central("cluster-b"), &host), cx)
        })
    });
    show(cx, handle, &page);
    let observed = host_seen.clone();
    cx.wait_for_io(handle, move |_, _| !proxied(&observed).is_empty())
        .await;
    let old = cx.read(|cx| match &page.read(cx).connection {
        Connection::Looking { _request } => _request.clone(),
        _ => panic!("not looking"),
    });
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_scope(scoped(central("cluster-c"), &host), cx)
        })
    });

    // The previous entry's probe answers after the switch, by the page's own
    // read generation: it neither connects nor replaces the new read.
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let stale = Prometheus::direct("http://127.0.0.1:1/".into(), None)
                .unwrap()
                .with_matcher(Matcher::new("cluster", "cluster-b"));
            page.land(
                &old,
                |page| Some(&mut page.connection_reads.looking),
                Ok((stale, None)),
                cx,
                |page, result, cx| {
                    let (prometheus, version) = result.unwrap();
                    page.use_prometheus(prometheus, version, cx)
                },
            );
            assert_eq!(page.connection_name(), "looking");
        })
    });
    assert_eq!(matcher_of(cx, &page), None);
    hold.store(false, SeqCst);
    settled(cx, handle, &page).await;
    assert_eq!(
        matcher_of(cx, &page),
        Some(Matcher::new("cluster", "cluster-c"))
    );
}

#[gpui_kit::test]
async fn the_same_scope_again_keeps_what_was_read(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (open_url, _) = recording_api(&runtime, Default::default());
    let (host_url, _) = recording_api(&runtime, Default::default());
    let (open, host) = (FakeCluster::new(open_url), FakeCluster::new(host_url));
    let (handle, page) = mount_with(cx, &runtime, Some(open_source(&open)), None, None);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_scope(scoped(central("cluster-b"), &host), cx)
        })
    });
    show(cx, handle, &page);
    settled(cx, handle, &page).await;
    let (clients, board) = cx.read(|cx| {
        let page = page.read(cx);
        (page.clients, page.board.as_ref().map(|board| board.id))
    });
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_scope(scoped(central("cluster-b"), &host), cx)
        })
    });
    cx.read(|cx| {
        let page = page.read(cx);
        assert_eq!(page.connection_name(), "ready");
        assert_eq!(page.clients, clients);
        assert_eq!(page.board.as_ref().map(|board| board.id), board);
    });
}

#[gpui_kit::test]
async fn discovery_is_untouched_by_a_scope_that_says_discovery(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(live_source()));
    cx.update(|cx| page.update(cx, |page, cx| page.set_scope(Scope::default(), cx)));
    show(cx, handle, &page);
    settled(cx, handle, &page).await;
    // The live source never builds a client, so discovery fails as it did.
    assert_eq!(cx.read(|cx| page.read(cx).connection_name()), "failed");
}
