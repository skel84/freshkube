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
        // Record before answering: failures must be asserted on the test task,
        // never hidden in kube's buffered service worker.
        seen.lock()
            .unwrap()
            .push(format!("{} {}", request.method(), request.uri()));
        let path = request.uri().path();
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
                if mode == "oversized" {
                    json!({"items":(0..201).map(|index|json!({"metadata":{"name":format!("orders-{index}"),"namespace":"shop","uid":format!("invented-{index}")}})).collect::<Vec<_>>()})
                } else if mode == "capped" || (mode == "slow-pages" && path.ends_with("/clusters"))
                {
                    json!({"items":[],"metadata":{"continue":"more&not-a-query=1"}})
                } else if mode == "malformed" {
                    json!({"items":[{"metadata":{"name":"missing-uid"}}]})
                } else {
                    json!({"items":[]})
                }
            }
            _ => {
                status = 500;
                json!({"kind":"Status","status":"Failure","reason":"UnexpectedRequest","code":500})
            }
        };
        let hold = mode == "timeout" && path.ends_with("/backups");
        let slow = mode == "slow-pages"
            && (path.ends_with("/clusters")
                || path == "/apis/postgresql.cnpg.io"
                || path == "/apis/postgresql.cnpg.io/v1");
        async move {
            if slow {
                tokio::time::sleep(std::time::Duration::from_secs(4)).await;
            }
            if hold {
                std::future::pending::<()>().await;
            }
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
        assert_requests(&calls, vec!["GET /apis/postgresql.cnpg.io".into()]);
    }
}
#[tokio::test]
async fn lists_are_get_only_scoped_and_refused_backup_is_not_empty_coverage() {
    let (client, calls) = client("backup-refused");
    let inventory = read(&client, Some("shop"), now()).await;
    assert_eq!(inventory.databases.coverage, Coverage::Complete);
    assert_eq!(inventory.backups.coverage, Coverage::Refused);
    assert_eq!(inventory.schedules.coverage, Coverage::Complete);
    assert_requests(&calls, expected_requests(1));
    assert!(!format!("{inventory:?}").contains("sensitive-canary"));
}
#[tokio::test]
async fn pagination_is_bounded_and_opaque_tokens_are_encoded() {
    let (client, calls) = client("capped");
    let inventory = read(&client, Some("shop"), now()).await;
    assert_eq!(inventory.databases.coverage, Coverage::Partial);
    assert_requests(&calls, expected_requests(5));
}
#[tokio::test]
async fn invalid_objects_do_not_make_a_successful_empty_inventory() {
    let (client, calls) = client("malformed");
    let inventory = read(&client, Some("shop"), now()).await;
    assert_eq!(inventory.databases.coverage, Coverage::Partial);
    assert!(inventory.databases.items.is_empty());
    assert_requests(&calls, expected_requests(1));
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

#[test]
fn topology_uses_reported_roles_and_never_guesses_replication_health() {
    let db=serde_json::from_value::<cnpg::Cluster>(json!({"metadata":metadata("db"),"status":{"currentPrimary":"orders-1","instancesStatus":{"healthy":["orders-1","orders-2","orders-3"]},"instancesReportedState":{"orders-1":{"isPrimary":true},"orders-2":{"isPrimary":false}}}})).unwrap().project().unwrap();
    assert_eq!(db.instances[0].role, Fact::Known(Role::Primary));
    assert_eq!(db.instances[1].role, Fact::Known(Role::Replica));
    assert_eq!(db.instances[2].role, Fact::Unknown);
    assert!(
        db.instances
            .iter()
            .all(|instance| instance.replication == Fact::Unknown)
    );
}

#[tokio::test(start_paused = true)]
async fn a_kind_timeout_does_not_turn_other_successful_reads_into_unknown() {
    let (client, calls) = client("timeout");
    let inv = read(&client, Some("shop"), now()).await;
    assert_eq!(inv.backups.coverage, Coverage::TimedOut);
    assert_eq!(inv.databases.coverage, Coverage::Complete);
    assert_requests(&calls, expected_requests(1));
}
#[tokio::test]
async fn a_server_ignoring_the_page_size_cannot_make_an_unbounded_inventory() {
    let (client, calls) = client("oversized");
    let inv = read(&client, Some("shop"), now()).await;
    assert_eq!(inv.backups.coverage, Coverage::Partial);
    assert_eq!(inv.backups.items.len(), 200);
    assert_requests(&calls, expected_requests(1));
}

fn assert_requests(calls: &Mutex<Vec<String>>, mut expected: Vec<String>) {
    let mut actual = calls.lock().unwrap().clone();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
}
fn expected_requests(pages: usize) -> Vec<String> {
    let mut expected = vec![
        "GET /apis/postgresql.cnpg.io".into(),
        "GET /apis/postgresql.cnpg.io/v1".into(),
    ];
    for kind in ["clusters", "backups", "scheduledbackups"] {
        for page in 0..pages {
            expected.push(format!(
                "GET /apis/postgresql.cnpg.io/v1/namespaces/shop/{kind}?limit=200{}",
                if page == 0 {
                    ""
                } else {
                    "&continue=more%26not-a-query%3D1"
                }
            ));
        }
    }
    expected
}
#[test]
fn ready_without_observed_generation_is_current_but_reported_old_generation_is_stale() {
    use crate::{HasHealth, HealthIndicator};
    for (observed, current) in [(None, true), (Some(1), false), (Some(2), true)] {
        let mut condition = json!({"type":"Ready", "status":"True", "reason":"ClusterIsReady"});
        if let Some(generation) = observed {
            condition["observedGeneration"] = generation.into();
        }
        let db = serde_json::from_value::<cnpg::Cluster>(
            json!({"metadata":metadata("db"),"status":{"conditions":[condition]}}),
        )
        .unwrap()
        .project()
        .unwrap();
        assert_eq!(db.conditions[0].current, current);
        assert_eq!(
            db.health(),
            if current {
                HealthIndicator::Healthy
            } else {
                HealthIndicator::Unknown
            }
        );
    }
}
#[test]
fn wal_archiver_and_reported_backup_configuration_are_projected() {
    for (spec, expected) in [
        (
            json!({"plugins":[{"isWALArchiver":true,"parameters":{"token":"sensitive-canary"}}]}),
            Fact::Known(true),
        ),
        (
            json!({"plugins":[{"isWALArchiver":false}]}),
            Fact::Known(false),
        ),
        (json!({}), Fact::Known(false)),
        (serde_json::Value::Null, Fact::Unknown),
    ] {
        let db =
            serde_json::from_value::<cnpg::Cluster>(json!({"metadata":metadata("db"),"spec":spec}))
                .unwrap()
                .project()
                .unwrap();
        assert_eq!(db.backup_configuration, expected);
        assert!(!format!("{db:?}").contains("sensitive-canary"));
    }
    let mut inv = example::inventory(now());
    let mut db = inv.databases.items[0].clone();
    db.backup_configuration = Fact::Known(false);
    inv.schedules.items.clear();
    assert_eq!(coverage(&db, &inv, now()).restore, Restore::NoConfiguration);
}
#[test]
fn backup_finalizing_and_wal_archiving_failure_use_cnpg_phases() {
    for (phase, expected) in [
        ("finalizing", BackupState::Running),
        ("walArchivingFailing", BackupState::Failed),
        ("walArchiving", BackupState::Unknown),
    ] {
        let backup = serde_json::from_value::<cnpg::BackupObject>(
            json!({"metadata":metadata("backup"),"status":{"phase":phase}}),
        )
        .unwrap()
        .project()
        .unwrap();
        assert_eq!(backup.state, expected);
    }
}
#[tokio::test(start_paused = true)]
async fn discovery_and_slow_pages_stay_within_budget_and_keep_other_kinds() {
    let (client, calls) = client("slow-pages");
    let start = tokio::time::Instant::now();
    let inv = read(&client, Some("shop"), now()).await;
    assert_eq!(inv.discovery, Coverage::Complete);
    assert_eq!(inv.databases.coverage, Coverage::TimedOut);
    assert_eq!(inv.backups.coverage, Coverage::Complete);
    assert_eq!(inv.schedules.coverage, Coverage::Complete);
    assert!(start.elapsed() <= READ_BUDGET);
    let mut expected = expected_requests(1);
    for _ in 0..3 {
        expected.push("GET /apis/postgresql.cnpg.io/v1/namespaces/shop/clusters?limit=200&continue=more%26not-a-query%3D1".into());
    }
    assert_requests(&calls, expected);
}

#[tokio::test]
async fn fake_records_unknown_requests_and_returns_an_error_without_losing_the_worker() {
    let (client, calls) = client("empty");
    let request = http::Request::builder()
        .method("POST")
        .uri("/api/v1/secrets")
        .body(Vec::new())
        .unwrap();
    assert!(client.request::<serde_json::Value>(request).await.is_err());
    assert_requests(&calls, vec!["POST /api/v1/secrets".into()]);
    let inv = read(&client, Some("shop"), now()).await;
    assert_eq!(inv.discovery, Coverage::Complete);
    let mut expected = expected_requests(1);
    expected.push("POST /api/v1/secrets".into());
    assert_requests(&calls, expected);
}

#[test]
fn every_example_shape_has_consistent_backup_times_and_old_is_old() {
    let at = now();
    for shape in [
        "",
        "not-installed",
        "refused",
        "unsupported",
        "failed",
        "empty",
        "backup-refused",
        "partial",
        "old",
        "backup-failed",
        "no-primary",
        "stale",
        "no-backup",
        "no-schedule",
        "suspended",
        "mariadb-model",
    ] {
        let inv = example::shaped(at, shape);
        for backup in &inv.backups.items {
            if let (Some(start), Some(end)) = (backup.started, backup.completed) {
                assert!(
                    start <= end,
                    "{shape}: {} ends before it starts",
                    backup.identity.name
                );
            }
            if let (Some(created), Some(start)) = (backup.created, backup.started) {
                assert!(
                    created <= start,
                    "{shape}: {} starts before creation",
                    backup.identity.name
                );
            }
        }
        if shape == "old" {
            assert_eq!(
                coverage(&inv.databases.items[0], &inv, at).freshness,
                Freshness::Old
            );
        }
    }
}

#[test]
fn example_daily_schedule_brackets_observation_at_two_utc_every_day() {
    use chrono::Timelike;
    for hour in [0, 1, 2, 12, 23] {
        let at = now().with_hour(hour).unwrap();
        let inv = example::inventory(at);
        let schedule = &inv.schedules.items[0];
        let (last, next) = (schedule.last.unwrap(), schedule.next.unwrap());
        assert_eq!(schedule.schedule, Fact::Known("0 0 2 * * *".into()));
        assert!(last <= at && at < next);
        assert_eq!(next - last, Duration::days(1));
        assert_eq!((last.hour(), last.minute(), last.second()), (2, 0, 0));
        assert_eq!((next.hour(), next.minute(), next.second()), (2, 0, 0));
    }
}
