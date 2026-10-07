use super::*;
use crate::indicators::QuorumState;

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
    assert!(default_audit_path("../../prod").ends_with(".freshkube/operations-.._.._prod.yaml"));
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
            let mut result = not_started(OperationKind::Drain, &[target], "done", false).remove(0);
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
