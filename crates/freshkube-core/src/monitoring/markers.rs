//! Events drawn across a dashboard's charts: Deployments rolling out a new
//! ReplicaSet, nodes turning NotReady or Ready, and node reboots.
//!
//! Each comes from state, not logs: a ReplicaSet's creation time and
//! revision, a node's Ready condition and its events, and with Talos the
//! node's boot time. Every read is a list (metadata only for ReplicaSets)
//! served from the API server's cache, and a part that is refused or fails
//! leaves the others standing.
use std::time::Duration;

use chrono::{DateTime, Utc};
use k8s_openapi::api::apps::v1::ReplicaSet;
use k8s_openapi::api::core::v1::{Event, Node};
use kube::api::{Api, ListParams, ObjectList};
use kube::core::PartialObjectMeta;
use talos_rs::TalosClient;

/// How long each list or boot-time read may take.
const DEADLINE: Duration = Duration::from_secs(8);

/// Markers kept at most, the newest.
pub const MAX_MARKERS: usize = 200;

/// Two node markers of one kind closer than this are one.
const SAME_NODE_EVENT: i64 = 120;

/// A reboot's boot time and the kubelet's event after it are one.
const SAME_REBOOT: i64 = 300;

const REVISION: &str = "deployment.kubernetes.io/revision";
const VERSION: &str = "app.kubernetes.io/version";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MarkerKind {
    /// A Deployment's new ReplicaSet.
    Deploy,
    NodeNotReady,
    /// Ready again, or for the first time.
    NodeReady,
    NodeReboot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub kind: MarkerKind,
    /// Unix seconds.
    pub at: i64,
    /// The Deployment's namespace.
    pub namespace: Option<String>,
    /// The node, for node markers.
    pub node: Option<String>,
    /// "deploy/api → 1.8.2", "worker-2 NotReady".
    pub label: String,
}

impl Marker {
    fn node(kind: MarkerKind, node: &str, at: i64) -> Self {
        let what = match kind {
            MarkerKind::NodeNotReady => "NotReady",
            MarkerKind::NodeReady => "Ready",
            MarkerKind::NodeReboot => "rebooted",
            MarkerKind::Deploy => "deployed",
        };
        Self {
            kind,
            at,
            namespace: None,
            node: Some(node.to_owned()),
            label: format!("{node} {what}"),
        }
    }
}

/// The markers since a time, and the parts that couldn't be read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Markers {
    pub markers: Vec<Marker>,
    /// "Can't list replicasets: forbidden" and the like.
    pub unavailable: Vec<String>,
}

/// Reads every Kubernetes marker since `since`: ReplicaSets, Nodes and the
/// Nodes' events, concurrently.
pub async fn read_markers(client: kube::Client, since: i64) -> Markers {
    let params = ListParams::default().match_any();
    let node_events = params.clone().fields("involvedObject.kind=Node");
    let (replicasets, nodes, events) = (
        Api::<ReplicaSet>::all(client.clone()),
        Api::<Node>::all(client.clone()),
        Api::<Event>::all(client),
    );
    let (replicasets, nodes, events) = tokio::join!(
        timed("replicasets", replicasets.list_metadata(&params)),
        timed("nodes", nodes.list(&params)),
        timed("events", events.list(&node_events)),
    );
    let mut unavailable = Vec::new();
    let replicasets = available(replicasets, &mut unavailable);
    let nodes = available(nodes, &mut unavailable);
    let events = available(events, &mut unavailable);
    let mut markers = deploys(&replicasets, since);
    markers.extend(node_markers(&nodes, &events, since));
    Markers {
        markers: merge(markers),
        unavailable,
    }
}

fn available<K>(part: Result<Vec<K>, String>, unavailable: &mut Vec<String>) -> Vec<K> {
    part.unwrap_or_else(|error| {
        unavailable.push(error);
        Vec::new()
    })
}

async fn timed<K>(
    name: &str,
    list: impl Future<Output = kube::Result<ObjectList<K>>>,
) -> Result<Vec<K>, String>
where
    K: Clone,
{
    match tokio::time::timeout(DEADLINE, list).await {
        Ok(Ok(list)) => Ok(list.items),
        Ok(Err(kube::Error::Api(error))) if error.code == 403 => {
            Err(format!("Can't list {name}: forbidden"))
        }
        Ok(Err(_)) => Err(format!("Can't list {name}: request failed")),
        Err(_) => Err(format!("Can't list {name}: timed out")),
    }
}

/// Reboots since `since`, from each Talos node's boot time. `nodes` pairs
/// each node's name with its address. A node that doesn't answer is
/// skipped; the result says how many didn't.
pub async fn boot_markers(
    client: TalosClient,
    nodes: Vec<(String, String)>,
    since: i64,
) -> (Vec<Marker>, usize) {
    let reads = nodes.into_iter().map(|(name, address)| {
        let client = client.with_node(&address);
        async move {
            let stat = tokio::time::timeout(DEADLINE, client.system_stat()).await;
            let boot = match stat {
                Ok(Ok(stats)) => stats.first().map(|stat| stat.boot_time),
                _ => None,
            };
            (name, boot)
        }
    });
    let answers = futures::future::join_all(reads).await;
    let missing = answers.iter().filter(|(_, boot)| boot.is_none()).count();
    let markers = answers
        .into_iter()
        .filter_map(|(name, boot)| {
            let at = i64::try_from(boot?)
                .ok()
                .filter(|at| *at > 0 && *at >= since)?;
            Some(Marker::node(MarkerKind::NodeReboot, &name, at))
        })
        .collect();
    (markers, missing)
}

/// Each Deployment-owned ReplicaSet created since `since`: the rollout of
/// its revision. The version label names the release when there is one.
pub fn deploys(replicasets: &[PartialObjectMeta<ReplicaSet>], since: i64) -> Vec<Marker> {
    replicasets
        .iter()
        .filter_map(|replicaset| {
            let meta = &replicaset.metadata;
            let at = meta.creation_timestamp.as_ref()?.0.timestamp();
            if at < since {
                return None;
            }
            let owner = meta
                .owner_references
                .as_ref()?
                .iter()
                .find(|owner| owner.kind == "Deployment" && owner.controller == Some(true))?;
            let revision = meta.annotations.as_ref().and_then(|a| a.get(REVISION));
            let version = meta.labels.as_ref().and_then(|l| l.get(VERSION));
            let to = match (version, revision) {
                (Some(version), _) => version.clone(),
                (None, Some(revision)) => format!("revision {revision}"),
                (None, None) => meta.name.clone().unwrap_or_default(),
            };
            Some(Marker {
                kind: MarkerKind::Deploy,
                at,
                namespace: meta.namespace.clone(),
                node: None,
                label: format!("deploy/{} → {to}", owner.name),
            })
        })
        .collect()
}

/// Ready and NotReady transitions since `since`: the last one from each
/// node's Ready condition, older ones from the node controller's and the
/// kubelet's events, which are kept for about an hour.
pub fn node_markers(nodes: &[Node], events: &[Event], since: i64) -> Vec<Marker> {
    let mut markers = Vec::new();
    for node in nodes {
        let Some(name) = node.metadata.name.as_deref() else {
            continue;
        };
        let ready = node
            .status
            .as_ref()
            .and_then(|status| status.conditions.as_ref())
            .and_then(|conditions| conditions.iter().find(|c| c.type_ == "Ready"));
        let Some(ready) = ready else { continue };
        let Some(at) = ready.last_transition_time.as_ref().map(|t| t.0.timestamp()) else {
            continue;
        };
        if at >= since {
            let kind = if ready.status == "True" {
                MarkerKind::NodeReady
            } else {
                MarkerKind::NodeNotReady
            };
            markers.push(Marker::node(kind, name, at));
        }
    }
    for event in events {
        let kind = match event.reason.as_deref() {
            Some("NodeNotReady") => MarkerKind::NodeNotReady,
            Some("NodeReady") => MarkerKind::NodeReady,
            Some("Rebooted") => MarkerKind::NodeReboot,
            _ => continue,
        };
        let Some(name) = event.involved_object.name.as_deref() else {
            continue;
        };
        for at in event_times(event) {
            if at >= since {
                markers.push(Marker::node(kind, name, at));
            }
        }
    }
    markers
}

/// When an event happened: its first and last time when it repeated.
fn event_times(event: &Event) -> Vec<i64> {
    let stamp = |time: Option<&DateTime<Utc>>| time.map(DateTime::timestamp);
    let last = stamp(event.last_timestamp.as_ref().map(|t| &t.0))
        .or_else(|| stamp(event.event_time.as_ref().map(|t| &t.0)));
    let first = stamp(event.first_timestamp.as_ref().map(|t| &t.0));
    match (first, last) {
        (Some(first), Some(last)) if first != last && event.count.unwrap_or(1) > 1 => {
            vec![first, last]
        }
        (_, Some(at)) | (Some(at), None) => vec![at],
        (None, None) => Vec::new(),
    }
}

/// Oldest first, one marker per node event however many sources saw it
/// (a reboot's boot time before the kubelet's event, a condition before
/// its event), at most [`MAX_MARKERS`] of the newest.
pub fn merge(mut markers: Vec<Marker>) -> Vec<Marker> {
    markers.sort_by(|a, b| (a.at, a.kind, &a.label).cmp(&(b.at, b.kind, &b.label)));
    let mut kept: Vec<Marker> = Vec::with_capacity(markers.len());
    for marker in markers {
        let close = match marker.kind {
            MarkerKind::Deploy => 0,
            MarkerKind::NodeReboot => SAME_REBOOT,
            _ => SAME_NODE_EVENT,
        };
        let duplicate = kept
            .iter()
            .rev()
            .take_while(|k| marker.at - k.at <= close)
            .any(|k| k.kind == marker.kind && k.node == marker.node && k.label == marker.label);
        if !duplicate {
            kept.push(marker);
        }
    }
    let excess = kept.len().saturating_sub(MAX_MARKERS);
    kept.drain(..excess);
    kept
}

/// Example markers for `--fixture`, relative to `now`, on the example
/// data's namespaces and nodes.
pub fn example_markers(now: i64) -> Vec<Marker> {
    let deploy = |minutes: i64, namespace: &str, name: &str, to: &str| Marker {
        kind: MarkerKind::Deploy,
        at: now - minutes * 60,
        namespace: Some(namespace.into()),
        node: None,
        label: format!("deploy/{name} → {to}"),
    };
    let node = |minutes: i64, kind, name: &str| Marker::node(kind, name, now - minutes * 60);
    merge(vec![
        deploy(250, "kube-system", "coredns", "1.11.3"),
        deploy(170, "payments", "checkout", "revision 14"),
        deploy(95, "monitoring", "kube-state-metrics", "2.13.0"),
        deploy(38, "payments", "api", "1.8.2"),
        node(320, MarkerKind::NodeReboot, "cp-2"),
        node(316, MarkerKind::NodeReady, "cp-2"),
        node(9, MarkerKind::NodeNotReady, "worker-2"),
    ])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::api::core::v1::{NodeCondition, NodeStatus, ObjectReference};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference, Time};
    use kube::core::TypeMeta;

    use super::*;

    fn time(at: i64) -> Time {
        Time(DateTime::from_timestamp(at, 0).unwrap())
    }

    fn replicaset(
        name: &str,
        owner: Option<&str>,
        at: i64,
        version: Option<&str>,
    ) -> PartialObjectMeta<ReplicaSet> {
        PartialObjectMeta {
            types: Some(TypeMeta::resource::<ReplicaSet>()),
            metadata: ObjectMeta {
                name: Some(name.into()),
                namespace: Some("payments".into()),
                creation_timestamp: Some(time(at)),
                owner_references: owner.map(|owner| {
                    vec![OwnerReference {
                        kind: "Deployment".into(),
                        name: owner.into(),
                        controller: Some(true),
                        ..Default::default()
                    }]
                }),
                annotations: Some(BTreeMap::from([(REVISION.into(), "7".into())])),
                labels: version.map(|version| BTreeMap::from([(VERSION.into(), version.into())])),
                ..Default::default()
            },
            _phantom: Default::default(),
        }
    }

    fn node(name: &str, ready: &str, at: i64) -> Node {
        Node {
            metadata: ObjectMeta {
                name: Some(name.into()),
                ..Default::default()
            },
            status: Some(NodeStatus {
                conditions: Some(vec![NodeCondition {
                    type_: "Ready".into(),
                    status: ready.into(),
                    last_transition_time: Some(time(at)),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn event(node: &str, reason: &str, first: i64, last: i64, count: i32) -> Event {
        Event {
            involved_object: ObjectReference {
                kind: Some("Node".into()),
                name: Some(node.into()),
                ..Default::default()
            },
            reason: Some(reason.into()),
            first_timestamp: Some(time(first)),
            last_timestamp: Some(time(last)),
            count: Some(count),
            ..Default::default()
        }
    }

    #[test]
    fn a_deployments_new_replicaset_is_a_deploy_named_by_its_version_or_revision() {
        let markers = deploys(
            &[
                replicaset("api-7f9c6d", Some("api"), 1_000, Some("1.8.2")),
                replicaset("web-5d8b7", Some("web"), 1_100, None),
                replicaset("old-1", Some("old"), 10, None),
                replicaset("bare-1", None, 1_200, None),
            ],
            500,
        );
        let labels: Vec<_> = markers.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, ["deploy/api → 1.8.2", "deploy/web → revision 7"]);
        assert_eq!(markers[0].namespace.as_deref(), Some("payments"));
        assert_eq!(markers[0].at, 1_000);
    }

    #[test]
    fn a_nodes_condition_and_its_events_make_one_marker_each() {
        let markers = merge(node_markers(
            &[node("worker-2", "Unknown", 2_000), node("cp-1", "True", 10)],
            &[
                // The node controller's event for the same transition.
                event("worker-2", "NodeNotReady", 2_005, 2_005, 1),
                // Repeated: its first and last times are both drawn.
                event("worker-3", "NodeNotReady", 1_000, 1_500, 2),
                event("cp-1", "Starting", 1_000, 1_000, 1),
            ],
            500,
        ));
        let seen: Vec<_> = markers.iter().map(|m| (m.at, m.label.as_str())).collect();
        assert_eq!(
            seen,
            [
                (1_000, "worker-3 NotReady"),
                (1_500, "worker-3 NotReady"),
                (2_000, "worker-2 NotReady"),
            ]
        );
    }

    #[test]
    fn a_reboot_seen_by_talos_and_the_kubelet_is_one_marker() {
        let mut markers = vec![Marker::node(MarkerKind::NodeReboot, "cp-2", 1_000)];
        markers.extend(node_markers(
            &[],
            &[event("cp-2", "Rebooted", 1_090, 1_090, 1)],
            0,
        ));
        let merged = merge(markers);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].at, 1_000);
    }

    #[test]
    fn only_the_newest_markers_are_kept() {
        let markers: Vec<Marker> = (0..MAX_MARKERS as i64 + 20)
            .map(|at| Marker::node(MarkerKind::NodeReady, &format!("n{at}"), at))
            .collect();
        let merged = merge(markers);
        assert_eq!(merged.len(), MAX_MARKERS);
        assert_eq!(merged[0].at, 20);
    }

    #[test]
    fn example_markers_fall_within_six_hours_and_some_within_the_hour() {
        let markers = example_markers(100_000);
        assert!(markers.iter().all(|m| m.at >= 100_000 - 6 * 3600));
        assert!(markers.iter().filter(|m| m.at >= 100_000 - 3600).count() >= 2);
        assert!(markers.windows(2).all(|pair| pair[0].at <= pair[1].at));
    }
}
