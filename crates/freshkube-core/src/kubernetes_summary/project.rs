//! Derive bounded display evidence from committed compact reflector objects.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::Event;

use super::{
    ClaimSummary, EventSummary, ISSUE_LIMIT, KubernetesSummary, NodeSummary, Observations, Part,
    PendingClaim, PodSummary, RetainedObject, Session, Source, retained::Facts, session::Evidence,
};
use crate::resources::Amounts;
use crate::workloads::{
    self, HealthState, NamespaceSummary, PodInfo, WorkloadCollectionOutcome, WorkloadSnapshot,
    WorkloadSource, WorkloadSourceError,
};

pub(super) fn derive(evidence: Evidence, now: DateTime<Utc>) -> KubernetesSummary {
    let Evidence {
        objects,
        observations,
        version,
        ..
    } = evidence;
    let mut nodes = node_facts(&objects);
    let not_ready: BTreeSet<_> = nodes
        .iter()
        .filter(|n| !n.is_ready())
        .map(|n| n.name.clone())
        .collect();
    let PodDerivation {
        pods,
        mut snapshot,
        namespaces,
        pod_counts,
        requests,
    } = derive_pods(&objects, &not_ready);
    derive_nodes(&mut nodes, &pod_counts, &requests, &observations);
    derive_workloads(&objects, &mut snapshot, namespaces);
    let mut events = super::summarize::summarize_events(
        objects[&Source::Events]
            .iter()
            .filter_map(|o| {
                if let Facts::Event(event) = &o.facts {
                    Some((**event).clone())
                } else {
                    None
                }
            })
            .collect(),
        now,
    );
    let claims = derive_claims(&objects, &events, &observations);
    events.newest.truncate(ISSUE_LIMIT);
    let workloads = workload_outcome(snapshot, &observations);
    let references = references(&objects, &pods, &claims);
    let volumes = objects[&Source::Volumes]
        .iter()
        .filter(|o| matches!(&o.facts, Facts::Volume(true)))
        .count();
    KubernetesSummary {
        references,
        version,
        nodes: Part::observed(nodes, &observations[&Source::Nodes]),
        pods: Part::observed(pods, &observations[&Source::Pods]),
        events: Part::observed(events, &observations[&Source::Events]),
        claims: Part::observed(claims, &observations[&Source::Claims]),
        available_volumes: Part::observed(volumes, &observations[&Source::Volumes]),
        namespaces: Part::observed(
            objects[&Source::Namespaces].len(),
            &observations[&Source::Namespaces],
        ),
        workloads,
        observations,
    }
}

type Objects = BTreeMap<Source, Vec<Arc<RetainedObject>>>;

/// Retained nodes, by name.
fn node_facts(objects: &Objects) -> Vec<NodeSummary> {
    let mut nodes: Vec<_> = objects[&Source::Nodes]
        .iter()
        .filter_map(|object| {
            if let Facts::Node(node) = &object.facts {
                Some((**node).clone())
            } else {
                None
            }
        })
        .collect();
    nodes.sort_by(|left, right| left.name.cmp(&right.name));
    nodes
}

/// What the pods give the summary, the workload snapshot and the nodes.
struct PodDerivation {
    pods: PodSummary,
    snapshot: WorkloadSnapshot,
    namespaces: BTreeMap<String, NamespaceSummary>,
    pod_counts: BTreeMap<String, usize>,
    requests: BTreeMap<String, Amounts>,
}

/// Pod counts, phases and issues, with each node's pod count and requests.
fn derive_pods(objects: &Objects, not_ready: &BTreeSet<String>) -> PodDerivation {
    let mut pod_counts = BTreeMap::new();
    let mut requests = BTreeMap::new();
    let mut pods = PodSummary::default();
    let mut snapshot = WorkloadSnapshot::default();
    let mut namespaces: BTreeMap<String, NamespaceSummary> = BTreeMap::new();
    for object in &objects[&Source::Pods] {
        let Facts::Pod(pod) = &object.facts else {
            continue;
        };
        pods.total += 1;
        *pods
            .by_namespace
            .entry(object.namespace.clone())
            .or_default() += 1;
        *pods.phases.entry(pod.phase.clone()).or_default() += 1;
        if let Some(node) = &pod.node {
            *pod_counts.entry(node.clone()).or_insert(0) += 1;
            if !matches!(pod.phase.as_str(), "Succeeded" | "Failed") {
                super::requests::add(
                    requests
                        .entry(node.clone())
                        .or_insert(super::requests::ZERO),
                    pod.requests,
                );
            }
            if not_ready.contains(node) {
                pods.on_not_ready += 1;
            }
        }
        match pod.issue.as_ref().map(|issue| issue.severity()) {
            Some(HealthState::Failing) => snapshot.total_pods_failing += 1,
            Some(HealthState::Degraded | HealthState::Pending) => snapshot.total_pods_degraded += 1,
            Some(HealthState::Healthy) => snapshot.total_pods_healthy += 1,
            None if pod.phase == "Running" || pod.phase == "Succeeded" => {
                snapshot.total_pods_healthy += 1
            }
            None => {}
        }
        if let Some(issue) = &pod.issue {
            let namespace = namespaces
                .entry(object.namespace.clone())
                .or_insert_with(|| NamespaceSummary {
                    name: object.namespace.clone(),
                    ..Default::default()
                });
            namespace.health = namespace.health.min(issue.severity());
            *pods
                .issues_by_status
                .entry(issue.label().into())
                .or_default() += 1;
            pods.issues.push(PodInfo {
                name: object.name.clone(),
                namespace: object.namespace.clone(),
                node: pod.node.clone(),
                phase: pod.phase.clone(),
                restarts: pod.restarts,
                issue: issue.clone(),
                created_at: object.created,
            });
        }
    }
    pods.issues.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)))
    });
    pods.issues.truncate(ISSUE_LIMIT);
    for pod in &pods.issues {
        namespaces
            .entry(pod.namespace.clone())
            .or_insert_with(|| NamespaceSummary {
                name: pod.namespace.clone(),
                ..Default::default()
            })
            .problem_pods
            .push(pod.clone());
    }
    PodDerivation {
        pods,
        snapshot,
        namespaces,
        pod_counts,
        requests,
    }
}

/// Each node's pods, current only while the pods are, and its requests only
/// when the pods were ever observed.
fn derive_nodes(
    nodes: &mut [NodeSummary],
    pod_counts: &BTreeMap<String, usize>,
    requests: &BTreeMap<String, Amounts>,
    observations: &Observations,
) {
    for node in nodes {
        node.pods = pod_counts.get(&node.name).copied().unwrap_or_default();
        node.pods_current = observations[&Source::Pods].is_current();
        node.pods_observed = observations[&Source::Pods].has_data();
        node.requests = if node.pods_observed {
            requests
                .get(&node.name)
                .copied()
                .unwrap_or(super::requests::ZERO)
        } else {
            crate::resources::Amounts::default()
        };
    }
}

/// Workload totals, and every namespace folded into the snapshot.
fn derive_workloads(
    objects: &Objects,
    snapshot: &mut WorkloadSnapshot,
    mut namespaces: BTreeMap<String, NamespaceSummary>,
) {
    for source in [
        Source::Deployments,
        Source::StatefulSets,
        Source::DaemonSets,
    ] {
        let objects = &objects[&source];
        match source {
            Source::Deployments => snapshot.total_deployments = objects.len(),
            Source::StatefulSets => snapshot.total_statefulsets = objects.len(),
            Source::DaemonSets => snapshot.total_daemonsets = objects.len(),
            _ => unreachable!(),
        }
        for object in objects {
            if let Facts::Workload(info) = &object.facts {
                namespaces
                    .entry(info.namespace.clone())
                    .or_insert_with(|| NamespaceSummary {
                        name: info.namespace.clone(),
                        ..Default::default()
                    })
                    .workloads
                    .push((**info).clone());
            }
        }
    }
    snapshot.namespaces = namespaces
        .into_values()
        .map(|namespace| {
            let health = namespace.health;
            let mut namespace = workloads::finalize_namespace(namespace);
            namespace.health = namespace.health.min(health);
            namespace
        })
        .collect();
    for ns in &mut snapshot.namespaces {
        ns.workloads
            .sort_by(|a, b| a.health.cmp(&b.health).then_with(|| a.name.cmp(&b.name)));
    }
    snapshot
        .namespaces
        .sort_by(|a, b| a.health.cmp(&b.health).then_with(|| a.name.cmp(&b.name)));
}

/// Bound and pending claims, each pending one with its newest warning, marked
/// last known while the events are not current.
fn derive_claims(
    objects: &Objects,
    events: &EventSummary,
    observations: &Observations,
) -> ClaimSummary {
    let mut claims = ClaimSummary::default();
    for object in &objects[&Source::Claims] {
        match &object.facts {
            Facts::Claim(phase) if phase == "Bound" => claims.bound += 1,
            Facts::Claim(phase) if phase == "Pending" => {
                claims.pending_count += 1;
                let reason = events
                    .newest
                    .iter()
                    .find(|warning| {
                        warning.kind == "PersistentVolumeClaim"
                            && warning.name == object.name
                            && warning.namespace == object.namespace
                    })
                    .map(|warning| {
                        if observations[&Source::Events].is_current() {
                            warning.message.clone()
                        } else {
                            format!("Last known warning: {}", warning.message)
                        }
                    })
                    .unwrap_or_default();
                claims.pending.push(PendingClaim {
                    namespace: object.namespace.clone(),
                    name: object.name.clone(),
                    since: object.created,
                    reason,
                });
            }
            _ => {}
        }
    }
    claims
        .pending
        .sort_by_key(|claim| std::cmp::Reverse(claim.since));
    claims.pending.truncate(ISSUE_LIMIT);
    claims
}

/// Complete, Partial with what failed, or Unavailable when all four workload
/// sources failed and none has data to show.
fn workload_outcome(
    snapshot: WorkloadSnapshot,
    observations: &Observations,
) -> WorkloadCollectionOutcome {
    let errors: Vec<_> = [
        (Source::Pods, WorkloadSource::Pods),
        (Source::Deployments, WorkloadSource::Deployments),
        (Source::StatefulSets, WorkloadSource::StatefulSets),
        (Source::DaemonSets, WorkloadSource::DaemonSets),
    ]
    .into_iter()
    .filter_map(|(source, workload)| {
        observations[&source]
            .message()
            .map(|message| WorkloadSourceError {
                source: workload,
                message: if observations[&source].has_data() {
                    format!("{message} · showing last known data")
                } else {
                    message.into()
                },
            })
    })
    .collect();
    if errors.is_empty() {
        WorkloadCollectionOutcome::Complete(snapshot)
    } else if errors.len() == 4
        && [
            Source::Pods,
            Source::Deployments,
            Source::StatefulSets,
            Source::DaemonSets,
        ]
        .iter()
        .all(|s| !observations[s].has_data())
    {
        WorkloadCollectionOutcome::Unavailable {
            target: String::new(),
            errors,
        }
    } else {
        WorkloadCollectionOutcome::Partial {
            snapshot,
            unavailable: errors,
        }
    }
}

/// UIDs of the objects the summary shows as issues, for opening them.
fn references(
    objects: &Objects,
    pods: &PodSummary,
    claims: &ClaimSummary,
) -> BTreeMap<(String, String, String), String> {
    let mut wanted = BTreeSet::new();
    for pod in &pods.issues {
        wanted.insert((Source::Pods.key(), pod.namespace.clone(), pod.name.clone()));
    }
    for claim in &claims.pending {
        wanted.insert((
            Source::Claims.key(),
            claim.namespace.clone(),
            claim.name.clone(),
        ));
    }
    for source in [
        Source::Deployments,
        Source::StatefulSets,
        Source::DaemonSets,
    ] {
        for object in &objects[&source] {
            if matches!(&object.facts, Facts::Workload(info) if info.health != HealthState::Healthy)
            {
                wanted.insert((source.key(), object.namespace.clone(), object.name.clone()));
            }
        }
    }
    objects
        .iter()
        .flat_map(|(source, objects)| {
            objects.iter().filter_map(|object| {
                let key = (source.key(), object.namespace.clone(), object.name.clone());
                if wanted.contains(&key) {
                    Some((key, object.uid.clone()))
                } else {
                    None
                }
            })
        })
        .collect()
}

pub(super) fn event_time(event: &Event) -> Option<DateTime<Utc>> {
    event
        .series
        .as_ref()
        .and_then(|s| s.last_observed_time.as_ref().map(|t| t.0))
        .or_else(|| event.last_timestamp.as_ref().map(|t| t.0))
        .or_else(|| event.event_time.as_ref().map(|t| t.0))
        .or_else(|| event.metadata.creation_timestamp.as_ref().map(|t| t.0))
}
impl Session {
    /// Local wall-clock eligibility/expiry changes need no API read. Calculate
    /// once after a publication, not on every raw watch event.
    pub fn next_warning_change(&self, now: DateTime<Utc>) -> Option<Duration> {
        let events = self.warning_objects();
        events
            .iter()
            .filter_map(|object| {
                let Facts::Event(event) = &object.facts else {
                    return None;
                };
                if event.type_.as_deref() != Some("Warning") {
                    return None;
                }
                let at = event_time(event)?;
                let next = if at > now {
                    at
                } else {
                    at.checked_add_signed(
                        chrono::Duration::hours(1) + chrono::Duration::milliseconds(1),
                    )?
                };
                (next > now).then(|| (next - now).to_std().ok()).flatten()
            })
            .min()
    }
}
