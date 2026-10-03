//! Shared attention rows, derived when either cluster summary changes.
use crate::{
    desktop::{
        Page,
        nodes::{NodeKey, NodeRow, NodeTab},
    },
    resources::{Tab, model::ObjectRef},
    ui::Tone,
};
use chrono::{DateTime, Utc};
use freshkube_core::{
    cluster_overview::ClusterOverview,
    kubernetes_summary::KubernetesSummary,
    workloads::{HealthState, WorkloadKind},
};
use gpui_kit::SharedString;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(crate) enum Destination {
    Node(NodeKey, NodeTab),
    Object(&'static str, ObjectRef, Tab),
    Page(Page),
    Service {
        node: String,
        service: String,
        logs: bool,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct AttentionRow {
    pub(crate) id: SharedString,
    pub(crate) kind: &'static str,
    pub(crate) name: SharedString,
    pub(crate) reason: SharedString,
    pub(crate) tone: Tone,
    pub(crate) open: Destination,
    pub(crate) logs: Option<Destination>,
    pub(crate) node: Option<String>,
    pub(crate) open_node: Option<Destination>,
    since: Option<DateTime<Utc>>,
}

#[derive(Default)]
pub(crate) struct Attention {
    pub(crate) rows: Vec<AttentionRow>,
    pub(crate) by_node: BTreeMap<String, Vec<AttentionRow>>,
    pub(crate) total: usize,
    pub(crate) more: SharedString,
}

fn object(
    summary: &KubernetesSummary,
    kind: &'static str,
    namespace: &str,
    name: &str,
    tab: Tab,
) -> Destination {
    Destination::Object(
        kind,
        ObjectRef {
            namespace: namespace.into(),
            name: name.into(),
            uid: summary
                .references
                .get(&(kind.into(), namespace.into(), name.into()))
                .cloned()
                .unwrap_or_default(),
        },
        tab,
    )
}
fn standing(since: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let seconds = since.map(|since| now.signed_duration_since(since).num_seconds().max(0));
    match seconds {
        Some(seconds) if seconds >= 3600 => format!(" for {} h", seconds / 3600),
        Some(seconds) if seconds >= 60 => format!(" for {} min", seconds / 60),
        Some(seconds) => format!(" for {seconds} s"),
        None => String::new(),
    }
}

pub(crate) fn build(
    nodes: &[NodeRow],
    kubernetes: Option<&KubernetesSummary>,
    talos: Option<&ClusterOverview>,
    now: DateTime<Utc>,
) -> Attention {
    let mut rows = Vec::new();
    for row in nodes {
        append_node_problem(row, now, &mut rows);
        append_services(row, &mut rows);
    }
    if let Some(summary) = kubernetes {
        append_pods(summary, &mut rows);
        append_workloads(summary, &mut rows);
        append_claims(summary, &mut rows);
    }
    if let Some(cluster) = talos {
        append_etcd(cluster, &mut rows);
    }
    finish(rows)
}

fn append_node_problem(row: &NodeRow, now: DateTime<Utc>, rows: &mut Vec<AttentionRow>) {
    let mut problems = Vec::new();
    let mut since = None;
    let mut tone = Tone::Warn;
    if row.talos.as_ref().is_some_and(|node| !node.responding) {
        problems.push("Talos API not answering".to_owned());
        tone = Tone::Crit;
    }
    if let Some(node) = &row.kubernetes
        && !node.is_ready()
    {
        since = node.ready().and_then(|condition| condition.since);
        problems.push(format!("Kubernetes NotReady{}", standing(since, now)));
        tone = Tone::Crit;
    }
    if let Some(node) = &row.talos {
        let count = node.unhealthy_services().count();
        if count > 0 {
            problems.push(format!("{count} unhealthy system services"));
        }
        if let Some(memory) = node.memory
            && memory.percent() >= 90.
        {
            problems.push(format!("Memory at {:.0} %", memory.percent()));
        }
    }
    if !problems.is_empty() {
        rows.push(AttentionRow {
            id: format!("attention-node-{}-{}", row.name, row.name).into(),
            kind: "Node",
            name: row.name.clone(),
            reason: problems.join(" · ").into(),
            tone,
            open: Destination::Node(row.key.clone(), NodeTab::Overview),
            logs: None,
            open_node: None,
            node: Some(row.name.to_string()),
            since,
        });
    }
}

fn append_services(row: &NodeRow, rows: &mut Vec<AttentionRow>) {
    if let Some(node) = &row.talos {
        for service in node.unhealthy_services() {
            let reason = service
                .health
                .as_ref()
                .map(|health| health.last_message.as_str())
                .filter(|message| !message.is_empty())
                .unwrap_or("Service is unhealthy");
            rows.push(AttentionRow {
                id: format!("attention-service-{}-{}", node.name, service.id).into(),
                kind: "System service",
                name: format!("{}/{}", node.name, service.id).into(),
                reason: reason.to_owned().into(),
                tone: Tone::Warn,
                open: Destination::Service {
                    node: node.name.clone(),
                    service: service.id.clone(),
                    logs: false,
                },
                logs: Some(Destination::Service {
                    node: node.name.clone(),
                    service: service.id.clone(),
                    logs: true,
                }),
                open_node: Some(Destination::Node(row.key.clone(), NodeTab::Services)),
                node: Some(row.name.to_string()),
                since: None,
            });
        }
    }
}

fn append_pods(summary: &KubernetesSummary, rows: &mut Vec<AttentionRow>) {
    if let Some(pods) = summary.pods.loaded() {
        for pod in &pods.issues {
            let tone = if pod.issue.severity() == HealthState::Failing {
                Tone::Crit
            } else {
                Tone::Warn
            };
            rows.push(AttentionRow {
                id: format!("attention-pod-{}-{}", pod.namespace, pod.name).into(),
                kind: "Pod",
                name: format!("{}/{}", pod.namespace, pod.name).into(),
                reason: pod.issue.label().to_owned().into(),
                tone,
                open: object(summary, "pods", &pod.namespace, &pod.name, Tab::Overview),
                logs: Some(object(
                    summary,
                    "pods",
                    &pod.namespace,
                    &pod.name,
                    Tab::Logs,
                )),
                node: pod.node.clone(),
                open_node: None,
                since: pod.created_at,
            });
        }
    }
}

fn append_workloads(summary: &KubernetesSummary, rows: &mut Vec<AttentionRow>) {
    if let Some(snapshot) = summary.workloads.snapshot() {
        for workload in snapshot.namespaces.iter().flat_map(|ns| &ns.workloads) {
            if workload.health == HealthState::Healthy {
                continue;
            }
            let (kind, label) = match workload.kind {
                WorkloadKind::Deployment => ("deployments.apps", "Deployment"),
                WorkloadKind::StatefulSet => ("statefulsets.apps", "StatefulSet"),
                WorkloadKind::DaemonSet => ("daemonsets.apps", "DaemonSet"),
            };
            rows.push(AttentionRow {
                id: format!(
                    "attention-{}-{}-{}",
                    kind, workload.namespace, workload.name
                )
                .into(),
                kind: label,
                name: format!("{}/{}", workload.namespace, workload.name).into(),
                reason: workload.issues.join(" · ").into(),
                tone: if workload.health == HealthState::Failing {
                    Tone::Crit
                } else {
                    Tone::Warn
                },
                open: object(
                    summary,
                    kind,
                    &workload.namespace,
                    &workload.name,
                    Tab::Overview,
                ),
                logs: None,
                open_node: None,
                node: None,
                since: None,
            });
        }
    }
}

fn append_claims(summary: &KubernetesSummary, rows: &mut Vec<AttentionRow>) {
    if let Some(claims) = summary.claims.loaded() {
        for claim in &claims.pending {
            rows.push(AttentionRow {
                id: format!("attention-claim-{}-{}", claim.namespace, claim.name).into(),
                kind: "Claim",
                name: format!("{}/{}", claim.namespace, claim.name).into(),
                reason: claim.reason.clone().into(),
                tone: Tone::Warn,
                open: object(
                    summary,
                    "persistentvolumeclaims",
                    &claim.namespace,
                    &claim.name,
                    Tab::Overview,
                ),
                logs: None,
                open_node: None,
                node: None,
                since: claim.since,
            });
        }
    }
}

fn append_etcd(cluster: &ClusterOverview, rows: &mut Vec<AttentionRow>) {
    let mut problems = Vec::new();
    if let Some(etcd) = &cluster.etcd_summary
        && !etcd.has_quorum
    {
        problems.push(format!(
            "{} of {} members answered; quorum needs {}",
            etcd.healthy,
            etcd.total,
            etcd.total / 2 + 1
        ));
    }
    if let Some(alarms) = &cluster.etcd_alarms {
        let mut alarms = alarms
            .iter()
            .filter(|alarm| alarm.alarm_type != talos_rs::EtcdAlarmType::None)
            .map(|alarm| format!("Member {}: {}", alarm.member_id, alarm.alarm_type.as_str()))
            .collect::<Vec<_>>();
        alarms.sort();
        alarms.dedup();
        problems.extend(alarms);
    }
    if !problems.is_empty() {
        rows.push(AttentionRow {
            id: "attention-etcd-cluster-etcd".into(),
            kind: "etcd",
            name: "etcd".into(),
            reason: problems.join(" · ").into(),
            tone: Tone::Crit,
            open: Destination::Page(Page::Etcd),
            logs: None,
            open_node: None,
            node: None,
            since: None,
        });
    }
}

fn finish(mut rows: Vec<AttentionRow>) -> Attention {
    rows.sort_by(|left, right| {
        let rank = |tone| if tone == Tone::Crit { 0 } else { 1 };
        rank(left.tone)
            .cmp(&rank(right.tone))
            .then_with(|| {
                left.since
                    .unwrap_or(DateTime::<Utc>::MAX_UTC)
                    .cmp(&right.since.unwrap_or(DateTime::<Utc>::MAX_UTC))
            })
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    let total = rows.len();
    let mut by_node: BTreeMap<String, Vec<AttentionRow>> = BTreeMap::new();
    for row in &rows {
        if let Some(node) = &row.node {
            let node_rows = by_node.entry(node.clone()).or_default();
            if node_rows.len() < 50 {
                node_rows.push(row.clone());
            }
        }
    }
    rows.truncate(50);
    Attention {
        rows,
        by_node,
        total,
        more: format!("Show all {total}").into(),
    }
}

#[cfg(test)]
mod tests;
