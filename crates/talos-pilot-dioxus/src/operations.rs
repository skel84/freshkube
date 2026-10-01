//! Confirmed, session-owned operations. Dropping a panel requests cancellation; it never
//! aborts a mutation or releases the global lock before compensation and auditing finish.
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::feature::{FeatureContext, LogRequest};
use dioxus::prelude::*;
use futures::FutureExt;
use parking_lot::Mutex;
use talos_pilot_core::{
    indicators::SafetyStatus,
    operations::{
        AuditLog, DrainOptions, EtcdQuorumImpact, NodeOperationRequest, NodeOperationResult,
        NodeTarget, OperationConfirmation, OperationContext, OperationKind, OperationsEvent,
        POD_SAMPLE_LIMIT, TARGET_LIMIT, audit_result, evaluate_operation_safety, move_target,
        not_started, ordered_sequence, panic_outcomes, preflight_nodes, run_node_operation,
        selection_toggle,
    },
};

const PROGRESS_LIMIT: usize = 256;
const NODE_PAGE_SIZE: usize = 32;
const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(45);
static BUSY: AtomicBool = AtomicBool::new(false);

pub(crate) fn operation_busy() -> bool {
    BUSY.load(Ordering::Acquire)
}
fn active_cancellation() -> &'static Mutex<Option<Arc<AtomicBool>>> {
    static CANCELLATION: OnceLock<Mutex<Option<Arc<AtomicBool>>>> = OnceLock::new();
    CANCELLATION.get_or_init(|| Mutex::new(None))
}
/// Window-close integration requests cancellation, then waits for operation_busy() == false.
/// This does not abort a task and also covers diagnostics/restart/maintenance mutation leases.
pub(crate) fn cancel_current_mutation() {
    if let Some(cancellation) = &*active_cancellation().lock() {
        cancellation.store(true, Ordering::Release);
    }
}

/// A mutation lease is owned by the runtime worker, not a component's future.
#[derive(Debug)]
pub(crate) struct MutationGuard {
    busy: &'static AtomicBool,
    cancellation: Arc<AtomicBool>,
}
impl MutationGuard {
    pub(crate) fn cancellation(&self) -> Arc<AtomicBool> {
        self.cancellation.clone()
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.cancellation.load(Ordering::Acquire)
    }
    #[allow(dead_code)] // Optional shared caller API; most callers retain the cancellation flag.
    pub(crate) fn cancel(&self) {
        self.cancellation.store(true, Ordering::Release);
    }
}
impl Drop for MutationGuard {
    fn drop(&mut self) {
        if std::ptr::eq(self.busy, &BUSY) {
            *active_cancellation().lock() = None;
        }
        self.busy.store(false, Ordering::Release);
    }
}
pub(crate) type OperationGuard = MutationGuard;
pub(crate) fn try_begin_operation(_label: String) -> Result<OperationGuard, String> {
    try_acquire_mutation()
}
pub(crate) fn try_acquire_mutation() -> Result<MutationGuard, String> {
    // Serialize registration with close cancellation so it cannot miss a newly acquired lease.
    let mut active = active_cancellation().lock();
    let guard = acquire_mutation(&BUSY)?;
    *active = Some(guard.cancellation());
    Ok(guard)
}
fn acquire_mutation(busy: &'static AtomicBool) -> Result<MutationGuard, String> {
    busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| {
            "Another mutation is still running (including compensation/audit).".to_owned()
        })?;
    Ok(MutationGuard {
        busy,
        cancellation: Arc::new(AtomicBool::new(false)),
    })
}

#[derive(Clone, Debug, Default)]
struct JobSnapshot {
    id: u64,
    key: String,
    context: String,
    running: bool,
    progress: VecDeque<String>,
    dropped_progress: usize,
    results: Vec<NodeOperationResult>,
    message: String,
    audit_path: PathBuf,
}
impl JobSnapshot {
    fn push(&mut self, message: String) {
        if self.progress.len() == PROGRESS_LIMIT {
            self.progress.pop_front();
            self.dropped_progress += 1;
        }
        self.progress
            .push_back(message.chars().take(2048).collect());
    }
    fn completed(&self) -> usize {
        self.results
            .iter()
            .filter(|r| r.status.is_success())
            .count()
    }
}
#[derive(Default)]
struct Registry {
    snapshot: JobSnapshot,
    cancellation: Option<Arc<AtomicBool>>,
    // Retained even if the tab unmounts. Never abort a mutating worker.
    handle: Option<tokio::task::JoinHandle<()>>,
}
fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}
fn snapshot() -> JobSnapshot {
    registry().lock().snapshot.clone()
}
fn cancel_for(key: &str) {
    let registry = registry().lock();
    if registry.snapshot.running
        && registry.snapshot.key == key
        && let Some(cancel) = &registry.cancellation
    {
        cancel.store(true, Ordering::Release);
    }
}
fn progress(id: u64, event: OperationsEvent) {
    let line = match event {
        OperationsEvent::Operation(e) => format!(
            "{} ({}) · {:?} / {:?}: {}",
            e.target.name, e.target.address, e.phase, e.step, e.message
        ),
        OperationsEvent::Rolling(e) => format!(
            "Rolling {}/{} · {:?}: {}",
            e.current_node_index + 1,
            e.total_nodes,
            e.step,
            e.message
        ),
    };
    let mut registry = registry().lock();
    if registry.snapshot.id == id {
        registry.snapshot.push(line);
    }
}

fn audit_path(context: &str) -> PathBuf {
    talos_pilot_core::operations::default_audit_path(context)
}
fn roster(ctx: &FeatureContext) -> Vec<NodeTarget> {
    let mut nodes: Vec<_> = ctx
        .cluster
        .node_ips
        .iter()
        .filter(|(name, address)| !name.trim().is_empty() && !address.trim().is_empty())
        .map(|(name, address)| NodeTarget::new(name, address))
        .collect();
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    nodes
}
fn confirmation_phrase(context: &str, operation: OperationKind, targets: &[NodeTarget]) -> String {
    format!(
        "{} {} {}",
        context,
        operation.label(),
        targets
            .iter()
            .map(|n| format!("{}@{}", n.name, n.address))
            .collect::<Vec<_>>()
            .join(",")
    )
}
fn confirmed(
    context: &str,
    operation: OperationKind,
    targets: &[NodeTarget],
    text: &str,
    acknowledged: bool,
) -> bool {
    acknowledged
        && !targets.is_empty()
        && targets.len() <= TARGET_LIMIT
        && matches!(operation, OperationKind::Drain | OperationKind::Reboot)
        && targets
            .iter()
            .enumerate()
            .all(|(index, target)| !targets[..index].contains(target))
        && text == confirmation_phrase(context, operation, targets)
}
#[derive(Clone)]
struct Preflight {
    kubernetes: kube::Client,
    impacts: Vec<EtcdQuorumImpact>,
    summary: Vec<String>,
}
async fn preflight(ctx: FeatureContext, targets: Vec<NodeTarget>) -> Result<Preflight, String> {
    tokio::time::timeout(PREFLIGHT_TIMEOUT, async {
        let (kubernetes, source, warning) = ctx.kubernetes_with_source().await?;
        let mut summary = vec![format!("Validated selected Kubernetes source: {source}")];
        if let Some(warning) = warning {
            summary.push(format!("Source warning: {warning}"));
        }
        let checked = preflight_nodes(
            &kubernetes,
            ctx.cluster
                .client
                .clone()
                .unwrap_or_else(|| ctx.client.clone()),
            ctx.inspection_target(),
            &targets,
        )
        .await?;
        for node in &checked {
            summary.push(format!(
                "{} ({}) Ready={} cordoned={} pods={} (sample limit {POD_SAMPLE_LIMIT})",
                node.target.name,
                node.target.address,
                node.ready,
                node.unschedulable,
                node.pod_count
            ));
        }
        let impacts = checked
            .into_iter()
            .map(|node| node.impact)
            .collect::<Vec<_>>();
        for (target, impact) in targets.iter().zip(&impacts) {
            summary.push(format!(
                "{} reboot safety: {:?}",
                target.name,
                evaluate_operation_safety(OperationKind::Reboot, impact)
            ));
        }
        Ok(Preflight {
            kubernetes,
            impacts,
            summary,
        })
    })
    .await
    .map_err(|_| "Latest prechecks timed out; no mutation was made".to_owned())?
}

struct OperationSubmission {
    operation: OperationKind,
    targets: Vec<NodeTarget>,
    text: String,
    acknowledged: bool,
    rolling: bool,
    stop_on_failure: bool,
    delay: u64,
    delete_emptydir: bool,
}

impl OperationSubmission {
    fn validate(&self, context: &str, authoritative: &[NodeTarget]) -> Result<(), String> {
        if !confirmed(
            context,
            self.operation,
            &self.targets,
            &self.text,
            self.acknowledged,
        ) || (!self.rolling && self.targets.len() != 1)
        {
            return Err(
                "Type the exact context/action/ordered-target confirmation and acknowledge disruption"
                    .to_owned(),
            );
        }
        if self
            .targets
            .iter()
            .any(|target| !authoritative.contains(target))
        {
            return Err(
                "Target is no longer in the selected context's authoritative roster".to_owned(),
            );
        }
        Ok(())
    }
}

fn start(ctx: FeatureContext, submission: OperationSubmission) -> Result<(), String> {
    submission.validate(&ctx.context, &roster(&ctx))?;
    let OperationSubmission {
        operation,
        targets,
        text: _,
        acknowledged: _,
        rolling,
        stop_on_failure,
        delay,
        delete_emptydir,
    } = submission;
    let guard = try_begin_operation(format!("{} {}", ctx.context, operation.label()))?;
    let cancellation = guard.cancellation();
    let key = ctx.key();
    let audit_path = audit_path(&ctx.context);
    let id = {
        let mut registry = registry().lock();
        let id = registry.snapshot.id.wrapping_add(1);
        registry.snapshot = JobSnapshot {
            id,
            key,
            context: ctx.context.clone(),
            running: true,
            audit_path: audit_path.clone(),
            message: "Checking latest Kubernetes and etcd state before any mutation".to_owned(),
            ..Default::default()
        };
        registry.cancellation = Some(cancellation.clone());
        id
    };
    let runtime = ctx.runtime.clone();
    let handle = runtime.spawn(async move {
        let audit = AuditLog::new(audit_path, ctx.context.clone());
        // Catch panics so the retained result is truthful and the mutation lease is released.
        let run = std::panic::AssertUnwindSafe(async {
            if guard.cancelled() { return Err("Cancelled before latest prechecks; no mutation was made".to_owned()); }
            let latest = preflight(ctx.clone(), targets.clone()).await?;
            { let mut r = registry().lock(); for line in latest.summary { r.snapshot.push(line); } }
            let cancellation_for_context = cancellation.clone();
            let cancelled = move || cancellation_for_context.load(Ordering::Acquire);
            let context = OperationContext { kubernetes: &latest.kubernetes, talos: Some(&ctx.client), audit: &audit, is_cancelled: &cancelled };
            let mut callback = move |event| progress(id, event);
            let options = DrainOptions { delete_emptydir_data: delete_emptydir, ..DrainOptions::default() };
            if rolling {
                if let Some((target, safety)) = targets.iter().zip(&latest.impacts).find_map(|(target, impact)| {
                    let safety = evaluate_operation_safety(operation, impact);
                    matches!(safety, SafetyStatus::Unknown | SafetyStatus::Unsafe(_)).then_some((target, safety))
                }) { return Err(format!("Whole-selection preflight blocked {}: {safety:?}", target.name)); }
                let total = targets.len();
                let results = ordered_sequence(operation, &targets, stop_on_failure, Duration::from_secs(delay.min(600)), &|| cancellation.load(Ordering::Acquire), |index, target| {
                    let ctx = ctx.clone(); let audit = audit.clone(); let cancellation = cancellation.clone(); let options = options.clone();
                    async move {
                        { let mut r = registry().lock(); r.snapshot.push(format!("Rolling {}/{}: fresh etcd and Kubernetes prechecks for {} ({})", index + 1, total, target.name, target.address)); }
                        let fresh = preflight(ctx.clone(), vec![target.clone()]).await;
                        let result = match fresh {
                            Ok(fresh) => {
                                { let mut r = registry().lock(); for line in fresh.summary { r.snapshot.push(line); } }
                                let cancellation_for_context = cancellation.clone();
                                let cancelled = move || cancellation_for_context.load(Ordering::Acquire);
                                let context = OperationContext { kubernetes: &fresh.kubernetes, talos: Some(&ctx.client), audit: &audit, is_cancelled: &cancelled };
                                let mut request = NodeOperationRequest::new(operation, target);
                                request.etcd_impact = fresh.impacts[0].clone(); request.drain_options = options;
                                let mut callback = move |event| progress(id, event);
                                run_node_operation(request, &context, OperationConfirmation::confirmed(), &mut callback).await
                            }
                            Err(error) => not_started(operation, &[target], &error, cancellation.load(Ordering::Acquire)).remove(0),
                        };
                        { let mut r = registry().lock(); if r.snapshot.id == id { r.snapshot.results.push(result.clone()); } }
                        result
                    }
                }).await;
                let completed = results.iter().filter(|r| r.status.is_success()).count();
                let message = format!("Sequential {} completed {completed}/{total} targets; remaining statuses are retained individually", operation.label());
                Ok::<_, String>((results, message))
            } else {
                let mut request = NodeOperationRequest::new(operation, targets[0].clone());
                request.etcd_impact = latest.impacts[0].clone(); request.drain_options = options;
                let result = run_node_operation(request, &context, OperationConfirmation::confirmed(), &mut callback).await;
                let message = result.message.clone(); Ok::<_, String>((vec![result], message))
            }
        }).catch_unwind().await;
        let (results, message) = match run {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => (not_started(operation, &targets, &error, guard.cancelled()), error),
            Err(_) => (panic_outcomes(operation, &targets, snapshot().results), "Operation worker panicked; inspect node state and audit".to_owned()),
        };
        let mut results = results;
        for result in &mut results { audit_result(&audit, &ctx.context, result); }
        let mut registry = registry().lock();
        if registry.snapshot.id == id { registry.snapshot.results = results; registry.snapshot.message = message; registry.snapshot.running = false; registry.cancellation = None; }
        drop(registry);
        drop(guard); // Only after core compensation and audit have finished.
    });
    registry().lock().handle = Some(handle);
    Ok(())
}

#[component]
pub(crate) fn OperationsPanel(ctx: FeatureContext, on_logs: EventHandler<LogRequest>) -> Element {
    let nodes = roster(&ctx);
    let selected_ctx = ctx.clone();
    let mut selection = use_signal(move || {
        nodes
            .iter()
            .find(|n| n.name == selected_ctx.node && n.address == selected_ctx.address)
            .cloned()
            .into_iter()
            .collect::<Vec<_>>()
    });
    let mut node_page = use_signal(|| 0_usize);
    let mut rolling = use_signal(|| false);
    let mut reboot = use_signal(|| false);
    let mut stop_on_failure = use_signal(|| true);
    let mut delay = use_signal(|| 30_u64);
    let mut delete_emptydir = use_signal(|| true);
    let mut text = use_signal(String::new);
    let mut acknowledged = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut checks = use_signal(Vec::<String>::new);
    let mut checking = use_signal(|| false);
    let mut job = use_signal(snapshot);
    let poll_ctx = ctx.clone();
    use_future(move || {
        let ctx = poll_ctx.clone();
        async move {
            loop {
                job.set(snapshot());
                if ctx
                    .run(async { tokio::time::sleep(Duration::from_millis(250)).await })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    });
    let close_key = ctx.key();
    use_drop(move || cancel_for(&close_key));
    let operation = if reboot() {
        OperationKind::Reboot
    } else {
        OperationKind::Drain
    };
    let targets = if rolling() {
        selection()
    } else {
        selection().into_iter().take(1).collect()
    };
    let phrase = confirmation_phrase(&ctx.context, operation, &targets);
    let can_start =
        confirmed(&ctx.context, operation, &targets, &text(), acknowledged()) && !operation_busy();
    let display = job();
    let live = display.running;
    let logs_match_context = display.key == ctx.key();
    let all_nodes = roster(&ctx);
    let node_count = all_nodes.len();
    let page_count = node_count.div_ceil(NODE_PAGE_SIZE).max(1);
    let page = node_page().min(page_count - 1);
    let visible_nodes: Vec<_> = all_nodes
        .into_iter()
        .skip(page * NODE_PAGE_SIZE)
        .take(NODE_PAGE_SIZE)
        .collect();
    let preflight_ctx = ctx.clone();
    let start_ctx = ctx.clone();
    let cancel_key = ctx.key();
    rsx! {
        section { class: "panel",
            h2 { "Node and rolling operations" }
            p { "Actual Talos context: {ctx.context}. Targets use Kubernetes names and separate Talos addresses. Rolling operations are sequential." }
            p { "No force deletion. Reboot drains, verifies a reboot transition and Kubernetes Ready, then restores scheduling only if this operation cordoned the node." }
            label { input { r#type: "checkbox", checked: rolling(), disabled: live, onchange: move |e| { rolling.set(e.checked()); text.set(String::new()); acknowledged.set(false); } } "Rolling multi-node operation" }
            label { input { r#type: "checkbox", checked: reboot(), disabled: live, onchange: move |e| { reboot.set(e.checked()); text.set(String::new()); acknowledged.set(false); } } "Reboot after drain (otherwise drain only)" }
            p { "{node_count} roster targets · page {page + 1}/{page_count}. At most {TARGET_LIMIT} nodes may be selected per rolling run." }
            button { disabled: live || page == 0, onclick: move |_| node_page.set(page.saturating_sub(1)), "Previous nodes" }
            button { disabled: live || page + 1 >= page_count, onclick: move |_| node_page.set(page + 1), "Next nodes" }
            div { class: "scroll-pane",
                for target in visible_nodes {
                    { let selected = selection.read().contains(&target); let toggle = target.clone(); rsx! {
                        label { key: "{target.name}@{target.address}",
                            input { r#type: "checkbox", checked: selected, disabled: live, onchange: move |_| {
                                if rolling() { selection_toggle(&mut selection.write(), toggle.clone()); } else { selection.set(vec![toggle.clone()]); }
                                text.set(String::new()); acknowledged.set(false); checks.set(Vec::new());
                            } }
                            "{target.name} ({target.address})"
                        }
                    } }
                }
            }
            h3 { "Confirmed execution order" }
            for (index, target) in targets.iter().enumerate() {
                div { key: "ordered-{target.name}", "{index + 1}. {target.name} ({target.address})"
                    if rolling() {
                        button { aria_label: "Move {target.name} ({target.address}) earlier", disabled: live || index == 0, onclick: move |_| { move_target(&mut selection.write(), index, true); text.set(String::new()); acknowledged.set(false); }, "↑" }
                        button { aria_label: "Move {target.name} ({target.address}) later", disabled: live || index + 1 == targets.len(), onclick: move |_| { move_target(&mut selection.write(), index, false); text.set(String::new()); acknowledged.set(false); }, "↓" }
                    }
                }
            }
            if rolling() {
                label { input { r#type: "checkbox", checked: stop_on_failure(), disabled: live, onchange: move |e| stop_on_failure.set(e.checked()) } "Stop on first failure (cancellation always stops)" }
                label { "Delay between nodes (0–600 seconds)" input { r#type: "number", min: "0", max: "600", value: "{delay}", disabled: live, oninput: move |e| if let Ok(value) = e.value().parse::<u64>() { delay.set(value.min(600)); } } }
            }
            label { input { r#type: "checkbox", checked: delete_emptydir(), disabled: live, onchange: move |e| { delete_emptydir.set(e.checked()); acknowledged.set(false); } } "Allow eviction of pods with emptyDir (ephemeral data is lost)" }
            button { disabled: checking() || live || targets.is_empty(), onclick: move |_| {
                let ctx = preflight_ctx.clone(); let targets = if rolling() { selection() } else { selection().into_iter().take(1).collect() };
                checking.set(true); error.set(None);
                spawn(async move {
                    let worker = ctx.clone(); let expected = targets.clone();
                    let result = ctx.run(preflight(worker, targets)).await;
                    checking.set(false);
                    let current: Vec<_> = if rolling() { selection() } else { selection().into_iter().take(1).collect() };
                    if current != expected { return; }
                    match result { Ok(Ok(result)) => checks.set(result.summary), Ok(Err(e)) | Err(e) => error.set(Some(e)) }
                });
            }, "Check latest state" }
            for line in checks() { p { "{line}" } }
            p { "Execution always repeats latest prechecks; reboot is blocked when etcd impact is unknown or unsafe." }
            p { id: "operation-confirmation-phrase", "Type exactly: {phrase}" }
            label { r#for: "operation-confirmation", "Exact operation confirmation" }
            input { id: "operation-confirmation", aria_describedby: "operation-confirmation-phrase", value: text(), disabled: live, oninput: move |e| text.set(e.value()), placeholder: "context action ordered-node@address,..." }
            label { input { r#type: "checkbox", checked: acknowledged(), disabled: live, onchange: move |e| acknowledged.set(e.checked()) } "I confirm the actual context, ordered target(s), disruption and drain/data-loss options above." }
            button { disabled: !can_start, onclick: move |_| {
                let targets = if rolling() { selection() } else { selection().into_iter().take(1).collect() };
                let submission = OperationSubmission {
                    operation: if reboot() { OperationKind::Reboot } else { OperationKind::Drain },
                    targets,
                    text: text(),
                    acknowledged: acknowledged(),
                    rolling: rolling(),
                    stop_on_failure: stop_on_failure(),
                    delay: delay(),
                    delete_emptydir: delete_emptydir(),
                };
                match start(start_ctx.clone(), submission) { Ok(()) => { job.set(snapshot()); text.set(String::new()); acknowledged.set(false); error.set(None); }, Err(e) => error.set(Some(e)) }
            }, "Execute confirmed operation" }
            if live { button { onclick: move |_| cancel_for(&cancel_key), "Cancel safely (wait for compensation/audit)" } }
            if let Some(message) = error() { p { class: "error", "{message}" } }
            if display.id != 0 {
                h3 { "Session operation · {display.context}" }
                p { "{display.message}" }
                p { "Audit: {display.audit_path.display()}" }
                if display.running { p { "Running; navigation/settings remain locked until compensation/audit finish." } }
                if !display.results.is_empty() { p { "Completed {display.completed()}/{display.results.len()} targets. Failed, cancelled and unstarted nodes are not successes." } }
                div { class: "scroll-pane text-review-region", tabindex: "0", role: "region", aria_label: "Operation results and scheduling evidence",
                    for result in &display.results {
                        article { key: "result-{result.target.name}",
                            strong { "{result.target.name} ({result.target.address}) · {result.status:?}" }
                            p { "{result.message}" }
                            p { "Scheduling: {result.scheduling:?}" }
                            if let Some(drain) = &result.drain { p { "Drain: {drain:?}" } }
                            if let Some(audit_error) = &result.audit_error { p { class: "error", "AUDIT ERROR: {audit_error}" } }
                            { let request = LogRequest { node: Some(result.target.name.clone()), address: Some(result.target.address.clone()), services: vec!["kubelet".to_owned()] }; rsx! { button { disabled: live || !logs_match_context, onclick: move |_| on_logs.call(request.clone()), "Kubelet logs" } } }
                        }
                    }
                }
                h3 { "Bounded progress" }
                if display.dropped_progress != 0 { p { "{display.dropped_progress} older progress messages omitted; structured audit retains operation records." } }
                div { class: "scroll-pane text-review-region", tabindex: "0", role: "region", aria_label: "Operation progress", for line in &display.progress { p { "{line}" } } }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::Node;
    use talos_pilot_core::indicators::SafetyStatus;
    use talos_pilot_core::inspection::EtcdHealthSnapshot;
    use talos_pilot_core::operations::node_matches_target;
    use talos_pilot_core::operations::{OperationStatus, SchedulingState, etcd_impact};
    fn node(name: &str) -> NodeTarget {
        NodeTarget::new(name, format!("10.0.0.{}", if name == "a" { 1 } else { 2 }))
    }

    fn submission(targets: Vec<NodeTarget>, rolling: bool) -> OperationSubmission {
        OperationSubmission {
            operation: OperationKind::Reboot,
            text: confirmation_phrase("production", OperationKind::Reboot, &targets),
            targets,
            acknowledged: true,
            rolling,
            stop_on_failure: true,
            delay: 30,
            delete_emptydir: false,
        }
    }

    #[test]
    fn submission_requires_exact_confirmation_and_current_authoritative_targets() {
        let authoritative = vec![node("a"), node("b")];
        let mut request = submission(vec![node("a"), node("b")], true);
        assert!(request.validate("production", &authoritative).is_ok());
        assert!(request.validate("staging", &authoritative).is_err());
        request.acknowledged = false;
        assert!(request.validate("production", &authoritative).is_err());
        request.acknowledged = true;
        request.targets.swap(0, 1);
        assert!(request.validate("production", &authoritative).is_err());
        request.targets.swap(0, 1);
        request.targets[0].address = "10.0.0.99".to_owned();
        request.text = confirmation_phrase("production", request.operation, &request.targets);
        assert!(request.validate("production", &authoritative).is_err());
        request.targets[0] = node("a");
        request.text = confirmation_phrase("production", request.operation, &request.targets);
        assert!(request.validate("production", &[node("a")]).is_err());
        request.rolling = false;
        assert!(request.validate("production", &authoritative).is_err());
        request.targets = vec![node("a")];
        request.text = confirmation_phrase("production", request.operation, &request.targets);
        assert!(request.validate("production", &authoritative).is_ok());
        request.text.push(' ');
        assert!(request.validate("production", &authoritative).is_err());
    }

    #[test]
    fn structured_submission_retains_explicit_rolling_and_data_loss_options() {
        let mut request = submission(vec![node("b"), node("a")], true);
        request.stop_on_failure = false;
        request.delay = 123;
        request.delete_emptydir = true;
        assert!(
            request
                .validate("production", &[node("a"), node("b")])
                .is_ok()
        );
        assert_eq!(request.targets, [node("b"), node("a")]);
        assert!(request.rolling);
        assert!(!request.stop_on_failure);
        assert_eq!(request.delay, 123);
        assert!(request.delete_emptydir);
    }

    #[test]
    fn confirmation_and_scroll_evidence_have_accessible_keyboard_markup() {
        // The RSX contract is checked without mounting a desktop window or a real mutation.
        let markup = include_str!("operations.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(
            markup.contains(
                "p { id: \"operation-confirmation-phrase\", \"Type exactly: {phrase}\" }"
            )
        );
        assert!(markup.contains(
            "label { r#for: \"operation-confirmation\", \"Exact operation confirmation\" }"
        ));
        assert!(markup.contains(
            "input { id: \"operation-confirmation\", aria_describedby: \"operation-confirmation-phrase\""
        ));
        for label in [
            "Operation results and scheduling evidence",
            "Operation progress",
        ] {
            assert!(markup.contains(&format!(
                "class: \"scroll-pane text-review-region\", tabindex: \"0\", role: \"region\", aria_label: \"{label}\""
            )));
        }
        assert!(markup.contains("aria_label: \"Move {target.name} ({target.address}) earlier\""));
        assert!(markup.contains("aria_label: \"Move {target.name} ({target.address}) later\""));
    }

    #[test]
    fn confirmation_pins_context_action_address_and_order() {
        let targets = vec![node("a"), node("b")];
        let phrase = confirmation_phrase("production", OperationKind::Reboot, &targets);
        assert!(!confirmed(
            "production",
            OperationKind::Reboot,
            &targets,
            &phrase,
            false
        ));
        assert!(confirmed(
            "production",
            OperationKind::Reboot,
            &targets,
            &phrase,
            true
        ));
        assert!(!confirmed(
            "staging",
            OperationKind::Reboot,
            &targets,
            &phrase,
            true
        ));
        assert!(!confirmed(
            "production",
            OperationKind::Drain,
            &targets,
            &phrase,
            true
        ));
        assert!(!confirmed(
            "production",
            OperationKind::Reboot,
            &[node("b"), node("a")],
            &phrase,
            true
        ));
        assert!(!confirmed(
            "production",
            OperationKind::Reboot,
            &[],
            &phrase,
            true
        ));
        let duplicates = vec![node("a"), node("a")];
        assert!(!confirmed(
            "production",
            OperationKind::Reboot,
            &duplicates,
            &confirmation_phrase("production", OperationKind::Reboot, &duplicates),
            true
        ));
    }
    #[test]
    fn selection_order_survives_toggle_and_reorder() {
        let mut selected = vec![];
        selection_toggle(&mut selected, node("b"));
        selection_toggle(&mut selected, node("a"));
        assert_eq!(selected, vec![node("b"), node("a")]);
        move_target(&mut selected, 1, true);
        assert_eq!(selected, vec![node("a"), node("b")]);
        selection_toggle(&mut selected, node("a"));
        assert_eq!(selected, vec![node("b")]);
    }
    #[test]
    fn progress_is_bounded_and_results_do_not_hide_partial_failure() {
        let mut job = JobSnapshot::default();
        for i in 0..PROGRESS_LIMIT + 12 {
            job.push(i.to_string());
        }
        assert_eq!(job.progress.len(), PROGRESS_LIMIT);
        assert_eq!(job.dropped_progress, 12);
        let mut results = not_started(
            OperationKind::Reboot,
            &[node("a"), node("b")],
            "blocked",
            false,
        );
        results[0].status = OperationStatus::Succeeded;
        results[1].status = OperationStatus::Failed;
        job.results = results;
        assert_eq!(job.completed(), 1);
        assert_eq!(job.results.len(), 2);
    }
    #[test]
    fn quorum_unknown_blocks_reboot_but_not_scheduling_only_drain() {
        let impact = EtcdQuorumImpact::unavailable("no authoritative data");
        assert!(matches!(
            evaluate_operation_safety(OperationKind::Reboot, &impact),
            SafetyStatus::Unknown
        ));
        assert!(matches!(
            evaluate_operation_safety(OperationKind::Drain, &impact),
            SafetyStatus::Safe
        ));
    }
    #[test]
    fn global_single_flight_and_cancellation_belong_to_worker_not_observer() {
        static FIXTURE_BUSY: AtomicBool = AtomicBool::new(false);
        let busy = &FIXTURE_BUSY;
        let guard = acquire_mutation(busy).unwrap();
        let cancellation = guard.cancellation();
        assert!(busy.load(Ordering::Acquire));
        assert!(acquire_mutation(busy).is_err());
        cancellation.store(true, Ordering::Release);
        assert!(guard.cancelled());
        drop(cancellation);
        assert!(busy.load(Ordering::Acquire)); // a UI observer cannot unlock the worker
        let guard = std::thread::spawn(move || {
            assert!(guard.cancelled());
            guard
        })
        .join()
        .unwrap();
        assert!(busy.load(Ordering::Acquire));
        drop(guard);
        assert!(!busy.load(Ordering::Acquire));
        let next = acquire_mutation(busy).unwrap();
        assert!(!next.cancelled());
        drop(next);
    }

    #[test]
    fn cancellation_predicate_owns_flag_without_owning_mutation_lease() {
        static FIXTURE_BUSY: AtomicBool = AtomicBool::new(false);
        let guard = acquire_mutation(&FIXTURE_BUSY).unwrap();
        let cancellation = guard.cancellation();
        let cancellation_for_context = cancellation.clone();
        let cancelled = move || cancellation_for_context.load(Ordering::Acquire);
        let predicate: &talos_pilot_core::operations::CancellationPredicate = &cancelled;
        assert!(!predicate());
        guard.cancel();
        assert!(predicate());
        drop(cancellation);
        assert!(FIXTURE_BUSY.load(Ordering::Acquire));
        assert!(acquire_mutation(&FIXTURE_BUSY).is_err());
        drop(guard);
        assert!(!FIXTURE_BUSY.load(Ordering::Acquire));
        assert!(predicate());
    }

    #[tokio::test]
    async fn dropping_ui_handle_does_not_abort_compensation_or_unlock_mutation() {
        static FIXTURE_BUSY: AtomicBool = AtomicBool::new(false);
        let guard = acquire_mutation(&FIXTURE_BUSY).unwrap();
        let cancellation = guard.cancellation();
        let compensating = Arc::new(tokio::sync::Notify::new());
        let finish = Arc::new(tokio::sync::Notify::new());
        let done = Arc::new(tokio::sync::Notify::new());
        let (worker_compensating, worker_finish, worker_done) =
            (compensating.clone(), finish.clone(), done.clone());
        let handle = tokio::spawn(async move {
            while !guard.cancelled() {
                tokio::task::yield_now().await;
            }
            worker_compensating.notify_one();
            worker_finish.notified().await; // mock compensating uncordon + audit flush
            assert!(FIXTURE_BUSY.load(Ordering::Acquire));
            drop(guard);
            worker_done.notify_one();
        });
        cancellation.store(true, Ordering::Release);
        drop(handle); // unmount cannot own an abort-on-drop mutation handle
        tokio::time::timeout(Duration::from_secs(5), compensating.notified())
            .await
            .unwrap();
        assert!(FIXTURE_BUSY.load(Ordering::Acquire));
        assert!(acquire_mutation(&FIXTURE_BUSY).is_err());
        finish.notify_one();
        tokio::time::timeout(Duration::from_secs(5), done.notified())
            .await
            .unwrap();
        assert!(!FIXTURE_BUSY.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn fresh_preflight_failure_after_success_keeps_partial_results_and_skips_tail() {
        let targets = vec![node("a"), node("b"), NodeTarget::new("c", "10.0.0.3")];
        let cancellation = AtomicBool::new(false);
        let mut attempted = Vec::new();
        let results = ordered_sequence(
            OperationKind::Reboot,
            &targets,
            true,
            Duration::ZERO,
            &|| cancellation.load(Ordering::Acquire),
            |index, target| {
                attempted.push(target.name.clone());
                let mut result = not_started(
                    OperationKind::Reboot,
                    &[target],
                    "Fresh etcd source unavailable; no mutation made",
                    false,
                )
                .remove(0);
                if index == 0 {
                    result.status = OperationStatus::Succeeded;
                    result.message = "Completed reboot".to_owned();
                }
                std::future::ready(result)
            },
        )
        .await;
        assert_eq!(attempted, ["a", "b"]);
        assert_eq!(
            results.iter().map(|r| r.status).collect::<Vec<_>>(),
            [
                OperationStatus::Succeeded,
                OperationStatus::Blocked,
                OperationStatus::NotStarted
            ]
        );
        assert_eq!(
            results.iter().map(|r| &r.target).collect::<Vec<_>>(),
            targets.iter().collect::<Vec<_>>()
        );
        let retained = panic_outcomes(OperationKind::Reboot, &targets, vec![results[0].clone()]);
        assert_eq!(retained[0].status, OperationStatus::Succeeded);
        assert!(matches!(
            retained[1].scheduling,
            SchedulingState::Unknown(_)
        ));
        assert_eq!(retained[2].status, OperationStatus::NotStarted);
    }

    #[tokio::test]
    async fn continue_on_failure_is_ordered_but_cancellation_always_stops() {
        let targets = vec![node("a"), node("b"), NodeTarget::new("c", "10.0.0.3")];
        let cancellation = AtomicBool::new(false);
        let mut attempted = Vec::new();
        let results = ordered_sequence(
            OperationKind::Drain,
            &targets,
            false,
            Duration::ZERO,
            &|| cancellation.load(Ordering::Acquire),
            |index, target| {
                attempted.push(target.name.clone());
                let mut result =
                    not_started(OperationKind::Drain, &[target], "Fixture failure", false)
                        .remove(0);
                if index == 1 {
                    result.status = OperationStatus::Cancelled;
                }
                std::future::ready(result)
            },
        )
        .await;
        assert_eq!(attempted, ["a", "b"]);
        assert_eq!(results[0].status, OperationStatus::Blocked);
        assert_eq!(results[1].status, OperationStatus::Cancelled);
        assert_eq!(results[2].status, OperationStatus::NotStarted);
    }

    #[tokio::test]
    async fn cancellation_during_delay_prevents_next_preflight() {
        let targets = vec![node("a"), node("b")];
        let cancellation = AtomicBool::new(false);
        let mut attempts = 0;
        let results = ordered_sequence(
            OperationKind::Drain,
            &targets,
            true,
            Duration::from_secs(30),
            &|| cancellation.load(Ordering::Acquire),
            |_, target| {
                attempts += 1;
                let mut result =
                    not_started(OperationKind::Drain, &[target], "Completed", false).remove(0);
                result.status = OperationStatus::Succeeded;
                cancellation.store(true, Ordering::Release);
                std::future::ready(result)
            },
        )
        .await;
        assert_eq!(attempts, 1);
        assert_eq!(results[0].status, OperationStatus::Succeeded);
        assert_eq!(results[1].status, OperationStatus::NotStarted);
    }

    #[test]
    fn incomplete_etcd_fixture_never_classifies_an_unmatched_target_as_safe() {
        use talos_pilot_core::{indicators::QuorumState, inspection::InspectionTarget};
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
        let impact = etcd_impact(&snapshot, &node("a"));
        assert!(matches!(impact, EtcdQuorumImpact::Unavailable { .. }));
        assert!(matches!(
            evaluate_operation_safety(OperationKind::Reboot, &impact),
            SafetyStatus::Unknown
        ));
    }

    #[test]
    fn kubernetes_precheck_requires_exact_name_and_separate_ip() {
        use k8s_openapi::api::core::v1::{NodeAddress, NodeStatus};
        let target = node("a");
        let mut fixture = Node::default();
        fixture.metadata.name = Some(target.name.clone());
        fixture.status = Some(NodeStatus {
            addresses: Some(vec![NodeAddress {
                type_: "InternalIP".to_owned(),
                address: target.address.clone(),
            }]),
            ..Default::default()
        });
        assert!(node_matches_target(&fixture, &target));
        fixture.metadata.name = Some("10.0.0.1".to_owned());
        assert!(!node_matches_target(&fixture, &target));
        fixture.metadata.name = Some(target.name.clone());
        fixture.status.as_mut().unwrap().addresses.as_mut().unwrap()[0].address =
            "10.0.0.99".to_owned();
        assert!(!node_matches_target(&fixture, &target));
    }
    #[test]
    fn audit_path_is_context_local_and_cannot_escape_directory() {
        assert_eq!(
            audit_path("../../prod").file_name().unwrap(),
            "operations-.._.._prod.yaml"
        );
        assert_eq!(
            audit_path("").file_name().unwrap(),
            "operations-default.yaml"
        );
    }
}
