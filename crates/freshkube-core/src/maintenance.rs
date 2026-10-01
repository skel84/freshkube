//! Framework-neutral maintenance-mode bootstrap workflow.
//!
//! This module deliberately separates pure state reduction from I/O. A frontend
//! sends [`MaintenanceAction`] values to a worker, the worker calls
//! [`execute_maintenance_action`], then sends the resulting [`MaintenanceEvent`]
//! back to the frontend. [`BootstrapSession::reduce`] is pure, so workers never
//! mutate frontend state.
//!
//! Talos exposes target-aware configuration generation through
//! `gen_config_with_install_disk`. The selected install disk is mandatory and
//! passed to that API, so generated configuration cannot silently fall back to
//! an unintended disk.

use std::{
    net::Ipv6Addr,
    path::{Path, PathBuf},
    time::Duration,
};

use k8s_openapi::api::core::v1::Node;
use kube::{
    Client, Config,
    api::{Api, ListParams},
};
use talos_rs::{
    DiskInfo, GenConfigResult, InsecureVersionInfo, TalosClient, TalosConfig, VolumeStatus,
    apply_config_insecure, gen_config_with_install_disk, get_disks_insecure, get_version_insecure,
    get_volume_status_insecure,
};
use tokio::process::Command;

use crate::errors::format_talos_error;
use crate::kubeconfig_selection::{
    KubeconfigSelection, parse_pinned_kubeconfig, prepare_kubeconfig, run_roster_setup,
};

/// Recommended interval for polling reboot and bootstrap readiness.
pub const BOOTSTRAP_POLL_INTERVAL: Duration =
    Duration::from_secs(crate::constants::refresh_intervals::NORMAL);

/// An address normalized for `talosctl --insecure -n` and Talos node targeting.
///
/// The stored address never includes a scheme or port. This prevents the
/// insecure maintenance workflow from accidentally treating a Kubernetes API
/// port as the Talos node identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MaintenanceEndpoint {
    address: String,
}

impl MaintenanceEndpoint {
    /// Parses a host, IP, bracketed IPv6 address, or one of those with a port.
    pub fn parse(input: impl AsRef<str>) -> Result<Self, MaintenanceError> {
        let original = input.as_ref();
        let mut value = original.trim();
        if value.is_empty() {
            return Err(MaintenanceError::InvalidEndpoint {
                input: original.to_string(),
                reason: "the address is empty".to_string(),
            });
        }

        if let Some((scheme, remainder)) = value.split_once("://") {
            if !matches!(scheme, "http" | "https") {
                return Err(MaintenanceError::InvalidEndpoint {
                    input: original.to_string(),
                    reason: "only http and https schemes may be normalized".to_string(),
                });
            }
            value = remainder;
        }

        if value.contains('/')
            || value.contains('?')
            || value.contains('#')
            || value.contains('@')
            || value.chars().any(char::is_whitespace)
        {
            return Err(MaintenanceError::InvalidEndpoint {
                input: original.to_string(),
                reason: "the endpoint must contain only a host or IP address and optional port"
                    .to_string(),
            });
        }

        let address = if let Some(bracketed) = value.strip_prefix('[') {
            let Some((host, suffix)) = bracketed.split_once(']') else {
                return Err(MaintenanceError::InvalidEndpoint {
                    input: original.to_string(),
                    reason: "the bracketed IPv6 address is missing a closing bracket".to_string(),
                });
            };
            if !suffix.is_empty() {
                let Some(port) = suffix.strip_prefix(':') else {
                    return Err(MaintenanceError::InvalidEndpoint {
                        input: original.to_string(),
                        reason: "unexpected text after bracketed IPv6 address".to_string(),
                    });
                };
                validate_port(port, original)?;
            }
            host.parse::<Ipv6Addr>()
                .map_err(|_| MaintenanceError::InvalidEndpoint {
                    input: original.to_string(),
                    reason: "the bracketed address is not a valid IPv6 address".to_string(),
                })?;
            host.to_string()
        } else {
            match value.matches(':').count() {
                0 => {
                    validate_host(value, original)?;
                    value.to_string()
                }
                1 => {
                    let (host, port) = value.split_once(':').expect("one colon was counted");
                    validate_host(host, original)?;
                    validate_port(port, original)?;
                    host.to_string()
                }
                _ => {
                    value
                        .parse::<Ipv6Addr>()
                        .map_err(|_| MaintenanceError::InvalidEndpoint {
                            input: original.to_string(),
                            reason: "IPv6 addresses with a port must be bracketed".to_string(),
                        })?;
                    value.to_string()
                }
            }
        };

        Ok(Self { address })
    }

    /// Address passed to Talos as its explicit node target.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Builds the conventional Kubernetes API URL for this node.
    pub fn kubernetes_api_url(&self) -> String {
        if self.address.parse::<Ipv6Addr>().is_ok() {
            format!("https://[{}]:6443", self.address)
        } else {
            format!("https://{}:6443", self.address)
        }
    }
}

impl std::fmt::Display for MaintenanceEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.address.fmt(formatter)
    }
}

fn validate_host(host: &str, input: &str) -> Result<(), MaintenanceError> {
    let valid = !host.is_empty()
        && !host.starts_with('.')
        && !host.starts_with('-')
        && !host.ends_with('.')
        && !host.ends_with('-')
        && !host.contains("..")
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(MaintenanceError::InvalidEndpoint {
            input: input.to_string(),
            reason: "the hostname contains unsupported characters".to_string(),
        })
    }
}

fn validate_port(port: &str, input: &str) -> Result<(), MaintenanceError> {
    if port.parse::<u16>().is_ok_and(|port| port != 0) {
        Ok(())
    } else {
        Err(MaintenanceError::InvalidEndpoint {
            input: input.to_string(),
            reason: "the port must be a number from 1 through 65535".to_string(),
        })
    }
}

/// Whether an optional maintenance source returned useful data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceAvailability<T> {
    /// The source was queried successfully.
    Available(T),
    /// The source could not be queried, but its absence does not invalidate the
    /// rest of the maintenance snapshot.
    Unavailable { reason: String },
}

impl<T> SourceAvailability<T> {
    /// Returns the available value, if any.
    pub fn available(&self) -> Option<&T> {
        match self {
            Self::Available(value) => Some(value),
            Self::Unavailable { .. } => None,
        }
    }

    /// Returns the source failure, if any.
    pub fn unavailable_reason(&self) -> Option<&str> {
        match self {
            Self::Available(_) => None,
            Self::Unavailable { reason } => Some(reason),
        }
    }
}

/// Insecure maintenance information collected from Talos APIs.
///
/// Disks are required because they establish maintenance connectivity and are
/// the source of truth for the install-target picker. Version and volume status
/// are optional, so an unavailable API is retained as partial data rather than
/// failing the collection.
#[derive(Debug, Clone)]
pub struct InsecureMaintenanceSnapshot {
    pub endpoint: MaintenanceEndpoint,
    pub version: SourceAvailability<InsecureVersionInfo>,
    pub disks: Vec<DiskInfo>,
    pub volumes: SourceAvailability<Vec<VolumeStatus>>,
}

impl InsecureMaintenanceSnapshot {
    /// Returns disks that Talos reports as safe candidates for installation.
    pub fn installable_disks(&self) -> impl Iterator<Item = &DiskInfo> {
        self.disks
            .iter()
            .filter(|disk| !disk.readonly && !disk.cdrom)
    }
}

/// A user-selected install target that was validated against Talos disk data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallTarget {
    disk_id: String,
    device_path: String,
}

impl InstallTarget {
    /// Validates and selects an installable Talos disk by its device path.
    pub fn select(
        disks: &[DiskInfo],
        device_path: impl AsRef<str>,
    ) -> Result<Self, MaintenanceError> {
        let device_path = device_path.as_ref();
        let Some(disk) = disks.iter().find(|disk| disk.dev_path == device_path) else {
            return Err(MaintenanceError::InstallTargetNotFound {
                device_path: device_path.to_string(),
            });
        };

        if disk.readonly || disk.cdrom {
            return Err(MaintenanceError::InstallTargetNotInstallable {
                device_path: disk.dev_path.clone(),
                reason: if disk.readonly {
                    "Talos reports the disk as read-only"
                } else {
                    "Talos reports the disk as a CD-ROM"
                }
                .to_string(),
            });
        }

        if disk.id.is_empty() || disk.dev_path.is_empty() {
            return Err(MaintenanceError::InstallTargetNotInstallable {
                device_path: disk.dev_path.clone(),
                reason: "Talos returned an incomplete disk identity".to_string(),
            });
        }

        Ok(Self {
            disk_id: disk.id.clone(),
            device_path: disk.dev_path.clone(),
        })
    }

    /// Talos disk resource identifier.
    pub fn disk_id(&self) -> &str {
        &self.disk_id
    }

    /// Linux device path that must appear in generated machine configuration.
    pub fn device_path(&self) -> &str {
        &self.device_path
    }
}

/// The role represented by a generated machine configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MachineRole {
    /// A control-plane node.
    #[default]
    ControlPlane,
    /// A worker node.
    Worker,
}

impl MachineRole {
    /// Standard `talosctl gen config` machine configuration filename.
    pub fn configuration_filename(self) -> &'static str {
        match self {
            Self::ControlPlane => "controlplane.yaml",
            Self::Worker => "worker.yaml",
        }
    }
}

/// Paths deliberately selected by the caller for authenticated bootstrap work.
///
/// The module never falls back to ambient `TALOSCONFIG` or `KUBECONFIG`: doing
/// so could report readiness for a different cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapPaths {
    /// Talos credentials used for bootstrap and authenticated Talos probing.
    pub talosconfig_path: PathBuf,
    /// Optional, explicitly selected Kubernetes credentials. If absent, a
    /// kubeconfig is fetched through the authenticated Talos API instead.
    pub kubeconfig_path: Option<PathBuf>,
}

/// Explicit authenticated identity used by bootstrap and readiness actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedTarget {
    /// Named context in [`BootstrapPaths::talosconfig_path`].
    pub talos_context: String,
    /// Talos node address to bootstrap and query directly.
    pub node: MaintenanceEndpoint,
    /// Optional Kubernetes node name that must become Ready. If omitted, at
    /// least one Ready node is required.
    pub kubernetes_node_name: Option<String>,
}

impl AuthenticatedTarget {
    /// Creates an explicit Talos/Kubernetes target identity.
    pub fn new(
        talos_context: impl Into<String>,
        node: MaintenanceEndpoint,
        kubernetes_node_name: Option<String>,
    ) -> Result<Self, MaintenanceError> {
        let talos_context = talos_context.into();
        if talos_context.trim().is_empty() {
            return Err(MaintenanceError::InvalidTalosContext);
        }
        if kubernetes_node_name
            .as_deref()
            .is_some_and(|name| name.trim().is_empty())
        {
            return Err(MaintenanceError::InvalidKubernetesNodeName);
        }
        Ok(Self {
            talos_context,
            node,
            kubernetes_node_name,
        })
    }
}

/// Cluster settings submitted before generating configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapPlan {
    pub endpoint: MaintenanceEndpoint,
    pub cluster_name: String,
    pub kubernetes_endpoint: String,
    pub output_dir: PathBuf,
    pub role: MachineRole,
    pub paths: BootstrapPaths,
    pub authenticated_target: AuthenticatedTarget,
}

impl BootstrapPlan {
    /// Validates bootstrap settings before any maintenance I/O begins.
    pub fn new(
        endpoint: MaintenanceEndpoint,
        cluster_name: impl Into<String>,
        kubernetes_endpoint: impl Into<String>,
        output_dir: PathBuf,
        role: MachineRole,
        paths: BootstrapPaths,
        authenticated_target: AuthenticatedTarget,
    ) -> Result<Self, MaintenanceError> {
        let cluster_name = cluster_name.into();
        let kubernetes_endpoint = kubernetes_endpoint.into();
        if cluster_name.trim().is_empty() {
            return Err(MaintenanceError::InvalidClusterName);
        }
        if kubernetes_endpoint.trim().is_empty() {
            return Err(MaintenanceError::InvalidKubernetesEndpoint);
        }
        if output_dir.as_os_str().is_empty() {
            return Err(MaintenanceError::EmptyPath {
                purpose: "configuration output directory".to_string(),
            });
        }
        if paths.talosconfig_path.as_os_str().is_empty() {
            return Err(MaintenanceError::EmptyPath {
                purpose: "talosconfig".to_string(),
            });
        }
        if endpoint != authenticated_target.node {
            return Err(MaintenanceError::TargetEndpointMismatch {
                maintenance_endpoint: endpoint.to_string(),
                authenticated_node: authenticated_target.node.to_string(),
            });
        }

        Ok(Self {
            endpoint,
            cluster_name,
            kubernetes_endpoint,
            output_dir,
            role,
            paths,
            authenticated_target,
        })
    }
}

/// A configuration-generation action. An [`InstallTarget`] is mandatory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationGenerationRequest {
    pub plan: BootstrapPlan,
    pub install_target: InstallTarget,
}

/// A configuration generated with the selected install target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedConfiguration {
    pub machine_config_path: PathBuf,
    pub talosconfig_path: PathBuf,
    pub role: MachineRole,
    pub install_target: InstallTarget,
}

/// Configuration selected for the destructive insecure apply operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationApplicationRequest {
    pub endpoint: MaintenanceEndpoint,
    pub config_path: PathBuf,
    pub install_target: InstallTarget,
}

impl ConfigurationApplicationRequest {
    /// Builds an application request that retains the validated install target.
    pub fn new(
        endpoint: MaintenanceEndpoint,
        config_path: PathBuf,
        install_target: InstallTarget,
    ) -> Result<Self, MaintenanceError> {
        if config_path.as_os_str().is_empty() {
            return Err(MaintenanceError::EmptyPath {
                purpose: "machine configuration".to_string(),
            });
        }
        Ok(Self {
            endpoint,
            config_path,
            install_target,
        })
    }
}

/// Opaque data that can be passed to apply only after the frontend obtained a
/// separate, user-visible confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationApplicationConfirmation {
    request: ConfigurationApplicationRequest,
    reviewed_content: Option<String>,
}

impl ConfigurationApplicationConfirmation {
    /// Configuration path that will be applied if confirmation is accepted.
    pub fn config_path(&self) -> &Path {
        &self.request.config_path
    }

    /// Explicit install target associated with the configuration review.
    pub fn install_target(&self) -> &InstallTarget {
        &self.request.install_target
    }

    /// Node receiving the insecure apply request.
    pub fn endpoint(&self) -> &MaintenanceEndpoint {
        &self.request.endpoint
    }
}

/// Creates the data that must be presented to, and explicitly confirmed by,
/// the user before [`apply_confirmed_configuration`] is called.
pub fn require_configuration_confirmation(
    request: ConfigurationApplicationRequest,
) -> ConfigurationApplicationConfirmation {
    ConfigurationApplicationConfirmation {
        request,
        reviewed_content: None,
    }
}

/// Result from an insecure configuration application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationApplicationOutcome {
    pub applied: bool,
    pub message: String,
}

/// Explicit `talosctl bootstrap` action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapCommand {
    pub paths: BootstrapPaths,
    pub target: AuthenticatedTarget,
}

/// Outcome of `talosctl bootstrap`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapCommandOutcome {
    pub started: bool,
    pub message: String,
}

/// Source used to obtain Kubernetes credentials for a readiness query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KubernetesCredentialsSource {
    /// An explicitly selected kubeconfig file.
    SelectedPath(PathBuf),
    /// Kubeconfig returned by the authenticated Talos API for the target node.
    TalosApi,
}

/// Actual Talos API evidence gathered by a readiness poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TalosReadinessEvidence {
    /// A secure `Version` API response arrived from the explicit target.
    Available {
        versions: Vec<TalosVersionEvidence>,
        etcd: EtcdReadinessEvidence,
    },
    /// The Talos API did not provide evidence during this poll. This is expected
    /// while a node reboots and is not an inferred failure from log text.
    Unavailable { reason: String },
}

impl TalosReadinessEvidence {
    fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }

    fn etcd_is_ready(&self) -> bool {
        matches!(
            self,
            Self::Available {
                etcd: EtcdReadinessEvidence::Ready { .. },
                ..
            }
        )
    }
}

/// Version data observed through Talos's secure API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalosVersionEvidence {
    pub node: String,
    pub version: String,
}

/// Actual etcd API evidence gathered through Talos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EtcdReadinessEvidence {
    /// At least one responding non-learner etcd member reports a leader and no
    /// member errors.
    Ready { responding_members: usize },
    /// The etcd API responded but has not reached a healthy state.
    NotReady {
        responding_members: usize,
        errors: Vec<String>,
    },
    /// The etcd API could not be queried during this poll.
    Unavailable { reason: String },
}

/// Kubernetes node state obtained through an actual Kubernetes API list call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KubernetesNodeEvidence {
    pub name: String,
    pub ready: bool,
}

/// Actual Kubernetes API evidence gathered by a readiness poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KubernetesReadinessEvidence {
    /// The selected or Talos-served kubeconfig reached the Kubernetes API and
    /// met the requested node readiness requirement.
    Ready {
        credentials_source: KubernetesCredentialsSource,
        nodes: Vec<KubernetesNodeEvidence>,
    },
    /// The Kubernetes API responded, but the expected Ready node is absent.
    NotReady {
        credentials_source: KubernetesCredentialsSource,
        nodes: Vec<KubernetesNodeEvidence>,
    },
    /// No Kubernetes API result was available during this poll.
    Unavailable {
        credentials_source: KubernetesCredentialsSource,
        reason: String,
    },
    /// Kubernetes was not queried because Talos was unavailable.
    NotChecked,
}

impl KubernetesReadinessEvidence {
    fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

/// One complete, API-backed readiness poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapReadinessEvidence {
    pub target: AuthenticatedTarget,
    pub talos: TalosReadinessEvidence,
    pub kubernetes: KubernetesReadinessEvidence,
}

impl BootstrapReadinessEvidence {
    /// Secure Talos API evidence sufficient to finish the reboot wait.
    pub fn talos_is_ready(&self) -> bool {
        self.talos.is_available()
    }

    /// Requires Talos Version, Talos etcd, and Kubernetes Node API evidence.
    pub fn cluster_is_ready(&self) -> bool {
        self.talos.is_available() && self.talos.etcd_is_ready() && self.kubernetes.is_ready()
    }
}

/// State phases for the bootstrap workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapPhase {
    /// Fetching maintenance-mode APIs.
    CollectingInsecureData,
    /// A valid Talos disk must be selected before configuration is possible.
    SelectingInstallTarget,
    /// Cluster settings are ready for generation.
    Configuring,
    /// A target-aware generator is running.
    GeneratingConfiguration,
    /// A target-aware configuration is ready for review.
    ConfigurationReady,
    /// Waiting for an explicit configuration-apply confirmation.
    AwaitingApplyConfirmation,
    /// An insecure apply operation is running.
    ApplyingConfiguration,
    /// Waiting for a secure Talos API response after the node reboots.
    WaitingForTalos,
    /// Talos is back and bootstrap may be explicitly requested.
    ReadyToBootstrap,
    /// A `talosctl bootstrap` action is running.
    Bootstrapping,
    /// Waiting for Talos etcd and Kubernetes API readiness evidence.
    WaitingForKubernetes,
    /// Bootstrap is complete and API-backed evidence has been collected.
    Complete,
    /// The caller cancelled before an in-flight network step began.
    Cancelled,
    /// A non-recoverable operation error.
    Failed(MaintenanceError),
}

impl BootstrapPhase {
    fn is_terminal(&self) -> bool {
        matches!(self, Self::Complete | Self::Cancelled | Self::Failed(_))
    }
}

/// Immutable snapshot reduced by the frontend from worker events.
#[derive(Debug, Clone)]
pub struct BootstrapSession {
    pub plan: BootstrapPlan,
    pub phase: BootstrapPhase,
    pub insecure_snapshot: Option<InsecureMaintenanceSnapshot>,
    pub install_target: Option<InstallTarget>,
    pub generated_configuration: Option<GeneratedConfiguration>,
    pub pending_confirmation: Option<ConfigurationApplicationConfirmation>,
    pub latest_readiness: Option<BootstrapReadinessEvidence>,
    pub poll_attempts: u32,
    pub last_message: Option<String>,
}

impl BootstrapSession {
    /// Starts a session whose first worker action collects insecure Talos data.
    pub fn new(plan: BootstrapPlan) -> Self {
        Self {
            plan,
            phase: BootstrapPhase::CollectingInsecureData,
            insecure_snapshot: None,
            install_target: None,
            generated_configuration: None,
            pending_confirmation: None,
            latest_readiness: None,
            poll_attempts: 0,
            last_message: None,
        }
    }

    /// Returns the one-time worker action that starts insecure collection.
    pub fn initial_collection_action(&self) -> Option<MaintenanceAction> {
        matches!(&self.phase, BootstrapPhase::CollectingInsecureData).then(|| {
            MaintenanceAction::CollectInsecure {
                endpoint: self.plan.endpoint.clone(),
            }
        })
    }

    /// Returns a polling worker action only while a reboot or cluster-readiness
    /// wait is active. Frontends own scheduling with [`BOOTSTRAP_POLL_INTERVAL`].
    pub fn polling_action(&self) -> Option<MaintenanceAction> {
        matches!(
            &self.phase,
            BootstrapPhase::WaitingForTalos | BootstrapPhase::WaitingForKubernetes
        )
        .then(|| {
            MaintenanceAction::PollReadiness(BootstrapReadinessRequest {
                paths: self.plan.paths.clone(),
                target: self.plan.authenticated_target.clone(),
            })
        })
    }

    /// Produces the next immutable snapshot after a worker event.
    pub fn reduce(&self, event: MaintenanceEvent) -> Result<Self, MaintenanceError> {
        if self.phase.is_terminal() {
            return Err(MaintenanceError::InvalidTransition {
                phase: format!("{:?}", self.phase),
                event: event.name(),
            });
        }

        if matches!(&event, MaintenanceEvent::Cancelled) {
            let mut cancelled = self.clone();
            cancelled.phase = BootstrapPhase::Cancelled;
            cancelled.last_message = Some("Maintenance operation cancelled".to_string());
            return Ok(cancelled);
        }

        if let MaintenanceEvent::OperationFailed(error) = event {
            let mut failed = self.clone();
            failed.last_message = Some(error.to_string());
            failed.phase = BootstrapPhase::Failed(error);
            return Ok(failed);
        }

        let mut next = self.clone();
        match &event {
            MaintenanceEvent::InsecureDataCollected(snapshot)
                if snapshot.endpoint != self.plan.endpoint =>
            {
                return Err(MaintenanceError::TargetEndpointMismatch {
                    maintenance_endpoint: self.plan.endpoint.to_string(),
                    authenticated_node: snapshot.endpoint.to_string(),
                });
            }
            MaintenanceEvent::ReadinessPolled(evidence)
                if evidence.target != self.plan.authenticated_target =>
            {
                return Err(MaintenanceError::InvalidTransition {
                    phase: format!("{:?}", self.phase),
                    event: "ReadinessPolled for a different authenticated target",
                });
            }
            _ => {}
        }
        match (&self.phase, event) {
            (
                BootstrapPhase::CollectingInsecureData,
                MaintenanceEvent::InsecureDataCollected(snapshot),
            ) => {
                next.insecure_snapshot = Some(snapshot);
                next.phase = BootstrapPhase::SelectingInstallTarget;
                next.last_message = None;
            }
            (
                BootstrapPhase::SelectingInstallTarget,
                MaintenanceEvent::InstallTargetSelected(target),
            ) => {
                let snapshot = self
                    .insecure_snapshot
                    .as_ref()
                    .ok_or(MaintenanceError::MissingInsecureSnapshot)?;
                let selected = InstallTarget::select(&snapshot.disks, target.device_path())?;
                if selected != target {
                    return Err(MaintenanceError::InstallTargetChanged {
                        device_path: target.device_path().to_string(),
                    });
                }
                next.install_target = Some(target);
                next.phase = BootstrapPhase::Configuring;
                next.last_message = None;
            }
            (BootstrapPhase::Configuring, MaintenanceEvent::ConfigurationGenerationStarted) => {
                next.phase = BootstrapPhase::GeneratingConfiguration;
                next.last_message = None;
            }
            (
                BootstrapPhase::GeneratingConfiguration,
                MaintenanceEvent::ConfigurationGenerationFinished(configuration),
            ) => {
                let target = self
                    .install_target
                    .as_ref()
                    .ok_or(MaintenanceError::MissingInstallTarget)?;
                if configuration.install_target != *target
                    || configuration.role != self.plan.role
                    || configuration.talosconfig_path != self.plan.paths.talosconfig_path
                    || configuration.machine_config_path
                        != self
                            .plan
                            .output_dir
                            .join(self.plan.role.configuration_filename())
                {
                    return Err(MaintenanceError::GeneratedConfigurationMismatch);
                }
                next.generated_configuration = Some(configuration);
                next.phase = BootstrapPhase::ConfigurationReady;
                next.last_message = None;
            }
            (
                BootstrapPhase::ConfigurationReady,
                MaintenanceEvent::ConfigurationApplicationRequested(request),
            ) => {
                let configuration = self
                    .generated_configuration
                    .as_ref()
                    .ok_or(MaintenanceError::MissingGeneratedConfiguration)?;
                if request.endpoint != self.plan.endpoint
                    || request.install_target != configuration.install_target
                    || request.config_path != configuration.machine_config_path
                {
                    return Err(MaintenanceError::ConfigurationApplicationMismatch);
                }
                next.pending_confirmation = Some(require_configuration_confirmation(request));
                next.phase = BootstrapPhase::AwaitingApplyConfirmation;
                next.last_message = None;
            }
            (
                BootstrapPhase::AwaitingApplyConfirmation,
                MaintenanceEvent::ConfigurationApplicationConfirmed(confirmation),
            ) => {
                if self.pending_confirmation.as_ref() != Some(&confirmation) {
                    return Err(MaintenanceError::ConfigurationApplicationMismatch);
                }
                next.pending_confirmation = Some(confirmation);
                next.phase = BootstrapPhase::ApplyingConfiguration;
                next.last_message = None;
            }
            (
                BootstrapPhase::ApplyingConfiguration,
                MaintenanceEvent::ConfigurationApplicationFinished(outcome),
            ) => {
                next.last_message = Some(outcome.message.clone());
                if outcome.applied {
                    next.phase = BootstrapPhase::WaitingForTalos;
                } else {
                    next.phase =
                        BootstrapPhase::Failed(MaintenanceError::ConfigurationApplicationFailed {
                            message: outcome.message,
                        });
                }
            }
            (BootstrapPhase::WaitingForTalos, MaintenanceEvent::ReadinessPolled(evidence)) => {
                next.poll_attempts = next.poll_attempts.saturating_add(1);
                next.last_message = readiness_message(&evidence);
                if evidence.talos_is_ready() {
                    next.phase = BootstrapPhase::ReadyToBootstrap;
                }
                next.latest_readiness = Some(evidence);
            }
            (BootstrapPhase::ReadyToBootstrap, MaintenanceEvent::BootstrapStarted) => {
                next.phase = BootstrapPhase::Bootstrapping;
                next.last_message = None;
            }
            (BootstrapPhase::Bootstrapping, MaintenanceEvent::BootstrapFinished(outcome)) => {
                next.last_message = Some(outcome.message.clone());
                if outcome.started {
                    next.phase = BootstrapPhase::WaitingForKubernetes;
                } else {
                    next.phase = BootstrapPhase::Failed(MaintenanceError::BootstrapCommandFailed {
                        message: outcome.message,
                    });
                }
            }
            (BootstrapPhase::WaitingForKubernetes, MaintenanceEvent::ReadinessPolled(evidence)) => {
                next.poll_attempts = next.poll_attempts.saturating_add(1);
                next.last_message = readiness_message(&evidence);
                if evidence.cluster_is_ready() {
                    next.phase = BootstrapPhase::Complete;
                }
                next.latest_readiness = Some(evidence);
            }
            (phase, event) => {
                return Err(MaintenanceError::InvalidTransition {
                    phase: format!("{phase:?}"),
                    event: event.name(),
                });
            }
        }
        Ok(next)
    }

    /// Validates a selection and supplies the event that moves the session into
    /// configuration. The returned event is pure and requires no worker I/O.
    pub fn select_install_target(
        &self,
        device_path: impl AsRef<str>,
    ) -> Result<MaintenanceEvent, MaintenanceError> {
        if self.phase != BootstrapPhase::SelectingInstallTarget {
            return Err(MaintenanceError::InvalidTransition {
                phase: format!("{:?}", self.phase),
                event: "InstallTargetSelected",
            });
        }
        let snapshot = self
            .insecure_snapshot
            .as_ref()
            .ok_or(MaintenanceError::MissingInsecureSnapshot)?;
        let target = InstallTarget::select(&snapshot.disks, device_path)?;
        Ok(MaintenanceEvent::InstallTargetSelected(target))
    }

    /// Starts a target-aware configuration-generation worker action.
    pub fn begin_configuration_generation(
        &self,
    ) -> Result<(Self, MaintenanceAction), MaintenanceError> {
        let target = self
            .install_target
            .clone()
            .ok_or(MaintenanceError::MissingInstallTarget)?;
        let next = self.reduce(MaintenanceEvent::ConfigurationGenerationStarted)?;
        Ok((
            next,
            MaintenanceAction::GenerateConfiguration(ConfigurationGenerationRequest {
                plan: self.plan.clone(),
                install_target: target,
            }),
        ))
    }

    /// Requests a separate, explicit application confirmation for the generated
    /// machine configuration.
    pub fn request_configuration_application(
        &self,
    ) -> Result<(Self, ConfigurationApplicationConfirmation), MaintenanceError> {
        let configuration = self
            .generated_configuration
            .as_ref()
            .ok_or(MaintenanceError::MissingGeneratedConfiguration)?;
        let request = ConfigurationApplicationRequest::new(
            self.plan.endpoint.clone(),
            configuration.machine_config_path.clone(),
            configuration.install_target.clone(),
        )?;
        let next = self.reduce(MaintenanceEvent::ConfigurationApplicationRequested(request))?;
        let confirmation = next
            .pending_confirmation
            .clone()
            .ok_or(MaintenanceError::MissingApplyConfirmation)?;
        Ok((next, confirmation))
    }

    /// Binds confirmation to the exact bounded YAML shown in a configuration
    /// review. Applying this confirmation sends frozen bytes via stdin, never
    /// reopens a file that may have changed since review.
    pub fn request_reviewed_configuration_application(
        &self,
        content: String,
    ) -> Result<(Self, ConfigurationApplicationConfirmation), MaintenanceError> {
        validate_reviewed_configuration(&content, self)?;
        let (mut next, mut confirmation) = self.request_configuration_application()?;
        confirmation.reviewed_content = Some(content);
        next.pending_confirmation = Some(confirmation.clone());
        Ok((next, confirmation))
    }

    /// Starts application only after the caller confirms the exact reviewed
    /// endpoint, config path, and install target.
    pub fn confirm_configuration_application(
        &self,
        confirmation: ConfigurationApplicationConfirmation,
    ) -> Result<(Self, MaintenanceAction), MaintenanceError> {
        let next = self.reduce(MaintenanceEvent::ConfigurationApplicationConfirmed(
            confirmation.clone(),
        ))?;
        Ok((next, MaintenanceAction::ApplyConfiguration(confirmation)))
    }

    /// Starts the explicit cluster bootstrap command after secure Talos evidence
    /// has made the session ready.
    pub fn begin_bootstrap(&self) -> Result<(Self, MaintenanceAction), MaintenanceError> {
        let next = self.reduce(MaintenanceEvent::BootstrapStarted)?;
        Ok((
            next,
            MaintenanceAction::Bootstrap(BootstrapCommand {
                paths: self.plan.paths.clone(),
                target: self.plan.authenticated_target.clone(),
            }),
        ))
    }
}

/// Worker inputs. These values are portable across GUI, CLI, and TUI frontends.
#[derive(Debug, Clone)]
pub enum MaintenanceAction {
    CollectInsecure { endpoint: MaintenanceEndpoint },
    GenerateConfiguration(ConfigurationGenerationRequest),
    ApplyConfiguration(ConfigurationApplicationConfirmation),
    Bootstrap(BootstrapCommand),
    PollReadiness(BootstrapReadinessRequest),
}

/// Worker output delivered back to [`BootstrapSession::reduce`].
#[derive(Debug, Clone)]
pub enum MaintenanceEvent {
    InsecureDataCollected(InsecureMaintenanceSnapshot),
    InstallTargetSelected(InstallTarget),
    ConfigurationGenerationStarted,
    ConfigurationGenerationFinished(GeneratedConfiguration),
    ConfigurationApplicationRequested(ConfigurationApplicationRequest),
    ConfigurationApplicationConfirmed(ConfigurationApplicationConfirmation),
    ConfigurationApplicationFinished(ConfigurationApplicationOutcome),
    ReadinessPolled(BootstrapReadinessEvidence),
    BootstrapStarted,
    BootstrapFinished(BootstrapCommandOutcome),
    Cancelled,
    OperationFailed(MaintenanceError),
}

impl MaintenanceEvent {
    fn name(&self) -> &'static str {
        match self {
            Self::InsecureDataCollected(_) => "InsecureDataCollected",
            Self::InstallTargetSelected(_) => "InstallTargetSelected",
            Self::ConfigurationGenerationStarted => "ConfigurationGenerationStarted",
            Self::ConfigurationGenerationFinished(_) => "ConfigurationGenerationFinished",
            Self::ConfigurationApplicationRequested(_) => "ConfigurationApplicationRequested",
            Self::ConfigurationApplicationConfirmed(_) => "ConfigurationApplicationConfirmed",
            Self::ConfigurationApplicationFinished(_) => "ConfigurationApplicationFinished",
            Self::ReadinessPolled(_) => "ReadinessPolled",
            Self::BootstrapStarted => "BootstrapStarted",
            Self::BootstrapFinished(_) => "BootstrapFinished",
            Self::Cancelled => "Cancelled",
            Self::OperationFailed(_) => "OperationFailed",
        }
    }
}

/// Inputs for one API-backed readiness probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapReadinessRequest {
    pub paths: BootstrapPaths,
    pub target: AuthenticatedTarget,
}

/// Plain-data failures used in session transitions and worker events.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MaintenanceError {
    #[error("Invalid maintenance endpoint '{input}': {reason}")]
    InvalidEndpoint { input: String, reason: String },
    #[error("A Talos context name is required")]
    InvalidTalosContext,
    #[error("A Kubernetes node name, when provided, cannot be empty")]
    InvalidKubernetesNodeName,
    #[error("A cluster name is required")]
    InvalidClusterName,
    #[error("A Kubernetes endpoint is required")]
    InvalidKubernetesEndpoint,
    #[error("The {purpose} path is required")]
    EmptyPath { purpose: String },
    #[error(
        "Maintenance endpoint {maintenance_endpoint} does not match authenticated Talos node {authenticated_node}"
    )]
    TargetEndpointMismatch {
        maintenance_endpoint: String,
        authenticated_node: String,
    },
    #[error("Install target '{device_path}' was not returned by Talos")]
    InstallTargetNotFound { device_path: String },
    #[error("Install target '{device_path}' cannot be used: {reason}")]
    InstallTargetNotInstallable { device_path: String, reason: String },
    #[error("The selected install target '{device_path}' changed before it was applied")]
    InstallTargetChanged { device_path: String },
    #[error("An install target must be selected before configuration generation")]
    MissingInstallTarget,
    #[error("No insecure Talos snapshot is available")]
    MissingInsecureSnapshot,
    #[error("No target-aware generated configuration is available")]
    MissingGeneratedConfiguration,
    #[error("No pending configuration application confirmation is available")]
    MissingApplyConfirmation,
    #[error("Generated configuration does not match the selected role and install target")]
    GeneratedConfigurationMismatch,
    #[error(
        "Configuration application does not match the reviewed endpoint, path, and install target"
    )]
    ConfigurationApplicationMismatch,
    #[error("The selected {purpose} path is not valid UTF-8: {path}")]
    NonUtf8Path { purpose: String, path: PathBuf },
    #[error("Maintenance operation cancelled")]
    Cancelled,
    #[error("Failed to generate target-aware machine configuration: {message}")]
    ConfigurationGenerationFailed { message: String },
    #[error("Failed to collect maintenance disks: {message}")]
    InsecureDiskCollectionFailed { message: String },
    #[error("Configuration application failed: {message}")]
    ConfigurationApplicationFailed { message: String },
    #[error("Bootstrap command failed: {message}")]
    BootstrapCommandFailed { message: String },
    #[error("Invalid transition from {phase} for event {event}")]
    InvalidTransition { phase: String, event: &'static str },
}

/// Executes one worker action without ever changing a [`BootstrapSession`].
///
/// `is_cancelled` is checked before every network step. It does not claim to
/// interrupt an already-submitted Talos or Kubernetes request, because that
/// request may already have changed machine state.
pub async fn execute_maintenance_action<F>(
    action: MaintenanceAction,
    is_cancelled: F,
) -> MaintenanceEvent
where
    F: Fn() -> bool,
{
    match action {
        MaintenanceAction::CollectInsecure { endpoint } => {
            match collect_insecure_snapshot(&endpoint, &is_cancelled).await {
                Ok(snapshot) => MaintenanceEvent::InsecureDataCollected(snapshot),
                Err(MaintenanceError::Cancelled) => MaintenanceEvent::Cancelled,
                Err(error) => MaintenanceEvent::OperationFailed(error),
            }
        }
        MaintenanceAction::GenerateConfiguration(request) => {
            match generate_configuration(&request, &is_cancelled).await {
                Ok(configuration) => {
                    MaintenanceEvent::ConfigurationGenerationFinished(configuration)
                }
                Err(MaintenanceError::Cancelled) => MaintenanceEvent::Cancelled,
                Err(error) => MaintenanceEvent::OperationFailed(error),
            }
        }
        MaintenanceAction::ApplyConfiguration(confirmation) => {
            match apply_confirmed_configuration(&confirmation, &is_cancelled).await {
                Ok(outcome) => MaintenanceEvent::ConfigurationApplicationFinished(outcome),
                Err(MaintenanceError::Cancelled) => MaintenanceEvent::Cancelled,
                Err(error) => MaintenanceEvent::OperationFailed(error),
            }
        }
        MaintenanceAction::Bootstrap(command) => {
            match bootstrap_cluster(&command, &is_cancelled).await {
                Ok(outcome) => MaintenanceEvent::BootstrapFinished(outcome),
                Err(MaintenanceError::Cancelled) => MaintenanceEvent::Cancelled,
                Err(error) => MaintenanceEvent::OperationFailed(error),
            }
        }
        MaintenanceAction::PollReadiness(request) => {
            match poll_bootstrap_readiness(&request, &is_cancelled).await {
                Ok(evidence) => MaintenanceEvent::ReadinessPolled(evidence),
                Err(MaintenanceError::Cancelled) => MaintenanceEvent::Cancelled,
                Err(error) => MaintenanceEvent::OperationFailed(error),
            }
        }
    }
}

/// Collects maintenance-mode data from actual insecure Talos APIs.
pub async fn collect_insecure_snapshot<F>(
    endpoint: &MaintenanceEndpoint,
    is_cancelled: &F,
) -> Result<InsecureMaintenanceSnapshot, MaintenanceError>
where
    F: Fn() -> bool,
{
    ensure_not_cancelled(is_cancelled)?;
    let version = match get_version_insecure(endpoint.address()).await {
        Ok(version) => SourceAvailability::Available(version),
        Err(error) => SourceAvailability::Unavailable {
            reason: format_talos_error(&error),
        },
    };

    ensure_not_cancelled(is_cancelled)?;
    let disks = get_disks_insecure(endpoint.address())
        .await
        .map_err(|error| MaintenanceError::InsecureDiskCollectionFailed {
            message: format_talos_error(&error),
        })?;
    if disks.len() > 256 {
        return Err(MaintenanceError::InsecureDiskCollectionFailed {
            message: "Talos disk inventory exceeded the 256-disk workflow limit".into(),
        });
    }

    ensure_not_cancelled(is_cancelled)?;
    let volumes = match get_volume_status_insecure(endpoint.address()).await {
        Ok(volumes) if volumes.len() <= 256 => SourceAvailability::Available(volumes),
        Ok(_) => SourceAvailability::Unavailable {
            reason: "Talos volume inventory exceeded the 256-volume workflow limit".into(),
        },
        Err(error) => SourceAvailability::Unavailable {
            reason: format_talos_error(&error),
        },
    };

    Ok(InsecureMaintenanceSnapshot {
        endpoint: endpoint.clone(),
        version,
        disks,
        volumes,
    })
}

/// Generates configuration with the validated install disk passed directly to
/// `talosctl gen config --install-disk`; the selected disk is never ignored.
pub async fn generate_configuration<F>(
    request: &ConfigurationGenerationRequest,
    is_cancelled: &F,
) -> Result<GeneratedConfiguration, MaintenanceError>
where
    F: Fn() -> bool,
{
    ensure_not_cancelled(is_cancelled)?;
    let output_dir = path_as_utf8(&request.plan.output_dir, "configuration output directory")?;
    let additional_sans = [request.plan.endpoint.address(), "127.0.0.1"];
    let result = gen_config_with_install_disk(
        &request.plan.cluster_name,
        &request.plan.kubernetes_endpoint,
        output_dir,
        Some(&additional_sans),
        true,
        Some(request.install_target.device_path()),
    )
    .await
    .map_err(|error| MaintenanceError::ConfigurationGenerationFailed {
        message: format_talos_error(&error),
    })?;

    Ok(generated_configuration_from_result(request, result))
}

fn generated_configuration_from_result(
    request: &ConfigurationGenerationRequest,
    result: GenConfigResult,
) -> GeneratedConfiguration {
    let machine_config_path = match request.plan.role {
        MachineRole::ControlPlane => PathBuf::from(result.controlplane_path),
        MachineRole::Worker => PathBuf::from(result.worker_path),
    };
    GeneratedConfiguration {
        machine_config_path,
        talosconfig_path: PathBuf::from(result.talosconfig_path),
        role: request.plan.role,
        install_target: request.install_target.clone(),
    }
}

fn validate_reviewed_configuration(
    content: &str,
    session: &BootstrapSession,
) -> Result<(), MaintenanceError> {
    if content.len() > 512 * 1024 {
        return Err(MaintenanceError::ConfigurationApplicationMismatch);
    }
    let yaml: serde_yaml::Value = serde_yaml::from_str(content)
        .map_err(|_| MaintenanceError::ConfigurationApplicationMismatch)?;
    let target = session
        .install_target
        .as_ref()
        .ok_or(MaintenanceError::MissingInstallTarget)?;
    let role = match session.plan.role {
        MachineRole::ControlPlane => "controlplane",
        MachineRole::Worker => "worker",
    };
    if yaml["machine"]["install"]["disk"].as_str() != Some(target.device_path())
        || yaml["machine"]["type"].as_str() != Some(role)
    {
        return Err(MaintenanceError::ConfigurationApplicationMismatch);
    }
    Ok(())
}

/// Applies a reviewed configuration to a maintenance-mode Talos node.
pub async fn apply_confirmed_configuration<F>(
    confirmation: &ConfigurationApplicationConfirmation,
    is_cancelled: &F,
) -> Result<ConfigurationApplicationOutcome, MaintenanceError>
where
    F: Fn() -> bool,
{
    ensure_not_cancelled(is_cancelled)?;
    if let Some(content) = &confirmation.reviewed_content {
        use tokio::io::AsyncWriteExt;
        // Refresh actual disk identity immediately before the destructive step.
        let disks = tokio::time::timeout(
            Duration::from_secs(20),
            get_disks_insecure(confirmation.endpoint().address()),
        )
        .await
        .map_err(|_| MaintenanceError::InsecureDiskCollectionFailed {
            message: "Pre-apply disk identity refresh timed out".into(),
        })?
        .map_err(|error| MaintenanceError::InsecureDiskCollectionFailed {
            message: format_talos_error(&error),
        })?;
        let current = InstallTarget::select(&disks, confirmation.install_target().device_path())?;
        if current != *confirmation.install_target() {
            return Err(MaintenanceError::InstallTargetChanged {
                device_path: confirmation.install_target().device_path().into(),
            });
        }
        ensure_not_cancelled(is_cancelled)?;
        // This submitted mutation MUST be awaited to completion, not aborted.
        let mut child = Command::new("talosctl")
            .args([
                "apply-config",
                "--insecure",
                "-n",
                confirmation.endpoint().address(),
                "-f",
                "-",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| MaintenanceError::ConfigurationApplicationFailed {
                message: error.to_string(),
            })?;
        let write_result = match child.stdin.take() {
            Some(mut stdin) => stdin.write_all(content.as_bytes()).await,
            None => Err(std::io::Error::other("No configuration stdin")),
        };
        // Even a failed stdin write must reap the mutation before releasing
        // the caller's shared guard.
        let output = child.wait_with_output().await.map_err(|error| {
            MaintenanceError::ConfigurationApplicationFailed {
                message: error.to_string(),
            }
        })?;
        write_result.map_err(|error| MaintenanceError::ConfigurationApplicationFailed {
            message: error.to_string(),
        })?;
        return Ok(ConfigurationApplicationOutcome {
            applied: output.status.success(),
            message: if output.status.success() {
                "Reviewed configuration accepted; waiting for secure Talos API after installation/reboot.".into()
            } else {
                command_failure_message(&output.stderr, &output.stdout)
            },
        });
    }
    let config_path = path_as_utf8(&confirmation.request.config_path, "machine configuration")?;
    let result = apply_config_insecure(confirmation.request.endpoint.address(), config_path)
        .await
        .map_err(|error| MaintenanceError::ConfigurationApplicationFailed {
            message: format_talos_error(&error),
        })?;

    Ok(ConfigurationApplicationOutcome {
        applied: result.success,
        message: result.message,
    })
}

/// Executes bootstrap against the explicit target and selected talosconfig.
pub async fn bootstrap_cluster<F>(
    command: &BootstrapCommand,
    is_cancelled: &F,
) -> Result<BootstrapCommandOutcome, MaintenanceError>
where
    F: Fn() -> bool,
{
    ensure_not_cancelled(is_cancelled)?;
    let output = Command::new("talosctl")
        .arg("--context")
        .arg(&command.target.talos_context)
        .arg("--talosconfig")
        .arg(&command.paths.talosconfig_path)
        .arg("-n")
        .arg(command.target.node.address())
        .arg("-e")
        .arg(command.target.node.address())
        .arg("bootstrap")
        .output()
        .await
        .map_err(|error| MaintenanceError::BootstrapCommandFailed {
            message: error.to_string(),
        })?;

    let message = if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() {
            "Bootstrap command completed; waiting for Talos and Kubernetes API readiness."
                .to_string()
        } else {
            stdout
        }
    } else {
        command_failure_message(&output.stderr, &output.stdout)
    };

    Ok(BootstrapCommandOutcome {
        started: output.status.success(),
        message,
    })
}

/// Polls secure Talos, etcd, and Kubernetes APIs. No log output is parsed or
/// used as readiness evidence.
pub async fn poll_bootstrap_readiness<F>(
    request: &BootstrapReadinessRequest,
    is_cancelled: &F,
) -> Result<BootstrapReadinessEvidence, MaintenanceError>
where
    F: Fn() -> bool,
{
    ensure_not_cancelled(is_cancelled)?;
    let target = request.target.clone();
    let talos_client = match load_talos_client(&request.paths, &target).await {
        Ok(client) => client,
        Err(reason) => {
            return Ok(BootstrapReadinessEvidence {
                target,
                talos: TalosReadinessEvidence::Unavailable { reason },
                kubernetes: KubernetesReadinessEvidence::NotChecked,
            });
        }
    };

    ensure_not_cancelled(is_cancelled)?;
    let versions = match talos_client
        .with_node(target.node.address())
        .version()
        .await
    {
        Ok(versions) if !versions.is_empty() => versions
            .into_iter()
            .map(|version| TalosVersionEvidence {
                node: version.node,
                version: version.version,
            })
            .collect(),
        Ok(_) => {
            return Ok(BootstrapReadinessEvidence {
                target,
                talos: TalosReadinessEvidence::Unavailable {
                    reason: "Talos Version API returned no node responses".to_string(),
                },
                kubernetes: KubernetesReadinessEvidence::NotChecked,
            });
        }
        Err(error) => {
            return Ok(BootstrapReadinessEvidence {
                target,
                talos: TalosReadinessEvidence::Unavailable {
                    reason: format_talos_error(&error),
                },
                kubernetes: KubernetesReadinessEvidence::NotChecked,
            });
        }
    };

    ensure_not_cancelled(is_cancelled)?;
    let etcd = match talos_client
        .etcd_status_for_nodes(&[target.node.address().to_string()])
        .await
    {
        Ok(statuses) => {
            let errors = statuses
                .iter()
                .flat_map(|status| status.errors.iter().cloned())
                .collect::<Vec<_>>();
            let healthy = !statuses.is_empty()
                && statuses.iter().any(|status| !status.is_learner)
                && statuses
                    .iter()
                    .all(|status| status.leader_id != 0 && status.errors.is_empty());
            if healthy {
                EtcdReadinessEvidence::Ready {
                    responding_members: statuses.len(),
                }
            } else {
                EtcdReadinessEvidence::NotReady {
                    responding_members: statuses.len(),
                    errors,
                }
            }
        }
        Err(error) => EtcdReadinessEvidence::Unavailable {
            reason: format_talos_error(&error),
        },
    };

    ensure_not_cancelled(is_cancelled)?;
    let (credentials_source, kubernetes_client) =
        match kubernetes_client_for_probe(&request.paths, &target, &talos_client).await {
            Ok(value) => value,
            Err((credentials_source, reason)) => {
                return Ok(BootstrapReadinessEvidence {
                    target,
                    talos: TalosReadinessEvidence::Available { versions, etcd },
                    kubernetes: KubernetesReadinessEvidence::Unavailable {
                        credentials_source,
                        reason,
                    },
                });
            }
        };

    ensure_not_cancelled(is_cancelled)?;
    let kubernetes = match collect_kubernetes_node_evidence(kubernetes_client).await {
        Ok(nodes) => {
            let ready = match target.kubernetes_node_name.as_deref() {
                Some(expected_name) => nodes
                    .iter()
                    .any(|node| node.name == expected_name && node.ready),
                None => nodes.iter().any(|node| node.ready),
            };
            if ready {
                KubernetesReadinessEvidence::Ready {
                    credentials_source,
                    nodes,
                }
            } else {
                KubernetesReadinessEvidence::NotReady {
                    credentials_source,
                    nodes,
                }
            }
        }
        Err(reason) => KubernetesReadinessEvidence::Unavailable {
            credentials_source,
            reason,
        },
    };

    Ok(BootstrapReadinessEvidence {
        target,
        talos: TalosReadinessEvidence::Available { versions, etcd },
        kubernetes,
    })
}

fn ensure_not_cancelled(is_cancelled: &impl Fn() -> bool) -> Result<(), MaintenanceError> {
    if is_cancelled() {
        Err(MaintenanceError::Cancelled)
    } else {
        Ok(())
    }
}

async fn load_talos_client(
    paths: &BootstrapPaths,
    target: &AuthenticatedTarget,
) -> Result<TalosClient, String> {
    let path = paths.talosconfig_path.clone();
    let name = target.talos_context.clone();
    let context = tokio::task::spawn_blocking(move || {
        let config = TalosConfig::load_from(&path).map_err(|error| format_talos_error(&error))?;
        config
            .get_context(&name)
            .cloned()
            .map_err(|error| format_talos_error(&error))
    })
    .await
    .map_err(|_| "Talos credential worker failed".to_string())??;
    let context = pinned_talos_context(context, &target.node);
    TalosClient::from_context(&context)
        .await
        .map_err(|error| format_talos_error(&error))
}

fn pinned_talos_context(
    mut context: talos_rs::Context,
    node: &MaintenanceEndpoint,
) -> talos_rs::Context {
    // Freshly generated talosconfig may have no endpoints, and an existing
    // context may point elsewhere. Neither can override this explicit node.
    context.endpoints = vec![node.address().to_string()];
    context.nodes = vec![node.address().to_string()];
    context
}

async fn kubernetes_client_for_probe(
    paths: &BootstrapPaths,
    target: &AuthenticatedTarget,
    talos_client: &TalosClient,
) -> Result<(KubernetesCredentialsSource, Client), (KubernetesCredentialsSource, String)> {
    let source = paths
        .kubeconfig_path
        .as_ref()
        .map(|path| KubernetesCredentialsSource::SelectedPath(path.clone()))
        .unwrap_or(KubernetesCredentialsSource::TalosApi);
    // Fetch authenticated, node-pinned identity BEFORE opening any selected
    // credentials or executing a kubeconfig authentication plugin.
    let pinned_yaml = talos_client
        .with_node(target.node.address())
        .kubeconfig()
        .await
        .map_err(|error| (source.clone(), format_talos_error(&error)))?;
    let path = paths.kubeconfig_path.clone();
    let node = target.node.address().to_string();
    let prepared = run_roster_setup(move || {
        prepare_probe_kubeconfig(path, &pinned_yaml, &node)
            .map_err(crate::cluster_overview::K8sError::ClientCreate)
    })
    .await
    .map_err(|error| (source.clone(), error.to_string()))?;
    let runtime = tokio::runtime::Handle::current();
    let client = run_roster_setup(move || {
        runtime.block_on(async {
            let config = Config::from_custom_kubeconfig(prepared.0, &prepared.1)
                .await
                .map_err(|_| {
                    crate::cluster_overview::K8sError::ClientCreate(
                        "Could not load credentials for verified readiness source".into(),
                    )
                })?;
            Client::try_from(config).map_err(|_| {
                crate::cluster_overview::K8sError::ClientCreate(
                    "Could not create verified readiness client".into(),
                )
            })
        })
    })
    .await
    .map_err(|error| (source.clone(), error.to_string()))?;
    Ok((source, client))
}

fn prepare_probe_kubeconfig(
    path: Option<PathBuf>,
    pinned_yaml: &str,
    node: &str,
) -> Result<(kube::config::Kubeconfig, kube::config::KubeConfigOptions), String> {
    let selected = path.is_some();
    let selection = path
        .map(|path| KubeconfigSelection::File {
            path,
            context: None,
        })
        .unwrap_or(KubeconfigSelection::TalosControlPlane);
    let mut prepared = prepare_kubeconfig(
        &selection,
        parse_pinned_kubeconfig(pinned_yaml),
        Some(node),
        || None,
    );
    // Readiness must report the explicitly selected source as unavailable on
    // rejection, not silently report a different source as healthy.
    if selected && !prepared.selected_file {
        return Err(prepared.warning.unwrap_or_else(|| {
            "Selected Kubernetes identity could not be verified against Talos".into()
        }));
    }
    let config = prepared
        .config
        .take()
        .ok_or_else(|| "Authenticated Talos kubeconfig is unavailable or invalid".to_string())?;
    Ok((config, prepared.options))
}

async fn collect_kubernetes_node_evidence(
    client: Client,
) -> Result<Vec<KubernetesNodeEvidence>, String> {
    let nodes: Api<Node> = Api::all(client);
    let response = nodes
        .list(&ListParams::default().limit(256))
        .await
        .map_err(|error| format!("Kubernetes Node API request failed: {error}"))?;
    if response
        .metadata
        .continue_
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        return Err(
            "Kubernetes node list exceeded the 256-node evidence limit; readiness is unknown"
                .into(),
        );
    }
    Ok(response
        .items
        .iter()
        .map(|node| KubernetesNodeEvidence {
            name: node.metadata.name.clone().unwrap_or_default(),
            ready: node
                .status
                .as_ref()
                .and_then(|status| status.conditions.as_ref())
                .is_some_and(|conditions| {
                    conditions
                        .iter()
                        .any(|condition| condition.type_ == "Ready" && condition.status == "True")
                }),
        })
        .collect())
}

fn path_as_utf8<'a>(path: &'a Path, purpose: &str) -> Result<&'a str, MaintenanceError> {
    path.to_str().ok_or_else(|| MaintenanceError::NonUtf8Path {
        purpose: purpose.to_string(),
        path: path.to_path_buf(),
    })
}

fn command_failure_message(stderr: &[u8], stdout: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    if !stderr.is_empty() {
        stderr
    } else {
        let stdout = String::from_utf8_lossy(stdout).trim().to_string();
        if stdout.is_empty() {
            "talosctl bootstrap exited unsuccessfully without output".to_string()
        } else {
            stdout
        }
    }
}

fn readiness_message(evidence: &BootstrapReadinessEvidence) -> Option<String> {
    match &evidence.talos {
        TalosReadinessEvidence::Unavailable { reason } => Some(reason.clone()),
        TalosReadinessEvidence::Available {
            etcd: EtcdReadinessEvidence::Unavailable { reason },
            ..
        } => Some(reason.clone()),
        TalosReadinessEvidence::Available {
            etcd: EtcdReadinessEvidence::NotReady { errors, .. },
            ..
        } if !errors.is_empty() => Some(errors.join("; ")),
        _ => match &evidence.kubernetes {
            KubernetesReadinessEvidence::Unavailable { reason, .. } => Some(reason.clone()),
            KubernetesReadinessEvidence::NotReady { .. } => {
                Some("Kubernetes API responded, but no required node is Ready".to_string())
            }
            _ => None,
        },
    }
}

// ---------------------------------------------------------------------------
// Frontend-neutral workflow helpers shared by every desktop frontend.
// ---------------------------------------------------------------------------

/// Largest generated configuration a frontend will show for review and apply.
pub const REVIEW_LIMIT: usize = 512 * 1024;
/// Number of progress lines a frontend keeps.
pub const PROGRESS_HISTORY_LIMIT: usize = 64;
/// Longest a read-only maintenance action may run before it is reported as
/// unknown rather than failed.
pub const READ_TIMEOUT: Duration = Duration::from_secs(65);

/// Checks a `--endpoint` value before any window opens or I/O begins.
pub fn validate_maintenance_endpoint(endpoint: &str) -> Result<(), MaintenanceError> {
    MaintenanceEndpoint::parse(endpoint).map(|_| ())
}

/// The editable maintenance form, before it becomes a validated
/// [`BootstrapPlan`]. Generation's credentials always belong to this workflow
/// (`<output>/talosconfig`), never an ambient or previously configured cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceDraft {
    pub endpoint: String,
    pub cluster: String,
    pub kubernetes_api: String,
    pub output: String,
    pub role: MachineRole,
    pub context: String,
    pub bootstrap_node: String,
    pub kubernetes_node: String,
    pub kubeconfig: String,
}

impl MaintenanceDraft {
    /// Defaults for a node reached at `endpoint` (possibly empty or invalid;
    /// validation happens in [`Self::plan`]).
    pub fn new(endpoint: &str) -> Self {
        let api = MaintenanceEndpoint::parse(endpoint)
            .map(|endpoint| endpoint.kubernetes_api_url())
            .unwrap_or_default();
        Self {
            bootstrap_node: endpoint.to_string(),
            endpoint: endpoint.to_string(),
            kubernetes_api: api,
            cluster: "talos-cluster".into(),
            output: "freshkube-generated".into(),
            role: MachineRole::ControlPlane,
            context: "talos-cluster".into(),
            kubernetes_node: String::new(),
            kubeconfig: String::new(),
        }
    }

    /// Renames the cluster. The authenticated context follows while it still
    /// equals the cluster name, as `talosctl gen config` names them alike.
    pub fn set_cluster_name(&mut self, value: String) {
        if self.context == self.cluster {
            self.context = value.clone();
        }
        self.cluster = value;
    }

    /// Validates the form into a plan, before any I/O.
    pub fn plan(&self) -> Result<BootstrapPlan, MaintenanceError> {
        if !self.kubernetes_api.trim().starts_with("https://")
            || self.kubernetes_api.chars().any(char::is_whitespace)
        {
            return Err(MaintenanceError::InvalidKubernetesEndpoint);
        }
        let output = PathBuf::from(self.output.trim());
        BootstrapPlan::new(
            MaintenanceEndpoint::parse(&self.endpoint)?,
            self.cluster.trim(),
            self.kubernetes_api.trim(),
            output.clone(),
            self.role,
            BootstrapPaths {
                talosconfig_path: output.join("talosconfig"),
                kubeconfig_path: (!self.kubeconfig.trim().is_empty())
                    .then(|| PathBuf::from(self.kubeconfig.trim())),
            },
            AuthenticatedTarget::new(
                self.context.trim(),
                MaintenanceEndpoint::parse(&self.bootstrap_node)?,
                (!self.kubernetes_node.trim().is_empty())
                    .then(|| self.kubernetes_node.trim().into()),
            )?,
        )
    }
}

/// Whether an action changes a node or its credentials, as opposed to reading.
pub fn is_mutation(action: &MaintenanceAction) -> bool {
    matches!(
        action,
        MaintenanceAction::GenerateConfiguration(_)
            | MaintenanceAction::ApplyConfiguration(_)
            | MaintenanceAction::Bootstrap(_)
    )
}

/// As [`execute_maintenance_action`], but a read-only action that outlives
/// `read_timeout` ends as *unknown* evidence (never a guessed failure or
/// readiness). Mutations are never timed out: a submitted change must be
/// awaited to its real outcome.
pub async fn execute_maintenance_action_bounded<F>(
    action: MaintenanceAction,
    is_cancelled: F,
    read_timeout: Duration,
) -> MaintenanceEvent
where
    F: Fn() -> bool,
{
    if is_mutation(&action) {
        return execute_maintenance_action(action, is_cancelled).await;
    }
    let poll_target = match &action {
        MaintenanceAction::PollReadiness(request) => Some(request.target.clone()),
        _ => None,
    };
    match tokio::time::timeout(
        read_timeout,
        execute_maintenance_action(action, is_cancelled),
    )
    .await
    {
        Ok(event) => event,
        Err(_) => match poll_target {
            Some(target) => MaintenanceEvent::ReadinessPolled(BootstrapReadinessEvidence {
                target,
                talos: TalosReadinessEvidence::Unavailable {
                    reason: "Secure readiness probe timed out; status is unknown".into(),
                },
                kubernetes: KubernetesReadinessEvidence::NotChecked,
            }),
            None => {
                MaintenanceEvent::OperationFailed(MaintenanceError::InsecureDiskCollectionFailed {
                    message: "Maintenance read timed out; no readiness was inferred".into(),
                })
            }
        },
    }
}

/// Reads a generated configuration for review: one regular file, bounded by
/// [`REVIEW_LIMIT`], valid UTF-8. The returned text is what gets applied.
pub fn frozen_review(path: &Path) -> Result<String, String> {
    use std::io::Read;
    if !std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("Configuration review requires a regular file".into());
    }
    let file = std::fs::File::open(path)
        .map_err(|error| format!("Cannot review generated configuration: {error}"))?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("Configuration review requires a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(REVIEW_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > REVIEW_LIMIT {
        return Err("Configuration exceeds the 512 KiB review limit".into());
    }
    String::from_utf8(bytes).map_err(|_| "Configuration is not valid UTF-8".into())
}

/// Whether `entered` is exactly the confirmation phrase that was required.
pub fn confirmation_matches(expected: Option<String>, entered: &str) -> bool {
    expected.is_some_and(|expected| expected == entered)
}

impl BootstrapSession {
    /// The phrase that confirms applying the reviewed configuration, naming
    /// the exact node and disk.
    pub fn apply_confirmation_phrase(&self) -> Option<String> {
        let confirmation = self.pending_confirmation.as_ref()?;
        Some(format!(
            "APPLY {} {}",
            confirmation.endpoint(),
            confirmation.install_target().device_path()
        ))
    }

    /// The phrase that confirms bootstrapping etcd. Only a control plane that
    /// has answered on its secure Talos API can bootstrap.
    pub fn bootstrap_confirmation_phrase(&self) -> Option<String> {
        (self.phase == BootstrapPhase::ReadyToBootstrap
            && self.plan.role == MachineRole::ControlPlane)
            .then(|| {
                format!(
                    "BOOTSTRAP {} {}",
                    self.plan.authenticated_target.talos_context,
                    self.plan.authenticated_target.node,
                )
            })
    }

    /// One progress line describing this session's phase and latest message.
    pub fn progress_line(&self) -> String {
        format!(
            "{:?}: {}",
            self.phase,
            self.last_message.as_deref().unwrap_or("")
        )
    }
}

/// A bounded history of progress lines.
#[derive(Debug, Clone, Default)]
pub struct ProgressLog {
    entries: std::collections::VecDeque<String>,
}

impl ProgressLog {
    /// Appends a line, dropping the oldest beyond [`PROGRESS_HISTORY_LIMIT`]
    /// and truncating overlong lines.
    pub fn record(&mut self, message: impl Into<String>) {
        if self.entries.len() == PROGRESS_HISTORY_LIMIT {
            self.entries.pop_front();
        }
        self.entries
            .push_back(message.into().chars().take(4096).collect());
    }

    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn last(&self) -> Option<&String> {
        self.entries.back()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(path: &str, readonly: bool, cdrom: bool) -> DiskInfo {
        DiskInfo {
            id: path.trim_start_matches("/dev/").to_string(),
            dev_path: path.to_string(),
            size: 1024,
            size_pretty: "1 KB".to_string(),
            model: None,
            serial: None,
            transport: None,
            rotational: false,
            readonly,
            cdrom,
            wwid: None,
            bus_path: None,
        }
    }

    fn endpoint() -> MaintenanceEndpoint {
        MaintenanceEndpoint::parse("https://192.0.2.10:50000").unwrap()
    }

    fn plan() -> BootstrapPlan {
        let endpoint = endpoint();
        BootstrapPlan::new(
            endpoint.clone(),
            "test-cluster",
            endpoint.kubernetes_api_url(),
            PathBuf::from("generated"),
            MachineRole::ControlPlane,
            BootstrapPaths {
                talosconfig_path: PathBuf::from("generated/talosconfig"),
                kubeconfig_path: None,
            },
            AuthenticatedTarget::new("test-cluster", endpoint, Some("cp-1".to_string())).unwrap(),
        )
        .unwrap()
    }

    fn snapshot() -> InsecureMaintenanceSnapshot {
        InsecureMaintenanceSnapshot {
            endpoint: endpoint(),
            version: SourceAvailability::Unavailable {
                reason: "maintenance version API unavailable".to_string(),
            },
            disks: vec![disk("/dev/sda", false, false)],
            volumes: SourceAvailability::Unavailable {
                reason: "volume status unavailable".to_string(),
            },
        }
    }

    fn talos_ready() -> TalosReadinessEvidence {
        TalosReadinessEvidence::Available {
            versions: vec![TalosVersionEvidence {
                node: "cp-1".to_string(),
                version: "v1.10.0".to_string(),
            }],
            etcd: EtcdReadinessEvidence::Ready {
                responding_members: 1,
            },
        }
    }

    fn cluster_ready_evidence() -> BootstrapReadinessEvidence {
        BootstrapReadinessEvidence {
            target: plan().authenticated_target,
            talos: talos_ready(),
            kubernetes: KubernetesReadinessEvidence::Ready {
                credentials_source: KubernetesCredentialsSource::TalosApi,
                nodes: vec![KubernetesNodeEvidence {
                    name: "cp-1".to_string(),
                    ready: true,
                }],
            },
        }
    }

    #[test]
    fn normalizes_maintenance_endpoint_without_retaining_port() {
        assert_eq!(endpoint().address(), "192.0.2.10");
        assert_eq!(
            MaintenanceEndpoint::parse("[2001:db8::10]:50000")
                .unwrap()
                .address(),
            "2001:db8::10"
        );
        assert_eq!(
            MaintenanceEndpoint::parse("2001:db8::10")
                .unwrap()
                .kubernetes_api_url(),
            "https://[2001:db8::10]:6443"
        );
    }

    #[test]
    fn rejects_endpoint_paths_and_invalid_ports() {
        assert!(MaintenanceEndpoint::parse("https://node.example/path").is_err());
        assert!(MaintenanceEndpoint::parse("node.example:0").is_err());
        assert!(MaintenanceEndpoint::parse("2001:db8::10:50000").is_err());
    }

    #[test]
    fn install_target_must_be_a_writable_non_optical_talos_disk() {
        let disks = vec![disk("/dev/sda", true, false), disk("/dev/sr0", false, true)];
        assert!(matches!(
            InstallTarget::select(&disks, "/dev/sda"),
            Err(MaintenanceError::InstallTargetNotInstallable { .. })
        ));
        assert!(matches!(
            InstallTarget::select(&disks, "/dev/sr0"),
            Err(MaintenanceError::InstallTargetNotInstallable { .. })
        ));
        assert!(matches!(
            InstallTarget::select(&disks, "/dev/nvme0n1"),
            Err(MaintenanceError::InstallTargetNotFound { .. })
        ));
    }

    #[test]
    fn state_requires_talos_then_kubernetes_api_evidence() {
        let selected = InstallTarget::select(&snapshot().disks, "/dev/sda").unwrap();
        let mut session = BootstrapSession::new(plan());
        session = session
            .reduce(MaintenanceEvent::InsecureDataCollected(snapshot()))
            .unwrap();
        session = session
            .reduce(MaintenanceEvent::InstallTargetSelected(selected.clone()))
            .unwrap();
        assert_eq!(session.phase, BootstrapPhase::Configuring);

        session = session
            .reduce(MaintenanceEvent::ConfigurationGenerationStarted)
            .unwrap();
        let generated = GeneratedConfiguration {
            machine_config_path: PathBuf::from("generated/controlplane.yaml"),
            talosconfig_path: PathBuf::from("generated/talosconfig"),
            role: MachineRole::ControlPlane,
            install_target: selected,
        };
        session = session
            .reduce(MaintenanceEvent::ConfigurationGenerationFinished(
                generated.clone(),
            ))
            .unwrap();
        session = session
            .reduce(MaintenanceEvent::ConfigurationApplicationRequested(
                ConfigurationApplicationRequest::new(
                    endpoint(),
                    generated.machine_config_path,
                    generated.install_target,
                )
                .unwrap(),
            ))
            .unwrap();
        let confirmation = session.pending_confirmation.clone().unwrap();
        session = session
            .reduce(MaintenanceEvent::ConfigurationApplicationConfirmed(
                confirmation,
            ))
            .unwrap();
        session = session
            .reduce(MaintenanceEvent::ConfigurationApplicationFinished(
                ConfigurationApplicationOutcome {
                    applied: true,
                    message: "accepted".to_string(),
                },
            ))
            .unwrap();

        let unavailable = BootstrapReadinessEvidence {
            target: plan().authenticated_target,
            talos: TalosReadinessEvidence::Unavailable {
                reason: "node rebooting".to_string(),
            },
            kubernetes: KubernetesReadinessEvidence::NotChecked,
        };
        session = session
            .reduce(MaintenanceEvent::ReadinessPolled(unavailable))
            .unwrap();
        assert_eq!(session.phase, BootstrapPhase::WaitingForTalos);

        let talos_only = BootstrapReadinessEvidence {
            target: plan().authenticated_target,
            talos: talos_ready(),
            kubernetes: KubernetesReadinessEvidence::NotChecked,
        };
        session = session
            .reduce(MaintenanceEvent::ReadinessPolled(talos_only))
            .unwrap();
        assert_eq!(session.phase, BootstrapPhase::ReadyToBootstrap);

        session = session.reduce(MaintenanceEvent::BootstrapStarted).unwrap();
        session = session
            .reduce(MaintenanceEvent::BootstrapFinished(
                BootstrapCommandOutcome {
                    started: true,
                    message: "accepted".to_string(),
                },
            ))
            .unwrap();

        let no_kubernetes_evidence = BootstrapReadinessEvidence {
            target: plan().authenticated_target,
            talos: talos_ready(),
            kubernetes: KubernetesReadinessEvidence::NotChecked,
        };
        session = session
            .reduce(MaintenanceEvent::ReadinessPolled(no_kubernetes_evidence))
            .unwrap();
        assert_eq!(session.phase, BootstrapPhase::WaitingForKubernetes);

        session = session
            .reduce(MaintenanceEvent::ReadinessPolled(cluster_ready_evidence()))
            .unwrap();
        assert_eq!(session.phase, BootstrapPhase::Complete);
    }

    #[test]
    fn generated_configuration_keeps_selected_disk_and_role() {
        let request = ConfigurationGenerationRequest {
            plan: plan(),
            install_target: InstallTarget::select(&snapshot().disks, "/dev/sda").unwrap(),
        };
        let generated = generated_configuration_from_result(
            &request,
            GenConfigResult {
                controlplane_path: "generated/controlplane.yaml".to_string(),
                worker_path: "generated/worker.yaml".to_string(),
                talosconfig_path: "generated/talosconfig".to_string(),
                output_dir: "generated".to_string(),
            },
        );

        assert_eq!(
            generated.machine_config_path,
            PathBuf::from("generated/controlplane.yaml")
        );
        assert_eq!(
            generated.talosconfig_path,
            request.plan.paths.talosconfig_path
        );
        assert_eq!(generated.install_target, request.install_target);
    }

    fn configuration_ready_session() -> BootstrapSession {
        let session = BootstrapSession::new(plan())
            .reduce(MaintenanceEvent::InsecureDataCollected(snapshot()))
            .unwrap();
        let selected = session.select_install_target("/dev/sda").unwrap();
        let session = session.reduce(selected).unwrap();
        let (session, action) = session.begin_configuration_generation().unwrap();
        let MaintenanceAction::GenerateConfiguration(request) = action else {
            panic!("wrong action")
        };
        assert_eq!(request.install_target.device_path(), "/dev/sda");
        session
            .reduce(MaintenanceEvent::ConfigurationGenerationFinished(
                GeneratedConfiguration {
                    machine_config_path: PathBuf::from("generated/controlplane.yaml"),
                    talosconfig_path: PathBuf::from("generated/talosconfig"),
                    role: MachineRole::ControlPlane,
                    install_target: request.install_target,
                },
            ))
            .unwrap()
    }

    #[test]
    fn review_binds_exact_yaml_and_rejects_changed_install_disk_or_role() {
        let session = configuration_ready_session();
        let reviewed = "machine:\n  type: controlplane\n  install:\n    disk: /dev/sda\n";
        assert!(
            session
                .request_reviewed_configuration_application(
                    reviewed.replace("/dev/sda", "/dev/sdb"),
                )
                .is_err()
        );
        assert!(
            session
                .request_reviewed_configuration_application(
                    reviewed.replace("controlplane", "worker"),
                )
                .is_err()
        );
        let (pending, confirmation) = session
            .request_reviewed_configuration_application(reviewed.into())
            .unwrap();
        assert_eq!(confirmation.reviewed_content.as_deref(), Some(reviewed));
        // An unreviewed or different confirmation cannot satisfy the pending
        // frozen-content confirmation even if endpoint/path/disk match.
        let (_, unreviewed) = session.request_configuration_application().unwrap();
        assert!(
            pending
                .confirm_configuration_application(unreviewed)
                .is_err()
        );
        let (applying, action) = pending
            .confirm_configuration_application(confirmation)
            .unwrap();
        assert_eq!(applying.phase, BootstrapPhase::ApplyingConfiguration);
        assert!(matches!(action, MaintenanceAction::ApplyConfiguration(_)));
    }

    #[test]
    fn reducer_rejects_stale_snapshot_and_authenticated_readiness_identity() {
        let session = BootstrapSession::new(plan());
        let mut stale = snapshot();
        stale.endpoint = MaintenanceEndpoint::parse("192.0.2.11").unwrap();
        assert!(
            session
                .reduce(MaintenanceEvent::InsecureDataCollected(stale))
                .is_err()
        );
        let mut waiting = configuration_ready_session();
        waiting.phase = BootstrapPhase::WaitingForTalos;
        let mut evidence = cluster_ready_evidence();
        evidence.target.node = MaintenanceEndpoint::parse("192.0.2.11").unwrap();
        assert!(
            waiting
                .reduce(MaintenanceEvent::ReadinessPolled(evidence))
                .is_err()
        );
        let mut evidence = cluster_ready_evidence();
        evidence.target.talos_context = "other-context".into();
        assert!(
            waiting
                .reduce(MaintenanceEvent::ReadinessPolled(evidence))
                .is_err()
        );
    }

    #[test]
    fn unknown_health_never_completes_bootstrap_and_cancel_retains_results() {
        let mut waiting = configuration_ready_session();
        waiting.phase = BootstrapPhase::WaitingForKubernetes;
        let evidence = BootstrapReadinessEvidence {
            target: plan().authenticated_target,
            talos: talos_ready(),
            kubernetes: KubernetesReadinessEvidence::Unavailable {
                credentials_source: KubernetesCredentialsSource::SelectedPath("rejected".into()),
                reason: "Selected CA does not match pinned Talos identity".into(),
            },
        };
        let waiting = waiting
            .reduce(MaintenanceEvent::ReadinessPolled(evidence.clone()))
            .unwrap();
        assert_eq!(waiting.phase, BootstrapPhase::WaitingForKubernetes);
        let cancelled = waiting.reduce(MaintenanceEvent::Cancelled).unwrap();
        assert_eq!(cancelled.phase, BootstrapPhase::Cancelled);
        assert_eq!(cancelled.latest_readiness, Some(evidence));
        assert!(cancelled.generated_configuration.is_some());
    }

    #[tokio::test]
    async fn cancellation_prevents_every_maintenance_action_before_io() {
        let session = configuration_ready_session();
        let (_, confirmation) = session
            .request_reviewed_configuration_application(
                "machine:\n  type: controlplane\n  install:\n    disk: /dev/sda\n".into(),
            )
            .unwrap();
        let actions = [
            MaintenanceAction::CollectInsecure {
                endpoint: endpoint(),
            },
            MaintenanceAction::GenerateConfiguration(ConfigurationGenerationRequest {
                plan: session.plan.clone(),
                install_target: session.install_target.clone().unwrap(),
            }),
            MaintenanceAction::ApplyConfiguration(confirmation),
            MaintenanceAction::Bootstrap(BootstrapCommand {
                paths: plan().paths,
                target: plan().authenticated_target,
            }),
            MaintenanceAction::PollReadiness(BootstrapReadinessRequest {
                paths: plan().paths,
                target: plan().authenticated_target,
            }),
        ];
        for action in actions {
            assert!(matches!(
                execute_maintenance_action(action, || true).await,
                MaintenanceEvent::Cancelled
            ));
        }
    }

    // Public certificate fixture; no private credentials and no live request.
    const TEST_CA: &str = "-----BEGIN CERTIFICATE-----\n\
MIIBUTCB2KADAgECAgkAtXGSKEzrTR4wCgYIKoZIzj0EAwIwEDEOMAwGA1UEAwwF\n\
YmVubm8wHhcNMTgxMTEzMDI1NDQwWhcNMTkxMTEzMDI1NDQwWjAQMQ4wDAYDVQQD\n\
DAViZW5ubzB2MBAGByqGSM49AgEGBSuBBAAiA2IABDhrHLVTMHC7GTyB/MNztToW\n\
ss2zlmvR62X1pQaBN6fYhBJE1XYa0V2C1fGGXj92MencOtXyfYVxn+DY07gyT/71\n\
HQ12TJOe90wwjy2/6N1W1jOv5HjphVT8JQlVNqAC+DAKBggqhkjOPQQDAgNoADBl\n\
AjAfzS1tmZ+GSAaXrsPcfAd1A9yfWVtB8tWxFNNo2j7/cL3puf2vQnlwV/0BoZZ1\n\
K4ICMQDaE0HemgYp6RPQzeb96a2gjlaEbwNy1B8O74fE24WRXiOUm4eln9wGQ3I1\n\
iV6NtZU=\n-----END CERTIFICATE-----\n";

    fn kubeconfig_fixture(certificate: &str, server: &str, insecure: bool) -> String {
        use base64::Engine;
        let ca = base64::engine::general_purpose::STANDARD.encode(certificate);
        format!(
            r#"apiVersion: v1
kind: Config
current-context: fixture
clusters:
- name: fixture
  cluster:
    server: {server}
    certificate-authority-data: {ca}
    insecure-skip-tls-verify: {insecure}
contexts:
- name: fixture
  context:
    cluster: fixture
    user: never-run
users:
- name: never-run
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: should-never-be-executed-during-identity-validation
"#
        )
    }

    struct FixtureFile(PathBuf);
    impl FixtureFile {
        fn new(content: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "talos-maintenance-source-{}-{}.yaml",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ));
            std::fs::write(&path, content).unwrap();
            Self(path)
        }
    }
    impl Drop for FixtureFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn selected_readiness_requires_pinned_ca_and_tls_before_plugins() {
        let pinned = kubeconfig_fixture(TEST_CA, "https://pinned.invalid:6443", false);
        let selected = FixtureFile::new(&kubeconfig_fixture(
            TEST_CA,
            "https://selected.invalid:6443",
            false,
        ));
        let (config, options) =
            prepare_probe_kubeconfig(Some(selected.0.clone()), &pinned, "192.0.2.10").unwrap();
        assert_eq!(options.context.as_deref(), Some("fixture"));
        assert_eq!(
            config.clusters[0]
                .cluster
                .as_ref()
                .unwrap()
                .server
                .as_deref(),
            Some("https://selected.invalid:6443")
        );
        assert!(
            prepare_probe_kubeconfig(
                Some(selected.0.clone()),
                "invalid pinned config",
                "192.0.2.10"
            )
            .is_err()
        );
        let insecure = FixtureFile::new(&kubeconfig_fixture(
            TEST_CA,
            "https://selected.invalid:6443",
            true,
        ));
        assert!(prepare_probe_kubeconfig(Some(insecure.0.clone()), &pinned, "192.0.2.10").is_err());
        let plain = FixtureFile::new(&kubeconfig_fixture(
            TEST_CA,
            "http://selected.invalid:6443",
            false,
        ));
        assert!(prepare_probe_kubeconfig(Some(plain.0.clone()), &pinned, "192.0.2.10").is_err());
        // Change a certificate signature byte, preserving parseable ASN.1.
        let changed = TEST_CA.replace("iV6NtZU=", "iV6NtZQ=");
        let mismatched = FixtureFile::new(&kubeconfig_fixture(
            &changed,
            "https://other.invalid:6443",
            false,
        ));
        assert!(
            prepare_probe_kubeconfig(Some(mismatched.0.clone()), &pinned, "192.0.2.10").is_err()
        );
    }

    #[test]
    fn authenticated_clients_pin_transport_and_node_not_context_endpoints() {
        let context = talos_rs::Context {
            endpoints: vec!["unrelated.invalid".into()],
            nodes: vec!["another-node.invalid".into()],
            ca: "preserved-ca".into(),
            crt: "preserved-client".into(),
            key: "preserved-key".into(),
        };
        let context = pinned_talos_context(context, &endpoint());
        assert_eq!(context.endpoints, vec!["192.0.2.10"]);
        assert_eq!(context.nodes, vec!["192.0.2.10"]);
        assert_eq!(context.ca, "preserved-ca");
    }

    fn draft() -> MaintenanceDraft {
        MaintenanceDraft::new("192.0.2.10")
    }

    #[test]
    fn draft_generation_uses_owned_credentials() {
        let plan = draft().plan().unwrap();
        assert_eq!(
            plan.paths.talosconfig_path,
            PathBuf::from("freshkube-generated/talosconfig")
        );
        assert_eq!(plan.authenticated_target.talos_context, "talos-cluster");
        assert_eq!(plan.endpoint, plan.authenticated_target.node);
        assert!(plan.paths.kubeconfig_path.is_none());
    }

    #[test]
    fn draft_rejects_a_different_bootstrap_node_before_io() {
        let mut draft = draft();
        draft.bootstrap_node = "192.0.2.11".into();
        assert!(matches!(
            draft.plan(),
            Err(MaintenanceError::TargetEndpointMismatch { .. })
        ));
    }

    #[test]
    fn draft_context_follows_cluster_name_until_edited() {
        let mut draft = draft();
        draft.set_cluster_name("lab".into());
        assert_eq!(draft.context, "lab");
        draft.context = "custom".into();
        draft.set_cluster_name("other".into());
        assert_eq!(draft.context, "custom");
    }

    #[test]
    fn draft_requires_an_https_kubernetes_api() {
        let mut draft = draft();
        draft.kubernetes_api = "http://192.0.2.10:6443".into();
        assert_eq!(
            draft.plan().unwrap_err(),
            MaintenanceError::InvalidKubernetesEndpoint
        );
    }

    #[test]
    fn endpoint_validation_is_shared() {
        assert!(validate_maintenance_endpoint("192.0.2.10").is_ok());
        assert!(validate_maintenance_endpoint("[2001:db8::10]:50000").is_ok());
        for bad in ["", " ", "https://node.example/path", "node.example:0"] {
            assert!(validate_maintenance_endpoint(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn confirmation_requires_exact_identity_phrase() {
        assert!(confirmation_matches(
            Some("APPLY 192.0.2.10 /dev/sda".into()),
            "APPLY 192.0.2.10 /dev/sda"
        ));
        assert!(!confirmation_matches(
            Some("APPLY 192.0.2.10 /dev/sda".into()),
            "APPLY 192.0.2.11 /dev/sda"
        ));
        assert!(!confirmation_matches(None, ""));
    }

    #[test]
    fn workers_cannot_obtain_a_bootstrap_phrase() {
        let mut draft = draft();
        draft.role = MachineRole::Worker;
        let mut session = BootstrapSession::new(draft.plan().unwrap());
        session.phase = BootstrapPhase::ReadyToBootstrap;
        assert!(session.bootstrap_confirmation_phrase().is_none());
        let mut control = BootstrapSession::new(self::draft().plan().unwrap());
        assert!(control.bootstrap_confirmation_phrase().is_none());
        control.phase = BootstrapPhase::ReadyToBootstrap;
        assert_eq!(
            control.bootstrap_confirmation_phrase().as_deref(),
            Some("BOOTSTRAP talos-cluster 192.0.2.10")
        );
    }

    #[test]
    fn progress_history_is_bounded() {
        let mut log = ProgressLog::default();
        for index in 0..1000 {
            log.record(index.to_string());
        }
        assert_eq!(log.len(), PROGRESS_HISTORY_LIMIT);
        assert_eq!(log.last().unwrap(), "999");
    }

    #[test]
    fn review_is_bounded_and_rejects_non_regular_files() {
        let directory = std::env::temp_dir().join(format!(
            "talos-maintenance-review-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir(&directory).unwrap();
        let file = directory.join("controlplane.yaml");
        std::fs::write(&file, "reviewed bytes").unwrap();
        assert_eq!(frozen_review(&file).unwrap(), "reviewed bytes");
        std::fs::write(&file, vec![b'x'; REVIEW_LIMIT + 1]).unwrap();
        assert!(frozen_review(&file).is_err());
        assert!(frozen_review(&directory).is_err());
        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
