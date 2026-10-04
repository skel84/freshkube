//! Pure projections of Kubernetes evidence.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use k8s_openapi::api::{
    apps::v1::{DaemonSet, Deployment, StatefulSet},
    core::v1::{Event, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Pod},
};
use kube::Resource;

use super::{
    ClaimSummary, EventSummary, ISSUE_LIMIT, KubernetesSummary, NodeCondition, NodeSummary, Part,
    PendingClaim, PodSummary, Warning,
};
use crate::workloads::{
    self, PodInfo, WorkloadCollectionOutcome, WorkloadSource, WorkloadSourceError,
};

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
    let mut references = BTreeMap::new();
    fn remember<T: Resource>(
        kind: &str,
        part: &Part<Vec<T>>,
        out: &mut BTreeMap<(String, String, String), String>,
    ) {
        for object in part.loaded().into_iter().flatten() {
            let meta = object.meta();
            if let (Some(name), Some(uid)) = (&meta.name, &meta.uid) {
                out.insert(
                    (
                        kind.into(),
                        meta.namespace.clone().unwrap_or_default(),
                        name.clone(),
                    ),
                    uid.clone(),
                );
            }
        }
    }
    remember("pods", &pods, &mut references);
    remember("deployments.apps", &deployments, &mut references);
    remember("statefulsets.apps", &statefulsets, &mut references);
    remember("daemonsets.apps", &daemonsets, &mut references);
    remember("persistentvolumeclaims", &claims, &mut references);
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
    let nodes = nodes.map(|nodes| {
        let mut nodes =
            summarize_nodes(nodes, pods.loaded().map(Vec::as_slice).unwrap_or_default());
        for node in &mut nodes {
            node.pods_current = pods.is_current();
            node.pods_observed = pods.loaded().is_some();
        }
        nodes
    });
    let pod_summary = pods
        .clone()
        .map(|pods| summarize_pods(&pods, nodes.loaded().map(Vec::as_slice).unwrap_or_default()));
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
    let mut kept = std::collections::BTreeSet::new();
    if let Some(pods) = pod_summary.loaded() {
        kept.extend(
            pods.issues
                .iter()
                .map(|pod| ("pods".to_owned(), pod.namespace.clone(), pod.name.clone())),
        );
    }
    if let Some(snapshot) = workloads.snapshot() {
        for namespace in &snapshot.namespaces {
            for workload in &namespace.workloads {
                if workload.health != workloads::HealthState::Healthy {
                    let kind = match workload.kind {
                        workloads::WorkloadKind::Deployment => "deployments.apps",
                        workloads::WorkloadKind::StatefulSet => "statefulsets.apps",
                        workloads::WorkloadKind::DaemonSet => "daemonsets.apps",
                    };
                    kept.insert((
                        kind.into(),
                        workload.namespace.clone(),
                        workload.name.clone(),
                    ));
                }
            }
        }
    }
    if let Some(claims) = claims.loaded() {
        kept.extend(claims.pending.iter().map(|claim| {
            (
                "persistentvolumeclaims".to_owned(),
                claim.namespace.clone(),
                claim.name.clone(),
            )
        }));
    }
    references.retain(|key, _| kept.contains(key));
    KubernetesSummary {
        observations: Default::default(),
        references,
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

pub(super) fn summarize_nodes(nodes: Vec<Node>, pods: &[Pod]) -> Vec<NodeSummary> {
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
                pods_current: true,
                pods_observed: true,
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
        if let Some(namespace) = &pod.metadata.namespace {
            *summary.by_namespace.entry(namespace.clone()).or_default() += 1;
        }
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
            *summary
                .issues_by_status
                .entry(issue.label().into())
                .or_default() += 1;
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

pub(super) fn summarize_events(events: Vec<Event>, now: DateTime<Utc>) -> EventSummary {
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
