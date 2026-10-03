use super::*;
use http::{Method, Request, Response};
use http_body_util::BodyExt;
use kube::client::Body;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    convert::Infallible,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::Notify;

#[derive(Default)]
struct PendingPatch {
    started: Notify,
    release: Notify,
    dropped: AtomicBool,
}

struct PatchGuard(Arc<PendingPatch>, bool);

impl Drop for PatchGuard {
    fn drop(&mut self) {
        if !self.1 {
            self.0.dropped.store(true, Ordering::SeqCst);
        }
    }
}

#[derive(Default)]
struct ApiState {
    requests: Vec<(Method, String, Value)>,
    nodes: BTreeMap<String, Value>,
    pods: Vec<Value>,
    fail_patch: BTreeSet<String>,
    fail_eviction: bool,
    pending_patch: Option<Arc<PendingPatch>>,
}

fn fake_client(state: Arc<Mutex<ApiState>>) -> Client {
    let service = tower::service_fn(move |request: Request<Body>| {
        let state = state.clone();
        async move {
            let method = request.method().clone();
            let uri = request.uri().to_string();
            let path = request.uri().path().to_owned();
            let bytes = request.into_body().collect().await.unwrap().to_bytes();
            let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let (status, answer, pending) = {
                let mut state = state.lock().unwrap();
                state.requests.push((method.clone(), uri, body.clone()));
                let pending = if method == Method::PATCH {
                    state.pending_patch.take()
                } else {
                    None
                };
                let (status, answer) = if let Some(name) = path.strip_prefix("/api/v1/nodes/") {
                    if method == Method::PATCH && state.fail_patch.contains(name) {
                        (
                            403,
                            json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"Forbidden","message":"patch refused","code":403}),
                        )
                    } else {
                        let node = state.nodes.get_mut(name).expect("explicit test node");
                        if method == Method::PATCH {
                            node["spec"]["unschedulable"] = body["spec"]["unschedulable"].clone();
                        } else {
                            assert_eq!(method, Method::GET);
                        }
                        (200, node.clone())
                    }
                } else if path == "/api/v1/pods" && method == Method::GET {
                    (
                        200,
                        json!({"apiVersion":"v1","kind":"PodList","metadata":{},"items":state.pods}),
                    )
                } else if path.ends_with("/eviction") && method == Method::POST {
                    if state.fail_eviction {
                        (
                            429,
                            json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"TooManyRequests","message":"Cannot evict pod as it would violate the pod disruption budget.","code":429}),
                        )
                    } else {
                        (
                            201,
                            json!({"apiVersion":"v1","kind":"Status","status":"Success"}),
                        )
                    }
                } else if method == Method::DELETE && path.contains("/pods/") {
                    (
                        200,
                        json!({"apiVersion":"v1","kind":"Status","status":"Success"}),
                    )
                } else {
                    panic!("unexpected fake API request: {method} {path}");
                };
                (status, answer, pending)
            };
            if let Some(pending) = pending {
                let mut guard = PatchGuard(pending.clone(), false);
                pending.started.notify_one();
                pending.release.notified().await;
                guard.1 = true;
            }
            Ok::<_, Infallible>(
                Response::builder()
                    .status(status)
                    .body(Body::from(answer.to_string().into_bytes()))
                    .unwrap(),
            )
        }
    });
    Client::new(service, "default")
}

#[derive(Clone, Default)]
struct Scenario {
    /// One sample for the whole selection, then a fresh sample per selected node.
    impacts: Vec<Vec<EtcdQuorumImpact>>,
    cancel_at_check: Option<usize>,
    stall_at_check: Option<usize>,
    mismatch_at_check: Option<usize>,
    panic_on_cordon: Option<String>,
    cancel_on_node_done: bool,
    connect_error: bool,
    connect_panic: bool,
}

struct Harness {
    client: Client,
    api: Arc<Mutex<ApiState>>,
    cancelled: Arc<AtomicBool>,
    checks: Arc<Mutex<Vec<Vec<String>>>>,
    events: Arc<Mutex<Vec<SelectionEvent>>>,
    audit: AuditLog,
    directory: PathBuf,
}

impl Harness {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "freshkube-selection-{}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let api = Arc::new(Mutex::new(ApiState::default()));
        for (ix, name) in ["first", "second", "third"].into_iter().enumerate() {
            api.lock().unwrap().nodes.insert(name.into(), json!({
                "apiVersion":"v1", "kind":"Node", "metadata":{"name":name},
                "spec":{"unschedulable":false},
                "status":{"addresses":[{"type":"InternalIP","address":format!("10.0.0.{}",ix+1)}],
                    "conditions":[{"type":"Ready","status":"True"}]}
            }));
        }
        Self {
            client: fake_client(api.clone()),
            api,
            cancelled: Arc::new(AtomicBool::new(false)),
            checks: Arc::new(Mutex::new(Vec::new())),
            events: Arc::new(Mutex::new(Vec::new())),
            audit: AuditLog::with_actor(directory.join("audit.yaml"), "test", "test"),
            directory,
        }
    }

    fn request(&self, operation: OperationKind, count: usize) -> SelectionRequest {
        SelectionRequest::new(
            operation,
            ["first", "second", "third"]
                .into_iter()
                .take(count)
                .enumerate()
                .map(|(ix, name)| NodeTarget::new(name, format!("10.0.0.{}", ix + 1)))
                .collect(),
            crate::inspection::InspectionTarget::new("first", "10.0.0.1"),
            OperationConfirmation::confirmed(),
        )
    }

    async fn run(&self, request: SelectionRequest, scenario: Scenario) -> SelectionOutcome {
        let client = self.client.clone();
        let connection = {
            let scenario = scenario.clone();
            async move {
                assert!(!scenario.connect_panic, "connection worker panicked");
                if scenario.connect_error {
                    Err("access identity could not be revalidated".into())
                } else {
                    Ok(client)
                }
            }
        };
        let cancelled = self.cancelled.clone();
        let on_event = {
            let events = self.events.clone();
            let cancelled = cancelled.clone();
            let scenario = scenario.clone();
            move |event: SelectionEvent| {
                if let SelectionEvent::Progress(OperationsEvent::Operation(progress)) = &event
                    && progress.step == OperationStep::Cordon
                    && scenario.panic_on_cordon.as_ref() == Some(&progress.target.name)
                {
                    panic!("operation worker panicked before patch");
                }
                if matches!(event, SelectionEvent::NodeDone(_)) && scenario.cancel_on_node_done {
                    cancelled.store(true, Ordering::SeqCst);
                }
                events.lock().unwrap().push(event);
            }
        };
        let preflight = {
            let checks = self.checks.clone();
            let api = self.api.clone();
            move |client: Client, targets: Vec<NodeTarget>| {
                let (checks, api, cancelled, scenario) = (
                    checks.clone(),
                    api.clone(),
                    cancelled.clone(),
                    scenario.clone(),
                );
                async move {
                    let call = {
                        let mut checks = checks.lock().unwrap();
                        let call = checks.len();
                        checks.push(targets.iter().map(|target| target.name.clone()).collect());
                        call
                    };
                    if scenario.stall_at_check == Some(call) {
                        std::future::pending::<()>().await;
                    }
                    if scenario.mismatch_at_check == Some(call) {
                        api.lock().unwrap().nodes.get_mut(&targets[0].name).unwrap()["status"]["addresses"]
                            [0]["address"] = json!("10.99.99.99");
                    }
                    // Exercise the same identity, readiness, scheduling and pod reads as live.
                    let mut nodes = preflight_kubernetes(&client, &targets).await?;
                    if let Some(impacts) = scenario.impacts.get(call) {
                        assert_eq!(impacts.len(), nodes.len());
                        for (node, impact) in nodes.iter_mut().zip(impacts) {
                            node.impact = impact.clone();
                        }
                    }
                    if scenario.cancel_at_check == Some(call) {
                        cancelled.store(true, Ordering::SeqCst);
                    }
                    Ok(nodes)
                }
            }
        };
        let cancelled = self.cancelled.clone();
        run_with_preflight(
            request,
            None,
            connection,
            &self.audit,
            Arc::new(move || cancelled.load(Ordering::SeqCst)),
            on_event,
            preflight,
        )
        .await
    }

    fn mutations(&self) -> Vec<(Method, String)> {
        self.api
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(method, _, _)| method != Method::GET)
            .map(|(method, path, _)| (method.clone(), path.clone()))
            .collect()
    }

    fn outcomes(&self) -> Vec<AuditEntry> {
        self.audit
            .read_entries()
            .unwrap()
            .into_iter()
            .filter(|entry| entry.message.starts_with("Frontend sequence outcome "))
            .collect()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn statuses(outcome: &SelectionOutcome) -> Vec<OperationStatus> {
    outcome.results.iter().map(|result| result.status).collect()
}

fn safe() -> EtcdQuorumImpact {
    EtcdQuorumImpact::known(true, true, 3, 3)
}

#[tokio::test]
async fn confirmation_and_cancellation_gate_connection_and_every_mutation() {
    for cancelled in [false, true] {
        let harness = Harness::new();
        let mut request = harness.request(OperationKind::Cordon, 2);
        if cancelled {
            harness.cancelled.store(true, Ordering::SeqCst);
        } else {
            request.confirmation = OperationConfirmation::rejected();
        }
        let outcome = harness
            .run(
                request,
                Scenario {
                    connect_panic: true,
                    ..Scenario::default()
                },
            )
            .await;
        assert!(harness.checks.lock().unwrap().is_empty());
        assert!(harness.api.lock().unwrap().requests.is_empty());
        let status = if cancelled {
            OperationStatus::Cancelled
        } else {
            OperationStatus::NotConfirmed
        };
        assert_eq!(statuses(&outcome), [status, status]);
        assert_eq!(harness.outcomes().len(), 2);
    }
}

#[tokio::test]
async fn connection_errors_and_panics_preserve_explicit_audited_outcomes() {
    for panic in [false, true] {
        let harness = Harness::new();
        let outcome = harness
            .run(
                harness.request(OperationKind::Cordon, 2),
                Scenario {
                    connect_error: !panic,
                    connect_panic: panic,
                    ..Scenario::default()
                },
            )
            .await;
        assert_eq!(outcome.results.len(), 2);
        assert!(
            outcome
                .results
                .iter()
                .all(|result| !result.status.is_success())
        );
        assert!(outcome.note.is_some());
        assert!(harness.api.lock().unwrap().requests.is_empty());
        assert_eq!(harness.outcomes().len(), 2);
    }
}

#[tokio::test]
async fn whole_selection_blocks_before_first_mutation_when_a_later_node_is_unsafe_or_unknown() {
    for impact in [
        EtcdQuorumImpact::known(true, true, 2, 3),
        EtcdQuorumImpact::unavailable("no status"),
    ] {
        let harness = Harness::new();
        let outcome = harness
            .run(
                harness.request(OperationKind::Reboot, 2),
                Scenario {
                    impacts: vec![vec![safe(), impact]],
                    ..Scenario::default()
                },
            )
            .await;
        assert_eq!(
            statuses(&outcome),
            [OperationStatus::Blocked, OperationStatus::Blocked]
        );
        assert!(outcome.note.as_ref().unwrap().contains("second"));
        assert_eq!(*harness.checks.lock().unwrap(), [vec!["first", "second"]]);
        assert!(harness.mutations().is_empty());
        assert_eq!(harness.outcomes().len(), 2);
    }
}

#[tokio::test]
async fn fresh_per_node_etcd_impact_replaces_the_safe_initial_sample() {
    let harness = Harness::new();
    let outcome = harness
        .run(
            harness.request(OperationKind::Reboot, 2),
            Scenario {
                impacts: vec![
                    vec![safe(), safe()],
                    vec![EtcdQuorumImpact::known(true, true, 2, 3)],
                ],
                ..Scenario::default()
            },
        )
        .await;
    assert_eq!(
        statuses(&outcome),
        [OperationStatus::Blocked, OperationStatus::NotStarted]
    );
    assert_eq!(
        *harness.checks.lock().unwrap(),
        [vec!["first", "second"], vec!["first"]]
    );
    assert!(harness.mutations().is_empty());
}

#[tokio::test]
async fn selection_order_is_preserved_and_every_node_is_rechecked() {
    let harness = Harness::new();
    let mut request = harness.request(OperationKind::Cordon, 2);
    request.targets.reverse();
    let outcome = harness.run(request, Scenario::default()).await;
    assert_eq!(
        statuses(&outcome),
        [OperationStatus::Succeeded, OperationStatus::Succeeded]
    );
    assert_eq!(
        *harness.checks.lock().unwrap(),
        [vec!["second", "first"], vec!["second"], vec!["first"]]
    );
    assert_eq!(
        harness.mutations(),
        [
            (Method::PATCH, "/api/v1/nodes/second".into()),
            (Method::PATCH, "/api/v1/nodes/first".into())
        ]
    );
    let entries = harness.audit.read_entries().unwrap();
    assert_eq!(harness.outcomes().len(), 2);
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.phase == AuditPhase::Success
                && entry.step == OperationStep::Complete
                && !entry.message.starts_with("Frontend sequence outcome "))
            .count(),
        2
    );
}

#[tokio::test]
async fn a_single_node_uses_the_whole_selection_sample_once() {
    let harness = Harness::new();
    let outcome = harness
        .run(
            harness.request(OperationKind::Cordon, 1),
            Scenario::default(),
        )
        .await;
    assert_eq!(statuses(&outcome), [OperationStatus::Succeeded]);
    assert_eq!(*harness.checks.lock().unwrap(), [vec!["first"]]);
}

#[tokio::test]
async fn identity_change_before_a_later_node_preserves_completed_results() {
    let harness = Harness::new();
    let outcome = harness
        .run(
            harness.request(OperationKind::Cordon, 3),
            Scenario {
                mismatch_at_check: Some(2),
                ..Scenario::default()
            },
        )
        .await;
    assert_eq!(
        statuses(&outcome),
        [
            OperationStatus::Succeeded,
            OperationStatus::Blocked,
            OperationStatus::NotStarted
        ]
    );
    assert!(outcome.results[1].message.contains("identity mismatch"));
    assert_eq!(harness.mutations().len(), 1);
    assert_eq!(harness.outcomes().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn both_whole_selection_and_per_node_prechecks_have_a_45_second_deadline() {
    for call in [0, 1] {
        let harness = Harness::new();
        let start = tokio::time::Instant::now();
        let outcome = harness
            .run(
                harness.request(OperationKind::Cordon, 2),
                Scenario {
                    stall_at_check: Some(call),
                    ..Scenario::default()
                },
            )
            .await;
        assert_eq!(start.elapsed(), Duration::from_secs(45));
        assert!(outcome.results[0].message.contains("timed out"));
        assert!(harness.mutations().is_empty());
        assert_eq!(harness.outcomes().len(), 2);
    }
}

#[tokio::test]
async fn cancellation_during_either_precheck_prevents_mutation() {
    for call in [0, 1] {
        let harness = Harness::new();
        let outcome = harness
            .run(
                harness.request(OperationKind::Cordon, 2),
                Scenario {
                    cancel_at_check: Some(call),
                    ..Scenario::default()
                },
            )
            .await;
        assert_eq!(
            statuses(&outcome),
            [OperationStatus::Cancelled, OperationStatus::NotStarted]
        );
        assert!(harness.mutations().is_empty());
        assert_eq!(harness.outcomes().len(), 2);
    }
}

#[tokio::test]
async fn cancellation_never_drops_a_submitted_mutation() {
    let harness = Harness::new();
    let pending = Arc::new(PendingPatch::default());
    harness.api.lock().unwrap().pending_patch = Some(pending.clone());
    let run = harness.run(
        harness.request(OperationKind::Cordon, 2),
        Scenario::default(),
    );
    let cancel = async {
        pending.started.notified().await;
        harness.cancelled.store(true, Ordering::SeqCst);
        tokio::task::yield_now().await;
        assert!(!pending.dropped.load(Ordering::SeqCst));
        pending.release.notify_one();
    };
    let (outcome, ()) = tokio::join!(run, cancel);
    assert_eq!(
        statuses(&outcome),
        [OperationStatus::Succeeded, OperationStatus::Cancelled]
    );
    assert!(!pending.dropped.load(Ordering::SeqCst));
    assert_eq!(harness.mutations().len(), 1);
}

#[tokio::test]
async fn cancellation_after_node_completion_stops_the_next_node() {
    let harness = Harness::new();
    let outcome = harness
        .run(
            harness.request(OperationKind::Cordon, 3),
            Scenario {
                cancel_on_node_done: true,
                ..Scenario::default()
            },
        )
        .await;
    assert_eq!(
        statuses(&outcome),
        [
            OperationStatus::Succeeded,
            OperationStatus::Cancelled,
            OperationStatus::NotStarted
        ]
    );
    assert_eq!(harness.mutations().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn cancellation_during_the_delay_keeps_later_nodes_unstarted() {
    let harness = Harness::new();
    let mut request = harness.request(OperationKind::Cordon, 2);
    request.delay_between_nodes = Duration::from_secs(30);
    let cancel = async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        harness.cancelled.store(true, Ordering::SeqCst);
    };
    let (outcome, ()) = tokio::join!(harness.run(request, Scenario::default()), cancel);
    assert_eq!(
        statuses(&outcome),
        [OperationStatus::Succeeded, OperationStatus::NotStarted]
    );
    assert_eq!(harness.mutations().len(), 1);
}

#[tokio::test]
async fn stop_on_failure_controls_continuation_and_audits_unstarted_nodes() {
    for stop in [true, false] {
        let harness = Harness::new();
        harness
            .api
            .lock()
            .unwrap()
            .fail_patch
            .insert("first".into());
        let mut request = harness.request(OperationKind::Cordon, 2);
        request.stop_on_failure = stop;
        let outcome = harness.run(request, Scenario::default()).await;
        assert_eq!(
            statuses(&outcome),
            [
                OperationStatus::Failed,
                if stop {
                    OperationStatus::NotStarted
                } else {
                    OperationStatus::Succeeded
                }
            ]
        );
        assert_eq!(harness.mutations().len(), if stop { 1 } else { 2 });
        assert_eq!(harness.outcomes().len(), 2);
    }
}

#[tokio::test]
async fn panic_keeps_completed_nodes_and_marks_the_inflight_outcome_unknown() {
    let harness = Harness::new();
    let outcome = harness
        .run(
            harness.request(OperationKind::Cordon, 3),
            Scenario {
                panic_on_cordon: Some("second".into()),
                ..Scenario::default()
            },
        )
        .await;
    assert_eq!(
        statuses(&outcome),
        [
            OperationStatus::Succeeded,
            OperationStatus::Failed,
            OperationStatus::NotStarted
        ]
    );
    assert!(matches!(
        outcome.results[1].scheduling,
        SchedulingState::Unknown(_)
    ));
    assert!(outcome.note.as_ref().unwrap().contains("panicked"));
    assert_eq!(harness.mutations().len(), 1);
    assert_eq!(harness.outcomes().len(), 3);
}

#[tokio::test]
async fn drain_options_reach_the_real_eviction_path() {
    for force in [false, true] {
        let harness = Harness::new();
        harness.api.lock().unwrap().pods.push(json!({
            "apiVersion":"v1", "kind":"Pod", "metadata":{"name":"unmanaged","namespace":"default"},
            "spec":{"nodeName":"first","containers":[{"name":"app","image":"test"}],"volumes":[{"name":"scratch","emptyDir":{}}]}
        }));
        harness.api.lock().unwrap().fail_eviction = true;
        let mut request = harness.request(OperationKind::Drain, 1);
        request.drain_options.per_pod_timeout_secs = 1;
        request.drain_options.force_delete_unmanaged = force;
        let outcome = harness.run(request, Scenario::default()).await;
        assert_eq!(
            outcome.results[0].status,
            if force {
                OperationStatus::Succeeded
            } else {
                OperationStatus::Failed
            }
        );
        assert_eq!(
            harness
                .mutations()
                .iter()
                .filter(|(method, _)| *method == Method::DELETE)
                .count(),
            usize::from(force)
        );
        let result = &outcome.results[0];
        assert_eq!(
            result.drain.as_ref().unwrap().force_deleted_pods.len(),
            usize::from(force)
        );
        if !force {
            assert_eq!(result.scheduling, SchedulingState::Schedulable);
        }
    }
}

#[tokio::test]
async fn audit_failures_remain_visible_on_successful_results() {
    let mut harness = Harness::new();
    harness.audit = AuditLog::with_actor(&harness.directory, "test", "test");
    let outcome = harness
        .run(
            harness.request(OperationKind::Cordon, 1),
            Scenario::default(),
        )
        .await;
    assert_eq!(statuses(&outcome), [OperationStatus::Succeeded]);
    assert!(outcome.results[0].audit_error.is_some());
}
