use chrono::{TimeZone, Utc};
use serde_json::json;

use super::{ISSUE_LIMIT, Part, derive};

fn summary(
    nodes: serde_json::Value,
    pods: serde_json::Value,
    events: Part<Vec<k8s_openapi::api::core::v1::Event>>,
    claims: serde_json::Value,
) -> super::KubernetesSummary {
    derive(
        Part::Loaded("v1.32.3".into()),
        Part::Loaded(serde_json::from_value(nodes).unwrap()),
        Part::Loaded(serde_json::from_value(pods).unwrap()),
        Part::Loaded(Vec::new()),
        Part::Loaded(Vec::new()),
        Part::Loaded(Vec::new()),
        Part::Loaded(Vec::new()),
        Part::Loaded(serde_json::from_value(claims).unwrap()),
        Part::Loaded(Vec::new()),
        events,
        Utc.timestamp_opt(1_790_000_000, 0).unwrap(),
    )
}

#[test]
fn ready_since_pod_issues_and_not_ready_counts_come_from_status() {
    let data = summary(
        json!([{"metadata":{"name":"worker"},"status":{"conditions":[{"type":"Ready","status":"False","reason":"KubeletNotReady","lastTransitionTime":"2026-09-01T00:00:00Z"}]}}]),
        json!([
            {"metadata":{"name":"pending","namespace":"batch"},"spec":{"containers":[],"nodeName":"worker"},"status":{"phase":"Pending"}},
            {"metadata":{"name":"crash","namespace":"payments"},"spec":{"containers":[],"nodeName":"worker"},"status":{"phase":"Running","containerStatuses":[{"name":"app","image":"app","imageID":"app","ready":false,"restartCount":14,"state":{"waiting":{"reason":"CrashLoopBackOff"}}}]}}
        ]),
        Part::Loaded(Vec::new()),
        json!([]),
    );
    let nodes = data.nodes.loaded().unwrap();
    assert!(!nodes[0].is_ready());
    assert_eq!(nodes[0].ready().unwrap().reason, "KubeletNotReady");
    assert!(nodes[0].ready().unwrap().since.is_some());
    assert_eq!(nodes[0].pods, 2);
    let pods = data.pods.loaded().unwrap();
    assert_eq!(pods.total, 2);
    assert_eq!(pods.on_not_ready, 2);
    assert!(
        pods.issues
            .iter()
            .any(|pod| pod.issue.label() == "CrashLoopBackOff" && pod.restarts == 14)
    );
    assert!(pods.issues.iter().any(|pod| pod.issue.label() == "Pending"));
}

#[test]
fn recent_warnings_exclude_old_normal_and_future_and_explain_pending_claim() {
    let now = Utc.timestamp_opt(1_790_000_000, 0).unwrap();
    let event = |name: &str, seconds: i64, kind: &str| json!({"metadata":{"name":name},"involvedObject":{"kind":"PersistentVolumeClaim","namespace":"batch","name":"report-data"},"type":kind,"reason":"ProvisioningFailed","message":"StorageClass fast not found","lastTimestamp":(now + chrono::Duration::seconds(seconds)).to_rfc3339()});
    let events = serde_json::from_value(json!([
        event("old", -3601, "Warning"),
        event("new", -10, "Warning"),
        event("normal", -10, "Normal"),
        event("future", 10, "Warning")
    ]))
    .unwrap();
    let data = summary(
        json!([]),
        json!([]),
        Part::Loaded(events),
        json!([{"metadata":{"namespace":"batch","name":"report-data","creationTimestamp":"2026-09-01T00:00:00Z"},"status":{"phase":"Pending"}},{"metadata":{"name":"bound"},"status":{"phase":"Bound"}}]),
    );
    assert_eq!(data.events.loaded().unwrap().total, 1);
    let claims = data.claims.loaded().unwrap();
    assert_eq!(claims.bound, 1);
    assert_eq!(claims.pending_count, 1);
    assert_eq!(claims.pending[0].reason, "StorageClass fast not found");
    assert!(claims.pending[0].since.is_some());
}

#[test]
fn refused_or_failed_events_leave_other_parts_loaded() {
    for events in [
        Part::Refused("Can't list events: forbidden".into()),
        Part::Failed("Can't list events: timed out".into()),
    ] {
        let data = summary(json!([]), json!([]), events, json!([]));
        assert!(data.events.error().is_some());
        assert!(data.nodes.loaded().is_some());
        assert!(data.pods.loaded().is_some());
        assert!(data.workloads.snapshot().is_some());
    }
}

#[test]
fn issue_caps_keep_newest_and_preserve_counts() {
    let pods: Vec<_> = (0..250).map(|ix| json!({"metadata":{"name":format!("pod-{ix}"),"creationTimestamp":format!("2026-09-01T00:{:02}:{:02}Z",ix/60,ix%60)},"status":{"phase":"Pending"}})).collect();
    let claims: Vec<_> = pods
        .iter()
        .map(|pod| json!({"metadata":pod["metadata"],"status":{"phase":"Pending"}}))
        .collect();
    let data = summary(
        json!([]),
        json!(pods),
        Part::Loaded(Vec::new()),
        json!(claims),
    );
    assert_eq!(data.pods.loaded().unwrap().total, 250);
    assert_eq!(data.pods.loaded().unwrap().issues.len(), ISSUE_LIMIT);
    assert_eq!(data.pods.loaded().unwrap().issues[0].name, "pod-249");
    assert_eq!(data.claims.loaded().unwrap().pending_count, 250);
    assert_eq!(data.claims.loaded().unwrap().pending.len(), ISSUE_LIMIT);
}

#[tokio::test]
async fn collector_uses_cache_and_warning_selector_and_isolates_forbidden() {
    use http::{Request, Response};
    use kube::{Client, client::Body};
    use std::sync::{Arc, Mutex};
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let service = tower::service_fn(move |request: Request<Body>| {
        let uri = request.uri().to_string();
        log.lock().unwrap().push(uri.clone());
        let (status, value) = if uri == "/version" {
            (
                200,
                json!({"major":"1","minor":"32","gitVersion":"v1.32.3"}),
            )
        } else if uri.starts_with("/api/v1/events?") {
            (
                403,
                json!({"kind":"Status","apiVersion":"v1","status":"Failure","reason":"Forbidden","code":403,"message":"events forbidden"}),
            )
        } else {
            (200, json!({"metadata":{"resourceVersion":"1"},"items":[]}))
        };
        async move {
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(status)
                    .body(Body::from(value.to_string().into_bytes()))
                    .unwrap(),
            )
        }
    });
    let data = super::collect_kubernetes_summary(Client::new(service, "default")).await;
    assert!(matches!(data.events, Part::Refused(_)));
    assert!(data.pods.loaded().is_some());
    assert!(data.nodes.loaded().is_some());
    assert!(data.workloads.snapshot().is_some());
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 10);
    for uri in seen.iter().filter(|uri| uri.as_str() != "/version") {
        assert!(uri.contains("resourceVersion=0"), "{uri}");
    }
    assert!(
        seen.iter()
            .any(|uri| uri.contains("fieldSelector=type%3DWarning"))
    );
}

#[test]
fn warning_cap_does_not_remove_a_pending_claims_reason() {
    let now = Utc.timestamp_opt(1_790_000_000, 0).unwrap();
    let mut events: Vec<_> = (0..250).map(|ix| json!({"metadata":{"name":format!("warning-{ix}")},"involvedObject":{"kind":"Pod","name":format!("pod-{ix}")},"type":"Warning","lastTimestamp":(now - chrono::Duration::seconds(1)).to_rfc3339()})).collect();
    events.push(json!({"metadata":{"name":"claim"},"involvedObject":{"kind":"PersistentVolumeClaim","namespace":"batch","name":"report-data"},"type":"Warning","message":"StorageClass fast not found","lastTimestamp":(now - chrono::Duration::seconds(60)).to_rfc3339()}));
    let data = summary(
        json!([]),
        json!([]),
        Part::Loaded(serde_json::from_value(json!(events)).unwrap()),
        json!([{"metadata":{"namespace":"batch","name":"report-data"},"status":{"phase":"Pending"}}]),
    );
    assert_eq!(data.events.loaded().unwrap().newest.len(), ISSUE_LIMIT);
    assert_eq!(data.events.loaded().unwrap().total, 251);
    assert_eq!(
        data.claims.loaded().unwrap().pending[0].reason,
        "StorageClass fast not found"
    );
}
