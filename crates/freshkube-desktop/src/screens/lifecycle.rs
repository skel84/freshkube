//! Lifecycle: per-node Talos and kubelet versions, version skew, config drift,
//! roster consistency between Talos discovery and Kubernetes, and the etcd
//! pre-operation audit.
//!
//! Read-only. Every source is kept apart: a source that didn't answer is shown
//! as "not reported" and named in the partial notice, never as failed, and no
//! comparison (drift, skew, roster) is made against data that wasn't read.
use std::collections::BTreeMap;

use freshkube_core::HealthIndicator;
use freshkube_core::QuorumState;
use freshkube_core::security_lifecycle::{
    ClusterIdentity, DiscoveryRosterEntry, EtcdPreOperationAudit, KubernetesNodeRosterEntry,
    LifecycleAlert, LifecycleAlertKind, LifecycleCollector, LifecycleSnapshot,
    NodeLifecycleSnapshot, SourceSnapshot, TimeSynchronizationAudit,
};
use futures::join;
use gpui_kit::assets::IconName;
use gpui_kit::component::{Sizable, button::Button, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;

use super::{
    Column, Loader, Scope, ScreenEvent, ScreenPanel, ScreenSource, cell, content_width,
    failure_banner, field, gated_page, header, mono, panel, partial_notice, stat, table_head,
    table_width,
};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, Tone};

const CONTEXT: &str = "TalosLifecycle";
const ROW_HEIGHT: f32 = 30.;
/// The details pane, when it sits beside the lists.
const DETAILS_WIDTH: f32 = 340.;
const GAP: f32 = 14.;

const COLUMNS: [Column; 6] = [
    Column {
        label: "Node",
        width: None,
    },
    Column {
        label: "Role",
        width: Some(120.),
    },
    Column {
        label: "Talos",
        width: Some(116.),
    },
    Column {
        label: "Kubelet",
        width: Some(116.),
    },
    Column {
        label: "Config",
        width: Some(108.),
    },
    Column {
        label: "Discovery · K8s",
        width: Some(132.),
    },
];

actions!(
    talos_lifecycle,
    [NextItem, PreviousItem, FirstItem, LastItem, ClearSelection]
);

/// What the screen shows: the core snapshot plus the kubelet version of every
/// Kubernetes node, which the core snapshot doesn't carry.
#[derive(Clone, Debug)]
struct LifecycleView {
    snapshot: LifecycleSnapshot,
    kubelets: SourceSnapshot<Vec<KubeletEntry>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct KubeletEntry {
    name: String,
    address: Option<String>,
    version: String,
}

/// A selectable thing: a node row or an alert.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Item {
    Node(String),
    Alert(usize),
}

pub(crate) struct LifecycleScreen {
    runtime: Handle,
    source: Option<ScreenSource>,
    loader: Loader<LifecycleView>,
    selected: Option<Item>,
    focus: FocusHandle,
}

impl EventEmitter<ScreenEvent> for LifecycleScreen {}

impl ScreenPanel for LifecycleScreen {
    fn new(runtime: Handle, _: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", NextItem, Some(CONTEXT)),
            KeyBinding::new("up", PreviousItem, Some(CONTEXT)),
            KeyBinding::new("home", FirstItem, Some(CONTEXT)),
            KeyBinding::new("end", LastItem, Some(CONTEXT)),
            KeyBinding::new("escape", ClearSelection, Some(CONTEXT)),
        ]);
        Self {
            runtime,
            source: None,
            loader: Loader::default(),
            selected: None,
            focus: cx.focus_handle(),
        }
    }

    fn set_source(&mut self, source: Option<ScreenSource>, _: &mut Window, cx: &mut Context<Self>) {
        let changed = self.source.as_ref().map(|source| &source.target)
            != source.as_ref().map(|source| &source.target);
        if changed {
            self.loader.reset();
            self.selected = None;
        }
        self.source = source;
        cx.notify();
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loader.data().is_none() && !self.loader.is_loading() {
            self.refresh(window, cx);
        }
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.loader.is_loading() {
            return;
        }
        let Some(live) = source.live.clone() else {
            self.loader.resolve(source.target.clone(), example(&source));
            cx.notify();
            return;
        };
        let context = source.target.context.clone();
        self.loader.load(
            source.target.clone(),
            &self.runtime,
            "lifecycle status",
            async move {
                let kubernetes = live.kubernetes().await;
                let had_client = kubernetes.is_ok();
                let collector = LifecycleCollector::new(live.config_path.clone());
                let (snapshot, kubelets) = join!(
                    collector.collect_with_kubernetes(&live.client, &context, kubernetes.clone()),
                    collect_kubelets(kubernetes),
                );
                if had_client && matches!(kubelets, SourceSnapshot::Unavailable { .. }) {
                    // The reused client failed; revalidate and rebuild next time.
                    live.forget_kubernetes();
                }
                Ok(LifecycleView { snapshot, kubelets })
            },
            |screen: &mut Self| &mut screen.loader,
            cx,
        );
        cx.notify();
    }
}

/// The kubelet version each Kubernetes node reports, read through the same
/// kubeconfig choice as the roster.
async fn collect_kubelets(
    client: Result<kube::Client, String>,
) -> SourceSnapshot<Vec<KubeletEntry>> {
    use kube::api::ListParams;
    use kube::core::{ApiResource, DynamicObject, GroupVersionKind};

    let client = match client {
        Ok(client) => client,
        Err(reason) => return SourceSnapshot::Unavailable { reason },
    };
    let resource = ApiResource::from_gvk(&GroupVersionKind::gvk("", "v1", "Node"));
    let api: kube::Api<DynamicObject> = kube::Api::all_with(client, &resource);
    let listed = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        api.list(&ListParams::default()),
    )
    .await;
    let list = match listed {
        Ok(Ok(list)) => list,
        Ok(Err(error)) => {
            return SourceSnapshot::Unavailable {
                reason: format!("Kubernetes node versions request failed: {error}"),
            };
        }
        Err(_) => {
            return SourceSnapshot::Unavailable {
                reason: "Kubernetes node versions request timed out".into(),
            };
        }
    };
    let mut skipped = 0;
    let entries: Vec<KubeletEntry> = list
        .items
        .iter()
        .take(256)
        .filter_map(|node| {
            let name = node.metadata.name.clone()?;
            let version = node
                .data
                .pointer("/status/nodeInfo/kubeletVersion")
                .and_then(|version| version.as_str())
                .filter(|version| !version.is_empty());
            let Some(version) = version else {
                skipped += 1;
                return None;
            };
            let address = node
                .data
                .pointer("/status/addresses")
                .and_then(|addresses| addresses.as_array())
                .and_then(|addresses| {
                    addresses.iter().find_map(|address| {
                        (address.get("type").and_then(|kind| kind.as_str()) == Some("InternalIP"))
                            .then(|| address.get("address").and_then(|a| a.as_str()))
                            .flatten()
                            .map(str::to_owned)
                    })
                });
            Some(KubeletEntry {
                name,
                address,
                version: version.to_owned(),
            })
        })
        .collect();
    if skipped > 0 || entries.is_empty() {
        SourceSnapshot::Partial {
            value: entries,
            warnings: vec![if skipped > 0 {
                format!("{skipped} Kubernetes node(s) reported no kubelet version")
            } else {
                "Kubernetes API returned no nodes".into()
            }],
        }
    } else {
        SourceSnapshot::Available(entries)
    }
}

// ---------------------------------------------------------------------------
// Derived data. Pure functions over the snapshot, so they can be tested alone.
// ---------------------------------------------------------------------------

/// How a node's config hash compares with the others that were read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drift {
    /// The hash wasn't read, so nothing can be said.
    Unknown,
    /// The only node whose hash was read.
    OnlyReading,
    InSync,
    Differs,
}

#[derive(Clone, Debug)]
struct NodeRow {
    name: String,
    address: Option<String>,
    role: Option<String>,
    talos: Result<String, String>,
    platform: Result<String, String>,
    kubelet: Result<String, String>,
    config: Result<String, String>,
    time: Result<TimeSynchronizationAudit, String>,
    drift: Drift,
    /// `None`: that roster couldn't be read, so presence is unknown.
    in_discovery: Option<bool>,
    in_kubernetes: Option<bool>,
    /// Newest kubelet version seen cluster-wide is newer than this node's.
    kubelet_behind: bool,
}

impl NodeRow {
    /// Whether any per-node source answered. A node with none is "not
    /// reported", not "down".
    fn reported(&self) -> bool {
        self.talos.is_ok() || self.config.is_ok() || self.time.is_ok()
    }

    fn role_label(&self) -> String {
        match self.role.as_deref() {
            Some("controlplane") => "Control plane".into(),
            Some("worker") => "Worker".into(),
            Some(other) if !other.is_empty() => other.to_owned(),
            _ => "unknown".into(),
        }
    }
}

fn read<T: Clone>(source: &SourceSnapshot<T>) -> Result<T, String> {
    match source {
        SourceSnapshot::Available(value) | SourceSnapshot::Partial { value, .. } => {
            Ok(value.clone())
        }
        SourceSnapshot::Unavailable { reason } => Err(reason.clone()),
    }
}

fn address_matches(left: Option<&str>, right: Option<&str>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if left == right)
}

fn discovery_has(roster: &[DiscoveryRosterEntry], name: &str, address: Option<&str>) -> bool {
    roster.iter().any(|member| {
        member.name == name
            || address.is_some_and(|address| member.addresses.iter().any(|a| a == address))
    })
}

fn kubernetes_has(roster: &[KubernetesNodeRosterEntry], name: &str, address: Option<&str>) -> bool {
    roster.iter().any(|member| {
        member.name == name || address_matches(member.internal_address.as_deref(), address)
    })
}

/// `v1.34.3`, `1.13.2-rc1` or `v1.30.0+k3s1` as numbers.
fn parse_version(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.trim().trim_start_matches('v').split('.');
    let number = |part: Option<&str>| -> Option<u32> {
        let part = part?;
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };
    let major = number(parts.next())?;
    let minor = number(parts.next())?;
    let patch = number(parts.next()).unwrap_or(0);
    Some((major, minor, patch))
}

/// The Kubernetes minors a Talos minor supports, from the support matrix at
/// <https://www.talos.dev/latest/introduction/support-matrix/>. Versions not
/// listed are unknown rather than unsupported.
fn kubernetes_support(talos: &str) -> Option<(u32, u32)> {
    match parse_version(talos)? {
        (1, 12, _) => Some((30, 35)),
        (1, 11, _) => Some((29, 34)),
        (1, 10, _) => Some((28, 33)),
        (1, 9, _) => Some((27, 32)),
        (1, 8, _) => Some((26, 31)),
        (1, 7, _) => Some((25, 30)),
        (1, 6, _) => Some((24, 29)),
        _ => None,
    }
}

fn node_rows(view: &LifecycleView) -> Vec<NodeRow> {
    let snapshot = &view.snapshot;
    let discovery = snapshot
        .talos_discovery
        .value()
        .filter(|roster| !roster.is_empty());
    let kubernetes = snapshot
        .kubernetes_roster
        .value()
        .filter(|roster| !roster.is_empty());
    let kubelets = view.kubelets.value();
    let mut rows: Vec<NodeRow> = snapshot
        .nodes
        .iter()
        .map(|node: &NodeLifecycleSnapshot| {
            let address = node.address.as_deref();
            let in_kubernetes =
                kubernetes.map(|roster| kubernetes_has(roster, &node.name, address));
            let kubelet = match (&view.kubelets, kubelets) {
                (SourceSnapshot::Unavailable { reason }, _) => Err(reason.clone()),
                (_, Some(entries)) => entries
                    .iter()
                    .find(|entry| {
                        entry.name == node.name
                            || address_matches(entry.address.as_deref(), address)
                    })
                    .map(|entry| entry.version.clone())
                    .ok_or_else(|| "Kubernetes doesn't list this node".to_owned()),
                _ => Err("Kubernetes node versions weren't read".into()),
            };
            NodeRow {
                name: node.name.clone(),
                address: node.address.clone(),
                role: node.machine_type.clone(),
                talos: read(&node.version),
                platform: read(&node.platform),
                kubelet,
                config: read(&node.config_hash),
                time: read(&node.time_synchronization),
                drift: Drift::Unknown,
                in_discovery: discovery.map(|roster| discovery_has(roster, &node.name, address)),
                in_kubernetes,
                kubelet_behind: false,
            }
        })
        .collect();

    // Config drift: compare only hashes that were read.
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for hash in rows.iter().filter_map(|row| row.config.as_ref().ok()) {
        *counts.entry(hash.clone()).or_default() += 1;
    }
    let known: usize = counts.values().sum();
    let top = counts.values().copied().max().unwrap_or(0);
    let tied = counts.values().filter(|count| **count == top).count() > 1;
    for row in &mut rows {
        row.drift = match &row.config {
            Err(_) => Drift::Unknown,
            Ok(_) if known < 2 => Drift::OnlyReading,
            Ok(_) if counts.len() == 1 => Drift::InSync,
            Ok(hash) if !tied && counts.get(hash) == Some(&top) => Drift::InSync,
            Ok(_) => Drift::Differs,
        };
    }

    // Kubelet skew: behind the newest version seen.
    let newest = rows
        .iter()
        .filter_map(|row| row.kubelet.as_ref().ok())
        .filter_map(|version| parse_version(version))
        .max();
    if let Some(newest) = newest {
        for row in &mut rows {
            row.kubelet_behind = row
                .kubelet
                .as_ref()
                .ok()
                .and_then(|version| parse_version(version))
                .is_some_and(|version| version < newest);
        }
    }
    rows
}

#[derive(Clone, Debug)]
struct AlertRow {
    health: HealthIndicator,
    message: String,
    /// Where the alert comes from, shown in its details.
    origin: &'static str,
    /// What was compared: label and value pairs, labels being node names where
    /// the alert is about nodes.
    evidence: Vec<(String, String)>,
    nodes: Vec<String>,
}

fn names(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [first, second] => format!("{first} and {second}"),
        [first, second, rest @ ..] => {
            format!("{first}, {second} and {} more", rest.len())
        }
    }
}

const COLLECTOR: &str = "Reported by the lifecycle collector from what the nodes answered.";
const DERIVED: &str =
    "Derived here by comparing node data; nothing is flagged from data that wasn't read.";

fn alert_rows(view: &LifecycleView, rows: &[NodeRow]) -> Vec<AlertRow> {
    let snapshot = &view.snapshot;
    let mut alerts: Vec<AlertRow> = snapshot
        .alerts
        .iter()
        .map(|alert| core_alert(alert, view, rows))
        .collect();

    // Kubelet skew among the nodes whose kubelet version was read.
    let kubelets: Vec<(&NodeRow, (u32, u32, u32), &String)> = rows
        .iter()
        .filter_map(|row| {
            let version = row.kubelet.as_ref().ok()?;
            Some((row, parse_version(version)?, version))
        })
        .collect();
    if let Some((_, newest, newest_text)) = kubelets.iter().max_by_key(|(_, parsed, _)| *parsed) {
        let behind: Vec<&(&NodeRow, (u32, u32, u32), &String)> = kubelets
            .iter()
            .filter(|(_, parsed, _)| parsed < newest)
            .collect();
        if !behind.is_empty() {
            let minor = behind
                .iter()
                .any(|(_, parsed, _)| (parsed.0, parsed.1) != (newest.0, newest.1));
            let behind_names: Vec<&str> =
                behind.iter().map(|(row, _, _)| row.name.as_str()).collect();
            let message = if minor {
                format!(
                    "Kubelet minor version skew: {} behind {newest_text}",
                    names(&behind_names)
                )
            } else {
                format!(
                    "Kubelet patch version skew: {} behind {newest_text}",
                    names(&behind_names)
                )
            };
            alerts.push(AlertRow {
                health: HealthIndicator::Warning,
                message,
                origin: DERIVED,
                evidence: kubelets
                    .iter()
                    .map(|(row, _, version)| (row.name.clone(), (*version).clone()))
                    .collect(),
                nodes: behind.iter().map(|(row, _, _)| row.name.clone()).collect(),
            });
        }
    }

    // Kubelets outside what the cluster's Talos version supports.
    let talos = rows.iter().find_map(|row| row.talos.as_ref().ok());
    if let Some((talos, (low, high))) =
        talos.and_then(|talos| Some((talos, kubernetes_support(talos)?)))
    {
        let outside: Vec<&(&NodeRow, (u32, u32, u32), &String)> = kubelets
            .iter()
            .filter(|(_, parsed, _)| parsed.0 != 1 || parsed.1 < low || parsed.1 > high)
            .collect();
        if !outside.is_empty() {
            let outside_names: Vec<&str> = outside
                .iter()
                .map(|(row, _, _)| row.name.as_str())
                .collect();
            alerts.push(AlertRow {
                health: HealthIndicator::Warning,
                message: format!(
                    "Kubelet on {} is outside the Kubernetes range Talos {talos} supports (v1.{low} – v1.{high})",
                    names(&outside_names)
                ),
                origin: DERIVED,
                evidence: outside
                    .iter()
                    .map(|(row, _, version)| (row.name.clone(), (*version).clone()))
                    .collect(),
                nodes: outside.iter().map(|(row, _, _)| row.name.clone()).collect(),
            });
        }
    }

    // Roster consistency, only when both rosters were read and aren't empty.
    let discovery = snapshot
        .talos_discovery
        .value()
        .filter(|roster| !roster.is_empty());
    let kubernetes = snapshot
        .kubernetes_roster
        .value()
        .filter(|roster| !roster.is_empty());
    if let (Some(discovery), Some(kubernetes)) = (discovery, kubernetes) {
        let label = |member: &DiscoveryRosterEntry| {
            if member.name.is_empty() {
                member.addresses.first().cloned().unwrap_or_default()
            } else {
                member.name.clone()
            }
        };
        let only_discovery: Vec<String> = discovery
            .iter()
            .filter(|member| {
                !kubernetes_has(
                    kubernetes,
                    &member.name,
                    member.addresses.first().map(String::as_str),
                ) && !member
                    .addresses
                    .iter()
                    .any(|address| kubernetes_has(kubernetes, "", Some(address)))
            })
            .map(label)
            .collect();
        if !only_discovery.is_empty() {
            let shown: Vec<&str> = only_discovery.iter().map(String::as_str).collect();
            alerts.push(AlertRow {
                health: HealthIndicator::Warning,
                message: format!(
                    "In Talos discovery but not registered in Kubernetes: {}",
                    names(&shown)
                ),
                origin: DERIVED,
                evidence: only_discovery
                    .iter()
                    .map(|name| (name.clone(), "Talos discovery only".into()))
                    .collect(),
                nodes: only_discovery,
            });
        }
        let only_kubernetes: Vec<String> = kubernetes
            .iter()
            .filter(|member| {
                !discovery_has(discovery, &member.name, member.internal_address.as_deref())
            })
            .map(|member| member.name.clone())
            .collect();
        if !only_kubernetes.is_empty() {
            let shown: Vec<&str> = only_kubernetes.iter().map(String::as_str).collect();
            alerts.push(AlertRow {
                health: HealthIndicator::Warning,
                message: format!(
                    "In Kubernetes but not in Talos discovery: {}",
                    names(&shown)
                ),
                origin: DERIVED,
                evidence: only_kubernetes
                    .iter()
                    .map(|name| (name.clone(), "Kubernetes only".into()))
                    .collect(),
                nodes: only_kubernetes,
            });
        }
    }
    alerts
}

/// A collector alert, with the evidence behind it where the rows hold it.
fn core_alert(alert: &LifecycleAlert, view: &LifecycleView, rows: &[NodeRow]) -> AlertRow {
    let mut evidence = Vec::new();
    match alert.kind() {
        LifecycleAlertKind::VersionMismatch => {
            evidence = rows
                .iter()
                .filter_map(|row| Some((row.name.clone(), row.talos.clone().ok()?)))
                .collect();
        }
        LifecycleAlertKind::ConfigDrift => {
            evidence = rows
                .iter()
                .filter_map(|row| Some((row.name.clone(), row.config.clone().ok()?)))
                .collect();
        }
        LifecycleAlertKind::TimeUnsynchronized => {
            evidence = rows
                .iter()
                .filter_map(|row| match &row.time {
                    Ok(time) if !time.synced => Some((
                        row.name.clone(),
                        format!("offset {:.3} s from {}", time.offset_seconds, time.server),
                    )),
                    _ => None,
                })
                .collect();
        }
        LifecycleAlertKind::EtcdUnsafe => {
            evidence.push((
                "etcd".into(),
                etcd_verdict(&view.snapshot.etcd_pre_operation).detail,
            ));
        }
        LifecycleAlertKind::EtcdUnavailable => {
            if let SourceSnapshot::Unavailable { reason } = &view.snapshot.etcd_pre_operation {
                evidence.push(("etcd".into(), reason.clone()));
            }
        }
        LifecycleAlertKind::RosterUnavailable => {
            for (label, reason) in [
                (
                    "Talos discovery",
                    unavailable_reason(&view.snapshot.talos_discovery),
                ),
                (
                    "Kubernetes roster",
                    unavailable_reason(&view.snapshot.kubernetes_roster),
                ),
            ] {
                if let Some(reason) = reason {
                    evidence.push((label.into(), reason));
                }
            }
        }
        LifecycleAlertKind::Other => {}
    }
    let nodes = evidence
        .iter()
        .filter(|(label, _)| rows.iter().any(|row| &row.name == label))
        .map(|(label, _)| label.clone())
        .collect();
    AlertRow {
        health: alert.health,
        message: alert.message.clone(),
        origin: COLLECTOR,
        evidence,
        nodes,
    }
}

fn unavailable_reason<T>(source: &SourceSnapshot<T>) -> Option<String> {
    match source {
        SourceSnapshot::Unavailable { reason } => Some(reason.clone()),
        _ => None,
    }
}

struct EtcdVerdict {
    tone: Tone,
    icon: IconName,
    label: &'static str,
    detail: String,
}

/// Whether a control plane can be taken down. Unknown data is never a green
/// light, but it isn't "unsafe" either: it is not reported.
fn etcd_verdict(etcd: &SourceSnapshot<EtcdPreOperationAudit>) -> EtcdVerdict {
    let Some(audit) = etcd.value() else {
        return EtcdVerdict {
            tone: Tone::Unknown,
            icon: IconName::CircleDashed,
            label: "Not reported",
            detail: format!(
                "etcd status wasn't read, so there is no verdict yet. {}",
                unavailable_reason(etcd).unwrap_or_default()
            )
            .trim()
            .to_owned(),
        };
    };
    let detail = format!(
        "{}/{} members responding · quorum needs {} · can lose {}",
        audit.responding_members, audit.total_members, audit.quorum_required, audit.can_lose
    );
    if audit.safe_for_single_member_operation() {
        EtcdVerdict {
            tone: Tone::Good,
            icon: IconName::CircleCheck,
            label: "A control plane can be taken down",
            detail,
        }
    } else if matches!(audit.quorum, QuorumState::NoQuorum { .. }) {
        // Counts members that answered us, not etcd's own view: too few
        // answers means quorum can't be confirmed, not that it's lost.
        EtcdVerdict {
            tone: Tone::Warn,
            icon: IconName::CircleAlert,
            label: "Quorum unconfirmed",
            detail: format!(
                "{detail}. Members that didn't answer are not reported, not failed; nothing that takes a member down is safe until they answer."
            ),
        }
    } else if matches!(audit.quorum, QuorumState::Unknown) {
        EtcdVerdict {
            tone: Tone::Unknown,
            icon: IconName::CircleDashed,
            label: "Not reported",
            detail,
        }
    } else {
        EtcdVerdict {
            tone: Tone::Warn,
            icon: IconName::CircleAlert,
            label: "Not safe to take a control plane down",
            detail,
        }
    }
}

/// Sources that didn't answer fully, for the partial notice.
fn unavailable_sources(view: &LifecycleView, rows: &[NodeRow]) -> Vec<String> {
    fn note<T>(out: &mut Vec<String>, label: &str, source: &SourceSnapshot<T>) {
        match source {
            SourceSnapshot::Available(_) => {}
            SourceSnapshot::Partial { warnings, .. } => {
                out.push(format!("{label}: {}", warnings.join("; ")))
            }
            SourceSnapshot::Unavailable { reason } => out.push(format!("{label}: {reason}")),
        }
    }
    let snapshot = &view.snapshot;
    let mut out = Vec::new();
    note(&mut out, "Cluster identity", &snapshot.identity);
    note(&mut out, "Talos discovery", &snapshot.talos_discovery);
    note(&mut out, "Kubernetes roster", &snapshot.kubernetes_roster);
    note(&mut out, "Kubelet versions", &view.kubelets);
    note(&mut out, "etcd pre-operation", &snapshot.etcd_pre_operation);
    let silent: Vec<&str> = rows
        .iter()
        .filter(|row| !row.reported())
        .map(|row| row.name.as_str())
        .collect();
    if !silent.is_empty() {
        out.push(format!("{} didn't report to the Talos API", names(&silent)));
    }
    out
}

fn health_tone(health: &HealthIndicator) -> (Tone, IconName, &'static str) {
    match health {
        HealthIndicator::Healthy => (Tone::Good, IconName::CircleCheck, "OK"),
        HealthIndicator::Warning => (Tone::Warn, IconName::CircleAlert, "Warning"),
        HealthIndicator::Error => (Tone::Crit, IconName::CircleX, "Error"),
        HealthIndicator::Info => (Tone::Accent, IconName::Info, "Info"),
        _ => (Tone::Unknown, IconName::CircleDashed, "Unknown"),
    }
}

fn presence(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "✓",
        Some(false) => "✗",
        None => "?",
    }
}

fn presence_text(value: Option<bool>, roster: &str) -> String {
    match value {
        Some(true) => format!("listed in {roster}"),
        Some(false) => format!("not listed in {roster}"),
        None => format!("unknown ({roster} wasn't read)"),
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

// ---------------------------------------------------------------------------
// Screen
// ---------------------------------------------------------------------------

impl LifecycleScreen {
    fn rows_and_alerts(&self) -> (Vec<NodeRow>, Vec<AlertRow>) {
        let Some(view) = self.loader.data() else {
            return (Vec::new(), Vec::new());
        };
        let rows = node_rows(view);
        let alerts = alert_rows(view, &rows);
        (rows, alerts)
    }

    /// Nodes first, then alerts: the order they appear in.
    fn items(&self) -> Vec<Item> {
        let (rows, alerts) = self.rows_and_alerts();
        rows.into_iter()
            .map(|row| Item::Node(row.name))
            .chain((0..alerts.len()).map(Item::Alert))
            .collect()
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let items = self.items();
        if items.is_empty() {
            return;
        }
        let current = self
            .selected
            .as_ref()
            .and_then(|selected| items.iter().position(|item| item == selected));
        let next = match current {
            Some(ix) => ix.saturating_add_signed(delta).min(items.len() - 1),
            None if delta < 0 => items.len() - 1,
            None => 0,
        };
        self.selected = Some(items[next].clone());
        cx.notify();
    }

    fn select(&mut self, item: Item, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(item);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn can_target(&self, node: &str) -> bool {
        self.source.as_ref().is_some_and(|source| {
            source.target.node != node && source.nodes.iter().any(|known| known.name == node)
        })
    }

    fn target_button(&self, node: &str, cx: &mut Context<Self>) -> Button {
        let name = node.to_owned();
        Button::new(SharedString::from(format!("target-node-{node}")))
            .outline()
            .xsmall()
            .icon(IconName::Crosshair)
            .label(format!("Target {node}"))
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(ScreenEvent::SelectNode(name.clone()));
            }))
    }

    fn summary(
        &self,
        view: &LifecycleView,
        rows: &[NodeRow],
        alerts: &[AlertRow],
        cx: &App,
    ) -> Stateful<Div> {
        let distinct = |values: Vec<&String>| -> String {
            let mut unique: Vec<&String> = values;
            unique.sort();
            unique.dedup();
            if unique.is_empty() {
                "not reported".into()
            } else {
                unique
                    .into_iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        };
        let talos = distinct(
            rows.iter()
                .filter_map(|row| row.talos.as_ref().ok())
                .collect(),
        );
        let kubelet = distinct(
            rows.iter()
                .filter_map(|row| row.kubelet.as_ref().ok())
                .collect(),
        );
        let etcd = etcd_verdict(&view.snapshot.etcd_pre_operation);
        let etcd_short = match etcd.tone {
            Tone::Good => "Safe",
            Tone::Warn => "Not safe",
            _ => "Not reported",
        };
        let warnings = alerts
            .iter()
            .filter(|alert| {
                matches!(
                    alert.health,
                    HealthIndicator::Warning | HealthIndicator::Error
                )
            })
            .count();
        h_flex()
            .id("lifecycle-summary")
            .gap_2p5()
            .flex_wrap()
            .child(stat("Talos", talos, cx))
            .child(stat("Kubelet", kubelet, cx))
            .child(stat("etcd pre-check", etcd_short, cx))
            .child(stat(
                "Alerts",
                if alerts.is_empty() {
                    "none".to_owned()
                } else if warnings == alerts.len() {
                    alerts.len().to_string()
                } else {
                    format!("{} ({warnings} to review)", alerts.len())
                },
                cx,
            ))
    }

    fn render_row(
        &self,
        ix: usize,
        row: &NodeRow,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let selected = self.selected == Some(Item::Node(row.name.clone()));
        let name = row.name.clone();
        let value = |column: Column, text: String, kind: u8| {
            // kind: 0 plain, 1 not reported, 2 warning.
            cell(column)
                .text_right()
                .when(!selected && kind == 1, |this| this.text_color(p.unk_ink))
                .when(!selected && kind == 2, |this| this.text_color(p.warn_ink))
                .child(text)
        };
        let not_reported = || "not reported".to_owned();
        let talos = row.talos.clone().ok();
        let kubelet = row.kubelet.clone().ok();
        let config = match (&row.config, row.drift) {
            (Ok(hash), Drift::Differs) => (format!("{hash} · differs"), 2),
            (Ok(hash), _) => (hash.clone(), 0),
            (Err(_), _) => ("—".to_owned(), 1),
        };
        let role = row.role_label();
        h_flex()
            .id(("lifecycle-node", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(format!(
                "{} · {} · Talos {} · kubelet {} · config {} · discovery {} · Kubernetes {}",
                row.name,
                role,
                talos.clone().unwrap_or_else(not_reported),
                kubelet.clone().unwrap_or_else(not_reported),
                match (&row.config, row.drift) {
                    (Ok(hash), Drift::Differs) => format!("{hash}, differs from other nodes"),
                    (Ok(hash), _) => hash.clone(),
                    (Err(_), _) => not_reported(),
                },
                presence_text(row.in_discovery, "discovery"),
                presence_text(row.in_kubernetes, "Kubernetes"),
            ))
            .w_full()
            .h(px(ROW_HEIGHT))
            .font_family(MONO_FONT)
            .text_size(px(12.))
            .cursor_pointer()
            .when(selected, |this| this.bg(p.accent_soft).text_color(p.accent))
            .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
            .child(cell(COLUMNS[0]).child(row.name.clone()))
            .child(cell(COLUMNS[1]).child(role))
            .child(value(
                COLUMNS[2],
                talos.clone().unwrap_or_else(not_reported),
                if talos.is_some() { 0 } else { 1 },
            ))
            .child(value(
                COLUMNS[3],
                kubelet.clone().unwrap_or_else(not_reported),
                match (&kubelet, row.kubelet_behind) {
                    (None, _) => 1,
                    (Some(_), true) => 2,
                    _ => 0,
                },
            ))
            .child(value(COLUMNS[4], config.0, config.1))
            .child(cell(COLUMNS[5]).text_right().child(format!(
                "{} · {}",
                presence(row.in_discovery),
                presence(row.in_kubernetes)
            )))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.select(Item::Node(name.clone()), window, cx);
            }))
    }

    fn nodes_panel(&self, rows: &[NodeRow], cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let list = if rows.is_empty() {
            div()
                .px_3()
                .py_3p5()
                .text_size(px(12.5))
                .text_color(p.muted)
                .child(
                    "No node roster is available, so versions, time and configuration are unknown.",
                )
                .into_any_element()
        } else {
            v_flex()
                .children(
                    rows.iter()
                        .enumerate()
                        .map(|(ix, row)| self.render_row(ix, row, cx)),
                )
                .into_any_element()
        };
        panel(cx).overflow_hidden().child(
            div()
                .id("lifecycle-node-scroll")
                .test_support()
                .overflow_x_scroll()
                .child(
                // Fill the panel; without `w_full` the table takes its
                // max-content width and a long node name pushes columns out.
                v_flex()
                    .w_full()
                    .min_w(crate::ui::dp(table_width(&COLUMNS)))
                    .child(table_head(&COLUMNS, cx))
                    .child(
                        div()
                            .id("lifecycle-nodes")
                            .test_support()
                            .role(Role::ListBox)
                            .aria_label(
                                "Nodes; arrows select a node or alert, Escape clears the selection",
                            )
                            .child(list),
                    ),
            ),
        )
    }

    fn alerts_panel(&self, alerts: &[AlertRow], cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let body = if alerts.is_empty() {
            div()
                .id("lifecycle-no-alerts")
                .px_3()
                .py_3()
                .text_size(px(12.5))
                .text_color(p.muted)
                .child("No lifecycle alerts. Sources that didn't answer stay unknown, they aren't alerts.")
                .into_any_element()
        } else {
            v_flex()
                .children(alerts.iter().enumerate().map(|(ix, alert)| {
                    let (tone, icon, label) = health_tone(&alert.health);
                    let selected = self.selected == Some(Item::Alert(ix));
                    h_flex()
                        .id(("lifecycle-alert", ix))
                        .test_support()
                        .role(Role::ListBoxOption)
                        .aria_selected(selected)
                        .aria_label(format!("{label}: {}", alert.message))
                        .items_start()
                        .gap_2p5()
                        .px_3()
                        .py_2()
                        .text_size(px(12.5))
                        .cursor_pointer()
                        .when(selected, |this| this.bg(p.accent_soft))
                        .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
                        .child(ui::tag(tone, Some(icon), label, cx))
                        .child(div().flex_1().min_w_0().child(alert.message.clone()))
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.select(Item::Alert(ix), window, cx);
                        }))
                }))
                .into_any_element()
        };
        panel(cx)
            .child(
                div()
                    .px_3()
                    .py(px(9.))
                    .border_b_1()
                    .border_color(p.line)
                    .child(ui::caption("Alerts", cx)),
            )
            .child(
                div()
                    .id("lifecycle-alerts")
                    .test_support()
                    .role(Role::ListBox)
                    .aria_label("Lifecycle alerts")
                    .child(body),
            )
    }

    fn etcd_panel(&self, view: &LifecycleView, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let verdict = etcd_verdict(&view.snapshot.etcd_pre_operation);
        let warnings = match &view.snapshot.etcd_pre_operation {
            SourceSnapshot::Partial { warnings, .. } => warnings.join("; "),
            _ => String::new(),
        };
        panel(cx)
            .id("lifecycle-etcd")
            .test_support()
            .aria_label(format!(
                "etcd pre-operation check: {}. {}",
                verdict.label, verdict.detail
            ))
            .p_4()
            .gap_2p5()
            .child(ui::caption("etcd pre-operation check", cx))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(ui::tag(verdict.tone, Some(verdict.icon), verdict.label, cx)),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .child(verdict.detail.clone()),
            )
            .when(!warnings.is_empty(), |this| {
                this.child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child(warnings.clone()),
                )
            })
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child("A snapshot, not permission to act. Repeat the check right before any disruptive operation."),
            ).into_any_element()
    }

    fn sources_panel(&self, view: &LifecycleView, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let snapshot = &view.snapshot;
        let status = |text: String, known: bool| {
            div()
                .text_size(px(12.5))
                .when(!known, |this| this.text_color(p.unk_ink))
                .child(text)
        };
        fn roster_status<T>(
            source: &SourceSnapshot<Vec<T>>,
            noun_one: &str,
            noun_many: &str,
        ) -> (String, bool) {
            match source {
                SourceSnapshot::Available(items) => {
                    (plural(items.len(), noun_one, noun_many), true)
                }
                SourceSnapshot::Partial { value, warnings } => (
                    format!(
                        "{} · partial: {}",
                        plural(value.len(), noun_one, noun_many),
                        warnings.join("; ")
                    ),
                    !value.is_empty(),
                ),
                SourceSnapshot::Unavailable { reason } => {
                    (format!("not reported · {reason}"), false)
                }
            }
        }
        let identity = match &snapshot.identity {
            SourceSnapshot::Available(identity)
            | SourceSnapshot::Partial {
                value: identity, ..
            } => Some(identity),
            SourceSnapshot::Unavailable { .. } => None,
        };
        let (discovery, discovery_known) =
            roster_status(&snapshot.talos_discovery, "member", "members");
        let (kubernetes, kubernetes_known) =
            roster_status(&snapshot.kubernetes_roster, "node", "nodes");
        let (kubelets, kubelets_known) =
            roster_status(&view.kubelets, "kubelet version", "kubelet versions");
        let talos = node_rows(view).into_iter().find_map(|row| row.talos.ok());
        let support = match talos.as_deref().and_then(kubernetes_support) {
            Some((low, high)) => format!("v1.{low} – v1.{high}"),
            None => "not in the support table".to_owned(),
        };
        panel(cx)
            .id("lifecycle-sources")
            .p_4()
            .gap_2p5()
            .child(ui::caption("Cluster identity and sources", cx))
            .child(field(
                "Context",
                match identity {
                    Some(ClusterIdentity { context_name, .. }) => {
                        mono(context_name.clone()).into_any_element()
                    }
                    None => status(
                        format!(
                            "not reported · {}",
                            unavailable_reason(&snapshot.identity).unwrap_or_default()
                        ),
                        false,
                    )
                    .into_any_element(),
                },
                cx,
            ))
            .when_some(identity, |this, identity| {
                this.child(field(
                    "Endpoints",
                    mono(if identity.endpoint_addresses.is_empty() {
                        "none configured".to_owned()
                    } else {
                        identity.endpoint_addresses.join(", ")
                    }),
                    cx,
                ))
                .child(field(
                    "Target nodes",
                    mono(if identity.target_addresses.is_empty() {
                        "none configured".to_owned()
                    } else {
                        identity.target_addresses.join(", ")
                    }),
                    cx,
                ))
            })
            .child(field(
                "Talos discovery",
                status(discovery, discovery_known),
                cx,
            ))
            .child(field(
                "Kubernetes roster",
                status(kubernetes, kubernetes_known),
                cx,
            ))
            .child(field(
                "Kubelet versions",
                status(kubelets, kubelets_known),
                cx,
            ))
            .child(field(
                "Kubernetes support",
                status(
                    match &talos {
                        Some(talos) => format!("Talos {talos} supports {support}"),
                        None => "Talos version not reported".into(),
                    },
                    talos.is_some(),
                ),
                cx,
            ))
            .into_any_element()
    }

    fn details(&self, view: &LifecycleView, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let (rows, alerts) = self.rows_and_alerts();
        let empty = |text: &'static str| {
            panel(cx)
                .id("lifecycle-details")
                .test_support()
                .aria_label(text)
                .p_4()
                .text_color(p.muted)
                .text_size(px(12.5))
                .child(text)
                .into_any_element()
        };
        match &self.selected {
            None => empty("Select a node or an alert to see its details."),
            Some(Item::Node(name)) => {
                let Some(row) = rows.iter().find(|row| &row.name == name) else {
                    return empty("The selected node is no longer in the roster.");
                };
                self.node_details(row, view, cx)
            }
            Some(Item::Alert(ix)) => {
                let Some(alert) = alerts.get(*ix) else {
                    return empty("The selected alert is no longer raised.");
                };
                self.alert_details(alert, cx)
            }
        }
    }

    fn node_details(
        &self,
        row: &NodeRow,
        view: &LifecycleView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let unknown = |reason: &str| {
            v_flex()
                .gap_0p5()
                .child(div().text_color(p.unk_ink).child("Not reported"))
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.muted)
                        .child(reason.to_owned()),
                )
        };
        let text = |result: &Result<String, String>| match result {
            Ok(value) => mono(value.clone()).into_any_element(),
            Err(reason) => unknown(reason).into_any_element(),
        };
        let reported = row.reported();
        let config = match (&row.config, row.drift) {
            (Ok(hash), drift) => h_flex()
                .gap_2()
                .flex_wrap()
                .child(mono(hash.clone()))
                .child(match drift {
                    Drift::Differs => ui::tag(
                        Tone::Warn,
                        Some(IconName::GitCompareArrows),
                        "Differs from other nodes",
                        cx,
                    ),
                    Drift::InSync => ui::tag(Tone::Good, None, "Matches other nodes", cx),
                    _ => ui::tag(Tone::Outline, None, "Only reading", cx),
                })
                .into_any_element(),
            (Err(reason), _) => unknown(reason).into_any_element(),
        };
        let time = match &row.time {
            Ok(time) => h_flex()
                .gap_2()
                .flex_wrap()
                .child(if time.synced {
                    ui::tag(Tone::Good, None, "Synchronized", cx)
                } else {
                    ui::tag(Tone::Warn, None, "Not synchronized", cx)
                })
                .child(mono(format!(
                    "offset {:.3} s · {}",
                    time.offset_seconds, time.server
                )))
                .into_any_element(),
            Err(reason) => unknown(reason).into_any_element(),
        };
        let kubelet = match &row.kubelet {
            Ok(version) => h_flex()
                .gap_2()
                .flex_wrap()
                .child(mono(version.clone()))
                .when(row.kubelet_behind, |this| {
                    this.child(ui::tag(
                        Tone::Warn,
                        Some(IconName::CircleAlert),
                        "Behind the newest kubelet",
                        cx,
                    ))
                })
                .into_any_element(),
            Err(reason) => unknown(reason).into_any_element(),
        };
        let discovery_reason = unavailable_reason(&view.snapshot.talos_discovery);
        let kubernetes_reason = unavailable_reason(&view.snapshot.kubernetes_roster);
        let roster = |value: Option<bool>, reason: Option<String>, label: &str| match value {
            Some(true) => mono("listed").into_any_element(),
            Some(false) => div()
                .child(format!("Not listed in {label}"))
                .into_any_element(),
            None => unknown(&reason.unwrap_or_else(|| format!("{label} wasn't read or is empty")))
                .into_any_element(),
        };
        let target = self
            .can_target(&row.name)
            .then(|| self.target_button(&row.name, cx));
        panel(cx)
            .id("lifecycle-details")
            .test_support()
            .aria_label(format!("Details of node {}", row.name))
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(row.name.clone()),
                    )
                    .child(if reported {
                        ui::tag(Tone::Outline, None, row.role_label(), cx)
                    } else {
                        ui::tag(
                            Tone::Unknown,
                            Some(IconName::CircleDashed),
                            "Not reported",
                            cx,
                        )
                    })
                    .child(div().flex_1())
                    .children(target),
            )
            .when(!reported, |this| {
                this.child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child("No Talos answer was received from this node. That doesn't mean it is down; nothing positively reported a failure."),
                )
            })
            .child(field(
                "Address",
                mono(row.address.clone().unwrap_or_else(|| "not known".into())),
                cx,
            ))
            .child(field("Role", div().child(row.role_label()), cx))
            .child(field("Talos version", text(&row.talos), cx))
            .child(field("Platform", text(&row.platform), cx))
            .child(field("Kubelet version", kubelet, cx))
            .child(field("Machine config", config, cx))
            .child(field("Time sync", time, cx))
            .child(field(
                "Talos discovery",
                roster(row.in_discovery, discovery_reason, "Talos discovery"),
                cx,
            ))
            .child(field(
                "Kubernetes",
                roster(row.in_kubernetes, kubernetes_reason, "Kubernetes"),
                cx,
            ))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(p.muted)
                    .child("Config is the machineconfig resource version. A difference is a drift indicator, not a diff; nodes may legitimately differ."),
            ).into_any_element()
    }

    fn alert_details(&self, alert: &AlertRow, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let (tone, icon, label) = health_tone(&alert.health);
        let buttons: Vec<Button> = alert
            .nodes
            .iter()
            .filter(|node| self.can_target(node))
            .map(|node| self.target_button(node, cx))
            .collect();
        panel(cx)
            .id("lifecycle-details")
            .test_support()
            .aria_label(format!("Details of alert: {}", alert.message))
            .p_4()
            .gap_2p5()
            .child(h_flex().gap_2().child(ui::tag(tone, Some(icon), label, cx)))
            .child(
                div()
                    .text_size(px(13.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(alert.message.clone()),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(alert.origin),
            )
            .when(!alert.evidence.is_empty(), |this| {
                this.child(ui::caption("Evidence", cx))
                    .children(alert.evidence.iter().map(|(label, value)| {
                        h_flex()
                            .gap_3()
                            .items_start()
                            .child(
                                div()
                                    .w(px(150.))
                                    .flex_none()
                                    .min_w_0()
                                    .font_family(MONO_FONT)
                                    .text_size(px(12.))
                                    .truncate()
                                    .child(label.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(12.))
                                    .child(value.clone()),
                            )
                    }))
            })
            .when(!buttons.is_empty(), |this| {
                this.child(h_flex().gap_2().flex_wrap().children(buttons))
            })
            .into_any_element()
    }
}

impl Render for LifecycleScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(page) = gated_page(
            "lifecycle-page",
            "Lifecycle",
            Scope::Cluster,
            self.source.as_ref(),
            &self.loader,
            "lifecycle status",
            cx,
        ) {
            return page;
        }
        let (Some(source), Some(view)) = (self.source.clone(), self.loader.data()) else {
            return div().into_any_element();
        };
        let rows = node_rows(view);
        let alerts = alert_rows(view, &rows);
        let missing = unavailable_sources(view, &rows);
        let summary = self.summary(view, &rows, &alerts, cx);
        let nodes = self.nodes_panel(&rows, cx);
        let alerts_panel = self.alerts_panel(&alerts, cx);
        let etcd = self.etcd_panel(view, cx);
        let sources = self.sources_panel(view, cx);
        let details = self.details(view, cx);
        // Details sit beside the lists only when the roster still fits whole;
        // otherwise they'd push its last columns behind a horizontal scroll.
        let wide = content_width(window) >= table_width(&COLUMNS) + DETAILS_WIDTH + GAP;
        let body = if wide {
            h_flex()
                .items_start()
                .gap(px(14.))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(14.))
                        .child(nodes)
                        .child(alerts_panel)
                        .child(etcd)
                        .child(sources),
                )
                .child(div().w(px(DETAILS_WIDTH)).flex_none().child(details))
                .into_any_element()
        } else {
            v_flex()
                .gap(px(14.))
                .child(nodes)
                .child(details)
                .child(alerts_panel)
                .child(etcd)
                .child(sources)
                .into_any_element()
        };
        v_flex()
            .id("lifecycle-page")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(crate::desktop::PAGE_PADDING))
            .pt(px(22.))
            .pb(px(18.))
            .gap(px(14.))
            .child(header(
                "Lifecycle",
                &source,
                Scope::Cluster,
                &self.loader,
                cx,
            ))
            .children(failure_banner(&self.loader, cx))
            .children(partial_notice(missing, cx))
            .child(summary)
            .child(
                div()
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|view, _: &NextItem, _, cx| view.step(1, cx)))
                    .on_action(cx.listener(|view, _: &PreviousItem, _, cx| view.step(-1, cx)))
                    .on_action(cx.listener(|view, _: &FirstItem, _, cx| view.step(isize::MIN, cx)))
                    .on_action(cx.listener(|view, _: &LastItem, _, cx| view.step(isize::MAX, cx)))
                    .on_action(cx.listener(|view, _: &ClearSelection, _, cx| {
                        view.selected = None;
                        cx.notify();
                    }))
                    .child(body),
            )
            .into_any_element()
    }
}

// ---------------------------------------------------------------------------
// Example data
// ---------------------------------------------------------------------------

/// Example data for `--fixture`: six nodes on Talos v1.13.2, one worker on an
/// older Kubernetes patch, one worker that doesn't answer, a healthy etcd.
fn example(source: &ScreenSource) -> Result<LifecycleView, String> {
    let Some(node) = source.node() else {
        return Err("Example data has no such node".into());
    };
    if !node.responding {
        return Err(format!(
            "{} didn't answer the Talos API within 10 s (example)",
            node.name
        ));
    }
    let silent_reason = "Talos read request timed out (example)";
    let nodes: Vec<NodeLifecycleSnapshot> = source
        .nodes
        .iter()
        .map(|node| {
            let unavailable = |what: &str| SourceSnapshot::Unavailable {
                reason: format!("{what} is unavailable: {silent_reason}"),
            };
            if node.responding {
                NodeLifecycleSnapshot {
                    name: node.name.clone(),
                    address: Some(node.address.clone()),
                    machine_type: Some(
                        if node.role == crate::presentation::Role::ControlPlane {
                            "controlplane"
                        } else {
                            "worker"
                        }
                        .into(),
                    ),
                    version: SourceSnapshot::Available(
                        node.version.clone().unwrap_or_else(|| "v1.13.2".into()),
                    ),
                    platform: SourceSnapshot::Available("metal".into()),
                    time_synchronization: SourceSnapshot::Available(TimeSynchronizationAudit {
                        server: "time.cloudflare.com".into(),
                        offset_seconds: 0.0021,
                        synced: true,
                    }),
                    config_hash: SourceSnapshot::Available("42".into()),
                }
            } else {
                NodeLifecycleSnapshot {
                    name: node.name.clone(),
                    address: Some(node.address.clone()),
                    machine_type: Some(
                        if node.role == crate::presentation::Role::ControlPlane {
                            "controlplane"
                        } else {
                            "worker"
                        }
                        .into(),
                    ),
                    version: unavailable("Talos version"),
                    platform: unavailable("Talos platform"),
                    time_synchronization: SourceSnapshot::Unavailable {
                        reason: silent_reason.into(),
                    },
                    config_hash: SourceSnapshot::Unavailable {
                        reason: "Machineconfig request timed out (example)".into(),
                    },
                }
            }
        })
        .collect();
    let discovery = source
        .nodes
        .iter()
        .map(|node| DiscoveryRosterEntry {
            id: node.name.clone(),
            name: node.name.clone(),
            addresses: vec![node.address.clone()],
            machine_type: if node.role == crate::presentation::Role::ControlPlane {
                "controlplane"
            } else {
                "worker"
            }
            .into(),
        })
        .collect();
    let kubernetes = source
        .nodes
        .iter()
        .map(|node| KubernetesNodeRosterEntry {
            name: node.name.clone(),
            internal_address: Some(node.address.clone()),
            is_control_plane: node.role == crate::presentation::Role::ControlPlane,
        })
        .collect();
    let kubelets = source
        .nodes
        .iter()
        .map(|node| KubeletEntry {
            name: node.name.clone(),
            address: Some(node.address.clone()),
            version: if node.name.contains("wk-fra1-02") {
                "v1.34.1".into()
            } else {
                "v1.34.3".into()
            },
        })
        .collect();
    let total = 3.min(
        source
            .nodes
            .iter()
            .filter(|node| node.role == crate::presentation::Role::ControlPlane)
            .count()
            .max(1),
    );
    Ok(LifecycleView {
        snapshot: LifecycleSnapshot {
            identity: SourceSnapshot::Available(ClusterIdentity {
                context_name: source.target.context.clone(),
                endpoint_addresses: vec![source.target.address.clone()],
                target_addresses: vec![source.target.address.clone()],
            }),
            talos_discovery: SourceSnapshot::Available(discovery),
            kubernetes_roster: SourceSnapshot::Available(kubernetes),
            nodes,
            etcd_pre_operation: SourceSnapshot::Available(EtcdPreOperationAudit {
                total_members: total,
                responding_members: total,
                quorum_required: total / 2 + 1,
                can_lose: total.saturating_sub(total / 2 + 1),
                quorum: QuorumState::Healthy,
            }),
            alerts: Vec::new(),
        },
        kubelets: SourceSnapshot::Available(kubelets),
    })
}

#[cfg(test)]
mod ui_tests {
    use std::sync::Arc;

    use freshkube_core::security_lifecycle::SourceSnapshot;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
    use tokio::runtime::{Builder, Runtime};

    // Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
    use super::{
        Drift, Item, LifecycleScreen, LifecycleView, ScreenPanel, ScreenSource, alert_rows,
        etcd_verdict, example, node_rows, parse_version,
    };
    use crate::backend::Target;
    use crate::ui::Tone;
    use crate::{fixture, presentation};
    use freshkube_core::indicators::QuorumState;
    use freshkube_core::security_lifecycle::EtcdPreOperationAudit;

    fn source(node: &str) -> ScreenSource {
        let nodes = presentation::node_summaries(&fixture::cluster("prod-fra", 1));
        let summary = nodes.iter().find(|summary| summary.name == node).unwrap();
        ScreenSource {
            target: Target {
                epoch: 1,
                context: "prod-fra".into(),
                node: summary.name.clone(),
                address: summary.address.clone(),
            },
            nodes: Arc::new(nodes),
            live: None,
        }
    }

    fn mount(
        cx: &mut TestAppContext,
        node: &str,
    ) -> (Runtime, Entity<LifecycleScreen>, WindowHandle<Root>) {
        mount_sized(cx, node, 1100.)
    }

    fn mount_sized(
        cx: &mut TestAppContext,
        node: &str,
        width: f32,
    ) -> (Runtime, Entity<LifecycleScreen>, WindowHandle<Root>) {
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
        });
        let source = source(node);
        let mut screen = None;
        let handle = cx.open_window(size(px(width), px(760.)), |window, cx| {
            let view = cx.new(|cx| {
                let mut view = LifecycleScreen::new(runtime.handle().clone(), window, cx);
                view.set_source(Some(source), window, cx);
                view.activate(window, cx);
                view
            });
            screen = Some(view.clone());
            Root::new(view, window, cx)
        });
        cx.run_until_parked();
        (runtime, screen.unwrap(), handle)
    }

    fn example_view() -> LifecycleView {
        example(&source("talos-cp-fra1-01")).unwrap()
    }

    #[gpui_kit::test]
    fn keyboard_selection_moves_through_nodes_and_updates_details(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(screen.read(cx).loader.data().is_some());
            window.find("lifecycle-details");
            window.click(("lifecycle-node", 0usize), cx);
            window.press("down", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find(("lifecycle-node", 1usize)).selected(),
                Some(true)
            );
            assert_eq!(
                window.find(("lifecycle-node", 0usize)).selected(),
                Some(false)
            );
            let details = window.find("lifecycle-details").label().unwrap().to_owned();
            assert!(details.contains("talos-cp-fra1-02"), "{details}");
            // The alert follows the last node; End reaches it.
            window.press("end", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find(("lifecycle-alert", 0usize)).selected(),
                Some(true)
            );
            let details = window.find("lifecycle-details").label().unwrap().to_owned();
            assert!(details.contains("skew"), "{details}");
            window.press("escape", cx);
            window.render_frame(cx);
            assert!(screen.read(cx).selected.is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn details_sit_beside_the_roster_only_when_it_fits_whole(cx: &mut TestAppContext) {
        for (width, beside) in [(1700., true), (1300., false)] {
            let (_runtime, _screen, handle) = mount_sized(cx, "talos-cp-fra1-01", width);
            cx.update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                let scroll = window.find("lifecycle-node-scroll").bounds();
                let row = window.find(("lifecycle-node", 0usize)).bounds();
                let details = window.find("lifecycle-details").bounds();
                if beside {
                    assert!(details.left() >= scroll.right(), "{width}: {details:?}");
                    assert!(row.right() <= scroll.right(), "{width}: {row:?} {scroll:?}");
                } else {
                    assert!(details.top() >= scroll.bottom(), "{width}: {details:?}");
                }
            })
            .unwrap();
        }
    }

    #[test]
    fn too_few_answers_is_unconfirmed_and_not_safe_rather_than_lost() {
        let verdict = etcd_verdict(&SourceSnapshot::Available(EtcdPreOperationAudit {
            total_members: 3,
            responding_members: 1,
            quorum_required: 2,
            can_lose: 0,
            quorum: QuorumState::NoQuorum {
                healthy: 1,
                total: 3,
            },
        }));
        assert!(matches!(verdict.tone, Tone::Warn));
        assert_eq!(verdict.label, "Quorum unconfirmed");
        assert!(
            verdict.detail.contains("1/3 members responding"),
            "{}",
            verdict.detail
        );
    }

    #[gpui_kit::test]
    fn kubelet_skew_alert_is_shown(cx: &mut TestAppContext) {
        let (_runtime, _screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let label = window
                .find(("lifecycle-alert", 0usize))
                .label()
                .unwrap()
                .to_owned();
            assert!(label.contains("Kubelet patch version skew"), "{label}");
            assert!(label.contains("talos-wk-fra1-02"), "{label}");
            // The skewed worker's row marks its kubelet as behind.
            let row = window
                .find(("lifecycle-node", 4usize))
                .label()
                .unwrap()
                .to_owned();
            assert!(row.contains("kubelet v1.34.1"), "{row}");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn silent_node_is_not_reported_rather_than_failed(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let row = window
                .find(("lifecycle-node", 5usize))
                .label()
                .unwrap()
                .to_owned();
            assert!(row.starts_with("talos-wk-fra1-03"), "{row}");
            assert!(row.contains("Talos not reported"), "{row}");
            assert!(row.contains("config not reported"), "{row}");
            for word in ["down", "failed", "unhealthy"] {
                assert!(!row.contains(word), "{row}");
            }
            let view = screen.read(cx).loader.data().unwrap().clone();
            let rows = node_rows(&view);
            assert!(!rows[5].reported());
            assert_eq!(rows[5].drift, Drift::Unknown);
            // Not reporting is named in the notice, and raises no alert.
            let notice = window.find("partial-notice").label().unwrap().to_owned();
            assert!(notice.contains("talos-wk-fra1-03"), "{notice}");
            let alerts = alert_rows(&view, &rows);
            assert!(
                alerts
                    .iter()
                    .all(|alert| !alert.message.contains("wk-fra1-03"))
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn unavailable_source_is_named_in_the_partial_notice(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, cx| {
                let mut view = example_view();
                view.snapshot.talos_discovery = SourceSnapshot::Unavailable {
                    reason: "Talos discovery is unavailable: example".into(),
                };
                view.kubelets = SourceSnapshot::Unavailable {
                    reason: "kubeconfig example".into(),
                };
                let target = screen.source.as_ref().unwrap().target.clone();
                screen.loader.resolve(target, Ok(view));
                cx.notify();
            });
            window.render_frame(cx);
            let notice = window.find("partial-notice").label().unwrap().to_owned();
            assert!(notice.contains("Talos discovery"), "{notice}");
            assert!(notice.contains("Kubelet versions"), "{notice}");
            // Unknown sources raise no skew or roster alert.
            let view = screen.read(cx).loader.data().unwrap().clone();
            let rows = node_rows(&view);
            assert!(rows.iter().all(|row| row.in_discovery.is_none()));
            assert!(alert_rows(&view, &rows).is_empty());
            let row = window
                .find(("lifecycle-node", 0usize))
                .label()
                .unwrap()
                .to_owned();
            assert!(row.contains("unknown (discovery wasn't read)"), "{row}");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn changing_target_drops_old_data(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, cx| {
                screen.selected = Some(Item::Node("talos-cp-fra1-01".into()));
                // The same target keeps its data.
                screen.set_source(Some(source("talos-cp-fra1-01")), window, cx);
                assert!(screen.loader.data().is_some());
                screen.set_source(Some(source("talos-wk-fra1-02")), window, cx);
                assert!(screen.loader.data().is_none());
                assert!(screen.selected.is_none());
            });
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn silent_target_offers_retry_without_data(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-03");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(screen.read(cx).loader.data().is_none());
            window.find("screen-retry");
        })
        .unwrap();
    }

    #[test]
    fn drift_needs_two_observed_hashes() {
        let mut view = example_view();
        // Only one hash was read: no drift can be claimed.
        for node in view.snapshot.nodes.iter_mut().skip(1) {
            node.config_hash = SourceSnapshot::Unavailable {
                reason: "not read".into(),
            };
        }
        let rows = node_rows(&view);
        assert_eq!(rows[0].drift, Drift::OnlyReading);
        assert!(rows.iter().skip(1).all(|row| row.drift == Drift::Unknown));
        // Two different hashes read: the odd one out differs.
        view.snapshot.nodes[1].config_hash = SourceSnapshot::Available("43".into());
        view.snapshot.nodes[2].config_hash = SourceSnapshot::Available("42".into());
        let rows = node_rows(&view);
        assert_eq!(rows[0].drift, Drift::InSync);
        assert_eq!(rows[1].drift, Drift::Differs);
    }

    #[test]
    fn versions_parse_with_suffixes() {
        assert_eq!(parse_version("v1.34.3"), Some((1, 34, 3)));
        assert_eq!(parse_version("1.13.2-rc1"), Some((1, 13, 2)));
        assert_eq!(parse_version("v1.30.0+k3s1"), Some((1, 30, 0)));
        assert_eq!(parse_version("garbage"), None);
    }
}
