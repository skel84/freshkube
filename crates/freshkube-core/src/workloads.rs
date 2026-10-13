//! Framework-neutral Kubernetes workload health collection.
//!
//! Workload state is taken directly from Kubernetes API resources. A failed
//! list request makes only that resource kind unavailable: successful lists
//! remain in the returned snapshot so a frontend can still show useful state.

use std::collections::HashMap;

use crate::{
    constants::HIGH_RESTART_THRESHOLD,
    indicators::{HasHealth, HealthIndicator},
};
use chrono::{DateTime, Utc};
use k8s_openapi::api::{
    apps::v1::{DaemonSet, Deployment, StatefulSet},
    core::v1::Pod,
};

/// Health state of a workload or pod.
///
/// Variant ordering intentionally puts the most urgent state first, allowing
/// namespace summaries to sort issues before healthy namespaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum HealthState {
    /// Critical failure requiring immediate attention.
    Failing,
    /// Partially working workload or a pod with excessive restarts.
    Degraded,
    /// Waiting on scheduling or another external condition.
    Pending,
    /// Fully available or intentionally scaled to zero.
    #[default]
    Healthy,
}

impl HasHealth for HealthState {
    fn health(&self) -> HealthIndicator {
        match self {
            Self::Failing => HealthIndicator::Error,
            Self::Degraded => HealthIndicator::Warning,
            Self::Pending => HealthIndicator::Pending,
            Self::Healthy => HealthIndicator::Healthy,
        }
    }
}

/// Kubernetes controller kind represented by a [`WorkloadInfo`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkloadKind {
    Deployment,
    StatefulSet,
    DaemonSet,
}

impl WorkloadKind {
    /// Concise, presentation-neutral kind label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Deployment => "Deploy",
            Self::StatefulSet => "StatefulSet",
            Self::DaemonSet => "DaemonSet",
        }
    }
}

/// A controller's observed replica state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadInfo {
    pub name: String,
    pub namespace: String,
    pub kind: WorkloadKind,
    pub ready: i32,
    pub desired: i32,
    pub health: HealthState,
    /// Authoritative replica-state discrepancies suitable for display.
    pub issues: Vec<String>,
}

impl WorkloadInfo {
    pub(crate) fn from_deployment(deployment: &Deployment) -> Self {
        let namespace = resource_namespace(&deployment.metadata.namespace);
        let status = deployment.status.as_ref();
        let desired = status.and_then(|status| status.replicas).unwrap_or(0);
        let ready = status.and_then(|status| status.ready_replicas).unwrap_or(0);
        let available = status
            .and_then(|status| status.available_replicas)
            .unwrap_or(0);
        let (health, issues) = classify_workload_health(ready, desired, available);
        Self {
            name: resource_name(&deployment.metadata.name),
            namespace,
            kind: WorkloadKind::Deployment,
            ready,
            desired,
            health,
            issues,
        }
    }

    pub(crate) fn from_statefulset(statefulset: &StatefulSet) -> Self {
        let namespace = resource_namespace(&statefulset.metadata.namespace);
        let status = statefulset.status.as_ref();
        let desired = status.map(|status| status.replicas).unwrap_or(0);
        let ready = status.and_then(|status| status.ready_replicas).unwrap_or(0);
        let (health, issues) = classify_workload_health(ready, desired, ready);
        Self {
            name: resource_name(&statefulset.metadata.name),
            namespace,
            kind: WorkloadKind::StatefulSet,
            ready,
            desired,
            health,
            issues,
        }
    }

    pub(crate) fn from_daemonset(daemonset: &DaemonSet) -> Self {
        let namespace = resource_namespace(&daemonset.metadata.namespace);
        let status = daemonset.status.as_ref();
        let desired = status
            .map(|status| status.desired_number_scheduled)
            .unwrap_or(0);
        let ready = status.map(|status| status.number_ready).unwrap_or(0);
        let (health, issues) = classify_workload_health(ready, desired, ready);
        Self {
            name: resource_name(&daemonset.metadata.name),
            namespace,
            kind: WorkloadKind::DaemonSet,
            ready,
            desired,
            health,
            issues,
        }
    }
}

impl HasHealth for WorkloadInfo {
    fn health(&self) -> HealthIndicator {
        self.health.health()
    }
}

/// Problem state identified from a pod's current Kubernetes status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PodIssue {
    CrashLoopBackOff,
    ImagePullBackOff,
    ErrImagePull,
    Pending,
    OOMKilled,
    Error,
    HighRestarts(i32),
    NotReady,
    /// A failure reason diagnosed from current pod or container evidence.
    FailingReason(String),
    /// A scheduling or startup reason diagnosed from current evidence.
    WaitingReason(String),
    /// A container state that Kubernetes reports but this collector does not
    /// classify as one of the known failure reasons.
    Unknown(String),
}

impl PodIssue {
    /// Stable short label for frontend presentation.
    pub fn label(&self) -> &str {
        match self {
            Self::CrashLoopBackOff => "CrashLoopBackOff",
            Self::ImagePullBackOff => "ImagePullBackOff",
            Self::ErrImagePull => "ErrImagePull",
            Self::Pending => "Pending",
            Self::OOMKilled => "OOMKilled",
            Self::Error => "Error",
            Self::HighRestarts(_) => "High Restarts",
            Self::NotReady => "Not ready",
            Self::FailingReason(reason) | Self::WaitingReason(reason) => reason,
            Self::Unknown(_) => "Unknown",
        }
    }

    /// Whether a container ran and stopped: `CrashLoopBackOff`, `OOMKilled`
    /// or `Error`. A pod that couldn't start, such as `ImagePullBackOff` or a
    /// pending one, has nothing that died, though it can still be failing.
    pub const fn died(&self) -> bool {
        matches!(self, Self::CrashLoopBackOff | Self::OOMKilled | Self::Error)
    }

    /// Severity used when aggregating a pod into its namespace.
    pub const fn severity(&self) -> HealthState {
        match self {
            Self::CrashLoopBackOff
            | Self::ImagePullBackOff
            | Self::ErrImagePull
            | Self::OOMKilled
            | Self::Error
            | Self::FailingReason(_) => HealthState::Failing,
            Self::Pending | Self::WaitingReason(_) => HealthState::Pending,
            Self::HighRestarts(_) | Self::Unknown(_) | Self::NotReady => HealthState::Degraded,
        }
    }
}

impl HasHealth for PodIssue {
    fn health(&self) -> HealthIndicator {
        self.severity().health()
    }
}

/// Information about a pod that needs attention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodInfo {
    pub name: String,
    pub namespace: String,
    pub node: Option<String>,
    pub phase: String,
    pub restarts: i32,
    pub issue: PodIssue,
    /// Kubernetes creation time, retained rather than preformatted for one UI.
    pub created_at: Option<DateTime<Utc>>,
}

impl HasHealth for PodInfo {
    fn health(&self) -> HealthIndicator {
        self.issue.health()
    }
}

/// Aggregate workload state for a Kubernetes namespace.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NamespaceSummary {
    pub name: String,
    pub health: HealthState,
    pub workloads: Vec<WorkloadInfo>,
    pub problem_pods: Vec<PodInfo>,
    pub total_workloads: usize,
    pub healthy_workloads: usize,
}

impl NamespaceSummary {
    fn new(name: String) -> Self {
        Self {
            name,
            health: HealthState::Healthy,
            ..Self::default()
        }
    }
}

impl HasHealth for NamespaceSummary {
    fn health(&self) -> HealthIndicator {
        self.health.health()
    }
}

/// A snapshot assembled from the successfully listed Kubernetes resources.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkloadSnapshot {
    /// Explicit cluster name or address supplied with the Talos-pinned client.
    pub target: String,
    /// Namespace summaries, sorted from greatest need to least, then by name.
    pub namespaces: Vec<NamespaceSummary>,
    pub total_deployments: usize,
    pub total_statefulsets: usize,
    pub total_daemonsets: usize,
    pub total_pods_healthy: usize,
    pub total_pods_degraded: usize,
    pub total_pods_failing: usize,
}

/// Kubernetes resource list used by workload collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkloadSource {
    Deployments,
    StatefulSets,
    DaemonSets,
    Pods,
}

impl WorkloadSource {
    /// Lowercase resource name used in collection error messages.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Deployments => "deployments",
            Self::StatefulSets => "statefulsets",
            Self::DaemonSets => "daemonsets",
            Self::Pods => "pods",
        }
    }
}

/// One unavailable Kubernetes resource list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadSourceError {
    pub source: WorkloadSource,
    pub message: String,
}

impl std::fmt::Display for WorkloadSourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Failed to fetch {}: {}",
            self.source.label(),
            self.message
        )
    }
}

/// Result of collecting workload health from Kubernetes.
///
/// An unavailable list is not treated as a failure of the other lists. The
/// target is retained in the fully unavailable case because no resource data
/// remains from which a frontend can infer cluster identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkloadCollectionOutcome {
    Complete(WorkloadSnapshot),
    Partial {
        snapshot: WorkloadSnapshot,
        unavailable: Vec<WorkloadSourceError>,
    },
    Unavailable {
        target: String,
        errors: Vec<WorkloadSourceError>,
    },
}

impl WorkloadCollectionOutcome {
    /// Returns collected data when at least one Kubernetes list succeeded.
    pub fn snapshot(&self) -> Option<&WorkloadSnapshot> {
        match self {
            Self::Complete(snapshot) | Self::Partial { snapshot, .. } => Some(snapshot),
            Self::Unavailable { .. } => None,
        }
    }

    /// Returns resource-list failures while preserving any available data.
    pub fn unavailable(&self) -> &[WorkloadSourceError] {
        match self {
            Self::Complete(_) => &[],
            Self::Partial { unavailable, .. } => unavailable,
            Self::Unavailable { errors, .. } => errors,
        }
    }
}

pub(crate) fn build_snapshot(
    target: String,
    deployments: Vec<Deployment>,
    statefulsets: Vec<StatefulSet>,
    daemonsets: Vec<DaemonSet>,
    pods: Vec<Pod>,
) -> WorkloadSnapshot {
    let total_deployments = deployments.len();
    let total_statefulsets = statefulsets.len();
    let total_daemonsets = daemonsets.len();
    let mut namespaces = HashMap::new();

    for deployment in deployments {
        let namespace = resource_namespace(&deployment.metadata.namespace);
        let status = deployment.status.as_ref();
        let desired = status.and_then(|status| status.replicas).unwrap_or(0);
        let ready = status.and_then(|status| status.ready_replicas).unwrap_or(0);
        let available = status
            .and_then(|status| status.available_replicas)
            .unwrap_or(0);
        let (health, issues) = classify_workload_health(ready, desired, available);
        add_workload(
            &mut namespaces,
            namespace.clone(),
            WorkloadInfo {
                name: resource_name(&deployment.metadata.name),
                namespace,
                kind: WorkloadKind::Deployment,
                ready,
                desired,
                health,
                issues,
            },
        );
    }

    for statefulset in statefulsets {
        let namespace = resource_namespace(&statefulset.metadata.namespace);
        let status = statefulset.status.as_ref();
        let desired = status.map(|status| status.replicas).unwrap_or(0);
        let ready = status.and_then(|status| status.ready_replicas).unwrap_or(0);
        let (health, issues) = classify_workload_health(ready, desired, ready);
        add_workload(
            &mut namespaces,
            namespace.clone(),
            WorkloadInfo {
                name: resource_name(&statefulset.metadata.name),
                namespace,
                kind: WorkloadKind::StatefulSet,
                ready,
                desired,
                health,
                issues,
            },
        );
    }

    for daemonset in daemonsets {
        let namespace = resource_namespace(&daemonset.metadata.namespace);
        let status = daemonset.status.as_ref();
        let desired = status
            .map(|status| status.desired_number_scheduled)
            .unwrap_or(0);
        let ready = status.map(|status| status.number_ready).unwrap_or(0);
        let (health, issues) = classify_workload_health(ready, desired, ready);
        add_workload(
            &mut namespaces,
            namespace.clone(),
            WorkloadInfo {
                name: resource_name(&daemonset.metadata.name),
                namespace,
                kind: WorkloadKind::DaemonSet,
                ready,
                desired,
                health,
                issues,
            },
        );
    }

    let mut total_pods_healthy = 0;
    let mut total_pods_degraded = 0;
    let mut total_pods_failing = 0;
    for pod in pods {
        let namespace = resource_namespace(&pod.metadata.namespace);
        let phase = pod
            .status
            .as_ref()
            .and_then(|status| status.phase.clone())
            .unwrap_or_else(|| "Unknown".to_string());
        let (restarts, issue) = analyze_pod(&pod);

        match issue.as_ref().map(PodIssue::severity) {
            Some(HealthState::Failing) => total_pods_failing += 1,
            Some(HealthState::Degraded | HealthState::Pending) => total_pods_degraded += 1,
            Some(HealthState::Healthy) => total_pods_healthy += 1,
            None if phase == "Running" || phase == "Succeeded" => total_pods_healthy += 1,
            None => {}
        }

        if let Some(issue) = issue {
            let pod_info = PodInfo {
                name: resource_name(&pod.metadata.name),
                namespace: namespace.clone(),
                node: pod.spec.as_ref().and_then(|spec| spec.node_name.clone()),
                phase,
                restarts,
                issue,
                created_at: pod.metadata.creation_timestamp.map(|timestamp| timestamp.0),
            };
            namespaces
                .entry(namespace.clone())
                .or_insert_with(|| NamespaceSummary::new(namespace))
                .problem_pods
                .push(pod_info);
        }
    }

    let mut namespaces: Vec<_> = namespaces.into_values().map(finalize_namespace).collect();
    namespaces.sort_by(|left, right| {
        left.health
            .cmp(&right.health)
            .then_with(|| left.name.cmp(&right.name))
    });

    WorkloadSnapshot {
        target,
        namespaces,
        total_deployments,
        total_statefulsets,
        total_daemonsets,
        total_pods_healthy,
        total_pods_degraded,
        total_pods_failing,
    }
}

fn resource_namespace(namespace: &Option<String>) -> String {
    namespace.clone().unwrap_or_else(|| "default".to_string())
}

fn resource_name(name: &Option<String>) -> String {
    name.clone().unwrap_or_else(|| "unknown".to_string())
}

fn add_workload(
    namespaces: &mut HashMap<String, NamespaceSummary>,
    namespace: String,
    workload: WorkloadInfo,
) {
    namespaces
        .entry(namespace.clone())
        .or_insert_with(|| NamespaceSummary::new(namespace))
        .workloads
        .push(workload);
}

pub(crate) fn finalize_namespace(mut summary: NamespaceSummary) -> NamespaceSummary {
    summary.total_workloads = summary.workloads.len();
    summary.healthy_workloads = summary
        .workloads
        .iter()
        .filter(|workload| workload.health == HealthState::Healthy)
        .count();
    let worst_workload = summary
        .workloads
        .iter()
        .map(|workload| workload.health)
        .min()
        .unwrap_or(HealthState::Healthy);
    let worst_pod = summary
        .problem_pods
        .iter()
        .map(|pod| pod.issue.severity())
        .min()
        .unwrap_or(HealthState::Healthy);
    summary.health = worst_workload.min(worst_pod);
    summary
}

fn classify_workload_health(
    ready: i32,
    desired: i32,
    available: i32,
) -> (HealthState, Vec<String>) {
    if desired == 0 {
        return (HealthState::Healthy, Vec::new());
    }
    if ready == desired && available == desired {
        return (HealthState::Healthy, Vec::new());
    }
    if ready == 0 {
        return (
            HealthState::Failing,
            vec![format!("No pods ready (0/{desired})")],
        );
    }
    if ready < desired {
        return (
            HealthState::Degraded,
            vec![format!("Partial: {ready}/{desired} ready")],
        );
    }
    (HealthState::Healthy, Vec::new())
}

/// Only current state enters the decision. Restart totals and last instances
/// are history and belong to each consumer's display or advisory.
#[derive(Clone, Copy)]
pub(crate) struct ContainerState<'a> {
    pub init: bool,
    pub ready: bool,
    pub waiting_reason: Option<&'a str>,
    pub terminated: Option<(i64, &'a str)>,
}

/// Container indexes refer to init containers followed by app containers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PodEvidence {
    Pod,
    Waiting(usize),
    Terminated(usize),
    Unschedulable,
    Pending,
    NotReady(Option<usize>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PodDecision<'a> {
    pub severity: HealthState,
    pub reason: &'a str,
    pub evidence: PodEvidence,
}

/// The detail pane's current-state policy, shared by all pod consumers.
/// Absent Ready evidence is deliberately different from a non-True condition.
pub(crate) fn decide_pod_health<'a>(
    phase: &str,
    reason: &'a str,
    unschedulable: bool,
    not_ready: bool,
    containers: impl Iterator<Item = ContainerState<'a>> + Clone,
) -> Option<PodDecision<'a>> {
    let decision = |severity, reason, evidence| {
        Some(PodDecision {
            severity,
            reason,
            evidence,
        })
    };
    if phase == "Failed" && !reason.is_empty() {
        return decision(HealthState::Failing, reason, PodEvidence::Pod);
    }
    for (index, container) in containers.clone().enumerate() {
        if let Some(reason) = container.waiting_reason
            && crate::resources::is_error_reason(reason)
        {
            return decision(HealthState::Failing, reason, PodEvidence::Waiting(index));
        }
        if let Some((exit_code, reason)) = container.terminated
            && exit_code != 0
            && (phase == "Failed" || container.init)
        {
            let reason = if reason.is_empty() { "Error" } else { reason };
            return decision(HealthState::Failing, reason, PodEvidence::Terminated(index));
        }
    }
    if unschedulable {
        return decision(
            HealthState::Pending,
            "Unschedulable",
            PodEvidence::Unschedulable,
        );
    }
    if phase == "Pending" {
        return match containers
            .clone()
            .enumerate()
            .find_map(|(index, container)| container.waiting_reason.map(|reason| (index, reason)))
        {
            Some((index, reason)) => {
                decision(HealthState::Pending, reason, PodEvidence::Waiting(index))
            }
            None => decision(HealthState::Pending, "Pending", PodEvidence::Pending),
        };
    }
    if phase == "Running" && not_ready {
        let index = containers
            .enumerate()
            .find_map(|(index, container)| (!container.init && !container.ready).then_some(index));
        return decision(
            HealthState::Degraded,
            "Not ready",
            PodEvidence::NotReady(index),
        );
    }
    None
}

/// Borrow the API pod's compact facts without constructing a detail model.
pub(crate) fn pod_health(pod: &Pod) -> Option<PodDecision<'_>> {
    let status = pod.status.as_ref()?;
    let condition = |kind| {
        status
            .conditions
            .iter()
            .flatten()
            .find(|condition| condition.type_ == kind)
    };
    let containers = [
        (true, &status.init_container_statuses),
        (false, &status.container_statuses),
    ]
    .into_iter()
    .flat_map(|(init, statuses)| {
        statuses.iter().flatten().map(move |container| {
            let state = container.state.as_ref();
            ContainerState {
                init,
                ready: container.ready,
                waiting_reason: state
                    .and_then(|s| s.waiting.as_ref())
                    .map(|waiting| waiting.reason.as_deref().unwrap_or("")),
                terminated: state.and_then(|s| s.terminated.as_ref()).map(|ended| {
                    (
                        i64::from(ended.exit_code),
                        ended.reason.as_deref().unwrap_or(""),
                    )
                }),
            }
        })
    });
    decide_pod_health(
        status.phase.as_deref().unwrap_or("Unknown"),
        status.reason.as_deref().unwrap_or(""),
        condition("PodScheduled").is_some_and(|c| c.status == "False"),
        condition("Ready").is_some_and(|c| c.status != "True"),
        containers,
    )
}

fn decision_issue(decision: PodDecision<'_>) -> PodIssue {
    match (decision.severity, decision.reason) {
        (HealthState::Failing, "CrashLoopBackOff") => PodIssue::CrashLoopBackOff,
        (HealthState::Failing, "ImagePullBackOff") => PodIssue::ImagePullBackOff,
        (HealthState::Failing, "ErrImagePull") => PodIssue::ErrImagePull,
        (HealthState::Failing, "OOMKilled") => PodIssue::OOMKilled,
        (HealthState::Failing, "Error") => PodIssue::Error,
        (HealthState::Failing, reason) => PodIssue::FailingReason(reason.into()),
        (HealthState::Pending, "Pending") => PodIssue::Pending,
        (HealthState::Pending, reason) => PodIssue::WaitingReason(reason.into()),
        _ => PodIssue::NotReady,
    }
}

pub(crate) fn analyze_pod(pod: &Pod) -> (i32, Option<PodIssue>) {
    let Some(status) = pod.status.as_ref() else {
        return (0, None);
    };
    // Keep today's app-container restart total as a summary-only advisory.
    let restarts = status
        .container_statuses
        .iter()
        .flatten()
        .map(|c| c.restart_count)
        .sum();
    let issue = pod_health(pod).map(decision_issue).or_else(|| {
        if status.phase.as_deref() == Some("Failed") {
            // Missing reasons keep the summary's existing phase fallback.
            Some(PodIssue::Error)
        } else {
            summary_restart_advisory(restarts)
        }
    });
    (restarts, issue)
}

fn summary_restart_advisory(restarts: i32) -> Option<PodIssue> {
    (restarts >= HIGH_RESTART_THRESHOLD).then_some(PodIssue::HighRestarts(restarts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_to_zero_workload_is_healthy() {
        assert_eq!(
            classify_workload_health(0, 0, 0),
            (HealthState::Healthy, Vec::new())
        );
    }

    #[test]
    fn workload_replica_deficits_are_classified() {
        assert_eq!(
            classify_workload_health(0, 3, 0),
            (
                HealthState::Failing,
                vec!["No pods ready (0/3)".to_string()]
            )
        );
        assert_eq!(
            classify_workload_health(2, 3, 2),
            (
                HealthState::Degraded,
                vec!["Partial: 2/3 ready".to_string()]
            )
        );
    }

    #[test]
    fn pod_waiting_failures_take_precedence_over_restart_counts() {
        let pod = serde_json::from_value(serde_json::json!({
            "status": {"phase": "Running", "containerStatuses": [{
                "name": "app", "image": "example", "imageID": "example", "ready": false,
                "restartCount": HIGH_RESTART_THRESHOLD,
                "state": {"waiting": {"reason": "CrashLoopBackOff"}}
            }]}
        }))
        .unwrap();
        assert_eq!(
            analyze_pod(&pod),
            (HIGH_RESTART_THRESHOLD, Some(PodIssue::CrashLoopBackOff))
        );
    }

    #[test]
    fn only_a_container_that_ran_and_stopped_died() {
        for issue in [
            PodIssue::CrashLoopBackOff,
            PodIssue::OOMKilled,
            PodIssue::Error,
        ] {
            assert!(issue.died(), "{issue:?}");
            assert_eq!(issue.severity(), HealthState::Failing);
        }
        for issue in [
            PodIssue::ImagePullBackOff,
            PodIssue::ErrImagePull,
            PodIssue::Pending,
            PodIssue::HighRestarts(HIGH_RESTART_THRESHOLD),
            PodIssue::Unknown("ContainerCreating".into()),
        ] {
            assert!(!issue.died(), "{issue:?}");
        }
    }

    #[test]
    fn pod_pending_and_high_restarts_are_classified() {
        assert_eq!(
            decide_pod_health("Pending", "", false, false, [].into_iter()).map(decision_issue),
            Some(PodIssue::Pending)
        );
        assert_eq!(
            summary_restart_advisory(HIGH_RESTART_THRESHOLD),
            Some(PodIssue::HighRestarts(HIGH_RESTART_THRESHOLD))
        );
    }
}
