//! Framework-neutral diagnostics collection and remediation primitives.
//!
//! This module deliberately contains no UI state or terminal/desktop framework
//! types. A frontend owns refresh scheduling, presentation, confirmation, and
//! cancellation tokens; this module owns the state queries and the narrowly
//! scoped Talos actions that can be safely confirmed and executed.

use crate::{
    constants::{ARGOCD_CRDS, CERT_MANAGER_CRDS, EXTERNAL_SECRETS_CRDS, FLUX_CRDS, KYVERNO_CRDS},
    diagnostics::{
        CheckCategory, CheckStatus, CniInfo, CniPodInfo, CniType, PodHealthInfo, UnhealthyPodInfo,
    },
    errors::format_talos_error,
    formatting::{format_bytes, format_bytes_signed},
};
use k8s_openapi::{
    api::{apps::v1::Deployment, core::v1::Pod},
    apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition,
};
use kube::{
    Client,
    api::{Api, ListParams},
};
use talos_rs::{ApplyMode, TalosClient};

mod evaluate;
use evaluate::*;

/// Result of querying one source of diagnostic state.
///
/// An unavailable source is intentionally distinct from a negative result. For
/// example, a pod API permission error is not evidence that no unhealthy pods
/// exist.
#[derive(Debug, Clone)]
pub enum SourceState<T> {
    /// The source returned an authoritative value.
    Available(T),
    /// The source could not be queried or did not return the requested value.
    Unavailable(SourceUnavailable),
}

impl<T> SourceState<T> {
    /// Constructs an unavailable source result with a displayable cause.
    pub fn unavailable(source: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Unavailable(SourceUnavailable {
            source: source.into(),
            reason: reason.into(),
        })
    }

    /// Returns the authoritative value when it is available.
    pub fn as_ref(&self) -> Option<&T> {
        match self {
            Self::Available(value) => Some(value),
            Self::Unavailable(_) => None,
        }
    }

    /// Returns the source failure when it is unavailable.
    pub fn unavailable_info(&self) -> Option<&SourceUnavailable> {
        match self {
            Self::Available(_) => None,
            Self::Unavailable(info) => Some(info),
        }
    }

    /// Whether the source returned an authoritative value.
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }
}

/// Why an otherwise optional diagnostic source could not be queried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceUnavailable {
    /// Human-readable name of the attempted source.
    pub source: String,
    /// Error or missing-prerequisite detail suitable for a UI details pane.
    pub reason: String,
}

impl SourceUnavailable {
    fn details(&self) -> String {
        format!("{} unavailable: {}", self.source, self.reason)
    }
}

/// Identity of the selected node and the Talos configuration it belongs to.
///
/// `node_address` is always used to target Talos requests. Worker-node
/// Kubernetes checks use `control_plane_address`; they never infer an ambient
/// `KUBECONFIG` context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticTarget {
    /// Kubernetes/Talos node name shown to users.
    pub node_name: String,
    /// Explicit Talos node address for all node-local requests.
    pub node_address: String,
    /// Role reported by the caller, such as `controlplane` or `worker`.
    pub node_role: String,
    /// Explicit control-plane address used to obtain a pinned kubeconfig.
    pub control_plane_address: Option<String>,
    /// Caller-owned identity for the Talos configuration/context in use.
    pub config_identity: String,
}

impl DiagnosticTarget {
    /// Builds a target without silently selecting a Talos configuration.
    pub fn new(
        node_name: impl Into<String>,
        node_address: impl Into<String>,
        node_role: impl Into<String>,
        config_identity: impl Into<String>,
    ) -> Self {
        Self {
            node_name: node_name.into(),
            node_address: node_address.into(),
            node_role: node_role.into(),
            control_plane_address: None,
            config_identity: config_identity.into(),
        }
    }

    /// Associates this target with the control plane that serves its kubeconfig.
    pub fn with_control_plane_address(mut self, address: impl Into<String>) -> Self {
        self.control_plane_address = Some(address.into());
        self
    }

    /// Whether the caller identified this node as a control-plane node.
    pub fn is_control_plane(&self) -> bool {
        let role = self.node_role.to_ascii_lowercase();
        role.contains("controlplane") || role.contains("control-plane") || role == "control"
    }

    fn kubeconfig_endpoint(&self) -> Option<&str> {
        self.control_plane_address.as_deref().or_else(|| {
            self.is_control_plane()
                .then_some(self.node_address.as_str())
        })
    }
}

/// Kubernetes access identity used for a diagnostics refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KubernetesAccess {
    /// Talos control-plane node that supplied the kubeconfig.
    pub control_plane_address: String,
    /// The Talos configuration identity supplied with the target.
    pub config_identity: String,
    /// The kubeconfig actually in use, when the caller resolved a selection
    /// (a chosen file, or the Talos fallback after that file was rejected).
    pub source: Option<String>,
    /// Why a selected kubeconfig wasn't used, when it wasn't.
    pub warning: Option<String>,
}

/// Context shared by diagnostics and exposed to a frontend.
#[derive(Debug, Clone)]
pub struct DiagnosticContext {
    /// Explicit selected-node identity.
    pub target: DiagnosticTarget,
    /// Platform returned by the Talos Version API.
    pub platform: SourceState<String>,
    /// CPU count returned by the Talos CPUInfo API.
    pub cpu_count: SourceState<usize>,
    /// Whether a Kubernetes client was built from a pinned Talos kubeconfig.
    pub kubernetes_access: SourceState<KubernetesAccess>,
}

/// A UI-ready diagnostic result.
#[derive(Debug, Clone)]
pub struct DiagnosticCheck {
    /// Stable identifier used for row selection and fix confirmation.
    pub id: String,
    /// Check group used for frontend navigation.
    pub category: CheckCategory,
    /// Display label.
    pub name: String,
    /// Result status.
    pub status: CheckStatus,
    /// Concise status summary.
    pub message: String,
    /// Optional detailed evidence or unavailability reason.
    pub details: Option<String>,
    /// Confirmation-ready remediation, when a safe remediation exists.
    pub fix: Option<DiagnosticFix>,
}

impl DiagnosticCheck {
    /// Creates a passing check.
    pub fn pass(
        id: impl Into<String>,
        category: CheckCategory,
        name: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(id, category, name, CheckStatus::Pass, message)
    }

    /// Creates a warning check.
    pub fn warn(
        id: impl Into<String>,
        category: CheckCategory,
        name: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(id, category, name, CheckStatus::Warn, message)
    }

    /// Creates a failing check.
    pub fn fail(
        id: impl Into<String>,
        category: CheckCategory,
        name: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(id, category, name, CheckStatus::Fail, message)
    }

    /// Creates an unknown check from an unavailable source.
    pub fn unknown(
        id: impl Into<String>,
        category: CheckCategory,
        name: impl Into<String>,
        unavailable: &SourceUnavailable,
    ) -> Self {
        Self::new(id, category, name, CheckStatus::Unknown, "Unknown")
            .with_details(unavailable.details())
    }

    fn new(
        id: impl Into<String>,
        category: CheckCategory,
        name: impl Into<String>,
        status: CheckStatus,
        message: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            category,
            name: name.into(),
            status,
            message: message.into(),
            details: None,
            fix: None,
        }
    }

    /// Adds frontend-displayable evidence.
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    /// Adds a remediation action for this check.
    pub fn with_fix(mut self, fix: DiagnosticFix) -> Self {
        self.fix = Some(fix);
        self
    }
}

/// A remediation described by a diagnostic check.
#[derive(Debug, Clone)]
pub struct DiagnosticFix {
    /// User-facing explanation of the intended remediation.
    pub description: String,
    /// The only action that may be executed after confirmation.
    pub action: DiagnosticFixAction,
}

impl DiagnosticFix {
    fn kernel_module(module: &str) -> Self {
        Self {
            description: format!("Add {module} kernel module"),
            action: DiagnosticFixAction::ApplyConfigPatch {
                yaml: format!("machine:\n  kernel:\n    modules:\n      - name: {module}"),
                requires_reboot: true,
            },
        }
    }
}

/// Remediation actions supported by the GUI diagnostics screen.
///
/// There is intentionally no host-command execution and no Cilium installer.
/// Guidance that needs an operator action is copy-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticFixAction {
    /// Restart a Talos service on the explicitly selected node.
    RestartService { service_id: String },
    /// Validate then apply a Talos machine configuration patch.
    ApplyConfigPatch {
        /// Patch YAML sent directly to Talos; no temporary host file is used.
        yaml: String,
        /// Whether applying the patch requests an immediate reboot.
        requires_reboot: bool,
    },
    /// Text that a UI may copy for an operator, but must never execute.
    CopyGuidance { title: String, text: String },
}

impl DiagnosticFixAction {
    /// Whether the action asks Talos to reboot the selected node.
    pub fn requires_reboot(&self) -> bool {
        matches!(
            self,
            Self::ApplyConfigPatch {
                requires_reboot: true,
                ..
            }
        )
    }
}

/// A fix selected by a frontend but not yet confirmed.
#[derive(Debug, Clone)]
pub struct DiagnosticFixRequest {
    /// Check that supplied the remediation.
    pub check_id: String,
    /// Node identity retained through confirmation and execution.
    pub target: DiagnosticTarget,
    /// Selected remediation.
    pub fix: DiagnosticFix,
}

impl DiagnosticFixRequest {
    /// Creates a request tied to the selected node.
    pub fn new(check_id: impl Into<String>, target: DiagnosticTarget, fix: DiagnosticFix) -> Self {
        Self {
            check_id: check_id.into(),
            target,
            fix,
        }
    }

    /// Marks the request as confirmed for execution.
    pub fn confirm(self) -> ConfirmedDiagnosticFix {
        ConfirmedDiagnosticFix { request: self }
    }
}

/// A request the caller has explicitly confirmed.
#[derive(Debug, Clone)]
pub struct ConfirmedDiagnosticFix {
    request: DiagnosticFixRequest,
}

/// Result of a completed, cancelled, or copy-only fix request.
#[derive(Debug, Clone)]
pub enum FixExecution {
    /// Talos accepted the remediation request.
    Applied {
        /// Results mapped from Talos responses for frontend display.
        results: Vec<FixTargetResult>,
        /// Whether the applied patch requested a reboot.
        requires_reboot: bool,
    },
    /// Caller cancelled before the next network step, so no further request ran.
    Cancelled {
        /// Whether the original request would have rebooted the node.
        requires_reboot: bool,
    },
    /// Copy-only guidance deliberately caused no network or host action.
    CopyOnly {
        /// Guidance text a frontend may offer to copy.
        text: String,
    },
}

/// A frontend-friendly result from a Talos mutation RPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixTargetResult {
    /// Service response or configuration apply-mode detail.
    pub message: String,
}

/// Failure to execute a confirmed fix.
#[derive(Debug, thiserror::Error)]
pub enum DiagnosticFixError {
    /// The preflight patch validation failed, so the patch was not applied.
    #[error("Talos rejected configuration patch validation: {0}")]
    Validation(String),
    /// Talos rejected or could not complete the requested mutation.
    #[error("Talos action failed: {0}")]
    Execution(String),
}

/// Executes a confirmed fix against its explicitly selected node.
///
/// Configuration patches are first validated through Talos with `dry_run` and
/// only applied after a second cancellation check. A cancellation predicate is
/// checked before every network step; it never attempts to interrupt an
/// already-started RPC.
pub async fn execute_confirmed_fix<F>(
    selected_node_client: &TalosClient,
    confirmed: ConfirmedDiagnosticFix,
    is_cancelled: &mut F,
) -> Result<FixExecution, DiagnosticFixError>
where
    F: FnMut() -> bool,
{
    let request = confirmed.request;
    let targeted_client = selected_node_client.with_node(&request.target.node_address);
    let requires_reboot = request.fix.action.requires_reboot();

    match request.fix.action {
        DiagnosticFixAction::CopyGuidance { text, .. } => Ok(FixExecution::CopyOnly { text }),
        DiagnosticFixAction::RestartService { service_id } => {
            if is_cancelled() {
                return Ok(FixExecution::Cancelled { requires_reboot });
            }

            let results = targeted_client
                .service_restart(&service_id)
                .await
                .map_err(|error| DiagnosticFixError::Execution(format_talos_error(&error)))?
                .into_iter()
                .map(|result| FixTargetResult {
                    message: result.response,
                })
                .collect();

            Ok(FixExecution::Applied {
                results,
                requires_reboot,
            })
        }
        DiagnosticFixAction::ApplyConfigPatch {
            yaml,
            requires_reboot,
        } => {
            let mode = if requires_reboot {
                ApplyMode::Reboot
            } else {
                ApplyMode::Auto
            };

            execute_config_patch(
                |dry_run| targeted_client.apply_configuration(&yaml, mode, dry_run),
                requires_reboot,
                is_cancelled,
            )
            .await
        }
    }
}

/// The injected RPC keeps the validation/apply sequencing testable without
/// invoking a mutation against a real node.
async fn execute_config_patch<Apply, ApplyFuture, Cancel>(
    mut apply: Apply,
    requires_reboot: bool,
    is_cancelled: &mut Cancel,
) -> Result<FixExecution, DiagnosticFixError>
where
    Apply: FnMut(bool) -> ApplyFuture,
    ApplyFuture: Future<Output = Result<Vec<talos_rs::ApplyConfigResult>, talos_rs::TalosError>>,
    Cancel: FnMut() -> bool,
{
    if is_cancelled() {
        return Ok(FixExecution::Cancelled { requires_reboot });
    }

    apply(true)
        .await
        .map_err(|error| DiagnosticFixError::Validation(format_talos_error(&error)))?;

    if is_cancelled() {
        return Ok(FixExecution::Cancelled { requires_reboot });
    }

    let results = apply(false)
        .await
        .map_err(|error| DiagnosticFixError::Execution(format_talos_error(&error)))?
        .into_iter()
        .map(|result| FixTargetResult {
            message: result.mode_result,
        })
        .collect();

    Ok(FixExecution::Applied {
        results,
        requires_reboot,
    })
}

/// Memory statistics collected from Talos `/proc/meminfo` state.
#[derive(Debug, Clone)]
pub struct MemorySnapshot {
    /// Total memory in bytes.
    pub total_bytes: u64,
    /// Available memory in bytes.
    pub available_bytes: u64,
    /// Used memory in bytes, calculated as total minus available.
    pub used_bytes: u64,
    /// Used-memory percentage.
    pub usage_percent: f32,
}

/// Load average values collected from the Talos LoadAvg API.
#[derive(Debug, Clone)]
pub struct LoadAverageSnapshot {
    /// One-minute load average.
    pub one_minute: f64,
    /// Five-minute load average.
    pub five_minutes: f64,
    /// Fifteen-minute load average.
    pub fifteen_minutes: f64,
}

/// Node-local system state used by system diagnostic checks.
#[derive(Debug, Clone)]
pub struct SystemSnapshot {
    /// Memory state from Talos.
    pub memory: SourceState<MemorySnapshot>,
    /// Load state from Talos.
    pub load_average: SourceState<LoadAverageSnapshot>,
}

/// Service health state reported by Talos.
#[derive(Debug, Clone)]
pub struct ServiceHealthSnapshot {
    /// Whether Talos marked the service healthy.
    pub healthy: bool,
    /// Talos health detail, when supplied.
    pub last_message: String,
}

/// A service returned by Talos ServiceList.
#[derive(Debug, Clone)]
pub struct ServiceSnapshot {
    /// Node that reported the service.
    pub node: String,
    /// Talos service identifier.
    pub id: String,
    /// Talos service state string.
    pub state: String,
    /// Health field returned by Talos, or explicitly unavailable when omitted.
    pub health: SourceState<ServiceHealthSnapshot>,
}

/// Service-list snapshot.
#[derive(Debug, Clone)]
pub struct ServicesSnapshot {
    /// Services flattened from the selected node's ServiceList response.
    pub services: SourceState<Vec<ServiceSnapshot>>,
}

/// Etcd state for the selected control-plane node.
#[derive(Debug, Clone)]
pub enum EtcdSnapshot {
    /// The selected node is not a control plane, so etcd does not apply.
    NotApplicable,
    /// Etcd status returned by the targeted Talos API request.
    Available(EtcdStatusSnapshot),
    /// The status request could not establish current etcd state.
    Unavailable(SourceUnavailable),
}

/// Etcd member state exposed for a GUI details panel.
#[derive(Debug, Clone)]
pub struct EtcdStatusSnapshot {
    /// Node that returned the status.
    pub node: String,
    /// Reporting etcd member ID.
    pub member_id: u64,
    /// Current leader member ID.
    pub leader_id: u64,
    /// Whether the reporting member is leader.
    pub is_leader: bool,
    /// Protocol version.
    pub protocol_version: String,
    /// Total etcd database size in bytes.
    pub db_size_bytes: i64,
    /// Used etcd database size in bytes.
    pub db_size_in_use_bytes: i64,
    /// Raft index.
    pub raft_index: u64,
    /// Raft term.
    pub raft_term: u64,
    /// Applied raft index.
    pub raft_applied_index: u64,
    /// Errors reported by the member itself.
    pub errors: Vec<String>,
    /// Whether the member is a learner.
    pub is_learner: bool,
}

/// Kubernetes state collected through a Talos-pinned kubeconfig.
#[derive(Debug, Clone)]
pub struct KubernetesSnapshot {
    /// Cluster-wide pod health from Kubernetes API state.
    pub pod_health: SourceState<PodHealthInfo>,
}

/// A direct file-state probe used by CNI checks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FileProbe {
    /// The probe was irrelevant to the detected provider and was not sent.
    #[default]
    NotChecked,
    /// The path was read successfully.
    Present,
    /// Talos explicitly reported that the path does not exist.
    Missing,
    /// Talos could not distinguish file absence from an unavailable source.
    Unavailable(SourceUnavailable),
}

impl FileProbe {
    fn unavailable_info(&self) -> Option<&SourceUnavailable> {
        match self {
            Self::Unavailable(info) => Some(info),
            Self::NotChecked | Self::Present | Self::Missing => None,
        }
    }
}

/// File evidence used to detect and validate CNI state.
#[derive(Debug, Clone, Default)]
pub struct CniFileEvidence {
    /// `/run/flannel/subnet.env` state.
    pub flannel_subnet: FileProbe,
    /// Whether the Flannel subnet file contains the required network values.
    pub flannel_subnet_valid: Option<bool>,
    /// Cilium conflist state.
    pub cilium_config: FileProbe,
    /// Calico conflist state.
    pub calico_config: FileProbe,
    /// `/etc/cni/net.d` state.
    pub cni_directory: FileProbe,
    /// Whether the CNI directory returned non-empty content.
    pub generic_config_present: Option<bool>,
    /// br_netfilter sysctl file state.
    pub br_netfilter: FileProbe,
}

/// CNI state from Kubernetes and node filesystem sources.
#[derive(Debug, Clone)]
pub struct CniSnapshot {
    /// Provider detected through Kubernetes pods or direct CNI file state.
    pub cni_type: SourceState<CniType>,
    /// CNI pod state from the Kubernetes API.
    pub pods: SourceState<CniInfo>,
    /// Node-local CNI file evidence.
    pub files: CniFileEvidence,
}

/// Presence state for a supported Kubernetes addon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddonPresence {
    /// Direct Kubernetes state identified the addon.
    Detected,
    /// All relevant direct sources were queried and found no addon.
    NotDetected,
    /// At least one necessary source was unavailable and no source found it.
    Unknown,
}

/// A supported addon and the state of its discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddonStatus {
    /// Stable identifier.
    pub id: &'static str,
    /// User-facing addon name.
    pub name: &'static str,
    /// Discovery outcome.
    pub presence: AddonPresence,
}

/// Direct pod-list evidence used for namespace-based addon detection.
#[derive(Debug, Clone)]
pub struct AddonPodSource {
    /// Namespace queried from the Kubernetes API.
    pub namespace: String,
    /// Pod names returned by the source, or why it was unavailable.
    pub pods: SourceState<Vec<String>>,
}

/// Kubernetes addon discovery state.
#[derive(Debug, Clone)]
pub struct AddonSnapshot {
    /// CRD names returned by the Kubernetes API.
    pub crd_names: SourceState<Vec<String>>,
    /// Namespace pod checks that supplement CRD detection.
    pub pod_sources: Vec<AddonPodSource>,
    /// Supported addon presence states.
    pub addons: Vec<AddonStatus>,
}

impl AddonSnapshot {
    /// Returns the presence state for one supported addon.
    pub fn presence(&self, id: &str) -> Option<AddonPresence> {
        self.addons
            .iter()
            .find(|addon| addon.id == id)
            .map(|addon| addon.presence)
    }

    /// Returns names for addons directly detected as installed.
    pub fn detected_names(&self) -> Vec<&'static str> {
        self.addons
            .iter()
            .filter(|addon| addon.presence == AddonPresence::Detected)
            .map(|addon| addon.name)
            .collect()
    }

    fn has_unknown_sources(&self) -> bool {
        !self.crd_names.is_available()
            || self
                .pod_sources
                .iter()
                .any(|source| !source.pods.is_available())
    }
}

/// Complete diagnostics data for one selected node refresh.
#[derive(Debug, Clone)]
pub struct DiagnosticSnapshot {
    /// Selected node and collected access/context state.
    pub context: DiagnosticContext,
    /// Node system state.
    pub system: SystemSnapshot,
    /// Talos service state.
    pub services: ServicesSnapshot,
    /// Etcd state when applicable to the selected node.
    pub etcd: EtcdSnapshot,
    /// Kubernetes workload state.
    pub kubernetes: KubernetesSnapshot,
    /// CNI state and direct file evidence.
    pub cni: CniSnapshot,
    /// Supported addon discovery state.
    pub addons: AddonSnapshot,
    /// UI-ready checks derived exclusively from the snapshots above.
    pub checks: Vec<DiagnosticCheck>,
}

/// Collector for a selected node's framework-neutral diagnostic snapshot.
#[derive(Debug, Clone)]
pub struct DiagnosticCollector {
    target: DiagnosticTarget,
}

impl DiagnosticCollector {
    /// Creates a collector bound to an explicit selected node.
    pub fn new(target: DiagnosticTarget) -> Self {
        Self { target }
    }

    /// Returns the immutable selected-node identity.
    pub fn target(&self) -> &DiagnosticTarget {
        &self.target
    }

    /// Collects diagnostics from the supplied selected-node Talos client.
    ///
    /// Every Talos node-local call is re-targeted to `target.node_address`.
    /// Kubernetes access is built only with
    /// [`crate::cluster_overview::create_k8s_client_from_node`], which fetches
    /// a kubeconfig from the explicit control plane and never consults ambient
    /// `KUBECONFIG`.
    pub async fn collect(&self, selected_node_client: &TalosClient) -> DiagnosticSnapshot {
        self.collect_internal(selected_node_client, None).await
    }

    /// Collects using an already identity-validated, caller-selected Kubernetes
    /// client, or the exact reason that selected access was unavailable.
    ///
    /// Unlike [`Self::collect`], this method never fetches a replacement
    /// kubeconfig or falls back to an ambient Kubernetes configuration. The
    /// caller must validate the supplied client against the selected Talos
    /// cluster before supplying it.
    pub async fn collect_with_kubernetes(
        &self,
        selected_node_client: &TalosClient,
        kubernetes: Result<(Client, KubernetesAccess), SourceUnavailable>,
    ) -> DiagnosticSnapshot {
        self.collect_internal(selected_node_client, Some(kubernetes))
            .await
    }

    async fn collect_internal(
        &self,
        selected_node_client: &TalosClient,
        kubernetes: Option<Result<(Client, KubernetesAccess), SourceUnavailable>>,
    ) -> DiagnosticSnapshot {
        let node_client = selected_node_client.with_node(&self.target.node_address);

        let platform = collect_platform(&node_client).await;
        let cpu_count = collect_cpu_count(&node_client).await;
        let system = collect_system(&node_client).await;
        let services = collect_services(&node_client).await;
        let etcd = collect_etcd(&node_client, &self.target).await;

        let uses_pinned_client = kubernetes.is_none();
        let (kubernetes_access, k8s_client) = match kubernetes {
            Some(access) => selected_kubernetes_access(access),
            None => self.create_pinned_k8s_client(&node_client).await,
        };
        let kubernetes = collect_kubernetes(k8s_client.as_ref(), &kubernetes_access).await;
        if uses_pinned_client
            && k8s_client.is_some()
            && matches!(kubernetes.pod_health, SourceState::Unavailable(_))
        {
            // The reused client failed its request; rebuild it next time.
            if let Some(address) = self.target.kubeconfig_endpoint() {
                crate::cluster_overview::forget_cached_k8s_client_from_node(&node_client, address);
            }
        }
        let cni = collect_cni(&node_client, k8s_client.as_ref(), &kubernetes_access).await;
        let addons = collect_addons(k8s_client.as_ref(), &kubernetes_access).await;

        let context = DiagnosticContext {
            target: self.target.clone(),
            platform,
            cpu_count,
            kubernetes_access,
        };

        let mut checks = build_system_checks(&context, &system);
        checks.extend(build_service_checks(&services));
        checks.extend(build_etcd_checks(&etcd));
        checks.extend(build_kubernetes_checks(&context, &kubernetes));
        checks.extend(build_cni_checks(&context, &cni, k8s_client.as_ref()).await);
        checks.extend(build_addon_checks(&addons, k8s_client.as_ref()).await);

        DiagnosticSnapshot {
            context,
            system,
            services,
            etcd,
            kubernetes,
            cni,
            addons,
            checks,
        }
    }

    async fn create_pinned_k8s_client(
        &self,
        node_client: &TalosClient,
    ) -> (SourceState<KubernetesAccess>, Option<Client>) {
        let Some(control_plane_address) = self.target.kubeconfig_endpoint() else {
            let unavailable = SourceState::unavailable(
                "Kubernetes kubeconfig",
                "No control-plane address was supplied for this non-control-plane node",
            );
            return (unavailable, None);
        };

        match crate::cluster_overview::create_k8s_client_from_node_cached(
            node_client,
            control_plane_address,
        )
        .await
        {
            Ok(client) => (
                SourceState::Available(KubernetesAccess {
                    control_plane_address: control_plane_address.to_string(),
                    config_identity: self.target.config_identity.clone(),
                    source: None,
                    warning: None,
                }),
                Some(client),
            ),
            Err(error) => (
                SourceState::unavailable("Kubernetes kubeconfig", error.to_string()),
                None,
            ),
        }
    }
}

fn selected_kubernetes_access(
    access: Result<(Client, KubernetesAccess), SourceUnavailable>,
) -> (SourceState<KubernetesAccess>, Option<Client>) {
    match access {
        Ok((client, identity)) => (SourceState::Available(identity), Some(client)),
        Err(reason) => (SourceState::Unavailable(reason), None),
    }
}

async fn collect_platform(client: &TalosClient) -> SourceState<String> {
    match client.version().await {
        Ok(versions) => versions.first().map_or_else(
            || SourceState::unavailable("Talos Version API", "Talos returned no version data"),
            |version| SourceState::Available(version.platform.clone()),
        ),
        Err(error) => SourceState::unavailable("Talos Version API", format_talos_error(&error)),
    }
}

async fn collect_cpu_count(client: &TalosClient) -> SourceState<usize> {
    match client.cpu_info().await {
        Ok(cpus) => cpus.first().map_or_else(
            || SourceState::unavailable("Talos CPUInfo API", "Talos returned no CPU information"),
            |cpu| {
                if cpu.cpu_count == 0 {
                    SourceState::unavailable(
                        "Talos CPUInfo API",
                        "Talos returned an invalid CPU count of zero",
                    )
                } else {
                    SourceState::Available(cpu.cpu_count)
                }
            },
        ),
        Err(error) => SourceState::unavailable("Talos CPUInfo API", format_talos_error(&error)),
    }
}

async fn collect_system(client: &TalosClient) -> SystemSnapshot {
    let memory = match client.memory().await {
        Ok(memories) => memories
            .first()
            .and_then(|memory| memory.meminfo.as_ref())
            .map_or_else(
                || {
                    SourceState::unavailable(
                        "Talos Memory API",
                        "Talos returned no memory information",
                    )
                },
                |memory| {
                    let used_bytes = memory.mem_total.saturating_sub(memory.mem_available);
                    SourceState::Available(MemorySnapshot {
                        total_bytes: memory.mem_total,
                        available_bytes: memory.mem_available,
                        used_bytes,
                        usage_percent: memory.usage_percent(),
                    })
                },
            ),
        Err(error) => SourceState::unavailable("Talos Memory API", format_talos_error(&error)),
    };

    let load_average = match client.load_avg().await {
        Ok(loads) => loads.first().map_or_else(
            || SourceState::unavailable("Talos LoadAvg API", "Talos returned no load averages"),
            |load| {
                SourceState::Available(LoadAverageSnapshot {
                    one_minute: load.load1,
                    five_minutes: load.load5,
                    fifteen_minutes: load.load15,
                })
            },
        ),
        Err(error) => SourceState::unavailable("Talos LoadAvg API", format_talos_error(&error)),
    };

    SystemSnapshot {
        memory,
        load_average,
    }
}

async fn collect_services(client: &TalosClient) -> ServicesSnapshot {
    let services = match client.services().await {
        Ok(nodes) => {
            let services = nodes
                .into_iter()
                .flat_map(|node| {
                    let node_name = node.node;
                    node.services
                        .into_iter()
                        .map(move |service| ServiceSnapshot {
                            node: node_name.clone(),
                            id: service.id,
                            state: service.state,
                            health: service_health_source(service.health),
                        })
                })
                .collect();
            SourceState::Available(services)
        }
        Err(error) => SourceState::unavailable("Talos ServiceList API", format_talos_error(&error)),
    };

    ServicesSnapshot { services }
}

fn service_health_source(
    health: Option<talos_rs::ServiceHealth>,
) -> SourceState<ServiceHealthSnapshot> {
    match health {
        None => SourceState::unavailable(
            "Talos service health",
            "Talos did not return a health state for this service",
        ),
        Some(health) if health.unknown => SourceState::unavailable(
            "Talos service health",
            if health.last_message.is_empty() {
                "Talos reports service health as unknown".to_string()
            } else {
                format!(
                    "Talos reports service health as unknown: {}",
                    health.last_message
                )
            },
        ),
        Some(health) => SourceState::Available(ServiceHealthSnapshot {
            healthy: health.healthy,
            last_message: health.last_message,
        }),
    }
}

async fn collect_etcd(client: &TalosClient, target: &DiagnosticTarget) -> EtcdSnapshot {
    if !target.is_control_plane() {
        return EtcdSnapshot::NotApplicable;
    }

    match client
        .etcd_status_for_nodes(std::slice::from_ref(&target.node_address))
        .await
    {
        Ok(statuses) => match statuses.into_iter().next() {
            Some(status) => {
                let is_leader = status.is_leader();
                EtcdSnapshot::Available(EtcdStatusSnapshot {
                    node: status.node,
                    member_id: status.member_id,
                    leader_id: status.leader_id,
                    is_leader,
                    protocol_version: status.protocol_version,
                    db_size_bytes: status.db_size,
                    db_size_in_use_bytes: status.db_size_in_use,
                    raft_index: status.raft_index,
                    raft_term: status.raft_term,
                    raft_applied_index: status.raft_applied_index,
                    errors: status.errors,
                    is_learner: status.is_learner,
                })
            }
            None => EtcdSnapshot::Unavailable(SourceUnavailable {
                source: "Talos EtcdStatus API".to_string(),
                reason: "Talos returned no etcd member status".to_string(),
            }),
        },
        Err(error) => EtcdSnapshot::Unavailable(SourceUnavailable {
            source: "Talos EtcdStatus API".to_string(),
            reason: format_talos_error(&error),
        }),
    }
}

async fn collect_kubernetes(
    client: Option<&Client>,
    access: &SourceState<KubernetesAccess>,
) -> KubernetesSnapshot {
    let pod_health = match client {
        Some(client) => match collect_pod_health(client).await {
            Ok(health) => SourceState::Available(health),
            Err(error) => SourceState::unavailable("Kubernetes Pod API", error),
        },
        None => unavailable_from_access(access, "Kubernetes Pod API"),
    };

    KubernetesSnapshot { pod_health }
}

async fn collect_pod_health(client: &Client) -> Result<PodHealthInfo, String> {
    let pods: Api<Pod> = Api::all(client.clone());
    let pod_list = pods
        .list(&ListParams::default())
        .await
        .map_err(|error| error.to_string())?;

    let mut health = PodHealthInfo {
        total_pods: pod_list.items.len(),
        ..Default::default()
    };

    for pod in pod_list.items {
        let name = pod.metadata.name.unwrap_or_default();
        let namespace = pod.metadata.namespace.unwrap_or_default();
        let status = pod.status.as_ref();

        if let Some(container_statuses) =
            status.and_then(|status| status.container_statuses.as_ref())
        {
            for container in container_statuses {
                let Some(waiting) = container
                    .state
                    .as_ref()
                    .and_then(|state| state.waiting.as_ref())
                else {
                    continue;
                };
                let reason = waiting.reason.clone().unwrap_or_default();
                let unhealthy = UnhealthyPodInfo {
                    name: name.clone(),
                    namespace: namespace.clone(),
                    state: reason.clone(),
                    restart_count: container.restart_count,
                };

                match reason.as_str() {
                    "CrashLoopBackOff" => health.crashing.push(unhealthy),
                    "ImagePullBackOff" | "ErrImagePull" => health.image_pull_errors.push(unhealthy),
                    _ => {}
                }
            }
        }
    }

    Ok(health)
}

async fn collect_cni(
    node_client: &TalosClient,
    k8s_client: Option<&Client>,
    access: &SourceState<KubernetesAccess>,
) -> CniSnapshot {
    let pods = match k8s_client {
        Some(client) => match collect_cni_info(client).await {
            Ok(info) => SourceState::Available(info),
            Err(error) => SourceState::unavailable("Kubernetes CNI pod API", error),
        },
        None => unavailable_from_access(access, "Kubernetes CNI pod API"),
    };

    let detected_from_pods = pods
        .as_ref()
        .map(|info| info.cni_type.clone())
        .unwrap_or(CniType::Unknown);
    let mut files = CniFileEvidence::default();

    let cni_type = match detected_from_pods {
        CniType::Flannel => {
            populate_flannel_file_evidence(node_client, &mut files).await;
            SourceState::Available(CniType::Flannel)
        }
        CniType::Cilium => SourceState::Available(CniType::Cilium),
        CniType::Calico => SourceState::Available(CniType::Calico),
        CniType::None | CniType::Unknown => detect_cni_from_files(node_client, &mut files).await,
    };

    CniSnapshot {
        cni_type,
        pods,
        files,
    }
}

async fn collect_cni_info(client: &Client) -> Result<CniInfo, String> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), "kube-system");
    let pod_list = pods
        .list(&ListParams::default())
        .await
        .map_err(|error| error.to_string())?;

    let mut cni_info = CniInfo::default();
    for pod in pod_list.items {
        let name = pod.metadata.name.clone().unwrap_or_default();
        let lower_name = name.to_ascii_lowercase();
        let detected =
            if lower_name.starts_with("kube-flannel") || lower_name.starts_with("flannel") {
                cni_info.cni_type = CniType::Flannel;
                true
            } else if lower_name.starts_with("cilium") {
                cni_info.cni_type = CniType::Cilium;
                true
            } else if lower_name.starts_with("calico") || lower_name.starts_with("calico-node") {
                cni_info.cni_type = CniType::Calico;
                true
            } else {
                false
            };

        if !detected {
            continue;
        }

        let status = pod.status.as_ref();
        let phase = status
            .and_then(|status| status.phase.clone())
            .unwrap_or_else(|| "Unknown".to_string());
        let ready = status
            .and_then(|status| status.conditions.as_ref())
            .is_some_and(|conditions| {
                conditions
                    .iter()
                    .any(|condition| condition.type_ == "Ready" && condition.status == "True")
            });
        let restart_count = status
            .and_then(|status| status.container_statuses.as_ref())
            .map(|containers| {
                containers
                    .iter()
                    .map(|container| container.restart_count)
                    .sum()
            })
            .unwrap_or(0);
        let node_name = pod.spec.as_ref().and_then(|spec| spec.node_name.clone());

        cni_info.pods.push(CniPodInfo {
            name,
            node_name,
            phase,
            ready,
            restart_count,
        });
    }

    Ok(cni_info)
}

async fn populate_flannel_file_evidence(client: &TalosClient, evidence: &mut CniFileEvidence) {
    let (subnet, content) = probe_file(client, "/run/flannel/subnet.env").await;
    evidence.flannel_subnet = subnet;
    evidence.flannel_subnet_valid = content
        .map(|content| content.contains("FLANNEL_NETWORK=") && content.contains("FLANNEL_SUBNET="));
    evidence.br_netfilter = probe_file(client, "/proc/sys/net/bridge/bridge-nf-call-iptables")
        .await
        .0;
}

async fn detect_cni_from_files(
    client: &TalosClient,
    evidence: &mut CniFileEvidence,
) -> SourceState<CniType> {
    populate_flannel_file_evidence(client, evidence).await;
    if evidence.flannel_subnet == FileProbe::Present {
        return SourceState::Available(CniType::Flannel);
    }

    evidence.cilium_config = probe_file(client, "/etc/cni/net.d/05-cilium.conflist")
        .await
        .0;
    if evidence.cilium_config == FileProbe::Present {
        return SourceState::Available(CniType::Cilium);
    }

    evidence.calico_config = probe_file(client, "/etc/cni/net.d/10-calico.conflist")
        .await
        .0;
    if evidence.calico_config == FileProbe::Present {
        return SourceState::Available(CniType::Calico);
    }

    let (directory, content) = probe_file(client, "/etc/cni/net.d").await;
    evidence.cni_directory = directory;
    evidence.generic_config_present = content.as_ref().map(|content| !content.trim().is_empty());

    if evidence.generic_config_present == Some(true) {
        return SourceState::Available(CniType::Unknown);
    }

    if evidence.flannel_subnet == FileProbe::Missing
        && evidence.cilium_config == FileProbe::Missing
        && evidence.calico_config == FileProbe::Missing
        && evidence.cni_directory == FileProbe::Missing
    {
        return SourceState::Available(CniType::None);
    }

    let unavailable = [
        evidence.flannel_subnet.unavailable_info(),
        evidence.cilium_config.unavailable_info(),
        evidence.calico_config.unavailable_info(),
        evidence.cni_directory.unavailable_info(),
    ]
    .into_iter()
    .flatten()
    .next()
    .cloned()
    .unwrap_or(SourceUnavailable {
        source: "Talos CNI file API".to_string(),
        reason: "Talos did not return enough file state to identify a CNI".to_string(),
    });

    SourceState::Unavailable(unavailable)
}

async fn probe_file(client: &TalosClient, path: &str) -> (FileProbe, Option<String>) {
    match client.read_file(path).await {
        Ok(content) => (FileProbe::Present, Some(content)),
        Err(error) if is_missing_file_error(&error) => (FileProbe::Missing, None),
        Err(error) => (
            FileProbe::Unavailable(SourceUnavailable {
                source: format!("Talos file {path}"),
                reason: format_talos_error(&error),
            }),
            None,
        ),
    }
}

fn is_missing_file_error(error: &talos_rs::TalosError) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("not found") || message.contains("no such file")
}

async fn collect_addons(
    client: Option<&Client>,
    access: &SourceState<KubernetesAccess>,
) -> AddonSnapshot {
    let crd_names = match client {
        Some(client) => collect_crd_names(client).await,
        None => unavailable_from_access(access, "Kubernetes CRD API"),
    };
    let pod_sources = match client {
        Some(client) => collect_addon_pod_sources(client).await,
        None => unavailable_addon_pod_sources(access),
    };
    let addons = derive_addon_statuses(&crd_names, &pod_sources);

    AddonSnapshot {
        crd_names,
        pod_sources,
        addons,
    }
}

async fn collect_crd_names(client: &Client) -> SourceState<Vec<String>> {
    let crds: Api<CustomResourceDefinition> = Api::all(client.clone());
    match crds.list(&ListParams::default()).await {
        Ok(list) => SourceState::Available(
            list.items
                .into_iter()
                .filter_map(|crd| crd.metadata.name)
                .collect(),
        ),
        Err(error) => SourceState::unavailable("Kubernetes CRD API", error.to_string()),
    }
}

async fn collect_addon_pod_sources(client: &Client) -> Vec<AddonPodSource> {
    let mut sources = Vec::with_capacity(5);
    for namespace in ["ingress-nginx", "traefik", "monitoring", "prometheus"] {
        let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
        let state = match pods.list(&ListParams::default().limit(1)).await {
            Ok(list) => SourceState::Available(
                list.items
                    .into_iter()
                    .filter_map(|pod| pod.metadata.name)
                    .collect(),
            ),
            Err(error) => SourceState::unavailable(
                format!("Kubernetes pods in {namespace}"),
                error.to_string(),
            ),
        };
        sources.push(AddonPodSource {
            namespace: namespace.to_string(),
            pods: state,
        });
    }

    let kube_system: Api<Pod> = Api::namespaced(client.clone(), "kube-system");
    let state = match kube_system.list(&ListParams::default()).await {
        Ok(list) => SourceState::Available(
            list.items
                .into_iter()
                .filter_map(|pod| pod.metadata.name)
                .collect(),
        ),
        Err(error) => SourceState::unavailable("Kubernetes pods in kube-system", error.to_string()),
    };
    sources.push(AddonPodSource {
        namespace: "kube-system".to_string(),
        pods: state,
    });

    sources
}

fn unavailable_addon_pod_sources(access: &SourceState<KubernetesAccess>) -> Vec<AddonPodSource> {
    [
        "ingress-nginx",
        "traefik",
        "monitoring",
        "prometheus",
        "kube-system",
    ]
    .into_iter()
    .map(|namespace| AddonPodSource {
        namespace: namespace.to_string(),
        pods: unavailable_from_access(access, format!("Kubernetes pods in {namespace}")),
    })
    .collect()
}

fn derive_addon_statuses(
    crd_names: &SourceState<Vec<String>>,
    pod_sources: &[AddonPodSource],
) -> Vec<AddonStatus> {
    vec![
        crd_addon_status("cert-manager", "cert-manager", CERT_MANAGER_CRDS, crd_names),
        crd_addon_status(
            "external-secrets",
            "external-secrets",
            EXTERNAL_SECRETS_CRDS,
            crd_names,
        ),
        crd_addon_status("kyverno", "kyverno", KYVERNO_CRDS, crd_names),
        pod_addon_status(
            "ingress-nginx",
            "ingress-nginx",
            pod_sources,
            &["ingress-nginx", "kube-system"],
            |source, pod| {
                source.namespace == "ingress-nginx"
                    || pod.to_ascii_lowercase().contains("ingress-nginx")
            },
        ),
        pod_addon_status(
            "traefik",
            "traefik",
            pod_sources,
            &["traefik", "kube-system"],
            |source, pod| {
                source.namespace == "traefik" || pod.to_ascii_lowercase().contains("traefik")
            },
        ),
        pod_addon_status(
            "prometheus",
            "prometheus",
            pod_sources,
            &["monitoring", "prometheus"],
            |_, _| true,
        ),
        crd_addon_status("argocd", "Argo CD", ARGOCD_CRDS, crd_names),
        crd_addon_status("flux", "Flux", FLUX_CRDS, crd_names),
    ]
}

fn crd_addon_status(
    id: &'static str,
    name: &'static str,
    markers: &[&str],
    crd_names: &SourceState<Vec<String>>,
) -> AddonStatus {
    let presence = match crd_names {
        SourceState::Available(crds) => {
            if markers
                .iter()
                .any(|marker| crds.iter().any(|crd| crd == marker))
            {
                AddonPresence::Detected
            } else {
                AddonPresence::NotDetected
            }
        }
        SourceState::Unavailable(_) => AddonPresence::Unknown,
    };

    AddonStatus { id, name, presence }
}

fn pod_addon_status(
    id: &'static str,
    name: &'static str,
    sources: &[AddonPodSource],
    namespaces: &[&str],
    matches_pod: impl Fn(&AddonPodSource, &str) -> bool,
) -> AddonStatus {
    let relevant: Vec<&AddonPodSource> = sources
        .iter()
        .filter(|source| namespaces.contains(&source.namespace.as_str()))
        .collect();
    let detected = relevant.iter().any(|source| {
        source
            .pods
            .as_ref()
            .is_some_and(|pods| pods.iter().any(|pod| matches_pod(source, pod)))
    });
    let all_available = relevant.iter().all(|source| source.pods.is_available());

    AddonStatus {
        id,
        name,
        presence: if detected {
            AddonPresence::Detected
        } else if all_available {
            AddonPresence::NotDetected
        } else {
            AddonPresence::Unknown
        },
    }
}

fn unavailable_from_access<T>(
    access: &SourceState<KubernetesAccess>,
    source: impl Into<String>,
) -> SourceState<T> {
    SourceState::Unavailable(unavailable_info_from_access(access, source))
}

fn unavailable_info_from_access(
    access: &SourceState<KubernetesAccess>,
    source: impl Into<String>,
) -> SourceUnavailable {
    let source = source.into();
    match access {
        SourceState::Available(_) => SourceUnavailable {
            source,
            reason: "The Kubernetes client was not retained for this check".to_string(),
        },
        SourceState::Unavailable(unavailable) => SourceUnavailable {
            source,
            reason: unavailable.details(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> DiagnosticTarget {
        DiagnosticTarget::new("cp-1", "10.0.0.10", "controlplane", "lab-admin")
    }

    #[tokio::test]
    async fn failed_dry_run_never_invokes_actual_apply() {
        let mut calls = Vec::new();
        let result = execute_config_patch(
            |dry_run| {
                calls.push(dry_run);
                std::future::ready(Err(talos_rs::TalosError::Grpc(
                    tonic::Status::permission_denied(
                        "validate configuration on cp-1: proxy denied",
                    ),
                )))
            },
            true,
            &mut || false,
        )
        .await;

        assert_eq!(calls, vec![true]);
        let Err(DiagnosticFixError::Validation(message)) = result else {
            panic!("expected validation failure, never Applied");
        };
        assert!(message.contains("proxy denied"));
    }

    #[tokio::test]
    async fn successful_dry_run_precedes_apply_and_preserves_acknowledgement() {
        let mut calls = Vec::new();
        let result = execute_config_patch(
            |dry_run| {
                calls.push(dry_run);
                std::future::ready(Ok(vec![talos_rs::ApplyConfigResult {
                    node: "cp-1".to_string(),
                    mode_result: if dry_run { "validated" } else { "applied" }.to_string(),
                    warnings: vec!["warning".to_string()],
                }]))
            },
            true,
            &mut || false,
        )
        .await
        .unwrap();

        assert_eq!(calls, vec![true, false]);
        let FixExecution::Applied {
            results,
            requires_reboot,
        } = result
        else {
            panic!("expected applied result");
        };
        assert!(requires_reboot);
        assert_eq!(results[0].message, "applied");
    }

    #[tokio::test]
    async fn failed_actual_apply_is_execution_error_not_accepted() {
        let mut calls = Vec::new();
        let result = execute_config_patch(
            |dry_run| {
                calls.push(dry_run);
                std::future::ready(if dry_run {
                    Ok(vec![talos_rs::ApplyConfigResult {
                        node: "cp-1".to_string(),
                        mode_result: "validated".to_string(),
                        warnings: Vec::new(),
                    }])
                } else {
                    Err(talos_rs::TalosError::Grpc(tonic::Status::internal(
                        "apply configuration on cp-1: proxy failure",
                    )))
                })
            },
            false,
            &mut || false,
        )
        .await;

        assert_eq!(calls, vec![true, false]);
        assert!(matches!(result, Err(DiagnosticFixError::Execution(_))));
    }

    #[tokio::test]
    async fn cancellation_after_validation_prevents_actual_apply() {
        let mut calls = Vec::new();
        let mut cancellation_checks = 0;
        let result = execute_config_patch(
            |dry_run| {
                calls.push(dry_run);
                std::future::ready(Ok(vec![talos_rs::ApplyConfigResult {
                    node: "cp-1".to_string(),
                    mode_result: "validated".to_string(),
                    warnings: Vec::new(),
                }]))
            },
            false,
            &mut || {
                cancellation_checks += 1;
                cancellation_checks == 2
            },
        )
        .await
        .unwrap();

        assert_eq!(calls, vec![true]);
        assert!(matches!(
            result,
            FixExecution::Cancelled {
                requires_reboot: false,
            }
        ));
    }

    #[tokio::test]
    async fn injected_unavailable_access_never_becomes_an_ambient_client() {
        let reason = SourceUnavailable {
            source: "Selected kubeconfig".to_string(),
            reason: "CA identity validation failed".to_string(),
        };
        let (access, client) = selected_kubernetes_access(Err(reason.clone()));
        assert!(client.is_none());
        assert_eq!(access.unavailable_info(), Some(&reason));

        let kubernetes = collect_kubernetes(client.as_ref(), &access).await;
        assert!(
            kubernetes
                .pod_health
                .unavailable_info()
                .unwrap()
                .reason
                .contains(&reason.reason)
        );
        let addons = collect_addons(client.as_ref(), &access).await;
        assert!(addons.crd_names.unavailable_info().is_some());
        assert!(
            addons
                .addons
                .iter()
                .all(|addon| addon.presence == AddonPresence::Unknown)
        );
        let context = DiagnosticContext {
            target: target(),
            platform: SourceState::Available("metal".to_string()),
            cpu_count: SourceState::Available(4),
            kubernetes_access: access,
        };
        let checks = build_kubernetes_checks(&context, &kubernetes);
        let pod_check = checks
            .iter()
            .find(|check| check.id == "pod_health")
            .unwrap();
        assert_eq!(pod_check.status, CheckStatus::Unknown);
        assert!(
            pod_check
                .details
                .as_deref()
                .unwrap()
                .contains(&reason.reason)
        );
    }

    #[test]
    fn kubernetes_api_names_the_kubeconfig_actually_used() {
        let access = |source: Option<&str>, warning: Option<&str>| DiagnosticContext {
            target: target(),
            platform: SourceState::Available("metal".to_string()),
            cpu_count: SourceState::Available(4),
            kubernetes_access: SourceState::Available(KubernetesAccess {
                control_plane_address: "10.0.0.10".to_string(),
                config_identity: "lab-admin".to_string(),
                source: source.map(str::to_string),
                warning: warning.map(str::to_string),
            }),
        };
        let snapshot = KubernetesSnapshot {
            pod_health: SourceState::unavailable("Kubernetes Pod API", "RBAC denied"),
        };
        let api = |context: &DiagnosticContext| {
            build_kubernetes_checks(context, &snapshot)
                .into_iter()
                .find(|check| check.id == "kubernetes_api")
                .expect("api check")
        };

        let pinned = api(&access(None, None));
        assert_eq!(pinned.message, "Pinned kubeconfig from 10.0.0.10");

        let rejected = api(&access(
            Some("Talos control plane 10.0.0.10 (context: admin) — selected file rejected"),
            Some("Selected kubeconfig /k (context: x) was not used: CA mismatch."),
        ));
        assert_eq!(rejected.status, CheckStatus::Pass);
        assert!(rejected.message.ends_with("selected file rejected"));
        assert!(
            rejected
                .details
                .as_deref()
                .is_some_and(|details| details.contains("CA mismatch")),
            "{:?}",
            rejected.details
        );
    }

    #[test]
    fn unavailable_pod_api_is_unknown_not_a_failure() {
        let context = DiagnosticContext {
            target: target(),
            platform: SourceState::Available("metal".to_string()),
            cpu_count: SourceState::Available(4),
            kubernetes_access: SourceState::Available(KubernetesAccess {
                control_plane_address: "10.0.0.10".to_string(),
                config_identity: "lab-admin".to_string(),
                source: None,
                warning: None,
            }),
        };
        let snapshot = KubernetesSnapshot {
            pod_health: SourceState::unavailable("Kubernetes Pod API", "RBAC denied"),
        };

        let checks = build_kubernetes_checks(&context, &snapshot);
        let pod_health = checks
            .iter()
            .find(|check| check.id == "pod_health")
            .expect("pod health check");

        assert_eq!(pod_health.status, CheckStatus::Unknown);
        assert!(pod_health.fix.is_none());
    }

    #[test]
    fn service_without_talos_health_is_unknown_not_unhealthy() {
        let services = ServicesSnapshot {
            services: SourceState::Available(vec![ServiceSnapshot {
                node: "cp-1".to_string(),
                id: "kubelet".to_string(),
                state: "Running".to_string(),
                health: SourceState::unavailable("Talos service health", "field omitted"),
            }]),
        };

        let checks = build_service_checks(&services);

        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, CheckStatus::Unknown);
        assert!(checks[0].fix.is_none());
    }

    #[test]
    fn talos_unknown_service_health_never_infers_restart() {
        for healthy in [false, true] {
            let services = ServicesSnapshot {
                services: SourceState::Available(vec![ServiceSnapshot {
                    node: "cp-1".to_string(),
                    id: "kubelet".to_string(),
                    state: "Running".to_string(),
                    health: service_health_source(Some(talos_rs::ServiceHealth {
                        unknown: true,
                        healthy,
                        last_message: "health probe is unavailable".to_string(),
                    })),
                }]),
            };
            assert!(!services.services.as_ref().unwrap()[0].health.is_available());
            let checks = build_service_checks(&services);
            assert_eq!(checks[0].status, CheckStatus::Unknown);
            assert!(checks[0].fix.is_none());
            assert!(
                checks[0]
                    .details
                    .as_deref()
                    .unwrap()
                    .contains("health probe is unavailable")
            );
        }
    }

    #[test]
    fn authoritative_unhealthy_service_still_offers_restart() {
        let services = ServicesSnapshot {
            services: SourceState::Available(vec![ServiceSnapshot {
                node: "cp-1".to_string(),
                id: "kubelet".to_string(),
                state: "Running".to_string(),
                health: service_health_source(Some(talos_rs::ServiceHealth {
                    unknown: false,
                    healthy: false,
                    last_message: "health probe failed".to_string(),
                })),
            }]),
        };
        let checks = build_service_checks(&services);
        assert_eq!(checks[0].status, CheckStatus::Fail);
        assert!(matches!(
            checks[0].fix.as_ref().map(|fix| &fix.action),
            Some(DiagnosticFixAction::RestartService { service_id }) if service_id == "kubelet"
        ));
    }

    #[test]
    fn missing_file_is_recognized_from_the_error_text_alone() {
        use talos_rs::TalosError;
        use tonic::Status;
        // Every gRPC shape the old category check called not-found carries
        // "not found" in the status message, which the error's Display prints.
        for error in [
            TalosError::Grpc(Status::not_found("file not found")),
            TalosError::Grpc(Status::unknown("open /run/x: No Such File or directory")),
            TalosError::Grpc(Status::permission_denied("denied: path NOT FOUND")),
            TalosError::Grpc(Status::unavailable("peer unavailable, not found in cache")),
        ] {
            assert!(is_missing_file_error(&error), "{error}");
        }
        for error in [
            TalosError::Grpc(Status::unavailable("connection refused")),
            TalosError::Grpc(Status::permission_denied("denied")),
            TalosError::Connection("timeout".into()),
            TalosError::ConfigInvalid("bad".into()),
        ] {
            assert!(!is_missing_file_error(&error), "{error}");
        }
    }
}
