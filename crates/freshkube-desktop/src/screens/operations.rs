//! Operations: cordon, uncordon, drain, reboot and shutdown of one node or an
//! ordered selection of nodes (a rolling run), with a preview before anything
//! is sent.
//!
//! The preview (preflight) reads the real sources: Kubernetes for node
//! identity, readiness, scheduling and pods, and a fresh etcd sample for quorum
//! impact. Reboot and shutdown are refused when etcd is unsafe, and when etcd
//! is unknown they are refused too and shown as unknown, never as safe. Drain,
//! cordon and uncordon don't need etcd. Confirming goes through
//! [`crate::mutation::confirm`], whose fingerprint covers everything the user
//! saw; the run takes the app-wide operation slot, is audited by the core
//! runner, and is bound to the context it was started in.
//!
//! Example data (`--fixture`) simulates a run with timed fake steps. It takes
//! and releases the slot and honours cancellation, but never reads or writes
//! the audit file and never touches the filesystem or the network.
//!
//! Element ids (all start with `ops-`):
//! - page and notices: `ops-page`, `ops-notice`, `ops-busy`;
//! - operation choice: `ops-kinds`, `ops-kind-{cordon,uncordon,drain,reboot,shutdown}`;
//! - options: `ops-options`, `ops-opt-*` (toggles, and `-dec`/`-inc` steppers);
//! - nodes: `ops-node-scroll`, `ops-nodes`, `ops-node` (row `ix`);
//! - plan: `ops-plan`, `ops-plan-target` (row `k`), `ops-move-earlier`,
//!   `ops-move-later` (button `k`), `ops-preview`, `ops-verdict` (row `k`),
//!   `ops-blocked`, `ops-review`, `ops-clear`;
//! - run: `ops-run`, `ops-run-context`, `ops-run-status`, `ops-run-cancel`,
//!   `ops-progress`, `ops-result` (row `i`);
//! - audit: `ops-audit`, `ops-audit-note`, `ops-audit-entry` (row `i`).
use std::{
    collections::{VecDeque, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    panic::AssertUnwindSafe,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use freshkube_core::{
    indicators::SafetyStatus,
    inspection::{
        EtcdHealthSnapshot, InspectionSource, InspectionTarget, InspectionUnavailable,
        assemble_etcd_health,
    },
    operations::{
        AuditEntry, AuditPhase, BlockingPdb, DrainOptions, DrainSummary, EtcdQuorumImpact,
        NodeOperationResult, NodeTarget, OperationConfirmation, OperationKind,
        OperationProgressEvent, OperationStatus, OperationStep, OperationsEvent, POD_SAMPLE_LIMIT,
        PREFLIGHT_TIMEOUT, PodReference, RollingProgressEvent, SchedulingState, SelectionEvent,
        SelectionRequest, blocking_pdbs, etcd_impact, evaluate_operation_safety, move_target,
        not_started, ordered_sequence, panic_outcomes, preflight_nodes, run_selection,
        selection_blocked_reason as first_blocking, selection_toggle,
    },
};
use futures::FutureExt;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use talos_rs::{EtcdMemberInfo, EtcdMemberStatus};
use tokio::{runtime::Handle, sync::mpsc};

use super::{
    Column, Loader, SCREEN_DEADLINE, Scope, ScreenEvent, ScreenPanel, ScreenSource, cell,
    content_width, header, panel, retry_button, table_head, table_width,
};
use crate::backend::{self, OwnedJob};
use crate::mutation::{self, Confirmation, Operations};
use crate::palette::palette;
use crate::presentation::Role as NodeRole;
use crate::ui::{self, MONO_FONT, Tone, dp};

const CONTEXT: &str = "TalosOperations";
const ROW_HEIGHT: f32 = 30.;
/// The plan pane, when it sits beside the roster.
const PLAN_WIDTH: f32 = 400.;
const GAP: f32 = 14.;
const PROGRESS_LIMIT: usize = 256;
const AUDIT_ENTRIES: usize = 25;
/// How long one simulated step takes.
const SIMULATED_STEP: Duration = Duration::from_millis(650);

const COLUMNS: [Column; 4] = [
    Column {
        label: "Order",
        width: Some(72.),
    },
    Column {
        label: "Node",
        width: None,
    },
    Column {
        label: "Address",
        width: Some(132.),
    },
    Column {
        label: "Role",
        width: Some(116.),
    },
];

actions!(
    talos_operations,
    [
        NextNode,
        PreviousNode,
        FirstNode,
        LastNode,
        ToggleNode,
        MoveEarlier,
        MoveLater,
        ClearSelection
    ]
);

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

const KINDS: [OperationKind; 5] = [
    OperationKind::Cordon,
    OperationKind::Uncordon,
    OperationKind::Drain,
    OperationKind::Reboot,
    OperationKind::Shutdown,
];

fn kind_id(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Cordon => "ops-kind-cordon",
        OperationKind::Uncordon => "ops-kind-uncordon",
        OperationKind::Drain => "ops-kind-drain",
        OperationKind::Reboot => "ops-kind-reboot",
        OperationKind::Shutdown => "ops-kind-shutdown",
    }
}

fn kind_title(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Cordon => "Cordon",
        OperationKind::Uncordon => "Uncordon",
        OperationKind::Drain => "Drain",
        OperationKind::Reboot => "Reboot",
        OperationKind::Shutdown => "Shutdown",
    }
}

fn kind_blurb(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Cordon => "Stop new pods from being scheduled. Running pods stay.",
        OperationKind::Uncordon => "Allow pods to be scheduled again.",
        OperationKind::Drain => "Cordon, then evict the pods that can be evicted.",
        OperationKind::Reboot => {
            "Drain, then ask Talos to reboot. Needs a verified etcd quorum impact."
        }
        OperationKind::Shutdown => {
            "Drain, then ask Talos to shut down. The node stays down until it is started by hand. Needs a verified etcd quorum impact."
        }
    }
}

/// Whether the operation evicts pods, and so uses the drain options.
fn drains(kind: OperationKind) -> bool {
    matches!(
        kind,
        OperationKind::Drain | OperationKind::Reboot | OperationKind::Shutdown
    )
}

/// What the user can set for a run. The drain options are core's; the rest is
/// how a multi-node run proceeds.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Options {
    drain: DrainOptions,
    stop_on_failure: bool,
    delay_secs: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            drain: DrainOptions::default(),
            stop_on_failure: true,
            delay_secs: 30,
        }
    }
}

const GRACE_CHOICES: [Option<u32>; 5] = [None, Some(0), Some(15), Some(30), Some(60)];

fn stepped(value: u64, up: bool, step: u64, min: u64, max: u64) -> u64 {
    if up {
        value.saturating_add(step).min(max)
    } else {
        value.saturating_sub(step).max(min)
    }
}

fn grace_text(grace: Option<u32>) -> String {
    grace.map_or("default".to_owned(), |secs| format!("{secs} s"))
}

impl Options {
    /// The options that apply to this run, as (label, value) lines for the
    /// confirmation. Options that can't affect the run aren't listed.
    fn lines(&self, kind: OperationKind, targets: usize) -> Vec<(&'static str, String)> {
        let yes = |on: bool| if on { "yes" } else { "no" };
        let mut lines = Vec::new();
        if drains(kind) {
            let d = &self.drain;
            lines.push((
                "Drain",
                format!(
                    "{} per pod · DaemonSet pods {} · emptyDir data {}",
                    format_secs(d.per_pod_timeout_secs),
                    if d.ignore_daemonsets {
                        "skipped"
                    } else {
                        "evicted"
                    },
                    if d.delete_emptydir_data {
                        "deleted"
                    } else {
                        "blocks the drain"
                    },
                ),
            ));
            lines.push((
                "Unmanaged pods",
                if d.force_delete_unmanaged {
                    format!(
                        "force-deleted when eviction fails (grace {})",
                        grace_text(d.force_delete_grace_period_secs)
                    )
                } else {
                    "never force-deleted".to_owned()
                },
            ));
        }
        if kind == OperationKind::Reboot {
            let d = &self.drain;
            lines.push((
                "After the reboot",
                if d.wait_for_node_ready {
                    format!(
                        "wait up to {} for Ready · uncordon afterwards: {}",
                        format_secs(d.post_reboot_timeout_secs),
                        yes(d.uncordon_after_reboot)
                    )
                } else {
                    "not waited for; the result only says the request was sent".to_owned()
                },
            ));
        }
        if targets > 1 {
            lines.push((
                "Sequence",
                format!(
                    "one node at a time, in the order shown · {} · {} between nodes",
                    if self.stop_on_failure {
                        "stop at the first failure"
                    } else {
                        "continue after a failure"
                    },
                    format_secs(self.delay_secs)
                ),
            ));
        }
        lines
    }
}

fn format_secs(secs: u64) -> String {
    if secs >= 120 && secs.is_multiple_of(60) {
        format!("{} min", secs / 60)
    } else {
        format!("{secs} s")
    }
}

/// A node of the roster, as selectable here.
#[derive(Clone, Debug)]
struct RosterNode {
    target: NodeTarget,
    role: NodeRole,
    responding: bool,
}

/// What a preview was taken for. Options aren't part of it: they don't change
/// what the sources say, only what the confirmation shows.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PreviewKey {
    context: String,
    epoch: u64,
    operation: OperationKind,
    targets: Vec<NodeTarget>,
}

#[derive(Clone, Debug)]
struct Facts {
    ready: bool,
    unschedulable: bool,
    pods: usize,
}

#[derive(Clone, Debug)]
struct NodeView {
    target: NodeTarget,
    /// `None` when Kubernetes wasn't read for this target (never in live mode:
    /// an unreadable node fails the whole preview).
    facts: Option<Facts>,
    impact: EtcdQuorumImpact,
    safety: SafetyStatus,
}

#[derive(Clone, Debug)]
enum PdbNote {
    /// The operation doesn't evict pods.
    NotNeeded,
    None,
    Blocking(Vec<BlockingPdb>),
    Unknown(String),
}

#[derive(Clone, Debug)]
struct Preview {
    key: PreviewKey,
    nodes: Vec<NodeView>,
    pdb: PdbNote,
    /// The sources behind the preview, e.g. the example data note.
    source: String,
}

enum PreviewState {
    /// Nothing selected, or not requested yet.
    Idle,
    Loading(PreviewKey),
    Ready(Preview),
    Failed {
        key: PreviewKey,
        error: String,
    },
}

impl PreviewState {
    fn key(&self) -> Option<&PreviewKey> {
        match self {
            PreviewState::Idle => None,
            PreviewState::Loading(key) | PreviewState::Failed { key, .. } => Some(key),
            PreviewState::Ready(preview) => Some(&preview.key),
        }
    }
}

struct Verdict {
    tone: Tone,
    /// Marks only the Outline verdict; status tones draw their glyph.
    icon: Option<IconName>,
    label: &'static str,
    detail: String,
    /// Reboot and shutdown refuse on this.
    blocks: bool,
}

fn impact_detail(impact: &EtcdQuorumImpact) -> String {
    match impact {
        EtcdQuorumImpact::Known {
            target_is_member,
            target_is_healthy,
            healthy_members,
            total_members,
        } => format!(
            "etcd {healthy_members}/{total_members} healthy · {}",
            match (target_is_member, target_is_healthy) {
                (false, _) => "not an etcd member",
                (true, true) => "an etcd member, healthy",
                (true, false) => "an etcd member, not healthy",
            }
        ),
        EtcdQuorumImpact::Unavailable { reason } => format!("etcd not read: {reason}"),
    }
}

/// What a node's safety means for `kind`. Unknown is never shown as safe.
fn verdict(kind: OperationKind, node: &NodeView) -> Verdict {
    if !kind.is_destructive() {
        return Verdict {
            tone: Tone::Outline,
            icon: Some(IconName::Info),
            label: "No etcd impact",
            detail: format!(
                "{} doesn't take the node down; etcd isn't consulted.",
                kind_title(kind)
            ),
            blocks: false,
        };
    }
    match &node.safety {
        SafetyStatus::Safe => Verdict {
            tone: Tone::Good,
            icon: None,
            label: "Safe",
            detail: impact_detail(&node.impact),
            blocks: false,
        },
        SafetyStatus::Warning(reason) => Verdict {
            tone: Tone::Warn,
            icon: None,
            label: "Warning",
            detail: reason.clone(),
            blocks: false,
        },
        SafetyStatus::Unsafe(reason) => Verdict {
            tone: Tone::Crit,
            icon: None,
            label: "Unsafe",
            detail: reason.clone(),
            blocks: true,
        },
        SafetyStatus::Unknown => Verdict {
            tone: Tone::Unknown,
            icon: None,
            label: "Unknown",
            detail: impact_detail(&node.impact),
            blocks: true,
        },
    }
}

/// Why the plan can't run, if it can't. `None` means it may be reviewed and
/// confirmed (with the preview's warnings).
fn blocked_reason(state: &PreviewState, kind: OperationKind) -> Option<String> {
    match state {
        PreviewState::Idle => Some("Select at least one node.".into()),
        PreviewState::Loading(_) => Some("Checking the nodes and etcd.".into()),
        PreviewState::Failed { error, .. } => Some(format!(
            "The preview failed, so nothing is known about these nodes. {error}"
        )),
        PreviewState::Ready(preview) => preview.nodes.iter().find_map(|node| {
            let verdict = verdict(kind, node);
            verdict.blocks.then(|| {
                format!(
                    "{} is refused for {}: etcd impact is {}. {}",
                    kind_title(kind),
                    node.target.name,
                    verdict.label.to_lowercase(),
                    verdict.detail
                )
            })
        }),
    }
}

/// A hash of everything the user was shown: the context and target epoch, the
/// operation, the ordered targets with addresses, the options and each
/// verdict. A confirmation only goes through while the screen still produces
/// the same hash.
fn fingerprint(preview: &Preview, options: &Options) -> u64 {
    let mut hasher = DefaultHasher::new();
    let key = &preview.key;
    key.context.hash(&mut hasher);
    key.epoch.hash(&mut hasher);
    key.operation.label().hash(&mut hasher);
    for target in &key.targets {
        target.name.hash(&mut hasher);
        target.address.hash(&mut hasher);
    }
    format!("{options:?}").hash(&mut hasher);
    for node in &preview.nodes {
        format!("{:?}", node.safety).hash(&mut hasher);
    }
    hasher.finish()
}

// ---------------------------------------------------------------------------
// Runs
// ---------------------------------------------------------------------------

/// Everything a run needs, fixed when the user confirmed.
#[derive(Clone)]
struct RunPlan {
    context: String,
    operation: OperationKind,
    targets: Vec<NodeTarget>,
    options: Options,
    endpoint: InspectionTarget,
    /// Set for example data.
    world: Option<ExampleWorld>,
    step: Duration,
}

/// What the example cluster looks like to a simulated run.
#[derive(Clone)]
struct ExampleWorld {
    /// Nodes that don't answer the Talos API.
    unreachable: Vec<String>,
    impacts: Vec<EtcdQuorumImpact>,
}

enum RunEvent {
    Progress(OperationsEvent),
    NodeDone(NodeOperationResult),
    Done(Done),
}

struct Done {
    results: Vec<NodeOperationResult>,
    /// Why nothing (or not everything) ran, when the run didn't get to start.
    note: Option<String>,
    /// Simulated audit records; the real audit is the core's file.
    audit: Vec<AuditEntry>,
}

type Cancel = Arc<dyn Fn() -> bool + Send + Sync>;
type Retained = Arc<Mutex<Vec<NodeOperationResult>>>;

/// What the screen shows for a run: its own context and targets, live
/// progress, and a result for every target.
struct RunView {
    id: u64,
    context: String,
    operation: OperationKind,
    targets: Vec<NodeTarget>,
    example: bool,
    progress: VecDeque<String>,
    dropped: usize,
    results: Vec<NodeOperationResult>,
    note: Option<String>,
    finished: bool,
    cancel_requested: bool,
}

impl RunView {
    fn push(&mut self, line: String) {
        if self.progress.len() == PROGRESS_LIMIT {
            self.progress.pop_front();
            self.dropped += 1;
        }
        self.progress.push_back(line.chars().take(2048).collect());
    }

    fn count(&self, status: OperationStatus) -> usize {
        self.results.iter().filter(|r| r.status == status).count()
    }
}

fn step_label(step: OperationStep) -> &'static str {
    match step {
        OperationStep::Preflight => "preflight",
        OperationStep::InspectScheduling => "inspect scheduling",
        OperationStep::Cordon => "cordon",
        OperationStep::Uncordon => "uncordon",
        OperationStep::ListPods => "list pods",
        OperationStep::EvictPod => "evict pods",
        OperationStep::ForceDeletePod => "force delete",
        OperationStep::Reboot => "reboot",
        OperationStep::Shutdown => "shutdown",
        OperationStep::WaitForReady => "wait for Ready",
        OperationStep::RestoreScheduling => "restore scheduling",
        OperationStep::InterNodeDelay => "delay",
        OperationStep::Complete => "result",
    }
}

fn phase_label(phase: AuditPhase) -> &'static str {
    match phase {
        AuditPhase::Start => "start",
        AuditPhase::Progress => "progress",
        AuditPhase::Success => "success",
        AuditPhase::Failure => "failure",
        AuditPhase::Cancelled => "cancelled",
    }
}

/// A progress line for the log, and a short step for the status bar.
fn describe(event: &OperationsEvent) -> (String, String) {
    match event {
        OperationsEvent::Operation(e) => (
            format!(
                "{} ({}) · {}: {}",
                e.target.name,
                e.target.address,
                step_label(e.step),
                e.message
            ),
            format!("{} · {}", e.target.name, e.message),
        ),
        OperationsEvent::Rolling(e) => (
            format!(
                "Rolling {}/{} · {}: {}",
                e.current_node_index + 1,
                e.total_nodes,
                step_label(e.step),
                e.message
            ),
            format!(
                "{}/{} · {}",
                e.current_node_index + 1,
                e.total_nodes,
                e.message
            ),
        ),
    }
}

fn status_label(status: OperationStatus) -> (Tone, Option<IconName>, &'static str) {
    match status {
        OperationStatus::Succeeded => (Tone::Good, None, "Completed"),
        OperationStatus::Failed => (Tone::Crit, None, "Failed"),
        OperationStatus::Cancelled => (Tone::Warn, None, "Cancelled"),
        OperationStatus::Blocked => (Tone::Warn, None, "Blocked"),
        OperationStatus::NotConfirmed => (Tone::Warn, None, "Not confirmed"),
        OperationStatus::Unsupported => (Tone::Unknown, None, "Unsupported"),
        OperationStatus::NotStarted => (Tone::Outline, Some(IconName::Pause), "Skipped"),
    }
}

fn scheduling_text(state: &SchedulingState) -> String {
    match state {
        SchedulingState::Unchanged => "Scheduling unchanged".into(),
        SchedulingState::AlreadyCordoned => "Already cordoned before this run".into(),
        SchedulingState::CordonedByOperation => "Cordoned by this run".into(),
        SchedulingState::Schedulable => "Schedulable".into(),
        SchedulingState::Unknown(detail) => format!("Scheduling unknown: {detail}"),
    }
}

fn drain_text(drain: &DrainSummary) -> String {
    let pods = |pods: &[PodReference]| {
        let shown: Vec<String> = pods.iter().take(4).map(|pod| pod.to_string()).collect();
        let more = pods.len().saturating_sub(shown.len());
        if more > 0 {
            format!("{} and {more} more", shown.join(", "))
        } else {
            shown.join(", ")
        }
    };
    let mut text = format!(
        "{} of {} eligible pods evicted",
        drain.evicted_pods, drain.eligible_pods
    );
    if !drain.force_deleted_pods.is_empty() {
        text.push_str(&format!(
            " · {} force-deleted ({})",
            drain.force_deleted_pods.len(),
            pods(&drain.force_deleted_pods)
        ));
    }
    if !drain.failed_pods.is_empty() {
        text.push_str(&format!(
            " · {} could not be evicted ({})",
            drain.failed_pods.len(),
            pods(&drain.failed_pods)
        ));
    }
    text
}

/// Overall outcome of a finished run. Counts every state, so a partial
/// failure is never folded into one status.
fn run_summary(run: &RunView) -> (Tone, String) {
    let total = run.targets.len();
    let completed = run.count(OperationStatus::Succeeded);
    let mut parts = vec![format!("{completed} of {total} completed")];
    for (status, word) in [
        (OperationStatus::Failed, "failed"),
        (OperationStatus::Cancelled, "cancelled"),
        (OperationStatus::Blocked, "blocked"),
        (OperationStatus::NotStarted, "skipped"),
        (OperationStatus::NotConfirmed, "not confirmed"),
        (OperationStatus::Unsupported, "unsupported"),
    ] {
        let count = run.count(status);
        if count > 0 {
            parts.push(format!("{count} {word}"));
        }
    }
    let tone = if completed == total && total > 0 {
        Tone::Good
    } else if completed > 0 {
        Tone::Warn
    } else if run.count(OperationStatus::Failed) > 0 {
        Tone::Crit
    } else {
        Tone::Warn
    };
    (tone, parts.join(" · "))
}

fn emit(tx: &mpsc::UnboundedSender<RunEvent>, event: OperationsEvent) {
    // The receiver only goes away with the app; the run itself carries on.
    let _ = tx.send(RunEvent::Progress(event));
}

fn node_event(
    operation: OperationKind,
    target: &NodeTarget,
    phase: AuditPhase,
    step: OperationStep,
    message: impl Into<String>,
) -> OperationsEvent {
    OperationsEvent::Operation(OperationProgressEvent {
        operation,
        target: target.clone(),
        phase,
        step,
        message: message.into(),
    })
}

fn sequence_delay(options: &Options, targets: usize) -> Duration {
    if targets > 1 {
        Duration::from_secs(options.delay_secs.min(600))
    } else {
        Duration::ZERO
    }
}

/// The real run. Mutations happen only through core's runner, which audits
/// them; a sequence re-checks etcd and Kubernetes before each node.
async fn run_live(
    plan: RunPlan,
    live: super::LiveSource,
    cancel: Cancel,
    tx: mpsc::UnboundedSender<RunEvent>,
) -> Done {
    let audit = mutation::audit_log(&plan.context);
    let delay = sequence_delay(&plan.options, plan.targets.len());
    let mut request = SelectionRequest::new(
        plan.operation,
        plan.targets,
        plan.endpoint,
        OperationConfirmation::confirmed(),
    );
    request.drain_options = plan.options.drain;
    request.stop_on_failure = plan.options.stop_on_failure;
    request.delay_between_nodes = delay;
    let client = live.client.clone();
    // Identity revalidation belongs to the desktop's access session. Core polls this
    // future after its confirmation/cancellation gates and catches access failures.
    let connect = async move {
        live.forget_kubernetes();
        live.kubernetes().await
    };
    let outcome = run_selection(request, client, connect, &audit, cancel, move |event| {
        let event = match event {
            SelectionEvent::Progress(event) => RunEvent::Progress(event),
            SelectionEvent::NodeDone(result) => RunEvent::NodeDone(result),
        };
        // A closed receiver stops display only; the submitted run carries on.
        let _ = tx.send(event);
    })
    .await;
    Done {
        results: outcome.results,
        note: outcome.note,
        audit: Vec::new(),
    }
}

/// Turns the body's outcome into the run's results: an early error becomes a
/// result for every target, and a panic keeps what already finished. Example
/// outcomes are kept in memory and never touch the audit file.
async fn finish_example(
    plan: &RunPlan,
    body: impl std::future::Future<Output = Result<Vec<NodeOperationResult>, String>>,
    cancel: &Cancel,
    retained: &Retained,
) -> Done {
    let operation = plan.operation;
    let (results, note) = match AssertUnwindSafe(body).catch_unwind().await {
        Ok(Ok(results)) => (results, None),
        Ok(Err(error)) => (
            not_started(operation, &plan.targets, &error, cancel()),
            Some(error),
        ),
        Err(_) => (
            panic_outcomes(operation, &plan.targets, retained.lock().unwrap().clone()),
            Some("The operation worker panicked; inspect the nodes and the audit log.".to_owned()),
        ),
    };
    Done {
        results,
        note,
        audit: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Example data: a simulated cluster and run. Nothing here reads or writes a
// file or uses the network.
// ---------------------------------------------------------------------------

/// The etcd sample of the example cluster. prod-fra is healthy; staging-eu's
/// sample is incomplete (the alarm list timed out), so quorum impact is
/// unknown; homelab has a single control plane.
fn example_etcd(source: &ScreenSource) -> EtcdHealthSnapshot {
    let control_planes: Vec<_> = source
        .nodes
        .iter()
        .filter(|node| node.role == NodeRole::ControlPlane)
        .collect();
    let member_id = |ix: usize| 0x4c1d_9e07_a3b5_2f60_u64 + ix as u64 * 0x1111;
    let members: Vec<EtcdMemberInfo> = control_planes
        .iter()
        .enumerate()
        .map(|(ix, node)| EtcdMemberInfo {
            id: member_id(ix),
            hostname: node.name.clone(),
            peer_urls: vec![format!("https://{}:2380", node.address)],
            client_urls: vec![format!("https://{}:2379", node.address)],
            is_learner: false,
        })
        .collect();
    let statuses: Vec<EtcdMemberStatus> = control_planes
        .iter()
        .enumerate()
        .map(|(ix, node)| EtcdMemberStatus {
            node: node.name.clone(),
            member_id: member_id(ix),
            protocol_version: "3.6.0".into(),
            db_size: 96 * 1024 * 1024,
            db_size_in_use: 71 * 1024 * 1024,
            leader_id: member_id(0),
            raft_index: 4_812_377,
            raft_term: 7,
            raft_applied_index: 4_812_377,
            errors: Vec::new(),
            is_learner: false,
        })
        .collect();
    let unavailable = if source.target.context == "staging-eu" {
        vec![InspectionUnavailable {
            source: InspectionSource::EtcdAlarms,
            message: "the etcd alarm list timed out (example)".into(),
        }]
    } else {
        Vec::new()
    };
    assemble_etcd_health(
        source.inspection_target(),
        members,
        statuses,
        Vec::new(),
        unavailable,
    )
}

fn example_pods(target: &NodeTarget) -> usize {
    4 + (target.name.len() * 3) % 9
}

fn example_world(source: &ScreenSource, targets: &[NodeTarget]) -> ExampleWorld {
    let etcd = example_etcd(source);
    ExampleWorld {
        unreachable: source
            .nodes
            .iter()
            .filter(|node| !node.responding)
            .map(|node| node.name.clone())
            .collect(),
        impacts: targets
            .iter()
            .map(|target| etcd_impact(&etcd, target))
            .collect(),
    }
}

fn example_preview(source: &ScreenSource, key: PreviewKey) -> Preview {
    let world = example_world(source, &key.targets);
    let nodes = key
        .targets
        .iter()
        .zip(world.impacts)
        .map(|(target, impact)| NodeView {
            target: target.clone(),
            facts: Some(Facts {
                ready: !world.unreachable.contains(&target.name),
                unschedulable: false,
                pods: example_pods(target),
            }),
            safety: evaluate_operation_safety(key.operation, &impact),
            impact,
        })
        .collect();
    Preview {
        pdb: if drains(key.operation) {
            PdbNote::None
        } else {
            PdbNote::NotNeeded
        },
        source: "Example data; no cluster was contacted.".into(),
        nodes,
        key,
    }
}

/// One simulated node. Mirrors the core runner's order of steps and its
/// cancellation rules: it stops between steps, and a cordon it made is
/// restored.
async fn simulate_node(
    plan: RunPlan,
    target: NodeTarget,
    impact: EtcdQuorumImpact,
    cancel: Cancel,
    tx: mpsc::UnboundedSender<RunEvent>,
) -> NodeOperationResult {
    let operation = plan.operation;
    let say = |phase: AuditPhase, step: OperationStep, message: String| {
        emit(&tx, node_event(operation, &target, phase, step, message));
    };
    let result = |status: OperationStatus,
                  scheduling: SchedulingState,
                  drain: Option<DrainSummary>,
                  message: &str| NodeOperationResult {
        operation,
        target: target.clone(),
        status,
        scheduling,
        drain,
        message: message.to_owned(),
        audit_error: None,
    };
    let pause = || tokio::time::sleep(plan.step);
    say(
        AuditPhase::Start,
        OperationStep::Preflight,
        format!("Starting {} operation", operation.label()),
    );
    if cancel() {
        return result(
            OperationStatus::Cancelled,
            SchedulingState::Unchanged,
            None,
            "operation cancelled before mutation",
        );
    }
    if operation.is_destructive() {
        match evaluate_operation_safety(operation, &impact) {
            SafetyStatus::Unsafe(reason) => {
                return result(
                    OperationStatus::Blocked,
                    SchedulingState::Unchanged,
                    None,
                    &format!("operation blocked: {reason}"),
                );
            }
            SafetyStatus::Unknown => {
                return result(
                    OperationStatus::Blocked,
                    SchedulingState::Unchanged,
                    None,
                    "operation blocked: etcd quorum impact is unavailable or unknown",
                );
            }
            _ => {}
        }
    }
    let unreachable = plan
        .world
        .as_ref()
        .is_some_and(|world| world.unreachable.contains(&target.name));
    match operation {
        OperationKind::Cordon | OperationKind::Uncordon => {
            let (step, verb, state) = if operation == OperationKind::Cordon {
                (
                    OperationStep::Cordon,
                    "Cordoning",
                    SchedulingState::CordonedByOperation,
                )
            } else {
                (
                    OperationStep::Uncordon,
                    "Uncordoning",
                    SchedulingState::Schedulable,
                )
            };
            say(
                AuditPhase::Progress,
                step,
                format!("{verb} {}", target.name),
            );
            pause().await;
            say(
                AuditPhase::Success,
                OperationStep::Complete,
                format!("{} completed", kind_title(operation)),
            );
            result(
                OperationStatus::Succeeded,
                state,
                None,
                &format!("{} completed", kind_title(operation)),
            )
        }
        _ => {
            say(
                AuditPhase::Progress,
                OperationStep::InspectScheduling,
                "Reading scheduling state".into(),
            );
            pause().await;
            if cancel() {
                return result(
                    OperationStatus::Cancelled,
                    SchedulingState::Unchanged,
                    None,
                    "operation cancelled before mutation",
                );
            }
            say(
                AuditPhase::Progress,
                OperationStep::Cordon,
                "Cordoning the node".into(),
            );
            pause().await;
            let eligible = example_pods(&target);
            say(
                AuditPhase::Progress,
                OperationStep::ListPods,
                format!("{eligible} pods eligible for eviction"),
            );
            pause().await;
            let mut evicted = 0;
            for chunk in 1..=3 {
                if cancel() {
                    say(
                        AuditPhase::Progress,
                        OperationStep::RestoreScheduling,
                        "Cancelled; restoring scheduling".into(),
                    );
                    pause().await;
                    return result(
                        OperationStatus::Cancelled,
                        SchedulingState::Schedulable,
                        Some(DrainSummary {
                            eligible_pods: eligible,
                            evicted_pods: evicted,
                            ..Default::default()
                        }),
                        "cancelled during eviction; scheduling restored",
                    );
                }
                evicted = eligible * chunk / 3;
                if unreachable && chunk == 2 {
                    say(
                        AuditPhase::Failure,
                        OperationStep::EvictPod,
                        "The kubelet doesn't answer; 2 pods could not be evicted".into(),
                    );
                    let pod = |name: &str| PodReference {
                        namespace: "apps".into(),
                        name: name.into(),
                    };
                    return result(
                        OperationStatus::Failed,
                        SchedulingState::Unknown(
                            "the restoring uncordon wasn't confirmed (example)".into(),
                        ),
                        Some(DrainSummary {
                            eligible_pods: eligible,
                            evicted_pods: evicted.saturating_sub(2),
                            failed_pods: vec![pod("api-7d9f"), pod("worker-5c2b")],
                            ..Default::default()
                        }),
                        "drain failed: 2 pods could not be evicted because the kubelet doesn't answer",
                    );
                }
                say(
                    AuditPhase::Progress,
                    OperationStep::EvictPod,
                    format!("Evicted {evicted} of {eligible} pods"),
                );
                pause().await;
            }
            let force_deleted = if plan.options.drain.force_delete_unmanaged && eligible > 8 {
                vec![PodReference {
                    namespace: "default".into(),
                    name: "debug-shell".into(),
                }]
            } else {
                Vec::new()
            };
            let summary = DrainSummary {
                eligible_pods: eligible,
                evicted_pods: eligible - force_deleted.len(),
                force_deleted_pods: force_deleted,
                failed_pods: Vec::new(),
            };
            match operation {
                OperationKind::Drain => {
                    say(
                        AuditPhase::Success,
                        OperationStep::Complete,
                        "Drain completed".into(),
                    );
                    result(
                        OperationStatus::Succeeded,
                        SchedulingState::CordonedByOperation,
                        Some(summary),
                        "drain completed; the node stays cordoned",
                    )
                }
                OperationKind::Shutdown => {
                    if cancel() {
                        return result(
                            OperationStatus::Cancelled,
                            SchedulingState::CordonedByOperation,
                            Some(summary),
                            "cancelled before the shutdown request; the node stays cordoned",
                        );
                    }
                    say(
                        AuditPhase::Progress,
                        OperationStep::Shutdown,
                        "Talos accepted the shutdown request".into(),
                    );
                    pause().await;
                    say(
                        AuditPhase::Success,
                        OperationStep::Complete,
                        "Shutdown requested".into(),
                    );
                    result(
                        OperationStatus::Succeeded,
                        SchedulingState::CordonedByOperation,
                        Some(summary),
                        "shutdown requested; the node stays cordoned until it is started and uncordoned",
                    )
                }
                _ => {
                    if cancel() {
                        return result(
                            OperationStatus::Cancelled,
                            SchedulingState::CordonedByOperation,
                            Some(summary),
                            "cancelled before the reboot request; the node stays cordoned",
                        );
                    }
                    if unreachable {
                        say(
                            AuditPhase::Failure,
                            OperationStep::Reboot,
                            "The Talos API didn't answer".into(),
                        );
                        return result(
                            OperationStatus::Failed,
                            SchedulingState::CordonedByOperation,
                            Some(summary),
                            "the Talos API didn't answer, so no reboot was sent; the node stays cordoned",
                        );
                    }
                    say(
                        AuditPhase::Progress,
                        OperationStep::Reboot,
                        "Talos accepted the reboot request".into(),
                    );
                    pause().await;
                    if !plan.options.drain.wait_for_node_ready {
                        return result(
                            OperationStatus::Succeeded,
                            SchedulingState::CordonedByOperation,
                            Some(summary),
                            "reboot requested; Ready wasn't waited for",
                        );
                    }
                    say(
                        AuditPhase::Progress,
                        OperationStep::WaitForReady,
                        "Waiting for the node to be Ready".into(),
                    );
                    pause().await;
                    if plan.options.drain.uncordon_after_reboot {
                        say(
                            AuditPhase::Progress,
                            OperationStep::Uncordon,
                            "Restoring scheduling".into(),
                        );
                        pause().await;
                    }
                    say(
                        AuditPhase::Success,
                        OperationStep::Complete,
                        "Reboot completed".into(),
                    );
                    result(
                        OperationStatus::Succeeded,
                        if plan.options.drain.uncordon_after_reboot {
                            SchedulingState::Schedulable
                        } else {
                            SchedulingState::CordonedByOperation
                        },
                        Some(summary),
                        "reboot completed; the node is Ready",
                    )
                }
            }
        }
    }
}

async fn run_example(
    plan: RunPlan,
    cancel: Cancel,
    tx: mpsc::UnboundedSender<RunEvent>,
    retained: Retained,
) -> Done {
    let operation = plan.operation;
    let world = plan.world.clone().unwrap_or(ExampleWorld {
        unreachable: Vec::new(),
        impacts: Vec::new(),
    });
    let body = async {
        if cancel() {
            return Err("Cancelled before the latest prechecks; no mutation was made".to_owned());
        }
        if let Some(reason) = first_blocking(operation, &plan.targets, &world.impacts) {
            return Err(reason);
        }
        let total = plan.targets.len();
        Ok(ordered_sequence(
            operation,
            &plan.targets,
            plan.options.stop_on_failure,
            // A simulated pause between nodes, not the real delay.
            if total > 1 { plan.step } else { Duration::ZERO },
            &*cancel,
            |index, target| {
                let impact = world
                    .impacts
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| EtcdQuorumImpact::unavailable("not sampled"));
                let (plan, cancel, tx, retained) =
                    (plan.clone(), cancel.clone(), tx.clone(), retained.clone());
                async move {
                    let result = simulate_node(plan, target, impact, cancel, tx.clone()).await;
                    retained.lock().unwrap().push(result.clone());
                    let _ = tx.send(RunEvent::NodeDone(result.clone()));
                    result
                }
            },
        )
        .await)
    };
    let mut done = finish_example(&plan, body, &cancel, &retained).await;
    // In memory only: shown on this screen, never written.
    done.audit = done
        .results
        .iter()
        .map(|result| AuditEntry {
            timestamp: chrono::Utc::now(),
            cluster: plan.context.clone(),
            actor: "example".into(),
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
            message: format!("{:?}: {}", result.status, result.message),
        })
        .collect();
    done
}

// ---------------------------------------------------------------------------
// Audit
// ---------------------------------------------------------------------------

struct AuditView {
    /// `None` for example data, which has no file.
    path: Option<std::path::PathBuf>,
    /// Oldest first, as stored.
    entries: Vec<AuditEntry>,
}

// ---------------------------------------------------------------------------
// Screen
// ---------------------------------------------------------------------------

pub(crate) struct OperationsScreen {
    runtime: Handle,
    source: Option<ScreenSource>,
    /// Set once the screen has been shown; before that nothing is requested.
    activated: bool,
    operation: OperationKind,
    options: Options,
    /// Selected targets in run order.
    selected: Vec<NodeTarget>,
    cursor: usize,
    preview: PreviewState,
    preview_generation: u64,
    preview_job: Option<OwnedJob>,
    preview_task: Option<Task<()>>,
    /// The user asked to run: open the confirmation as soon as the fresh
    /// preview is in and the plan may run.
    confirm_when_ready: bool,
    audit: Loader<AuditView>,
    example_audit: Vec<AuditEntry>,
    notice: Option<String>,
    run: Option<RunView>,
    next_run: u64,
    /// How long a simulated step takes; tests shorten it.
    step: Duration,
    focus: FocusHandle,
    _operations: Subscription,
}

impl EventEmitter<ScreenEvent> for OperationsScreen {}

impl ScreenPanel for OperationsScreen {
    fn new(runtime: Handle, _: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", NextNode, Some(CONTEXT)),
            KeyBinding::new("up", PreviousNode, Some(CONTEXT)),
            KeyBinding::new("home", FirstNode, Some(CONTEXT)),
            KeyBinding::new("end", LastNode, Some(CONTEXT)),
            KeyBinding::new("space", ToggleNode, Some(CONTEXT)),
            KeyBinding::new("enter", ToggleNode, Some(CONTEXT)),
            KeyBinding::new("alt-up", MoveEarlier, Some(CONTEXT)),
            KeyBinding::new("alt-down", MoveLater, Some(CONTEXT)),
            KeyBinding::new("escape", ClearSelection, Some(CONTEXT)),
        ]);
        let operations = Operations::global(cx);
        Self {
            runtime,
            source: None,
            activated: false,
            operation: OperationKind::Drain,
            options: Options::default(),
            selected: Vec::new(),
            cursor: 0,
            preview: PreviewState::Idle,
            preview_generation: 0,
            preview_job: None,
            preview_task: None,
            confirm_when_ready: false,
            audit: Loader::default(),
            example_audit: Vec::new(),
            notice: None,
            run: None,
            next_run: 0,
            step: SIMULATED_STEP,
            focus: cx.focus_handle(),
            // The slot is app-wide: redraw whenever anything takes or frees it.
            _operations: cx.observe(&operations, |_, _, cx| cx.notify()),
        }
    }

    fn set_source(
        &mut self,
        source: Option<ScreenSource>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let changed = self.source.as_ref().map(|source| &source.target)
            != source.as_ref().map(|source| &source.target);
        self.source = source;
        if changed {
            // A different cluster or node: nothing selected or previewed for
            // the old one applies. A run in progress is not affected.
            self.audit.reset();
            self.example_audit.clear();
            self.notice = None;
            self.drop_preview();
            self.selected = self
                .source
                .as_ref()
                .and_then(|source| {
                    self.roster()
                        .into_iter()
                        .find(|node| node.target.name == source.target.node)
                })
                .map(|node| vec![node.target])
                .unwrap_or_default();
            self.cursor = 0;
        } else {
            // Same target, possibly a new roster: drop what is no longer there
            // or whose address changed.
            let roster = self.roster();
            let before = self.selected.len();
            self.selected
                .retain(|target| roster.iter().any(|node| &node.target == target));
            if self.selected.len() != before {
                self.drop_preview();
            }
        }
        self.cursor = self.cursor.min(self.roster().len().saturating_sub(1));
        if self.activated {
            self.ensure_preview(false, window, cx);
            self.load_audit(cx);
        }
        cx.notify();
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.activated = true;
        self.ensure_preview(false, window, cx);
        if self.audit.data().is_none() && !self.audit.is_loading() {
            self.load_audit(cx);
        }
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.notice = None;
        self.ensure_preview(true, window, cx);
        self.load_audit(cx);
        cx.notify();
    }
}

impl OperationsScreen {
    fn roster(&self) -> Vec<RosterNode> {
        let Some(source) = &self.source else {
            return Vec::new();
        };
        source
            .nodes
            .iter()
            .filter(|node| !node.name.trim().is_empty() && !node.address.trim().is_empty())
            .map(|node| RosterNode {
                target: NodeTarget::new(node.name.clone(), node.address.clone()),
                role: node.role,
                responding: node.responding,
            })
            .collect()
    }

    /// What a preview would be for right now, or `None` with nothing selected.
    fn key(&self) -> Option<PreviewKey> {
        let source = self.source.as_ref()?;
        (!self.selected.is_empty()).then(|| PreviewKey {
            context: source.target.context.clone(),
            epoch: source.target.epoch,
            operation: self.operation,
            targets: self.selected.clone(),
        })
    }

    fn drop_preview(&mut self) {
        self.preview = PreviewState::Idle;
        self.preview_job = None;
        self.preview_task = None;
        self.preview_generation += 1;
        self.confirm_when_ready = false;
    }

    /// Takes a preview of the current selection unless one for it is already
    /// there (or on its way). `force` takes a fresh one regardless.
    fn ensure_preview(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.key() else {
            self.drop_preview();
            cx.notify();
            return;
        };
        if !force && self.preview.key() == Some(&key) {
            return;
        }
        self.preview_generation += 1;
        let generation = self.preview_generation;
        self.preview_job = None;
        self.preview_task = None;
        let Some(source) = self.source.clone() else {
            return;
        };
        let Some(live) = source.live.clone() else {
            // Example data answers at once, from the example cluster.
            self.preview = PreviewState::Ready(example_preview(&source, key));
            self.preview_ready(window, cx);
            cx.notify();
            return;
        };
        self.preview = PreviewState::Loading(key.clone());
        let endpoint = source.inspection_target();
        let work_key = key.clone();
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            SCREEN_DEADLINE,
            "Checking the nodes timed out".into(),
            async move { preview_live(live, endpoint, work_key).await },
        );
        self.preview_job = Some(job);
        self.preview_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Checking the nodes stopped unexpectedly".into()));
            _ = this.update_in(cx, |screen, window, cx| {
                if screen.preview_generation != generation {
                    return;
                }
                screen.preview_job = None;
                screen.preview = match result {
                    Ok(preview) => PreviewState::Ready(preview),
                    Err(error) => PreviewState::Failed { key, error },
                };
                screen.preview_ready(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// A preview just landed: if the user is waiting to confirm and the plan
    /// can run, open the confirmation on exactly this preview.
    fn preview_ready(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.confirm_when_ready) {
            return;
        }
        if blocked_reason(&self.preview, self.operation).is_none() {
            self.open_confirmation(window, cx);
        }
    }

    /// The selection, operation or options changed.
    fn plan_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.notice = None;
        self.confirm_when_ready = false;
        self.ensure_preview(false, window, cx);
        cx.notify();
    }

    fn toggle(&mut self, target: NodeTarget, window: &mut Window, cx: &mut Context<Self>) {
        selection_toggle(&mut self.selected, target);
        self.plan_changed(window, cx);
    }

    fn set_operation(&mut self, kind: OperationKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.operation != kind {
            self.operation = kind;
            self.plan_changed(window, cx);
        }
    }

    fn move_selected(
        &mut self,
        index: usize,
        up: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        move_target(&mut self.selected, index, up);
        self.plan_changed(window, cx);
    }

    fn step_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.roster().len();
        if len == 0 {
            return;
        }
        self.cursor = self.cursor.saturating_add_signed(delta).min(len - 1);
        cx.notify();
    }

    fn cursor_target(&self) -> Option<NodeTarget> {
        self.roster()
            .get(self.cursor)
            .map(|node| node.target.clone())
    }

    /// Moves the node under the cursor within the run order.
    fn move_cursor_node(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = self.cursor_target()
            && let Some(index) = self.selected.iter().position(|t| t == &target)
        {
            self.move_selected(index, up, window, cx);
        }
    }

    fn load_audit(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if source.live.is_none() {
            // Example data has no file: show what this session simulated.
            let entries = self.example_audit.clone();
            self.audit.resolve(
                source.target.clone(),
                Ok(AuditView {
                    path: None,
                    entries,
                }),
            );
            return;
        }
        let context = source.target.context.clone();
        self.audit.load(
            source.target.clone(),
            &self.runtime,
            "the audit log",
            async move {
                tokio::task::spawn_blocking(move || {
                    let log = mutation::audit_log(&context);
                    log.read_recent(AUDIT_ENTRIES)
                        .map(|entries| AuditView {
                            path: Some(log.path().to_path_buf()),
                            entries,
                        })
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("Reading the audit log stopped: {error}"))?
            },
            |screen: &mut Self| &mut screen.audit,
            cx,
        );
    }

    /// Opens the confirmation for the preview on screen, after a fresh one.
    fn review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.notice = None;
        if let Some(running) = Operations::current(cx) {
            self.notice = Some(format!(
                "{} is running. Wait for it to finish, or cancel it from the status bar.",
                running.label
            ));
            cx.notify();
            return;
        }
        if self.key().is_none() {
            return;
        }
        // Take the preview again right before confirming: what the user
        // confirms must be what the sources say now.
        self.confirm_when_ready = true;
        self.ensure_preview(true, window, cx);
    }

    fn open_confirmation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(source), PreviewState::Ready(preview)) = (self.source.clone(), &self.preview)
        else {
            return;
        };
        let preview = preview.clone();
        let kind = preview.key.operation;
        let options = self.options.clone();
        let mut facts: Vec<(SharedString, SharedString)> = vec![
            ("Context".into(), preview.key.context.clone().into()),
            (
                "Operation".into(),
                format!("{} · {}", kind_title(kind), kind_blurb(kind)).into(),
            ),
        ];
        let mut warnings: Vec<SharedString> = Vec::new();
        for (ix, node) in preview.nodes.iter().enumerate() {
            let verdict = verdict(kind, node);
            let state = node
                .facts
                .as_ref()
                .map(|f| {
                    format!(
                        " · {} · {}{} pods",
                        if f.ready { "Ready" } else { "not Ready" },
                        if f.unschedulable { "cordoned · " } else { "" },
                        f.pods
                    )
                })
                .unwrap_or_default();
            facts.push((
                format!("Target {}", ix + 1).into(),
                format!(
                    "{} · {}{state} · etcd: {}",
                    node.target.name,
                    node.target.address,
                    verdict.label.to_lowercase()
                )
                .into(),
            ));
            if verdict.tone == Tone::Warn {
                warnings.push(format!("{}: {}", node.target.name, verdict.detail).into());
            }
        }
        for (label, value) in options.lines(kind, preview.nodes.len()) {
            facts.push((label.into(), value.into()));
        }
        match &preview.pdb {
            PdbNote::Blocking(pdbs) => {
                let names: Vec<String> = pdbs
                    .iter()
                    .take(4)
                    .map(|pdb| format!("{}/{}", pdb.namespace, pdb.name))
                    .collect();
                warnings.push(
                    format!(
                        "{} PodDisruptionBudget(s) allow no disruption and can stall the drain: {}{}",
                        pdbs.len(),
                        names.join(", "),
                        if pdbs.len() > names.len() { ", …" } else { "" }
                    )
                    .into(),
                );
            }
            PdbNote::Unknown(reason) => {
                warnings.push(format!("PodDisruptionBudgets weren't read: {reason}").into());
            }
            PdbNote::None | PdbNote::NotNeeded => {}
        }
        if matches!(kind, OperationKind::Cordon | OperationKind::Drain) {
            warnings.push("The nodes stay unschedulable until they are uncordoned.".into());
        }
        let count = preview.nodes.len();
        let summary = if count == 1 {
            format!(
                "{} {} in {}.",
                kind_title(kind),
                preview.nodes[0].target.name,
                preview.key.context
            )
        } else {
            format!(
                "{} {count} nodes in {}, one at a time, in the order listed.",
                kind_title(kind),
                preview.key.context
            )
        };
        let shown = fingerprint(&preview, &options);
        let plan = RunPlan {
            context: preview.key.context.clone(),
            operation: kind,
            targets: preview.key.targets.clone(),
            options: options.clone(),
            endpoint: source.inspection_target(),
            world: source
                .is_example()
                .then(|| example_world(&source, &preview.key.targets)),
            step: self.step,
        };
        let weak = cx.weak_entity();
        let current = weak.clone();
        let confirm_label = if count == 1 {
            format!("{} node", kind_title(kind))
        } else {
            format!("{} {count} nodes", kind_title(kind))
        };
        mutation::confirm(
            Confirmation {
                title: format!(
                    "{} {}?",
                    kind_title(kind),
                    if count == 1 { "node" } else { "nodes" }
                )
                .into(),
                summary: summary.into(),
                facts,
                warnings,
                confirm_label: confirm_label.into(),
                destructive: matches!(
                    kind,
                    OperationKind::Drain | OperationKind::Reboot | OperationKind::Shutdown
                ),
                fingerprint: shown,
                current: Rc::new(move |cx: &App| {
                    current
                        .read_with(cx, |screen, _| screen.current_fingerprint())
                        .ok()
                        .flatten()
                }),
                on_confirm: Rc::new(move |window, cx| {
                    _ = weak.update(cx, |screen, cx| {
                        screen.start_run(plan.clone(), source.clone(), window, cx)
                    });
                }),
            },
            window,
            cx,
        );
    }

    /// The fingerprint of what the screen would confirm right now. `None` when
    /// the preview is loading, failed, belongs to something else, or the plan
    /// is blocked.
    fn current_fingerprint(&self) -> Option<u64> {
        let PreviewState::Ready(preview) = &self.preview else {
            return None;
        };
        (self.key().as_ref() == Some(&preview.key)
            && blocked_reason(&self.preview, self.operation).is_none())
        .then(|| fingerprint(preview, &self.options))
    }

    fn start_run(
        &mut self,
        plan: RunPlan,
        source: ScreenSource,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let operations = Operations::global(cx);
        let label = format!(
            "{}: {} {}",
            plan.context,
            plan.operation.label(),
            plan.targets
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let ticket = match operations.update(cx, |operations, cx| operations.begin(label, cx)) {
            Ok(ticket) => ticket,
            Err(running) => {
                self.notice = Some(format!(
                    "{} is running. Nothing was started; wait for it to finish, or cancel it from the status bar.",
                    running.label
                ));
                cx.notify();
                return;
            }
        };
        self.notice = None;
        self.next_run += 1;
        let id = self.next_run;
        self.run = Some(RunView {
            id,
            context: plan.context.clone(),
            operation: plan.operation,
            targets: plan.targets.clone(),
            example: source.is_example(),
            progress: VecDeque::new(),
            dropped: 0,
            results: Vec::new(),
            note: None,
            finished: false,
            cancel_requested: false,
        });
        let cancel: Cancel = Arc::new(ticket.cancellation());
        let (tx, mut rx) = mpsc::unbounded_channel::<RunEvent>();
        let retained: Retained = Arc::new(Mutex::new(Vec::new()));
        // Spawned on Tokio and never aborted: a mutation that was sent runs to
        // its end, and the screen going away only stops the display.
        let worker = {
            let (cancel, tx, retained) = (cancel.clone(), tx.clone(), retained.clone());
            let plan = plan.clone();
            let live = source.live.clone();
            async move {
                let done = match live {
                    Some(live) => run_live(plan, live, cancel, tx.clone()).await,
                    None => run_example(plan, cancel, tx.clone(), retained).await,
                };
                let _ = tx.send(RunEvent::Done(done));
            }
        };
        drop(tx);
        self.runtime.spawn(worker);
        let (targets, operation) = (plan.targets.clone(), plan.operation);
        let example = source.is_example();
        // Detached on purpose: this owns the ticket, so the slot is released
        // only when the run really ended, even if the screen is gone by then.
        cx.spawn(async move |this, cx| {
            let mut finished = None;
            while let Some(event) = rx.recv().await {
                match event {
                    RunEvent::Progress(event) => {
                        let (line, step) = describe(&event);
                        operations
                            .update(cx, |operations, cx| operations.progress(&ticket, step, cx));
                        _ = this.update(cx, |screen, cx| screen.run_progress(id, line, cx));
                    }
                    RunEvent::NodeDone(result) => {
                        _ = this.update(cx, |screen, cx| screen.run_node_done(id, result, cx));
                    }
                    RunEvent::Done(done) => {
                        finished = Some(done);
                        break;
                    }
                }
            }
            let done = finished.unwrap_or_else(|| Done {
                results: not_started(
                    operation,
                    &targets,
                    "The operation worker stopped unexpectedly; inspect the nodes",
                    false,
                ),
                note: Some("The operation worker stopped unexpectedly.".into()),
                audit: Vec::new(),
            });
            _ = this.update(cx, |screen, cx| screen.run_finished(id, done, example, cx));
            operations.update(cx, |operations, cx| operations.finish(ticket, cx));
        })
        .detach();
        cx.notify();
    }

    fn run_progress(&mut self, id: u64, line: String, cx: &mut Context<Self>) {
        if let Some(run) = self.run.as_mut().filter(|run| run.id == id) {
            run.push(line);
            cx.notify();
        }
    }

    fn run_node_done(&mut self, id: u64, result: NodeOperationResult, cx: &mut Context<Self>) {
        if let Some(run) = self.run.as_mut().filter(|run| run.id == id) {
            run.results.push(result);
            cx.notify();
        }
    }

    fn run_finished(&mut self, id: u64, done: Done, example: bool, cx: &mut Context<Self>) {
        let Some(run) = self.run.as_mut().filter(|run| run.id == id) else {
            return;
        };
        run.results = done.results;
        run.note = done.note;
        run.finished = true;
        if example {
            self.example_audit.extend(done.audit);
        }
        // What the run changed is not what the old preview showed.
        self.drop_preview();
        self.load_audit(cx);
        cx.notify();
    }

    fn cancel_run(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = self.run.as_mut().filter(|run| !run.finished) {
            run.cancel_requested = true;
            Operations::global(cx).update(cx, |operations, cx| operations.request_cancel(cx));
            cx.notify();
        }
    }
}

/// The real preview: Kubernetes identity, readiness, scheduling and pods per
/// target, a fresh etcd sample, and the PodDisruptionBudgets a drain might hit.
async fn preview_live(
    live: super::LiveSource,
    endpoint: InspectionTarget,
    key: PreviewKey,
) -> Result<Preview, String> {
    let kubernetes = live.kubernetes().await?;
    let checked = tokio::time::timeout(
        PREFLIGHT_TIMEOUT,
        preflight_nodes(&kubernetes, live.client.clone(), endpoint, &key.targets),
    )
    .await
    .map_err(|_| "The checks timed out; nothing was changed.".to_owned())
    .and_then(|checked| checked);
    let checked = match checked {
        Ok(checked) => checked,
        Err(error) => {
            // The reused client may be at fault; revalidate and rebuild next time.
            live.forget_kubernetes();
            return Err(error);
        }
    };
    let pdb = if drains(key.operation) {
        match blocking_pdbs(&kubernetes).await {
            Ok(pdbs) if pdbs.is_empty() => PdbNote::None,
            Ok(pdbs) => PdbNote::Blocking(pdbs),
            Err(error) => PdbNote::Unknown(error),
        }
    } else {
        PdbNote::NotNeeded
    };
    let nodes = checked
        .into_iter()
        .map(|node| NodeView {
            safety: evaluate_operation_safety(key.operation, &node.impact),
            facts: Some(Facts {
                ready: node.ready,
                unschedulable: node.unschedulable,
                pods: node.pod_count,
            }),
            target: node.target,
            impact: node.impact,
        })
        .collect();
    Ok(Preview {
        key,
        nodes,
        pdb,
        source: "Kubernetes API and a fresh etcd sample.".into(),
    })
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn chip(
    id: &'static str,
    label: &'static str,
    on: bool,
    role: Role,
    cx: &App,
) -> impl StatefulInteractiveElement + IntoElement + use<> {
    let p = palette(cx);
    let label: SharedString = label.into();
    h_flex()
        .id(id)
        .test_support()
        .role(role)
        .aria_toggled(if on { Toggled::True } else { Toggled::False })
        .aria_label(label.clone())
        .tab_index(0)
        .h(dp(28.))
        .px(dp(11.))
        .gap(dp(6.))
        .rounded(px(6.))
        .border_1()
        .text_size(dp(12.5))
        .cursor_pointer()
        .whitespace_nowrap()
        .when(on, |this| {
            this.border_color(p.accent_line)
                .bg(p.accent_soft)
                .text_color(p.accent)
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!on, |this| {
            this.border_color(p.line_strong)
                .bg(p.surface)
                .text_color(p.muted)
                .hover(|style| style.bg(p.hover))
        })
        .when(on, |this| {
            this.child(
                Icon::new(IconName::Check)
                    .size(dp(13.))
                    .text_color(p.accent),
            )
        })
        .child(label)
}

fn section_title(text: &'static str, cx: &App) -> Div {
    div().px_3().pt_2p5().pb_1p5().child(ui::caption(text, cx))
}

impl OperationsScreen {
    fn kinds_panel(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        panel(cx).child(section_title("Operation", cx)).child(
            v_flex()
                .px_3()
                .pb_3()
                .gap_2()
                .child(
                    h_flex()
                        .id("ops-kinds")
                        .test_support()
                        .role(Role::Group)
                        .aria_label("Operation")
                        .gap_2()
                        .flex_wrap()
                        .children(KINDS.into_iter().map(|kind| {
                            chip(
                                kind_id(kind),
                                kind_title(kind),
                                self.operation == kind,
                                Role::RadioButton,
                                cx,
                            )
                            .on_click(cx.listener(
                                move |screen, _, window, cx| screen.set_operation(kind, window, cx),
                            ))
                        })),
                )
                .child(
                    div()
                        .text_size(dp(12.5))
                        .text_color(p.muted)
                        .child(kind_blurb(self.operation)),
                ),
        )
    }

    fn options_panel(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let kind = self.operation;
        let multi = self.selected.len() > 1;
        let d = &self.options.drain;
        let mut controls: Vec<AnyElement> = Vec::new();
        let flag = |id: &'static str,
                    label: &'static str,
                    on: bool,
                    set: fn(&mut Options),
                    cx: &mut Context<Self>| {
            chip(id, label, on, Role::CheckBox, cx)
                .on_click(cx.listener(move |screen, _, window, cx| {
                    set(&mut screen.options);
                    screen.plan_changed(window, cx);
                }))
                .into_any_element()
        };
        let stepper = |dec: &'static str,
                       inc: &'static str,
                       label: &'static str,
                       value: String,
                       change: fn(&mut Options, bool),
                       cx: &mut Context<Self>| {
            h_flex()
                .gap_1p5()
                .items_center()
                .child(div().text_size(dp(12.5)).text_color(p.muted).child(label))
                .child(
                    Button::new(dec)
                        .outline()
                        .xsmall()
                        .label("−")
                        .on_click(cx.listener(move |screen, _, window, cx| {
                            change(&mut screen.options, false);
                            screen.plan_changed(window, cx);
                        })),
                )
                .child(
                    div()
                        .min_w(dp(54.))
                        .text_center()
                        .font_family(MONO_FONT)
                        .text_size(dp(12.))
                        .child(value),
                )
                .child(
                    Button::new(inc)
                        .outline()
                        .xsmall()
                        .label("+")
                        .on_click(cx.listener(move |screen, _, window, cx| {
                            change(&mut screen.options, true);
                            screen.plan_changed(window, cx);
                        })),
                )
                .into_any_element()
        };
        if drains(kind) {
            controls.push(flag(
                "ops-opt-ignore-daemonsets",
                "Skip DaemonSet pods",
                d.ignore_daemonsets,
                |o| o.drain.ignore_daemonsets = !o.drain.ignore_daemonsets,
                cx,
            ));
            controls.push(flag(
                "ops-opt-delete-emptydir",
                "Delete emptyDir data",
                d.delete_emptydir_data,
                |o| o.drain.delete_emptydir_data = !o.drain.delete_emptydir_data,
                cx,
            ));
            controls.push(flag(
                "ops-opt-force-unmanaged",
                "Force-delete unmanaged pods",
                d.force_delete_unmanaged,
                |o| o.drain.force_delete_unmanaged = !o.drain.force_delete_unmanaged,
                cx,
            ));
            controls.push(stepper(
                "ops-opt-pod-timeout-dec",
                "ops-opt-pod-timeout-inc",
                "Per-pod timeout",
                format_secs(d.per_pod_timeout_secs),
                |o, up| {
                    o.drain.per_pod_timeout_secs =
                        stepped(o.drain.per_pod_timeout_secs, up, 15, 5, 600)
                },
                cx,
            ));
            if d.force_delete_unmanaged {
                controls.push(stepper(
                    "ops-opt-grace-dec",
                    "ops-opt-grace-inc",
                    "Force-delete grace",
                    grace_text(d.force_delete_grace_period_secs),
                    |o, up| {
                        let at = GRACE_CHOICES
                            .iter()
                            .position(|g| *g == o.drain.force_delete_grace_period_secs)
                            .unwrap_or(0);
                        let next = if up {
                            (at + 1).min(GRACE_CHOICES.len() - 1)
                        } else {
                            at.saturating_sub(1)
                        };
                        o.drain.force_delete_grace_period_secs = GRACE_CHOICES[next];
                    },
                    cx,
                ));
            }
        }
        if kind == OperationKind::Reboot {
            controls.push(flag(
                "ops-opt-wait-ready",
                "Wait for Ready after reboot",
                d.wait_for_node_ready,
                |o| o.drain.wait_for_node_ready = !o.drain.wait_for_node_ready,
                cx,
            ));
            if d.wait_for_node_ready {
                controls.push(flag(
                    "ops-opt-uncordon-after",
                    "Uncordon afterwards",
                    d.uncordon_after_reboot,
                    |o| o.drain.uncordon_after_reboot = !o.drain.uncordon_after_reboot,
                    cx,
                ));
                controls.push(stepper(
                    "ops-opt-ready-timeout-dec",
                    "ops-opt-ready-timeout-inc",
                    "Ready timeout",
                    format_secs(d.post_reboot_timeout_secs),
                    |o, up| {
                        o.drain.post_reboot_timeout_secs =
                            stepped(o.drain.post_reboot_timeout_secs, up, 60, 60, 1800)
                    },
                    cx,
                ));
            }
        }
        if multi {
            controls.push(flag(
                "ops-opt-stop-on-failure",
                "Stop at the first failure",
                self.options.stop_on_failure,
                |o| o.stop_on_failure = !o.stop_on_failure,
                cx,
            ));
            controls.push(stepper(
                "ops-opt-delay-dec",
                "ops-opt-delay-inc",
                "Delay between nodes",
                format_secs(self.options.delay_secs),
                |o, up| o.delay_secs = stepped(o.delay_secs, up, 15, 0, 600),
                cx,
            ));
        }
        let empty = controls.is_empty();
        panel(cx).child(section_title("Options", cx)).child(
            h_flex()
                .id("ops-options")
                .test_support()
                .role(Role::Group)
                .aria_label("Options")
                .px_3()
                .pb_3()
                .gap_x_4()
                .gap_y_2()
                .flex_wrap()
                .items_center()
                .when(empty, |this| {
                    this.child(
                        div()
                            .text_size(dp(12.5))
                            .text_color(p.muted)
                            .child(format!("{} has no options.", kind_title(kind))),
                    )
                })
                .children(controls),
        )
    }

    fn render_node(
        &self,
        ix: usize,
        node: &RosterNode,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let order = self.selected.iter().position(|t| t == &node.target);
        let on = order.is_some();
        let under_cursor = self.cursor == ix;
        let target = node.target.clone();
        h_flex()
            .id(("ops-node", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(on)
            .aria_label(format!(
                "{} · {} · {} · {}{}",
                node.target.name,
                node.target.address,
                node.role.label(),
                match order {
                    Some(k) => format!("selected, run order {}", k + 1),
                    None => "not selected".to_owned(),
                },
                if node.responding {
                    ""
                } else {
                    " · not responding to the Talos API"
                }
            ))
            .w_full()
            .h(dp(ROW_HEIGHT))
            .font_family(MONO_FONT)
            .text_size(dp(12.))
            .cursor_pointer()
            .when(on, |this| this.bg(p.accent_soft).text_color(p.accent))
            .when(!on, |this| this.hover(|style| style.bg(p.hover)))
            .when(under_cursor, |this| {
                this.border_l_2().border_color(p.accent)
            })
            .child(cell(COLUMNS[0]).child(match order {
                Some(k) => format!("✓ {}", k + 1),
                None => "·".to_owned(),
            }))
            .child(cell(COLUMNS[1]).child(node.target.name.clone()))
            .child(cell(COLUMNS[2]).child(node.target.address.clone()))
            .child(cell(COLUMNS[3]).child(node.role.label()))
            .on_click(cx.listener(move |screen, _, window, cx| {
                screen.cursor = ix;
                window.focus(&screen.focus, cx);
                screen.toggle(target.clone(), window, cx);
            }))
    }

    fn nodes_panel(&self, roster: &[RosterNode], cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let list = if roster.is_empty() {
            div()
                .px_3()
                .py_3p5()
                .text_size(dp(12.5))
                .text_color(p.muted)
                .child("No node roster is available, so there is nothing to select.")
                .into_any_element()
        } else {
            v_flex()
                .children(
                    roster
                        .iter()
                        .enumerate()
                        .map(|(ix, node)| self.render_node(ix, node, cx)),
                )
                .into_any_element()
        };
        panel(cx).overflow_hidden().child(
            div()
                .id("ops-node-scroll")
                .test_support()
                .overflow_x_scroll()
                .child(
                    v_flex()
                        .w_full()
                        .min_w(dp(table_width(&COLUMNS)))
                        .child(table_head(&COLUMNS, cx))
                        .child(
                            div()
                                .id("ops-nodes")
                                .test_support()
                                .role(Role::ListBox)
                                .aria_label(
                                    "Nodes; arrows move, Space selects, Alt+arrows reorder the run, Escape clears",
                                )
                                .child(list),
                        ),
                ),
        )
    }

    fn verdict_row(&self, k: usize, node: &NodeView, cx: &App) -> impl IntoElement + use<> {
        let p = palette(cx);
        let v = verdict(self.operation, node);
        let state = node.facts.as_ref().map(|f| {
            format!(
                "{} · {}{} pods{}",
                if f.ready { "Ready" } else { "not Ready" },
                if f.unschedulable { "cordoned · " } else { "" },
                f.pods,
                if f.pods >= POD_SAMPLE_LIMIT as usize {
                    "+"
                } else {
                    ""
                }
            )
        });
        v_flex()
            .id(("ops-verdict", k))
            .test_support()
            .role(Role::Status)
            .aria_label(format!(
                "{}: etcd {}. {}{}",
                node.target.name,
                v.label,
                v.detail,
                state.as_ref().map(|s| format!(" {s}.")).unwrap_or_default()
            ))
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(ui::tag(v.tone, v.icon, v.label, cx))
                    .child(
                        div()
                            .text_size(dp(12.5))
                            .text_color(p.muted)
                            .child(v.detail),
                    ),
            )
            .children(state.map(|state| {
                div()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child(format!("Kubernetes: {state}"))
            }))
    }

    fn plan_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = palette(cx);
        let kind = self.operation;
        let count = self.selected.len();
        let preview = match &self.preview {
            PreviewState::Ready(preview) if self.key().as_ref() == Some(&preview.key) => {
                Some(preview)
            }
            _ => None,
        };
        let rows = self
            .selected
            .iter()
            .enumerate()
            .map(|(k, target)| {
                let node = preview.and_then(|preview| preview.nodes.get(k));
                v_flex()
                    .id(("ops-plan-target", k))
                    .test_support()
                    .role(Role::ListBoxOption)
                    .aria_label(format!("{}. {} · {}", k + 1, target.name, target.address))
                    .gap_1p5()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(p.line)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(dp(22.))
                                    .flex_none()
                                    .font_family(MONO_FONT)
                                    .text_color(p.muted)
                                    .child(format!("{}.", k + 1)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(MONO_FONT)
                                    .text_size(dp(12.))
                                    .child(format!("{} · {}", target.name, target.address)),
                            )
                            .when(count > 1, |this| {
                                this.child(
                                    Button::new(("ops-move-earlier", k))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::ChevronUp)
                                        .tooltip("Run earlier")
                                        .disabled(k == 0)
                                        .on_click(cx.listener(move |screen, _, window, cx| {
                                            screen.move_selected(k, true, window, cx)
                                        })),
                                )
                                .child(
                                    Button::new(("ops-move-later", k))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::ChevronDown)
                                        .tooltip("Run later")
                                        .disabled(k + 1 == count)
                                        .on_click(cx.listener(move |screen, _, window, cx| {
                                            screen.move_selected(k, false, window, cx)
                                        })),
                                )
                            }),
                    )
                    .children(node.map(|node| self.verdict_row(k, node, cx)))
            })
            .collect::<Vec<_>>();
        let status: AnyElement = match &self.preview {
            PreviewState::Idle => div()
                .text_size(dp(12.5))
                .text_color(p.muted)
                .child("Select nodes in the roster to see what the operation would do.")
                .into_any_element(),
            PreviewState::Loading(_) => div()
                .text_size(dp(12.5))
                .text_color(p.muted)
                .child("Checking Kubernetes and etcd. Nothing is changed by this.")
                .into_any_element(),
            PreviewState::Failed { error, .. } => ui::warning_banner(
                Some("The preview failed.".into()),
                format!("Nothing is known about these nodes yet. {error}"),
                Some(retry_button("ops-retry", cx)),
                cx,
            )
            .into_any_element(),
            PreviewState::Ready(preview) => {
                let mut notes = vec![preview.source.clone()];
                match &preview.pdb {
                    PdbNote::Blocking(pdbs) => notes.push(format!(
                        "{} PodDisruptionBudget(s) allow no disruption and can stall a drain.",
                        pdbs.len()
                    )),
                    PdbNote::Unknown(reason) => {
                        notes.push(format!("PodDisruptionBudgets weren't read: {reason}"))
                    }
                    PdbNote::None => notes.push("No PodDisruptionBudget blocks a drain.".into()),
                    PdbNote::NotNeeded => {}
                }
                div()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child(notes.join(" "))
                    .into_any_element()
            }
        };
        let blocked = blocked_reason(&self.preview, kind);
        let busy = Operations::current(cx);
        let can_review = blocked.is_none() && busy.is_none();
        let danger = matches!(
            kind,
            OperationKind::Drain | OperationKind::Reboot | OperationKind::Shutdown
        );
        let review_label = if count > 1 {
            format!("Review and {} {count} nodes…", kind.label())
        } else {
            format!("Review and {}…", kind.label())
        };
        panel(cx)
            .id("ops-plan")
            .test_support()
            .role(Role::Group)
            .aria_label("Plan")
            .child(section_title("Plan", cx))
            .child(
                v_flex()
                    .id("ops-preview")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(match &self.preview {
                        PreviewState::Idle => "No nodes selected".to_owned(),
                        PreviewState::Loading(_) => "Checking the selected nodes".to_owned(),
                        PreviewState::Failed { error, .. } => format!("Preview failed: {error}"),
                        PreviewState::Ready(preview) => format!(
                            "Preview ready for {} node(s). {}",
                            preview.nodes.len(),
                            preview.source
                        ),
                    })
                    .px_3()
                    .pb_2()
                    .child(status),
            )
            .children(rows)
            .child(
                v_flex()
                    .gap_2()
                    .px_3()
                    .py_3()
                    .border_t_1()
                    .border_color(p.line)
                    .children((count > 0).then(|| {
                        match &blocked {
                            Some(reason) if !matches!(self.preview, PreviewState::Loading(_)) => {
                                div()
                                    .id("ops-blocked")
                                    .test_support()
                                    .role(Role::Alert)
                                    .aria_label(reason.clone())
                                    .child(ui::warning_banner(
                                        Some("Can't run this.".into()),
                                        reason.clone(),
                                        None,
                                        cx,
                                    ))
                                    .into_any_element()
                            }
                            _ => div().into_any_element(),
                        }
                    }))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("ops-review")
                                    .map(|button| {
                                        if danger {
                                            button.danger()
                                        } else {
                                            button.primary()
                                        }
                                    })
                                    .icon(IconName::Play)
                                    .label(review_label)
                                    .disabled(!can_review)
                                    .on_click(cx.listener(|screen, _, window, cx| {
                                        screen.review(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("ops-clear")
                                    .outline()
                                    .label("Clear selection")
                                    .disabled(count == 0)
                                    .on_click(cx.listener(|screen, _, window, cx| {
                                        screen.selected.clear();
                                        screen.plan_changed(window, cx);
                                    })),
                            ),
                    ),
            )
    }

    fn run_panel(
        &self,
        run: &RunView,
        current: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let heading = format!(
            "{} {}",
            kind_title(run.operation),
            run.targets
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let status: AnyElement = if run.finished {
            let (tone, text) = run_summary(run);
            h_flex()
                .id("ops-run-status")
                .test_support()
                .role(Role::Status)
                .aria_label(format!("Finished: {text}"))
                .gap_2()
                .items_center()
                .child(ui::tag(tone, None, "Finished", cx))
                .child(div().text_size(dp(12.5)).child(text))
                .into_any_element()
        } else {
            let label = if run.cancel_requested {
                "Cancelling after the current step"
            } else {
                "Running"
            };
            h_flex()
                .id("ops-run-status")
                .test_support()
                .role(Role::Status)
                .aria_label(label)
                .gap_2()
                .items_center()
                .child(ui::tag(
                    Tone::Accent,
                    Some(IconName::LoaderCircle),
                    label,
                    cx,
                ))
                .child(
                    Button::new("ops-run-cancel")
                        .outline()
                        .xsmall()
                        .label("Cancel")
                        .disabled(run.cancel_requested)
                        .tooltip("Stops before the next step; a step already sent still completes")
                        .on_click(cx.listener(|screen, _, _, cx| screen.cancel_run(cx))),
                )
                .into_any_element()
        };
        // Targets that have no result yet (still running or waiting).
        let waiting: Vec<&NodeTarget> = run
            .targets
            .iter()
            .filter(|t| !run.results.iter().any(|r| &r.target == *t))
            .collect();
        let results = run.results.iter().enumerate().map(|(i, result)| {
            let (tone, icon, label) = status_label(result.status);
            let scheduling = scheduling_text(&result.scheduling);
            let drain = result.drain.as_ref().map(drain_text);
            v_flex()
                .id(("ops-result", i))
                .test_support()
                .role(Role::ListBoxOption)
                .aria_label(format!(
                    "{} · {} · {} · {}{}",
                    result.target.name,
                    label,
                    scheduling,
                    result.message,
                    drain
                        .as_ref()
                        .map(|d| format!(" · {d}"))
                        .unwrap_or_default()
                ))
                .gap_1()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(p.line)
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(ui::tag(tone, icon, label, cx))
                        .child(
                            div()
                                .font_family(MONO_FONT)
                                .text_size(dp(12.))
                                .child(format!(
                                    "{} · {}",
                                    result.target.name, result.target.address
                                )),
                        ),
                )
                .child(div().text_size(dp(12.5)).child(result.message.clone()))
                .child(
                    div()
                        .text_size(dp(12.))
                        .text_color(p.muted)
                        .child(scheduling),
                )
                .children(
                    drain.map(|drain| div().text_size(dp(12.)).text_color(p.muted).child(drain)),
                )
                .children(result.audit_error.clone().map(|error| {
                    div()
                        .text_size(dp(12.))
                        .text_color(p.warn_ink)
                        .child(format!("Audit record not saved: {error}"))
                }))
        });
        let progress = v_flex()
            .id("ops-progress")
            .test_support()
            .role(Role::Log)
            .aria_label("Operation progress")
            .max_h(dp(220.))
            .overflow_y_scroll()
            .px_3()
            .py_2()
            .gap_0p5()
            .font_family(MONO_FONT)
            .text_size(dp(11.5))
            .text_color(p.ink_2)
            .when(run.dropped > 0, |this| {
                this.child(div().text_color(p.muted).child(format!(
                    "{} older lines omitted; the audit log keeps the record.",
                    run.dropped
                )))
            })
            .children(run.progress.iter().map(|line| div().child(line.clone())))
            .when(run.progress.is_empty(), |this| {
                this.child(div().text_color(p.muted).child("No progress yet."))
            });
        panel(cx)
            .id("ops-run")
            .test_support()
            .role(Role::Group)
            .aria_label(format!("Run: {heading}"))
            .child(section_title("Run", cx))
            .child(
                v_flex()
                    .gap_2()
                    .px_3()
                    .pb_2p5()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(dp(12.5))
                            .child(heading),
                    )
                    .child(
                        div()
                            .id("ops-run-context")
                            .test_support()
                            .role(Role::Status)
                            .aria_label(format!(
                                "Run in context {}{}",
                                run.context,
                                if run.context == current { "" } else { ", not the context shown now" }
                            ))
                            .text_size(dp(12.))
                            .text_color(p.muted)
                            .child(if run.context == current {
                                format!("in context {}", run.context)
                            } else {
                                format!(
                                    "in context {}, which is not the one you're viewing ({current}); it carries on regardless",
                                    run.context
                                )
                            }),
                    )
                    .when(run.example, |this| {
                        this.child(
                            div()
                                .text_size(dp(12.))
                                .text_color(p.muted)
                                .child("Simulated with example data. Nothing was sent to a cluster."),
                        )
                    })
                    .child(status)
                    .children(run.note.clone().map(|note| {
                        ui::warning_banner(Some("The run didn't go through.".into()), note, None, cx)
                    })),
            )
            .child(progress)
            .children(results)
            .children((!run.finished && !waiting.is_empty()).then(|| {
                div()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(p.line)
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child(format!(
                        "Waiting: {}",
                        waiting.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ")
                    ))
            }))
    }

    fn audit_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = palette(cx);
        let body: AnyElement = match (self.audit.data(), self.audit.error()) {
            (Some(view), _) => {
                let note = match &view.path {
                    Some(path) if view.entries.is_empty() => {
                        format!("No operations recorded yet in {}.", path.display())
                    }
                    Some(path) => format!(
                        "The last {} of {}",
                        view.entries.len().min(AUDIT_ENTRIES),
                        path.display()
                    ),
                    None if view.entries.is_empty() => {
                        "Example data: nothing is read from or written to the audit file. Simulated runs appear here, in memory only.".to_owned()
                    }
                    None => "Example data: simulated records, in memory only; the audit file is never touched.".to_owned(),
                };
                v_flex()
                    .child(
                        div()
                            .id("ops-audit-note")
                            .test_support()
                            .role(Role::Status)
                            .aria_label(note.clone())
                            .px_3()
                            .pb_2()
                            .text_size(dp(12.))
                            .text_color(p.muted)
                            .child(note),
                    )
                    .children(view.entries.iter().rev().enumerate().map(|(i, entry)| {
                        let when: chrono::DateTime<chrono::Local> = entry.timestamp.into();
                        let phase = phase_label(entry.phase);
                        let tone = match entry.phase {
                            AuditPhase::Success => Tone::Good,
                            AuditPhase::Failure => Tone::Crit,
                            AuditPhase::Cancelled => Tone::Warn,
                            _ => Tone::Outline,
                        };
                        h_flex()
                            .id(("ops-audit-entry", i))
                            .test_support()
                            .role(Role::ListBoxOption)
                            .aria_label(format!(
                                "{} {} {} {} {}: {}",
                                when.format("%Y-%m-%d %H:%M:%S"),
                                entry.operation.label(),
                                entry.target.name,
                                phase,
                                step_label(entry.step),
                                entry.message
                            ))
                            .gap_2()
                            .items_start()
                            .px_3()
                            .py_1p5()
                            .border_t_1()
                            .border_color(p.line)
                            .text_size(dp(12.))
                            .child(
                                div()
                                    .w(dp(124.))
                                    .flex_none()
                                    .font_family(MONO_FONT)
                                    .text_color(p.muted)
                                    .child(when.format("%m-%d %H:%M:%S").to_string()),
                            )
                            .child(ui::tag(tone, None, phase, cx))
                            .child(div().flex_1().min_w_0().child(format!(
                                "{} {} · {}: {}",
                                entry.operation.label(),
                                entry.target.name,
                                step_label(entry.step),
                                entry.message
                            )))
                    }))
                    .into_any_element()
            }
            (None, Some(error)) => div()
                .px_3()
                .pb_3()
                .child(ui::warning_banner(
                    Some("Couldn't read the audit log.".into()),
                    format!(
                        "{error} Operations still work; their records are written by the runner."
                    ),
                    Some(retry_button("ops-audit-retry", cx)),
                    cx,
                ))
                .into_any_element(),
            (None, None) => div()
                .px_3()
                .pb_3()
                .text_size(dp(12.5))
                .text_color(p.muted)
                .child("Reading the audit log…")
                .into_any_element(),
        };
        panel(cx)
            .id("ops-audit")
            .test_support()
            .role(Role::Group)
            .aria_label("Recent audit log")
            .child(section_title("Recent audit log", cx))
            .child(body)
    }
}

impl Render for OperationsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(source) = self.source.clone() else {
            return ui::empty_state(
                IconName::Server,
                "No node selected",
                "Pick a target node in the title bar.",
                None,
                Vec::new(),
                cx,
            )
            .into_any_element();
        };
        let roster = self.roster();
        let p = palette(cx);
        let busy = Operations::current(cx);
        let own_running = self.run.as_ref().is_some_and(|run| !run.finished);
        let busy_banner = busy.filter(|_| !own_running).map(|running| {
            div()
                .id("ops-busy")
                .test_support()
                .role(Role::Status)
                .aria_label(format!(
                    "{} is running{}",
                    running.label,
                    running
                        .step
                        .as_ref()
                        .map(|s| format!(": {s}"))
                        .unwrap_or_default()
                ))
                .child(ui::warning_banner(
                    Some("Another operation is running.".into()),
                    format!(
                        "{}{}. Starting a new one waits until it has finished.",
                        running.label,
                        running
                            .step
                            .as_ref()
                            .map(|s| format!(" · {s}"))
                            .unwrap_or_default()
                    ),
                    None,
                    cx,
                ))
        });
        let notice = self.notice.clone().map(|text| {
            div()
                .id("ops-notice")
                .test_support()
                .role(Role::Alert)
                .aria_label(text.clone())
                .child(ui::warning_banner(None, text, None, cx))
        });
        let nodes = self.nodes_panel(&roster, cx);
        let plan = self.plan_panel(cx);
        let wide = content_width(window) >= table_width(&COLUMNS) + PLAN_WIDTH + GAP;
        let selection = if wide {
            h_flex()
                .items_start()
                .gap(dp(GAP))
                .child(div().flex_1().min_w_0().child(nodes))
                .child(div().w(dp(PLAN_WIDTH)).flex_none().child(plan))
                .into_any_element()
        } else {
            v_flex()
                .gap(dp(GAP))
                .child(nodes)
                .child(plan)
                .into_any_element()
        };
        let run = match self.run.as_ref() {
            Some(run) => Some(self.run_panel(run, &source.target.context, cx)),
            None => None,
        };
        let audit = self.audit_panel(cx);
        let kinds = self.kinds_panel(cx);
        let options = self.options_panel(cx);
        v_flex()
            .id("ops-page")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .px(dp(crate::desktop::PAGE_PADDING))
            .pt(dp(22.))
            .pb(dp(18.))
            .gap(dp(GAP))
            .child(header("Operations", &source, Scope::Cluster, &self.audit, cx))
            .children(busy_banner)
            .children(notice)
            .child(
                div()
                    .text_size(dp(12.5))
                    .text_color(p.muted)
                    .child("Changes the cluster. Every operation is previewed, confirmed and recorded in the audit log."),
            )
            .child(
                div()
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|screen, _: &NextNode, _, cx| screen.step_cursor(1, cx)))
                    .on_action(cx.listener(|screen, _: &PreviousNode, _, cx| screen.step_cursor(-1, cx)))
                    .on_action(cx.listener(|screen, _: &FirstNode, _, cx| screen.step_cursor(isize::MIN, cx)))
                    .on_action(cx.listener(|screen, _: &LastNode, _, cx| screen.step_cursor(isize::MAX, cx)))
                    .on_action(cx.listener(|screen, _: &ToggleNode, window, cx| {
                        if let Some(target) = screen.cursor_target() {
                            screen.toggle(target, window, cx);
                        }
                    }))
                    .on_action(cx.listener(|screen, _: &MoveEarlier, window, cx| {
                        screen.move_cursor_node(true, window, cx)
                    }))
                    .on_action(cx.listener(|screen, _: &MoveLater, window, cx| {
                        screen.move_cursor_node(false, window, cx)
                    }))
                    .on_action(cx.listener(|screen, _: &ClearSelection, window, cx| {
                        screen.selected.clear();
                        screen.plan_changed(window, cx);
                    }))
                    .child(
                        v_flex()
                            .gap(dp(GAP))
                            .child(kinds)
                            .child(options)
                            .child(selection),
                    ),
            )
            .children(run)
            .child(audit)
            .into_any_element()
    }
}

#[cfg(test)]
mod ui_tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use freshkube_core::operations::{NodeTarget, OperationKind, OperationStatus};
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
    use tokio::runtime::{Builder, Runtime};

    // Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
    use super::{
        Operations, OperationsScreen, PreviewKey, PreviewState, ScreenPanel, ScreenSource,
        blocked_reason, example_preview, verdict,
    };
    use crate::backend::Target;
    use crate::{fixture, presentation};

    fn source(context: &str, node_ix: usize) -> ScreenSource {
        let nodes = presentation::node_summaries(&fixture::cluster(context, 1));
        let summary = nodes[node_ix].clone();
        ScreenSource {
            target: Target {
                epoch: 1,
                context: context.into(),
                node: summary.name.clone(),
                address: summary.address.clone(),
            },
            nodes: Arc::new(nodes),
            live: None,
        }
    }

    fn mount(
        cx: &mut TestAppContext,
        context: &str,
        step_ms: u64,
    ) -> (Runtime, Entity<OperationsScreen>, WindowHandle<Root>) {
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
        });
        // A tokio worker wakes GPUI from another thread.
        cx.executor().allow_parking();
        let source = source(context, 0);
        let mut screen = None;
        let handle = cx.open_window(size(px(1500.), px(1000.)), |window, cx| {
            let view = cx.new(|cx| {
                let mut view = OperationsScreen::new(runtime.handle().clone(), window, cx);
                view.step = Duration::from_millis(step_ms);
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

    fn wait_until(
        cx: &mut TestAppContext,
        what: &str,
        mut done: impl FnMut(&gpui_kit::App) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            cx.run_until_parked();
            if cx.read(|cx| done(cx)) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn names(screen: &Entity<OperationsScreen>, cx: &TestAppContext) -> Vec<String> {
        cx.read(|cx| {
            screen
                .read(cx)
                .selected
                .iter()
                .map(|t| t.name.clone())
                .collect()
        })
    }

    fn render(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    /// Opens the confirmation for the current selection and lets it settle.
    fn review(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("ops-review", cx);
        })
        .unwrap();
        crate::mutation::settle_confirmation(cx, handle.into());
    }

    fn confirm(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
        cx.update_window(handle.into(), |_, window, _| {
            assert!(window.find("confirm-dialog").visible());
        })
        .unwrap();
        press_ok(cx, handle);
    }

    fn press_ok(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
        cx.update_window(handle.into(), |_, window, cx| {
            // The first pointer press after a dialog opens only settles it.
            window.click("confirm-dialog", cx);
            window.click("confirm-ok", cx);
        })
        .unwrap();
        // The dialog layer only redraws when its root is notified.
        handle.update(cx, |_, _, cx| cx.notify()).unwrap();
        render(cx, handle);
    }

    fn finished(screen: &Entity<OperationsScreen>) -> impl FnMut(&gpui_kit::App) -> bool + use<> {
        let screen = screen.clone();
        move |cx| {
            screen.read(cx).run.as_ref().is_some_and(|run| run.finished)
                && Operations::current(cx).is_none()
        }
    }

    fn results(
        screen: &Entity<OperationsScreen>,
        cx: &TestAppContext,
    ) -> Vec<(String, OperationStatus)> {
        cx.read(|cx| {
            screen
                .read(cx)
                .run
                .as_ref()
                .unwrap()
                .results
                .iter()
                .map(|r| (r.target.name.clone(), r.status))
                .collect()
        })
    }

    fn set_operation(
        cx: &mut TestAppContext,
        handle: WindowHandle<Root>,
        screen: &Entity<OperationsScreen>,
        kind: OperationKind,
    ) {
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, cx| screen.set_operation(kind, window, cx));
            window.render_frame(cx);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn selection_keeps_its_order_and_the_order_can_change(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", 1);
        // The target node starts selected.
        assert_eq!(names(&screen, cx), ["talos-cp-fra1-01"]);
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(("ops-node", 3usize), cx);
            window.click(("ops-node", 1usize), cx);
            window.render_frame(cx);
            assert_eq!(window.find(("ops-node", 3usize)).selected(), Some(true));
            assert_eq!(window.find(("ops-node", 2usize)).selected(), Some(false));
            window.click(("ops-move-earlier", 2usize), cx);
            window.render_frame(cx);
        })
        .unwrap();
        let after = names(&screen, cx);
        assert_eq!(
            after,
            ["talos-cp-fra1-01", "talos-cp-fra1-02", "talos-wk-fra1-01"]
        );
        cx.update_window(handle.into(), |_, window, cx| {
            let label = window
                .find(("ops-plan-target", 1usize))
                .label()
                .unwrap()
                .to_owned();
            assert!(label.contains("talos-cp-fra1-02"), "{label}");
            // Deselecting keeps the order of the rest.
            window.click(("ops-node", 1usize), cx);
            window.render_frame(cx);
        })
        .unwrap();
        assert_eq!(names(&screen, cx), ["talos-cp-fra1-01", "talos-wk-fra1-01"]);
    }

    #[gpui_kit::test]
    fn keyboard_selects_reorders_and_clears(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", 1);
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            // Clicking the target row deselects it and puts focus in the roster.
            window.click(("ops-node", 0usize), cx);
            window.press("down", cx);
            window.press("space", cx);
            window.press("down", cx);
            window.press("enter", cx);
            window.render_frame(cx);
        })
        .unwrap();
        assert_eq!(
            names(&screen, cx),
            ["talos-cp-fra1-02", "talos-cp-fra1-03-baremetal-rack-b7"]
        );
        cx.update_window(handle.into(), |_, window, cx| {
            window.press("alt-up", cx);
            window.render_frame(cx);
        })
        .unwrap();
        assert_eq!(
            names(&screen, cx),
            ["talos-cp-fra1-03-baremetal-rack-b7", "talos-cp-fra1-02"]
        );
        cx.update_window(handle.into(), |_, window, cx| {
            window.press("escape", cx);
            window.render_frame(cx);
            let row = window
                .find(("ops-node", 1usize))
                .label()
                .unwrap()
                .to_owned();
            assert!(row.contains("not selected"), "{row}");
        })
        .unwrap();
        assert!(names(&screen, cx).is_empty());
    }

    #[test]
    fn unknown_etcd_blocks_reboot_and_shutdown_but_not_drain_or_cordon() {
        // staging-eu's etcd sample is incomplete.
        let source = source("staging-eu", 0);
        let key = |operation| PreviewKey {
            context: "staging-eu".into(),
            epoch: 1,
            operation,
            targets: vec![NodeTarget::new(
                source.target.node.clone(),
                source.target.address.clone(),
            )],
        };
        for kind in [OperationKind::Reboot, OperationKind::Shutdown] {
            let preview = example_preview(&source, key(kind));
            let v = verdict(kind, &preview.nodes[0]);
            assert_eq!(v.label, "Unknown");
            assert!(v.blocks);
            let reason = blocked_reason(&PreviewState::Ready(preview), kind).unwrap();
            assert!(reason.contains("unknown"), "{reason}");
        }
        for kind in [
            OperationKind::Drain,
            OperationKind::Cordon,
            OperationKind::Uncordon,
        ] {
            let preview = example_preview(&source, key(kind));
            assert_eq!(verdict(kind, &preview.nodes[0]).label, "No etcd impact");
            assert_eq!(blocked_reason(&PreviewState::Ready(preview), kind), None);
        }
    }

    #[test]
    fn a_sole_control_plane_is_unsafe_to_reboot() {
        let source = source("homelab", 0);
        let key = PreviewKey {
            context: "homelab".into(),
            epoch: 1,
            operation: OperationKind::Reboot,
            targets: vec![NodeTarget::new(
                source.target.node.clone(),
                source.target.address.clone(),
            )],
        };
        let preview = example_preview(&source, key);
        let reason = blocked_reason(&PreviewState::Ready(preview), OperationKind::Reboot).unwrap();
        assert!(reason.contains("refused"), "{reason}");
        let healthy = source_healthy_reboot_is_safe();
        assert_eq!(healthy, "Safe");
    }

    fn source_healthy_reboot_is_safe() -> &'static str {
        let source = source("prod-fra", 3);
        let key = PreviewKey {
            context: "prod-fra".into(),
            epoch: 1,
            operation: OperationKind::Reboot,
            targets: vec![NodeTarget::new(
                source.target.node.clone(),
                source.target.address.clone(),
            )],
        };
        let preview = example_preview(&source, key);
        verdict(OperationKind::Reboot, &preview.nodes[0]).label
    }

    #[gpui_kit::test]
    fn the_screen_refuses_reboot_on_unknown_etcd_and_says_so(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "staging-eu", 1);
        set_operation(cx, handle, &screen, OperationKind::Reboot);
        cx.update_window(handle.into(), |_, window, cx| {
            let blocked = window.find("ops-blocked").label().unwrap().to_owned();
            assert!(blocked.contains("unknown"), "{blocked}");
            let verdict = window
                .find(("ops-verdict", 0usize))
                .label()
                .unwrap()
                .to_owned();
            assert!(verdict.contains("Unknown"), "{verdict}");
            assert!(!verdict.contains("Safe"), "{verdict}");
            // Reviewing is not possible: no confirmation opens.
            window.click("ops-review", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, _| {
            assert!(window.try_find("confirm-dialog").is_none());
        })
        .unwrap();
        for kind in [OperationKind::Drain, OperationKind::Cordon] {
            set_operation(cx, handle, &screen, kind);
            cx.update_window(handle.into(), |_, window, _| {
                assert!(window.try_find("ops-blocked").is_none(), "{kind:?}");
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn confirming_in_example_mode_runs_the_simulation_and_shows_results(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", 25);
        review(cx, handle);
        cx.update_window(handle.into(), |_, window, _| {
            assert!(window.find("confirm-dialog").visible());
        })
        .unwrap();
        // Nothing runs until the user confirms.
        assert!(cx.read(|cx| Operations::current(cx).is_none()));
        confirm(cx, handle);
        assert!(cx.read(Operations::current).is_some());
        wait_until(cx, "the simulated run", finished(&screen));
        assert!(cx.read(Operations::current).is_none());
        assert_eq!(
            results(&screen, cx),
            [("talos-cp-fra1-01".to_owned(), OperationStatus::Succeeded)]
        );
        render(cx, handle);
        cx.update_window(handle.into(), |_, window, _| {
            let result = window
                .find(("ops-result", 0usize))
                .label()
                .unwrap()
                .to_owned();
            assert!(result.contains("Completed"), "{result}");
            assert!(result.contains("evicted"), "{result}");
            let context = window.find("ops-run-context").label().unwrap().to_owned();
            assert!(context.contains("prod-fra"), "{context}");
            let status = window.find("ops-run-status").label().unwrap().to_owned();
            assert!(status.contains("1 of 1 completed"), "{status}");
            let audit = window.find("ops-audit-note").label().unwrap().to_owned();
            assert!(audit.contains("Example data"), "{audit}");
            assert!(window.try_find(("ops-audit-entry", 0usize)).is_some());
        })
        .unwrap();
        // The audit shown is simulated and was never given a file.
        cx.read(|cx| {
            let view = screen.read(cx).audit.data().unwrap();
            assert!(view.path.is_none());
            assert!(!view.entries.is_empty());
        });
    }

    #[gpui_kit::test]
    fn cancelling_stops_the_simulation_between_steps(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", 200);
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(("ops-node", 1usize), cx);
        })
        .unwrap();
        review(cx, handle);
        confirm(cx, handle);
        render(cx, handle);
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("ops-run-cancel", cx);
        })
        .unwrap();
        wait_until(cx, "the run to stop", finished(&screen));
        let outcome = results(&screen, cx);
        assert_eq!(outcome.len(), 2, "{outcome:?}");
        assert!(
            outcome
                .iter()
                .any(|(_, status)| *status == OperationStatus::Cancelled),
            "{outcome:?}"
        );
        assert!(
            outcome
                .iter()
                .all(|(_, status)| *status != OperationStatus::Succeeded),
            "{outcome:?}"
        );
        render(cx, handle);
        cx.update_window(handle.into(), |_, window, _| {
            let status = window.find("ops-run-status").label().unwrap().to_owned();
            assert!(status.contains("cancelled"), "{status}");
            let first = window
                .find(("ops-result", 0usize))
                .label()
                .unwrap()
                .to_owned();
            assert!(first.contains("Cancelled"), "{first}");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn changing_the_selection_after_opening_the_confirmation_blocks_it(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", 1);
        review(cx, handle);
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, cx| {
                screen.toggle(
                    NodeTarget::new("talos-cp-fra1-02", "10.20.0.12"),
                    window,
                    cx,
                )
            });
            window.render_frame(cx);
        })
        .unwrap();
        press_ok(cx, handle);
        cx.update_window(handle.into(), |_, window, _| {
            assert!(window.find("confirm-blocked").visible());
        })
        .unwrap();
        assert!(cx.read(Operations::current).is_none());
        assert!(cx.read(|cx| screen.read(cx).run.is_none()));
    }

    #[gpui_kit::test]
    fn a_busy_slot_is_refused_before_and_while_confirming(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", 1);
        // Held by something else: reviewing explains instead of opening.
        let ticket = cx.update(|cx| {
            let operations = Operations::global(cx);
            operations
                .update(cx, |operations, cx| {
                    operations.begin("prod-fra: Reboot talos-cp-fra1-02", cx)
                })
                .ok()
                .unwrap()
        });
        render(cx, handle);
        cx.update_window(handle.into(), |_, window, cx| {
            let busy = window.find("ops-busy").label().unwrap().to_owned();
            assert!(busy.contains("Reboot talos-cp-fra1-02"), "{busy}");
            window.click("ops-review", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, _| {
            assert!(window.try_find("confirm-dialog").is_none());
        })
        .unwrap();
        // Released, the dialog opens; taken again before confirming, it refuses.
        cx.update(|cx| {
            Operations::global(cx).update(cx, |operations, cx| operations.finish(ticket, cx))
        });
        review(cx, handle);
        let ticket = cx.update(|cx| {
            Operations::global(cx)
                .update(cx, |operations, cx| operations.begin("other", cx))
                .ok()
                .unwrap()
        });
        press_ok(cx, handle);
        cx.update_window(handle.into(), |_, window, _| {
            assert!(window.find("confirm-blocked").visible());
        })
        .unwrap();
        assert!(cx.read(|cx| screen.read(cx).run.is_none()));
        drop(ticket);
    }

    #[gpui_kit::test]
    fn a_partial_failure_stays_visible(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", 2);
        // talos-wk-fra1-02 answers; talos-wk-fra1-03 does not.
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(("ops-node", 0usize), cx);
            window.click(("ops-node", 4usize), cx);
            window.click(("ops-node", 5usize), cx);
        })
        .unwrap();
        review(cx, handle);
        confirm(cx, handle);
        wait_until(cx, "the run", finished(&screen));
        assert_eq!(
            results(&screen, cx),
            [
                ("talos-wk-fra1-02".to_owned(), OperationStatus::Succeeded),
                ("talos-wk-fra1-03".to_owned(), OperationStatus::Failed),
            ]
        );
        render(cx, handle);
        cx.update_window(handle.into(), |_, window, _| {
            let status = window.find("ops-run-status").label().unwrap().to_owned();
            assert!(status.contains("1 of 2 completed"), "{status}");
            assert!(status.contains("1 failed"), "{status}");
            let failed = window
                .find(("ops-result", 1usize))
                .label()
                .unwrap()
                .to_owned();
            assert!(failed.contains("Failed"), "{failed}");
            assert!(failed.contains("Scheduling unknown"), "{failed}");
            assert!(failed.contains("could not be evicted"), "{failed}");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_run_keeps_its_context_when_the_target_changes(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", 40);
        review(cx, handle);
        confirm(cx, handle);
        cx.update_window(handle.into(), |_, window, cx| {
            let other = source("staging-eu", 0);
            screen.update(cx, |screen, cx| screen.set_source(Some(other), window, cx));
            window.render_frame(cx);
            let context = window.find("ops-run-context").label().unwrap().to_owned();
            assert!(context.contains("prod-fra"), "{context}");
            assert!(context.contains("not the context"), "{context}");
        })
        .unwrap();
        // The selection and preview belong to the new target.
        assert_eq!(names(&screen, cx), ["stg-cp-01"]);
        wait_until(cx, "the run", finished(&screen));
        assert_eq!(
            results(&screen, cx),
            [("talos-cp-fra1-01".to_owned(), OperationStatus::Succeeded)]
        );
    }
}
