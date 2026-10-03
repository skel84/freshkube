//! Confirmed selection policy. The caller owns access revalidation, task lifetime and UI.

use super::*;
use crate::inspection::InspectionTarget;
use futures::FutureExt;
use std::{
    future::Future,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
};

/// A bounded read deadline, never applied to a submitted mutation.
pub const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(45);

/// The ordered targets and options frozen by the caller's confirmation.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SelectionRequest {
    pub operation: OperationKind,
    pub targets: Vec<NodeTarget>,
    pub endpoint: InspectionTarget,
    pub confirmation: OperationConfirmation,
    pub drain_options: DrainOptions,
    pub stop_on_failure: bool,
    pub delay_between_nodes: Duration,
}

impl SelectionRequest {
    pub fn new(
        operation: OperationKind,
        targets: Vec<NodeTarget>,
        endpoint: InspectionTarget,
        confirmation: OperationConfirmation,
    ) -> Self {
        Self {
            operation,
            targets,
            endpoint,
            confirmation,
            drain_options: DrainOptions::default(),
            stop_on_failure: true,
            delay_between_nodes: Duration::ZERO,
        }
    }
}

/// The caller chooses how to deliver progress and completed node outcomes.
#[derive(Clone, Debug)]
pub enum SelectionEvent {
    Progress(OperationsEvent),
    NodeDone(NodeOperationResult),
}

/// One final result per selected target, including failures before execution and panics.
#[derive(Debug)]
#[non_exhaustive]
pub struct SelectionOutcome {
    pub results: Vec<NodeOperationResult>,
    pub note: Option<String>,
}

/// Execute a confirmed selection with fresh Kubernetes and etcd prechecks.
///
/// `connect` must revalidate the caller's selected access identity. It is polled only after
/// confirmation/cancellation gates, inside panic recovery. Progress is synchronous and must
/// not block. Dropping a frontend receiver must not cancel this future: the caller retains
/// its operation slot until completion and uses only `is_cancelled` to request cancellation.
/// A submitted mutation is always awaited, without a timeout or abort on cancellation.
pub async fn run_selection(
    request: SelectionRequest,
    talos: TalosClient,
    connect: impl Future<Output = Result<Client, String>>,
    audit: &AuditLog,
    is_cancelled: Arc<CancellationPredicate>,
    on_event: impl Fn(SelectionEvent) + Clone + Send + Sync + 'static,
) -> SelectionOutcome {
    let endpoint = request.endpoint.clone();
    let client = talos.clone();
    run_with_preflight(
        request,
        Some(talos),
        connect,
        audit,
        is_cancelled,
        on_event,
        move |kubernetes, targets| {
            let (client, endpoint) = (client.clone(), endpoint.clone());
            async move { preflight_nodes(&kubernetes, client, endpoint, &targets).await }
        },
    )
    .await
}

/// The same live loop is tested with a fake Kubernetes client and controlled etcd evidence.
/// Only the preflight I/O is replaceable; execution always calls `run_node_operation`.
async fn run_with_preflight<Check, CheckFuture>(
    request: SelectionRequest,
    talos: Option<TalosClient>,
    connect: impl Future<Output = Result<Client, String>>,
    audit: &AuditLog,
    is_cancelled: Arc<CancellationPredicate>,
    on_event: impl Fn(SelectionEvent) + Clone + Send + Sync + 'static,
    preflight: Check,
) -> SelectionOutcome
where
    Check: Fn(Client, Vec<NodeTarget>) -> CheckFuture + Clone,
    CheckFuture: Future<Output = Result<Vec<NodePreflight>, String>>,
{
    let operation = request.operation;
    let completed = Arc::new(Mutex::new(Vec::new()));
    let body = async {
        if !request.confirmation.is_confirmed() {
            return Err("The operation was not confirmed by the frontend".to_owned());
        }
        if is_cancelled() {
            return Err("Cancelled before the latest prechecks; no mutation was made".to_owned());
        }
        let kubernetes = connect.await?;
        let check = |targets: Vec<NodeTarget>| {
            let future = preflight(kubernetes.clone(), targets);
            async move {
                tokio::time::timeout(PREFLIGHT_TIMEOUT, future)
                    .await
                    .map_err(|_| "Latest prechecks timed out; no mutation was made".to_owned())?
            }
        };
        let latest = check(request.targets.clone()).await?;
        let impacts: Vec<_> = latest.iter().map(|node| node.impact.clone()).collect();
        if let Some(reason) = selection_blocked_reason(operation, &request.targets, &impacts) {
            return Err(reason);
        }
        let total = request.targets.len();
        let results = ordered_sequence(
            operation,
            &request.targets,
            request.stop_on_failure,
            request.delay_between_nodes,
            &*is_cancelled,
            |index, target| {
                let (on_event, completed, is_cancelled, kubernetes, talos) = (
                    on_event.clone(),
                    completed.clone(),
                    is_cancelled.clone(),
                    kubernetes.clone(),
                    talos.clone(),
                );
                let first = latest.first().cloned().filter(|_| total == 1);
                let drain_options = request.drain_options.clone();
                let check = &check;
                async move {
                    let fresh = match first {
                        Some(node) => Ok(node),
                        None => {
                            on_event(SelectionEvent::Progress(OperationsEvent::Rolling(
                                RollingProgressEvent {
                                    operation,
                                    current_node_index: index,
                                    total_nodes: total,
                                    target: Some(target.clone()),
                                    phase: AuditPhase::Progress,
                                    step: OperationStep::Preflight,
                                    message: format!(
                                        "Fresh etcd and Kubernetes checks for {} ({})",
                                        target.name, target.address
                                    ),
                                },
                            )));
                            check(vec![target.clone()]).await.and_then(|mut nodes| {
                                (!nodes.is_empty())
                                    .then(|| nodes.remove(0))
                                    .ok_or_else(|| "No precheck result".to_owned())
                            })
                        }
                    };
                    let result = match fresh {
                        Ok(node) => {
                            let context = OperationContext {
                                kubernetes: &kubernetes,
                                talos: talos.as_ref(),
                                audit,
                                is_cancelled: &*is_cancelled,
                            };
                            let mut node_request = NodeOperationRequest::new(operation, target);
                            node_request.etcd_impact = node.impact;
                            node_request.drain_options = drain_options;
                            let progress = on_event.clone();
                            run_node_operation(
                                node_request,
                                &context,
                                request.confirmation,
                                &mut move |event| progress(SelectionEvent::Progress(event)),
                            )
                            .await
                        }
                        Err(error) => {
                            not_started(operation, &[target], &error, is_cancelled()).remove(0)
                        }
                    };
                    completed.lock().unwrap().push(result.clone());
                    on_event(SelectionEvent::NodeDone(result.clone()));
                    result
                }
            },
        )
        .await;
        Ok::<_, String>(results)
    };
    let (mut results, note) = match AssertUnwindSafe(body).catch_unwind().await {
        Ok(Ok(results)) => (results, None),
        Ok(Err(error)) => {
            let mut results = not_started(operation, &request.targets, &error, is_cancelled());
            if !request.confirmation.is_confirmed() {
                for result in &mut results {
                    result.status = OperationStatus::NotConfirmed;
                }
            }
            (results, Some(error))
        }
        Err(_) => (
            panic_outcomes(
                operation,
                &request.targets,
                completed.lock().unwrap().clone(),
            ),
            Some("The operation worker panicked; inspect the nodes and the audit log.".to_owned()),
        ),
    };
    for result in &mut results {
        audit_result(audit, &audit.cluster, result);
    }
    SelectionOutcome { results, note }
}

/// A destructive selection must pass as a whole before any target can start.
pub fn selection_blocked_reason(
    operation: OperationKind,
    targets: &[NodeTarget],
    impacts: &[EtcdQuorumImpact],
) -> Option<String> {
    if !operation.is_destructive() {
        return None;
    }
    targets.iter().zip(impacts).find_map(|(target, impact)| {
        match evaluate_operation_safety(operation, impact) {
            SafetyStatus::Unsafe(reason) => Some(format!(
                "Whole-selection preflight blocked {}: {reason}",
                target.name
            )),
            SafetyStatus::Unknown => Some(format!(
                "Whole-selection preflight blocked {}: etcd quorum impact is unknown",
                target.name
            )),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests;
