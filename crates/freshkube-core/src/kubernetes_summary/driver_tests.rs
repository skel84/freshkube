//! Exercise real kube watcher pagination, HTTP/watch errors and cancellation.
use std::{
    collections::BTreeMap,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::body::Frame;
use kube::{Client, client::Body};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::*;

type Sender = mpsc::Sender<Result<Frame<Bytes>, Infallible>>;
#[derive(Clone)]
struct ApiLog {
    calls: Arc<Mutex<Vec<String>>>,
    streams: Arc<Mutex<BTreeMap<String, Sender>>>,
}
fn pod(name: &str, version: &str) -> Value {
    json!({"apiVersion":"v1","kind":"Pod","metadata":{"namespace":"ns","name":name,"uid":name,"resourceVersion":version},"status":{"phase":"Pending"}})
}
fn status(code: u16) -> Value {
    json!({"apiVersion":"v1","kind":"Status","status":"Failure","code":code,"reason":"Synthetic","message":"synthetic failure"})
}
fn full(value: Value) -> UnsyncBoxBody<Bytes, Infallible> {
    Full::new(Bytes::from(value.to_string())).boxed_unsync()
}
fn api(http_gone: bool, nodes_refused: bool) -> (Client, ApiLog) {
    api_with_delay(http_gone, nodes_refused, Duration::ZERO)
}
fn api_with_delay(http_gone: bool, nodes_refused: bool, page_delay: Duration) -> (Client, ApiLog) {
    let log = ApiLog {
        calls: Default::default(),
        streams: Default::default(),
    };
    let service_log = log.clone();
    let service = tower::service_fn(move |request: Request<Body>| {
        let uri = request.uri().to_string();
        let path = request.uri().path().to_owned();
        let delayed = path == "/api/v1/pods" && !uri.contains("watch=true");
        let mut calls = service_log.calls.lock().unwrap();
        calls.push(uri.clone());
        let pod_lists = calls
            .iter()
            .filter(|u| {
                u.starts_with("/api/v1/pods?")
                    && !u.contains("watch=true")
                    && !u.contains("continue=")
            })
            .count();
        let pod_watches = calls
            .iter()
            .filter(|u| u.starts_with("/api/v1/pods?") && u.contains("watch=true"))
            .count();
        drop(calls);
        let kind = path.rsplit('/').next().unwrap().to_owned();
        let (code, body) = if path == "/version" {
            (
                200,
                full(json!({"major":"1","minor":"32","gitVersion":"v1.32.3"})),
            )
        } else if kind == "nodes" && nodes_refused {
            (403, full(status(403)))
        } else if uri.contains("watch=true") {
            if kind == "pods" && http_gone && pod_watches == 1 {
                (410, full(status(410)))
            } else {
                let (sender, receiver) = mpsc::channel::<Result<Frame<Bytes>, Infallible>>(4);
                service_log.streams.lock().unwrap().insert(kind, sender);
                let stream = futures::stream::unfold(receiver, |mut receiver| async move {
                    receiver.recv().await.map(|frame| (frame, receiver))
                });
                (200, StreamBody::new(Box::pin(stream)).boxed_unsync())
            }
        } else if kind == "pods" && pod_lists == 1 {
            let next = !uri.contains("continue=");
            (
                200,
                full(
                    json!({"metadata":{"resourceVersion":"10","continue":if next {"page2"} else {""}},
                "items":[pod(if next {"first"} else {"missed-delete"},"10")]}),
                ),
            )
        } else {
            (
                200,
                full(
                    json!({"metadata":{"resourceVersion":"20"}, "items": if kind == "pods" {vec![pod("replacement","20")]} else {vec![]} }),
                ),
            )
        };
        async move {
            if delayed && !page_delay.is_zero() {
                tokio::time::sleep(page_delay).await;
            }
            Ok::<_, Infallible>(Response::builder().status(code).body(body).unwrap())
        }
    });
    (Client::new(service, "ns"), log)
}
async fn until(mut predicate: impl FnMut() -> bool) {
    for _ in 0..2000 {
        if predicate() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("watch driver did not reach the expected state");
}
fn session() -> Session {
    Session::new(SessionIdentity::new("synthetic", 1))
}

#[tokio::test(start_paused = true)]
async fn progressing_pages_can_take_longer_than_one_read_deadline() {
    let (client, log) = api_with_delay(false, false, Duration::from_secs(8));
    let session = session();
    let worker = session.clone();
    let task = tokio::spawn(async move { worker.run(client).await });
    until(|| {
        log.calls
            .lock()
            .unwrap()
            .iter()
            .any(|u| u.starts_with("/api/v1/pods?"))
    })
    .await;
    tokio::time::advance(Duration::from_secs(8)).await;
    until(|| {
        log.calls
            .lock()
            .unwrap()
            .iter()
            .any(|u| u.contains("continue=page2"))
    })
    .await;
    assert!(
        session
            .derive(chrono::Utc::now())
            .summary
            .pods
            .loaded()
            .is_none()
    );
    // Step through the old total deadline while page two is still pending,
    // rather than making the response and timeout ready in the same poll.
    tokio::time::advance(Duration::from_secs(7)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        session.derive(chrono::Utc::now()).summary.observations[&Source::Pods].status(),
        ReadStatus::Syncing
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    until(|| log.streams.lock().unwrap().len() == 9).await;
    let publication = session.derive(chrono::Utc::now());
    assert!(publication.summary.pods.is_current());
    assert_eq!(publication.pod_count, 2);
    task.abort();
    let _ = task.await;
}

#[tokio::test(start_paused = true)]
async fn paginated_sync_debounces_watch_changes_and_idle_never_relists() {
    let (client, log) = api(false, false);
    let session = session();
    let worker = session.clone();
    let task = tokio::spawn(async move { worker.run(client).await });
    until(|| log.streams.lock().unwrap().len() == 9).await;
    assert_eq!(session.derive(chrono::Utc::now()).pod_count, 2);
    let mut publications = session.publications();
    let key = SubscriptionKey::summary(session.identity().clone(), Source::Nodes);
    let summary_nodes = session.subscribe(key.clone()).unwrap();
    let lifecycle_nodes = session.subscribe(key).unwrap();
    tokio::time::advance(DEBOUNCE).await;
    until(|| publications.has_changed().unwrap()).await;
    publications.borrow_and_update();
    assert!(Arc::ptr_eq(
        &summary_nodes.latest().unwrap(),
        &lifecycle_nodes.latest().unwrap()
    ));
    let calls = log.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 20); // 9 initial lists + one extra Pod page + 9 watches + version.
    assert!(calls.iter().any(|u| u.contains("continue=page2")));
    assert!(
        calls
            .iter()
            .filter(|u| u.starts_with("/api/v1/events?"))
            .all(|u| u.contains("fieldSelector=type%3DWarning"))
    );
    let sender = log.streams.lock().unwrap()["pods"].clone();
    for version in 21..25 {
        sender
            .send(Ok(Frame::data(Bytes::from(format!(
                "{}\n",
                json!({"type":"MODIFIED","object":pod("new",&version.to_string())})
            )))))
            .await
            .unwrap();
    }
    until(|| session.derive(chrono::Utc::now()).pod_count == 3).await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    tokio::time::advance(DEBOUNCE - Duration::from_millis(1)).await;
    assert!(!publications.has_changed().unwrap());
    tokio::time::advance(Duration::from_millis(1)).await;
    until(|| publications.has_changed().unwrap()).await;
    assert_eq!(
        publications.borrow_and_update().as_ref().unwrap().pod_count,
        3
    );
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    assert_eq!(*log.calls.lock().unwrap(), calls);
    assert!(!publications.has_changed().unwrap());
    task.abort();
    let _ = task.await;
    until(|| log.streams.lock().unwrap().values().all(Sender::is_closed)).await;
}

#[tokio::test(start_paused = true)]
async fn http_and_streamed_410_each_relist_only_the_expired_collection() {
    for http in [true, false] {
        let (client, log) = api(http, false);
        let session = session();
        let worker = session.clone();
        let task = tokio::spawn(async move { worker.run(client).await });
        until(|| log.streams.lock().unwrap().len() == 9).await;
        if !http {
            let sender = log.streams.lock().unwrap()["pods"].clone();
            sender
                .send(Ok(Frame::data(Bytes::from(format!(
                    "{}\n",
                    json!({"type":"ERROR","object":status(410)})
                )))))
                .await
                .unwrap();
        }
        until(|| session.derive(chrono::Utc::now()).pod_count == 1).await;
        {
            let calls = log.calls.lock().unwrap();
            assert_eq!(
                calls
                    .iter()
                    .filter(|u| u.starts_with("/api/v1/pods?") && !u.contains("watch=true"))
                    .count(),
                3
            );
            assert_eq!(
                calls
                    .iter()
                    .filter(|u| u.starts_with("/api/v1/nodes?") && !u.contains("watch=true"))
                    .count(),
                1
            );
        }
        assert_eq!(
            session
                .derive(chrono::Utc::now())
                .summary
                .pods
                .loaded()
                .unwrap()
                .issues[0]
                .name,
            "replacement"
        );
        task.abort();
        let _ = task.await;
    }
}

#[tokio::test(start_paused = true)]
async fn refused_nodes_do_not_stop_other_kinds_and_refresh_retries_all_once() {
    let (client, log) = api(false, true);
    let session = session();
    let worker = session.clone();
    let task = tokio::spawn(async move { worker.run(client).await });
    until(|| log.streams.lock().unwrap().len() == 8).await;
    assert!(matches!(
        session.derive(chrono::Utc::now()).summary.nodes,
        Part::Refused(_)
    ));
    assert!(session.derive(chrono::Utc::now()).summary.pods.is_current());
    let first = log.calls.lock().unwrap().len();
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    assert_eq!(log.calls.lock().unwrap().len(), first);
    session.relist();
    session.relist();
    session.relist();
    until(|| {
        log.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|u| u.as_str() == "/version")
            .count()
            == 2
    })
    .await;
    until(|| session.derive(chrono::Utc::now()).summary.pods.is_current()).await;
    assert_eq!(
        log.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|u| u.starts_with("/api/v1/nodes?"))
            .count(),
        2
    );
    task.abort();
    let _ = task.await;
}
