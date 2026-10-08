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
//! - nodes: the shared table's `ops-list`, `ops-rows`, `ops-table-scroll`
//!   and `ops-empty`, a row `ops-node-<name>`;
//! - plan: `ops-plan`, `ops-plan-target` (row `k`), `ops-move-earlier`,
//!   `ops-move-later` (button `k`), `ops-preview`, `ops-verdict` (row `k`),
//!   `ops-blocked`, `ops-review`, `ops-clear`;
//! - run: `ops-run`, `ops-run-context`, `ops-run-status`, `ops-run-cancel`,
//!   `ops-progress`, `ops-result` (row `i`);
//! - audit: `ops-audit`, `ops-audit-note`, `ops-audit-entry` (row `i`).
mod example;
mod table;
mod view;

#[cfg(test)]
mod tests;

use example::{example_preview, example_world, run_example};
use std::collections::HashMap;

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
        PREFLIGHT_TIMEOUT, PodReference, SchedulingState, SelectionEvent, SelectionRequest,
        blocking_pdbs, etcd_impact, evaluate_operation_safety, move_target, not_started,
        ordered_sequence, panic_outcomes, preflight_nodes, run_selection,
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

use crate::resources::talos::AppliedAccess;

use super::{
    Loader, Reading, SCREEN_DEADLINE, ScreenEvent, ScreenPanel, ScreenSource, TableLoading,
    page_width, panel, reading, refresh_control, retry_button, segment, split_at,
};
use crate::backend::{self, OwnedJob};
use crate::mutation::{self, Confirmation, Operations};
use crate::palette::palette;
use crate::presentation::Role as NodeRole;
use crate::ui::{self, MONO_FONT, Tone, dp};
use freshkube_ui::status::Segment;

const CONTEXT: &str = "TalosOperations";
const PREFIX: &str = "ops";
/// The plan pane, when it sits beside the roster.
const PLAN_WIDTH: f32 = 400.;
const GAP: f32 = 14.;
const PROGRESS_LIMIT: usize = 256;
const AUDIT_ENTRIES: usize = 25;
/// How long one simulated step takes.
const SIMULATED_STEP: Duration = Duration::from_millis(650);

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
pub(crate) struct RosterNode {
    target: NodeTarget,
    role: NodeRole,
    responding: bool,
    /// The row's id and the start of its accessibility label, derived with
    /// the roster.
    element_id: SharedString,
    label: SharedString,
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
    /// The access the preview was taken with, `None` for example data. A
    /// run confirmed on this preview builds its client only through it.
    access: Option<AppliedAccess>,
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
    /// The preview's access; a live run refuses to start without it.
    access: Option<AppliedAccess>,
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
    // Core polls this future after its confirmation/cancellation gates and
    // catches access failures; it mutates nothing until the client is in.
    let applied = live.applied.clone();
    let connect = run_client(plan.access, applied, async move {
        live.forget_kubernetes();
        live.kubernetes().await
    });
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

/// The run's Kubernetes client, built only through the access its preview
/// was taken with: the confirmed source must carry the same access identity,
/// and the local files must keep the preview's revision before and after
/// `connect` builds the client. Any change refuses the run, with nothing
/// submitted, until the user refreshes and reviews again.
async fn run_client<T>(
    previewed: Option<AppliedAccess>,
    confirmed: Option<AppliedAccess>,
    connect: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    let Some(previewed) = previewed else {
        return Err("The preview didn't record its access; refresh and review again".into());
    };
    if confirmed.map(|access| access.identity) != Some(previewed.identity) {
        return Err("Access changed since the preview; refresh and review again".into());
    }
    previewed.client(connect).await
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
    operation: OperationKind,
    options: Options,
    /// Selected targets in run order.
    selected: Vec<NodeTarget>,
    /// Each selected node's place in the run, by name, derived when the
    /// selection changes, for the table's Order column.
    order: HashMap<String, usize>,
    cursor: usize,
    /// The source's nodes that can be targeted, and the table's columns
    /// for them, derived when the source changes.
    roster: Vec<RosterNode>,
    columns: (Vec<table::Column>, f32),
    table: freshkube_ui::table::TableState,
    /// The roster's loading rows while the overview is read.
    loading: TableLoading,
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
    /// The status bar's segment, none without a target, and the audit
    /// revision and source it was derived from.
    status: Option<(StatusKey, Option<Segment>)>,
}

/// What the status bar's segment shows: the audit log's revision, and the
/// context and whether it is example data.
type StatusKey = (u64, Option<(String, bool)>);

impl EventEmitter<ScreenEvent> for OperationsScreen {}

impl ScreenPanel for OperationsScreen {
    fn loading_motion(&self, cx: &App) -> Option<Entity<freshkube_ui::table::LoadingMotion>> {
        self.loading.motion(self.first_read(cx))
    }

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
            operation: OperationKind::Drain,
            options: Options::default(),
            selected: Vec::new(),
            order: HashMap::new(),
            cursor: 0,
            roster: Vec::new(),
            columns: table::columns(&[]),
            table: freshkube_ui::table::TableState::new(PREFIX),
            loading: TableLoading::new(PREFIX, cx),
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
            status: None,
        }
    }

    fn set_source(&mut self, source: Option<ScreenSource>, _: &mut Window, cx: &mut Context<Self>) {
        // Every retained screen gets the source, shown or not, so this only
        // invalidates; `activate` reads, and only the shown screen is activated.
        let changed = self.source.as_ref().map(|source| &source.target)
            != source.as_ref().map(|source| &source.target);
        self.source = source;
        self.derive_roster();
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
                    self.roster
                        .iter()
                        .find(|node| node.target.name == source.target.node)
                })
                .map(|node| vec![node.target.clone()])
                .unwrap_or_default();
            self.cursor = 0;
        } else {
            // Same target, possibly a new roster: drop what is no longer there
            // or whose address changed.
            let roster = &self.roster;
            let before = self.selected.len();
            self.selected
                .retain(|target| roster.iter().any(|node| &node.target == target));
            if self.selected.len() != before {
                self.drop_preview();
            }
        }
        self.derive_order();
        self.cursor = self.cursor.min(self.roster.len().saturating_sub(1));
        cx.notify();
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_preview(false, window, cx);
        if self.audit.data().is_none() && !self.audit.is_loading() {
            self.load_audit(cx);
        }
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn status(&mut self) -> Option<&Segment> {
        self.sync_status();
        self.status.as_ref().and_then(|(_, line)| line.as_ref())
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.notice = None;
        self.ensure_preview(true, window, cx);
        self.load_audit(cx);
        cx.notify();
    }
}

impl OperationsScreen {
    /// Derives the status bar's segment again when the audit log or the
    /// source changed; the shell asks for it before the screen renders.
    fn sync_status(&mut self) {
        let key = (
            self.audit.revision(),
            self.source
                .as_ref()
                .map(|source| (source.target.context.clone(), source.is_example())),
        );
        if self.status.as_ref().is_some_and(|(at, _)| *at == key) {
            return;
        }
        let line = self
            .source
            .as_ref()
            .map(|source| segment(Some(source), &self.audit, []));
        self.status = Some((key, line));
    }

    /// Whether the roster is still to come: the overview is being read and
    /// there is no source yet. The table shows its loading rows meanwhile.
    fn first_read(&self, cx: &App) -> bool {
        self.source.is_none() && reading(cx) == Reading::Waiting
    }

    /// The source's nodes that can be targeted, and the table's columns.
    fn derive_roster(&mut self) {
        self.roster = self
            .source
            .iter()
            .flat_map(|source| source.nodes.iter())
            .filter(|node| !node.name.trim().is_empty() && !node.address.trim().is_empty())
            .map(|node| {
                RosterNode::new(
                    NodeTarget::new(node.name.clone(), node.address.clone()),
                    node.role,
                    node.responding,
                )
            })
            .collect();
        self.columns = table::columns(&self.roster);
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
        let Some(access) = live.applied.clone() else {
            // The overview hasn't said which access it used yet.
            self.preview = PreviewState::Failed {
                key,
                error: "The access configuration isn't known yet; refresh".into(),
            };
            self.confirm_when_ready = false;
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
            async move { preview_live(live, access, endpoint, work_key).await },
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
        self.derive_order();
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
        let len = self.roster.len();
        if len == 0 {
            return;
        }
        self.cursor = self.cursor.saturating_add_signed(delta).min(len - 1);
        cx.notify();
    }

    fn cursor_target(&self) -> Option<NodeTarget> {
        self.roster.get(self.cursor).map(|node| node.target.clone())
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
            access: preview.access.clone(),
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
    access: AppliedAccess,
    endpoint: InspectionTarget,
    key: PreviewKey,
) -> Result<Preview, String> {
    let kubernetes = access.client(live.kubernetes()).await?;
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
        access: Some(access),
    })
}
