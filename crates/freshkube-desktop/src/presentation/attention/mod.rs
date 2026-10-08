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
    workloads::{HealthState, PodIssue, WorkloadKind},
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

/// Needs attention's severity groups, in the order they show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AttentionGroup {
    Failing,
    Warning,
    /// Last-known evidence from a stale source.
    Unknown,
}

impl AttentionGroup {
    fn of(tone: Tone) -> Self {
        match tone {
            Tone::Crit | Tone::Died => Self::Failing,
            Tone::Warn => Self::Warning,
            _ => Self::Unknown,
        }
    }

    pub(crate) fn index(self) -> usize {
        self as usize
    }

    pub(crate) fn tone(self) -> Tone {
        match self {
            Self::Failing => Tone::Crit,
            Self::Warning => Tone::Warn,
            Self::Unknown => Tone::Unknown,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Failing => "Failing",
            Self::Warning => "Warning",
            Self::Unknown => "Unknown",
        }
    }

    /// `overview-group-failing`, …
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Failing => "overview-group-failing",
            Self::Warning => "overview-group-warning",
            Self::Unknown => "overview-group-unknown",
        }
    }
}

/// A group header's detail for each group: how many problems it holds,
/// counted before any cap.
pub(crate) type GroupDetails = [SharedString; 3];

fn details(counts: [usize; 3]) -> GroupDetails {
    counts.map(|count| {
        format!(
            "{count} {}",
            if count == 1 { "problem" } else { "problems" }
        )
        .into()
    })
}

#[derive(Clone, Debug)]
pub(crate) struct AttentionRow {
    pub(crate) id: SharedString,
    pub(crate) kind: &'static str,
    pub(crate) name: SharedString,
    pub(crate) reason: SharedString,
    pub(crate) tone: Tone,
    /// Set from the final tone when the rows are finished.
    pub(crate) group: AttentionGroup,
    pub(crate) open: Destination,
    pub(crate) logs: Option<Destination>,
    pub(crate) node: Option<String>,
    pub(crate) open_node: Option<Destination>,
    since: Option<DateTime<Utc>>,
}

#[derive(Default)]
pub(crate) struct Attention {
    pub(crate) complete: bool,
    pub(crate) rows: Vec<AttentionRow>,
    pub(crate) details: GroupDetails,
    pub(crate) by_node: BTreeMap<String, Vec<AttentionRow>>,
    pub(crate) node_details: BTreeMap<String, GroupDetails>,
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
            connection: None,
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
/// How long a state has held, in the format Pods' group rows give it
/// (`format_age`), so both pages say "198d" for the same node.
fn standing(since: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    since
        .map(|since| {
            let seconds = now.signed_duration_since(since).num_seconds().max(0);
            format!(
                " for {}",
                crate::resources::model::format_age(seconds as u64)
            )
        })
        .unwrap_or_default()
}

/// Names the cluster the rows were derived in on every object they open, so
/// a row left on screen after the cluster changed cannot open a same-named
/// object in the next one. `None` leaves them naming the open cluster.
fn name_cluster(rows: &mut [AttentionRow], connection: Option<&str>) {
    let Some(connection) = connection else {
        return;
    };
    for row in rows {
        let destinations = [
            Some(&mut row.open),
            row.logs.as_mut(),
            row.open_node.as_mut(),
        ];
        for destination in destinations.into_iter().flatten() {
            if let Destination::Object(_, object, _) = destination {
                object.connection = Some(connection.to_owned());
            }
        }
    }
}

/// `connection` is the open cluster's `kube_identity()`, when it has one.
pub(crate) fn build(
    nodes: &[NodeRow],
    kubernetes: Option<&KubernetesSummary>,
    talos: Option<&ClusterOverview>,
    connection: Option<&str>,
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
        for row in &mut rows {
            use freshkube_core::kubernetes_summary::Source;
            let source = match row.kind {
                "Pod" => Some(Source::Pods),
                "Deployment" => Some(Source::Deployments),
                "StatefulSet" => Some(Source::StatefulSets),
                "DaemonSet" => Some(Source::DaemonSets),
                "Claim" => Some(Source::Claims),
                _ => None,
            };
            if source
                .and_then(|source| summary.observations.get(&source))
                .is_some_and(|observation| observation.is_stale())
            {
                row.reason = format!("Last known · {}", row.reason).into();
                row.tone = Tone::Unknown;
            }
        }
    }
    if let Some(cluster) = talos {
        append_etcd(cluster, &mut rows);
    }
    name_cluster(&mut rows, connection);
    let mut result = finish(rows);
    result.complete = kubernetes.is_some_and(|summary| {
        summary.nodes.is_current()
            && summary.pods.is_current()
            && summary.claims.is_current()
            && summary.events.is_current()
            && summary.workloads.unavailable().is_empty()
    });
    result
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
        && row.kubernetes_current
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
            && let Some((memory_tone, _)) =
                crate::ui::memory_tone(super::memory_level(memory.percent()))
        {
            problems.push(format!(
                "Memory at {} %",
                super::whole_percent(memory.percent())
            ));
            if memory_tone == Tone::Crit {
                tone = Tone::Crit;
            }
        }
    }
    if !problems.is_empty() {
        rows.push(AttentionRow {
            id: format!("attention-node-{}-{}", row.name, row.name).into(),
            kind: "Node",
            name: row.name.clone(),
            reason: problems.join(" · ").into(),
            tone,
            group: AttentionGroup::Warning,
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
                group: AttentionGroup::Warning,
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
            let tone = pod_issue_tone(&pod.issue);
            rows.push(AttentionRow {
                id: format!("attention-pod-{}-{}", pod.namespace, pod.name).into(),
                kind: "Pod",
                name: format!("{}/{}", pod.namespace, pod.name).into(),
                reason: pod.issue.label().to_owned().into(),
                tone,
                group: AttentionGroup::Warning,
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

/// A pod's tone: the skull when a container ran and stopped, critical when
/// it otherwise fails, such as an image it can't pull, and a warning
/// otherwise.
fn pod_issue_tone(issue: &PodIssue) -> Tone {
    if issue.died() {
        Tone::Died
    } else if issue.severity() == HealthState::Failing {
        Tone::Crit
    } else {
        Tone::Warn
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
                group: AttentionGroup::Warning,
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
                group: AttentionGroup::Warning,
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
            freshkube_core::indicators::quorum(etcd.healthy, etcd.total).required
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
            group: AttentionGroup::Warning,
            open: Destination::Page(Page::Etcd),
            logs: None,
            open_node: None,
            node: None,
            since: None,
        });
    }
}

fn finish(mut rows: Vec<AttentionRow>) -> Attention {
    for row in &mut rows {
        row.group = AttentionGroup::of(row.tone);
    }
    // Failing, then current warnings, then last-known evidence, so the cap
    // drops stale rows first.
    rows.sort_by(|left, right| {
        left.group
            .cmp(&right.group)
            .then_with(|| {
                left.since
                    .unwrap_or(DateTime::<Utc>::MAX_UTC)
                    .cmp(&right.since.unwrap_or(DateTime::<Utc>::MAX_UTC))
            })
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    let total = rows.len();
    let mut counts = [0; 3];
    let mut by_node: BTreeMap<String, Vec<AttentionRow>> = BTreeMap::new();
    let mut node_counts: BTreeMap<String, [usize; 3]> = BTreeMap::new();
    for row in &rows {
        counts[row.group.index()] += 1;
        if let Some(node) = &row.node {
            node_counts.entry(node.clone()).or_default()[row.group.index()] += 1;
            let node_rows = by_node.entry(node.clone()).or_default();
            if node_rows.len() < 50 {
                node_rows.push(row.clone());
            }
        }
    }
    rows.truncate(50);
    Attention {
        complete: false,
        rows,
        details: details(counts),
        by_node,
        node_details: node_counts
            .into_iter()
            .map(|(node, counts)| (node, details(counts)))
            .collect(),
        total,
        more: format!("Show all {total}").into(),
    }
}

#[cfg(test)]
mod tests;
