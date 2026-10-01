//! The events recorded about one object, matched by its UID, listed and then
//! watched.

use std::pin::pin;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::StreamExt;
use k8s_openapi::api::core::v1::Event;
use kube::runtime::watcher;
use kube::{Api, Client};
use tokio::sync::mpsc;

use super::failure::{Failure, FailureKind};
use super::kinds::ResourceKind;

const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// One event, reduced to what a reader needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectEvent {
    /// The event's own UID: its identity across updates.
    pub uid: String,
    /// `Normal` or `Warning`.
    pub event_type: String,
    pub reason: String,
    pub message: String,
    /// How many times it was seen; at least 1.
    pub count: u32,
    pub first_seen: Option<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
    /// The component that reported it, with its host when known.
    pub source: String,
    /// The part of the object it concerns, such as a container.
    pub field_path: String,
}

impl ObjectEvent {
    pub fn is_warning(&self) -> bool {
        self.event_type == "Warning"
    }

    fn new(event: Event) -> Self {
        let series = event.series.as_ref();
        let last_seen = series
            .and_then(|series| series.last_observed_time.as_ref())
            .map(|time| time.0)
            .or_else(|| event.last_timestamp.as_ref().map(|time| time.0))
            .or_else(|| event.event_time.as_ref().map(|time| time.0))
            .or_else(|| event.first_timestamp.as_ref().map(|time| time.0))
            .or_else(|| {
                event
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .map(|time| time.0)
            });
        let first_seen = event
            .first_timestamp
            .as_ref()
            .map(|time| time.0)
            .or_else(|| event.event_time.as_ref().map(|time| time.0))
            .or(last_seen);
        let count = series
            .and_then(|series| series.count)
            .or(event.count)
            .and_then(|count| u32::try_from(count).ok())
            .unwrap_or(1)
            .max(1);
        let source = event.source.unwrap_or_default();
        let component = event
            .reporting_component
            .filter(|component| !component.is_empty())
            .or(source.component)
            .unwrap_or_default();
        let host = source
            .host
            .or(event.reporting_instance)
            .filter(|host| !host.is_empty() && *host != component);
        Self {
            uid: event.metadata.uid.unwrap_or_default(),
            event_type: event.type_.unwrap_or_default(),
            reason: event.reason.unwrap_or_default(),
            message: event.message.unwrap_or_default().trim_end().to_owned(),
            count,
            first_seen,
            last_seen,
            source: match host {
                Some(host) if !component.is_empty() => format!("{component} on {host}"),
                Some(host) => host,
                None => component,
            },
            field_path: event.involved_object.field_path.unwrap_or_default(),
        }
    }
}

/// Which events belong to one object, and where to look for them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventScope {
    /// The object's namespace, or every namespace for a cluster-scoped kind,
    /// whose events land in whichever namespace the reporter chose.
    namespace: Option<String>,
    field_selector: String,
    /// UIDs an event may name to be about this object.
    uids: Vec<String>,
}

impl EventScope {
    pub fn new(kind: &ResourceKind, namespace: Option<&str>, name: &str, uid: &str) -> Self {
        let namespace = namespace.filter(|_| kind.namespaced).map(str::to_owned);
        if kind.group.is_empty() && kind.plural == "nodes" {
            // kubelet records a Node's events with the node's name as its UID;
            // other reporters use the real one. Select by name, keep both.
            return Self {
                namespace,
                field_selector: format!("involvedObject.kind=Node,involvedObject.name={name}"),
                uids: vec![uid.to_owned(), name.to_owned()],
            };
        }
        Self {
            namespace,
            field_selector: format!("involvedObject.uid={uid}"),
            uids: vec![uid.to_owned()],
        }
    }

    fn accepts(&self, event: &Event) -> bool {
        event
            .involved_object
            .uid
            .as_ref()
            .is_some_and(|uid| self.uids.contains(uid))
    }
}

/// A change to an object's events, in the shape of a list then watch.
#[derive(Clone, Debug)]
pub enum EventUpdate {
    /// Every event, replacing what was there.
    Reset(Vec<ObjectEvent>),
    Upsert(ObjectEvent),
    /// An event expired or was removed, by its UID.
    Delete(String),
    /// The read failed. When `retrying`, it lists again after a backoff;
    /// otherwise the failure is permanent (403, 404) and the watch stopped.
    Failed {
        failure: Failure,
        retrying: bool,
    },
}

/// Lists and then watches an object's events until `sink` closes or a
/// failure is permanent. Transient failures are reported, then listed again
/// after a backoff. Run it on Tokio and abort it to stop.
pub async fn watch_object_events(
    client: Client,
    scope: EventScope,
    sink: mpsc::Sender<EventUpdate>,
) {
    let api: Api<Event> = match &scope.namespace {
        Some(namespace) => Api::namespaced(client, namespace),
        None => Api::all(client),
    };
    let config = watcher::Config::default().fields(&scope.field_selector);
    let mut backoff = MIN_BACKOFF;
    loop {
        let mut stream = pin!(watcher(api.clone(), config.clone()));
        let mut listed = Vec::new();
        let failure = loop {
            let Some(item) = stream.next().await else {
                return;
            };
            let update = match item {
                Ok(watcher::Event::Init) => {
                    listed.clear();
                    continue;
                }
                Ok(watcher::Event::InitApply(event)) => {
                    if scope.accepts(&event) {
                        listed.push(ObjectEvent::new(event));
                    }
                    continue;
                }
                Ok(watcher::Event::InitDone) => {
                    backoff = MIN_BACKOFF;
                    EventUpdate::Reset(std::mem::take(&mut listed))
                }
                Ok(watcher::Event::Apply(event)) if scope.accepts(&event) => {
                    EventUpdate::Upsert(ObjectEvent::new(event))
                }
                Ok(watcher::Event::Apply(_)) => continue,
                Ok(watcher::Event::Delete(event)) => {
                    EventUpdate::Delete(event.metadata.uid.unwrap_or_default())
                }
                // An expired version: the watcher lists again by itself.
                Err(watcher::Error::WatchError(status)) if status.code == 410 => continue,
                Err(error) => break classify(error),
            };
            if sink.send(update).await.is_err() {
                return;
            }
        };
        let retrying = !failure.kind.is_permanent();
        if sink
            .send(EventUpdate::Failed { failure, retrying })
            .await
            .is_err()
            || !retrying
        {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

fn classify(error: watcher::Error) -> Failure {
    match error {
        watcher::Error::InitialListFailed(error)
        | watcher::Error::WatchStartFailed(error)
        | watcher::Error::WatchFailed(error) => Failure::from_kube(error),
        watcher::Error::WatchError(status) => Failure::from_kube(kube::Error::Api(status)),
        watcher::Error::NoResourceVersion => Failure::new(
            FailureKind::Other,
            "The server sent events without a version",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::builtin;
    use crate::resources::watch::tests::server;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    fn event(
        uid: &str,
        involved: &str,
        reason: &str,
        extra: serde_json::Value,
    ) -> serde_json::Value {
        let mut event = json!({
            "apiVersion": "v1", "kind": "Event",
            "metadata": {"name": format!("e.{uid}"), "namespace": "shop", "uid": uid,
                "resourceVersion": "1", "creationTimestamp": "2026-10-01T09:00:00Z"},
            "involvedObject": {"kind": "Pod", "name": "web-0", "uid": involved,
                "fieldPath": "spec.containers{app}"},
            "reason": reason, "message": format!("{reason} happened\n"), "type": "Normal",
            "source": {"component": "kubelet", "host": "node-1"}
        });
        for (key, value) in extra.as_object().unwrap() {
            event[key] = value.clone();
        }
        event
    }

    fn list(version: &str, items: Vec<serde_json::Value>) -> String {
        json!({"kind": "EventList", "apiVersion": "v1",
            "metadata": {"resourceVersion": version}, "items": items})
        .to_string()
    }

    fn watch_line(kind: &str, object: serde_json::Value) -> String {
        json!({"type": kind, "object": object}).to_string()
    }

    fn forbidden() -> String {
        json!({"kind": "Status", "apiVersion": "v1", "status": "Failure",
            "message": "events is forbidden", "reason": "Forbidden", "code": 403})
        .to_string()
    }

    async fn drain(mut receiver: mpsc::Receiver<EventUpdate>) -> Vec<EventUpdate> {
        let mut updates = Vec::new();
        while let Some(update) = receiver.recv().await {
            updates.push(update);
        }
        updates
    }

    fn summary(updates: &[EventUpdate]) -> Vec<String> {
        updates
            .iter()
            .map(|update| match update {
                EventUpdate::Reset(events) => format!(
                    "reset {}",
                    events
                        .iter()
                        .map(|e| e.reason.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                EventUpdate::Upsert(event) => format!("upsert {} x{}", event.reason, event.count),
                EventUpdate::Delete(uid) => format!("delete {uid}"),
                EventUpdate::Failed { failure, retrying } => {
                    format!("failed {:?} retrying={retrying}", failure.kind)
                }
            })
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn an_objects_events_are_listed_by_uid_then_watched() {
        let watches = Arc::new(Mutex::new(0));
        let counter = watches.clone();
        let (client, seen) = server(move |uri| {
            if !uri.contains("watch=true") {
                return (
                    200,
                    list(
                        "5",
                        vec![
                            event(
                                "e1",
                                "pod-uid",
                                "Scheduled",
                                json!({
                                "lastTimestamp": "2026-10-01T09:01:00Z",
                                "firstTimestamp": "2026-10-01T09:00:30Z", "count": 1}),
                            ),
                            // A newer-style event: a series, no timestamps.
                            event(
                                "e2",
                                "pod-uid",
                                "BackOff",
                                json!({
                                "type": "Warning", "eventTime": "2026-10-01T09:02:00.000000Z",
                                "series": {"count": 4, "lastObservedTime": "2026-10-01T09:05:00.000000Z"},
                                "reportingComponent": "kubelet", "reportingInstance": "node-1"}),
                            ),
                        ],
                    ),
                );
            }
            let mut watches = counter.lock().unwrap();
            *watches += 1;
            if *watches > 1 {
                return (403, forbidden());
            }
            let lines = [
                watch_line(
                    "ADDED",
                    event(
                        "e3",
                        "pod-uid",
                        "Pulled",
                        json!({"metadata": {
                    "name": "e.e3", "namespace": "shop", "uid": "e3", "resourceVersion": "6"}}),
                    ),
                ),
                watch_line(
                    "MODIFIED",
                    event(
                        "e2",
                        "pod-uid",
                        "BackOff",
                        json!({"count": 5,
                    "metadata": {"name": "e.e2", "namespace": "shop", "uid": "e2", "resourceVersion": "7"}}),
                    ),
                ),
                watch_line(
                    "DELETED",
                    event(
                        "e1",
                        "pod-uid",
                        "Scheduled",
                        json!({"metadata": {
                    "name": "e.e1", "namespace": "shop", "uid": "e1", "resourceVersion": "8"}}),
                    ),
                ),
            ];
            (200, lines.join("\n"))
        });
        let scope = EventScope::new(&builtin("pods").unwrap(), Some("shop"), "web-0", "pod-uid");
        let (sender, receiver) = mpsc::channel(8);
        tokio::spawn(watch_object_events(client, scope, sender));
        let updates = drain(receiver).await;
        assert_eq!(
            summary(&updates),
            [
                "reset Scheduled,BackOff",
                "upsert Pulled x1",
                "upsert BackOff x5",
                "delete e1",
                "failed Forbidden retrying=false",
            ]
        );
        let EventUpdate::Reset(events) = &updates[0] else {
            unreachable!()
        };
        let (scheduled, backoff) = (&events[0], &events[1]);
        assert!(!scheduled.is_warning());
        assert_eq!(scheduled.message, "Scheduled happened");
        assert_eq!(scheduled.source, "kubelet on node-1");
        assert_eq!(scheduled.field_path, "spec.containers{app}");
        assert_eq!(
            scheduled.first_seen.unwrap().to_rfc3339(),
            "2026-10-01T09:00:30+00:00"
        );
        assert_eq!(
            scheduled.last_seen.unwrap().to_rfc3339(),
            "2026-10-01T09:01:00+00:00"
        );
        assert!(backoff.is_warning());
        assert_eq!(backoff.count, 4);
        assert_eq!(
            backoff.last_seen.unwrap().to_rfc3339(),
            "2026-10-01T09:05:00+00:00"
        );
        assert_eq!(
            backoff.first_seen.unwrap().to_rfc3339(),
            "2026-10-01T09:02:00+00:00"
        );
        let seen = seen.lock().unwrap();
        assert!(
            seen[0].starts_with("/api/v1/namespaces/shop/events?"),
            "{}",
            seen[0]
        );
        assert!(
            seen[0].contains("fieldSelector=involvedObject.uid%3Dpod-uid"),
            "{}",
            seen[0]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn node_events_match_its_uid_or_its_name_across_namespaces() {
        let (client, seen) = server(|uri| {
            if uri.contains("watch=true") {
                return (403, forbidden());
            }
            let mut kubelet = event("k", "node-1", "NodeReady", json!({}));
            kubelet["involvedObject"] = json!({"kind": "Node", "name": "node-1", "uid": "node-1"});
            let mut controller = event("c", "node-uid", "RegisteredNode", json!({}));
            controller["involvedObject"] =
                json!({"kind": "Node", "name": "node-1", "uid": "node-uid"});
            // An earlier node of the same name.
            let mut old = event("o", "old-uid", "Rebooted", json!({}));
            old["involvedObject"] = json!({"kind": "Node", "name": "node-1", "uid": "old-uid"});
            (200, list("3", vec![kubelet, controller, old]))
        });
        let scope = EventScope::new(
            &builtin("nodes").unwrap(),
            Some("ignored"),
            "node-1",
            "node-uid",
        );
        let (sender, receiver) = mpsc::channel(8);
        tokio::spawn(watch_object_events(client, scope, sender));
        let updates = drain(receiver).await;
        assert_eq!(
            summary(&updates),
            [
                "reset NodeReady,RegisteredNode",
                "failed Forbidden retrying=false"
            ]
        );
        let seen = seen.lock().unwrap();
        assert!(seen[0].starts_with("/api/v1/events?"), "{}", seen[0]);
        assert!(
            seen[0].contains("involvedObject.kind%3DNode%2CinvolvedObject.name%3Dnode-1"),
            "{}",
            seen[0]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_transient_failure_is_reported_then_listed_again() {
        let lists = Arc::new(Mutex::new(0));
        let counter = lists.clone();
        let (client, _) = server(move |uri| {
            if uri.contains("watch=true") {
                return (403, forbidden());
            }
            let mut lists = counter.lock().unwrap();
            *lists += 1;
            if *lists == 1 {
                let status = json!({"kind": "Status", "apiVersion": "v1", "status": "Failure",
                    "message": "unavailable", "code": 503});
                (503, status.to_string())
            } else {
                (
                    200,
                    list("2", vec![event("a", "pod-uid", "Started", json!({}))]),
                )
            }
        });
        let scope = EventScope::new(&builtin("pods").unwrap(), Some("shop"), "web-0", "pod-uid");
        let (sender, receiver) = mpsc::channel(8);
        tokio::spawn(watch_object_events(client, scope, sender));
        let updates = drain(receiver).await;
        assert_eq!(
            summary(&updates),
            [
                "failed Other retrying=true",
                "reset Started",
                "failed Forbidden retrying=false"
            ]
        );
        assert_eq!(*lists.lock().unwrap(), 2);
    }
}
