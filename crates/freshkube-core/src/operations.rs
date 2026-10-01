//! Framework-neutral node and rolling-operation orchestration.
//!
//! This module owns operation semantics rather than any frontend state. Callers provide
//! Kubernetes and Talos clients, a confirmation made by their frontend, a cancellation
//! predicate, and a progress callback. Every mutation is audited as structured YAML
//! documents at a caller-selected local path.

use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::{
    Client,
    api::{Api, DeleteParams, EvictParams, ListParams, Patch, PatchParams},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use talos_rs::{RebootMode, TalosClient};
use thiserror::Error;

use crate::indicators::{QuorumState, SafetyStatus};
use crate::inspection::EtcdHealthSnapshot;

/// An explicit Kubernetes/Talos target. Kubernetes mutations use [`Self::name`] and Talos
/// mutations use [`Self::address`]; callers must not infer one from the other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeTarget {
    /// Kubernetes node name.
    pub name: String,
    /// Talos API node address.
    pub address: String,
}

impl NodeTarget {
    /// Create an explicit node target.
    pub fn new(name: impl Into<String>, address: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            address: address.into(),
        }
    }

    fn is_valid(&self) -> bool {
        !self.name.trim().is_empty() && !self.address.trim().is_empty()
    }
}

/// A frontend's explicit confirmation for a mutating operation.
///
/// The core cannot render confirmation UI. Requiring this value makes the handoff from a
/// frontend confirmation step explicit and prevents a runner from having a permissive default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationConfirmation {
    confirmed: bool,
}

impl OperationConfirmation {
    /// Construct confirmation after the frontend has obtained user approval.
    pub const fn confirmed() -> Self {
        Self { confirmed: true }
    }

    /// Construct a rejected confirmation, useful when a frontend dismisses a dialog.
    pub const fn rejected() -> Self {
        Self { confirmed: false }
    }

    /// Whether the frontend approved the action.
    pub const fn is_confirmed(self) -> bool {
        self.confirmed
    }
}

/// Caller-owned cooperative cancellation check.
pub type CancellationPredicate = dyn Fn() -> bool + Send + Sync;

/// Types of node mutations supported by the core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// Prevent new workload scheduling on a node.
    Cordon,
    /// Allow workload scheduling on a node.
    Uncordon,
    /// Cordon a node and evict eligible workloads.
    Drain,
    /// Drain a node and request a Talos reboot.
    Reboot,
    /// Drain a node and request a graceful Talos shutdown.
    Shutdown,
}

impl OperationKind {
    /// Human-readable operation name.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cordon => "cordon",
            Self::Uncordon => "uncordon",
            Self::Drain => "drain",
            Self::Reboot => "reboot",
            Self::Shutdown => "shutdown",
        }
    }

    /// Whether taking the node down can reduce etcd quorum.
    pub const fn is_destructive(self) -> bool {
        matches!(self, Self::Reboot | Self::Shutdown)
    }
}

/// Result state of an operation. It always describes the observed outcome rather than relying
/// on an absent error to imply success.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    /// Every requested step completed.
    Succeeded,
    /// A request or verification step failed.
    Failed,
    /// Cancellation was observed before the next requested mutation.
    Cancelled,
    /// Safety evaluation blocked the operation before mutation.
    Blocked,
    /// The required frontend confirmation was absent.
    NotConfirmed,
    /// The configured client cannot perform the requested action.
    Unsupported,
    /// The node was intentionally not started by a rolling runner.
    NotStarted,
}

impl OperationStatus {
    /// Whether this represents a fully successful operation.
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Succeeded)
    }
}

/// Kubernetes scheduling state as observed or changed by an operation.
///
/// In particular, a failed compensating uncordon is never reported as restored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "details")]
pub enum SchedulingState {
    /// No scheduling mutation was made by this operation.
    Unchanged,
    /// The node was already unschedulable before this operation.
    AlreadyCordoned,
    /// This operation set the node unschedulable.
    CordonedByOperation,
    /// Kubernetes reported the node as schedulable after an uncordon.
    Schedulable,
    /// Kubernetes did not provide enough evidence to state the current scheduling state.
    Unknown(String),
}

/// How strongly the etcd facts supporting an operation are known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "source", content = "details")]
pub enum EtcdQuorumImpact {
    /// Facts obtained from the etcd/Talos API for this exact target.
    Known {
        /// Whether the target belongs to the etcd membership.
        target_is_member: bool,
        /// Whether the target is currently counted as healthy by etcd.
        target_is_healthy: bool,
        /// Number of healthy etcd members.
        healthy_members: usize,
        /// Total etcd membership size.
        total_members: usize,
    },
    /// The required etcd source was unavailable or incomplete.
    Unavailable { reason: String },
}

impl EtcdQuorumImpact {
    /// Build a snapshot from counts obtained from the etcd API.
    pub const fn known(
        target_is_member: bool,
        target_is_healthy: bool,
        healthy_members: usize,
        total_members: usize,
    ) -> Self {
        Self::Known {
            target_is_member,
            target_is_healthy,
            healthy_members,
            total_members,
        }
    }

    /// Build an explicitly unavailable snapshot instead of treating unavailable state as safe.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }

    /// Convert the API counts to the existing shared quorum semantics.
    pub fn quorum_state(&self) -> QuorumState {
        match self {
            Self::Known {
                healthy_members,
                total_members,
                ..
            } => QuorumState::from_counts(*healthy_members, *total_members),
            Self::Unavailable { .. } => QuorumState::Unknown,
        }
    }
}

/// Evaluate whether a node operation may run from the current etcd source of truth.
///
/// Drains and scheduling-only changes do not take an etcd member down. Reboot and shutdown are
/// blocked for an unknown or unsafe etcd impact. A degraded but still safe quorum is a warning,
/// preserving the previous frontend behavior of allowing an explicitly confirmed operation.
pub fn evaluate_operation_safety(
    operation: OperationKind,
    impact: &EtcdQuorumImpact,
) -> SafetyStatus {
    if !operation.is_destructive() {
        return SafetyStatus::Safe;
    }

    let EtcdQuorumImpact::Known {
        target_is_member,
        target_is_healthy,
        healthy_members,
        total_members,
    } = impact
    else {
        return SafetyStatus::Unknown;
    };

    if *total_members == 0 || *healthy_members > *total_members {
        return SafetyStatus::Unknown;
    }

    if !target_is_member {
        return SafetyStatus::Safe;
    }

    let required = total_members / 2 + 1;
    if *healthy_members < required {
        return SafetyStatus::Unsafe(format!(
            "etcd already lacks quorum ({}/{})",
            healthy_members, total_members
        ));
    }

    let healthy_after = healthy_members.saturating_sub(if *target_is_healthy { 1 } else { 0 });
    if healthy_after < required {
        return SafetyStatus::Unsafe(format!(
            "taking this etcd member down would leave {}/{} healthy; quorum requires {}",
            healthy_after, total_members, required
        ));
    }

    if *healthy_members < *total_members || !target_is_healthy {
        return SafetyStatus::Warning(format!(
            "etcd is already degraded ({}/{} healthy); operation retains quorum",
            healthy_members, total_members
        ));
    }

    SafetyStatus::Safe
}

/// Options for Kubernetes drain and post-reboot validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrainOptions {
    /// Maximum time to retry a PDB-blocked pod eviction.
    pub per_pod_timeout_secs: u64,
    /// Grace period used only for force-deleting an unmanaged pod.
    pub force_delete_grace_period_secs: Option<u32>,
    /// Permit force deletion only for unmanaged pods when normal eviction failed.
    pub force_delete_unmanaged: bool,
    /// Do not evict DaemonSet pods.
    pub ignore_daemonsets: bool,
    /// Permit eviction of pods using emptyDir volumes.
    pub delete_emptydir_data: bool,
    /// Verify Kubernetes Ready after a successful reboot request.
    pub wait_for_node_ready: bool,
    /// Maximum Kubernetes Ready wait after reboot.
    pub post_reboot_timeout_secs: u64,
    /// Restore scheduling after the node is Ready, but only if this operation cordoned it.
    pub uncordon_after_reboot: bool,
}

impl Default for DrainOptions {
    fn default() -> Self {
        Self {
            per_pod_timeout_secs: 30,
            force_delete_grace_period_secs: None,
            force_delete_unmanaged: false,
            ignore_daemonsets: true,
            delete_emptydir_data: true,
            wait_for_node_ready: true,
            post_reboot_timeout_secs: 300,
            uncordon_after_reboot: true,
        }
    }
}

/// A requested single-node operation.
#[derive(Debug, Clone)]
pub struct NodeOperationRequest {
    /// Requested mutation.
    pub operation: OperationKind,
    /// Explicit Kubernetes and Talos identity.
    pub target: NodeTarget,
    /// Source-of-truth etcd impact used for destructive safety evaluation.
    pub etcd_impact: EtcdQuorumImpact,
    /// Drain and verification options.
    pub drain_options: DrainOptions,
}

impl NodeOperationRequest {
    /// Create a request with the safe drain defaults. Scheduling-only operations do not consume
    /// the unavailable etcd value; destructive callers must supply a real impact snapshot.
    pub fn new(operation: OperationKind, target: NodeTarget) -> Self {
        Self {
            operation,
            target,
            etcd_impact: EtcdQuorumImpact::unavailable("etcd impact was not supplied"),
            drain_options: DrainOptions::default(),
        }
    }
}

/// Summary of evictions attempted during a drain.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrainSummary {
    /// Eligible pods discovered after mirror, DaemonSet, and emptyDir filtering.
    pub eligible_pods: usize,
    /// Pods successfully evicted through the eviction API.
    pub evicted_pods: usize,
    /// Unmanaged pods force-deleted after normal eviction failed or timed out.
    pub force_deleted_pods: Vec<PodReference>,
    /// Pods that could not be evicted, including malformed pod records.
    pub failed_pods: Vec<PodReference>,
}

/// Stable pod identity included in drain results and audit messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodReference {
    /// Namespace containing the pod.
    pub namespace: String,
    /// Pod name.
    pub name: String,
}

impl std::fmt::Display for PodReference {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}/{}", self.namespace, self.name)
    }
}

/// Explicit final result for a single-node operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeOperationResult {
    /// Requested operation.
    pub operation: OperationKind,
    /// Target operated on.
    pub target: NodeTarget,
    /// Final result state.
    pub status: OperationStatus,
    /// Source-of-truth scheduling state or an explicit uncertainty.
    pub scheduling: SchedulingState,
    /// Eviction details for drain-based operations.
    pub drain: Option<DrainSummary>,
    /// Human-readable outcome suitable for a frontend.
    pub message: String,
    /// Audit write failure, if any. The operation result remains explicit rather than hiding it.
    pub audit_error: Option<String>,
}

impl NodeOperationResult {
    fn new(
        operation: OperationKind,
        target: NodeTarget,
        status: OperationStatus,
        scheduling: SchedulingState,
        drain: Option<DrainSummary>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            target,
            status,
            scheduling,
            drain,
            message: message.into(),
            audit_error: None,
        }
    }
}

/// Audit phase stored for every operation event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditPhase {
    /// Operation invocation began.
    Start,
    /// A step or observable state transition occurred.
    Progress,
    /// The requested operation completed.
    Success,
    /// The requested operation failed or was blocked.
    Failure,
    /// Cancellation was observed.
    Cancelled,
}

/// A structured, append-only audit record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    /// UTC time when this entry was emitted.
    pub timestamp: DateTime<Utc>,
    /// Caller-selected cluster or context label.
    pub cluster: String,
    /// User or automation identity associated with the operation.
    pub actor: String,
    /// Operation being recorded.
    pub operation: OperationKind,
    /// Explicit target identity.
    pub target: NodeTarget,
    /// Lifecycle phase.
    pub phase: AuditPhase,
    /// Operation step.
    pub step: OperationStep,
    /// Human-readable event details.
    pub message: String,
}

/// A caller-configured local structured audit store.
#[derive(Debug, Clone)]
pub struct AuditLog {
    path: PathBuf,
    cluster: String,
    actor: String,
}

impl AuditLog {
    /// Create an audit store at `path`, using the current user as actor when available.
    pub fn new(path: impl Into<PathBuf>, cluster: impl Into<String>) -> Self {
        Self::with_actor(path, cluster, current_actor())
    }

    /// Create an audit store with an explicit automation or user identity.
    pub fn with_actor(
        path: impl Into<PathBuf>,
        cluster: impl Into<String>,
        actor: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            cluster: cluster.into(),
            actor: actor.into(),
        }
    }

    /// Return the caller-selected audit path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append a structured YAML document. Creating parent directories is intentionally local to
    /// this configured path and never touches global application state.
    pub fn append(&self, entry: &AuditEntry) -> Result<(), OperationsError> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|source| OperationsError::AuditIo {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let serialized =
            serde_yaml::to_string(entry).map_err(|source| OperationsError::AuditEncode {
                message: source.to_string(),
            })?;
        let document = format!("---\n{serialized}");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| OperationsError::AuditIo {
                path: self.path.clone(),
                source,
            })?;
        file.write_all(document.as_bytes())
            .map_err(|source| OperationsError::AuditIo {
                path: self.path.clone(),
                source,
            })
    }

    /// Read the structured history in chronological order. A missing audit file is an empty
    /// history; malformed persisted data is reported explicitly rather than ignored.
    pub fn read_entries(&self) -> Result<Vec<AuditEntry>, OperationsError> {
        let content = match fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(OperationsError::AuditIo {
                    path: self.path.clone(),
                    source,
                });
            }
        };

        serde_yaml::Deserializer::from_str(&content)
            .map(|document| {
                AuditEntry::deserialize(document).map_err(|source| OperationsError::AuditDecode {
                    path: self.path.clone(),
                    message: source.to_string(),
                })
            })
            .collect()
    }

    /// Read at most `count` most recent structured audit entries, retaining chronological order.
    pub fn read_recent(&self, count: usize) -> Result<Vec<AuditEntry>, OperationsError> {
        let entries = self.read_entries()?;
        let start = entries.len().saturating_sub(count);
        Ok(entries[start..].to_vec())
    }
}

/// Errors local to structured audit storage.
#[derive(Debug, Error)]
pub enum OperationsError {
    /// The configured audit path could not be read or written.
    #[error("audit I/O error at {path}: {source}")]
    AuditIo {
        /// Affected path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// An audit entry could not be encoded.
    #[error("could not encode audit entry: {message}")]
    AuditEncode {
        /// Serialization detail.
        message: String,
    },
    /// Persisted audit history could not be decoded.
    #[error("could not decode audit history at {path}: {message}")]
    AuditDecode {
        /// Affected path.
        path: PathBuf,
        /// Parse detail.
        message: String,
    },
}

/// The step associated with an operation or rolling event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStep {
    /// Request validation and confirmation.
    Preflight,
    /// Reading current Kubernetes scheduling state.
    InspectScheduling,
    /// Setting Kubernetes `spec.unschedulable` to true.
    Cordon,
    /// Setting Kubernetes `spec.unschedulable` to false.
    Uncordon,
    /// Listing eligible pods bound to the node.
    ListPods,
    /// Evicting a pod through the Kubernetes eviction API.
    EvictPod,
    /// Force-deleting an unmanaged pod.
    ForceDeletePod,
    /// Sending a Talos reboot request.
    Reboot,
    /// Sending a Talos shutdown request.
    Shutdown,
    /// Waiting for the Kubernetes Ready condition after reboot.
    WaitForReady,
    /// Compensating for a failed or cancelled drain by restoring scheduling.
    RestoreScheduling,
    /// Waiting between rolling nodes.
    InterNodeDelay,
    /// Final outcome.
    Complete,
}

/// Per-node progress event delivered synchronously to the caller's own channel or state bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationProgressEvent {
    /// Operation in progress.
    pub operation: OperationKind,
    /// Explicit target.
    pub target: NodeTarget,
    /// Event lifecycle phase.
    pub phase: AuditPhase,
    /// Current operation step.
    pub step: OperationStep,
    /// Human-readable state detail.
    pub message: String,
}

/// Progress event for sequential rolling orchestration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollingProgressEvent {
    /// Operation applied to each selected node.
    pub operation: OperationKind,
    /// Index in the ordered selected-node sequence, zero based.
    pub current_node_index: usize,
    /// Number of selected nodes.
    pub total_nodes: usize,
    /// Target currently being processed, if applicable.
    pub target: Option<NodeTarget>,
    /// Event lifecycle phase.
    pub phase: AuditPhase,
    /// Current orchestration step.
    pub step: OperationStep,
    /// Human-readable state detail.
    pub message: String,
}

/// Unified callback payload. The core never writes frontend state; callers choose how to carry
/// these events to a GUI or TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationsEvent {
    /// Single-node operation progress.
    Operation(OperationProgressEvent),
    /// Rolling orchestration progress.
    Rolling(RollingProgressEvent),
}

/// Callback receiving operation and rolling progress events.
pub type ProgressCallback = dyn FnMut(OperationsEvent) + Send;

/// Borrowed dependencies for one operation run.
pub struct OperationContext<'a> {
    /// Kubernetes client used for scheduling and eviction source-of-truth calls.
    pub kubernetes: &'a Client,
    /// Talos client used for reboot and shutdown requests.
    pub talos: Option<&'a TalosClient>,
    /// Structured local audit store.
    pub audit: &'a AuditLog,
    /// Cooperative cancellation predicate checked before and between mutations.
    pub is_cancelled: &'a CancellationPredicate,
}

/// Cordon a Kubernetes node after confirmation.
pub async fn cordon_node(
    target: NodeTarget,
    context: &OperationContext<'_>,
    confirmation: OperationConfirmation,
    progress: &mut ProgressCallback,
) -> NodeOperationResult {
    run_node_operation(
        NodeOperationRequest::new(OperationKind::Cordon, target),
        context,
        confirmation,
        progress,
    )
    .await
}

/// Uncordon a Kubernetes node after confirmation.
pub async fn uncordon_node(
    target: NodeTarget,
    context: &OperationContext<'_>,
    confirmation: OperationConfirmation,
    progress: &mut ProgressCallback,
) -> NodeOperationResult {
    run_node_operation(
        NodeOperationRequest::new(OperationKind::Uncordon, target),
        context,
        confirmation,
        progress,
    )
    .await
}

/// Drain a Kubernetes node after confirmation.
pub async fn drain_node(
    target: NodeTarget,
    options: DrainOptions,
    context: &OperationContext<'_>,
    confirmation: OperationConfirmation,
    progress: &mut ProgressCallback,
) -> NodeOperationResult {
    let mut request = NodeOperationRequest::new(OperationKind::Drain, target);
    request.drain_options = options;
    run_node_operation(request, context, confirmation, progress).await
}

/// Reboot a node after a frontend confirmation and etcd source-of-truth safety evaluation.
pub async fn reboot_node(
    target: NodeTarget,
    etcd_impact: EtcdQuorumImpact,
    options: DrainOptions,
    context: &OperationContext<'_>,
    confirmation: OperationConfirmation,
    progress: &mut ProgressCallback,
) -> NodeOperationResult {
    let mut request = NodeOperationRequest::new(OperationKind::Reboot, target);
    request.etcd_impact = etcd_impact;
    request.drain_options = options;
    run_node_operation(request, context, confirmation, progress).await
}

/// Shut down a node through Talos after a frontend confirmation and etcd safety evaluation.
pub async fn shutdown_node(
    target: NodeTarget,
    etcd_impact: EtcdQuorumImpact,
    options: DrainOptions,
    context: &OperationContext<'_>,
    confirmation: OperationConfirmation,
    progress: &mut ProgressCallback,
) -> NodeOperationResult {
    let mut request = NodeOperationRequest::new(OperationKind::Shutdown, target);
    request.etcd_impact = etcd_impact;
    request.drain_options = options;
    run_node_operation(request, context, confirmation, progress).await
}

/// Execute a confirmed single-node operation.
///
/// For destructive operations the supplied etcd impact is evaluated before any Kubernetes
/// mutation. A drain that cordoned a previously schedulable node always attempts a compensating
/// uncordon on failure or cancellation; the returned scheduling state makes any failed recovery
/// explicit.
pub async fn run_node_operation(
    request: NodeOperationRequest,
    context: &OperationContext<'_>,
    confirmation: OperationConfirmation,
    progress: &mut ProgressCallback,
) -> NodeOperationResult {
    let mut reporter = OperationReporter::new(&request, context.audit, progress);
    reporter.emit(
        AuditPhase::Start,
        OperationStep::Preflight,
        format!("Starting {} operation", request.operation.label()),
    );

    if !request.target.is_valid() {
        return reporter.finish(NodeOperationResult::new(
            request.operation,
            request.target,
            OperationStatus::Failed,
            SchedulingState::Unchanged,
            None,
            "node name and Talos address must both be non-empty",
        ));
    }

    if !confirmation.is_confirmed() {
        return reporter.finish(NodeOperationResult::new(
            request.operation,
            request.target,
            OperationStatus::NotConfirmed,
            SchedulingState::Unchanged,
            None,
            "operation was not confirmed by the frontend",
        ));
    }

    if is_cancelled(context) {
        return reporter.finish(NodeOperationResult::new(
            request.operation,
            request.target,
            OperationStatus::Cancelled,
            SchedulingState::Unchanged,
            None,
            "operation cancelled before mutation",
        ));
    }

    let safety = evaluate_operation_safety(request.operation, &request.etcd_impact);
    if request.operation.is_destructive()
        && matches!(safety, SafetyStatus::Unsafe(_) | SafetyStatus::Unknown)
    {
        let message = match safety {
            SafetyStatus::Unsafe(reason) => format!("operation blocked: {reason}"),
            SafetyStatus::Unknown => {
                "operation blocked: etcd quorum impact is unavailable or unknown".to_string()
            }
            SafetyStatus::Safe | SafetyStatus::Warning(_) => unreachable!(),
        };
        return reporter.finish(NodeOperationResult::new(
            request.operation,
            request.target,
            OperationStatus::Blocked,
            SchedulingState::Unchanged,
            None,
            message,
        ));
    }

    match request.operation {
        OperationKind::Cordon => {
            let step = cordon_inner(&request.target, context, &mut reporter).await;
            reporter.finish(step.into_result(request.operation, request.target, None))
        }
        OperationKind::Uncordon => {
            let step = uncordon_inner(&request.target, context, &mut reporter, false).await;
            reporter.finish(step.into_result(request.operation, request.target, None))
        }
        OperationKind::Drain => {
            let result = drain_operation(&request, context, &mut reporter).await;
            reporter.finish(result)
        }
        OperationKind::Reboot => {
            let result = destructive_operation(&request, context, &mut reporter, false).await;
            reporter.finish(result)
        }
        OperationKind::Shutdown => {
            let result = destructive_operation(&request, context, &mut reporter, true).await;
            reporter.finish(result)
        }
    }
}

#[derive(Debug)]
struct SchedulingStepResult {
    status: OperationStatus,
    scheduling: SchedulingState,
    message: String,
}

impl SchedulingStepResult {
    fn success(scheduling: SchedulingState, message: impl Into<String>) -> Self {
        Self {
            status: OperationStatus::Succeeded,
            scheduling,
            message: message.into(),
        }
    }

    fn failure(scheduling: SchedulingState, message: impl Into<String>) -> Self {
        Self {
            status: OperationStatus::Failed,
            scheduling,
            message: message.into(),
        }
    }

    fn cancelled(scheduling: SchedulingState, message: impl Into<String>) -> Self {
        Self {
            status: OperationStatus::Cancelled,
            scheduling,
            message: message.into(),
        }
    }

    fn into_result(
        self,
        operation: OperationKind,
        target: NodeTarget,
        drain: Option<DrainSummary>,
    ) -> NodeOperationResult {
        NodeOperationResult::new(
            operation,
            target,
            self.status,
            self.scheduling,
            drain,
            self.message,
        )
    }
}

async fn cordon_inner(
    target: &NodeTarget,
    context: &OperationContext<'_>,
    reporter: &mut OperationReporter<'_>,
) -> SchedulingStepResult {
    reporter.emit(
        AuditPhase::Progress,
        OperationStep::InspectScheduling,
        "Reading Kubernetes scheduling state",
    );
    let nodes: Api<Node> = Api::all(context.kubernetes.clone());
    let node = match nodes.get(&target.name).await {
        Ok(node) => node,
        Err(error) => {
            return SchedulingStepResult::failure(
                SchedulingState::Unknown(format!("could not read node: {error}")),
                format!("could not read Kubernetes node before cordon: {error}"),
            );
        }
    };

    if node_is_unschedulable(&node) {
        return SchedulingStepResult::success(
            SchedulingState::AlreadyCordoned,
            "node was already cordoned; scheduling was left unchanged",
        );
    }

    if is_cancelled(context) {
        return SchedulingStepResult::cancelled(
            SchedulingState::Schedulable,
            "operation cancelled before cordoning node",
        );
    }

    reporter.emit(
        AuditPhase::Progress,
        OperationStep::Cordon,
        "Cordoning node",
    );
    let patch = SchedulingPatch::cordon();
    match nodes
        .patch(&target.name, &PatchParams::default(), &Patch::Merge(&patch))
        .await
    {
        Ok(node) if node_is_unschedulable(&node) => {
            SchedulingStepResult::success(SchedulingState::CordonedByOperation, "node cordoned")
        }
        Ok(_) => SchedulingStepResult::failure(
            SchedulingState::Schedulable,
            "Kubernetes accepted the cordon patch but reported the node as schedulable",
        ),
        Err(error) => SchedulingStepResult::failure(
            SchedulingState::Unknown(format!("cordon patch failed: {error}")),
            format!("failed to cordon node: {error}"),
        ),
    }
}

async fn uncordon_inner(
    target: &NodeTarget,
    context: &OperationContext<'_>,
    reporter: &mut OperationReporter<'_>,
    compensating: bool,
) -> SchedulingStepResult {
    reporter.emit(
        AuditPhase::Progress,
        if compensating {
            OperationStep::RestoreScheduling
        } else {
            OperationStep::InspectScheduling
        },
        if compensating {
            "Restoring scheduling after incomplete drain"
        } else {
            "Reading Kubernetes scheduling state"
        },
    );
    let nodes: Api<Node> = Api::all(context.kubernetes.clone());
    let node = match nodes.get(&target.name).await {
        Ok(node) => node,
        Err(error) => {
            return SchedulingStepResult::failure(
                SchedulingState::Unknown(format!("could not read node: {error}")),
                format!("could not read Kubernetes node before uncordon: {error}"),
            );
        }
    };

    if !node_is_unschedulable(&node) {
        return SchedulingStepResult::success(SchedulingState::Schedulable, "node is schedulable");
    }

    // Cancellation must not suppress compensation: leaving a node cordoned after a cancelled
    // drain is a more surprising state than completing this one restoring mutation. Direct
    // uncordon operations still honor cancellation before their patch.
    if !compensating && is_cancelled(context) {
        return SchedulingStepResult::cancelled(
            SchedulingState::AlreadyCordoned,
            "operation cancelled before uncordoning node",
        );
    }

    reporter.emit(
        AuditPhase::Progress,
        if compensating {
            OperationStep::RestoreScheduling
        } else {
            OperationStep::Uncordon
        },
        "Uncordoning node",
    );
    let patch = SchedulingPatch::uncordon();
    match nodes
        .patch(&target.name, &PatchParams::default(), &Patch::Merge(&patch))
        .await
    {
        Ok(node) if !node_is_unschedulable(&node) => {
            SchedulingStepResult::success(SchedulingState::Schedulable, "node is schedulable")
        }
        Ok(_) => SchedulingStepResult::failure(
            SchedulingState::CordonedByOperation,
            "Kubernetes accepted the uncordon patch but reported the node as cordoned",
        ),
        Err(error) => SchedulingStepResult::failure(
            SchedulingState::Unknown(format!("uncordon patch failed: {error}")),
            format!("failed to uncordon node: {error}"),
        ),
    }
}

async fn drain_operation(
    request: &NodeOperationRequest,
    context: &OperationContext<'_>,
    reporter: &mut OperationReporter<'_>,
) -> NodeOperationResult {
    let cordon = cordon_inner(&request.target, context, reporter).await;
    if !cordon.status.is_success() {
        return cordon.into_result(request.operation, request.target.clone(), None);
    }

    let drain = drain_pods(&request.target, &request.drain_options, context, reporter).await;
    if drain.status.is_success() {
        return NodeOperationResult::new(
            request.operation,
            request.target.clone(),
            OperationStatus::Succeeded,
            cordon.scheduling,
            Some(drain.summary),
            drain.message,
        );
    }

    let scheduling =
        restore_after_incomplete_drain(&request.target, &cordon.scheduling, context, reporter)
            .await;
    NodeOperationResult::new(
        request.operation,
        request.target.clone(),
        drain.status,
        scheduling,
        Some(drain.summary),
        drain.message,
    )
}

async fn destructive_operation(
    request: &NodeOperationRequest,
    context: &OperationContext<'_>,
    reporter: &mut OperationReporter<'_>,
    shutdown: bool,
) -> NodeOperationResult {
    let cordon = cordon_inner(&request.target, context, reporter).await;
    if !cordon.status.is_success() {
        return cordon.into_result(request.operation, request.target.clone(), None);
    }

    let drain = drain_pods(&request.target, &request.drain_options, context, reporter).await;
    if !drain.status.is_success() {
        let scheduling =
            restore_after_incomplete_drain(&request.target, &cordon.scheduling, context, reporter)
                .await;
        return NodeOperationResult::new(
            request.operation,
            request.target.clone(),
            drain.status,
            scheduling,
            Some(drain.summary),
            if shutdown {
                format!("shutdown aborted: {}", drain.message)
            } else {
                format!("reboot aborted: {}", drain.message)
            },
        );
    }

    if is_cancelled(context) {
        let scheduling =
            restore_after_incomplete_drain(&request.target, &cordon.scheduling, context, reporter)
                .await;
        return NodeOperationResult::new(
            request.operation,
            request.target.clone(),
            OperationStatus::Cancelled,
            scheduling,
            Some(drain.summary),
            "operation cancelled after drain; scheduling restoration was attempted",
        );
    }

    let Some(talos) = context.talos else {
        let scheduling =
            restore_after_incomplete_drain(&request.target, &cordon.scheduling, context, reporter)
                .await;
        return NodeOperationResult::new(
            request.operation,
            request.target.clone(),
            OperationStatus::Unsupported,
            scheduling,
            Some(drain.summary),
            "Talos client is unavailable for this operation",
        );
    };

    let step = if shutdown {
        OperationStep::Shutdown
    } else {
        OperationStep::Reboot
    };
    reporter.emit(
        AuditPhase::Progress,
        step,
        if shutdown {
            "Sending graceful Talos shutdown request"
        } else {
            "Sending Talos reboot request"
        },
    );

    let talos_target = talos.with_node(&request.target.address);
    let request_accepted = if shutdown {
        match talos_target.shutdown(false).await {
            Ok(result) if result.success => Ok(()),
            Ok(_) => Err("Talos did not acknowledge the shutdown request".to_string()),
            Err(error) => Err(format!("Talos shutdown request failed: {error}")),
        }
    } else {
        match talos_target.reboot(RebootMode::Default).await {
            Ok(result) if result.success => Ok(()),
            Ok(_) => Err("Talos did not acknowledge the reboot request".to_string()),
            Err(error) => Err(format!("Talos reboot request failed: {error}")),
        }
    };

    if let Err(message) = request_accepted {
        let scheduling =
            restore_after_incomplete_drain(&request.target, &cordon.scheduling, context, reporter)
                .await;
        return NodeOperationResult::new(
            request.operation,
            request.target.clone(),
            OperationStatus::Failed,
            scheduling,
            Some(drain.summary),
            message,
        );
    }

    if shutdown || !request.drain_options.wait_for_node_ready {
        let message = if shutdown {
            "shutdown request accepted; node remains cordoned".to_string()
        } else {
            "reboot request accepted; readiness verification is disabled and node remains cordoned"
                .to_string()
        };
        return NodeOperationResult::new(
            request.operation,
            request.target.clone(),
            OperationStatus::Succeeded,
            cordon.scheduling,
            Some(drain.summary),
            message,
        );
    }

    match wait_for_node_ready(&request.target, &request.drain_options, context, reporter).await {
        ReadyWait::Ready(elapsed) => {
            if request.drain_options.uncordon_after_reboot
                && matches!(cordon.scheduling, SchedulingState::CordonedByOperation)
            {
                let restored = uncordon_inner(&request.target, context, reporter, true).await;
                if !restored.status.is_success() {
                    return NodeOperationResult::new(
                        request.operation,
                        request.target.clone(),
                        OperationStatus::Failed,
                        restored.scheduling,
                        Some(drain.summary),
                        format!(
                            "reboot completed in {}s, but scheduling recovery failed: {}",
                            elapsed.as_secs(),
                            restored.message
                        ),
                    );
                }
                return NodeOperationResult::new(
                    request.operation,
                    request.target.clone(),
                    OperationStatus::Succeeded,
                    restored.scheduling,
                    Some(drain.summary),
                    format!("reboot completed in {}s and scheduling was restored", elapsed.as_secs()),
                );
            }

            NodeOperationResult::new(
                request.operation,
                request.target.clone(),
                OperationStatus::Succeeded,
                cordon.scheduling,
                Some(drain.summary),
                format!("reboot completed in {}s; node scheduling was preserved", elapsed.as_secs()),
            )
        }
        ReadyWait::Cancelled => NodeOperationResult::new(
            request.operation,
            request.target.clone(),
            OperationStatus::Cancelled,
            cordon.scheduling,
            Some(drain.summary),
            "reboot request was accepted, but readiness verification was cancelled; node remains cordoned"
                .to_string(),
        ),
        ReadyWait::Failed(message) => NodeOperationResult::new(
            request.operation,
            request.target.clone(),
            OperationStatus::Failed,
            cordon.scheduling,
            Some(drain.summary),
            format!("reboot request was accepted, but readiness verification failed: {message}"),
        ),
    }
}

async fn restore_after_incomplete_drain(
    target: &NodeTarget,
    cordon_state: &SchedulingState,
    context: &OperationContext<'_>,
    reporter: &mut OperationReporter<'_>,
) -> SchedulingState {
    if !matches!(cordon_state, SchedulingState::CordonedByOperation) {
        return cordon_state.clone();
    }

    let restoration = uncordon_inner(target, context, reporter, true).await;
    if restoration.status.is_success() {
        restoration.scheduling
    } else {
        // `Unknown` intentionally captures the possible partial patch case. Do not claim that
        // the node was uncordoned simply because the recovery request was attempted.
        match restoration.scheduling {
            SchedulingState::Unknown(message) => SchedulingState::Unknown(format!(
                "scheduling recovery after incomplete drain is unknown: {message}"
            )),
            state => SchedulingState::Unknown(format!(
                "scheduling recovery after incomplete drain failed: {}; last observed state: {state:?}",
                restoration.message
            )),
        }
    }
}

#[derive(Debug)]
struct DrainExecution {
    status: OperationStatus,
    summary: DrainSummary,
    message: String,
}

async fn drain_pods(
    target: &NodeTarget,
    options: &DrainOptions,
    context: &OperationContext<'_>,
    reporter: &mut OperationReporter<'_>,
) -> DrainExecution {
    reporter.emit(
        AuditPhase::Progress,
        OperationStep::ListPods,
        "Listing eligible pods on node",
    );
    let pods: Api<Pod> = Api::all(context.kubernetes.clone());
    let selector = format!("spec.nodeName={}", target.name);
    let pod_list = match pods.list(&ListParams::default().fields(&selector)).await {
        Ok(pods) => pods,
        Err(error) => {
            return DrainExecution {
                status: OperationStatus::Failed,
                summary: DrainSummary::default(),
                message: format!("could not list node pods: {error}"),
            };
        }
    };

    if is_cancelled(context) {
        return DrainExecution {
            status: OperationStatus::Cancelled,
            summary: DrainSummary::default(),
            message: "operation cancelled before pod eviction".to_string(),
        };
    }

    let mut summary = DrainSummary::default();
    let mut candidates = Vec::new();
    for pod in pod_list.items {
        if is_mirror_pod(&pod) || (options.ignore_daemonsets && is_daemonset_pod(&pod)) {
            continue;
        }
        if has_emptydir(&pod) && !options.delete_emptydir_data {
            continue;
        }

        let namespace = pod.metadata.namespace.clone().unwrap_or_default();
        let Some(name) = pod.metadata.name.clone().filter(|name| !name.is_empty()) else {
            summary.failed_pods.push(PodReference {
                namespace,
                name: "<unnamed>".to_string(),
            });
            continue;
        };
        candidates.push(PodCandidate {
            reference: PodReference { namespace, name },
            managed: is_controller_managed(&pod),
        });
    }
    summary.eligible_pods = candidates.len();

    reporter.emit(
        AuditPhase::Progress,
        OperationStep::ListPods,
        format!("Found {} eligible pods to evict", summary.eligible_pods),
    );

    for (index, candidate) in candidates.into_iter().enumerate() {
        if is_cancelled(context) {
            let message = format!(
                "operation cancelled after evicting {} of {} eligible pods",
                summary.evicted_pods, summary.eligible_pods
            );
            return DrainExecution {
                status: OperationStatus::Cancelled,
                summary,
                message,
            };
        }

        let total = summary.eligible_pods;
        reporter.emit(
            AuditPhase::Progress,
            OperationStep::EvictPod,
            format!("Evicting {} ({}/{})", candidate.reference, index + 1, total),
        );
        let outcome = evict_pod(candidate, options, context, reporter).await;
        match outcome {
            PodEvictionOutcome::Evicted => summary.evicted_pods += 1,
            PodEvictionOutcome::ForceDeleted(reference) => {
                summary.evicted_pods += 1;
                summary.force_deleted_pods.push(reference);
            }
            PodEvictionOutcome::Failed(reference) => summary.failed_pods.push(reference),
            PodEvictionOutcome::Cancelled => {
                return DrainExecution {
                    status: OperationStatus::Cancelled,
                    summary,
                    message: "operation cancelled during pod eviction".to_string(),
                };
            }
        }
    }

    if summary.failed_pods.is_empty() {
        DrainExecution {
            status: OperationStatus::Succeeded,
            message: format!("drained {} eligible pods", summary.evicted_pods),
            summary,
        }
    } else {
        let failed = summary
            .failed_pods
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        DrainExecution {
            status: OperationStatus::Failed,
            message: format!(
                "evicted {} of {} eligible pods; failed: {failed}",
                summary.evicted_pods, summary.eligible_pods
            ),
            summary,
        }
    }
}

#[derive(Debug)]
struct PodCandidate {
    reference: PodReference,
    managed: bool,
}

#[derive(Debug)]
enum PodEvictionOutcome {
    Evicted,
    ForceDeleted(PodReference),
    Failed(PodReference),
    Cancelled,
}

async fn evict_pod(
    candidate: PodCandidate,
    options: &DrainOptions,
    context: &OperationContext<'_>,
    reporter: &mut OperationReporter<'_>,
) -> PodEvictionOutcome {
    let pods: Api<Pod> =
        Api::namespaced(context.kubernetes.clone(), &candidate.reference.namespace);
    let max_attempts = (options.per_pod_timeout_secs / 2).max(1);
    let eviction_parameters = EvictParams::default();

    for attempt in 1..=max_attempts {
        if is_cancelled(context) {
            return PodEvictionOutcome::Cancelled;
        }

        match pods
            .evict(&candidate.reference.name, &eviction_parameters)
            .await
        {
            Ok(_) => return PodEvictionOutcome::Evicted,
            Err(error) if is_not_found(&error.to_string()) => return PodEvictionOutcome::Evicted,
            Err(error) if is_pdb_block(&error.to_string()) && attempt < max_attempts => {
                reporter.emit(
                    AuditPhase::Progress,
                    OperationStep::EvictPod,
                    format!(
                        "PDB is blocking {} (retry {attempt}/{max_attempts})",
                        candidate.reference
                    ),
                );
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(error) => {
                return force_delete_or_fail(
                    candidate,
                    options,
                    context,
                    reporter,
                    error.to_string(),
                )
                .await;
            }
        }
    }

    force_delete_or_fail(
        candidate,
        options,
        context,
        reporter,
        "PDB retry time exhausted".to_string(),
    )
    .await
}

async fn force_delete_or_fail(
    candidate: PodCandidate,
    options: &DrainOptions,
    context: &OperationContext<'_>,
    reporter: &mut OperationReporter<'_>,
    reason: String,
) -> PodEvictionOutcome {
    if is_cancelled(context) {
        return PodEvictionOutcome::Cancelled;
    }
    if !options.force_delete_unmanaged || candidate.managed {
        reporter.emit(
            AuditPhase::Progress,
            OperationStep::EvictPod,
            format!("Could not evict {}: {reason}", candidate.reference),
        );
        return PodEvictionOutcome::Failed(candidate.reference);
    }

    reporter.emit(
        AuditPhase::Progress,
        OperationStep::ForceDeletePod,
        format!("Force-deleting unmanaged pod {}", candidate.reference),
    );
    let pods: Api<Pod> =
        Api::namespaced(context.kubernetes.clone(), &candidate.reference.namespace);
    let parameters = options.force_delete_grace_period_secs.map_or_else(
        || DeleteParams::default().grace_period(0),
        |grace| DeleteParams::default().grace_period(grace),
    );
    match pods.delete(&candidate.reference.name, &parameters).await {
        Ok(_) => PodEvictionOutcome::ForceDeleted(candidate.reference),
        Err(error) if is_not_found(&error.to_string()) => {
            PodEvictionOutcome::ForceDeleted(candidate.reference)
        }
        Err(error) => {
            reporter.emit(
                AuditPhase::Progress,
                OperationStep::ForceDeletePod,
                format!("Could not force-delete {}: {error}", candidate.reference),
            );
            PodEvictionOutcome::Failed(candidate.reference)
        }
    }
}

enum ReadyWait {
    Ready(Duration),
    Cancelled,
    Failed(String),
}

/// API unavailability is not evidence of a node transition. Only an observed Ready condition
/// becoming false/unknown or an authoritative missing-node response can establish it.
enum RebootObservation {
    Ready,
    NotReady,
    Missing,
    Unavailable(String),
}

impl RebootObservation {
    fn from_response(response: Result<Option<Node>, kube::Error>) -> Self {
        match response {
            Ok(None) => Self::Missing,
            Ok(Some(node)) => {
                if node_is_ready(&node) {
                    Self::Ready
                } else if node
                    .status
                    .as_ref()
                    .and_then(|status| status.conditions.as_ref())
                    .is_some_and(|conditions| {
                        conditions.iter().any(|condition| {
                            condition.type_ == "Ready"
                                && matches!(condition.status.as_str(), "False" | "Unknown")
                        })
                    })
                {
                    Self::NotReady
                } else {
                    Self::Unavailable("Kubernetes Ready condition is unavailable".to_string())
                }
            }
            Err(error) => Self::Unavailable(error.to_string()),
        }
    }
}

enum RebootReadinessDecision {
    Waiting(String),
    Finished(ReadyWait),
}

/// The same observation sequence drives production polling and deterministic fixtures.
struct RebootReadiness {
    observed_transition: bool,
    timeout: Duration,
    transition_timeout: Duration,
}

impl RebootReadiness {
    fn new(timeout: Duration) -> Self {
        Self {
            observed_transition: false,
            timeout,
            transition_timeout: Duration::from_secs(60).min(timeout),
        }
    }

    fn deadline(&self) -> Duration {
        if self.observed_transition {
            self.timeout
        } else {
            self.transition_timeout
        }
    }

    fn advance(
        &mut self,
        observation: Option<RebootObservation>,
        elapsed: Duration,
        cancelled: bool,
    ) -> RebootReadinessDecision {
        if cancelled {
            return RebootReadinessDecision::Finished(ReadyWait::Cancelled);
        }
        if elapsed >= self.deadline() {
            let message = match require_reboot_transition(self.observed_transition) {
                Err(message) => message,
                Ok(()) => format!(
                    "timed out after {}s waiting for Kubernetes Ready",
                    self.timeout.as_secs()
                ),
            };
            return RebootReadinessDecision::Finished(ReadyWait::Failed(message));
        }
        let message = match observation {
            Some(RebootObservation::Ready) if self.observed_transition => {
                return RebootReadinessDecision::Finished(ReadyWait::Ready(elapsed));
            }
            Some(RebootObservation::Ready) | None => {
                "Waiting for an observed reboot transition; an already-Ready node is not proof"
                    .to_string()
            }
            Some(RebootObservation::NotReady) => {
                self.observed_transition = true;
                format!("Node is not Ready ({}s elapsed)", elapsed.as_secs())
            }
            Some(RebootObservation::Missing) => {
                self.observed_transition = true;
                "Node is missing from Kubernetes; waiting for it to rejoin Ready".to_string()
            }
            Some(RebootObservation::Unavailable(error)) => {
                format!("Kubernetes observation unavailable (not reboot evidence): {error}")
            }
        };
        RebootReadinessDecision::Waiting(message)
    }
}

async fn wait_for_node_ready(
    target: &NodeTarget,
    options: &DrainOptions,
    context: &OperationContext<'_>,
    reporter: &mut OperationReporter<'_>,
) -> ReadyWait {
    let started = Instant::now();
    let mut readiness = RebootReadiness::new(Duration::from_secs(options.post_reboot_timeout_secs));
    let nodes: Api<Node> = Api::all(context.kubernetes.clone());

    loop {
        if let RebootReadinessDecision::Finished(result) =
            readiness.advance(None, started.elapsed(), is_cancelled(context))
        {
            if let ReadyWait::Failed(message) = &result {
                reporter.emit(
                    AuditPhase::Progress,
                    OperationStep::WaitForReady,
                    message.clone(),
                );
            }
            return result;
        }

        // Bound even a stalled API request by the current transition/readiness deadline, and
        // keep cancellation responsive while the request is pending.
        let remaining = readiness.deadline().saturating_sub(started.elapsed());
        let response = tokio::select! {
            response = tokio::time::timeout(remaining, nodes.get_opt(&target.name)) => response,
            _ = wait_with_cancellation(remaining, context) => continue,
        };
        let Ok(response) = response else {
            continue;
        };
        match readiness.advance(
            Some(RebootObservation::from_response(response)),
            started.elapsed(),
            is_cancelled(context),
        ) {
            RebootReadinessDecision::Finished(ReadyWait::Ready(elapsed)) => {
                reporter.emit(
                    AuditPhase::Progress,
                    OperationStep::WaitForReady,
                    format!("Node is Ready after {}s", elapsed.as_secs()),
                );
                return ReadyWait::Ready(elapsed);
            }
            RebootReadinessDecision::Finished(result) => return result,
            RebootReadinessDecision::Waiting(message) => {
                reporter.emit(AuditPhase::Progress, OperationStep::WaitForReady, message);
            }
        }

        let poll = if readiness.observed_transition { 5 } else { 2 };
        let remaining = readiness.deadline().saturating_sub(started.elapsed());
        if wait_with_cancellation(Duration::from_secs(poll).min(remaining), context).await {
            return ReadyWait::Cancelled;
        }
    }
}

fn require_reboot_transition(observed_transition: bool) -> Result<(), String> {
    if observed_transition {
        Ok(())
    } else {
        Err("Reboot transition was not observed: an already-Ready node is not proof that the reboot completed; scheduling was not restored".to_string())
    }
}

/// A selected node for a rolling operation. `selection_order` is explicit rather than inferred
/// from source-vector position so a frontend can preserve its selection semantics.
#[derive(Debug, Clone)]
pub struct RollingNode {
    /// Target node.
    pub target: NodeTarget,
    /// Etcd facts for this node, obtained before the rolling run begins.
    pub etcd_impact: EtcdQuorumImpact,
    /// One-based selection order from the frontend.
    pub selection_order: usize,
}

/// Sequential rolling-operation configuration.
#[derive(Debug, Clone)]
pub struct RollingOperationRequest {
    /// Operation applied to every selected node.
    pub operation: OperationKind,
    /// Selected nodes in any frontend storage order.
    pub nodes: Vec<RollingNode>,
    /// Shared drain and post-reboot behavior.
    pub drain_options: DrainOptions,
    /// Delay between completed node operations.
    pub delay_between_nodes: Duration,
    /// Stop immediately after a failed node operation. Cancellation always stops immediately.
    pub stop_on_failure: bool,
}

impl RollingOperationRequest {
    /// Construct a rolling request with TUI-compatible defaults.
    pub fn new(operation: OperationKind, nodes: Vec<RollingNode>) -> Self {
        Self {
            operation,
            nodes,
            drain_options: DrainOptions::default(),
            delay_between_nodes: Duration::from_secs(30),
            stop_on_failure: true,
        }
    }

    /// Return selected nodes in frontend selection order, without mutating the request.
    pub fn ordered_nodes(&self) -> Vec<&RollingNode> {
        let mut nodes = self.nodes.iter().collect::<Vec<_>>();
        nodes.sort_by_key(|node| node.selection_order);
        nodes
    }
}

/// Per-node result retained by a rolling run, including unstarted nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollingNodeResult {
    /// Target node.
    pub target: NodeTarget,
    /// Explicit result for this selected node.
    pub result: NodeOperationResult,
}

/// Explicit result for a sequential rolling operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollingOperationResult {
    /// Operation attempted on each node.
    pub operation: OperationKind,
    /// Aggregate status.
    pub status: OperationStatus,
    /// Per-node outcomes in selection order.
    pub nodes: Vec<RollingNodeResult>,
    /// Human-readable aggregate outcome.
    pub message: String,
    /// Audit write failure, if any.
    pub audit_error: Option<String>,
}

impl RollingOperationResult {
    /// Number of nodes whose requested operation completed successfully.
    pub fn completed_nodes(&self) -> usize {
        self.nodes
            .iter()
            .filter(|node| node.result.status.is_success())
            .count()
    }
}

/// Run a confirmed rolling operation sequentially.
///
/// The runner first validates confirmation, selection ordering, and every destructive etcd
/// impact. It therefore never mutates an earlier node before discovering an unsafe or unknown
/// quorum impact in a later selected node.
pub async fn run_rolling_operation(
    request: RollingOperationRequest,
    context: &OperationContext<'_>,
    confirmation: OperationConfirmation,
    progress: &mut ProgressCallback,
) -> RollingOperationResult {
    let ordered = request.ordered_nodes();
    let mut reporter = RollingReporter::new(request.operation, context.audit, progress);
    reporter.emit(
        0,
        ordered.len(),
        None,
        AuditPhase::Start,
        OperationStep::Preflight,
        format!(
            "Starting rolling {} across {} nodes",
            request.operation.label(),
            ordered.len()
        ),
    );

    if ordered.is_empty() {
        return reporter.finish(RollingOperationResult {
            operation: request.operation,
            status: OperationStatus::Failed,
            nodes: Vec::new(),
            message: "rolling operation requires at least one selected node".to_string(),
            audit_error: None,
        });
    }

    if let Some(message) = invalid_selection_order(&ordered) {
        return reporter.finish(rolling_not_started(
            &request,
            &ordered,
            OperationStatus::Blocked,
            message,
        ));
    }

    if !confirmation.is_confirmed() {
        return reporter.finish(rolling_not_started(
            &request,
            &ordered,
            OperationStatus::NotConfirmed,
            "rolling operation was not confirmed by the frontend".to_string(),
        ));
    }

    if is_cancelled(context) {
        return reporter.finish(rolling_not_started(
            &request,
            &ordered,
            OperationStatus::Cancelled,
            "rolling operation cancelled before mutation".to_string(),
        ));
    }

    if request.operation.is_destructive()
        && let Some((target, safety)) = ordered.iter().find_map(|node| {
            let safety = evaluate_operation_safety(request.operation, &node.etcd_impact);
            matches!(safety, SafetyStatus::Unsafe(_) | SafetyStatus::Unknown)
                .then_some((&node.target, safety))
        })
    {
        let detail = match safety {
            SafetyStatus::Unsafe(reason) => reason,
            SafetyStatus::Unknown => "etcd quorum impact is unavailable or unknown".to_string(),
            SafetyStatus::Safe | SafetyStatus::Warning(_) => unreachable!(),
        };
        return reporter.finish(rolling_not_started(
            &request,
            &ordered,
            OperationStatus::Blocked,
            format!("rolling operation blocked by {}: {detail}", target.name),
        ));
    }

    let mut results = Vec::with_capacity(ordered.len());
    for (index, node) in ordered.iter().enumerate() {
        if is_cancelled(context) {
            results.push(RollingNodeResult {
                target: node.target.clone(),
                result: NodeOperationResult::new(
                    request.operation,
                    node.target.clone(),
                    OperationStatus::Cancelled,
                    SchedulingState::Unchanged,
                    None,
                    "rolling operation cancelled before this node started",
                ),
            });
            append_not_started_results(
                &mut results,
                request.operation,
                &ordered[index + 1..],
                "not started because rolling operation was cancelled",
            );
            return reporter.finish(RollingOperationResult {
                operation: request.operation,
                status: OperationStatus::Cancelled,
                nodes: results,
                message: "rolling operation cancelled".to_string(),
                audit_error: None,
            });
        }

        reporter.emit(
            index,
            ordered.len(),
            Some(node.target.clone()),
            AuditPhase::Progress,
            OperationStep::Preflight,
            format!(
                "Processing node {}/{}: {}",
                index + 1,
                ordered.len(),
                node.target.name
            ),
        );
        let result = run_node_operation(
            NodeOperationRequest {
                operation: request.operation,
                target: node.target.clone(),
                etcd_impact: node.etcd_impact.clone(),
                drain_options: request.drain_options.clone(),
            },
            context,
            confirmation,
            &mut *reporter.progress,
        )
        .await;
        let terminal_status = result.status;
        results.push(RollingNodeResult {
            target: node.target.clone(),
            result,
        });

        if !terminal_status.is_success() {
            if terminal_status == OperationStatus::Cancelled {
                append_not_started_results(
                    &mut results,
                    request.operation,
                    &ordered[index + 1..],
                    "not started because rolling operation was cancelled",
                );
                return reporter.finish(RollingOperationResult {
                    operation: request.operation,
                    status: OperationStatus::Cancelled,
                    nodes: results,
                    message: "rolling operation cancelled".to_string(),
                    audit_error: None,
                });
            }
            if request.stop_on_failure {
                append_not_started_results(
                    &mut results,
                    request.operation,
                    &ordered[index + 1..],
                    "not started after a previous node operation failed",
                );
                return reporter.finish(RollingOperationResult {
                    operation: request.operation,
                    status: OperationStatus::Failed,
                    nodes: results,
                    message: format!(
                        "rolling operation stopped after {} failed",
                        node.target.name
                    ),
                    audit_error: None,
                });
            }
        }

        if index + 1 < ordered.len() && !request.delay_between_nodes.is_zero() {
            reporter.emit(
                index,
                ordered.len(),
                Some(node.target.clone()),
                AuditPhase::Progress,
                OperationStep::InterNodeDelay,
                format!(
                    "Waiting {}s before the next node",
                    request.delay_between_nodes.as_secs()
                ),
            );
            if wait_with_cancellation(request.delay_between_nodes, context).await {
                append_not_started_results(
                    &mut results,
                    request.operation,
                    &ordered[index + 1..],
                    "not started because rolling operation was cancelled during inter-node delay",
                );
                return reporter.finish(RollingOperationResult {
                    operation: request.operation,
                    status: OperationStatus::Cancelled,
                    nodes: results,
                    message: "rolling operation cancelled during inter-node delay".to_string(),
                    audit_error: None,
                });
            }
        }
    }

    let completed = results
        .iter()
        .filter(|result| result.result.status.is_success())
        .count();
    let status = if completed == results.len() {
        OperationStatus::Succeeded
    } else {
        OperationStatus::Failed
    };
    reporter.finish(RollingOperationResult {
        operation: request.operation,
        status,
        nodes: results,
        message: format!(
            "completed {completed}/{} rolling node operations",
            ordered.len()
        ),
        audit_error: None,
    })
}

fn rolling_not_started(
    request: &RollingOperationRequest,
    ordered: &[&RollingNode],
    status: OperationStatus,
    message: String,
) -> RollingOperationResult {
    RollingOperationResult {
        operation: request.operation,
        status,
        nodes: ordered
            .iter()
            .map(|node| RollingNodeResult {
                target: node.target.clone(),
                result: NodeOperationResult::new(
                    request.operation,
                    node.target.clone(),
                    status,
                    SchedulingState::Unchanged,
                    None,
                    message.clone(),
                ),
            })
            .collect(),
        message,
        audit_error: None,
    }
}

fn append_not_started_results(
    results: &mut Vec<RollingNodeResult>,
    operation: OperationKind,
    nodes: &[&RollingNode],
    message: &str,
) {
    results.extend(nodes.iter().map(|node| RollingNodeResult {
        target: node.target.clone(),
        result: NodeOperationResult::new(
            operation,
            node.target.clone(),
            OperationStatus::NotStarted,
            SchedulingState::Unchanged,
            None,
            message,
        ),
    }));
}

fn invalid_selection_order(nodes: &[&RollingNode]) -> Option<String> {
    if nodes.iter().any(|node| node.selection_order == 0) {
        return Some("selection order must start at one".to_string());
    }
    nodes
        .windows(2)
        .find(|pair| pair[0].selection_order == pair[1].selection_order)
        .map(|pair| format!("duplicate selection order {}", pair[0].selection_order))
}

async fn wait_with_cancellation(duration: Duration, context: &OperationContext<'_>) -> bool {
    let poll = Duration::from_millis(250);
    let started = Instant::now();
    while started.elapsed() < duration {
        if is_cancelled(context) {
            return true;
        }
        tokio::time::sleep(poll.min(duration.saturating_sub(started.elapsed()))).await;
    }
    is_cancelled(context)
}

struct OperationReporter<'a> {
    operation: OperationKind,
    target: NodeTarget,
    audit: &'a AuditLog,
    progress: &'a mut ProgressCallback,
    audit_error: Option<String>,
}

impl<'a> OperationReporter<'a> {
    fn new(
        request: &NodeOperationRequest,
        audit: &'a AuditLog,
        progress: &'a mut ProgressCallback,
    ) -> Self {
        Self {
            operation: request.operation,
            target: request.target.clone(),
            audit,
            progress,
            audit_error: None,
        }
    }

    fn emit(&mut self, phase: AuditPhase, step: OperationStep, message: impl Into<String>) {
        let message = message.into();
        (self.progress)(OperationsEvent::Operation(OperationProgressEvent {
            operation: self.operation,
            target: self.target.clone(),
            phase,
            step,
            message: message.clone(),
        }));
        self.record(phase, step, message);
    }

    fn finish(&mut self, mut result: NodeOperationResult) -> NodeOperationResult {
        let phase = match result.status {
            OperationStatus::Succeeded => AuditPhase::Success,
            OperationStatus::Cancelled => AuditPhase::Cancelled,
            OperationStatus::Failed
            | OperationStatus::Blocked
            | OperationStatus::NotConfirmed
            | OperationStatus::Unsupported
            | OperationStatus::NotStarted => AuditPhase::Failure,
        };
        self.emit(phase, OperationStep::Complete, result.message.clone());
        result.audit_error = self.audit_error.clone();
        result
    }

    fn record(&mut self, phase: AuditPhase, step: OperationStep, message: String) {
        let entry = AuditEntry {
            timestamp: Utc::now(),
            cluster: self.audit.cluster.clone(),
            actor: self.audit.actor.clone(),
            operation: self.operation,
            target: self.target.clone(),
            phase,
            step,
            message,
        };
        if let Err(error) = self.audit.append(&entry) {
            self.audit_error.get_or_insert_with(|| error.to_string());
        }
    }
}

struct RollingReporter<'a> {
    operation: OperationKind,
    audit: &'a AuditLog,
    progress: &'a mut ProgressCallback,
    audit_error: Option<String>,
}

impl<'a> RollingReporter<'a> {
    fn new(
        operation: OperationKind,
        audit: &'a AuditLog,
        progress: &'a mut ProgressCallback,
    ) -> Self {
        Self {
            operation,
            audit,
            progress,
            audit_error: None,
        }
    }

    fn emit(
        &mut self,
        current_node_index: usize,
        total_nodes: usize,
        target: Option<NodeTarget>,
        phase: AuditPhase,
        step: OperationStep,
        message: impl Into<String>,
    ) {
        let message = message.into();
        (self.progress)(OperationsEvent::Rolling(RollingProgressEvent {
            operation: self.operation,
            current_node_index,
            total_nodes,
            target: target.clone(),
            phase,
            step,
            message: message.clone(),
        }));
        let audit_target =
            target.unwrap_or_else(|| NodeTarget::new("rolling-operation", "not-applicable"));
        let entry = AuditEntry {
            timestamp: Utc::now(),
            cluster: self.audit.cluster.clone(),
            actor: self.audit.actor.clone(),
            operation: self.operation,
            target: audit_target,
            phase,
            step,
            message,
        };
        if let Err(error) = self.audit.append(&entry) {
            self.audit_error.get_or_insert_with(|| error.to_string());
        }
    }

    fn finish(&mut self, mut result: RollingOperationResult) -> RollingOperationResult {
        let phase = match result.status {
            OperationStatus::Succeeded => AuditPhase::Success,
            OperationStatus::Cancelled => AuditPhase::Cancelled,
            OperationStatus::Failed
            | OperationStatus::Blocked
            | OperationStatus::NotConfirmed
            | OperationStatus::Unsupported
            | OperationStatus::NotStarted => AuditPhase::Failure,
        };
        self.emit(
            result.nodes.len(),
            result.nodes.len(),
            None,
            phase,
            OperationStep::Complete,
            result.message.clone(),
        );
        result.audit_error = self.audit_error.clone();
        result
    }
}

#[derive(Debug, Serialize)]
struct SchedulingPatch {
    spec: SchedulingSpecPatch,
}

#[derive(Debug, Serialize)]
struct SchedulingSpecPatch {
    unschedulable: bool,
}

impl SchedulingPatch {
    const fn cordon() -> Self {
        Self {
            spec: SchedulingSpecPatch {
                unschedulable: true,
            },
        }
    }

    const fn uncordon() -> Self {
        Self {
            spec: SchedulingSpecPatch {
                unschedulable: false,
            },
        }
    }
}

fn node_is_unschedulable(node: &Node) -> bool {
    node.spec
        .as_ref()
        .and_then(|spec| spec.unschedulable)
        .unwrap_or(false)
}

fn node_is_ready(node: &Node) -> bool {
    node.status
        .as_ref()
        .and_then(|status| status.conditions.as_ref())
        .is_some_and(|conditions| {
            conditions
                .iter()
                .any(|condition| condition.type_ == "Ready" && condition.status == "True")
        })
}

fn is_mirror_pod(pod: &Pod) -> bool {
    pod.metadata
        .annotations
        .as_ref()
        .is_some_and(|annotations| annotations.contains_key("kubernetes.io/config.mirror"))
}

fn is_daemonset_pod(pod: &Pod) -> bool {
    pod.metadata
        .owner_references
        .as_ref()
        .is_some_and(|references| {
            references
                .iter()
                .any(|reference| reference.kind == "DaemonSet")
        })
}

fn is_controller_managed(pod: &Pod) -> bool {
    pod.metadata
        .owner_references
        .as_ref()
        .is_some_and(|references| {
            references.iter().any(|reference| {
                matches!(
                    reference.kind.as_str(),
                    "ReplicaSet" | "Deployment" | "StatefulSet" | "Job" | "DaemonSet"
                )
            })
        })
}

fn has_emptydir(pod: &Pod) -> bool {
    pod.spec.as_ref().is_some_and(|spec| {
        spec.volumes
            .as_ref()
            .is_some_and(|volumes| volumes.iter().any(|volume| volume.empty_dir.is_some()))
    })
}

fn is_not_found(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("404") || error.contains("not found")
}

fn is_pdb_block(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("429")
        || error.contains("disruption budget")
        || error.contains("poddisruptionbudget")
        || error.contains("cannot evict")
}

fn is_cancelled(context: &OperationContext<'_>) -> bool {
    (context.is_cancelled)()
}

/// The per-context audit file under `~/.freshkube`. The context name is
/// sanitized so it can never point outside that directory.
pub fn default_audit_path(context: &str) -> PathBuf {
    let safe: String = context
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .take(128)
        .collect();
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".freshkube")
        .join(format!(
            "operations-{}.yaml",
            if safe.is_empty() { "default" } else { &safe }
        ))
}

fn current_actor() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

// ---------------------------------------------------------------------------
// Frontend orchestration helpers shared by the desktop frontends: ordered
// selection, etcd impact from a real sample, Kubernetes identity prechecks,
// and outcome handling that never hides a partial failure.
// ---------------------------------------------------------------------------

/// Upper bound on targets in one run, so a hostile or broken roster can't create an unbounded
/// sequence.
pub const TARGET_LIMIT: usize = 128;

/// Pods listed per node by [`preflight_nodes`]; the count is a sample beyond this.
pub const POD_SAMPLE_LIMIT: u32 = 1000;

/// Toggles `target` in an ordered selection: removes it if present, otherwise appends it.
pub fn selection_toggle(nodes: &mut Vec<NodeTarget>, target: NodeTarget) {
    if let Some(index) = nodes.iter().position(|n| n == &target) {
        nodes.remove(index);
    } else if nodes.len() < TARGET_LIMIT {
        nodes.push(target);
    }
}

/// Moves the target at `index` one place earlier (`up`) or later in an ordered selection.
pub fn move_target(nodes: &mut [NodeTarget], index: usize, up: bool) {
    let other = if up {
        index.checked_sub(1)
    } else {
        index.checked_add(1).filter(|i| *i < nodes.len())
    };
    if let Some(other) = other {
        nodes.swap(index, other);
    }
}

/// Derives the etcd impact of taking `target` down from a correlated etcd sample.
///
/// A partial or uncorrelated sample is never classified: the result is
/// [`EtcdQuorumImpact::Unavailable`], which blocks reboot and shutdown.
pub fn etcd_impact(snapshot: &EtcdHealthSnapshot, target: &NodeTarget) -> EtcdQuorumImpact {
    if snapshot.is_partial()
        || snapshot.voting_members == 0
        || snapshot
            .members
            .iter()
            .filter(|m| !m.info.is_learner)
            .count()
            != snapshot.voting_members
        || !snapshot.unmatched_statuses.is_empty()
        || !snapshot.unmatched_alarms.is_empty()
    {
        return EtcdQuorumImpact::unavailable(
            "Detailed etcd sample is incomplete or cannot be correlated with membership",
        );
    }
    let member = snapshot.members.iter().find(|m| {
        m.info.hostname.eq_ignore_ascii_case(&target.name)
            || m.info.ip_address().is_some_and(|ip| ip == target.address)
    });
    EtcdQuorumImpact::known(
        member.is_some_and(|m| !m.info.is_learner),
        member.is_some_and(|m| !m.info.is_learner && m.is_reachable() && !m.has_problems()),
        snapshot
            .members
            .iter()
            .filter(|m| !m.info.is_learner && m.is_reachable() && !m.has_problems())
            .count(),
        snapshot.voting_members,
    )
}

/// Whether a Kubernetes node is the explicit `target`: the exact name and, separately, one of
/// its node addresses.
pub fn node_matches_target(node: &Node, target: &NodeTarget) -> bool {
    node.metadata.name.as_deref() == Some(target.name.as_str())
        && node
            .status
            .as_ref()
            .and_then(|s| s.addresses.as_ref())
            .is_some_and(|addresses| {
                addresses.iter().any(|a| {
                    matches!(a.type_.as_str(), "InternalIP" | "ExternalIP")
                        && a.address == target.address
                })
            })
}

/// What a prior check found for one target.
#[derive(Debug, Clone)]
pub struct NodePreflight {
    /// The checked target.
    pub target: NodeTarget,
    /// Whether Kubernetes reports the node Ready.
    pub ready: bool,
    /// Whether the node is already unschedulable.
    pub unschedulable: bool,
    /// Pods bound to the node (sampled up to [`POD_SAMPLE_LIMIT`]).
    pub pod_count: usize,
    /// Etcd impact from a fresh sample, or unavailable.
    pub impact: EtcdQuorumImpact,
}

/// Checks every target against Kubernetes (exact name plus address, readiness, scheduling, pod
/// count) and the current etcd sample. Any Kubernetes failure or identity mismatch is an error;
/// an etcd failure only makes the impact unavailable. Mutates nothing.
pub async fn preflight_nodes(
    kubernetes: &Client,
    talos: TalosClient,
    endpoint: crate::inspection::InspectionTarget,
    targets: &[NodeTarget],
) -> Result<Vec<NodePreflight>, String> {
    let nodes: Api<Node> = Api::all(kubernetes.clone());
    let pods: Api<Pod> = Api::all(kubernetes.clone());
    let mut found = Vec::with_capacity(targets.len());
    for target in targets {
        let node = nodes
            .get(&target.name)
            .await
            .map_err(|e| format!("Kubernetes precheck for {}: {e}", target.name))?;
        if !node_matches_target(&node, target) {
            return Err(format!(
                "Kubernetes identity mismatch for {} at {}; no mutation was made",
                target.name, target.address
            ));
        }
        let ready = node
            .status
            .as_ref()
            .and_then(|s| s.conditions.as_ref())
            .is_some_and(|conditions| {
                conditions
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True")
            });
        let pod_count = pods
            .list(
                &ListParams::default()
                    .fields(&format!("spec.nodeName={}", target.name))
                    .limit(POD_SAMPLE_LIMIT),
            )
            .await
            .map_err(|e| format!("Pod precheck for {}: {e}", target.name))?
            .items
            .len();
        found.push(NodePreflight {
            target: target.clone(),
            ready,
            unschedulable: node
                .spec
                .as_ref()
                .and_then(|s| s.unschedulable)
                .unwrap_or(false),
            pod_count,
            impact: EtcdQuorumImpact::unavailable("etcd was not sampled"),
        });
    }
    let etcd = crate::inspection::collect_etcd_health(
        talos,
        crate::inspection::EtcdInspectionRequest::new(endpoint),
    )
    .await;
    for node in &mut found {
        node.impact = match &etcd {
            Ok(snapshot) => etcd_impact(snapshot, &node.target),
            Err(error) => EtcdQuorumImpact::unavailable(error.to_string()),
        };
    }
    Ok(found)
}

/// A PodDisruptionBudget that currently allows no disruption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockingPdb {
    /// Namespace of the budget.
    pub namespace: String,
    /// Name of the budget.
    pub name: String,
}

/// PodDisruptionBudgets across all namespaces that allow no disruption while pods are expected.
/// These can stall a drain; the list is advisory and cluster-wide, not specific to one node.
pub async fn blocking_pdbs(kubernetes: &Client) -> Result<Vec<BlockingPdb>, String> {
    use k8s_openapi::api::policy::v1::PodDisruptionBudget;
    let api: Api<PodDisruptionBudget> = Api::all(kubernetes.clone());
    let list = api
        .list(&ListParams::default())
        .await
        .map_err(|e| format!("PodDisruptionBudget check: {e}"))?;
    Ok(list
        .items
        .into_iter()
        .filter(|pdb| {
            pdb.status
                .as_ref()
                .is_some_and(|status| status.disruptions_allowed == 0 && status.expected_pods > 0)
        })
        .map(|pdb| BlockingPdb {
            namespace: pdb.metadata.namespace.unwrap_or_default(),
            name: pdb.metadata.name.unwrap_or_default(),
        })
        .collect())
}

/// One result per target that never started (or was blocked/cancelled before mutation).
pub fn not_started(
    operation: OperationKind,
    targets: &[NodeTarget],
    message: &str,
    cancelled: bool,
) -> Vec<NodeOperationResult> {
    targets
        .iter()
        .map(|target| NodeOperationResult {
            operation,
            target: target.clone(),
            status: if cancelled {
                OperationStatus::Cancelled
            } else {
                OperationStatus::Blocked
            },
            scheduling: SchedulingState::Unchanged,
            drain: None,
            message: message.to_owned(),
            audit_error: None,
        })
        .collect()
}

/// Results for targets intentionally not started after an earlier stop.
pub fn skipped(
    operation: OperationKind,
    targets: &[NodeTarget],
    message: &str,
) -> Vec<NodeOperationResult> {
    let mut results = not_started(operation, targets, message, false);
    for result in &mut results {
        result.status = OperationStatus::NotStarted;
    }
    results
}

/// Outcomes after a worker panicked: the node in flight is a failure with unknown scheduling,
/// never a success, and the rest are not started. `completed` are the results already retained.
pub fn panic_outcomes(
    operation: OperationKind,
    targets: &[NodeTarget],
    mut completed: Vec<NodeOperationResult>,
) -> Vec<NodeOperationResult> {
    let next = completed.len();
    if let Some(target) = targets.get(next) {
        completed.push(NodeOperationResult {
            operation,
            target: target.clone(),
            status: OperationStatus::Failed,
            scheduling: SchedulingState::Unknown(
                "Worker panicked; inspect actual node state".to_owned(),
            ),
            drain: None,
            message: "Worker panicked; this node's outcome is unknown, not successful".to_owned(),
            audit_error: Some(
                "Worker panicked before final core audit could be guaranteed".to_owned(),
            ),
        });
        completed.extend(skipped(
            operation,
            &targets[next + 1..],
            "Not started after the operation worker panicked",
        ));
    }
    completed
}

/// Appends a frontend-level outcome record for `result` to the audit log. A failed write is
/// recorded on the result rather than lost.
pub fn audit_result(audit: &AuditLog, context: &str, result: &mut NodeOperationResult) {
    let entry = AuditEntry {
        timestamp: Utc::now(),
        cluster: context.to_owned(),
        actor: current_actor(),
        operation: result.operation,
        target: result.target.clone(),
        phase: if result.status.is_success() {
            AuditPhase::Success
        } else if result.status == OperationStatus::Cancelled {
            AuditPhase::Cancelled
        } else {
            AuditPhase::Failure
        },
        step: OperationStep::Complete,
        message: format!(
            "Frontend sequence outcome {:?}: {}",
            result.status, result.message
        ),
    };
    if let Err(error) = audit.append(&entry) {
        result.audit_error.get_or_insert_with(|| error.to_string());
    }
}

/// Runs `execute` for each target in order. Cancellation always stops the sequence (also during
/// the inter-node delay), `stop_on_failure` stops after a node that did not succeed, and every
/// target keeps an explicit result: the rest are [`OperationStatus::NotStarted`].
pub async fn ordered_sequence<F, Fut>(
    operation: OperationKind,
    targets: &[NodeTarget],
    stop_on_failure: bool,
    delay: Duration,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    mut execute: F,
) -> Vec<NodeOperationResult>
where
    F: FnMut(usize, NodeTarget) -> Fut,
    Fut: std::future::Future<Output = NodeOperationResult>,
{
    let mut results = Vec::with_capacity(targets.len());
    for (index, target) in targets.iter().enumerate() {
        if is_cancelled() {
            results.extend(not_started(
                operation,
                std::slice::from_ref(target),
                "Cancelled before this node started",
                true,
            ));
            results.extend(skipped(
                operation,
                &targets[index + 1..],
                "Not started because the sequence was cancelled",
            ));
            break;
        }
        let result = execute(index, target.clone()).await;
        let stop = result.status == OperationStatus::Cancelled
            || (stop_on_failure && !result.status.is_success());
        results.push(result);
        if stop {
            results.extend(skipped(
                operation,
                &targets[index + 1..],
                "Not started after a previous node failed, was blocked, or was cancelled",
            ));
            break;
        }
        if index + 1 < targets.len() && !delay.is_zero() {
            let deadline = tokio::time::Instant::now() + delay;
            while tokio::time::Instant::now() < deadline && !is_cancelled() {
                tokio::time::sleep(
                    Duration::from_millis(100)
                        .min(deadline.saturating_duration_since(tokio::time::Instant::now())),
                )
                .await;
            }
            if is_cancelled() {
                results.extend(skipped(
                    operation,
                    &targets[index + 1..],
                    "Not started because the sequence was cancelled during inter-node delay",
                ));
                break;
            }
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_path_is_context_local_and_cannot_escape_directory() {
        assert_eq!(
            default_audit_path("../../prod").file_name().unwrap(),
            "operations-.._.._prod.yaml"
        );
        assert_eq!(
            default_audit_path("").file_name().unwrap(),
            "operations-default.yaml"
        );
        assert!(
            default_audit_path("../../prod").ends_with(".freshkube/operations-.._.._prod.yaml")
        );
    }

    fn observed_node(ready: &str) -> RebootObservation {
        use k8s_openapi::api::core::v1::{NodeCondition, NodeStatus};
        let node = Node {
            status: Some(NodeStatus {
                conditions: Some(vec![NodeCondition {
                    type_: "Ready".to_string(),
                    status: ready.to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        RebootObservation::from_response(Ok(Some(node)))
    }

    fn unavailable_observation() -> RebootObservation {
        RebootObservation::from_response(Err(kube::Error::Api(kube::error::ErrorResponse {
            status: "Failure".to_string(),
            message: "API temporarily unavailable".to_string(),
            reason: "ServiceUnavailable".to_string(),
            code: 503,
        })))
    }

    #[test]
    fn api_error_then_still_ready_never_proves_reboot_completion() {
        let mut sequence = RebootReadiness::new(Duration::from_secs(120));
        for (seconds, observation) in [
            (0, unavailable_observation()),
            (2, observed_node("True")),
            (4, observed_node("True")),
        ] {
            assert!(matches!(
                sequence.advance(Some(observation), Duration::from_secs(seconds), false),
                RebootReadinessDecision::Waiting(_)
            ));
        }
        assert!(!sequence.observed_transition);
        let RebootReadinessDecision::Finished(ReadyWait::Failed(message)) =
            sequence.advance(None, Duration::from_secs(60), false)
        else {
            panic!("Unproven reboot must fail at the transition deadline");
        };
        assert!(message.contains("Reboot transition was not observed"));
        assert!(message.contains("scheduling was not restored"));
    }

    #[test]
    fn observed_not_ready_then_ready_completes_reboot() {
        for condition in ["False", "Unknown"] {
            let mut sequence = RebootReadiness::new(Duration::from_secs(120));
            assert!(matches!(
                sequence.advance(Some(unavailable_observation()), Duration::ZERO, false),
                RebootReadinessDecision::Waiting(_)
            ));
            assert!(matches!(
                sequence.advance(Some(observed_node(condition)), Duration::ZERO, false),
                RebootReadinessDecision::Waiting(_)
            ));
            // Unavailability after an actual transition cannot complete it either.
            assert!(matches!(
                sequence.advance(
                    Some(unavailable_observation()),
                    Duration::from_secs(2),
                    false
                ),
                RebootReadinessDecision::Waiting(_)
            ));
            assert!(matches!(
                sequence.advance(Some(observed_node("True")), Duration::from_secs(4), false),
                RebootReadinessDecision::Finished(ReadyWait::Ready(elapsed))
                    if elapsed == Duration::from_secs(4)
            ));
        }
    }

    #[test]
    fn authoritative_missing_node_then_ready_is_a_transition() {
        let mut sequence = RebootReadiness::new(Duration::from_secs(120));
        assert!(matches!(
            sequence.advance(
                Some(RebootObservation::from_response(Ok(None))),
                Duration::ZERO,
                false
            ),
            RebootReadinessDecision::Waiting(_)
        ));
        assert!(matches!(
            sequence.advance(Some(observed_node("True")), Duration::from_secs(2), false),
            RebootReadinessDecision::Finished(ReadyWait::Ready(_))
        ));
    }

    #[test]
    fn repeated_unavailability_times_out_or_cancels_without_transition() {
        let mut sequence = RebootReadiness::new(Duration::from_secs(10));
        for seconds in 0..10 {
            assert!(matches!(
                sequence.advance(
                    Some(unavailable_observation()),
                    Duration::from_secs(seconds),
                    false
                ),
                RebootReadinessDecision::Waiting(_)
            ));
        }
        assert!(!sequence.observed_transition);
        assert!(matches!(
            sequence.advance(
                Some(unavailable_observation()),
                Duration::from_secs(10),
                false
            ),
            RebootReadinessDecision::Finished(ReadyWait::Failed(_))
        ));
        let mut cancelling = RebootReadiness::new(Duration::from_secs(10));
        for seconds in 0..3 {
            assert!(matches!(
                cancelling.advance(
                    Some(unavailable_observation()),
                    Duration::from_secs(seconds),
                    false
                ),
                RebootReadinessDecision::Waiting(_)
            ));
        }
        assert!(matches!(
            cancelling.advance(
                Some(unavailable_observation()),
                Duration::from_secs(3),
                true
            ),
            RebootReadinessDecision::Finished(ReadyWait::Cancelled)
        ));
    }

    #[test]
    fn missing_ready_condition_is_unavailable_not_transition() {
        let mut sequence = RebootReadiness::new(Duration::from_secs(120));
        let observation = RebootObservation::from_response(Ok(Some(Node::default())));
        assert!(matches!(&observation, RebootObservation::Unavailable(_)));
        assert!(matches!(
            sequence.advance(Some(observation), Duration::ZERO, false),
            RebootReadinessDecision::Waiting(_)
        ));
        assert!(!sequence.observed_transition);
    }

    #[test]
    fn observed_transition_without_ready_still_times_out() {
        let mut sequence = RebootReadiness::new(Duration::from_secs(120));
        sequence.advance(Some(observed_node("False")), Duration::ZERO, false);
        let RebootReadinessDecision::Finished(ReadyWait::Failed(message)) =
            sequence.advance(None, Duration::from_secs(120), false)
        else {
            panic!("NotReady alone is not reboot completion");
        };
        assert!(message.contains("waiting for Kubernetes Ready"));
    }

    #[test]
    fn already_ready_without_observed_reboot_transition_is_not_success() {
        let error = require_reboot_transition(false).unwrap_err();
        assert!(error.contains("already-Ready"));
        assert!(error.contains("scheduling was not restored"));
        assert!(require_reboot_transition(true).is_ok());
        let mut sequence = RebootReadiness::new(Duration::from_secs(120));
        for seconds in [0, 2, 59] {
            assert!(matches!(
                sequence.advance(
                    Some(observed_node("True")),
                    Duration::from_secs(seconds),
                    false
                ),
                RebootReadinessDecision::Waiting(_)
            ));
        }
        assert!(matches!(
            sequence.advance(Some(observed_node("True")), Duration::from_secs(60), false),
            RebootReadinessDecision::Finished(ReadyWait::Failed(_))
        ));
    }

    #[test]
    fn evaluates_single_member_etcd_reboot_as_unsafe() {
        let safety = evaluate_operation_safety(
            OperationKind::Reboot,
            &EtcdQuorumImpact::known(true, true, 1, 1),
        );
        assert!(matches!(safety, SafetyStatus::Unsafe(_)));
    }

    #[test]
    fn preserves_explicit_rolling_selection_order() {
        let target = |name| NodeTarget::new(name, format!("{name}.example"));
        let request = RollingOperationRequest::new(
            OperationKind::Drain,
            vec![
                RollingNode {
                    target: target("third"),
                    etcd_impact: EtcdQuorumImpact::unavailable("not needed for drain"),
                    selection_order: 3,
                },
                RollingNode {
                    target: target("first"),
                    etcd_impact: EtcdQuorumImpact::unavailable("not needed for drain"),
                    selection_order: 1,
                },
                RollingNode {
                    target: target("second"),
                    etcd_impact: EtcdQuorumImpact::unavailable("not needed for drain"),
                    selection_order: 2,
                },
            ],
        );

        let names = request
            .ordered_nodes()
            .into_iter()
            .map(|node| node.target.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["first", "second", "third"]);
    }

    #[test]
    fn structured_audit_history_round_trips() {
        let path = std::env::temp_dir().join(format!(
            "freshkube-operations-audit-{}-{}.yaml",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let audit = AuditLog::with_actor(&path, "test-cluster", "test-user");
        let entry = AuditEntry {
            timestamp: Utc::now(),
            cluster: "test-cluster".to_string(),
            actor: "test-user".to_string(),
            operation: OperationKind::Drain,
            target: NodeTarget::new("node-a", "10.0.0.1"),
            phase: AuditPhase::Progress,
            step: OperationStep::EvictPod,
            message: "Evicting default/api".to_string(),
        };

        let mut completed = entry.clone();
        completed.phase = AuditPhase::Success;
        completed.step = OperationStep::Complete;
        completed.message = "Drain completed".to_string();

        audit.append(&entry).unwrap();
        audit.append(&completed).unwrap();
        assert_eq!(audit.read_entries().unwrap(), vec![entry, completed]);
        std::fs::remove_file(path).unwrap();
    }

    fn pair(name: &str) -> NodeTarget {
        NodeTarget::new(name, format!("10.0.0.{}", name.len()))
    }

    #[test]
    fn ordered_selection_toggles_and_moves() {
        let mut selected = Vec::new();
        selection_toggle(&mut selected, pair("b"));
        selection_toggle(&mut selected, pair("aa"));
        selection_toggle(&mut selected, pair("ccc"));
        assert_eq!(selected, vec![pair("b"), pair("aa"), pair("ccc")]);
        move_target(&mut selected, 2, true);
        move_target(&mut selected, 0, true);
        move_target(&mut selected, 2, false);
        assert_eq!(selected, vec![pair("b"), pair("ccc"), pair("aa")]);
        selection_toggle(&mut selected, pair("ccc"));
        assert_eq!(selected, vec![pair("b"), pair("aa")]);
    }

    #[tokio::test]
    async fn ordered_sequence_keeps_every_target_and_stops_on_cancellation() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let targets = vec![pair("a"), pair("bb"), pair("ccc")];
        let cancelled = AtomicBool::new(false);
        let predicate = || cancelled.load(Ordering::SeqCst);
        let results = ordered_sequence(
            OperationKind::Drain,
            &targets,
            false,
            Duration::ZERO,
            &predicate,
            |index, target| {
                if index == 0 {
                    cancelled.store(true, Ordering::SeqCst);
                }
                let mut result =
                    not_started(OperationKind::Drain, &[target], "done", false).remove(0);
                result.status = OperationStatus::Succeeded;
                std::future::ready(result)
            },
        )
        .await;
        let statuses: Vec<_> = results.iter().map(|r| r.status).collect();
        assert_eq!(
            statuses,
            [
                OperationStatus::Succeeded,
                OperationStatus::Cancelled,
                OperationStatus::NotStarted
            ]
        );
        let outcomes = panic_outcomes(OperationKind::Drain, &targets, vec![results[0].clone()]);
        assert_eq!(outcomes[1].status, OperationStatus::Failed);
        assert!(matches!(
            outcomes[1].scheduling,
            SchedulingState::Unknown(_)
        ));
        assert_eq!(outcomes[2].status, OperationStatus::NotStarted);
    }

    #[test]
    fn partial_etcd_sample_is_unavailable_never_safe() {
        use crate::inspection::InspectionTarget;
        let snapshot = EtcdHealthSnapshot {
            target: InspectionTarget::new("cp", "10.0.0.10"),
            members: vec![],
            unmatched_statuses: vec![],
            unmatched_alarms: vec![],
            quorum: QuorumState::Unknown,
            voting_members: 3,
            responding_voting_members: 0,
            reported_leader_ids: vec![],
            largest_database_size: 0,
            revision: 0,
            unavailable: vec![],
        };
        let impact = etcd_impact(&snapshot, &pair("a"));
        assert!(matches!(impact, EtcdQuorumImpact::Unavailable { .. }));
        assert_eq!(
            evaluate_operation_safety(OperationKind::Reboot, &impact),
            SafetyStatus::Unknown
        );
    }
}
