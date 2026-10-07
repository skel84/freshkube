use chrono::{TimeZone, Utc};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Event as KubeEvent, Pod};
use kube::runtime::watcher::Event;
use serde_json::json;

use super::*;
use crate::resources::FailureKind;

fn now() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_790_000_000, 0).unwrap()
}
fn session() -> Session {
    Session::new(SessionIdentity::new("test", 1))
}
fn pod(name: &str, uid: &str, version: &str) -> Pod {
    serde_json::from_value(json!({"metadata":{"name":name,"namespace":"ns","uid":uid,"resourceVersion":version},
        "spec":{"containers":[{"name":"app","image":"test","env":[{"name":"SENSITIVE","value":"must not be retained"}]}],"nodeName":"node"},
        "status":{"phase":"Running"}})).unwrap()
}
fn sync<K: SummaryResource>(session: &Session, objects: Vec<K>) {
    let generation = session.generation();
    session.apply::<K>(generation, Event::Init, now()).unwrap();
    for object in objects {
        session
            .apply(generation, Event::InitApply(object), now())
            .unwrap();
    }
    session
        .apply::<K>(generation, Event::InitDone, now())
        .unwrap();
}

#[test]
fn node_requests_reduce_only_assigned_active_pods_with_allocatable_and_freshness() {
    use crate::resources::Amounts;
    use k8s_openapi::api::core::v1::Node;
    let session = session();
    let node: Node = serde_json::from_value(
        json!({"metadata":{"name":"node","uid":"node-uid","resourceVersion":"1"},
        "status":{"capacity":{"cpu":"8"},"allocatable":{"cpu":"7500m","memory":"30Gi"}}}),
    )
    .unwrap();
    sync(&session, vec![node]);
    assert_eq!(
        session.derive(now()).summary.nodes.loaded().unwrap()[0].requests,
        Amounts::default()
    );
    let mut pods = Vec::new();
    for (ix, phase) in ["Running", "Pending", "Succeeded", "Failed"]
        .into_iter()
        .enumerate()
    {
        let mut object = pod(&format!("pod-{ix}"), &format!("uid-{ix}"), "1");
        object.status.as_mut().unwrap().phase = Some(phase.into());
        object.spec.as_mut().unwrap().containers[0].resources = Some(
            serde_json::from_value(json!({"requests":{"cpu":"250m","memory":"64Mi"}})).unwrap(),
        );
        let retained = object.retain();
        assert!(retained.bytes < 4 * 1024);
        assert!(retained.validate(Source::Pods).is_ok());
        pods.push(object);
    }
    let mut unassigned = pod("unassigned", "unassigned", "1");
    unassigned.spec.as_mut().unwrap().node_name = None;
    pods.push(unassigned);
    sync(&session, pods);
    let snapshot = session.derive(now());
    let node = &snapshot.summary.nodes.loaded().unwrap()[0];
    assert_eq!(node.pods, 4);
    assert_eq!(
        node.requests,
        Amounts {
            cpu_millis: Some(500.),
            memory_bytes: Some(128. * 1024. * 1024.)
        }
    );
    assert_eq!(node.allocatable["cpu"].0, "7500m");
    assert_eq!(node.capacity["cpu"].0, "8");
    assert!(node.pods_current);
    session.fail(
        0,
        Source::Pods,
        ObservationFailure::Read(FailureKind::Forbidden),
    );
    let stale = session.derive(now());
    let stale_node = &stale.summary.nodes.loaded().unwrap()[0];
    assert_eq!(stale_node.requests, node.requests);
    assert!(!stale_node.pods_current);
    assert!(stale_node.pods_observed);
    sync::<Pod>(&session, vec![]);
    assert_eq!(
        session.derive(now()).summary.nodes.loaded().unwrap()[0].requests,
        super::requests::ZERO
    );
}

#[test]
fn initial_and_replacement_pages_do_not_establish_absence() {
    let session = session();
    session.apply::<Pod>(0, Event::Init, now()).unwrap();
    session
        .apply(0, Event::InitApply(pod("first", "one", "1")), now())
        .unwrap();
    assert!(session.derive(now()).summary.pods.loaded().is_none());
    session.apply::<Pod>(0, Event::InitDone, now()).unwrap();
    assert_eq!(
        session.derive(now()).summary.pods.loaded().unwrap().total,
        1
    );
    session.apply::<Pod>(0, Event::Init, now()).unwrap();
    let staged = session.derive(now());
    assert!(!staged.summary.pods.is_current());
    assert_eq!(staged.summary.pods.loaded().unwrap().total, 1);
    session.fail(
        0,
        Source::Pods,
        ObservationFailure::Read(FailureKind::Timeout),
    );
    assert_eq!(
        session.derive(now()).summary.pods.loaded().unwrap().total,
        1
    );
    sync::<Pod>(&session, vec![]);
    let empty = session.derive(now());
    assert!(empty.summary.pods.is_current());
    assert_eq!(empty.summary.pods.loaded().unwrap().total, 0);
}

#[test]
fn independent_failure_retains_evidence_and_allows_other_successes_and_recovery() {
    let session = session();
    sync::<KubeEvent>(&session, vec![]);
    sync::<Pod>(&session, vec![]);
    let deployment: Deployment = serde_json::from_value(json!({
        "metadata":{"name":"app","namespace":"ns","uid":"controller","resourceVersion":"1"},
        "spec":{"replicas":1,"selector":{},"template":{}}, "status":{"readyReplicas":1,"availableReplicas":1}
    })).unwrap();
    sync(&session, vec![deployment]);
    session.fail(
        0,
        Source::Deployments,
        ObservationFailure::Read(FailureKind::Forbidden),
    );
    session.fail(
        0,
        Source::Events,
        ObservationFailure::Read(FailureKind::Forbidden),
    );
    session
        .apply(0, Event::Apply(pod("new", "new", "2")), now())
        .unwrap();
    let changed = session.derive(now());
    assert_eq!(changed.summary.pods.loaded().unwrap().total, 1);
    assert!(matches!(changed.summary.events, Part::Refused(_)));
    assert!(changed.summary.events.loaded().is_some());
    assert_eq!(
        changed
            .summary
            .workloads
            .snapshot()
            .unwrap()
            .total_deployments,
        1
    );
    let refused = &changed.summary.observations[&Source::Events];
    assert_eq!(
        refused.failure(),
        Some(&ObservationFailure::Read(FailureKind::Forbidden))
    );
    assert_eq!(refused.last_success(), Some(now()));
    sync::<KubeEvent>(&session, vec![]);
    let recovered = session.derive(now());
    assert!(recovered.summary.events.is_current());
    assert_eq!(recovered.summary.pods, changed.summary.pods);
    assert_eq!(
        recovered.summary.observations[&Source::Deployments].status(),
        ReadStatus::Refused
    );
}

#[test]
fn old_incarnation_delete_does_not_erase_recreated_pod() {
    let session = session();
    sync(&session, vec![pod("same", "old", "1")]);
    session
        .apply(0, Event::Apply(pod("same", "new", "2")), now())
        .unwrap();
    session
        .apply(0, Event::Delete(pod("same", "old", "3")), now())
        .unwrap();
    assert_eq!(session.derive(now()).pod_count, 1);
    session
        .apply(0, Event::Delete(pod("same", "new", "4")), now())
        .unwrap();
    assert_eq!(session.derive(now()).pod_count, 0);
}

#[test]
fn exact_keys_share_one_publication_and_incompatible_keys_never_share() {
    let session = session();
    let key = SubscriptionKey::summary(session.identity().clone(), Source::Nodes);
    let first = session.subscribe(key.clone()).unwrap();
    let second = session.subscribe(key.clone()).unwrap();
    for different in [
        key.clone().with_scope(Scope::Namespace("ns".into())),
        key.clone().with_labels("app=test"),
        key.clone().with_fields("metadata.name=test"),
        key.clone().with_version("v2"),
        key.with_representation("Table"),
        SubscriptionKey::summary(SessionIdentity::new("test", 2), Source::Nodes),
        SubscriptionKey::summary(SessionIdentity::new("another", 1), Source::Nodes),
    ] {
        assert!(session.subscribe(different).is_none());
    }
    session.publish(session.derive(now()));
    assert!(std::sync::Arc::ptr_eq(
        &first.latest().unwrap(),
        &second.latest().unwrap()
    ));
}

#[test]
fn stale_generations_and_sessions_cannot_publish_or_mutate_new_observations() {
    let session = session();
    sync(&session, vec![pod("old", "old", "1")]);
    let old = session.derive(now());
    let newer = session.derive(now());
    assert!(session.publish(newer));
    assert!(!session.publish(old.clone()));
    let generation = session.relist();
    assert!(!session.publish(old));
    session
        .apply(0, Event::Apply(pod("late", "late", "2")), now())
        .unwrap();
    session.fail(
        0,
        Source::Pods,
        ObservationFailure::Read(FailureKind::Forbidden),
    );
    assert_eq!(session.derive(now()).pod_count, 1);
    assert_eq!(
        session.derive(now()).summary.observations[&Source::Pods].status(),
        ReadStatus::Resyncing
    );
    assert_eq!(generation, 1);
    let other = Session::new(SessionIdentity::new("test", 2));
    assert!(!session.publish(other.derive(now())));
}

#[test]
fn compact_pods_drop_sensitive_payload_and_enforce_retention_limits() {
    let mut object = pod("example", "uid", "1");
    object.metadata.managed_fields =
        Some(serde_json::from_value(json!([{"fieldsV1":{"huge": "x".repeat(100_000)}}])).unwrap());
    object.metadata.annotations = Some([("secret".into(), "private".repeat(10000))].into());
    let compact = object.retain();
    assert!(compact.bytes < 1024, "{} bytes", compact.bytes);
    assert!(!format!("{compact:?}").contains("must not be retained"));
    assert!(!format!("{compact:?}").contains("private"));
    let mut oversized = pod("oversized", "oversized", "1");
    oversized.status.as_mut().unwrap().phase = Some("x".repeat(4096));
    assert_eq!(
        oversized.retain().validate(Source::Pods),
        Err(ObservationFailure::Capacity)
    );
    let session = Session::with_limits(
        SessionIdentity::new("bounded", 1),
        Limits {
            objects_per_kind: 1,
            bytes_per_generation: 4096,
        },
    );
    sync(&session, vec![object]);
    let result = session.apply(0, Event::Apply(pod("second", "second", "2")), now());
    assert_eq!(result, Err(ObservationFailure::Capacity));
    assert_eq!(session.derive(now()).pod_count, 1);
    session.apply::<Pod>(0, Event::Init, now()).unwrap();
    session
        .apply(0, Event::InitApply(pod("second", "second", "2")), now())
        .unwrap();
    assert_eq!(
        session.apply(0, Event::InitApply(pod("third", "third", "3")), now()),
        Err(ObservationFailure::Capacity)
    );
    let failed = session.derive(now());
    assert_eq!(failed.staging_bytes, 0);
    assert_eq!(failed.pod_count, 1);
    assert_eq!(
        failed.summary.observations[&Source::Pods].status(),
        ReadStatus::Limited
    );
}

#[test]
fn warning_expiry_is_local_and_never_needs_a_refresh() {
    let session = session();
    let event: KubeEvent = serde_json::from_value(json!({"metadata":{"name":"warning","namespace":"ns","uid":"e","resourceVersion":"1"},
        "involvedObject":{"kind":"Pod","name":"pod","namespace":"ns"},"type":"Warning","reason":"BackOff",
        "lastTimestamp":(now()-chrono::Duration::minutes(59)).to_rfc3339()})).unwrap();
    sync(&session, vec![event]);
    assert_eq!(
        session.derive(now()).summary.events.loaded().unwrap().total,
        1
    );
    assert_eq!(
        session.next_warning_change(now()),
        Some(std::time::Duration::from_millis(60_001))
    );
    assert_eq!(
        session
            .derive(now() + chrono::Duration::seconds(61))
            .summary
            .events
            .loaded()
            .unwrap()
            .total,
        0
    );
}

#[test]
fn capped_issue_rows_preserve_health_of_every_namespace() {
    let session = session();
    let pods = (0..300)
        .map(|ix| {
            let mut object = pod(&format!("pod-{ix:03}"), &ix.to_string(), "1");
            object.metadata.namespace = Some(format!("ns-{ix:03}"));
            object.status.as_mut().unwrap().phase = Some("Pending".into());
            object
        })
        .collect();
    sync::<Pod>(&session, pods);
    let publication = session.derive(now());
    let summary = &publication.summary;
    assert_eq!(summary.pods.loaded().unwrap().issues.len(), ISSUE_LIMIT);
    assert_eq!(summary.pods.loaded().unwrap().by_namespace.len(), 300);
    assert_eq!(summary.pods.loaded().unwrap().by_namespace["ns-299"], 1);
    let snapshot = summary.workloads.snapshot().unwrap();
    assert_eq!(snapshot.total_pods_degraded, 300);
    assert_eq!(snapshot.namespaces.len(), 300);
    assert!(
        snapshot
            .namespaces
            .iter()
            .all(|ns| ns.health == crate::workloads::HealthState::Pending)
    );
    let healthy = pod("healthy", "healthy", "2");
    session
        .apply(0, Event::Apply(healthy.clone()), now())
        .unwrap();
    assert_eq!(
        session
            .derive(now())
            .summary
            .pods
            .loaded()
            .unwrap()
            .by_namespace["ns"],
        1
    );
    session.apply(0, Event::Delete(healthy), now()).unwrap();
    assert!(
        !session
            .derive(now())
            .summary
            .pods
            .loaded()
            .unwrap()
            .by_namespace
            .contains_key("ns")
    );
}

#[test]
fn pods_never_listed_leave_node_pods_and_requests_unknown() {
    use k8s_openapi::api::core::v1::Node;
    let session = session();
    let node: Node = serde_json::from_value(
        json!({"metadata":{"name":"node","uid":"node-uid","resourceVersion":"1"}}),
    )
    .unwrap();
    sync(&session, vec![node]);
    let publication = session.derive(now());
    let node = &publication.summary.nodes.loaded().unwrap()[0];
    assert!(!node.pods_current);
    assert!(!node.pods_observed);
    assert_eq!(node.requests, crate::resources::Amounts::default());
}

#[test]
fn pods_failed_without_data_leave_node_pods_and_requests_unknown() {
    use k8s_openapi::api::core::v1::Node;
    let session = session();
    let node: Node = serde_json::from_value(
        json!({"metadata":{"name":"node","uid":"node-uid","resourceVersion":"1"}}),
    )
    .unwrap();
    sync(&session, vec![node]);
    session.fail(
        0,
        Source::Pods,
        ObservationFailure::Read(FailureKind::Forbidden),
    );
    let publication = session.derive(now());
    assert!(matches!(publication.summary.pods, Part::Refused(_)));
    assert!(publication.summary.pods.loaded().is_none());
    let node = &publication.summary.nodes.loaded().unwrap()[0];
    assert_eq!(node.pods, 0);
    assert!(!node.pods_current);
    assert!(!node.pods_observed);
    assert_eq!(node.requests, crate::resources::Amounts::default());
}

/// A part's states short of current: never listed, failed with nothing listed
/// (refused or failed), and failed after a list, which it keeps as stale.
fn assert_part_unavailable_and_stale<T: Clone + PartialEq + std::fmt::Debug>(
    source: Source,
    list: impl Fn(&Session),
    part: impl Fn(&KubernetesSummary) -> &Part<T>,
    listed: T,
) {
    let never_listed = session();
    let publication = never_listed.derive(now());
    let waiting = part(&publication.summary);
    assert!(
        matches!(waiting, Part::Failed(_)),
        "{source:?}: {waiting:?}"
    );
    assert!(waiting.loaded().is_none());
    assert_eq!(waiting.error(), Some("Waiting for the initial list"));
    assert!(!publication.summary.observations[&source].is_stale());

    for (failure, refused) in [
        (ObservationFailure::Read(FailureKind::Forbidden), true),
        (ObservationFailure::Read(FailureKind::Timeout), false),
    ] {
        let failed = session();
        failed.fail(0, source, failure.clone());
        let publication = failed.derive(now());
        let unavailable = part(&publication.summary);
        assert_eq!(
            matches!(unavailable, Part::Refused(_)),
            refused,
            "{source:?}: {unavailable:?}"
        );
        assert!(!unavailable.is_current());
        assert!(unavailable.loaded().is_none());
        assert_eq!(unavailable.error(), Some(failure.message()));
        assert!(!publication.summary.observations[&source].is_stale());
    }

    let session = session();
    list(&session);
    let publication = session.derive(now());
    assert_eq!(part(&publication.summary), &Part::Loaded(listed.clone()));
    session.fail(0, source, ObservationFailure::Read(FailureKind::Timeout));
    let publication = session.derive(now());
    let stale = part(&publication.summary);
    assert!(matches!(stale, Part::Failed(_)), "{source:?}: {stale:?}");
    assert!(!stale.is_current());
    assert_eq!(stale.loaded(), Some(&listed));
    assert_eq!(stale.error(), Some("The Kubernetes read timed out"));
    let observation = &publication.summary.observations[&source];
    assert!(observation.is_stale());
    assert_eq!(observation.last_success(), Some(now()));
}

#[test]
fn available_volumes_are_unavailable_until_listed_and_stale_after_a_failure() {
    use k8s_openapi::api::core::v1::PersistentVolume;
    assert_part_unavailable_and_stale(
        Source::Volumes,
        |session| {
            sync::<PersistentVolume>(
                session,
                objects(json!([
                    {"metadata":{"name":"pv-free"},"status":{"phase":"Available"}},
                    {"metadata":{"name":"pv-used"},"status":{"phase":"Bound"}}
                ])),
            )
        },
        |summary| &summary.available_volumes,
        1,
    );
}

#[test]
fn namespaces_are_unavailable_until_listed_and_stale_after_a_failure() {
    use k8s_openapi::api::core::v1::Namespace;
    assert_part_unavailable_and_stale(
        Source::Namespaces,
        |session| {
            sync::<Namespace>(
                session,
                objects(json!([
                    {"metadata":{"name":"batch"}},
                    {"metadata":{"name":"payments"}}
                ])),
            )
        },
        |summary| &summary.namespaces,
        2,
    );
}

#[test]
fn version_is_unavailable_until_read_and_stale_after_a_failure() {
    assert_part_unavailable_and_stale(
        Source::Version,
        |session| session.version(0, "v1.34.1".into(), now()),
        |summary| &summary.version,
        "v1.34.1".to_string(),
    );
}

#[test]
fn a_pending_claims_reason_is_last_known_while_events_are_stale() {
    use k8s_openapi::api::core::v1::PersistentVolumeClaim;
    let session = session();
    let claim: PersistentVolumeClaim = serde_json::from_value(json!({
        "metadata":{"name":"data","namespace":"ns","uid":"claim-uid","resourceVersion":"1"},
        "spec":{},"status":{"phase":"Pending"}
    }))
    .unwrap();
    sync(&session, vec![claim]);
    let reason = |session: &Session| {
        session
            .derive(now())
            .summary
            .claims
            .loaded()
            .unwrap()
            .pending[0]
            .reason
            .clone()
    };
    assert_eq!(reason(&session), "");
    let warning: KubeEvent = serde_json::from_value(json!({
        "metadata":{"name":"warning","namespace":"ns","uid":"event-uid","resourceVersion":"1"},
        "involvedObject":{"kind":"PersistentVolumeClaim","namespace":"ns","name":"data"},
        "type":"Warning","message":"StorageClass fast not found",
        "lastTimestamp":(now()-chrono::Duration::seconds(10)).to_rfc3339()
    }))
    .unwrap();
    sync(&session, vec![warning]);
    assert_eq!(reason(&session), "StorageClass fast not found");
    session.fail(
        0,
        Source::Events,
        ObservationFailure::Read(FailureKind::Timeout),
    );
    assert_eq!(
        reason(&session),
        "Last known warning: StorageClass fast not found"
    );
}

#[test]
fn workloads_are_unavailable_only_when_every_source_failed_without_data() {
    use crate::workloads::{WorkloadCollectionOutcome, WorkloadSource};
    use k8s_openapi::api::apps::v1::{DaemonSet, StatefulSet};
    let workload_sources = [
        Source::Pods,
        Source::Deployments,
        Source::StatefulSets,
        Source::DaemonSets,
    ];
    let messages = |outcome: &WorkloadCollectionOutcome| match outcome {
        WorkloadCollectionOutcome::Complete(_) => Vec::new(),
        WorkloadCollectionOutcome::Partial { unavailable, .. }
        | WorkloadCollectionOutcome::Unavailable {
            errors: unavailable,
            ..
        } => unavailable
            .iter()
            .map(|error| (error.source, error.message.clone()))
            .collect(),
    };

    let failed_without_data = session();
    for source in workload_sources {
        failed_without_data.fail(0, source, ObservationFailure::Read(FailureKind::Forbidden));
    }
    let outcome = failed_without_data.derive(now()).summary.workloads.clone();
    assert!(matches!(
        outcome,
        WorkloadCollectionOutcome::Unavailable { .. }
    ));
    assert_eq!(messages(&outcome).len(), 4);
    assert!(
        messages(&outcome)
            .iter()
            .all(|(_, message)| message == "Not allowed to read this collection")
    );

    let session = session();
    sync::<Pod>(&session, vec![]);
    sync::<Deployment>(&session, vec![]);
    sync::<StatefulSet>(&session, vec![]);
    sync::<DaemonSet>(&session, vec![]);
    assert!(matches!(
        session.derive(now()).summary.workloads,
        WorkloadCollectionOutcome::Complete(_)
    ));
    session.fail(
        0,
        Source::Deployments,
        ObservationFailure::Read(FailureKind::Forbidden),
    );
    let outcome = session.derive(now()).summary.workloads.clone();
    assert!(matches!(outcome, WorkloadCollectionOutcome::Partial { .. }));
    assert_eq!(
        messages(&outcome),
        [(
            WorkloadSource::Deployments,
            "Not allowed to read this collection · showing last known data".to_string()
        )]
    );
    for source in workload_sources {
        session.fail(0, source, ObservationFailure::Read(FailureKind::Timeout));
    }
    let outcome = session.derive(now()).summary.workloads.clone();
    assert!(matches!(outcome, WorkloadCollectionOutcome::Partial { .. }));
    assert_eq!(messages(&outcome).len(), 4);
    assert!(
        messages(&outcome)
            .iter()
            .all(|(_, message)| message.ends_with(" · showing last known data"))
    );
}

/// Objects from JSON, each given the uid and resourceVersion a reflector
/// requires: its namespace and name, and "1".
fn objects<K: serde::de::DeserializeOwned>(values: serde_json::Value) -> Vec<K> {
    let serde_json::Value::Array(values) = values else {
        panic!("expected an array");
    };
    values
        .into_iter()
        .map(|mut value| {
            let metadata = value["metadata"].as_object_mut().unwrap();
            let uid = format!(
                "{}/{}",
                metadata
                    .get("namespace")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
                metadata["name"].as_str().unwrap()
            );
            metadata.entry("uid").or_insert(json!(uid));
            metadata.entry("resourceVersion").or_insert(json!("1"));
            serde_json::from_value(value).unwrap()
        })
        .collect()
}

#[test]
fn ready_since_pod_issues_and_not_ready_counts_come_from_status() {
    use k8s_openapi::api::core::v1::Node;
    let session = session();
    sync::<Node>(
        &session,
        objects(
            json!([{"metadata":{"name":"worker"},"status":{"conditions":[{"type":"Ready","status":"False","reason":"KubeletNotReady","lastTransitionTime":"2026-09-01T00:00:00Z"}]}}]),
        ),
    );
    sync::<Pod>(
        &session,
        objects(json!([
            {"metadata":{"name":"pending","namespace":"batch"},"spec":{"containers":[],"nodeName":"worker"},"status":{"phase":"Pending"}},
            {"metadata":{"name":"crash","namespace":"payments"},"spec":{"containers":[],"nodeName":"worker"},"status":{"phase":"Running","containerStatuses":[{"name":"app","image":"app","imageID":"app","ready":false,"restartCount":14,"state":{"waiting":{"reason":"CrashLoopBackOff"}}}]}}
        ])),
    );
    let publication = session.derive(now());
    let data = &publication.summary;
    let nodes = data.nodes.loaded().unwrap();
    assert!(!nodes[0].is_ready());
    assert_eq!(nodes[0].ready().unwrap().reason, "KubeletNotReady");
    assert!(nodes[0].ready().unwrap().since.is_some());
    assert_eq!(nodes[0].pods, 2);
    let pods = data.pods.loaded().unwrap();
    assert_eq!(pods.total, 2);
    assert_eq!(
        pods.by_namespace,
        std::collections::BTreeMap::from([("batch".into(), 1), ("payments".into(), 1)])
    );
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
    use k8s_openapi::api::core::v1::PersistentVolumeClaim;
    let session = session();
    let event = |name: &str, seconds: i64, kind: &str| json!({"metadata":{"name":name,"namespace":"batch"},"involvedObject":{"kind":"PersistentVolumeClaim","namespace":"batch","name":"report-data"},"type":kind,"reason":"ProvisioningFailed","message":"StorageClass fast not found","lastTimestamp":(now() + chrono::Duration::seconds(seconds)).to_rfc3339()});
    sync::<KubeEvent>(
        &session,
        objects(json!([
            event("old", -3601, "Warning"),
            event("new", -10, "Warning"),
            event("normal", -10, "Normal"),
            event("future", 10, "Warning")
        ])),
    );
    sync::<PersistentVolumeClaim>(
        &session,
        objects(json!([
            {"metadata":{"namespace":"batch","name":"report-data","creationTimestamp":"2026-09-01T00:00:00Z"},"status":{"phase":"Pending"}},
            {"metadata":{"namespace":"batch","name":"bound"},"status":{"phase":"Bound"}}
        ])),
    );
    let publication = session.derive(now());
    let data = &publication.summary;
    assert_eq!(data.events.loaded().unwrap().total, 1);
    let claims = data.claims.loaded().unwrap();
    assert_eq!(claims.bound, 1);
    assert_eq!(claims.pending_count, 1);
    assert_eq!(claims.pending[0].reason, "StorageClass fast not found");
    assert!(claims.pending[0].since.is_some());
}

#[test]
fn refused_or_failed_events_leave_other_parts_loaded() {
    use k8s_openapi::api::core::v1::Node;
    for failure in [
        ObservationFailure::Read(FailureKind::Forbidden),
        ObservationFailure::Read(FailureKind::Timeout),
    ] {
        let session = session();
        sync::<Node>(&session, vec![]);
        sync::<Pod>(&session, vec![]);
        session.fail(0, Source::Events, failure);
        let publication = session.derive(now());
        let data = &publication.summary;
        assert!(data.events.error().is_some());
        assert!(data.nodes.loaded().is_some());
        assert!(data.pods.loaded().is_some());
        assert!(data.workloads.snapshot().is_some());
    }
}

#[test]
fn issue_caps_keep_newest_and_preserve_counts() {
    use k8s_openapi::api::core::v1::PersistentVolumeClaim;
    let session = session();
    let pods: Vec<_> = (0..250).map(|ix| json!({"metadata":{"name":format!("pod-{ix}"),"namespace":"ns","creationTimestamp":format!("2026-09-01T00:{:02}:{:02}Z",ix/60,ix%60)},"status":{"phase":"Pending"}})).collect();
    let claims: Vec<_> = pods
        .iter()
        .map(|pod| json!({"metadata":pod["metadata"],"status":{"phase":"Pending"}}))
        .collect();
    sync::<Pod>(&session, objects(json!(pods)));
    sync::<PersistentVolumeClaim>(&session, objects(json!(claims)));
    let publication = session.derive(now());
    let data = &publication.summary;
    assert_eq!(data.pods.loaded().unwrap().total, 250);
    assert_eq!(data.pods.loaded().unwrap().issues.len(), ISSUE_LIMIT);
    assert_eq!(
        data.pods
            .loaded()
            .unwrap()
            .issues_by_status
            .values()
            .sum::<usize>(),
        250
    );
    assert_eq!(data.pods.loaded().unwrap().issues[0].name, "pod-249");
    assert_eq!(data.claims.loaded().unwrap().pending_count, 250);
    assert_eq!(data.claims.loaded().unwrap().pending.len(), ISSUE_LIMIT);
}

#[test]
fn warning_cap_does_not_remove_a_pending_claims_reason() {
    use k8s_openapi::api::core::v1::PersistentVolumeClaim;
    let session = session();
    let mut events: Vec<_> = (0..250).map(|ix| json!({"metadata":{"name":format!("warning-{ix}"),"namespace":"ns"},"involvedObject":{"kind":"Pod","name":format!("pod-{ix}")},"type":"Warning","lastTimestamp":(now() - chrono::Duration::seconds(1)).to_rfc3339()})).collect();
    events.push(json!({"metadata":{"name":"claim","namespace":"batch"},"involvedObject":{"kind":"PersistentVolumeClaim","namespace":"batch","name":"report-data"},"type":"Warning","message":"StorageClass fast not found","lastTimestamp":(now() - chrono::Duration::seconds(60)).to_rfc3339()}));
    sync::<KubeEvent>(&session, objects(json!(events)));
    sync::<PersistentVolumeClaim>(
        &session,
        objects(
            json!([{"metadata":{"namespace":"batch","name":"report-data"},"status":{"phase":"Pending"}}]),
        ),
    );
    let publication = session.derive(now());
    let data = &publication.summary;
    assert_eq!(data.events.loaded().unwrap().newest.len(), ISSUE_LIMIT);
    assert_eq!(data.events.loaded().unwrap().total, 251);
    assert_eq!(
        data.claims.loaded().unwrap().pending[0].reason,
        "StorageClass fast not found"
    );
}

#[test]
fn summary_retains_only_issue_identities() {
    use k8s_openapi::api::core::v1::PersistentVolumeClaim;
    let session = session();
    sync::<Pod>(
        &session,
        objects(json!([
            {"metadata":{"name":"pending","namespace":"batch","uid":"pending-uid"},"spec":{"containers":[]},"status":{"phase":"Pending"}},
            {"metadata":{"name":"healthy","namespace":"batch","uid":"healthy-uid"},"spec":{"containers":[]},"status":{"phase":"Succeeded"}}
        ])),
    );
    sync::<PersistentVolumeClaim>(
        &session,
        objects(json!([
            {"metadata":{"name":"claim","namespace":"batch","uid":"claim-uid"},"spec":{},"status":{"phase":"Pending"}}
        ])),
    );
    let publication = session.derive(now());
    let data = &publication.summary;
    assert_eq!(
        data.references
            .get(&("pods".into(), "batch".into(), "pending".into()))
            .map(String::as_str),
        Some("pending-uid")
    );
    assert_eq!(
        data.references
            .get(&(
                "persistentvolumeclaims".into(),
                "batch".into(),
                "claim".into()
            ))
            .map(String::as_str),
        Some("claim-uid")
    );
    assert!(
        !data
            .references
            .contains_key(&("pods".into(), "batch".into(), "healthy".into()))
    );
}
