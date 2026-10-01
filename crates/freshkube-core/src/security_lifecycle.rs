//! Framework-neutral security and lifecycle collection for Talos clusters.
//!
//! The collectors in this module deliberately keep unavailable sources explicit.
//! In particular, Kubernetes roster data is obtained only through a kubeconfig
//! served by a pinned Talos control-plane node; ambient `KUBECONFIG` is never
//! consulted.

use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use talos_rs::{
    DiscoveryMember, EtcdMemberInfo, NodeTimeInfo, TalosClient, TalosConfig, VersionInfo,
    get_discovery_members_with_retry,
};
use x509_parser::prelude::*;

use crate::{
    HasHealth, HealthIndicator, QuorumState,
    cluster_overview::{
        create_k8s_client_from_node_cached, forget_cached_k8s_client_from_node, list_cluster_nodes,
    },
};

/// A data source outcome that preserves usable partial results and unavailable
/// sources separately from failures in unrelated collection steps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceSnapshot<T> {
    Available(T),
    Partial { value: T, warnings: Vec<String> },
    Unavailable { reason: String },
}

impl<T> SourceSnapshot<T> {
    /// Returns the collected value when one is present.
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Available(value) | Self::Partial { value, .. } => Some(value),
            Self::Unavailable { .. } => None,
        }
    }

    /// Returns whether the source was fully collected.
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }
}

/// The selected Talos identity, always derived from the selected talosconfig.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterIdentity {
    pub context_name: String,
    pub endpoint_addresses: Vec<String>,
    pub target_addresses: Vec<String>,
}

impl ClusterIdentity {
    fn from_config(config: &TalosConfig) -> Self {
        let context = config.current_context();
        Self {
            context_name: config.context.clone(),
            endpoint_addresses: context
                .map(|context| context.endpoints.clone())
                .unwrap_or_default(),
            target_addresses: context
                .map(|context| context.target_nodes().to_vec())
                .unwrap_or_default(),
        }
    }
}

/// Certificate expiry state, intentionally independent of any UI color scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CertificateExpiryStatus {
    Valid,
    ExpiringSoon,
    ExpiringVerySoon,
    Expired,
}

impl CertificateExpiryStatus {
    /// Computes expiry severity with the existing 30-day and 7-day thresholds.
    pub fn from_days_remaining(days_remaining: i64) -> Self {
        match days_remaining {
            days if days <= 0 => Self::Expired,
            days if days <= 7 => Self::ExpiringVerySoon,
            days if days <= 30 => Self::ExpiringSoon,
            _ => Self::Valid,
        }
    }

    pub fn health(self) -> HealthIndicator {
        match self {
            Self::Valid => HealthIndicator::Healthy,
            Self::ExpiringSoon => HealthIndicator::Warning,
            Self::ExpiringVerySoon | Self::Expired => HealthIndicator::Error,
        }
    }
}

/// Where a certificate was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CertificateSource {
    TalosConfigCa,
    TalosConfigClient,
    TalosKubeconfigCa,
    TalosKubeconfigClient,
}

/// Portable X.509 certificate audit result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateAudit {
    pub name: String,
    pub source: CertificateSource,
    pub subject: String,
    pub issuer: String,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    pub days_remaining: i64,
    pub expiry: CertificateExpiryStatus,
    pub is_ca: bool,
}

/// RBAC identity inferred from the Talos client certificate subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbacAudit {
    pub role: String,
    pub certificate_subject: String,
}

/// The encryption mechanism reported for a Talos volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncryptionProvider {
    None,
    Static,
    NodeId,
    Tpm,
    Kms,
    Other(String),
}

impl EncryptionProvider {
    fn from_talos(value: Option<&str>) -> Self {
        let Some(value) = value else {
            return Self::None;
        };
        let normalized = value.to_ascii_lowercase();
        if normalized == "luks2" {
            Self::Static
        } else if normalized.contains("tpm") {
            Self::Tpm
        } else if normalized.contains("kms") {
            Self::Kms
        } else if normalized.contains("nodeid") || normalized.contains("node-id") {
            Self::NodeId
        } else {
            Self::Other(value.to_string())
        }
    }

    pub fn health(&self) -> HealthIndicator {
        match self {
            Self::Tpm | Self::Kms => HealthIndicator::Healthy,
            Self::Static | Self::NodeId => HealthIndicator::Warning,
            Self::None => HealthIndicator::Warning,
            Self::Other(_) => HealthIndicator::Unknown,
        }
    }
}

/// Encryption state for a single Talos STATE or EPHEMERAL volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeEncryptionAudit {
    pub volume_name: String,
    pub provider: EncryptionProvider,
}

/// Framework-neutral PKI, RBAC, and volume-encryption snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityAuditSnapshot {
    pub identity: SourceSnapshot<ClusterIdentity>,
    pub talosconfig_certificates: SourceSnapshot<Vec<CertificateAudit>>,
    pub talos_kubeconfig_certificates: SourceSnapshot<Vec<CertificateAudit>>,
    pub rbac: SourceSnapshot<RbacAudit>,
    pub volume_encryption: SourceSnapshot<Vec<VolumeEncryptionAudit>>,
}

/// Collects security state using a specific talosconfig when supplied.
#[derive(Debug, Clone, Default)]
pub struct SecurityAuditCollector {
    talosconfig_path: Option<PathBuf>,
}

impl SecurityAuditCollector {
    pub fn new(talosconfig_path: Option<PathBuf>) -> Self {
        Self { talosconfig_path }
    }

    /// Collects PKI, RBAC, and encryption state without any UI interaction.
    pub async fn collect(&self, client: &TalosClient) -> SecurityAuditSnapshot {
        self.collect_inner(client, None, None).await
    }

    /// Audits the actual selected Talos context and target, independently of
    /// the talosconfig's saved current-context. Kubernetes certificates are
    /// from the supplied pinned Talos client, never ambient credentials.
    pub async fn collect_for_context(
        &self,
        client: &TalosClient,
        context: &str,
        target: &str,
    ) -> SecurityAuditSnapshot {
        self.collect_inner(client, Some(context), Some(target))
            .await
    }

    async fn collect_inner(
        &self,
        client: &TalosClient,
        context: Option<&str>,
        target: Option<&str>,
    ) -> SecurityAuditSnapshot {
        let config = load_selected_talosconfig(self.talosconfig_path.clone(), context).await;
        let identity = match &config {
            Ok(config) => {
                let mut identity = ClusterIdentity::from_config(config);
                if let Some(target) = target {
                    identity.target_addresses = vec![node_address(target)];
                }
                SourceSnapshot::Available(identity)
            }
            Err(reason) => SourceSnapshot::Unavailable {
                reason: reason.clone(),
            },
        };

        let (talosconfig_certificates, rbac) = match &config {
            Ok(config) => audit_talosconfig_certificates(config),
            Err(reason) => (
                SourceSnapshot::Unavailable {
                    reason: reason.clone(),
                },
                SourceSnapshot::Unavailable {
                    reason: reason.clone(),
                },
            ),
        };

        let talos_kubeconfig_certificates = match bounded_request(client.kubeconfig()).await {
            Ok(kubeconfig) => audit_kubeconfig_certificates(&kubeconfig),
            Err(error) => SourceSnapshot::Unavailable {
                reason: format!("Talos did not provide a kubeconfig: {error}"),
            },
        };

        let volume_encryption = match &config {
            Ok(config) => self.collect_volume_encryption(config, target).await,
            Err(reason) => SourceSnapshot::Unavailable {
                reason: reason.clone(),
            },
        };

        SecurityAuditSnapshot {
            identity,
            talosconfig_certificates,
            talos_kubeconfig_certificates,
            rbac,
            volume_encryption,
        }
    }

    async fn collect_volume_encryption(
        &self,
        config: &TalosConfig,
        target: Option<&str>,
    ) -> SourceSnapshot<Vec<VolumeEncryptionAudit>> {
        let Some(context) = config.current_context() else {
            return SourceSnapshot::Unavailable {
                reason: format!(
                    "Talos context '{}' is not present in talosconfig",
                    config.context
                ),
            };
        };
        let Some(target) = target.or_else(|| context.target_nodes().first().map(String::as_str))
        else {
            return SourceSnapshot::Unavailable {
                reason: "The selected Talos context has no target node or endpoint".to_string(),
            };
        };
        if self
            .talosconfig_path
            .as_ref()
            .is_some_and(|path| path.to_str().is_none())
        {
            return SourceSnapshot::Unavailable {
                reason: "Volume status cannot use the applied non-UTF-8 Talosconfig path; no default config was substituted".into(),
            };
        }
        let target = node_address(target);
        match bounded_request(talos_rs::get_volume_status_for_node(
            &config.context,
            &target,
            self.talosconfig_path.as_deref().and_then(Path::to_str),
        ))
        .await
        {
            Ok(volumes) => {
                let audits: Vec<_> = volumes
                    .into_iter()
                    .filter(|volume| volume.id.contains("STATE") || volume.id.contains("EPHEMERAL"))
                    .map(|volume| VolumeEncryptionAudit {
                        volume_name: volume.id,
                        provider: EncryptionProvider::from_talos(
                            volume.encryption_provider.as_deref(),
                        ),
                    })
                    .collect();
                if audits.is_empty() {
                    SourceSnapshot::Partial {
                        value: audits,
                        warnings: vec![
                            "Talos returned no STATE or EPHEMERAL volume status".to_string(),
                        ],
                    }
                } else {
                    SourceSnapshot::Available(audits)
                }
            }
            Err(error) => SourceSnapshot::Unavailable {
                reason: format!("Talos volume status for {target} is unavailable: {error}"),
            },
        }
    }
}

/// Kubernetes node information collected from a Talos-provided kubeconfig.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesNodeRosterEntry {
    pub name: String,
    pub internal_address: Option<String>,
    pub is_control_plane: bool,
}

/// A Talos-discovery roster entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryRosterEntry {
    pub id: String,
    pub name: String,
    pub addresses: Vec<String>,
    pub machine_type: String,
}

/// Time state returned by the Talos Time API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeSynchronizationAudit {
    pub server: String,
    pub offset_seconds: f64,
    pub synced: bool,
}

/// Lifecycle state for one node. Every optional operation source is represented
/// independently so a missing value is never interpreted as healthy or ready.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeLifecycleSnapshot {
    pub name: String,
    pub address: Option<String>,
    pub machine_type: Option<String>,
    pub version: SourceSnapshot<String>,
    pub platform: SourceSnapshot<String>,
    pub time_synchronization: SourceSnapshot<TimeSynchronizationAudit>,
    pub config_hash: SourceSnapshot<String>,
}

/// Etcd state used to decide whether a disruptive operation is safe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EtcdPreOperationAudit {
    pub total_members: usize,
    pub responding_members: usize,
    pub quorum_required: usize,
    pub can_lose: usize,
    pub quorum: QuorumState,
}

impl EtcdPreOperationAudit {
    /// A pre-operation green light requires both observed quorum and one member
    /// of remaining failure tolerance. Unknown data is never a green light.
    pub fn safe_for_single_member_operation(&self) -> bool {
        self.quorum.has_quorum() && self.can_lose > 0
    }
}

/// Severity-neutral lifecycle alert. Consumers choose their own presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleAlert {
    pub health: HealthIndicator,
    pub message: String,
}

/// What a [`LifecycleAlert`] is about, so consumers can attach evidence
/// without matching on its wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleAlertKind {
    VersionMismatch,
    ConfigDrift,
    TimeUnsynchronized,
    EtcdUnsafe,
    EtcdUnavailable,
    RosterUnavailable,
    /// An alert this build doesn't produce, e.g. one deserialized from
    /// another version.
    Other,
}

const VERSION_MISMATCH: &str = "Version mismatch detected across nodes";
const CONFIG_DRIFT: &str = "Configuration drift detected across nodes";
const TIME_UNSYNCHRONIZED: &str = "Time is not synchronized on: ";
const ETCD_UNSAFE: &str = "etcd is not safe for a single-member disruptive operation";
const ETCD_UNAVAILABLE: &str = "etcd pre-operation state is unavailable or partial";
const ROSTER_UNAVAILABLE: &str = "Neither Talos discovery nor the Kubernetes roster is available";

impl LifecycleAlert {
    pub fn kind(&self) -> LifecycleAlertKind {
        match self.message.as_str() {
            VERSION_MISMATCH => LifecycleAlertKind::VersionMismatch,
            CONFIG_DRIFT => LifecycleAlertKind::ConfigDrift,
            ETCD_UNSAFE => LifecycleAlertKind::EtcdUnsafe,
            ETCD_UNAVAILABLE => LifecycleAlertKind::EtcdUnavailable,
            ROSTER_UNAVAILABLE => LifecycleAlertKind::RosterUnavailable,
            message if message.starts_with(TIME_UNSYNCHRONIZED) => {
                LifecycleAlertKind::TimeUnsynchronized
            }
            _ => LifecycleAlertKind::Other,
        }
    }
}

/// Framework-neutral lifecycle/version/config-drift snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LifecycleSnapshot {
    pub identity: SourceSnapshot<ClusterIdentity>,
    pub talos_discovery: SourceSnapshot<Vec<DiscoveryRosterEntry>>,
    pub kubernetes_roster: SourceSnapshot<Vec<KubernetesNodeRosterEntry>>,
    pub nodes: Vec<NodeLifecycleSnapshot>,
    pub etcd_pre_operation: SourceSnapshot<EtcdPreOperationAudit>,
    pub alerts: Vec<LifecycleAlert>,
}

/// Collects lifecycle state using the selected talosconfig and Talos APIs.
#[derive(Debug, Clone, Default)]
pub struct LifecycleCollector {
    talosconfig_path: Option<PathBuf>,
}

impl LifecycleCollector {
    pub fn new(talosconfig_path: Option<PathBuf>) -> Self {
        Self { talosconfig_path }
    }

    /// Collects lifecycle information with the existing pinned-source default.
    pub async fn collect(&self, client: &TalosClient) -> LifecycleSnapshot {
        self.collect_inner(client, None, None).await
    }

    /// Collects using an already identity-validated Kubernetes client (or its
    /// unavailable reason). Explicit source failure never triggers ambient
    /// Kubernetes fallback. Context is the applied selection, not saved default.
    pub async fn collect_with_kubernetes(
        &self,
        client: &TalosClient,
        context: &str,
        kubernetes: Result<kube::Client, String>,
    ) -> LifecycleSnapshot {
        self.collect_inner(client, Some(context), Some(kubernetes))
            .await
    }

    async fn collect_inner(
        &self,
        client: &TalosClient,
        context: Option<&str>,
        kubernetes: Option<Result<kube::Client, String>>,
    ) -> LifecycleSnapshot {
        let config = load_selected_talosconfig(self.talosconfig_path.clone(), context).await;
        let identity = match &config {
            Ok(config) => SourceSnapshot::Available(ClusterIdentity::from_config(config)),
            Err(reason) => SourceSnapshot::Unavailable {
                reason: reason.clone(),
            },
        };

        let (versions, times, etcd_members) = tokio::join!(
            bounded_request(client.version()),
            bounded_request(client.time()),
            bounded_request(client.etcd_members()),
        );
        let talos_discovery = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            self.collect_talos_discovery(&config, versions.as_ref().ok()),
        )
        .await
        .unwrap_or_else(|_| SourceSnapshot::Unavailable {
            reason: "Selected-context Talos discovery timed out".into(),
        });
        let control_plane_address = control_plane_address(
            etcd_members.as_ref().ok(),
            talos_discovery.value(),
            versions.as_ref().ok(),
        );
        let kubernetes_roster = match kubernetes {
            Some(Ok(client)) => kubernetes_roster_from_client(&client).await,
            Some(Err(reason)) => SourceSnapshot::Unavailable { reason },
            None => {
                self.collect_kubernetes_roster(client, control_plane_address.as_deref())
                    .await
            }
        };

        let node_roster = merge_node_roster(talos_discovery.value(), kubernetes_roster.value());
        let nodes = self
            .collect_nodes(client, &node_roster, versions, times, &config)
            .await;
        let etcd_pre_operation = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            collect_etcd_pre_operation(client, etcd_members),
        )
        .await
        .unwrap_or_else(|_| SourceSnapshot::Unavailable {
            reason: "etcd pre-operation status timed out".into(),
        });
        let alerts = lifecycle_alerts(
            &nodes,
            &talos_discovery,
            &kubernetes_roster,
            &etcd_pre_operation,
        );

        LifecycleSnapshot {
            identity,
            talos_discovery,
            kubernetes_roster,
            nodes,
            etcd_pre_operation,
            alerts,
        }
    }

    async fn collect_talos_discovery(
        &self,
        config: &Result<TalosConfig, String>,
        versions: Option<&Vec<VersionInfo>>,
    ) -> SourceSnapshot<Vec<DiscoveryRosterEntry>> {
        let Ok(config) = config else {
            return SourceSnapshot::Unavailable {
                reason: "Talos discovery requires the selected talosconfig".to_string(),
            };
        };
        if self
            .talosconfig_path
            .as_ref()
            .is_some_and(|path| path.to_str().is_none())
        {
            return SourceSnapshot::Unavailable {
                reason: "Discovery cannot use the applied non-UTF-8 Talosconfig path; no default config was substituted".into(),
            };
        }
        let fallback_addresses = versions
            .into_iter()
            .flatten()
            .map(|version| node_address(&version.node))
            .collect::<Vec<_>>();
        match get_discovery_members_with_retry(
            &config.context,
            self.talosconfig_path.as_deref().and_then(Path::to_str),
            &fallback_addresses,
        )
        .await
        {
            Ok(members) if members.is_empty() => SourceSnapshot::Partial {
                value: Vec::new(),
                warnings: vec!["Talos discovery returned no members".to_string()],
            },
            Ok(members) => SourceSnapshot::Available(members_to_roster(members)),
            Err(error) => SourceSnapshot::Unavailable {
                reason: format!("Talos discovery is unavailable: {error}"),
            },
        }
    }

    async fn collect_kubernetes_roster(
        &self,
        client: &TalosClient,
        control_plane_address: Option<&str>,
    ) -> SourceSnapshot<Vec<KubernetesNodeRosterEntry>> {
        let Some(control_plane_address) = control_plane_address else {
            return SourceSnapshot::Unavailable {
                reason: "No control-plane address was available to request a Talos kubeconfig"
                    .to_string(),
            };
        };
        let k8s_client = match create_k8s_client_from_node_cached(client, control_plane_address)
            .await
        {
            Ok(client) => client,
            Err(error) => {
                return SourceSnapshot::Unavailable {
                    reason: format!(
                        "Kubernetes roster is unavailable from Talos node {control_plane_address}: {error}"
                    ),
                };
            }
        };
        let roster = kubernetes_roster_from_client(&k8s_client).await;
        if matches!(roster, SourceSnapshot::Unavailable { .. }) {
            forget_cached_k8s_client_from_node(client, control_plane_address);
        }
        roster
    }

    async fn collect_nodes(
        &self,
        client: &TalosClient,
        roster: &BTreeMap<String, RosterNode>,
        initial_versions: Result<Vec<VersionInfo>, String>,
        initial_times: Result<Vec<NodeTimeInfo>, String>,
        config: &Result<TalosConfig, String>,
    ) -> Vec<NodeLifecycleSnapshot> {
        let initial_version_error = initial_versions.as_ref().err().cloned();
        let initial_time_error = initial_times.as_ref().err().cloned();
        let versions = initial_versions.unwrap_or_default();
        let times = initial_times.unwrap_or_default();

        let nodes = with_reporting_nodes(
            roster,
            versions
                .iter()
                .map(|version| version.node.as_str())
                .chain(times.iter().map(|time| time.node.as_str())),
        );

        let mut snapshots = BTreeMap::new();
        let mut workers = tokio::task::JoinSet::new();
        let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
        for node in nodes.into_values().take(256) {
            let version = versions.iter().find(|version| {
                version.node == node.name
                    || node.address.as_deref() == Some(node_address(&version.node).as_str())
            });
            let time = times.iter().find(|time| {
                time.node == node.name
                    || node.address.as_deref() == Some(node_address(&time.node).as_str())
            });
            let snapshot = NodeLifecycleSnapshot {
                name: node.name.clone(),
                address: node.address.clone(),
                machine_type: node.machine_type,
                version: version.map_or_else(
                    || unavailable_value("Talos version", initial_version_error.as_deref()),
                    |version| available_string(&version.version, "Talos returned an empty version"),
                ),
                platform: version.map_or_else(
                    || unavailable_value("Talos platform", initial_version_error.as_deref()),
                    |version| {
                        available_string(&version.platform, "Talos returned an empty platform")
                    },
                ),
                time_synchronization: time.map_or_else(
                    || SourceSnapshot::Unavailable {
                        reason: initial_time_error
                            .clone()
                            .unwrap_or_else(|| "No time status was returned for this node".into()),
                    },
                    |time| {
                        SourceSnapshot::Available(TimeSynchronizationAudit {
                            server: time.server.clone(),
                            offset_seconds: time.offset_seconds,
                            synced: time.synced,
                        })
                    },
                ),
                config_hash: SourceSnapshot::Unavailable {
                    reason: "Machineconfig query did not complete within this collection deadline"
                        .into(),
                },
            };
            snapshots.insert(node.name, snapshot.clone());
            let client = client.clone();
            let permits = permits.clone();
            let context = config
                .as_ref()
                .map(|config| config.context.clone())
                .map_err(Clone::clone);
            let path = self.talosconfig_path.clone();
            workers.spawn(async move {
                let Ok(_permit) = permits.acquire_owned().await else {
                    return snapshot;
                };
                let mut snapshot = snapshot;
                if let Some(address) = &snapshot.address {
                    let node_client = client.with_node(address);
                    let hash = async {
                        match &context {
                            Ok(context) => {
                                config_hash_for_selected(Some(address), context, path.as_deref())
                                    .await
                            }
                            Err(reason) => SourceSnapshot::Unavailable {
                                reason: reason.clone(),
                            },
                        }
                    };
                    let (versions, times, hash) = tokio::join!(
                        bounded_request(node_client.version()),
                        bounded_request(node_client.time()),
                        hash,
                    );
                    if let Ok(versions) = versions
                        && let Some(version) = versions.first()
                    {
                        snapshot.version =
                            available_string(&version.version, "Talos returned an empty version");
                        snapshot.platform =
                            available_string(&version.platform, "Talos returned an empty platform");
                    }
                    if let Ok(times) = times
                        && let Some(time) = times.first()
                    {
                        snapshot.time_synchronization =
                            SourceSnapshot::Available(TimeSynchronizationAudit {
                                server: time.server.clone(),
                                offset_seconds: time.offset_seconds,
                                synced: time.synced,
                            });
                    }
                    snapshot.config_hash = hash;
                }
                snapshot
            });
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        while let Ok(Some(result)) = tokio::time::timeout_at(deadline, workers.join_next()).await {
            if let Ok(snapshot) = result {
                snapshots.insert(snapshot.name.clone(), snapshot);
            }
        }
        // JoinSet abort-on-drop cancels unfinished node read transports.
        snapshots.into_values().collect()
    }
}

#[derive(Debug, Clone)]
struct RosterNode {
    name: String,
    address: Option<String>,
    machine_type: Option<String>,
}

fn load_talosconfig(path: Option<&Path>) -> Result<TalosConfig, String> {
    match path {
        Some(path) => TalosConfig::load_from(&path.to_path_buf()),
        None => TalosConfig::load_default(),
    }
    .map_err(|error| format!("Failed to load talosconfig: {error}"))
}

async fn bounded_request<T, E: std::fmt::Display>(
    future: impl Future<Output = Result<T, E>>,
) -> Result<T, String> {
    tokio::time::timeout(std::time::Duration::from_secs(5), future)
        .await
        .map_err(|_| "Talos read request timed out".to_string())?
        .map_err(|error| error.to_string())
}
async fn load_selected_talosconfig(
    path: Option<PathBuf>,
    context: Option<&str>,
) -> Result<TalosConfig, String> {
    let context = context.map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        let mut config = load_talosconfig(path.as_deref())?;
        if let Some(context) = context {
            select_talos_context(&mut config, &context)?;
        }
        Ok(config)
    })
    .await
    .map_err(|_| "Talosconfig loading worker failed".to_string())?
}

fn select_talos_context(config: &mut TalosConfig, context: &str) -> Result<(), String> {
    config.get_context(context).map_err(|_| {
        format!("Selected Talos context '{context}' is not present in the applied talosconfig")
    })?;
    config.context = context.to_string();
    Ok(())
}

async fn kubernetes_roster_from_client(
    client: &kube::Client,
) -> SourceSnapshot<Vec<KubernetesNodeRosterEntry>> {
    match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        list_cluster_nodes(client),
    )
    .await
    {
        Ok(Ok(nodes)) => {
            let truncated = nodes.len() > 256;
            let nodes: Vec<_> = nodes
                .into_iter()
                .take(256)
                .map(|node| KubernetesNodeRosterEntry {
                    name: node.name,
                    internal_address: node.internal_ip,
                    is_control_plane: node.is_control_plane,
                })
                .collect();
            if nodes.is_empty() || truncated {
                SourceSnapshot::Partial {
                    value: nodes,
                    warnings: vec![if truncated {
                        "Roster display limited to 256 nodes".into()
                    } else {
                        "Kubernetes API returned no nodes".into()
                    }],
                }
            } else {
                SourceSnapshot::Available(nodes)
            }
        }
        Ok(Err(error)) => SourceSnapshot::Unavailable {
            reason: format!("Selected-source Kubernetes roster request failed: {error}"),
        },
        Err(_) => SourceSnapshot::Unavailable {
            reason: "Selected-source Kubernetes roster request timed out".into(),
        },
    }
}

fn audit_talosconfig_certificates(
    config: &TalosConfig,
) -> (
    SourceSnapshot<Vec<CertificateAudit>>,
    SourceSnapshot<RbacAudit>,
) {
    let Some(context) = config.current_context() else {
        let reason = format!(
            "Talos context '{}' is not present in talosconfig",
            config.context
        );
        return (
            SourceSnapshot::Unavailable {
                reason: reason.clone(),
            },
            SourceSnapshot::Unavailable { reason },
        );
    };

    let mut certificates = Vec::new();
    let mut warnings = Vec::new();
    let mut role = None;

    match context
        .ca_pem()
        .map(|pem| (pem, CertificateSource::TalosConfigCa))
    {
        Ok((pem, source)) => match parse_certificate_bundle("Talos CA", source, &pem) {
            Ok(chain) => certificates.extend(chain),
            Err(error) => warnings.push(error),
        },
        Err(error) => warnings.push(format!("Talos CA is unavailable: {error}")),
    }
    match context
        .client_cert_pem()
        .map(|pem| (pem, CertificateSource::TalosConfigClient))
    {
        Ok((pem, source)) => match parse_certificate_bundle("talosconfig", source, &pem) {
            Ok(chain) => {
                role = chain
                    .first()
                    .map(|certificate| role_from_subject(&certificate.subject));
                certificates.extend(chain);
            }
            Err(error) => warnings.push(error),
        },
        Err(error) => warnings.push(format!("Talos client certificate is unavailable: {error}")),
    }

    if certificates.len() > 256 {
        certificates.truncate(256);
        warnings.push("Talos certificate inventory limited to 256 certificates".into());
    }
    let certificate_snapshot = if certificates.is_empty() {
        SourceSnapshot::Unavailable {
            reason: warnings.join("; "),
        }
    } else if warnings.is_empty() {
        SourceSnapshot::Available(certificates)
    } else {
        SourceSnapshot::Partial {
            value: certificates,
            warnings,
        }
    };
    let rbac = match role {
        Some(role) => SourceSnapshot::Available(RbacAudit {
            role,
            certificate_subject: certificate_snapshot
                .value()
                .and_then(|certificates| {
                    certificates.iter().find(|certificate| {
                        certificate.source == CertificateSource::TalosConfigClient
                    })
                })
                .map(|certificate| certificate.subject.clone())
                .unwrap_or_default(),
        }),
        None => SourceSnapshot::Unavailable {
            reason: "The Talos client certificate could not be parsed to infer RBAC role"
                .to_string(),
        },
    };
    (certificate_snapshot, rbac)
}

fn audit_kubeconfig_certificates(kubeconfig: &str) -> SourceSnapshot<Vec<CertificateAudit>> {
    if kubeconfig.len() > 1024 * 1024 {
        return SourceSnapshot::Unavailable {
            reason: "Pinned kubeconfig certificate inventory exceeds 1 MiB".into(),
        };
    }
    let value: serde_yaml::Value = match serde_yaml::from_str(kubeconfig) {
        Ok(value) => value,
        Err(error) => {
            return SourceSnapshot::Unavailable {
                reason: format!("Talos-provided kubeconfig is invalid YAML: {error}"),
            };
        }
    };
    let mut certificates = Vec::new();
    let mut warnings = Vec::new();
    if let Some(clusters) = value
        .get("clusters")
        .and_then(serde_yaml::Value::as_sequence)
    {
        for cluster in clusters {
            if let Some(encoded) = cluster
                .get("cluster")
                .and_then(|cluster| cluster.get("certificate-authority-data"))
                .and_then(serde_yaml::Value::as_str)
            {
                match parse_base64_certificate(
                    &format!(
                        "Kubernetes CA ({})",
                        cluster
                            .get("name")
                            .and_then(serde_yaml::Value::as_str)
                            .unwrap_or("unnamed cluster")
                    ),
                    CertificateSource::TalosKubeconfigCa,
                    encoded,
                ) {
                    Ok(chain) => certificates.extend(chain),
                    Err(error) => warnings.push(error),
                }
            }
        }
    }
    if let Some(users) = value.get("users").and_then(serde_yaml::Value::as_sequence) {
        for user in users {
            if let Some(encoded) = user
                .get("user")
                .and_then(|user| user.get("client-certificate-data"))
                .and_then(serde_yaml::Value::as_str)
            {
                match parse_base64_certificate(
                    &format!(
                        "kubeconfig ({})",
                        user.get("name")
                            .and_then(serde_yaml::Value::as_str)
                            .unwrap_or("unnamed user")
                    ),
                    CertificateSource::TalosKubeconfigClient,
                    encoded,
                ) {
                    Ok(chain) => certificates.extend(chain),
                    Err(error) => warnings.push(error),
                }
            }
        }
    }
    if certificates.len() > 256 {
        certificates.truncate(256);
        warnings.push("Pinned Kubernetes certificate inventory limited to 256 certificates".into());
    }
    if certificates.is_empty() {
        warnings.push("Talos-provided kubeconfig contained no parseable certificates".to_string());
        SourceSnapshot::Partial {
            value: certificates,
            warnings,
        }
    } else if warnings.is_empty() {
        SourceSnapshot::Available(certificates)
    } else {
        SourceSnapshot::Partial {
            value: certificates,
            warnings,
        }
    }
}

fn parse_base64_certificate(
    name: &str,
    source: CertificateSource,
    encoded: &str,
) -> Result<Vec<CertificateAudit>, String> {
    let pem = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format!("Failed to decode {name}: {error}"))?;
    parse_certificate_bundle(name, source, &pem)
}

fn parse_certificate_bundle(
    name: &str,
    source: CertificateSource,
    pem_data: &[u8],
) -> Result<Vec<CertificateAudit>, String> {
    if pem_data.len() > 1024 * 1024 {
        return Err(format!("{name} certificate bundle exceeds 1 MiB"));
    }
    let mut remaining = pem_data;
    let mut certificates = Vec::new();
    while !remaining.iter().all(u8::is_ascii_whitespace) {
        if certificates.len() >= 256 {
            return Err(format!(
                "{name} certificate bundle exceeds 256 certificates"
            ));
        }
        let (rest, _) = parse_x509_pem(remaining)
            .map_err(|_| format!("{name} certificate bundle contains invalid PEM"))?;
        let consumed = remaining.len().saturating_sub(rest.len());
        if consumed == 0 {
            return Err(format!("{name} certificate parser made no progress"));
        }
        let label = if certificates.is_empty() {
            name.to_string()
        } else {
            format!("{name} · chain certificate {}", certificates.len() + 1)
        };
        certificates.push(parse_certificate(&label, source, &remaining[..consumed])?);
        remaining = rest;
    }
    if certificates.is_empty() {
        return Err(format!("{name} certificate bundle is empty"));
    }
    Ok(certificates)
}

fn parse_certificate(
    name: &str,
    source: CertificateSource,
    pem_data: &[u8],
) -> Result<CertificateAudit, String> {
    let (_, pem) =
        parse_x509_pem(pem_data).map_err(|error| format!("Failed to parse {name} PEM: {error}"))?;
    let certificate = pem
        .parse_x509()
        .map_err(|error| format!("Failed to parse {name} X.509 data: {error}"))?;
    let not_before = certificate.validity().not_before.to_datetime();
    let not_after = certificate.validity().not_after.to_datetime();
    let not_before = DateTime::from_timestamp(not_before.unix_timestamp(), 0)
        .ok_or_else(|| format!("{name} has an invalid not-before timestamp"))?;
    let not_after = DateTime::from_timestamp(not_after.unix_timestamp(), 0)
        .ok_or_else(|| format!("{name} has an invalid expiry timestamp"))?;
    let days_remaining = not_after.signed_duration_since(Utc::now()).num_days();
    let is_ca = certificate
        .basic_constraints()
        .ok()
        .flatten()
        .map(|constraints| constraints.value.ca)
        .unwrap_or(false);

    Ok(CertificateAudit {
        name: name.to_string(),
        source,
        subject: certificate.subject().to_string(),
        issuer: certificate.issuer().to_string(),
        not_before,
        not_after,
        days_remaining,
        expiry: expiry_at(not_after, Utc::now()),
        is_ca,
    })
}

fn expiry_at(expires: DateTime<Utc>, now: DateTime<Utc>) -> CertificateExpiryStatus {
    if expires <= now {
        CertificateExpiryStatus::Expired
    } else {
        CertificateExpiryStatus::from_days_remaining(
            expires.signed_duration_since(now).num_days().max(1),
        )
    }
}

fn role_from_subject(subject: &str) -> String {
    let parts: Vec<_> = subject.split(',').map(str::trim).collect();
    parts
        .iter()
        .find_map(|part| part.strip_prefix("O="))
        .or_else(|| parts.iter().find_map(|part| part.strip_prefix("CN=")))
        .map(str::to_string)
        .unwrap_or_else(|| subject.to_string())
}

fn node_address(value: &str) -> String {
    let value = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .unwrap_or(value);
    if let Some(bracketed) = value.strip_prefix('[') {
        return bracketed.split(']').next().unwrap_or(bracketed).to_string();
    }
    if value.matches(':').count() == 1 {
        return value.split(':').next().unwrap_or(value).to_string();
    }
    value.to_string()
}

fn members_to_roster(members: Vec<DiscoveryMember>) -> Vec<DiscoveryRosterEntry> {
    members
        .into_iter()
        .map(|member| DiscoveryRosterEntry {
            id: member.id,
            name: member.hostname,
            addresses: member.addresses,
            machine_type: member.machine_type,
        })
        .collect()
}

fn control_plane_address(
    etcd_members: Option<&Vec<EtcdMemberInfo>>,
    discovery: Option<&Vec<DiscoveryRosterEntry>>,
    versions: Option<&Vec<VersionInfo>>,
) -> Option<String> {
    etcd_members
        .into_iter()
        .flatten()
        .find_map(EtcdMemberInfo::ip_address)
        .or_else(|| {
            discovery.into_iter().flatten().find_map(|member| {
                (member.machine_type == "controlplane")
                    .then(|| member.addresses.first().cloned())
                    .flatten()
            })
        })
        .or_else(|| {
            versions
                .into_iter()
                .flatten()
                .map(|version| node_address(&version.node))
                .next()
        })
}

fn merge_node_roster(
    discovery: Option<&Vec<DiscoveryRosterEntry>>,
    kubernetes: Option<&Vec<KubernetesNodeRosterEntry>>,
) -> BTreeMap<String, RosterNode> {
    let mut nodes = BTreeMap::new();
    for member in discovery.into_iter().flatten() {
        let name = if member.name.is_empty() {
            member
                .addresses
                .first()
                .cloned()
                .unwrap_or_else(|| member.id.clone())
        } else {
            member.name.clone()
        };
        nodes.insert(
            name.clone(),
            RosterNode {
                name,
                address: member.addresses.first().cloned(),
                machine_type: Some(member.machine_type.clone()),
            },
        );
    }
    for member in kubernetes.into_iter().flatten() {
        nodes
            .entry(member.name.clone())
            .and_modify(|node| {
                if node.address.is_none() {
                    node.address = member.internal_address.clone();
                }
                if node.machine_type.is_none() {
                    node.machine_type = Some(
                        if member.is_control_plane {
                            "controlplane"
                        } else {
                            "worker"
                        }
                        .to_string(),
                    );
                }
            })
            .or_insert_with(|| RosterNode {
                name: member.name.clone(),
                address: member.internal_address.clone(),
                machine_type: Some(
                    if member.is_control_plane {
                        "controlplane"
                    } else {
                        "worker"
                    }
                    .to_string(),
                ),
            });
    }
    nodes
}

/// Adds nodes that answered Talos but aren't on the roster. Replies name
/// nodes by the address they were asked at, so one already listed under its
/// hostname at that address is the same machine, not a new row.
fn with_reporting_nodes<'a>(
    roster: &BTreeMap<String, RosterNode>,
    reported: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, RosterNode> {
    let mut nodes = roster.clone();
    for reported in reported {
        let address = node_address(reported);
        let known = nodes.contains_key(reported)
            || nodes
                .values()
                .any(|node| node.address.as_deref() == Some(address.as_str()));
        if !known {
            nodes.insert(
                reported.to_string(),
                RosterNode {
                    name: reported.to_string(),
                    address: Some(address),
                    machine_type: None,
                },
            );
        }
    }
    nodes
}

fn available_string(value: &str, empty_reason: &str) -> SourceSnapshot<String> {
    if value.is_empty() {
        SourceSnapshot::Unavailable {
            reason: empty_reason.to_string(),
        }
    } else {
        SourceSnapshot::Available(value.to_string())
    }
}

fn unavailable_value(label: &str, request_error: Option<&str>) -> SourceSnapshot<String> {
    let reason = request_error.map_or_else(
        || format!("No {label} record was returned for this node"),
        |error| format!("{label} is unavailable: {error}"),
    );
    SourceSnapshot::Unavailable { reason }
}

async fn config_hash_for_selected(
    address: Option<&str>,
    context: &str,
    config_path: Option<&Path>,
) -> SourceSnapshot<String> {
    let Some(address) = address else {
        return SourceSnapshot::Unavailable {
            reason: "No explicit node address was available for the machineconfig query".into(),
        };
    };
    use tokio::io::AsyncReadExt;
    let mut command = tokio::process::Command::new("talosctl");
    command.args([
        "get",
        "machineconfig",
        "--nodes",
        address,
        "--context",
        context,
        "-o",
        "yaml",
    ]);
    if let Some(path) = config_path {
        command.arg("--talosconfig").arg(path);
    }
    command
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let request = async {
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        let stdout = child.stdout.take().ok_or("No machineconfig output")?;
        let mut bytes = Vec::new();
        stdout
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| error.to_string())?;
        if bytes.len() > 1024 * 1024 {
            return Err("Machineconfig output exceeded 1 MiB".into());
        }
        if !child
            .wait()
            .await
            .map_err(|error| error.to_string())?
            .success()
        {
            return Err("Selected-context machineconfig request failed".into());
        }
        parse_config_hash(&bytes)
    };
    match tokio::time::timeout(std::time::Duration::from_secs(5), request).await {
        Ok(Ok(hash)) => SourceSnapshot::Available(hash),
        Ok(Err(reason)) => SourceSnapshot::Unavailable { reason },
        Err(_) => SourceSnapshot::Unavailable {
            reason: "Machineconfig request timed out".into(),
        },
    }
}

fn parse_config_hash(bytes: &[u8]) -> Result<String, String> {
    let mut fallback = None;
    for document in serde_yaml::Deserializer::from_slice(bytes).take(64) {
        let value = serde_yaml::Value::deserialize(document)
            .map_err(|_| "Machineconfig response is invalid YAML".to_string())?;
        let metadata = value.get("metadata");
        let version = metadata.and_then(|metadata| metadata.get("version"));
        let version = match version {
            Some(serde_yaml::Value::String(value)) if !value.is_empty() => value.clone(),
            Some(serde_yaml::Value::Number(value)) => value.to_string(),
            _ => continue,
        };
        if metadata
            .and_then(|metadata| metadata.get("id"))
            .and_then(serde_yaml::Value::as_str)
            == Some("v1alpha1")
        {
            return Ok(version);
        }
        fallback.get_or_insert(version);
    }
    fallback.ok_or_else(|| "Machineconfig resource version is unavailable".into())
}

async fn collect_etcd_pre_operation(
    client: &TalosClient,
    members: Result<Vec<EtcdMemberInfo>, String>,
) -> SourceSnapshot<EtcdPreOperationAudit> {
    let members = match members {
        Ok(members) if members.is_empty() => {
            return SourceSnapshot::Unavailable {
                reason: "Talos returned no etcd members".to_string(),
            };
        }
        Ok(members) => members,
        Err(error) => {
            return SourceSnapshot::Unavailable {
                reason: format!("Talos etcd member list is unavailable: {error}"),
            };
        }
    };
    let addresses: Vec<_> = members
        .iter()
        .filter_map(EtcdMemberInfo::ip_address)
        .collect();
    let statuses = match client.etcd_status_for_nodes(&addresses).await {
        Ok(statuses) => statuses,
        Err(error) => {
            return SourceSnapshot::Unavailable {
                reason: format!("Talos etcd member status is unavailable: {error}"),
            };
        }
    };
    let responding_members = members
        .iter()
        .filter(|member| {
            statuses.iter().any(|status| {
                status.member_id == member.id && status.errors.is_empty() && !status.is_learner
            })
        })
        .count();
    let total_members = members.len();
    let quorum_required = total_members / 2 + 1;
    let quorum = QuorumState::from_counts(responding_members, total_members);
    SourceSnapshot::Available(EtcdPreOperationAudit {
        total_members,
        responding_members,
        quorum_required,
        can_lose: responding_members.saturating_sub(quorum_required),
        quorum,
    })
}

fn lifecycle_alerts(
    nodes: &[NodeLifecycleSnapshot],
    discovery: &SourceSnapshot<Vec<DiscoveryRosterEntry>>,
    kubernetes: &SourceSnapshot<Vec<KubernetesNodeRosterEntry>>,
    etcd: &SourceSnapshot<EtcdPreOperationAudit>,
) -> Vec<LifecycleAlert> {
    let mut alerts = Vec::new();
    let versions: HashSet<_> = nodes
        .iter()
        .filter_map(|node| node.version.value().map(String::as_str))
        .collect();
    if versions.len() > 1 {
        alerts.push(LifecycleAlert {
            health: HealthIndicator::Warning,
            message: VERSION_MISMATCH.to_string(),
        });
    }
    let hashes: HashSet<_> = nodes
        .iter()
        .filter_map(|node| node.config_hash.value().map(String::as_str))
        .collect();
    if hashes.len() > 1 {
        alerts.push(LifecycleAlert {
            health: HealthIndicator::Warning,
            message: CONFIG_DRIFT.to_string(),
        });
    }
    let unsynced: Vec<_> = nodes
        .iter()
        .filter(|node| {
            matches!(
                node.time_synchronization.value(),
                Some(TimeSynchronizationAudit { synced: false, .. })
            )
        })
        .map(|node| node.name.as_str())
        .collect();
    if !unsynced.is_empty() {
        alerts.push(LifecycleAlert {
            health: HealthIndicator::Warning,
            message: format!("{TIME_UNSYNCHRONIZED}{}", unsynced.join(", ")),
        });
    }
    if let SourceSnapshot::Available(etcd) = etcd {
        if !etcd.safe_for_single_member_operation() {
            alerts.push(LifecycleAlert {
                health: if etcd.quorum.has_quorum() {
                    HealthIndicator::Warning
                } else {
                    etcd.quorum.clone().health()
                },
                message: ETCD_UNSAFE.to_string(),
            });
        }
    } else {
        alerts.push(LifecycleAlert {
            health: HealthIndicator::Unknown,
            message: ETCD_UNAVAILABLE.to_string(),
        });
    }
    if discovery.value().is_none() && kubernetes.value().is_none() {
        alerts.push(LifecycleAlert {
            health: HealthIndicator::Unknown,
            message: ROSTER_UNAVAILABLE.to_string(),
        });
    }
    alerts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_from_a_listed_address_is_the_same_node_not_a_new_row() {
        let roster = merge_node_roster(
            Some(&vec![DiscoveryRosterEntry {
                id: "a".into(),
                name: "cp-1".into(),
                addresses: vec!["10.0.0.1".into()],
                machine_type: "controlplane".into(),
            }]),
            None,
        );

        let nodes = with_reporting_nodes(&roster, ["10.0.0.1", "10.0.0.1:50000", "10.0.0.9"]);

        assert_eq!(
            nodes.keys().map(String::as_str).collect::<Vec<_>>(),
            ["10.0.0.9", "cp-1"]
        );
        assert_eq!(nodes["cp-1"].machine_type.as_deref(), Some("controlplane"));
        assert_eq!(nodes["10.0.0.9"].machine_type, None);
    }

    #[test]
    fn certificate_bundle_inventory_includes_every_chain_certificate() {
        // Public certificate only; no private key or live cluster access.
        let certificate = "-----BEGIN CERTIFICATE-----\n\
MIIBUTCB2KADAgECAgkAtXGSKEzrTR4wCgYIKoZIzj0EAwIwEDEOMAwGA1UEAwwF\n\
YmVubm8wHhcNMTgxMTEzMDI1NDQwWhcNMTkxMTEzMDI1NDQwWjAQMQ4wDAYDVQQD\n\
DAViZW5ubzB2MBAGByqGSM49AgEGBSuBBAAiA2IABDhrHLVTMHC7GTyB/MNztToW\n\
ss2zlmvR62X1pQaBN6fYhBJE1XYa0V2C1fGGXj92MencOtXyfYVxn+DY07gyT/71\n\
HQ12TJOe90wwjy2/6N1W1jOv5HjphVT8JQlVNqAC+DAKBggqhkjOPQQDAgNoADBl\n\
AjAfzS1tmZ+GSAaXrsPcfAd1A9yfWVtB8tWxFNNo2j7/cL3puf2vQnlwV/0BoZZ1\n\
K4ICMQDaE0HemgYp6RPQzeb96a2gjlaEbwNy1B8O74fE24WRXiOUm4eln9wGQ3I1\n\
iV6NtZU=\n-----END CERTIFICATE-----\n";
        let bundle = format!("{certificate}{certificate}");
        let inventory = parse_certificate_bundle(
            "fixture",
            CertificateSource::TalosConfigCa,
            bundle.as_bytes(),
        )
        .unwrap();
        assert_eq!(inventory.len(), 2);
        assert_eq!(inventory[0].subject, inventory[1].subject);
        assert!(inventory[1].name.contains("chain certificate 2"));
        assert_eq!(inventory[0].expiry, CertificateExpiryStatus::Expired);
    }

    #[test]
    fn selected_context_never_uses_saved_default_identity() {
        let mut config: TalosConfig = serde_yaml::from_str(
            "context: unrelated\ncontexts:\n  unrelated:\n    endpoints: [10.1.0.1]\n    nodes: [10.1.0.2]\n    ca: ''\n    crt: ''\n    key: ''\n  selected:\n    endpoints: [10.2.0.1]\n    nodes: [10.2.0.2]\n    ca: ''\n    crt: ''\n    key: ''\n",
        ).unwrap();
        select_talos_context(&mut config, "selected").unwrap();
        let identity = ClusterIdentity::from_config(&config);
        assert_eq!(identity.context_name, "selected");
        assert_eq!(identity.endpoint_addresses, ["10.2.0.1"]);
        assert_eq!(identity.target_addresses, ["10.2.0.2"]);
        assert!(select_talos_context(&mut config, "absent").is_err());
        assert_eq!(config.context, "selected");
    }

    #[test]
    fn certificate_that_expires_later_today_is_not_expired() {
        let now = DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        assert_eq!(
            expiry_at(now + chrono::Duration::hours(1), now),
            CertificateExpiryStatus::ExpiringVerySoon
        );
        assert_eq!(expiry_at(now, now), CertificateExpiryStatus::Expired);
        assert_eq!(
            expiry_at(now - chrono::Duration::seconds(1), now),
            CertificateExpiryStatus::Expired
        );
    }

    #[test]
    fn machineconfig_version_fixture_is_not_empty_or_ambient() {
        assert_eq!(
            parse_config_hash(b"metadata:\n  version: '42'\n").unwrap(),
            "42"
        );
        assert_eq!(
            parse_config_hash(b"metadata:\n  version: 43\n").unwrap(),
            "43"
        );
        assert!(parse_config_hash(b"metadata:\n  version: ''\n").is_err());
        assert!(parse_config_hash(b"spec:\n  version: not-a-resource-version\n").is_err());
        assert_eq!(
            parse_config_hash(b"metadata:\n  id: persistent\n  version: 7\n---\nmetadata:\n  id: v1alpha1\n  version: 3\n").unwrap(),
            "3",
        );
    }

    #[test]
    fn drift_requires_observed_different_versions_not_missing_data() {
        fn unknown<T>() -> SourceSnapshot<T> {
            SourceSnapshot::Unavailable {
                reason: "not observed".into(),
            }
        }
        let node = |name: &str, hash: SourceSnapshot<String>| NodeLifecycleSnapshot {
            name: name.into(),
            address: None,
            machine_type: None,
            version: unknown(),
            platform: unknown(),
            time_synchronization: unknown(),
            config_hash: hash,
        };
        let missing_discovery = SourceSnapshot::Unavailable {
            reason: "disabled".into(),
        };
        let missing_kubernetes = SourceSnapshot::Unavailable {
            reason: "offline".into(),
        };
        let missing_etcd = SourceSnapshot::Unavailable {
            reason: "offline".into(),
        };
        let mut nodes = vec![
            node("one", SourceSnapshot::Available("a".into())),
            node("two", unknown()),
        ];
        let alerts = lifecycle_alerts(
            &nodes,
            &missing_discovery,
            &missing_kubernetes,
            &missing_etcd,
        );
        assert!(!alerts.iter().any(|alert| alert.message.contains("drift")));
        nodes.push(node("three", SourceSnapshot::Available("b".into())));
        let alerts = lifecycle_alerts(
            &nodes,
            &missing_discovery,
            &missing_kubernetes,
            &missing_etcd,
        );
        assert!(alerts.iter().any(|alert| alert.message.contains("drift")));
    }

    #[test]
    fn certificate_expiry_thresholds_are_neutral_and_stable() {
        assert_eq!(
            CertificateExpiryStatus::from_days_remaining(31),
            CertificateExpiryStatus::Valid
        );
        assert_eq!(
            CertificateExpiryStatus::from_days_remaining(30),
            CertificateExpiryStatus::ExpiringSoon
        );
        assert_eq!(
            CertificateExpiryStatus::from_days_remaining(7),
            CertificateExpiryStatus::ExpiringVerySoon
        );
        assert_eq!(
            CertificateExpiryStatus::from_days_remaining(0),
            CertificateExpiryStatus::Expired
        );
    }

    #[test]
    fn role_prefers_organization_over_common_name() {
        assert_eq!(role_from_subject("CN=admin, O=os:admin"), "os:admin");
        assert_eq!(role_from_subject("O=os:admin, CN=admin"), "os:admin");
    }

    #[test]
    fn node_address_preserves_ipv6_and_removes_a_single_port() {
        assert_eq!(node_address("10.0.0.5:50000"), "10.0.0.5");
        assert_eq!(node_address("[fd00::5]:50000"), "fd00::5");
        assert_eq!(node_address("fd00::5"), "fd00::5");
    }

    #[test]
    fn etcd_requires_observed_failure_tolerance() {
        let one = EtcdPreOperationAudit {
            total_members: 1,
            responding_members: 1,
            quorum_required: 1,
            can_lose: 0,
            quorum: QuorumState::Healthy,
        };
        assert!(!one.safe_for_single_member_operation());
        let three = EtcdPreOperationAudit {
            total_members: 3,
            responding_members: 3,
            quorum_required: 2,
            can_lose: 1,
            quorum: QuorumState::Healthy,
        };
        assert!(three.safe_for_single_member_operation());
    }

    #[test]
    fn every_produced_alert_has_a_kind() {
        let node = |name: &str, version: &str, hash: &str, synced: bool| NodeLifecycleSnapshot {
            name: name.into(),
            address: None,
            machine_type: None,
            version: SourceSnapshot::Available(version.into()),
            platform: SourceSnapshot::Unavailable {
                reason: "not observed".into(),
            },
            time_synchronization: SourceSnapshot::Available(TimeSynchronizationAudit {
                server: "time.cloudflare.com".into(),
                offset_seconds: 0.0,
                synced,
            }),
            config_hash: SourceSnapshot::Available(hash.into()),
        };
        let nodes = [
            node("one", "v1.11.0", "1", true),
            node("two", "v1.11.1", "2", false),
        ];
        fn missing<T>() -> SourceSnapshot<T> {
            SourceSnapshot::Unavailable {
                reason: "offline".into(),
            }
        }
        let unsafe_etcd = SourceSnapshot::Available(EtcdPreOperationAudit {
            total_members: 1,
            responding_members: 1,
            quorum_required: 1,
            can_lose: 0,
            quorum: QuorumState::Healthy,
        });
        let mut kinds: Vec<_> = lifecycle_alerts(&nodes, &missing(), &missing(), &unsafe_etcd)
            .iter()
            .chain(&lifecycle_alerts(&[], &missing(), &missing(), &missing()))
            .map(LifecycleAlert::kind)
            .collect();
        kinds.sort_by_key(|kind| *kind as u8);
        kinds.dedup();
        assert_eq!(
            kinds,
            [
                LifecycleAlertKind::VersionMismatch,
                LifecycleAlertKind::ConfigDrift,
                LifecycleAlertKind::TimeUnsynchronized,
                LifecycleAlertKind::EtcdUnsafe,
                LifecycleAlertKind::EtcdUnavailable,
                LifecycleAlertKind::RosterUnavailable,
            ]
        );
    }

    #[test]
    fn unavailable_etcd_produces_an_unknown_alert() {
        let alerts = lifecycle_alerts(
            &[],
            &SourceSnapshot::Unavailable {
                reason: "disabled".to_string(),
            },
            &SourceSnapshot::Unavailable {
                reason: "disabled".to_string(),
            },
            &SourceSnapshot::Unavailable {
                reason: "unreachable".to_string(),
            },
        );
        assert!(
            alerts
                .iter()
                .any(|alert| alert.health == HealthIndicator::Unknown)
        );
    }
}
