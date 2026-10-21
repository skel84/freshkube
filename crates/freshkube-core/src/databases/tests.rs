use super::*;
use chrono::{Duration, TimeZone, Utc};
use serde_json::json;
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};
fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap()
}
fn metadata(kind: &str) -> serde_json::Value {
    json!({"name":format!("orders-{kind}"),"namespace":"shop","uid":format!("invented-{kind}"),"generation":2})
}

#[test]
fn typed_projection_skips_sensitive_fields_and_preserves_unknowns() {
    let raw = json!({"metadata":metadata("db"),"spec":{"instances":3,"superuserSecret":{"name":"sensitive-canary"}},"status":{"currentPrimary":"pending","demotionToken":"sensitive-canary","instancesStatus":{"healthy":["orders-1"]},"conditions":[{"type":"Ready","status":"True","reason":"ClusterIsReady","observedGeneration":1,"message":"sensitive-canary"}]}});
    let db = serde_json::from_value::<cnpg::Cluster>(raw)
        .unwrap()
        .project()
        .unwrap();
    assert_eq!(db.current_primary, Fact::Unknown);
    assert_eq!(db.ready_instances, Fact::Unknown);
    assert_eq!(db.instances[0].replication, Fact::Unknown);
    assert!(!db.conditions[0].current);
    assert!(!format!("{db:?}").contains("sensitive-canary"));
    for status in [serde_json::Value::Null, json!({})] {
        let db = serde_json::from_value::<cnpg::Cluster>(
            json!({"metadata":metadata("db"),"spec":null,"status":status}),
        )
        .unwrap()
        .project()
        .unwrap();
        assert_eq!(db.desired_instances, Fact::Unknown);
    }
    let backup = serde_json::from_value::<cnpg::BackupObject>(json!({"metadata":metadata("backup"),"status":{"phase":"new-phase","s3Credentials":{"token":"sensitive-canary"},"commandError":"sensitive-canary","startedAt":"bad-time"}})).unwrap().project().unwrap();
    assert_eq!(backup.state, BackupState::Unknown);
    assert!(backup.started.is_none());
    assert!(!format!("{backup:?}").contains("sensitive-canary"));
}
#[test]
fn coverage_separates_newest_attempt_from_completed_backup_and_24_hour_boundary() {
    let at = now();
    let mut inventory = example::inventory(at);
    let db = inventory.databases.items[0].clone();
    let result = coverage(&db, &inventory, at);
    assert_eq!(result.newest.unwrap().state, BackupState::Running);
    assert_eq!(result.completed.unwrap().state, BackupState::Completed);
    assert_eq!(result.freshness, Freshness::Fresh);
    inventory.backups.items[0].started = None;
    for (age, expected) in [
        (Duration::hours(24), Freshness::Fresh),
        (Duration::hours(24) + Duration::seconds(1), Freshness::Old),
        (Duration::seconds(-1), Freshness::Unknown),
    ] {
        inventory.backups.items[0].completed = Some(at - age);
        assert_eq!(coverage(&db, &inventory, at).freshness, expected);
    }
    inventory.backups.items[0].completed = None;
    assert_eq!(coverage(&db, &inventory, at).freshness, Freshness::Unknown);
}
#[test]
fn incomplete_reads_never_prove_absence_or_orphans_and_keep_old_evidence_stale() {
    let at = now();
    let old = example::inventory(at);
    let db = old.databases.items[0].clone();
    let mut new = Inventory::unavailable(Coverage::Refused, at + Duration::hours(1));
    new.retain(&old);
    assert!(new.backups.stale);
    assert_eq!(new.backups.observed_at, at);
    assert_eq!(coverage(&db, &new, at).freshness, Freshness::Unknown);
    assert!(!coverage(&db, &new, at).no_schedule);
    assert_eq!(
        link(new.backups.items[2].cluster.as_ref(), &new.databases),
        LinkState::Unknown
    );
    let mut complete = old;
    complete.backups.items.clear();
    complete.schedules.items.clear();
    assert_eq!(coverage(&db, &complete, at).freshness, Freshness::None);
    assert!(coverage(&db, &complete, at).no_schedule);
    assert_eq!(
        coverage(&db, &complete, at).restore,
        Restore::NoCompletedBackup
    );
}
#[test]
fn references_respect_namespaces_engines_and_incarnations() {
    let at = now();
    let mut inventory = example::inventory(at);
    let reference = inventory.backups.items[0].cluster.clone().unwrap();
    assert_eq!(
        link(Some(&reference), &inventory.databases),
        LinkState::Matched
    );
    inventory.databases.items[0].identity.uid = "replacement".into();
    assert_eq!(
        link(Some(&reference), &inventory.databases),
        LinkState::Orphaned
    );
    assert!(
        coverage(&inventory.databases.items[0], &inventory, at)
            .completed
            .is_none()
    );
    inventory.databases.items[0].identity.uid = reference.uid.clone().unwrap();
    inventory.databases.items[0].identity.namespace = "elsewhere".into();
    assert_eq!(
        link(Some(&reference), &inventory.databases),
        LinkState::Orphaned
    );
    let maria = example::mariadb(at);
    assert_eq!(maria.engine, Engine::MariaDb);
    assert_eq!(maria.current_primary, Fact::Unknown);
}

fn client(mode: &'static str) -> (kube::Client, Arc<Mutex<Vec<String>>>) {
    use http::{Request, Response};
    use kube::client::Body;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let service = tower::service_fn(move |request: Request<Body>| {
        assert_eq!(request.method(), "GET");
        let path = request.uri().path();
        assert!(path.starts_with("/apis/postgresql.cnpg.io"));
        assert!(
            !["secret", "credential", "exec", "portforward"]
                .iter()
                .any(|word| path.contains(word))
        );
        seen.lock().unwrap().push(request.uri().to_string());
        let mut status = 200;
        let body = match path {
            "/apis/postgresql.cnpg.io" => {
                if mode == "absent" {
                    status = 404;
                    json!({"kind":"Status","status":"Failure","reason":"NotFound","code":404})
                } else if mode == "discovery-refused" {
                    status = 403;
                    json!({"kind":"Status","status":"Failure","reason":"Forbidden","code":403})
                } else {
                    json!({"versions":[{"version":if mode=="unsupported" {"v9"}else{"v1"}}]})
                }
            }
            "/apis/postgresql.cnpg.io/v1" => {
                json!({"resources":[{"name":"clusters","kind":"Cluster","namespaced":true,"verbs":["list"]},{"name":"backups","kind":"Backup","namespaced":true,"verbs":["list"]},{"name":"scheduledbackups","kind":"ScheduledBackup","namespaced":true,"verbs":["list"]}]})
            }
            path if path.ends_with("/backups") && mode == "backup-refused" => {
                status = 403;
                json!({"kind":"Status","status":"Failure","reason":"Forbidden","message":"sensitive-canary","code":403})
            }
            path if ["clusters", "backups", "scheduledbackups"]
                .iter()
                .any(|kind| {
                    path == format!("/apis/postgresql.cnpg.io/v1/namespaces/shop/{kind}")
                }) =>
            {
                if mode == "capped" {
                    json!({"items":[],"metadata":{"continue":"more&not-a-query=1"}})
                } else if mode == "malformed" {
                    json!({"items":[{"metadata":{"name":"missing-uid"}}]})
                } else {
                    json!({"items":[]})
                }
            }
            _ => panic!("unexpected API path {path}"),
        };
        async move {
            Ok::<_, Infallible>(
                Response::builder()
                    .status(status)
                    .body(Body::from(body.to_string().into_bytes()))
                    .unwrap(),
            )
        }
    });
    (kube::Client::new(service, "shop"), calls)
}
#[tokio::test]
async fn discovery_absence_refusal_and_unsupported_version_are_distinct() {
    for (mode, expected) in [
        ("absent", Coverage::NotServed),
        ("discovery-refused", Coverage::Refused),
        ("unsupported", Coverage::Unsupported),
    ] {
        let (client, calls) = client(mode);
        let inventory = read(&client, Some("shop"), now()).await;
        assert_eq!(inventory.discovery, expected);
        assert_eq!(calls.lock().unwrap().len(), 1);
    }
}
#[tokio::test]
async fn lists_are_get_only_scoped_and_refused_backup_is_not_empty_coverage() {
    let (client, calls) = client("backup-refused");
    let inventory = read(&client, Some("shop"), now()).await;
    assert_eq!(inventory.databases.coverage, Coverage::Complete);
    assert_eq!(inventory.backups.coverage, Coverage::Refused);
    assert_eq!(inventory.schedules.coverage, Coverage::Complete);
    assert_eq!(calls.lock().unwrap().len(), 5);
    assert!(!format!("{inventory:?}").contains("sensitive-canary"));
}
#[tokio::test]
async fn pagination_is_bounded_and_opaque_tokens_are_encoded() {
    let (client, calls) = client("capped");
    let inventory = read(&client, Some("shop"), now()).await;
    assert_eq!(inventory.databases.coverage, Coverage::Partial);
    assert_eq!(calls.lock().unwrap().len(), 17);
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .any(|path| path.contains("continue=more%26not-a-query%3D1"))
    );
}
#[tokio::test]
async fn invalid_objects_do_not_make_a_successful_empty_inventory() {
    let (client, _) = client("malformed");
    let inventory = read(&client, Some("shop"), now()).await;
    assert_eq!(inventory.databases.coverage, Coverage::Partial);
    assert!(inventory.databases.items.is_empty());
}

#[test]
fn name_only_records_must_postdate_the_current_cluster_and_bad_times_are_unknown() {
    let at = now();
    let mut inv = example::inventory(at);
    let db = inv.databases.items[0].clone();
    inv.backups.items[0].cluster.as_mut().unwrap().uid = None;
    assert!(coverage(&db, &inv, at).completed.is_some());
    inv.backups.items[0].created = db.created.map(|created| created - Duration::hours(1));
    assert!(coverage(&db, &inv, at).completed.is_none());
    inv.backups.items[0].created = None;
    assert_eq!(coverage(&db, &inv, at).freshness, Freshness::Unknown);
    inv.backups.items[0].cluster.as_mut().unwrap().uid = Some(db.identity.uid.clone());
    inv.backups.items[0].completed = inv.backups.items[0]
        .started
        .map(|at| at - Duration::seconds(1));
    assert_eq!(coverage(&db, &inv, at).freshness, Freshness::Unknown);
}
#[tokio::test]
async fn invalid_namespace_makes_no_request() {
    let (client, calls) = client("empty");
    let inv = read(&client, Some("../secrets?x=1"), now()).await;
    assert_eq!(inv.discovery, Coverage::Failed);
    assert!(calls.lock().unwrap().is_empty());
}
