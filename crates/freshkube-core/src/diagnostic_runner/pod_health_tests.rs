//! The same synthetic API evidence through collection, retention and details.
use super::*;
use crate::{
    kubernetes_summary::{Session, SessionIdentity},
    resources::{Severity, builtin, object_from_yaml},
    workloads::HealthState,
};
use kube::runtime::watcher::Event;
use serde_json::{Value, json};

fn container(state: Value) -> Value {
    json!({"name":"app", "image":"example", "imageID":"example",
        "ready":false, "restartCount":0, "state":state})
}

fn waiting(reason: &str) -> Value {
    container(json!({"waiting":{"reason":reason}}))
}

async fn check(
    name: &str,
    status: Value,
    summary_issue: Option<(HealthState, &str)>,
    detail_issue: Option<(Severity, &str)>,
    diagnostic_issue: Option<(&str, &str)>,
) {
    let object = json!({"apiVersion":"v1", "kind":"Pod",
        "metadata":{"name":name,"namespace":"example","uid":name,"resourceVersion":"1"},
        "spec":{"containers":[{"name":"app","image":"example"}],
            "initContainers":[{"name":"migrate","image":"example"}]},
        "status":status});
    let pod: Pod = serde_json::from_value(object.clone()).unwrap();
    let session = Session::new(SessionIdentity::new("example", 1));
    let now = chrono::DateTime::from_timestamp(1_790_000_000, 0).unwrap();
    session.apply::<Pod>(0, Event::Init, now).unwrap();
    session.apply(0, Event::InitApply(pod), now).unwrap();
    session.apply::<Pod>(0, Event::InitDone, now).unwrap();
    let publication = session.derive(now);
    let summary = publication.summary.workloads.snapshot().unwrap();
    let document = object_from_yaml(
        &builtin("pods").unwrap(),
        &serde_yaml::to_string(&object).unwrap(),
    )
    .unwrap();
    let diagnosis = document.overview.pod_status.unwrap().diagnose();
    let response = serde_json::to_vec(&json!({"apiVersion":"v1", "kind":"PodList",
        "metadata":{"resourceVersion":"1"},"items":[object]}))
    .unwrap();
    // In-process transport only: no socket, configuration or cluster access.
    let body = String::from_utf8(response).unwrap();
    let (client, recorder) = crate::fake_api::client(move |request| {
        (request.path == "/api/v1/pods" && request.method == http::Method::GET)
            .then(|| crate::fake_api::json(200, &body))
    });
    let health = collect_pod_health(&client).await.unwrap();
    // Exactly one pod list. Anything else the fake answers 500 and records here.
    recorder.assert_lines(&["GET /api/v1/pods"]);
    let serialized = serde_json::to_value(&health).unwrap();
    let observed_diagnostics: Vec<_> = ["crashing", "image_pull_errors", "not_ready"]
        .into_iter()
        .flat_map(|category| {
            serialized[category]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |pod| (category, pod["state"].as_str().unwrap()))
        })
        .collect();
    let observed_summary: Vec<_> = publication
        .summary
        .pods
        .loaded()
        .unwrap()
        .issues
        .iter()
        .map(|pod| (pod.issue.severity(), pod.issue.label()))
        .collect();
    let observed_detail = diagnosis.as_ref().map(|d| (d.severity, d.state.as_str()));
    let counts = (
        summary.total_pods_healthy,
        summary.total_pods_degraded,
        summary.total_pods_failing,
    );
    println!(
        "{name}: summary={counts:?} {observed_summary:?}; detail={observed_detail:?}; diagnostics={observed_diagnostics:?}"
    );
    assert_eq!(
        observed_summary,
        summary_issue.into_iter().collect::<Vec<_>>(),
        "{name}: summary"
    );
    let expected_counts = match summary_issue.map(|issue| issue.0) {
        Some(HealthState::Failing) => (0, 0, 1),
        Some(HealthState::Degraded | HealthState::Pending) => (0, 1, 0),
        _ if matches!(status["phase"].as_str(), Some("Running" | "Succeeded")) => (1, 0, 0),
        _ => (0, 0, 0),
    };
    assert_eq!(counts, expected_counts, "{name}: summary counts");
    let collected_issues: Vec<_> = summary
        .namespaces
        .iter()
        .flat_map(|n| &n.problem_pods)
        .map(|pod| (pod.issue.severity(), pod.issue.label()))
        .collect();
    assert_eq!(collected_issues, observed_summary);
    assert_eq!(observed_detail, detail_issue, "{name}: detail");
    assert_eq!(
        observed_diagnostics,
        diagnostic_issue.into_iter().collect::<Vec<_>>(),
        "{name}: Diagnostics"
    );
    assert_eq!(health.total_pods, 1);
    assert_eq!(health.has_issues(), diagnostic_issue.is_some());
    if let Some((category, _)) = diagnostic_issue {
        assert_eq!(
            serialized[category][0]["restart_count"].as_u64().unwrap(),
            u64::from(diagnosis.as_ref().unwrap().restarts),
            "{name}: selected container restart count"
        );
    }
    let context = DiagnosticContext {
        target: DiagnosticTarget::new("node-a", "192.0.2.1", "controlplane", "example"),
        platform: SourceState::Available("metal".into()),
        cpu_count: SourceState::Available(4),
        kubernetes_access: SourceState::unavailable("example", "not used"),
    };
    let checks = build_kubernetes_checks(
        &context,
        &KubernetesSnapshot {
            pod_health: SourceState::Available(health),
        },
    );
    let result = checks
        .iter()
        .find(|check| check.id == "pod_health")
        .unwrap();
    assert_eq!(
        result.status,
        if diagnostic_issue.is_some() {
            CheckStatus::Warn
        } else {
            CheckStatus::Pass
        }
    );
    if let Some((category, reason)) = diagnostic_issue {
        assert!(result.details.as_deref().unwrap().contains(name));
        if category == "not_ready" {
            assert_eq!(result.message, "1 not ready");
            assert!(result.details.as_deref().unwrap().contains(reason));
        }
    }
    assert!(publication.pod_bytes < 4 * 1024);
}

#[tokio::test]
async fn running_not_ready() {
    check(
        "running-not-ready",
        json!({"phase":"Running",
        "conditions":[{"type":"Ready","status":"False","message":"readiness probe failed"}],
        "containerStatuses":[container(json!({"running":{}}))]}),
        Some((HealthState::Degraded, "Not ready")),
        Some((Severity::NotReady, "Not ready")),
        Some(("not_ready", "Not ready")),
    )
    .await;
}

#[tokio::test]
async fn init_crash_loop() {
    let mut init = waiting("CrashLoopBackOff");
    init["name"] = json!("migrate");
    init["restartCount"] = json!(6);
    check(
        "init-crash-loop",
        json!({"phase":"Pending",
        "conditions":[{"type":"Ready","status":"False","message":"init container failed"}],
        "initContainerStatuses":[init]}),
        Some((HealthState::Failing, "CrashLoopBackOff")),
        Some((Severity::Failing, "CrashLoopBackOff")),
        Some(("crashing", "CrashLoopBackOff")),
    )
    .await;
}

#[tokio::test]
async fn ready_running() {
    check(
        "ready-running",
        json!({"phase":"Running","conditions":[{"type":"Ready","status":"True"}],
        "containerStatuses":[container(json!({"running":{}}))]}),
        None,
        None,
        None,
    )
    .await;
}

#[tokio::test]
async fn completed() {
    check("completed", json!({"phase":"Succeeded",
        "containerStatuses":[container(json!({"terminated":{"exitCode":0,"reason":"Completed"}}))]}), None, None, None).await;
}

#[tokio::test]
async fn ordinary_pending() {
    check(
        "ordinary-pending",
        json!({"phase":"Pending"}),
        Some((HealthState::Pending, "Pending")),
        Some((Severity::Waiting, "Pending")),
        None,
    )
    .await;
}

#[tokio::test]
async fn missing_evidence() {
    check("missing-status", Value::Null, None, None, None).await;
    check(
        "missing-ready",
        json!({"phase":"Running",
        "containerStatuses":[container(json!({"running":{}}))]}),
        None,
        None,
        None,
    )
    .await;
}

#[tokio::test]
async fn init_image_and_other_waiting_errors() {
    for reason in [
        "ImagePullBackOff",
        "ErrImagePull",
        "CreateContainerConfigError",
        "InvalidImageName",
    ] {
        check(
            reason,
            json!({"phase":"Pending","initContainerStatuses":[waiting(reason)]}),
            Some((HealthState::Failing, reason)),
            Some((Severity::Failing, reason)),
            matches!(reason, "ImagePullBackOff" | "ErrImagePull")
                .then_some(("image_pull_errors", reason)),
        )
        .await;
    }
}

#[tokio::test]
async fn app_waiting_errors() {
    for reason in [
        "CrashLoopBackOff",
        "ImagePullBackOff",
        "ErrImagePull",
        "CreateContainerConfigError",
        "InvalidImageName",
    ] {
        let category = match reason {
            "CrashLoopBackOff" => Some("crashing"),
            "ImagePullBackOff" | "ErrImagePull" => Some("image_pull_errors"),
            _ => None,
        };
        check(
            reason,
            json!({"phase":"Running","containerStatuses":[waiting(reason)]}),
            Some((HealthState::Failing, reason)),
            Some((Severity::Failing, reason)),
            category.map(|c| (c, reason)),
        )
        .await;
    }
}

#[tokio::test]
async fn nonzero_init_termination_and_failed_app_termination() {
    for key in ["initContainerStatuses", "containerStatuses"] {
        for reason in ["Error", "OOMKilled", "CustomExit", ""] {
            let phase = if key == "initContainerStatuses" {
                "Pending"
            } else {
                "Failed"
            };
            let state = if reason.is_empty() { "Error" } else { reason };
            let mut status = json!({"phase":phase});
            status[key] = json!([container(
                json!({"terminated":{"exitCode":1,"reason":reason}})
            )]);
            let diagnostic = matches!(state, "Error" | "OOMKilled").then_some(("crashing", state));
            check(
                "nonzero-termination",
                status,
                Some((HealthState::Failing, state)),
                Some((Severity::Failing, state)),
                diagnostic,
            )
            .await;
        }
    }
}

#[tokio::test]
async fn non_failed_app_termination_is_not_a_fault() {
    for phase in ["Running", "Succeeded"] {
        for reason in ["Error", "OOMKilled"] {
            check("app-termination", json!({"phase":phase,"conditions":[{"type":"Ready","status":"True"}],
                "containerStatuses":[container(json!({"terminated":{"exitCode":1,"reason":reason}}))]}), None, None, None).await;
        }
    }
}

#[tokio::test]
async fn failed_reason_and_missing_reason_fallback() {
    check(
        "evicted",
        json!({"phase":"Failed","reason":"Evicted"}),
        Some((HealthState::Failing, "Evicted")),
        Some((Severity::Failing, "Evicted")),
        None,
    )
    .await;
    check(
        "failed-no-reason",
        json!({"phase":"Failed"}),
        Some((HealthState::Failing, "Error")),
        None,
        None,
    )
    .await;
}

#[tokio::test]
async fn scheduler_and_pending_waiting_reasons() {
    check("unschedulable", json!({"phase":"Pending","conditions":[{"type":"PodScheduled","status":"False","reason":"Unschedulable"}]}),
        Some((HealthState::Pending,"Unschedulable")), Some((Severity::Waiting,"Unschedulable")), None).await;
    for reason in ["ContainerCreating", "PodInitializing"] {
        check(
            "pending-waiting",
            json!({"phase":"Pending","containerStatuses":[waiting(reason)]}),
            Some((HealthState::Pending, reason)),
            Some((Severity::Waiting, reason)),
            None,
        )
        .await;
    }
}

#[tokio::test]
async fn faults_take_precedence_and_each_pod_counts_once() {
    check(
        "init-first",
        json!({"phase":"Pending","initContainerStatuses":[waiting("CrashLoopBackOff")],
        "containerStatuses":[waiting("ImagePullBackOff"),waiting("CrashLoopBackOff")],
        "conditions":[{"type":"PodScheduled","status":"False"}]}),
        Some((HealthState::Failing, "CrashLoopBackOff")),
        Some((Severity::Failing, "CrashLoopBackOff")),
        Some(("crashing", "CrashLoopBackOff")),
    )
    .await;
    check("pod-reason-first", json!({"phase":"Failed","reason":"Evicted","initContainerStatuses":[waiting("CrashLoopBackOff")]}),
        Some((HealthState::Failing,"Evicted")), Some((Severity::Failing,"Evicted")), None).await;
}

#[tokio::test]
async fn ready_unknown_is_explicit_evidence() {
    check(
        "ready-unknown",
        json!({"phase":"Running","conditions":[{"type":"Ready","status":"Unknown"}]}),
        Some((HealthState::Degraded, "Not ready")),
        Some((Severity::NotReady, "Not ready")),
        Some(("not_ready", "Not ready")),
    )
    .await;
}

#[tokio::test]
async fn restart_history_is_only_a_summary_advisory() {
    for phase in ["Running", "Succeeded"] {
        let mut app = container(json!({"running":{}}));
        app["restartCount"] = json!(crate::constants::HIGH_RESTART_THRESHOLD);
        app["lastState"] = json!({"terminated":{"exitCode":137,"reason":"OOMKilled"}});
        check("restart-history", json!({"phase":phase,"conditions":[{"type":"Ready","status":"True"}],"containerStatuses":[app.clone()]}),
            Some((HealthState::Degraded,"High Restarts")), None, None).await;
        check("decision-before-history", json!({"phase":"Running","conditions":[{"type":"Ready","status":"False"}],"containerStatuses":[app]}),
            Some((HealthState::Degraded,"Not ready")), Some((Severity::NotReady,"Not ready")), Some(("not_ready","Not ready"))).await;
    }
}
