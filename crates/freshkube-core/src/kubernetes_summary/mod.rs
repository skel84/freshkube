//! Cluster facts derived from cache-backed Kubernetes lists. Objects are
//! dropped after collection; a refused list does not hide other parts.
use std::{collections::BTreeMap, time::Duration};

use chrono::{DateTime, Utc};
use k8s_openapi::{
    api::{
        apps::v1::{DaemonSet, Deployment, StatefulSet},
        core::v1::{Event, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Pod},
    },
    apimachinery::pkg::api::resource::Quantity,
};
use kube::{
    Api, Client, Resource,
    api::{ListParams, ObjectList},
};
use serde::de::DeserializeOwned;

use crate::workloads::{
    self, PodInfo, WorkloadCollectionOutcome, WorkloadSource, WorkloadSourceError,
};

const DEADLINE: Duration = Duration::from_secs(15);
pub const ISSUE_LIMIT: usize = 200;

/// A successful part, or the reason that this list cannot be shown.
#[derive(Clone, Debug, PartialEq)]
pub enum Part<T> {
    Loaded(T),
    Refused(String),
    Failed(String),
}

impl<T> Part<T> {
    pub fn loaded(&self) -> Option<&T> {
        match self {
            Self::Loaded(value) => Some(value),
            _ => None,
        }
    }
    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Refused(error) | Self::Failed(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NodeCondition {
    pub kind: String,
    pub status: String,
    pub reason: String,
    pub message: String,
    pub since: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NodeSummary {
    pub uid: String,
    pub name: String,
    pub conditions: Vec<NodeCondition>,
    pub unschedulable: bool,
    pub roles: Vec<String>,
    pub addresses: Vec<(String, String)>,
    pub kubelet_version: String,
    pub capacity: BTreeMap<String, Quantity>,
    pub taints: Vec<String>,
    pub pods: usize,
}

impl NodeSummary {
    pub fn ready(&self) -> Option<&NodeCondition> {
        self.conditions
            .iter()
            .find(|condition| condition.kind == "Ready")
    }
    pub fn is_ready(&self) -> bool {
        self.ready()
            .is_some_and(|condition| condition.status == "True")
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PodSummary {
    pub total: usize,
    pub phases: BTreeMap<String, usize>,
    pub on_not_ready: usize,
    pub issues: Vec<PodInfo>,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Warning {
    pub namespace: String,
    pub kind: String,
    pub name: String,
    pub reason: String,
    pub message: String,
    pub at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct EventSummary {
    pub total: usize,
    pub reasons: BTreeMap<String, usize>,
    pub newest: Vec<Warning>,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PendingClaim {
    pub namespace: String,
    pub name: String,
    pub since: Option<DateTime<Utc>>,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ClaimSummary {
    pub pending_count: usize,
    pub pending: Vec<PendingClaim>,
    pub bound: usize,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct KubernetesSummary {
    pub version: Part<String>,
    pub nodes: Part<Vec<NodeSummary>>,
    pub pods: Part<PodSummary>,
    pub workloads: WorkloadCollectionOutcome,
    pub events: Part<EventSummary>,
    pub claims: Part<ClaimSummary>,
    pub available_volumes: Part<usize>,
    pub namespaces: Part<usize>,
}

impl KubernetesSummary {
    pub fn refresh_failure(&self) -> Option<&str> {
        fn failed<T>(part: &Part<T>) -> Option<&str> {
            match part {
                Part::Failed(message) => Some(message),
                _ => None,
            }
        }
        failed(&self.version)
            .or_else(|| failed(&self.nodes))
            .or_else(|| failed(&self.pods))
            .or_else(|| failed(&self.events))
            .or_else(|| failed(&self.claims))
            .or_else(|| failed(&self.available_volumes))
            .or_else(|| failed(&self.namespaces))
            .or_else(|| {
                self.workloads
                    .unavailable()
                    .iter()
                    .find(|error| !error.message.contains("forbidden"))
                    .map(|error| error.message.as_str())
            })
    }
}

async fn list<K>(client: Client, name: &str, params: &ListParams) -> Part<Vec<K>>
where
    K: Clone + std::fmt::Debug + DeserializeOwned + Resource<DynamicType = ()>,
{
    match tokio::time::timeout(DEADLINE, Api::<K>::all(client).list(params)).await {
        Ok(Ok(ObjectList { items, .. })) => Part::Loaded(items),
        Ok(Err(kube::Error::Api(error))) if error.code == 403 => {
            Part::Refused(format!("Can't list {name}: forbidden"))
        }
        Ok(Err(_)) => Part::Failed(format!("Can't list {name}: request failed")),
        Err(_) => Part::Failed(format!("Can't list {name}: timed out")),
    }
}

impl<T> Part<T> {
    fn map<U>(self, map: impl FnOnce(T) -> U) -> Part<U> {
        match self {
            Self::Loaded(value) => Part::Loaded(map(value)),
            Self::Refused(error) => Part::Refused(error),
            Self::Failed(error) => Part::Failed(error),
        }
    }
}

/// Lists each part concurrently, using the API server cache and an independent
/// deadline. This is the shell's one cluster-wide background read.
pub async fn collect_kubernetes_summary(client: Client) -> KubernetesSummary {
    let params = ListParams::default().match_any();
    let warnings = params.clone().fields("type=Warning");
    let version_client = client.clone();
    let (
        version,
        nodes,
        pods,
        deployments,
        statefulsets,
        daemonsets,
        namespaces,
        claims,
        volumes,
        events,
    ) = tokio::join!(
        tokio::time::timeout(DEADLINE, version_client.apiserver_version()),
        list::<Node>(client.clone(), "nodes", &params),
        list::<Pod>(client.clone(), "pods", &params),
        list::<Deployment>(client.clone(), "deployments", &params),
        list::<StatefulSet>(client.clone(), "statefulsets", &params),
        list::<DaemonSet>(client.clone(), "daemonsets", &params),
        list::<Namespace>(client.clone(), "namespaces", &params),
        list::<PersistentVolumeClaim>(client.clone(), "persistentvolumeclaims", &params),
        list::<PersistentVolume>(client.clone(), "persistentvolumes", &params),
        list::<Event>(client.clone(), "events", &warnings),
    );
    let version = match version {
        Ok(Ok(version)) => Part::Loaded(version.git_version),
        Ok(Err(kube::Error::Api(error))) if error.code == 403 => {
            Part::Refused("Can’t read Kubernetes version: forbidden".into())
        }
        _ => Part::Failed("Kubernetes API not answering".into()),
    };
    derive(
        version,
        nodes,
        pods,
        deployments,
        statefulsets,
        daemonsets,
        namespaces,
        claims,
        volumes,
        events,
        Utc::now(),
    )
}

/// Builds the same summary from an example store or typed API answers.
#[allow(clippy::too_many_arguments)]
pub fn derive(
    version: Part<String>,
    nodes: Part<Vec<Node>>,
    pods: Part<Vec<Pod>>,
    deployments: Part<Vec<Deployment>>,
    statefulsets: Part<Vec<StatefulSet>>,
    daemonsets: Part<Vec<DaemonSet>>,
    namespaces: Part<Vec<Namespace>>,
    claims: Part<Vec<PersistentVolumeClaim>>,
    volumes: Part<Vec<PersistentVolume>>,
    events: Part<Vec<Event>>,
    now: DateTime<Utc>,
) -> KubernetesSummary {
    let mut errors = Vec::new();
    for (source, error) in [
        (WorkloadSource::Pods, pods.error()),
        (WorkloadSource::Deployments, deployments.error()),
        (WorkloadSource::StatefulSets, statefulsets.error()),
        (WorkloadSource::DaemonSets, daemonsets.error()),
    ] {
        if let Some(message) = error {
            errors.push(WorkloadSourceError {
                source,
                message: message.into(),
            });
        }
    }
    let mut events = events.map(|events| summarize_events(events, now));
    let claims = claims.map(|claims| summarize_claims(claims, events.loaded()));
    if let Part::Loaded(events) = &mut events {
        events.newest.truncate(ISSUE_LIMIT);
    }
    let nodes = nodes
        .map(|nodes| summarize_nodes(nodes, pods.loaded().map(Vec::as_slice).unwrap_or_default()));
    let pod_summary = match &pods {
        Part::Loaded(pods) => Part::Loaded(summarize_pods(
            pods,
            nodes.loaded().map(Vec::as_slice).unwrap_or_default(),
        )),
        Part::Refused(error) => Part::Refused(error.clone()),
        Part::Failed(error) => Part::Failed(error.clone()),
    };
    fn objects<T>(part: Part<Vec<T>>) -> Vec<T> {
        match part {
            Part::Loaded(objects) => objects,
            _ => Vec::new(),
        }
    }
    let mut snapshot = workloads::build_snapshot(
        String::new(),
        objects(deployments),
        objects(statefulsets),
        objects(daemonsets),
        objects(pods),
    );
    if let Some(pods) = pod_summary.loaded() {
        let newest: std::collections::HashSet<_> = pods
            .issues
            .iter()
            .map(|pod| (&pod.namespace, &pod.name))
            .collect();
        for namespace in &mut snapshot.namespaces {
            namespace
                .problem_pods
                .retain(|pod| newest.contains(&(&pod.namespace, &pod.name)));
        }
    }
    let workloads = if errors.len() == 4 {
        WorkloadCollectionOutcome::Unavailable {
            target: String::new(),
            errors,
        }
    } else if errors.is_empty() {
        WorkloadCollectionOutcome::Complete(snapshot)
    } else {
        WorkloadCollectionOutcome::Partial {
            snapshot,
            unavailable: errors,
        }
    };
    KubernetesSummary {
        version,
        nodes,
        pods: pod_summary,
        workloads,
        events,
        claims,
        available_volumes: volumes.map(|volumes| {
            volumes
                .iter()
                .filter(|volume| {
                    volume
                        .status
                        .as_ref()
                        .and_then(|status| status.phase.as_deref())
                        == Some("Available")
                })
                .count()
        }),
        namespaces: namespaces.map(|namespaces| namespaces.len()),
    }
}

fn summarize_nodes(nodes: Vec<Node>, pods: &[Pod]) -> Vec<NodeSummary> {
    let mut counts = BTreeMap::new();
    for pod in pods {
        if let Some(name) = pod.spec.as_ref().and_then(|spec| spec.node_name.as_deref()) {
            *counts.entry(name).or_insert(0) += 1;
        }
    }
    nodes
        .into_iter()
        .map(|node| {
            let status = node.status.unwrap_or_default();
            let spec = node.spec.unwrap_or_default();
            let name = node.metadata.name.unwrap_or_default();
            NodeSummary {
                uid: node.metadata.uid.clone().unwrap_or_default(),
                pods: counts.get(name.as_str()).copied().unwrap_or_default(),
                name,
                conditions: status
                    .conditions
                    .unwrap_or_default()
                    .into_iter()
                    .map(|condition| NodeCondition {
                        kind: condition.type_,
                        status: condition.status,
                        reason: condition.reason.unwrap_or_default(),
                        message: condition.message.unwrap_or_default(),
                        since: condition.last_transition_time.map(|time| time.0),
                    })
                    .collect(),
                unschedulable: spec.unschedulable.unwrap_or(false),
                roles: node
                    .metadata
                    .labels
                    .unwrap_or_default()
                    .keys()
                    .filter_map(|key| {
                        key.strip_prefix("node-role.kubernetes.io/")
                            .map(str::to_owned)
                    })
                    .collect(),
                addresses: status
                    .addresses
                    .unwrap_or_default()
                    .into_iter()
                    .map(|address| (address.type_, address.address))
                    .collect(),
                kubelet_version: status
                    .node_info
                    .map(|info| info.kubelet_version)
                    .unwrap_or_default(),
                capacity: status.capacity.unwrap_or_default(),
                taints: spec
                    .taints
                    .unwrap_or_default()
                    .into_iter()
                    .map(|taint| format!("{}:{}", taint.key, taint.effect))
                    .collect(),
            }
        })
        .collect()
}

fn summarize_pods(pods: &[Pod], nodes: &[NodeSummary]) -> PodSummary {
    let mut summary = PodSummary {
        total: pods.len(),
        ..Default::default()
    };
    for pod in pods {
        let phase = pod
            .status
            .as_ref()
            .and_then(|status| status.phase.as_deref())
            .unwrap_or("Unknown");
        *summary.phases.entry(phase.into()).or_default() += 1;
        let node = pod.spec.as_ref().and_then(|spec| spec.node_name.clone());
        if nodes
            .iter()
            .any(|row| Some(&row.name) == node.as_ref() && !row.is_ready())
        {
            summary.on_not_ready += 1;
        }
        let (restarts, issue) = workloads::analyze_pod(pod);
        if let Some(issue) = issue {
            summary.issues.push(PodInfo {
                name: pod.metadata.name.clone().unwrap_or_default(),
                namespace: pod.metadata.namespace.clone().unwrap_or_default(),
                node,
                phase: phase.into(),
                restarts,
                issue,
                created_at: pod.metadata.creation_timestamp.as_ref().map(|time| time.0),
            });
        }
    }
    summary
        .issues
        .sort_by_key(|pod| std::cmp::Reverse(pod.created_at));
    summary.issues.truncate(ISSUE_LIMIT);
    summary
}

fn summarize_events(events: Vec<Event>, now: DateTime<Utc>) -> EventSummary {
    let mut summary = EventSummary::default();
    let mut newest = BTreeMap::new();
    for event in events {
        if event.type_.as_deref() != Some("Warning") {
            continue;
        }
        let at = event
            .series
            .as_ref()
            .and_then(|series| series.last_observed_time.as_ref().map(|time| time.0))
            .or_else(|| event.last_timestamp.as_ref().map(|time| time.0))
            .or_else(|| event.event_time.as_ref().map(|time| time.0))
            .or_else(|| {
                event
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .map(|time| time.0)
            });
        let Some(at) = at.filter(|at| *at >= now - chrono::Duration::hours(1) && *at <= now) else {
            continue;
        };
        summary.total += 1;
        let warning = Warning {
            namespace: event.involved_object.namespace.unwrap_or_default(),
            kind: event.involved_object.kind.unwrap_or_default(),
            name: event.involved_object.name.unwrap_or_default(),
            reason: event.reason.unwrap_or_default(),
            message: event.message.unwrap_or_default(),
            at,
        };
        *summary.reasons.entry(warning.reason.clone()).or_default() += 1;
        let key = (
            warning.namespace.clone(),
            warning.kind.clone(),
            warning.name.clone(),
        );
        if newest
            .get(&key)
            .is_none_or(|previous: &Warning| previous.at < at)
        {
            newest.insert(key, warning);
        }
    }
    summary.newest = newest.into_values().collect();
    summary
        .newest
        .sort_by_key(|warning| std::cmp::Reverse(warning.at));
    summary
}

fn summarize_claims(
    claims: Vec<PersistentVolumeClaim>,
    events: Option<&EventSummary>,
) -> ClaimSummary {
    let mut summary = ClaimSummary::default();
    for claim in claims {
        match claim
            .status
            .as_ref()
            .and_then(|status| status.phase.as_deref())
        {
            Some("Bound") => summary.bound += 1,
            Some("Pending") => {
                let namespace = claim.metadata.namespace.unwrap_or_default();
                let name = claim.metadata.name.unwrap_or_default();
                let reason = events
                    .and_then(|events| {
                        events.newest.iter().find(|warning| {
                            warning.kind == "PersistentVolumeClaim"
                                && warning.namespace == namespace
                                && warning.name == name
                        })
                    })
                    .map(|warning| warning.message.clone())
                    .unwrap_or_default();
                summary.pending.push(PendingClaim {
                    namespace,
                    name,
                    since: claim.metadata.creation_timestamp.map(|time| time.0),
                    reason,
                });
                summary.pending_count += 1;
            }
            _ => {}
        }
    }
    summary
        .pending
        .sort_by_key(|claim| std::cmp::Reverse(claim.since));
    summary.pending.truncate(ISSUE_LIMIT);
    summary
}

#[cfg(test)]
mod tests;
