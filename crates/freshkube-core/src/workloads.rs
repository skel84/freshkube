//! Framework-neutral Kubernetes workload health collection.
//!
//! Workload state is taken directly from Kubernetes API resources. A failed
//! list request makes only that resource kind unavailable: successful lists
//! remain in the returned snapshot so a frontend can still show useful state.

use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, Utc};
use k8s_openapi::api::{
    apps::v1::{DaemonSet, Deployment, StatefulSet},
    core::v1::Pod,
};
use kube::{
    Client,
    api::{Api, ListParams},
};

use crate::{
    constants::HIGH_RESTART_THRESHOLD,
    indicators::{HasHealth, HealthIndicator},
};

/// Default upper bound for the concurrent Kubernetes list requests.
pub const DEFAULT_WORKLOAD_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

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
            | Self::Error => HealthState::Failing,
            Self::Pending => HealthState::Pending,
            Self::HighRestarts(_) | Self::Unknown(_) => HealthState::Degraded,
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

/// Collects Kubernetes workload state for an explicit cluster target.
///
/// `client` must be created from the Talos-pinned kubeconfig helpers. All four
/// authoritative API lists are requested concurrently. A list error or timeout
/// produces [`WorkloadCollectionOutcome::Partial`] when another list succeeded,
/// and [`WorkloadCollectionOutcome::Unavailable`] only when none did.
pub async fn collect_workloads(
    target: impl Into<String>,
    client: Client,
) -> WorkloadCollectionOutcome {
    collect_workloads_with_timeout(target, client, DEFAULT_WORKLOAD_REQUEST_TIMEOUT).await
}

/// Collects workloads with a caller-selected deadline for all concurrent lists.
pub async fn collect_workloads_with_timeout(
    target: impl Into<String>,
    client: Client,
    timeout: Duration,
) -> WorkloadCollectionOutcome {
    let target = target.into();
    let deployments_api: Api<Deployment> = Api::all(client.clone());
    let statefulsets_api: Api<StatefulSet> = Api::all(client.clone());
    let daemonsets_api: Api<DaemonSet> = Api::all(client.clone());
    let pods_api: Api<Pod> = Api::all(client);
    let list_params = ListParams::default().match_any();

    let fetched = tokio::time::timeout(timeout, async {
        tokio::join!(
            deployments_api.list(&list_params),
            statefulsets_api.list(&list_params),
            daemonsets_api.list(&list_params),
            pods_api.list(&list_params),
        )
    })
    .await;

    let (deployments, statefulsets, daemonsets, pods, errors) = match fetched {
        Ok((deployments, statefulsets, daemonsets, pods)) => {
            let mut errors = Vec::new();
            let deployments = deployments.map_or_else(
                |error| {
                    errors.push(source_error(WorkloadSource::Deployments, error));
                    Vec::new()
                },
                |list| list.items,
            );
            let statefulsets = statefulsets.map_or_else(
                |error| {
                    errors.push(source_error(WorkloadSource::StatefulSets, error));
                    Vec::new()
                },
                |list| list.items,
            );
            let daemonsets = daemonsets.map_or_else(
                |error| {
                    errors.push(source_error(WorkloadSource::DaemonSets, error));
                    Vec::new()
                },
                |list| list.items,
            );
            let pods = pods.map_or_else(
                |error| {
                    errors.push(source_error(WorkloadSource::Pods, error));
                    Vec::new()
                },
                |list| list.items,
            );
            (deployments, statefulsets, daemonsets, pods, errors)
        }
        Err(_) => {
            let message = format!("Request timed out after {}s", timeout.as_secs());
            let errors = [
                WorkloadSource::Deployments,
                WorkloadSource::StatefulSets,
                WorkloadSource::DaemonSets,
                WorkloadSource::Pods,
            ]
            .into_iter()
            .map(|source| WorkloadSourceError {
                source,
                message: message.clone(),
            })
            .collect();
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), errors)
        }
    };

    if errors.len() == 4 {
        return WorkloadCollectionOutcome::Unavailable { target, errors };
    }

    let snapshot = build_snapshot(target, deployments, statefulsets, daemonsets, pods);
    if errors.is_empty() {
        WorkloadCollectionOutcome::Complete(snapshot)
    } else {
        WorkloadCollectionOutcome::Partial {
            snapshot,
            unavailable: errors,
        }
    }
}

fn source_error(source: WorkloadSource, error: kube::Error) -> WorkloadSourceError {
    WorkloadSourceError {
        source,
        message: error.to_string(),
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

#[derive(Clone, Copy)]
struct ContainerState<'a> {
    waiting_reason: Option<&'a str>,
    terminated_reason: Option<&'a str>,
}

pub(crate) fn analyze_pod(pod: &Pod) -> (i32, Option<PodIssue>) {
    let Some(status) = pod.status.as_ref() else {
        return (0, None);
    };
    let phase = status.phase.as_deref().unwrap_or("Unknown");
    let restarts = status
        .container_statuses
        .as_ref()
        .map(|statuses| statuses.iter().map(|status| status.restart_count).sum())
        .unwrap_or(0);
    let containers = status.container_statuses.iter().flatten().map(|status| {
        let state = status.state.as_ref();
        ContainerState {
            waiting_reason: state
                .and_then(|state| state.waiting.as_ref())
                .and_then(|waiting| waiting.reason.as_deref()),
            terminated_reason: state
                .and_then(|state| state.terminated.as_ref())
                .and_then(|terminated| terminated.reason.as_deref()),
        }
    });
    (restarts, classify_pod_status(phase, restarts, containers))
}

fn classify_pod_status<'a>(
    phase: &str,
    restarts: i32,
    containers: impl IntoIterator<Item = ContainerState<'a>>,
) -> Option<PodIssue> {
    for container in containers {
        match container.waiting_reason {
            Some("CrashLoopBackOff") => return Some(PodIssue::CrashLoopBackOff),
            Some("ImagePullBackOff") => return Some(PodIssue::ImagePullBackOff),
            Some("ErrImagePull") => return Some(PodIssue::ErrImagePull),
            _ => {}
        }
        match container.terminated_reason {
            Some("OOMKilled") => return Some(PodIssue::OOMKilled),
            Some("Error") => return Some(PodIssue::Error),
            _ => {}
        }
    }

    if phase == "Pending" {
        Some(PodIssue::Pending)
    } else if phase == "Failed" {
        Some(PodIssue::Error)
    } else if restarts >= HIGH_RESTART_THRESHOLD {
        Some(PodIssue::HighRestarts(restarts))
    } else {
        None
    }
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
        let containers = [ContainerState {
            waiting_reason: Some("CrashLoopBackOff"),
            terminated_reason: None,
        }];
        assert_eq!(
            classify_pod_status("Running", HIGH_RESTART_THRESHOLD, containers),
            Some(PodIssue::CrashLoopBackOff)
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
            classify_pod_status("Pending", 0, []),
            Some(PodIssue::Pending)
        );
        assert_eq!(
            classify_pod_status("Running", HIGH_RESTART_THRESHOLD, []),
            Some(PodIssue::HighRestarts(HIGH_RESTART_THRESHOLD))
        );
    }
}
