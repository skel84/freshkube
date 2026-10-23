//! The live reads of the sources that come from the open cluster: Warning
//! events, pod restarts, Deployment rollouts and node changes, for the
//! application's workloads. Each is a bounded listing through a [`Reader`],
//! which can only read: a namespace at a time (at most [`MAX_NAMESPACES`]),
//! each at most [`MAX_PAGES`](crate::read::MAX_PAGES) pages. A listing that
//! fails leaves its source not read, with the failure's kind and the
//! collection, and the others stand. Kargo, Argo CD and Coroot are not read
//! here yet: their sources say so.
use chrono::DateTime;
use serde_json::Value;

use super::{EVENT_RETENTION, Item, Severity, Source, SourceRead};
use crate::read::{ListRequest, Reader, Resource, Scope};
use freshkube_core::resources::Failure;

/// Namespaces read at most; the rest are named in the sources' words.
pub const MAX_NAMESPACES: usize = 5;

/// Why the sources this version doesn't read yet say.
pub const NOT_YET: &str = "not read yet";

/// Which objects of the open cluster the application is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Place {
    /// The cluster's name, for "read from".
    pub cluster: String,
    /// The workloads' namespaces, each with the names of its workloads.
    pub workloads: Vec<(String, Vec<String>)>,
}

/// Reads every cluster source for the window of `hours` ending at `now`.
pub async fn read(reader: &impl Reader, scope: &Place, now: i64, hours: u32) -> Vec<SourceRead> {
    let since = now - i64::from(hours) * 3600;
    let from = scope.cluster.as_str();
    let namespaces: Vec<_> = scope.workloads.iter().take(MAX_NAMESPACES).collect();
    let skipped = scope.workloads.len().saturating_sub(MAX_NAMESPACES);
    if namespaces.is_empty() {
        return Source::ALL
            .into_iter()
            .map(|source| {
                let why = match source {
                    Source::Promotions | Source::Syncs | Source::Incidents => NOT_YET,
                    _ => "the application has no workload in this cluster",
                };
                SourceRead::not_read(source, from, now, why)
            })
            .collect();
    }

    let mut events: Result<Vec<Item>, String> = Ok(Vec::new());
    let mut restarts: Result<Vec<Item>, String> = Ok(Vec::new());
    let mut deploys: Result<Vec<Item>, String> = Ok(Vec::new());
    let mut hosts: Vec<String> = Vec::new();
    for (namespace, workloads) in &namespaces {
        let (event_list, pod_list, set_list) = futures::join!(
            list(reader, events_resource(), namespace, "events"),
            list(reader, pods_resource(), namespace, "pods"),
            list(reader, replicasets_resource(), namespace, "replicasets"),
        );
        extend(&mut events, event_list, |list| {
            warnings(list, workloads, since)
        });
        extend(&mut deploys, set_list, |list| {
            rollouts(list, workloads, since)
        });
        match pod_list {
            Ok(list) => {
                if let Ok(items) = &mut restarts {
                    items.extend(restarted(&list, workloads, since));
                }
                hosts.extend(hosting(&list, workloads));
            }
            Err(why) => restarts = Err(why),
        }
    }
    hosts.sort();
    hosts.dedup();
    let nodes = list(reader, nodes_resource(), "", "nodes")
        .await
        .map(|list| node_changes(&list, &hosts, since));

    let from = if skipped > 0 {
        format!("{from}, the first {MAX_NAMESPACES} namespaces")
    } else {
        from.to_owned()
    };
    let read = |source, outcome: Result<Vec<Item>, String>| SourceRead {
        source,
        from: from.clone(),
        read_at: now,
        outcome,
        kept_from: None,
    };
    let reads = vec![
        not_read(Source::Promotions, now),
        not_read(Source::Syncs, now),
        not_read(Source::Incidents, now),
        read(Source::Deploys, deploys),
        read(Source::Events, events).kept_from(now - EVENT_RETENTION),
        read(Source::Restarts, restarts),
        read(Source::Nodes, nodes),
    ];
    reads
}

fn not_read(source: Source, now: i64) -> SourceRead {
    SourceRead::not_read(source, "", now, NOT_YET)
}

fn extend(
    into: &mut Result<Vec<Item>, String>,
    list: Result<Vec<Value>, String>,
    parse: impl FnOnce(&[Value]) -> Vec<Item>,
) {
    match (into.as_mut(), list) {
        (Ok(items), Ok(list)) => items.extend(parse(&list)),
        (Ok(_), Err(why)) => *into = Err(why),
        (Err(_), _) => {}
    }
}

fn events_resource() -> Resource {
    Resource::new("", "v1", "events", true)
}

fn pods_resource() -> Resource {
    Resource::new("", "v1", "pods", true)
}

fn replicasets_resource() -> Resource {
    Resource::new("apps", "v1", "replicasets", true)
}

fn nodes_resource() -> Resource {
    Resource::new("", "v1", "nodes", false)
}

async fn list(
    reader: &impl Reader,
    resource: Resource,
    namespace: &str,
    what: &str,
) -> Result<Vec<Value>, String> {
    let scope = if resource.namespaced {
        Scope::Namespace(namespace.to_owned())
    } else {
        Scope::Cluster
    };
    reader
        .list(&ListRequest { resource, scope })
        .await
        .map(|listing| listing.items)
        .map_err(|Failure { kind, .. }| format!("{}: can't list {what}", kind.label()))
}

/// Whether an object's name is one of the workloads' or a child's.
fn belongs(name: &str, workloads: &[String]) -> bool {
    workloads
        .iter()
        .any(|workload| name == workload || name.starts_with(&format!("{workload}-")))
}

fn text<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |value, key| value.get(key))?
        .as_str()
}

fn time(value: &Value, path: &[&str]) -> Option<i64> {
    DateTime::parse_from_rfc3339(text(value, path)?)
        .ok()
        .map(|time| time.timestamp())
}

fn warnings(list: &[Value], workloads: &[String], since: i64) -> Vec<Item> {
    list.iter()
        .filter(|event| text(event, &["type"]) == Some("Warning"))
        .filter(|event| {
            text(event, &["involvedObject", "name"]).is_some_and(|name| belongs(name, workloads))
        })
        .filter_map(|event| {
            let at = time(event, &["lastTimestamp"])
                .or_else(|| time(event, &["series", "lastObservedTime"]))
                .or_else(|| time(event, &["eventTime"]))
                .or_else(|| time(event, &["metadata", "creationTimestamp"]))?;
            (at >= since).then(|| {
                let name = text(event, &["involvedObject", "name"]).unwrap_or_default();
                let count = event.get("count").and_then(Value::as_u64).unwrap_or(1);
                let message = text(event, &["message"]).unwrap_or_default();
                Item {
                    at,
                    severity: Severity::Warn,
                    title: format!("{}: {name}", text(event, &["reason"]).unwrap_or("Warning")),
                    detail: if count > 1 {
                        format!("{message} (×{count})")
                    } else {
                        message.to_owned()
                    },
                }
            })
        })
        .collect()
}

fn restarted(list: &[Value], workloads: &[String], since: i64) -> Vec<Item> {
    let mut items = Vec::new();
    for pod in list {
        let Some(name) = text(pod, &["metadata", "name"]).filter(|name| belongs(name, workloads))
        else {
            continue;
        };
        let statuses = pod
            .pointer("/status/containerStatuses")
            .and_then(Value::as_array);
        for status in statuses.into_iter().flatten() {
            let restarts = status
                .get("restartCount")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let ended = status.pointer("/lastState/terminated");
            let Some(at) = ended.and_then(|ended| time(ended, &["finishedAt"])) else {
                continue;
            };
            if restarts == 0 || at < since {
                continue;
            }
            let reason = ended
                .and_then(|ended| text(ended, &["reason"]))
                .unwrap_or("terminated");
            let container = text(status, &["name"]).unwrap_or_default();
            items.push(Item {
                at,
                severity: if reason == "Completed" {
                    Severity::Warn
                } else {
                    Severity::Bad
                },
                title: format!("{name} restarted"),
                detail: format!("Container {container}, {reason}, {restarts} restarts"),
            });
        }
    }
    items
}

fn rollouts(list: &[Value], workloads: &[String], since: i64) -> Vec<Item> {
    list.iter()
        .filter_map(|set| {
            let at = time(set, &["metadata", "creationTimestamp"]).filter(|at| *at >= since)?;
            let owner = set
                .pointer("/metadata/ownerReferences")?
                .as_array()?
                .iter()
                .find(|owner| {
                    text(owner, &["kind"]) == Some("Deployment")
                        && owner.get("controller").and_then(Value::as_bool) == Some(true)
                })?;
            let deployment =
                text(owner, &["name"]).filter(|name| workloads.contains(&name.to_string()))?;
            let name = text(set, &["metadata", "name"]).unwrap_or_default();
            let to = match (
                text(set, &["metadata", "labels", "app.kubernetes.io/version"]),
                set.pointer("/metadata/annotations")
                    .and_then(|a| a.get("deployment.kubernetes.io/revision"))
                    .and_then(Value::as_str),
            ) {
                (Some(version), _) => version.to_owned(),
                (None, Some(revision)) => format!("revision {revision}"),
                (None, None) => name.to_owned(),
            };
            Some(Item {
                at,
                severity: Severity::Info,
                title: format!("deploy/{deployment} → {to}"),
                detail: format!("ReplicaSet {name} created"),
            })
        })
        .collect()
}

/// The nodes the application's pods run on.
fn hosting(list: &[Value], workloads: &[String]) -> Vec<String> {
    list.iter()
        .filter(|pod| text(pod, &["metadata", "name"]).is_some_and(|name| belongs(name, workloads)))
        .filter_map(|pod| text(pod, &["spec", "nodeName"]).map(str::to_owned))
        .collect()
}

fn node_changes(list: &[Value], hosts: &[String], since: i64) -> Vec<Item> {
    list.iter()
        .filter(|node| {
            text(node, &["metadata", "name"]).is_some_and(|name| hosts.iter().any(|h| h == name))
        })
        .filter_map(|node| {
            let name = text(node, &["metadata", "name"])?;
            let ready = node
                .pointer("/status/conditions")?
                .as_array()?
                .iter()
                .find(|condition| text(condition, &["type"]) == Some("Ready"))?;
            let at = time(ready, &["lastTransitionTime"]).filter(|at| *at >= since)?;
            let is_ready = text(ready, &["status"]) == Some("True");
            Some(Item {
                at,
                severity: if is_ready {
                    Severity::Info
                } else {
                    Severity::Warn
                },
                title: format!("{name} {}", if is_ready { "Ready" } else { "NotReady" }),
                detail: format!("Node {name} runs pods of the application"),
            })
        })
        .collect()
}
