use super::coroot::{Absence, CorootState, DatabaseReport, Evidence, Fired, app_id};
use super::*;
use crate::HealthIndicator;
use crate::coroot::{AppId, ReadError, Status};
use chrono::{DateTime, Duration, TimeZone, Utc};
use std::collections::BTreeMap;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap()
}
fn report(db: &Database) -> DatabaseReport {
    DatabaseReport {
        app: app_id("core-fra", db.engine, "shop", &db.identity.name),
        status: Status::Ok,
        fired: vec![],
        ok: vec!["Availability".into()],
        unused: vec![],
        slo: vec![],
        archiving: None,
        lag: BTreeMap::new(),
        read_at: now(),
    }
}
fn read(report: DatabaseReport) -> CorootState {
    CorootState::Read(Box::new(report))
}
fn verdict_of(db: &Database, inv: &Inventory, coroot: &CorootState) -> Verdict {
    verdict(db, inv, coroot, now())
}

#[test]
fn certificates_warn_under_fourteen_days_and_missing_dates_are_unknown() {
    let at = now();
    let cert = |expires| Certificate {
        name: "orders-db-server".into(),
        expires,
    };
    let state = |after: Duration| certificate(&cert(Some(at + after)), at);
    assert!(matches!(
        state(Duration::days(13)),
        CertificateState::Expiring(_)
    ));
    assert_eq!(state(Duration::days(14)), CertificateState::Valid);
    assert_eq!(state(Duration::days(15)), CertificateState::Valid);
    assert_eq!(state(Duration::seconds(0)), CertificateState::Expired);
    assert_eq!(state(Duration::days(-1)), CertificateState::Expired);
    assert_eq!(certificate(&cert(None), at), CertificateState::Unknown);
}

#[test]
fn an_expiring_certificate_is_a_warning_and_a_missing_date_is_not() {
    let inv = example::inventory(now());
    let mut db = inv.databases.items[0].clone();
    db.certificates = Fact::Known(vec![Certificate {
        name: "orders-db-server".into(),
        expires: Some(now() + Duration::days(13)),
    }]);
    let v = verdict_of(&db, &inv, &CorootState::NotRead);
    assert_eq!(v.state.health, HealthIndicator::Warning);
    assert!(v.headline.contains("orders-db-server expires in 13 days"));
    db.certificates = Fact::Known(vec![Certificate {
        name: "orders-db-server".into(),
        expires: None,
    }]);
    assert_eq!(
        verdict_of(&db, &inv, &CorootState::NotRead).state.health,
        HealthIndicator::Healthy
    );
    db.certificates = Fact::Unknown;
    assert_eq!(
        verdict_of(&db, &inv, &CorootState::NotRead).state.health,
        HealthIndicator::Healthy
    );
}

#[test]
fn lag_warns_above_thirty_seconds_from_coroot_only() {
    let inv = example::inventory(now());
    let db = inv.databases.items[0].clone();
    for (seconds, expected) in [
        (30.0, HealthIndicator::Healthy),
        (30.1, HealthIndicator::Warning),
    ] {
        let mut found = report(&db);
        found.lag.insert(
            "orders-db-2".into(),
            Lag {
                seconds,
                source: LagSource::Coroot,
            },
        );
        let v = verdict_of(&db, &inv, &read(found));
        assert_eq!(v.state.health, expected, "{seconds}");
        if expected == HealthIndicator::Warning {
            assert_eq!(v.state.source, Source::Coroot);
            assert!(v.headline.contains("orders-db-2 lags 30 s behind (Coroot)"));
        }
    }
    // Without Coroot there is no measurement: lag stays Unknown.
    assert_eq!(lag_of(&db.instances[1], None), Fact::Unknown);
}

#[test]
fn lag_prefers_coroots_measurement_over_the_operators() {
    let mut instance = Instance::named("ledger-db-1");
    instance.lag = Fact::Known(Lag {
        seconds: 1.,
        source: LagSource::Operator,
    });
    let inv = example::inventory(now());
    let mut found = report(&inv.databases.items[1]);
    assert_eq!(
        lag_of(&instance, Some(&found)),
        Fact::Known(Lag {
            seconds: 1.,
            source: LagSource::Operator
        })
    );
    found.lag.insert(
        "ledger-db-1".into(),
        Lag {
            seconds: 2.,
            source: LagSource::Coroot,
        },
    );
    assert_eq!(
        lag_of(&instance, Some(&found)),
        Fact::Known(Lag {
            seconds: 2.,
            source: LagSource::Coroot
        })
    );
}

#[test]
fn a_switchover_is_stuck_only_past_ten_minutes_and_only_with_a_time() {
    let inv = example::inventory(now());
    let mut db = inv.databases.items[1].clone();
    assert_eq!(switchover(&db, now()), Switchover::Settled);
    db.target_primary = Fact::Known("ledger-db-1".into());
    assert_eq!(switchover(&db, now()), Switchover::Unknown);
    for (age, expected) in [
        (Duration::minutes(9), false),
        (Duration::minutes(10), false),
        (Duration::minutes(10) + Duration::seconds(1), true),
    ] {
        db.primary_changed = Some(now() - age);
        match (switchover(&db, now()), expected) {
            (Switchover::Stuck(_), true) | (Switchover::InProgress(_), false) => {}
            (other, _) => panic!("{age}: {other:?}"),
        }
    }
    db.primary_changed = Some(now() + Duration::minutes(1));
    assert_eq!(switchover(&db, now()), Switchover::Unknown);
    db.primary_changed = Some(now() - Duration::hours(1));
    db.current_primary = Fact::Unknown;
    assert_eq!(switchover(&db, now()), Switchover::Unknown);
    let cnpg = &inv.databases.items[0];
    assert_eq!(switchover(cnpg, now()), Switchover::NotApplicable);
}

#[test]
fn a_stuck_switchover_is_an_error_naming_both_primaries() {
    let inv = example::shaped(now(), "switchover-stuck");
    let db = &inv.databases.items[1];
    let v = verdict_of(db, &inv, &CorootState::NotRead);
    assert_eq!(v.state.health, HealthIndicator::Error);
    assert!(v.headline.contains("Switchover stuck for 25 min"));
    assert!(v.headline.contains("ledger-db-1 requested"));
    assert!(v.headline.contains("ledger-db-0 still primary"));
}

fn archiving_db(operator: Option<&str>) -> Database {
    let mut db = example::inventory(now()).databases.items.remove(0);
    db.conditions.retain(|c| c.kind != "ContinuousArchiving");
    if let Some(status) = operator {
        db.conditions.push(Condition {
            kind: "ContinuousArchiving".into(),
            status: status.into(),
            reason: "x".into(),
            transition_time: None,
            observed_generation: None,
            current: true,
        });
    }
    db
}

#[test]
fn archiving_joins_the_operator_and_coroot_for_every_pair() {
    use Archiving::*;
    let coroot = |status| {
        let mut found = report(&archiving_db(None));
        found.archiving = status;
        found
    };
    let cases = [
        (Some("True"), Some(Status::Ok), Ok),
        (Some("True"), Some(Status::Critical), Disagree),
        (Some("True"), Some(Status::Unknown), Ok),
        (Some("True"), None, Ok),
        (Some("False"), Some(Status::Ok), Disagree),
        (Some("False"), Some(Status::Warning), Failing),
        (Some("False"), None, Failing),
        (Some("Unknown"), Some(Status::Ok), Ok),
        (None, Some(Status::Ok), Ok),
        (None, Some(Status::Critical), Failing),
        (None, Some(Status::Unknown), Unknown),
        (None, None, Unknown),
    ];
    for (operator, status, expected) in cases {
        let db = archiving_db(operator);
        assert_eq!(
            archiving(&db, Some(&coroot(status))),
            expected,
            "{operator:?} {status:?}"
        );
    }
    assert_eq!(archiving(&archiving_db(None), None), Unknown);
}

#[test]
fn a_disagreement_stops_the_restore_window_at_the_last_backup() {
    let inv = example::inventory(now());
    let db = &inv.databases.items[0];
    let found = coverage(db, &inv, now());
    let ok = restore_window(db, &inv, &found, Archiving::Ok, now()).unwrap();
    assert!(ok.to_now);
    assert_eq!(ok.until, now());
    for archiving in [Archiving::Disagree, Archiving::Failing, Archiving::Unknown] {
        let window = restore_window(db, &inv, &found, archiving, now()).unwrap();
        assert!(!window.to_now);
        assert_eq!(window.until, now() - Duration::hours(1));
        assert_eq!(window.from, now() - Duration::hours(1));
    }
    let mut none = inv.clone();
    none.backups.items.clear();
    let found = coverage(db, &none, now());
    assert!(restore_window(db, &none, &found, Archiving::Ok, now()).is_none());
}

#[test]
fn a_disagreeing_archive_is_a_warning_even_when_the_operator_says_ok() {
    let inv = example::inventory(now());
    let db = &inv.databases.items[0];
    let mut found = report(db);
    found.archiving = Some(Status::Critical);
    let v = verdict_of(db, &inv, &read(found));
    assert_eq!(v.state.health, HealthIndicator::Warning);
    assert!(v.headline.contains("disagree on WAL archiving"));
}

#[test]
fn a_coroot_unknown_never_worsens_a_row_and_unreachable_is_not_used() {
    let inv = example::inventory(now());
    let db = &inv.databases.items[0];
    let mut fired = report(db);
    fired.fired.push(Fired {
        title: "Postgres latency".into(),
        message: "slow".into(),
        status: Status::Warning,
    });
    let last = Box::new(fired);
    for state in [
        CorootState::NoCoroot(Absence::NotLinked),
        CorootState::AppNotFound,
        CorootState::NotRead,
        CorootState::Unreachable {
            why: ReadError::Unreachable,
            last: Some(last.clone()),
        },
    ] {
        let v = verdict_of(db, &inv, &state);
        assert_eq!(v.state.health, HealthIndicator::Healthy, "{state:?}");
        assert_eq!(v.state.source, Source::Operator);
    }
    let v = verdict_of(db, &inv, &CorootState::Read(last));
    assert_eq!(v.state.health, HealthIndicator::Warning);
    assert_eq!(v.state.source, Source::Coroot);
    assert_eq!(v.headline, "Postgres latency: slow");
}

#[test]
fn both_sources_agreeing_on_a_problem_say_both() {
    let inv = example::inventory(now());
    let mut db = inv.databases.items[0].clone();
    db.conditions[0].status = "False".into();
    db.conditions[0].reason = "ClusterIsNotReady".into();
    let mut found = report(&db);
    found.fired.push(Fired {
        title: "Postgres availability".into(),
        message: "down".into(),
        status: Status::Warning,
    });
    let v = verdict_of(&db, &inv, &read(found));
    assert_eq!(v.state.health, HealthIndicator::Warning);
    assert_eq!(v.state.source, Source::Both);
}

#[test]
fn proxy_findings_name_the_pods_and_a_pooler_has_no_readiness() {
    let inv = example::inventory(now());
    let mut db = inv.databases.items[0].clone();
    db.proxy = Fact::Known(Some(Proxy {
        kind: ProxyKind::Pooler,
        running: 1,
        desired: 2,
        ready: Fact::Unknown,
    }));
    let v = verdict_of(&db, &inv, &CorootState::NotRead);
    assert!(v.headline.contains("Pooler: 1 of 2 pods running"));
    db.proxy = Fact::Known(Some(Proxy {
        kind: ProxyKind::Pooler,
        running: 2,
        desired: 2,
        ready: Fact::Unknown,
    }));
    assert_eq!(
        verdict_of(&db, &inv, &CorootState::NotRead).state.health,
        HealthIndicator::Healthy
    );
    db.proxy = Fact::Known(Some(Proxy {
        kind: ProxyKind::MaxScale,
        running: 2,
        desired: 2,
        ready: Fact::Known(false),
    }));
    assert!(
        verdict_of(&db, &inv, &CorootState::NotRead)
            .headline
            .contains("MaxScale is not ready")
    );
    db.proxy = Fact::Known(Some(Proxy {
        kind: ProxyKind::MaxScale,
        running: 0,
        desired: 2,
        ready: Fact::Unknown,
    }));
    assert_eq!(
        verdict_of(&db, &inv, &CorootState::NotRead).state.health,
        HealthIndicator::Error
    );
    db.proxy = Fact::Unknown;
    assert_eq!(
        verdict_of(&db, &inv, &CorootState::NotRead).state.health,
        HealthIndicator::Healthy
    );
}

#[test]
fn old_and_missing_backups_are_warnings_and_unknown_freshness_is_not() {
    let at = now();
    for (shape, text) in [
        ("old", "older than 24 hours"),
        ("backup-failed", "backup attempt, orders-evening, failed"),
    ] {
        let inv = example::shaped(at, shape);
        let v = verdict_of(&inv.databases.items[0], &inv, &CorootState::NotRead);
        assert_eq!(v.state.health, HealthIndicator::Warning, "{shape}");
        assert!(
            v.problems().any(|f| f.why.contains(text)),
            "{shape}: {:?}",
            v.findings
        );
    }
    let inv = example::shaped(at, "no-backup");
    let v = verdict_of(&inv.databases.items[0], &inv, &CorootState::NotRead);
    assert!(v.problems().any(|f| f.why == "No completed backup"));
    let inv = example::shaped(at, "backup-refused");
    let v = verdict_of(&inv.databases.items[0], &inv, &CorootState::NotRead);
    assert_eq!(v.state.health, HealthIndicator::Healthy);
}

#[test]
fn no_readiness_is_unknown_not_healthy() {
    let inv = example::shaped(now(), "no-primary");
    let v = verdict_of(&inv.databases.items[0], &inv, &CorootState::NotRead);
    assert_eq!(v.state.health, HealthIndicator::Unknown);
    assert!(v.headline.starts_with("State unknown"));
}

#[test]
fn an_orphaned_backup_is_a_warning_naming_its_database_only_after_a_complete_read() {
    let inv = example::inventory(now());
    let orphan_backup = &inv.backups.items[2];
    let finding = orphan(orphan_backup.cluster.as_ref(), &inv).unwrap();
    assert_eq!(finding.health, HealthIndicator::Warning);
    assert!(finding.why.contains("archive-db"));
    assert!(orphan(inv.backups.items[0].cluster.as_ref(), &inv).is_none());
    let mut refused = inv.clone();
    refused.databases.coverage = Coverage::Refused;
    assert!(orphan(orphan_backup.cluster.as_ref(), &refused).is_none());
}

#[test]
fn evidence_keeps_the_last_good_answer_beside_an_unreachable_one() {
    let inv = example::inventory(now());
    let db = &inv.databases.items[0];
    let uid = db.identity.uid.clone();
    let mut previous = Evidence::default();
    previous.states.insert(uid.clone(), read(report(db)));
    let mut new = Evidence::default();
    new.states.insert(
        uid.clone(),
        CorootState::Unreachable {
            why: ReadError::Timeout,
            last: None,
        },
    );
    new.retain(&previous);
    match new.state(&uid).unwrap() {
        CorootState::Unreachable { last, .. } => assert!(last.is_some()),
        other => panic!("{other:?}"),
    }
    assert_eq!(new.state(&uid).unwrap().health(), HealthIndicator::Unknown);
}

#[test]
fn application_ids_follow_coroots_kinds() {
    assert_eq!(
        app_id("core-fra", Engine::CloudNativePg, "shop", "orders-db"),
        AppId::new("core-fra:shop:DatabaseCluster:orders-db")
    );
    assert_eq!(
        app_id("core-fra", Engine::MariaDb, "shop", "ledger-db"),
        AppId::new("core-fra:shop:StatefulSet:ledger-db")
    );
}

#[test]
fn an_operators_lag_is_the_operators_finding_and_counts_in_its_state() {
    let inv = example::inventory(now());
    let mut db = inv.databases.items[1].clone();
    db.instances[1].lag = Fact::Known(Lag {
        seconds: 45.,
        source: LagSource::Operator,
    });
    let v = verdict_of(&db, &inv, &CorootState::NotRead);
    assert_eq!(v.state.health, HealthIndicator::Warning);
    assert_eq!(v.state.source, Source::Operator);
    let finding = v.problems().next().unwrap();
    assert_eq!(finding.source, Source::Operator);
    assert!(finding.why.contains("the operator"));
    // Coroot's own measurement is Coroot's.
    let mut found = report(&db);
    found.lag.insert(
        "ledger-db-1".into(),
        Lag {
            seconds: 50.,
            source: LagSource::Coroot,
        },
    );
    let v = verdict_of(&db, &inv, &read(found));
    assert_eq!(v.problems().next().unwrap().source, Source::Coroot);
}

#[test]
fn archiving_findings_name_who_reported_them() {
    let inv = example::inventory(now());
    let mut db = inv.databases.items[0].clone();
    db.conditions.retain(|c| c.kind != "ContinuousArchiving");
    let mut found = report(&db);
    found.archiving = Some(Status::Critical);
    let v = verdict_of(&db, &inv, &read(found.clone()));
    let finding = v.problems().find(|f| f.why.contains("archiving")).unwrap();
    assert_eq!(finding.source, Source::Coroot, "only Coroot reported it");
    assert_eq!(v.state.source, Source::Coroot);
    db.conditions.push(Condition {
        kind: "ContinuousArchiving".into(),
        status: "False".into(),
        reason: "ContinuousArchivingFailing".into(),
        transition_time: None,
        observed_generation: None,
        current: true,
    });
    let v = verdict_of(&db, &inv, &read(found.clone()));
    assert_eq!(
        v.problems()
            .find(|f| f.why.contains("archiving"))
            .unwrap()
            .source,
        Source::Both
    );
    let v = verdict_of(&db, &inv, &CorootState::NotRead);
    assert_eq!(
        v.problems()
            .find(|f| f.why.contains("archiving"))
            .unwrap()
            .source,
        Source::Operator
    );
}

#[test]
fn a_mariadb_with_only_a_scheduled_backup_has_an_unknown_last_backup_not_none() {
    let inv = example::shaped(now(), "scheduled-only");
    let db = &inv.databases.items[1];
    assert_eq!(db.engine, Engine::MariaDb);
    let found = coverage(db, &inv, now());
    assert_eq!(found.freshness, Freshness::Unknown);
    assert_eq!(found.restore, Restore::Unknown);
    let v = verdict_of(db, &inv, &CorootState::NotRead);
    assert!(v.problems().all(|f| f.why != "No completed backup"));
    // With no schedule either, or on CloudNativePG, none is still none.
    let mut bare = inv.clone();
    bare.schedules.items.retain(|s| s.identity.kind != "Backup");
    assert_eq!(
        coverage(&bare.databases.items[1], &bare, now()).freshness,
        Freshness::None
    );
    let mut cnpg = inv.clone();
    cnpg.backups.items.clear();
    assert_eq!(
        coverage(&cnpg.databases.items[0], &cnpg, now()).freshness,
        Freshness::None
    );
}

#[test]
fn a_refused_engine_keeps_its_last_good_rows_as_retained_while_the_other_answers() {
    let at = now();
    let previous = example::inventory(at);
    let mut fresh = example::inventory(at);
    fresh.engines[1].1 = Coverage::Refused;
    fresh.databases.items.remove(1);
    fresh
        .backups
        .items
        .retain(|b| b.identity.group != "k8s.mariadb.com");
    fresh
        .schedules
        .items
        .retain(|s| s.identity.group != "k8s.mariadb.com");
    fresh.databases.coverage = Coverage::Refused;
    fresh.retain(&previous);
    assert_eq!(fresh.retained, vec![Engine::MariaDb]);
    assert_eq!(fresh.databases.items.len(), 2, "ledger-db is kept");
    assert!(
        fresh
            .backups
            .items
            .iter()
            .any(|b| b.identity.name == "ledger-manual")
    );
    assert!(
        fresh
            .schedules
            .items
            .iter()
            .any(|s| s.identity.name == "ledger-nightly")
    );
    // Nothing is retained for an engine that answers or is simply absent.
    let mut ok = example::inventory(at);
    ok.retain(&previous);
    assert!(ok.retained.is_empty());
    let mut absent = example::inventory(at);
    absent.engines[1].1 = Coverage::NotServed;
    absent.databases.items.remove(1);
    absent.retain(&previous);
    assert!(absent.retained.is_empty());
    assert_eq!(absent.databases.items.len(), 1);
}

#[test]
fn a_second_failed_refresh_still_retains_the_rows() {
    let at = now();
    let first_good = example::inventory(at);
    let refused = || {
        let mut fresh = example::inventory(at);
        fresh.engines[1].1 = Coverage::Refused;
        fresh.databases.items.remove(1);
        fresh
            .backups
            .items
            .retain(|b| b.identity.group != "k8s.mariadb.com");
        fresh
            .schedules
            .items
            .retain(|s| s.identity.group != "k8s.mariadb.com");
        fresh.databases.coverage = Coverage::Refused;
        fresh
    };
    let mut one = refused();
    one.retain(&first_good);
    let mut two = refused();
    two.retain(&one);
    assert_eq!(two.retained, vec![Engine::MariaDb]);
    assert!(
        two.databases
            .items
            .iter()
            .any(|d| d.identity.name == "ledger-db")
    );
    assert!(
        two.backups
            .items
            .iter()
            .any(|b| b.identity.name == "ledger-manual")
    );
    assert!(
        two.schedules
            .items
            .iter()
            .any(|s| s.identity.name == "ledger-nightly")
    );
    // And once the operator answers, nothing is kept.
    let mut back = example::inventory(at);
    back.retain(&two);
    assert!(back.retained.is_empty());
}
