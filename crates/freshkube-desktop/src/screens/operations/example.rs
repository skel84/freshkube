use super::*;

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

pub(super) fn example_world(source: &ScreenSource, targets: &[NodeTarget]) -> ExampleWorld {
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

pub(super) fn example_preview(source: &ScreenSource, key: PreviewKey) -> Preview {
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

pub(super) async fn run_example(
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
