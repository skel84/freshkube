//! List-then-watch of one collection, delivered in batches.

use std::time::Duration;

use futures::StreamExt;
use kube::Client;
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::failure::Failure;
use super::kinds::ResourceKind;
use super::table::{Table, TableColumn, TableRow, list_table, watch_table};

mod batch;

use batch::{Batcher, map_event};

/// How long events are collected before they're delivered as one batch, so a
/// frontend rebuilds at most about once per frame however fast changes arrive.
const BATCH_WINDOW: Duration = Duration::from_millis(16);
const MAX_BATCH: usize = 512;
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// A watch the server ends sooner than this is waited on before resuming, so
/// a server or proxy that closes watches at once isn't hammered.
const MIN_WATCH: Duration = Duration::from_secs(1);

/// Events from one watch, in order, delivered together.
#[derive(Clone, Debug)]
pub struct WatchBatch {
    pub events: Vec<WatchEvent>,
}

#[derive(Clone, Debug)]
pub enum WatchEvent {
    /// A complete list: replace everything with these rows and columns.
    Reset {
        columns: Vec<TableColumn>,
        rows: Vec<TableRow>,
    },
    Upsert(TableRow),
    Delete(TableRow),
    /// The list or watch failed, and rows already delivered are now stale.
    /// When `retrying`, it lists again after a backoff; otherwise the failure
    /// is permanent (403, 404) and the watch has stopped.
    Failed {
        failure: Failure,
        retrying: bool,
    },
}

/// Lists and then watches a collection, across all namespaces when
/// `namespace` is `None`, until `sink` closes or a failure is permanent.
/// Transient failures are retried with backoff; an expired watch (410) is
/// recovered by listing again. Run it on Tokio and abort it to stop.
pub async fn watch_collection(
    client: Client,
    kind: ResourceKind,
    namespace: Option<String>,
    field_selector: Option<String>,
    mut sink: mpsc::Sender<WatchBatch>,
) {
    let namespace = namespace.as_deref();
    let field_selector = field_selector.as_deref();
    let mut backoff = MIN_BACKOFF;
    loop {
        let table = match list_table(&client, &kind, namespace, field_selector).await {
            Ok(table) => table,
            Err(failure) => {
                if !fail(&mut sink, failure, &mut backoff).await {
                    return;
                }
                continue;
            }
        };
        let mut resource_version = table.metadata.resource_version.clone();
        let Table {
            column_definitions,
            rows,
            ..
        } = table;
        let reset = WatchEvent::Reset {
            columns: column_definitions,
            rows,
        };
        if !send(&mut sink, vec![reset]).await {
            return;
        }
        // Watch until the version expires or the watch fails, then list again.
        loop {
            let started = Instant::now();
            match watch_once(
                &client,
                &kind,
                namespace,
                field_selector,
                &mut resource_version,
                &mut sink,
            )
            .await
            {
                WatchEnd::Resume => {
                    if started.elapsed() < MIN_WATCH {
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                    } else {
                        backoff = MIN_BACKOFF;
                    }
                }
                WatchEnd::Relist => break,
                WatchEnd::Failed(failure) => {
                    if !fail(&mut sink, failure, &mut backoff).await {
                        return;
                    }
                    break;
                }
                WatchEnd::Closed => return,
            }
        }
    }
}

/// Reports a failure, then waits out the backoff. Returns false when the
/// watch should stop: the failure is permanent or nobody is listening.
async fn fail(
    sink: &mut mpsc::Sender<WatchBatch>,
    failure: Failure,
    backoff: &mut Duration,
) -> bool {
    let retrying = !failure.kind.is_permanent();
    if !send(sink, vec![WatchEvent::Failed { failure, retrying }]).await || !retrying {
        return false;
    }
    tokio::time::sleep(*backoff).await;
    *backoff = (*backoff * 2).min(MAX_BACKOFF);
    true
}

enum WatchEnd {
    /// The server ended the watch normally; resume from the last version.
    Resume,
    /// The version expired (410); list again.
    Relist,
    Failed(Failure),
    /// The receiver is gone; stop.
    Closed,
}

async fn watch_once(
    client: &Client,
    kind: &ResourceKind,
    namespace: Option<&str>,
    field_selector: Option<&str>,
    resource_version: &mut String,
    sink: &mut mpsc::Sender<WatchBatch>,
) -> WatchEnd {
    let stream = match watch_table(client, kind, namespace, field_selector, resource_version).await
    {
        Ok(stream) => stream,
        Err(failure) => return WatchEnd::Failed(failure),
    };
    let mut stream = std::pin::pin!(stream);
    let mut batcher = Batcher::new(BATCH_WINDOW, MAX_BATCH);
    loop {
        let item = match batcher.deadline() {
            Some(at) => tokio::select! {
                item = stream.next() => Some(item),
                _ = tokio::time::sleep_until(at) => None,
            },
            None => Some(stream.next().await),
        };
        let end = match item {
            // The batch window elapsed.
            None => None,
            Some(None) => Some(WatchEnd::Resume),
            Some(Some(item)) => map_event(item, resource_version, batcher.pending_mut()),
        };
        if let Some(batch) = batcher.flush(Instant::now(), end.is_some())
            && !send(sink, batch).await
        {
            return WatchEnd::Closed;
        }
        if let Some(end) = end {
            return end;
        }
    }
}

/// Returns false when the receiver has gone away.
async fn send(sink: &mut mpsc::Sender<WatchBatch>, events: Vec<WatchEvent>) -> bool {
    sink.send(WatchBatch { events }).await.is_ok()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::resources::{FailureKind, builtin};
    use http::{Request, Response};
    use kube::client::Body;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    /// A fake API server: `respond` answers each request URI with a status
    /// and body, and every URI asked is recorded.
    pub(crate) fn server(
        respond: impl Fn(&str) -> (u16, String) + Send + Sync + 'static,
    ) -> (Client, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let service = tower::service_fn(move |request: Request<Body>| {
            let uri = request.uri().to_string();
            log.lock().unwrap().push(uri.clone());
            let (status, body) = respond(&uri);
            async move {
                Ok::<_, std::convert::Infallible>(
                    Response::builder()
                        .status(status)
                        .body(Body::from(body.into_bytes()))
                        .unwrap(),
                )
            }
        });
        (Client::new(service, "default"), seen)
    }

    fn row(name: &str, version: &str) -> serde_json::Value {
        json!({"cells": [name, "Running"], "object": {"kind": "PartialObjectMetadata",
            "metadata": {"name": name, "namespace": "default", "uid": format!("uid-{name}"),
                "resourceVersion": version}}})
    }

    pub(crate) fn list(version: &str, next: &str, names: &[&str]) -> String {
        json!({"kind": "Table", "apiVersion": "meta.k8s.io/v1",
            "metadata": {"resourceVersion": version, "continue": next},
            "columnDefinitions": [
                {"name": "Name", "type": "string", "format": "name", "description": "", "priority": 0},
                {"name": "Status", "type": "string", "format": "", "description": "", "priority": 0}],
            "rows": names.iter().map(|name| row(name, version)).collect::<Vec<_>>()})
        .to_string()
    }

    fn event(kind: &str, name: &str, version: &str) -> String {
        json!({"type": kind, "object": {"kind": "Table", "apiVersion": "meta.k8s.io/v1",
            "metadata": {"resourceVersion": version}, "columnDefinitions": null,
            "rows": [row(name, version)]}})
        .to_string()
    }

    fn status(code: u16, reason: &str) -> String {
        json!({"kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Failure",
            "message": format!("{reason} for test"), "reason": reason, "code": code})
        .to_string()
    }

    fn names(rows: &[TableRow]) -> Vec<String> {
        rows.iter()
            .map(|row| row.metadata().unwrap().name.clone())
            .collect()
    }

    /// Every event until the watch stops, flattened across batches.
    async fn drain(mut receiver: mpsc::Receiver<WatchBatch>) -> Vec<WatchEvent> {
        let mut events = Vec::new();
        while let Some(batch) = receiver.recv().await {
            assert!(!batch.events.is_empty(), "batches are never empty");
            events.extend(batch.events);
        }
        events
    }

    #[tokio::test(start_paused = true)]
    async fn field_selector_survives_pagination_watch_and_expiration() {
        let lists = Arc::new(Mutex::new(0));
        let count = lists.clone();
        let (client, seen) = server(move |uri| {
            if uri.contains("watch=1") {
                return if uri.contains("resourceVersion=10") {
                    (410, status(410, "Expired"))
                } else {
                    (403, status(403, "Forbidden"))
                };
            }
            if uri.contains("continue=") {
                return (200, list("10", "", &["b"]));
            }
            let mut count = count.lock().unwrap();
            *count += 1;
            if *count == 1 {
                (200, list("10", "next", &["a"]))
            } else {
                (200, list("20", "", &["c"]))
            }
        });
        let (sender, receiver) = mpsc::channel(8);
        tokio::spawn(watch_collection(
            client,
            builtin("pods").unwrap(),
            None,
            Some("spec.nodeName=worker+/01".into()),
            sender,
        ));
        let events = drain(receiver).await;
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, WatchEvent::Reset { .. }))
                .count(),
            2
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 5);
        assert!(
            seen.iter()
                .all(|uri| uri.contains("fieldSelector=spec.nodeName%3Dworker%2B%2F01")),
            "{seen:?}"
        );
    }

    #[tokio::test]
    async fn list_table_follows_encoded_continue_tokens() {
        let (client, seen) = server(|uri| {
            if uri.contains("continue=") {
                (200, list("7", "", &["c"]))
            } else {
                (200, list("5", "tok+/=", &["a", "b"]))
            }
        });
        let pods = builtin("pods").unwrap();
        let table = list_table(&client, &pods, Some("default"), None)
            .await
            .unwrap();
        assert_eq!(names(&table.rows), ["a", "b", "c"]);
        assert_eq!(table.metadata.resource_version, "7");
        assert_eq!(table.column_definitions.len(), 2);
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen[0],
            "/api/v1/namespaces/default/pods?includeObject=Object&limit=500"
        );
        assert!(seen[1].ends_with("&continue=tok%2B%2F%3D"), "{}", seen[1]);
    }

    #[tokio::test(start_paused = true)]
    async fn watch_streams_changes_relists_after_410_and_stops_when_refused() {
        let lists = Arc::new(Mutex::new(0));
        let counter = lists.clone();
        let (client, seen) = server(move |uri| {
            if !uri.contains("watch=1") {
                let mut lists = counter.lock().unwrap();
                *lists += 1;
                return if *lists == 1 {
                    (200, list("10", "", &["a"]))
                } else {
                    (200, list("20", "", &["a", "c"]))
                };
            }
            if uri.contains("resourceVersion=10") {
                let lines = [
                    event("ADDED", "b", "11"),
                    event("MODIFIED", "a", "12"),
                    event("DELETED", "b", "13"),
                    json!({"type": "ERROR", "object": serde_json::from_str::<serde_json::Value>(
                        &status(410, "Expired")).unwrap()})
                    .to_string(),
                ];
                (200, lines.join("\n"))
            } else {
                (403, status(403, "Forbidden"))
            }
        });
        let (sender, receiver) = mpsc::channel(8);
        tokio::spawn(watch_collection(
            client,
            builtin("pods").unwrap(),
            None,
            None,
            sender,
        ));
        let events = drain(receiver).await;
        let summary: Vec<String> = events
            .iter()
            .map(|event| match event {
                WatchEvent::Reset { rows, .. } => format!("reset {}", names(rows).join(",")),
                WatchEvent::Upsert(row) => format!("upsert {}", row.metadata().unwrap().name),
                WatchEvent::Delete(row) => format!("delete {}", row.metadata().unwrap().name),
                WatchEvent::Failed { failure, retrying } => {
                    format!("failed {:?} retrying={retrying}", failure.kind)
                }
            })
            .collect();
        assert_eq!(
            summary,
            [
                "reset a",
                "upsert b",
                "upsert a",
                "delete b",
                // The expired version relists without reporting a failure.
                "reset a,c",
                "failed Forbidden retrying=false",
            ]
        );
        let seen = seen.lock().unwrap();
        let watches: Vec<_> = seen.iter().filter(|uri| uri.contains("watch=1")).collect();
        assert_eq!(watches.len(), 2, "a refused watch is not retried");
        assert!(watches[0].starts_with("/api/v1/pods?watch=1"));
        assert!(watches[0].contains("resourceVersion=10&timeoutSeconds=290"));
        assert!(watches[1].contains("resourceVersion=20"));
    }

    /// A burst followed by a 410 arrives as one batch, since the 410 ends the
    /// watch, and the list after it as a batch of its own. (The window itself
    /// is covered in `batch.rs`; this checks the batch boundaries.)
    #[tokio::test(start_paused = true)]
    async fn a_burst_is_one_batch_and_a_relist_starts_a_new_one() {
        let lists = Arc::new(Mutex::new(0));
        let counter = lists.clone();
        let (client, _) = server(move |uri| {
            if !uri.contains("watch=1") {
                let mut lists = counter.lock().unwrap();
                *lists += 1;
                return if *lists == 1 {
                    (200, list("10", "", &["a"]))
                } else {
                    (200, list("20", "", &["a", "c"]))
                };
            }
            if uri.contains("resourceVersion=10") {
                let lines = [
                    event("ADDED", "b", "11"),
                    event("MODIFIED", "a", "12"),
                    event("DELETED", "b", "13"),
                    json!({"type": "ERROR", "object": serde_json::from_str::<serde_json::Value>(
                        &status(410, "Expired")).unwrap()})
                    .to_string(),
                ];
                (200, lines.join("\n"))
            } else {
                (403, status(403, "Forbidden"))
            }
        });
        let (sender, mut receiver) = mpsc::channel(8);
        tokio::spawn(watch_collection(
            client,
            builtin("pods").unwrap(),
            None,
            None,
            sender,
        ));
        let mut batches = Vec::new();
        while let Some(batch) = receiver.recv().await {
            let kinds: Vec<&str> = batch
                .events
                .iter()
                .map(|event| match event {
                    WatchEvent::Reset { .. } => "reset",
                    WatchEvent::Upsert(_) => "upsert",
                    WatchEvent::Delete(_) => "delete",
                    WatchEvent::Failed { .. } => "failed",
                })
                .collect();
            batches.push(kinds.join(","));
        }
        assert_eq!(
            batches,
            ["reset", "upsert,upsert,delete", "reset", "failed"]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn transient_failures_are_reported_then_retried() {
        let attempts = Arc::new(Mutex::new(0));
        let counter = attempts.clone();
        let (client, _) = server(move |uri| {
            if uri.contains("watch=1") {
                return (403, status(403, "Forbidden"));
            }
            let mut attempts = counter.lock().unwrap();
            *attempts += 1;
            if *attempts == 1 {
                (503, status(503, "ServiceUnavailable"))
            } else {
                (200, list("3", "", &["a"]))
            }
        });
        let (sender, receiver) = mpsc::channel(8);
        tokio::spawn(watch_collection(
            client,
            builtin("pods").unwrap(),
            None,
            None,
            sender,
        ));
        let events = drain(receiver).await;
        assert!(matches!(
            &events[0],
            WatchEvent::Failed { failure, retrying: true } if failure.kind == FailureKind::Other
        ));
        assert!(matches!(&events[1], WatchEvent::Reset { rows, .. } if names(rows) == ["a"]));
        assert_eq!(*attempts.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn a_dropped_receiver_stops_the_watch() {
        let (client, seen) = server(|uri| {
            if uri.contains("watch=1") {
                (200, event("ADDED", "b", "2"))
            } else {
                (200, list("1", "", &["a"]))
            }
        });
        let (sender, mut receiver) = mpsc::channel(8);
        let task = tokio::spawn(watch_collection(
            client,
            builtin("pods").unwrap(),
            None,
            None,
            sender,
        ));
        assert!(receiver.recv().await.is_some());
        drop(receiver);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the watch ends once nobody listens")
            .unwrap();
        assert!(seen.lock().unwrap().len() <= 3);
    }
}
