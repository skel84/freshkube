//! Example workloads for `--fixture`.
use super::*;

/// Builds a controller the way the collector classifies it.
fn workload(
    namespace: &str,
    name: &str,
    kind: WorkloadKind,
    ready: i32,
    desired: i32,
) -> WorkloadInfo {
    let (health, issues) = if desired == 0 || ready == desired {
        (HealthState::Healthy, Vec::new())
    } else if ready == 0 {
        (
            HealthState::Failing,
            vec![format!("No pods ready (0/{desired})")],
        )
    } else {
        (
            HealthState::Degraded,
            vec![format!("Partial: {ready}/{desired} ready")],
        )
    };
    WorkloadInfo {
        name: name.into(),
        namespace: namespace.into(),
        kind,
        ready,
        desired,
        health,
        issues,
    }
}

fn pod(
    namespace: &str,
    name: &str,
    node: Option<&str>,
    phase: &str,
    restarts: i32,
    issue: PodIssue,
    age_minutes: i64,
) -> PodInfo {
    PodInfo {
        name: name.into(),
        namespace: namespace.into(),
        node: node.map(str::to_owned),
        phase: phase.into(),
        restarts,
        issue,
        created_at: Some(Utc::now() - chrono::Duration::minutes(age_minutes)),
    }
}

fn namespace(
    name: &str,
    workloads: Vec<WorkloadInfo>,
    problem_pods: Vec<PodInfo>,
) -> NamespaceSummary {
    let worst_workload = workloads
        .iter()
        .map(|workload| workload.health)
        .min()
        .unwrap_or(HealthState::Healthy);
    let worst_pod = problem_pods
        .iter()
        .map(|pod| pod.issue.severity())
        .min()
        .unwrap_or(HealthState::Healthy);
    NamespaceSummary {
        name: name.into(),
        health: worst_workload.min(worst_pod),
        total_workloads: workloads.len(),
        healthy_workloads: workloads
            .iter()
            .filter(|workload| workload.health == HealthState::Healthy)
            .count(),
        workloads,
        problem_pods,
    }
}

/// Example workloads for `--fixture`: the usual system components plus a few
/// applications. The degraded worker has crashing and restarting pods; the
/// silent worker simply isn't counted as ready. Nothing here depends on which
/// node is the target.
pub(super) fn example(source: &ScreenSource) -> WorkloadData {
    use WorkloadKind::{DaemonSet, Deployment, StatefulSet};
    let nodes = &source.nodes;
    let total = nodes.len() as i32;
    let degraded = nodes
        .iter()
        .find(|node| node.name.contains("wk-fra1-02"))
        .map(|node| node.name.as_str());
    let silent = nodes
        .iter()
        .find(|node| node.name.contains("wk-fra1-03"))
        .map(|node| node.name.as_str());
    let node_ready = total - i32::from(degraded.is_some()) - i32::from(silent.is_some());
    // Applications land on the degraded worker when there is one.
    let app_node = degraded.or_else(|| {
        nodes
            .iter()
            .find(|node| node.role != crate::presentation::Role::ControlPlane)
            .map(|node| node.name.as_str())
    });

    let kube_system = namespace(
        "kube-system",
        vec![
            workload("kube-system", "coredns", Deployment, 2, 2),
            workload("kube-system", "metrics-server", Deployment, 1, 1),
            workload("kube-system", "kube-flannel", DaemonSet, node_ready, total),
            workload("kube-system", "kube-proxy", DaemonSet, node_ready, total),
        ],
        degraded
            .map(|node| {
                vec![
                    pod(
                        "kube-system",
                        "kube-flannel-q7x4d",
                        Some(node),
                        "Running",
                        14,
                        PodIssue::CrashLoopBackOff,
                        60 * 24 * 3,
                    ),
                    pod(
                        "kube-system",
                        "kube-proxy-9tn2m",
                        Some(node),
                        "Running",
                        6,
                        PodIssue::HighRestarts(6),
                        60 * 24 * 3,
                    ),
                ]
            })
            .unwrap_or_default(),
    );
    let shop = namespace(
        "shop",
        vec![
            workload("shop", "web", Deployment, 3, 3),
            workload("shop", "api", Deployment, 2, 3),
            workload("shop", "worker", Deployment, 0, 2),
            workload("shop", "postgres", StatefulSet, 1, 1),
        ],
        vec![
            pod(
                "shop",
                "api-6d8f7c9b5-k2w9z",
                app_node,
                "Running",
                7,
                PodIssue::OOMKilled,
                95,
            ),
            pod(
                "shop",
                "worker-5c7b8d6f4-hq8r2",
                app_node,
                "Pending",
                0,
                PodIssue::ImagePullBackOff,
                12,
            ),
            pod(
                "shop",
                "worker-5c7b8d6f4-zl4vn",
                app_node,
                "Pending",
                0,
                PodIssue::ErrImagePull,
                12,
            ),
        ],
    );
    let monitoring = namespace(
        "monitoring",
        vec![
            workload("monitoring", "grafana", Deployment, 1, 1),
            workload("monitoring", "alertmanager", Deployment, 1, 1),
            workload("monitoring", "prometheus", StatefulSet, 1, 1),
            workload("monitoring", "node-exporter", DaemonSet, node_ready, total),
        ],
        Vec::new(),
    );
    let cert_manager = namespace(
        "cert-manager",
        vec![
            workload("cert-manager", "cert-manager", Deployment, 1, 1),
            workload("cert-manager", "cert-manager-cainjector", Deployment, 1, 1),
            workload("cert-manager", "cert-manager-webhook", Deployment, 1, 1),
        ],
        Vec::new(),
    );
    let mut namespaces = vec![kube_system, shop, monitoring, cert_manager];
    namespaces.sort_by(|left, right| {
        left.health
            .cmp(&right.health)
            .then_with(|| left.name.cmp(&right.name))
    });
    let count_kind = |kind: WorkloadKind| {
        namespaces
            .iter()
            .flat_map(|ns| &ns.workloads)
            .filter(|workload| workload.kind == kind)
            .count()
    };
    let healthy_pods: i32 = namespaces
        .iter()
        .flat_map(|ns| &ns.workloads)
        .map(|workload| workload.ready)
        .sum();
    let pods_with = |severity: fn(HealthState) -> bool| {
        namespaces
            .iter()
            .flat_map(|ns| &ns.problem_pods)
            .filter(|pod| severity(pod.issue.severity()))
            .count()
    };
    let snapshot = WorkloadSnapshot {
        target: source.target.context.clone(),
        total_deployments: count_kind(Deployment),
        total_statefulsets: count_kind(StatefulSet),
        total_daemonsets: count_kind(DaemonSet),
        total_pods_healthy: healthy_pods.max(0) as usize,
        total_pods_degraded: pods_with(|health| {
            matches!(health, HealthState::Degraded | HealthState::Pending)
        }),
        total_pods_failing: pods_with(|health| health == HealthState::Failing),
        namespaces,
    };
    WorkloadData {
        snapshot,
        unavailable: Vec::new(),
        missing_notice: Vec::new(),
    }
}
