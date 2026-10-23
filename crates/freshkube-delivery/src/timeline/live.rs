//! The live reads of the sources that come from the open cluster: Warning
//! events, pod restarts, Deployment rollouts and node changes, for the
//! application's workloads. Each is a bounded listing through a [`Reader`],
//! which can only read: a namespace at a time (at most [`MAX_NAMESPACES`]),
//! each at most [`MAX_PAGES`](crate::read::MAX_PAGES) pages. A listing that
//! fails leaves its source not read, with the failure's kind and the
//! collection, and the others stand. A listing cut at the page cap, and
//! namespaces past the cap, leave the source read in part: its `cut` says
//! what, and it speaks for no quiet hour. Restarts, Nodes and Deploys show
//! only the present state (a container keeps its last termination, a node
//! its current Ready transition, a rollback reuses an old ReplicaSet), so
//! they are `state_only` and vouch for no quiet hour either.
//!
//! An object is the application's by what owns it, never by its name's
//! start: a pod through its ReplicaSet to the Deployment (or directly its
//! StatefulSet or DaemonSet), an event through its involved object's kind and
//! name (a pod that is gone through its ReplicaSet's name). Kargo, Argo CD
//! and Coroot are not read here yet: their sources say so.
use std::collections::HashSet;

use chrono::DateTime;
use freshkube_core::workloads::WorkloadKind;
use serde_json::Value;

use super::{EVENT_RETENTION, Item, Severity, Source, SourceRead};
use crate::read::{ListRequest, Reader, Resource, Scope};
use freshkube_core::resources::Failure;

/// Namespaces read at most; the rest are named in the sources' words.
pub const MAX_NAMESPACES: usize = 5;

/// Why the sources this version doesn't read yet say.
pub const NOT_YET: &str = "not read yet";

/// One of the application's workloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workload {
    pub name: String,
    pub kind: WorkloadKind,
}

impl Workload {
    pub fn new(name: &str, kind: WorkloadKind) -> Self {
        Self {
            name: name.to_owned(),
            kind,
        }
    }
}

/// Which objects of the open cluster the application is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Place {
    /// The cluster's name, for "read from".
    pub cluster: String,
    /// The workloads' namespaces, each with its workloads.
    pub workloads: Vec<(String, Vec<Workload>)>,
}

/// What belongs to the application in one namespace, from its owners.
struct Owned<'a> {
    workloads: &'a [Workload],
    /// ReplicaSets a Deployment of the application controls.
    replicasets: HashSet<String>,
    /// Pods one of its ReplicaSets, StatefulSets or DaemonSets controls.
    pods: HashSet<String>,
    /// Every pod the namespace's listing held, whoever owns it; `None` when
    /// the listing failed.
    listed: Option<HashSet<String>>,
}

fn kind_name(kind: WorkloadKind) -> &'static str {
    match kind {
        WorkloadKind::Deployment => "Deployment",
        WorkloadKind::StatefulSet => "StatefulSet",
        WorkloadKind::DaemonSet => "DaemonSet",
    }
}

/// The controlling owner's kind and name.
fn controller(object: &Value) -> Option<(&str, &str)> {
    object
        .pointer("/metadata/ownerReferences")?
        .as_array()?
        .iter()
        .find(|owner| owner.get("controller").and_then(Value::as_bool) == Some(true))
        .and_then(|owner| Some((text(owner, &["kind"])?, text(owner, &["name"])?)))
}

/// `name` is `base`, a dash and one more segment: a pod of a StatefulSet
/// (`db-0`), a DaemonSet (`agent-x2k4`) or a ReplicaSet (`api-7d9f-x2k4`
/// under `api-7d9f`).
fn one_segment_after(name: &str, base: &str) -> bool {
    name.strip_prefix(base)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|rest| !rest.is_empty() && !rest.contains('-'))
}

impl<'a> Owned<'a> {
    fn new(workloads: &'a [Workload], replicasets: &[Value], pods: Option<&[Value]>) -> Self {
        let listed = pods.map(|pods| {
            pods.iter()
                .filter_map(|pod| text(pod, &["metadata", "name"]).map(str::to_owned))
                .collect()
        });
        let pods = pods.unwrap_or_default();
        let has = |kind: &str, name: &str| {
            workloads
                .iter()
                .any(|workload| kind_name(workload.kind) == kind && workload.name == name)
        };
        let replicasets: HashSet<String> = replicasets
            .iter()
            .filter(|set| controller(set).is_some_and(|(kind, name)| has(kind, name)))
            .filter_map(|set| text(set, &["metadata", "name"]).map(str::to_owned))
            .collect();
        let pods = pods
            .iter()
            .filter(|pod| {
                controller(pod).is_some_and(|(kind, name)| {
                    if kind == "ReplicaSet" {
                        replicasets.contains(name)
                    } else {
                        has(kind, name)
                    }
                })
            })
            .filter_map(|pod| text(pod, &["metadata", "name"]).map(str::to_owned))
            .collect();
        Self {
            workloads,
            replicasets,
            pods,
            listed,
        }
    }

    /// An event's object is the application's, or one of its pods.
    fn owns_event_object(&self, kind: &str, name: &str) -> bool {
        match kind {
            "ReplicaSet" => self.replicasets.contains(name),
            "Pod" => {
                self.pods.contains(name)
                    // A pod that is gone is the application's by its name's
                    // shape, but one that was listed is whoever's it is.
                    || self.listed.as_ref().is_some_and(|listed| !listed.contains(name))
                        && (self
                            .replicasets
                            .iter()
                            .any(|set| one_segment_after(name, set))
                            || self.workloads.iter().any(|workload| {
                                workload.kind != WorkloadKind::Deployment
                                    && one_segment_after(name, &workload.name)
                            }))
            }
            _ => self
                .workloads
                .iter()
                .any(|workload| kind_name(workload.kind) == kind && workload.name == name),
        }
    }
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

    let (mut events, mut restarts, mut deploys, mut nodes) = (
        Part::default(),
        Part::default(),
        Part::default(),
        Part::default(),
    );
    let mut hosts: HashSet<String> = HashSet::new();
    let mut oldest_event: Option<i64> = None;
    for (namespace, workloads) in &namespaces {
        let (event_list, pod_list, set_list) = futures::join!(
            list(reader, events_resource(), namespace, "events"),
            list(reader, pods_resource(), namespace, "pods"),
            list(reader, replicasets_resource(), namespace, "replicasets"),
        );
        let no_items = Vec::new();
        let pods_items = pod_list.as_ref().map_or(&no_items, |listed| &listed.items);
        let sets_items = set_list.as_ref().map_or(&no_items, |listed| &listed.items);
        let owned = Owned::new(
            workloads,
            sets_items,
            pod_list.as_ref().ok().map(|listed| listed.items.as_slice()),
        );
        // What couldn't be listed leaves what hangs on it unknown.
        if pod_list.is_err() {
            nodes
                .cuts
                .push(format!("hosts unknown: pods not listed in {namespace}"));
        }
        if set_list.is_err() && workloads.iter().any(|w| w.kind == WorkloadKind::Deployment) {
            let note =
                format!("ReplicaSets not listed in {namespace}: Deployments' pods not matched");
            // Nodes inherits the Restarts' cuts below.
            for part in [&mut events, &mut restarts] {
                part.cuts.push(note.clone());
            }
        }
        if let Ok(listed) = &event_list {
            let oldest = listed.items.iter().filter_map(event_time).min();
            oldest_event = oldest_event.into_iter().chain(oldest).min();
        }
        hosts.extend(hosting(pods_items, &owned));
        events.take(event_list, |list| warnings(list, &owned, since));
        restarts.take(pod_list, |list| restarted(list, &owned, since));
        deploys.take(set_list, |list| rollouts(list, &owned, since));
    }
    // The hosts are the pods' nodes: a pod list that was cut leaves some
    // unknown.
    nodes.cuts.extend(restarts.cuts.iter().cloned());
    nodes.take(list(reader, nodes_resource(), "", "nodes").await, |list| {
        node_changes(list, &hosts, since)
    });

    // Namespaces past the cap were not asked, which cuts every source that
    // reads by namespace.
    if skipped > 0 {
        let note = format!(
            "only the first {MAX_NAMESPACES} of {} namespaces were read",
            scope.workloads.len()
        );
        for part in [&mut events, &mut restarts, &mut deploys, &mut nodes] {
            part.cuts.push(note.clone());
        }
    }
    // The cluster's retention is at least what its oldest event shows.
    let kept_from = oldest_event.map_or(now - EVENT_RETENTION, |oldest| {
        oldest.min(now - EVENT_RETENTION)
    });
    let read = |source, part: Part| {
        let cut = (!part.cuts.is_empty()).then(|| part.cuts.join("; "));
        SourceRead {
            source,
            from: from.to_owned(),
            read_at: now,
            outcome: part.items,
            kept_from: None,
            cut,
            state_only: false,
        }
    };
    vec![
        not_read(Source::Promotions, now),
        not_read(Source::Syncs, now),
        not_read(Source::Incidents, now),
        read(Source::Deploys, deploys).state_only(),
        read(Source::Events, events).kept_from(kept_from),
        read(Source::Restarts, restarts).state_only(),
        read(Source::Nodes, nodes).state_only(),
    ]
}

fn not_read(source: Source, now: i64) -> SourceRead {
    SourceRead::not_read(source, "", now, NOT_YET)
}

/// The items of one source so far, or why it failed, and what was cut.
struct Part {
    items: Result<Vec<Item>, String>,
    cuts: Vec<String>,
}

impl Default for Part {
    fn default() -> Self {
        Self {
            items: Ok(Vec::new()),
            cuts: Vec::new(),
        }
    }
}

impl Part {
    /// Adds a listing's items, its cut, or its failure, which makes the
    /// whole source not read.
    fn take(&mut self, list: Result<Listed, String>, parse: impl FnOnce(&[Value]) -> Vec<Item>) {
        match (self.items.as_mut(), list) {
            (Ok(items), Ok(listed)) => {
                items.extend(parse(&listed.items));
                self.cuts.extend(listed.cut);
            }
            (Ok(_), Err(why)) => self.items = Err(why),
            (Err(_), _) => {}
        }
    }
}

/// A listing's items, and where it was cut off if it was.
struct Listed {
    items: Vec<Value>,
    cut: Option<String>,
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
) -> Result<Listed, String> {
    let scope = if resource.namespaced {
        Scope::Namespace(namespace.to_owned())
    } else {
        Scope::Cluster
    };
    reader
        .list(&ListRequest { resource, scope })
        .await
        .map(|listing| Listed {
            cut: listing.truncated.map(|cut| {
                let at = if namespace.is_empty() {
                    String::new()
                } else {
                    format!(" in {namespace}")
                };
                format!("cut after {} {what}{at}", cut.read)
            }),
            items: listing.items,
        })
        .map_err(|Failure { kind, .. }| format!("{}: can't list {what}", kind.label()))
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

/// When an event happened, as far as it says.
fn event_time(event: &Value) -> Option<i64> {
    time(event, &["lastTimestamp"])
        .or_else(|| time(event, &["series", "lastObservedTime"]))
        .or_else(|| time(event, &["eventTime"]))
        .or_else(|| time(event, &["metadata", "creationTimestamp"]))
}

fn warnings(list: &[Value], owned: &Owned, since: i64) -> Vec<Item> {
    list.iter()
        .filter(|event| text(event, &["type"]) == Some("Warning"))
        .filter(|event| {
            let kind = text(event, &["involvedObject", "kind"]).unwrap_or_default();
            text(event, &["involvedObject", "name"])
                .is_some_and(|name| owned.owns_event_object(kind, name))
        })
        .filter_map(|event| {
            let at = event_time(event)?;
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

fn restarted(list: &[Value], owned: &Owned, since: i64) -> Vec<Item> {
    let mut items = Vec::new();
    for pod in list {
        let Some(name) = text(pod, &["metadata", "name"]).filter(|name| owned.pods.contains(*name))
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

fn rollouts(list: &[Value], owned: &Owned, since: i64) -> Vec<Item> {
    list.iter()
        .filter(|set| {
            text(set, &["metadata", "name"]).is_some_and(|n| owned.replicasets.contains(n))
        })
        .filter_map(|set| {
            let at = time(set, &["metadata", "creationTimestamp"]).filter(|at| *at >= since)?;
            let (_, deployment) = controller(set)?;
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
fn hosting(list: &[Value], owned: &Owned) -> Vec<String> {
    list.iter()
        .filter(|pod| {
            text(pod, &["metadata", "name"]).is_some_and(|name| owned.pods.contains(name))
        })
        .filter_map(|pod| text(pod, &["spec", "nodeName"]).map(str::to_owned))
        .collect()
}

fn node_changes(list: &[Value], hosts: &HashSet<String>, since: i64) -> Vec<Item> {
    list.iter()
        .filter(|node| text(node, &["metadata", "name"]).is_some_and(|name| hosts.contains(name)))
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
