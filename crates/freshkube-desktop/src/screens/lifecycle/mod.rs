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
use crate::ui::{self, MONO_FONT, Tone, dp};

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
    display: LifecycleDisplay,
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
            self.resolve(source.target.clone(), example(&source));
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
                Ok(LifecycleView {
                    snapshot,
                    kubelets,
                    display: Default::default(),
                }
                .prepare())
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
        self.loader
            .data()
            .map(|view| (view.display.rows.clone(), view.display.alerts.clone()))
            .unwrap_or_default()
    }

    fn resolve(&mut self, target: crate::backend::Target, result: Result<LifecycleView, String>) {
        self.loader
            .resolve(target, result.map(LifecycleView::prepare));
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
        self.source
            .as_ref()
            .is_some_and(|source| source.nodes.iter().any(|known| known.name == node))
    }
}

mod example;
mod view;
use example::example;

#[cfg(test)]
#[path = "tests.rs"]
mod ui_tests;

#[derive(Clone, Debug, Default)]
struct LifecycleDisplay {
    rows: Vec<NodeRow>,
    alerts: Vec<AlertRow>,
    missing: Vec<String>,
    summary_labels: [String; 3],
}
impl LifecycleView {
    fn prepare(mut self) -> Self {
        self.display.rows = node_rows(&self);
        self.display.alerts = alert_rows(&self, &self.display.rows);
        self.display.missing = unavailable_sources(&self, &self.display.rows);
        fn distinct(values: impl Iterator<Item = String>) -> String {
            let mut values: Vec<_> = values.collect();
            values.sort();
            values.dedup();
            if values.is_empty() {
                "not reported".into()
            } else {
                values.join(", ")
            }
        }
        let warnings = self
            .display
            .alerts
            .iter()
            .filter(|alert| {
                matches!(
                    alert.health,
                    HealthIndicator::Warning | HealthIndicator::Error
                )
            })
            .count();
        self.display.summary_labels = [
            distinct(
                self.display
                    .rows
                    .iter()
                    .filter_map(|row| row.talos.as_ref().ok().cloned()),
            ),
            distinct(
                self.display
                    .rows
                    .iter()
                    .filter_map(|row| row.kubelet.as_ref().ok().cloned()),
            ),
            if self.display.alerts.is_empty() {
                "none".into()
            } else if warnings == self.display.alerts.len() {
                warnings.to_string()
            } else {
                format!("{} ({warnings} to review)", self.display.alerts.len())
            },
        ];
        self
    }
}
