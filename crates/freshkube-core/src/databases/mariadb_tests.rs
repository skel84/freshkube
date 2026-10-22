//! The MariaDB operator's reader, the CloudNativePG additions of slice 2, and
//! Coroot's projection, on invented objects.
use super::coroot::{self as evidence, CorootState};
use super::*;
use crate::coroot::{AppReport, AppView, Check, Status};
use chrono::{TimeZone, Utc};
use serde_json::json;
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap()
}
fn meta(name: &str) -> serde_json::Value {
    json!({"name":name,"namespace":"shop","uid":format!("invented-{name}"),"generation":3,"creationTimestamp":"2026-10-01T00:00:00Z"})
}
fn mariadb(spec: serde_json::Value, status: serde_json::Value) -> Database {
    serde_json::from_value::<mariadb::MariaDbObject>(
        json!({"metadata":meta("ledger-db"),"spec":spec,"status":status}),
    )
    .unwrap()
    .project()
    .unwrap()
    .database
}

#[test]
fn a_mariadb_projects_roles_replication_lag_and_version() {
    let db = mariadb(
        json!({"image":"docker.io/library/mariadb:11.4.2","replicas":3,"rootPasswordSecretKeyRef":{"name":"sensitive-canary"},
               "replication":{"enabled":true,"primary":{"podIndex":0}}}),
        json!({"currentPrimary":"ledger-db-0","currentPrimaryPodIndex":0,
               "replication":{"roles":{"ledger-db-0":"Primary","ledger-db-1":"Replica","ledger-db-2":"Unknown"},
                 "replicas":{"ledger-db-1":{"slaveIORunning":true,"slaveSQLRunning":false,"secondsBehindMaster":12.5}}},
               "conditions":[{"type":"Ready","status":"True","reason":"StatefulSetReady","observedGeneration":3}]}),
    );
    assert_eq!(db.engine, Engine::MariaDb);
    assert_eq!(db.engine_label(), "MariaDB 11.4");
    assert_eq!(db.desired_instances, Fact::Known(3));
    assert_eq!(
        db.ready_instances,
        Fact::Unknown,
        "the CRD has no ready count"
    );
    assert_eq!(db.current_primary, Fact::Known("ledger-db-0".into()));
    assert_eq!(db.target_primary, Fact::Known("ledger-db-0".into()));
    assert_eq!(db.instances[0].role, Fact::Known(Role::Primary));
    assert_eq!(db.instances[1].role, Fact::Known(Role::Replica));
    assert_eq!(db.instances[2].role, Fact::Unknown);
    assert_eq!(
        db.instances[1].replication,
        Fact::Known("IO running · SQL stopped".into())
    );
    assert_eq!(
        db.instances[1].lag,
        Fact::Known(Lag {
            seconds: 12.5,
            source: LagSource::Operator
        })
    );
    assert_eq!(db.instances[2].lag, Fact::Unknown);
    assert_eq!(db.certificates, Fact::Unknown);
    assert_eq!(
        crate::HasHealth::health(&db),
        crate::HealthIndicator::Healthy
    );
    assert!(!format!("{db:?}").contains("sensitive-canary"));
}

#[test]
fn versions_come_from_the_image_tag_then_the_default_version_else_unknown() {
    let version =
        |spec: serde_json::Value, status: serde_json::Value| mariadb(spec, status).version;
    assert_eq!(
        version(json!({"image":"mariadb:11"}), json!({})),
        Fact::Known("11".into())
    );
    assert_eq!(
        version(json!({"image":"mariadb:10.11.9-jammy"}), json!({})),
        Fact::Known("10.11".into())
    );
    assert_eq!(
        version(
            json!({"image":"mariadb:latest"}),
            json!({"defaultVersion":"11.8"})
        ),
        Fact::Known("11.8".into())
    );
    assert_eq!(
        version(json!({"image":"mariadb@sha256:abc"}), json!({})),
        Fact::Unknown
    );
    assert_eq!(version(json!({}), json!({})), Fact::Unknown);
}

#[test]
fn a_requested_primary_is_known_only_from_the_pod_index() {
    let db = mariadb(
        json!({"replication":{"primary":{"podIndex":1}}}),
        json!({"currentPrimary":"ledger-db-0"}),
    );
    assert_eq!(db.target_primary, Fact::Known("ledger-db-1".into()));
    let db = mariadb(json!({}), json!({"currentPrimary":"ledger-db-0"}));
    assert_eq!(db.target_primary, Fact::Unknown);
}

#[test]
fn the_switchover_start_is_the_primary_switched_false_transition_only() {
    let status = |condition: &str| json!({"currentPrimary":"ledger-db-0","conditions":[{"type":"PrimarySwitched","status":condition,"reason":"SwitchPrimary","lastTransitionTime":"2026-10-20T11:30:00Z"}]});
    let spec = json!({"replication":{"primary":{"podIndex":1}}});
    let db = mariadb(spec.clone(), status("False"));
    assert_eq!(
        db.primary_changed,
        Some(Utc.with_ymd_and_hms(2026, 10, 20, 11, 30, 0).unwrap())
    );
    assert!(matches!(switchover(&db, now()), Switchover::Stuck(_)));
    // A finished switchover's time says nothing of a later request.
    let db = mariadb(spec.clone(), status("True"));
    assert_eq!(db.primary_changed, None);
    assert_eq!(switchover(&db, now()), Switchover::Unknown);
    let db = mariadb(spec, json!({"currentPrimary":"ledger-db-0"}));
    assert_eq!(switchover(&db, now()), Switchover::Unknown);
}

fn backup(spec: serde_json::Value, status: serde_json::Value) -> mariadb::Item {
    serde_json::from_value::<mariadb::BackupObject>(
        json!({"metadata":meta("ledger-manual"),"spec":spec,"status":status}),
    )
    .unwrap()
    .project()
    .unwrap()
}
fn complete(status: &str, reason: &str) -> serde_json::Value {
    json!({"conditions":[{"type":"Complete","status":status,"reason":reason,"lastTransitionTime":"2026-10-20T09:00:00Z"}]})
}

#[test]
fn a_backups_complete_condition_maps_to_states_and_a_failed_job_is_true() {
    let spec = json!({"mariaDbRef":{"name":"ledger-db"},"maxRetention":"720h"});
    for (status, reason, expected) in [
        ("True", "JobComplete", BackupState::Completed),
        ("True", "JobFailed", BackupState::Failed),
        ("False", "JobRunning", BackupState::Running),
        ("False", "JobSuspended", BackupState::Pending),
        ("False", "SomethingNew", BackupState::Unknown),
    ] {
        let mariadb::Item::Run(run) = backup(spec.clone(), complete(status, reason)) else {
            panic!("a backup without a schedule is a run");
        };
        assert_eq!(run.state, expected, "{status} {reason}");
        assert_eq!(run.completed.is_some(), expected == BackupState::Completed);
        let reference = run.cluster.unwrap();
        assert_eq!(reference.engine, Engine::MariaDb);
        assert_eq!(
            (reference.namespace.as_str(), reference.name.as_str()),
            ("shop", "ledger-db")
        );
        assert_eq!(
            reference.uid, None,
            "no owner reference names the incarnation"
        );
    }
    let mariadb::Item::Run(run) = backup(spec, json!({})) else {
        panic!()
    };
    assert_eq!(run.state, BackupState::Unknown);
}

#[test]
fn a_backup_with_a_schedule_is_a_schedule_and_reports_no_run_time() {
    let item = backup(
        json!({"mariaDbRef":{"name":"ledger-db"},"schedule":{"cron":"0 3 * * *","suspend":true}}),
        complete("True", "CronJobSuccess"),
    );
    let mariadb::Item::Schedule(schedule) = item else {
        panic!("a scheduled backup is a schedule");
    };
    assert_eq!(schedule.schedule, Fact::Known("0 3 * * *".into()));
    assert_eq!(schedule.suspended, Fact::Known(true));
    assert_eq!((schedule.last, schedule.next), (None, None));
    let cron = cron::Cron::parse(Engine::MariaDb, "0 3 * * *").unwrap();
    assert_eq!(cron.words(), "Daily at 03:00");
}

#[test]
fn an_owner_reference_gives_a_mariadb_backup_its_incarnation() {
    let mut object = meta("ledger-manual");
    object["ownerReferences"] = json!([{"apiVersion":"k8s.mariadb.com/v1alpha1","kind":"MariaDB","name":"ledger-db","uid":"invented-ledger-db"}]);
    let item = serde_json::from_value::<mariadb::BackupObject>(
        json!({"metadata":object,"spec":{"mariaDbRef":{"name":"ledger-db"}}}),
    )
    .unwrap()
    .project()
    .unwrap();
    let mariadb::Item::Run(run) = item else {
        panic!()
    };
    assert_eq!(
        run.cluster.unwrap().uid.as_deref(),
        Some("invented-ledger-db")
    );
}

#[test]
fn maxscale_is_the_proxy_only_from_a_complete_read_that_names_it() {
    let scaler = |ready: &str| {
        serde_json::from_value::<mariadb::MaxScaleObject>(json!({
            "metadata":meta("ledger-db-maxscale"),
            "spec":{"replicas":2},
            "status":{"replicas":2,"conditions":[{"type":"Ready","status":ready,"reason":"MaxScaleReady","observedGeneration":3}]}
        }))
        .unwrap()
        .project()
        .unwrap()
    };
    let listed = |items, coverage| Read {
        items,
        coverage,
        observed_at: now(),
        stale: false,
    };
    let reference = ("shop".to_owned(), "ledger-db-maxscale".to_owned());
    let found = mariadb::proxy(
        Some(&reference),
        &listed(vec![scaler("True")], Coverage::Complete),
    );
    assert_eq!(
        found,
        Fact::Known(Some(Proxy {
            kind: ProxyKind::MaxScale,
            running: 2,
            desired: 2,
            ready: Fact::Known(true)
        }))
    );
    assert_eq!(
        mariadb::proxy(None, &listed(vec![], Coverage::Complete)),
        Fact::Known(None)
    );
    assert_eq!(
        mariadb::proxy(None, &listed(vec![], Coverage::Refused)),
        Fact::Unknown
    );
    assert_eq!(
        mariadb::proxy(Some(&reference), &listed(vec![], Coverage::Complete)),
        Fact::Unknown,
        "a named MaxScale that isn't listed is not a known absence"
    );
}

#[test]
fn cloudnativepg_additions_project_version_certificates_and_instance_health() {
    let db = serde_json::from_value::<cnpg::Cluster>(json!({
        "metadata":meta("orders-db"),
        "spec":{"instances":3,"imageName":"ghcr.io/cloudnative-pg/postgresql:15.3"},
        "status":{"image":"ghcr.io/cloudnative-pg/postgresql:16.4-12",
                  "pgDataImageInfo":{"majorVersion":16},
                  "instancesStatus":{"healthy":["orders-db-1"],"failed":["orders-db-2"]},
                  "certificates":{"serverTLSSecret":"sensitive-canary","expirations":{
                      "orders-db-server":"2027-01-02 15:04:05.123456789 +0000 UTC",
                      "orders-db-ca":"not a time"}}}
    }))
    .unwrap()
    .project()
    .unwrap();
    assert_eq!(db.engine_label(), "PostgreSQL 16");
    assert_eq!(db.instances[0].ready, Fact::Known(true));
    assert_eq!(db.instances[1].ready, Fact::Known(false));
    let certificates = db.certificates.known().unwrap();
    assert_eq!(certificates.len(), 2);
    let server = certificates
        .iter()
        .find(|c| c.name == "orders-db-server")
        .unwrap();
    assert_eq!(
        server.expires,
        Some(
            Utc.with_ymd_and_hms(2027, 1, 2, 15, 4, 5).unwrap()
                + chrono::Duration::nanoseconds(123_456_789)
        )
    );
    assert_eq!(
        certificates
            .iter()
            .find(|c| c.name == "orders-db-ca")
            .unwrap()
            .expires,
        None
    );
    assert!(!format!("{db:?}").contains("sensitive-canary"));
    // The image tag alone gives the major version when no info is reported.
    let db = serde_json::from_value::<cnpg::Cluster>(
        json!({"metadata":meta("orders-db"),"status":{"image":"ghcr.io/cloudnative-pg/postgresql:17.1@sha256:abc"}}),
    )
    .unwrap()
    .project()
    .unwrap();
    assert_eq!(db.version, Fact::Known("17".into()));
    assert_eq!(db.certificates, Fact::Unknown);
}

#[test]
fn poolers_add_up_per_cluster_and_never_invent_a_count() {
    let pooler = |name: &str, cluster: &str, instances: Option<u32>, running: Option<u32>| {
        let mut status = json!({});
        if let Some(running) = running {
            status["instances"] = running.into();
        }
        let mut spec = json!({"cluster":{"name":cluster},"type":"rw"});
        if let Some(instances) = instances {
            spec["instances"] = instances.into();
        }
        serde_json::from_value::<cnpg::PoolerObject>(
            json!({"metadata":meta(name),"spec":spec,"status":status}),
        )
        .unwrap()
        .project()
        .unwrap()
    };
    let db = example::inventory(now()).databases.items.remove(0);
    let listed = |items, coverage| Read {
        items,
        coverage,
        observed_at: now(),
        stale: false,
    };
    let two = listed(
        vec![
            pooler("orders-rw", "orders-db", Some(2), Some(2)),
            pooler("orders-ro", "orders-db", None, Some(1)),
            pooler("other", "other-db", Some(5), Some(5)),
        ],
        Coverage::Complete,
    );
    assert_eq!(
        cnpg::proxy(&db, &two),
        Fact::Known(Some(Proxy {
            kind: ProxyKind::Pooler,
            running: 3,
            desired: 3,
            ready: Fact::Unknown
        }))
    );
    let unreported = listed(
        vec![pooler("orders-rw", "orders-db", Some(2), None)],
        Coverage::Complete,
    );
    assert_eq!(cnpg::proxy(&db, &unreported), Fact::Unknown);
    assert_eq!(
        cnpg::proxy(&db, &listed(vec![], Coverage::Complete)),
        Fact::Known(None)
    );
    assert_eq!(
        cnpg::proxy(&db, &listed(vec![], Coverage::NotServed)),
        Fact::Known(None)
    );
    assert_eq!(
        cnpg::proxy(&db, &listed(vec![], Coverage::Refused)),
        Fact::Unknown
    );
}

// --- The reader, against a fake that records every request.

fn fake(maria: bool) -> (kube::Client, Arc<Mutex<Vec<String>>>) {
    use http::{Request, Response};
    use kube::client::Body;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let service = tower::service_fn(move |request: Request<Body>| {
        seen.lock()
            .unwrap()
            .push(format!("{} {}", request.method(), request.uri()));
        let path = request.uri().path().to_owned();
        let mut status = 200;
        let body = match path.as_str() {
            "/apis/postgresql.cnpg.io" => {
                status = 404;
                json!({"kind":"Status","status":"Failure","reason":"NotFound","code":404})
            }
            "/apis/k8s.mariadb.com" if maria => json!({"versions":[{"version":"v1alpha1"}]}),
            "/apis/k8s.mariadb.com/v1alpha1" if maria => json!({"resources":[
                {"name":"mariadbs","kind":"MariaDB","namespaced":true,"verbs":["list"]},
                {"name":"backups","kind":"Backup","namespaced":true,"verbs":["list"]},
                {"name":"maxscales","kind":"MaxScale","namespaced":true,"verbs":["list"]}]}),
            "/apis/k8s.mariadb.com/v1alpha1/namespaces/shop/mariadbs" if maria => json!({"items":[
                {"metadata":meta("ledger-db"),"spec":{"image":"mariadb:11.4","replicas":3,"maxScaleRef":{"name":"ledger-db-maxscale"},
                    "rootPasswordSecretKeyRef":{"name":"sensitive-canary"}},
                 "status":{"currentPrimary":"ledger-db-0"}}]}),
            "/apis/k8s.mariadb.com/v1alpha1/namespaces/shop/backups" if maria => json!({"items":[
                {"metadata":meta("ledger-manual"),"spec":{"mariaDbRef":{"name":"ledger-db"}},"status":complete("True","JobComplete")},
                {"metadata":meta("ledger-nightly"),"spec":{"mariaDbRef":{"name":"ledger-db"},"schedule":{"cron":"0 3 * * *"}}}]}),
            "/apis/k8s.mariadb.com/v1alpha1/namespaces/shop/maxscales" if maria => json!({"items":[
                {"metadata":meta("ledger-db-maxscale"),"spec":{"replicas":2},"status":{"replicas":2,"conditions":[{"type":"Ready","status":"True","reason":"MaxScaleReady"}]}}]}),
            _ => {
                status = if maria { 500 } else { 404 };
                json!({"kind":"Status","status":"Failure","reason":"x","code":status})
            }
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
async fn the_mariadb_reader_is_get_only_scoped_and_reads_no_secret() {
    let (client, calls) = fake(true);
    let inv = read(&client, Some("shop"), now()).await;
    assert_eq!(inv.discovery, Coverage::Complete);
    assert_eq!(
        inv.engines,
        vec![
            (Engine::CloudNativePg, Coverage::NotServed),
            (Engine::MariaDb, Coverage::Complete)
        ]
    );
    assert_eq!(inv.databases.items.len(), 1);
    let db = &inv.databases.items[0];
    assert_eq!(db.engine_label(), "MariaDB 11.4");
    assert_eq!(
        db.proxy,
        Fact::Known(Some(Proxy {
            kind: ProxyKind::MaxScale,
            running: 2,
            desired: 2,
            ready: Fact::Known(true)
        }))
    );
    assert_eq!(inv.backups.items.len(), 1);
    assert_eq!(inv.schedules.items.len(), 1);
    assert_eq!(inv.backups.coverage, Coverage::Complete);
    let found = coverage(db, &inv, now());
    assert_eq!(found.completed.unwrap().identity.name, "ledger-manual");
    assert_eq!(found.schedules.len(), 1);
    assert!(!format!("{inv:?}").contains("sensitive-canary"));
    let mut actual = calls.lock().unwrap().clone();
    actual.sort();
    let mut expected: Vec<String> = [
        "GET /apis/postgresql.cnpg.io",
        "GET /apis/k8s.mariadb.com",
        "GET /apis/k8s.mariadb.com/v1alpha1",
        "GET /apis/k8s.mariadb.com/v1alpha1/namespaces/shop/mariadbs?limit=200",
        "GET /apis/k8s.mariadb.com/v1alpha1/namespaces/shop/backups?limit=200",
        "GET /apis/k8s.mariadb.com/v1alpha1/namespaces/shop/maxscales?limit=200",
    ]
    .map(String::from)
    .to_vec();
    expected.sort();
    assert_eq!(actual, expected);
    assert!(actual.iter().all(|call| call.starts_with("GET ")));
    assert!(actual.iter().all(|call| !call.contains("secrets")));
}

#[tokio::test]
async fn neither_operator_installed_is_not_served_not_a_failure() {
    let (client, calls) = fake(false);
    let inv = read(&client, None, now()).await;
    assert_eq!(inv.discovery, Coverage::NotServed);
    assert_eq!(inv.databases.coverage, Coverage::NotServed);
    assert_eq!(calls.lock().unwrap().len(), 2);
}

#[test]
fn one_operators_failure_is_not_hidden_by_the_others_success() {
    let at = now();
    let ok = example::inventory(at);
    let refused = Inventory::unavailable(Coverage::Refused, at);
    let merged = Inventory::merge(
        vec![(Engine::CloudNativePg, ok), (Engine::MariaDb, refused)],
        at,
    );
    assert_eq!(merged.discovery, Coverage::Complete);
    assert_eq!(merged.databases.coverage, Coverage::Refused);
    assert_eq!(merged.databases.items.len(), 2, "what was read still shows");
    // An operator that is not installed leaves the other's coverage alone.
    let ok = example::inventory(at);
    let absent = Inventory::unavailable(Coverage::NotServed, at);
    let merged = Inventory::merge(
        vec![(Engine::CloudNativePg, ok), (Engine::MariaDb, absent)],
        at,
    );
    assert_eq!(merged.databases.coverage, Coverage::Complete);
}

// --- Coroot's projection.

fn check(id: &str, title: &str, status: Status, message: &str) -> Check {
    Check {
        id: id.into(),
        title: title.into(),
        status,
        message: message.into(),
        threshold: 0.,
        unit: String::new(),
        condition: String::new(),
    }
}
fn report(name: &str, checks: Vec<Check>) -> AppReport {
    AppReport {
        name: name.into(),
        status: Status::Ok,
        checks,
        widgets: vec![],
        custom: false,
        instrumentation: String::new(),
    }
}

#[test]
fn coroots_database_report_folds_ok_and_unused_checks_and_leads_with_fired_ones() {
    let mut view = AppView::default();
    view.map.app.id = evidence::app_id("core-fra", Engine::CloudNativePg, "shop", "orders-db");
    view.map.app.status = Status::Warning;
    view.reports = vec![
        report(
            "SLO",
            vec![
                check("SLOAvailability", "Availability", Status::Ok, ""),
                check("SLOLatency", "Latency", Status::Warning, "slow responses"),
            ],
        ),
        report(
            "Postgres",
            vec![
                check(
                    "PostgresAvailability",
                    "Postgres availability",
                    Status::Ok,
                    "",
                ),
                check(
                    "PostgresLatency",
                    "Postgres latency",
                    Status::Warning,
                    "p99 is 900ms",
                ),
                check(
                    "PostgresReplicationLag",
                    "Postgres replication lag",
                    Status::Critical,
                    "lag is 80s",
                ),
                check(
                    "PostgresWalArchiving",
                    "Postgres WAL archiving",
                    Status::Ok,
                    "",
                ),
                check("PostgresBackups", "Postgres backups", Status::Unknown, ""),
            ],
        ),
    ];
    let found = evidence::project(&view, Engine::CloudNativePg, now()).unwrap();
    assert_eq!(
        found
            .fired
            .iter()
            .map(|f| f.title.as_str())
            .collect::<Vec<_>>(),
        ["Postgres replication lag", "Postgres latency"],
        "worst first"
    );
    assert_eq!(found.fired[0].message, "lag is 80s");
    assert_eq!(
        found.ok,
        ["Postgres availability", "Postgres WAL archiving"]
    );
    assert_eq!(found.unused, ["Postgres backups"]);
    assert_eq!(found.slo.len(), 1);
    assert_eq!(found.slo[0].title, "Latency");
    assert_eq!(found.archiving, Some(Status::Ok));
    assert_eq!(found.health(), crate::HealthIndicator::Error);
    // The page of another engine's report is not this database's.
    assert!(evidence::project(&view, Engine::MariaDb, now()).is_none());
}

#[test]
fn a_report_with_only_unknown_checks_is_unknown_never_a_failure() {
    let view = AppView {
        reports: vec![report(
            "Mysql",
            vec![check(
                "MysqlAvailability",
                "Mysql availability",
                Status::Unknown,
                "",
            )],
        )],
        ..AppView::default()
    };
    let found = evidence::project(&view, Engine::MariaDb, now()).unwrap();
    assert!(found.fired.is_empty());
    assert_eq!(found.unused, ["Mysql availability"]);
    assert_eq!(found.health(), crate::HealthIndicator::Unknown);
    assert_eq!(
        CorootState::Read(Box::new(found)).health(),
        crate::HealthIndicator::Unknown
    );
}
