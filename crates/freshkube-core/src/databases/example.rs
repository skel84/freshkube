//! Invented records only (`orders-db`, `ledger-db`, namespace `shop`).
//!
//! The field and condition names follow the operators' own schemas:
//! - CloudNativePG `postgresql.cnpg.io/v1`, release-1.28 at a042dc8:
//!   `api/v1/cluster_types.go` (`status.certificates.expirations`,
//!   `status.pgDataImageInfo.majorVersion`, conditions `Ready` and
//!   `ContinuousArchiving`), `api/v1/pooler_types.go` (`status.instances`),
//!   `docs/src/backup.md` (a ScheduledBackup is a six-field cron, seconds first).
//! - mariadb-operator `k8s.mariadb.com/v1alpha1` at v26.10.1 (8f43f43):
//!   `api/v1alpha1/mariadb_types.go` (`status.currentPrimary`,
//!   `spec.replication.primary.podIndex`), `condition_types.go` and
//!   `pkg/condition/primary_switched.go` (a switchover is `PrimarySwitched`
//!   False with reason `SwitchPrimary`), `pkg/condition/complete.go` (a Backup's
//!   `Complete` condition), `base_types.go` (a five-field cron),
//!   `maxscale_types.go`. The StatefulSet is named after the MariaDB:
//!   `internal/controller/mariadb_controller.go:418`.
//! - Coroot at 780b17d: report names `Postgres` and `Mysql`
//!   (`model/audit_report.go`), check ids such as `PostgresWalArchiving`
//!   (`model/check.go`), and an application id `cluster:namespace:kind:name`
//!   (`model/application_id.go`).
use super::coroot::{CorootState, DatabaseReport, Evidence, Fired, app_id};
use super::*;
use crate::coroot::Status;
use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeMap;

/// The Coroot cluster the example entry names.
pub const COROOT_CLUSTER: &str = "core-fra";

fn identity(group: &str, version: &str, kind: &str, plural: &str, name: &str) -> Identity {
    Identity {
        group: group.into(),
        version: version.into(),
        kind: kind.into(),
        plural: plural.into(),
        namespace: "shop".into(),
        name: name.into(),
        uid: format!("invented-{name}"),
    }
}
fn cnpg_identity(kind: &str, plural: &str, name: &str) -> Identity {
    identity("postgresql.cnpg.io", "v1", kind, plural, name)
}
fn maria_identity(kind: &str, plural: &str, name: &str) -> Identity {
    identity("k8s.mariadb.com", "v1alpha1", kind, plural, name)
}
fn condition(kind: &str, status: &str, reason: &str, at: DateTime<Utc>) -> Condition {
    Condition {
        kind: kind.into(),
        status: status.into(),
        reason: reason.into(),
        transition_time: Some(at),
        observed_generation: Some(2),
        current: true,
    }
}

fn orders_db(at: DateTime<Utc>) -> Database {
    let instances = (1..=3)
        .map(|i| Instance {
            role: Fact::Known(if i == 1 { Role::Primary } else { Role::Replica }),
            state: Fact::Known("healthy".into()),
            ready: Fact::Known(true),
            ..Instance::named(format!("orders-db-{i}"))
        })
        .collect();
    Database {
        created: Some(at - Duration::days(10)),
        identity: cnpg_identity("Cluster", "clusters", "orders-db"),
        engine: Engine::CloudNativePg,
        desired_instances: Fact::Known(3),
        ready_instances: Fact::Known(3),
        current_primary: Fact::Known("orders-db-1".into()),
        target_primary: Fact::Known("orders-db-1".into()),
        phase: Fact::Known("Cluster in healthy state".into()),
        backup_configuration: Fact::Known(true),
        conditions: vec![
            condition("Ready", "True", "ClusterIsReady", at - Duration::hours(2)),
            condition(
                "ContinuousArchiving",
                "True",
                "ContinuousArchivingSuccess",
                at - Duration::hours(2),
            ),
        ],
        instances,
        version: Fact::Known("16".into()),
        proxy: Fact::Known(Some(Proxy {
            kind: ProxyKind::Pooler,
            running: 2,
            desired: 2,
            ready: Fact::Unknown,
        })),
        certificates: Fact::Known(
            ["orders-db-server", "orders-db-replication"]
                .into_iter()
                .map(|name| Certificate {
                    name: name.into(),
                    expires: Some(at + Duration::days(60)),
                })
                .collect(),
        ),
        primary_changed: None,
    }
}

fn ledger_db(at: DateTime<Utc>) -> Database {
    let instances = (0..3)
        .map(|i| Instance {
            role: Fact::Known(if i == 0 { Role::Primary } else { Role::Replica }),
            replication: if i == 0 {
                Fact::Unknown
            } else {
                Fact::Known("IO running · SQL running".into())
            },
            lag: if i == 0 {
                Fact::Unknown
            } else {
                Fact::Known(Lag {
                    seconds: 0.4,
                    source: LagSource::Operator,
                })
            },
            ..Instance::named(format!("ledger-db-{i}"))
        })
        .collect();
    Database {
        created: Some(at - Duration::days(20)),
        identity: maria_identity("MariaDB", "mariadbs", "ledger-db"),
        engine: Engine::MariaDb,
        desired_instances: Fact::Known(3),
        ready_instances: Fact::Unknown,
        current_primary: Fact::Known("ledger-db-0".into()),
        target_primary: Fact::Known("ledger-db-0".into()),
        phase: Fact::Unknown,
        backup_configuration: Fact::Unknown,
        conditions: vec![
            condition("Ready", "True", "StatefulSetReady", at - Duration::hours(5)),
            condition(
                "PrimarySwitched",
                "True",
                "SwitchPrimary",
                at - Duration::days(3),
            ),
        ],
        instances,
        version: Fact::Known("11.4".into()),
        proxy: Fact::Known(Some(Proxy {
            kind: ProxyKind::MaxScale,
            running: 2,
            desired: 2,
            ready: Fact::Known(true),
        })),
        certificates: Fact::Unknown,
        primary_changed: None,
    }
}

pub fn inventory(at: DateTime<Utc>) -> Inventory {
    let db = orders_db(at);
    let maria = ledger_db(at);
    let cnpg_ref = ClusterRef {
        engine: Engine::CloudNativePg,
        namespace: "shop".into(),
        name: "orders-db".into(),
        uid: Some(db.identity.uid.clone()),
    };
    let maria_ref = ClusterRef {
        engine: Engine::MariaDb,
        namespace: "shop".into(),
        name: "ledger-db".into(),
        uid: Some(maria.identity.uid.clone()),
    };
    let backup = Backup {
        identity: cnpg_identity("Backup", "backups", "orders-morning"),
        cluster: Some(cnpg_ref.clone()),
        state: BackupState::Completed,
        method: Fact::Known("plugin".into()),
        started: Some(at - Duration::hours(2)),
        completed: Some(at - Duration::hours(1)),
        created: Some(at - Duration::hours(2)),
    };
    let mut running = backup.clone();
    running.identity = cnpg_identity("Backup", "backups", "orders-evening");
    running.state = BackupState::Running;
    running.started = Some(at - Duration::minutes(5));
    running.created = running.started;
    running.completed = None;
    let mut orphan = backup.clone();
    orphan.identity = cnpg_identity("Backup", "backups", "archive-old");
    orphan.cluster = Some(ClusterRef {
        engine: Engine::CloudNativePg,
        namespace: "shop".into(),
        name: "archive-db".into(),
        uid: Some("invented-archive-db".into()),
    });
    let manual = Backup {
        identity: maria_identity("Backup", "backups", "ledger-manual"),
        cluster: Some(maria_ref.clone()),
        state: BackupState::Completed,
        method: Fact::Unknown,
        started: None,
        completed: Some(at - Duration::hours(3)),
        created: Some(at - Duration::hours(3)),
    };
    let today = at.date_naive().and_hms_opt(2, 0, 0).unwrap().and_utc();
    let last = if today > at {
        today - Duration::days(1)
    } else {
        today
    };
    let daily = Schedule {
        created: Some(at - Duration::days(9)),
        identity: cnpg_identity("ScheduledBackup", "scheduledbackups", "orders-daily"),
        cluster: Some(cnpg_ref),
        schedule: Fact::Known("0 0 2 * * *".into()),
        suspended: Fact::Known(false),
        method: Fact::Known("plugin".into()),
        last: Some(last),
        next: Some(last + Duration::days(1)),
    };
    let nightly = Schedule {
        created: Some(at - Duration::days(15)),
        identity: maria_identity("Backup", "backups", "ledger-nightly"),
        cluster: Some(maria_ref),
        schedule: Fact::Known("0 3 * * *".into()),
        suspended: Fact::Known(false),
        method: Fact::Unknown,
        last: None,
        next: None,
    };
    fn read<T>(items: Vec<T>, at: DateTime<Utc>) -> Read<T> {
        Read {
            items,
            coverage: Coverage::Complete,
            observed_at: at,
            stale: false,
        }
    }
    Inventory {
        discovery: Coverage::Complete,
        engines: vec![
            (Engine::CloudNativePg, Coverage::Complete),
            (Engine::MariaDb, Coverage::Complete),
        ],
        retained: Vec::new(),
        databases: read(vec![db, maria], at),
        backups: read(vec![backup, running, orphan, manual], at),
        schedules: read(vec![daily, nightly], at),
    }
}

/// A Coroot report for a healthy database, with the replicas' lag.
fn report(at: DateTime<Utc>, engine: Engine, name: &str, lag: &[(&str, f64)]) -> DatabaseReport {
    let titles = match engine {
        Engine::CloudNativePg => [
            "Postgres availability",
            "Postgres latency",
            "Postgres replication lag",
            "Postgres connections",
            "Postgres WAL archiving",
        ],
        Engine::MariaDb => [
            "Mysql availability",
            "Mysql latency",
            "Mysql replication lag",
            "Mysql connections",
            "Mysql replication status",
        ],
    };
    DatabaseReport {
        app: app_id(COROOT_CLUSTER, engine, "shop", name),
        status: Status::Ok,
        fired: Vec::new(),
        ok: titles.map(String::from).to_vec(),
        unused: match engine {
            Engine::CloudNativePg => vec!["Postgres backups".into()],
            Engine::MariaDb => vec!["Mysql Galera replication".into()],
        },
        slo: Vec::new(),
        archiving: (engine == Engine::CloudNativePg).then_some(Status::Ok),
        lag: lag
            .iter()
            .map(|(instance, seconds)| {
                (
                    (*instance).to_owned(),
                    Lag {
                        seconds: *seconds,
                        source: LagSource::Coroot,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
        read_at: at,
    }
}

/// What the example Coroot says of `inv`'s databases for a shape.
pub fn coroot(at: DateTime<Utc>, shape: &str, inv: &Inventory) -> Evidence {
    use super::coroot::Absence;
    let uids = || inv.databases.items.iter().map(|db| db.identity.uid.clone());
    match shape {
        "coroot-none" => return Evidence::absent(uids(), Absence::NotLinked),
        "coroot-not-found" => {
            return Evidence {
                states: uids().map(|uid| (uid, CorootState::AppNotFound)).collect(),
            };
        }
        "coroot-no-report" => {
            return Evidence {
                states: uids().map(|uid| (uid, CorootState::NoReport)).collect(),
            };
        }
        "coroot-unreachable" => {
            let healthy = coroot(at, "", inv);
            let mut out = Evidence {
                states: uids()
                    .map(|uid| {
                        (
                            uid,
                            CorootState::Unreachable {
                                why: crate::coroot::ReadError::Unreachable,
                                last: None,
                            },
                        )
                    })
                    .collect(),
            };
            out.retain(&healthy);
            return out;
        }
        _ => {}
    }
    let mut states = BTreeMap::new();
    for db in &inv.databases.items {
        let replicas: Vec<(&str, f64)> = db
            .instances
            .iter()
            .filter(|i| i.role == Fact::Known(Role::Replica))
            .map(|i| (i.name.as_str(), if shape == "lag-high" { 74. } else { 0.4 }))
            .collect();
        let mut found = report(at, db.engine, &db.identity.name, &replicas);
        if shape == "coroot-disagree" && db.engine == Engine::CloudNativePg {
            found.archiving = Some(Status::Critical);
            found.fired.push(Fired {
                title: "Postgres WAL archiving".into(),
                message: "the most recent WAL archive attempt has failed".into(),
                status: Status::Critical,
            });
            found.ok.retain(|title| title != "Postgres WAL archiving");
        }
        // Coroot sees what the operator does not: slow queries on the MariaDB,
        // so the example shows a row the operator calls ready and Coroot
        // does not.
        if db.engine == Engine::MariaDb && (shape.is_empty() || shape == "mariadb") {
            found.fired.push(Fired {
                title: "Mysql latency".into(),
                message: "the p99 latency is 240ms, above the 100ms threshold".into(),
                status: Status::Warning,
            });
            found.ok.retain(|title| title != "Mysql latency");
        }
        if shape == "lag-high" {
            let title = match db.engine {
                Engine::CloudNativePg => "Postgres replication lag",
                Engine::MariaDb => "Mysql replication lag",
            };
            found.fired.push(Fired {
                title: title.into(),
                message: "replication lag is above 30s".into(),
                status: Status::Warning,
            });
            found.ok.retain(|ok| ok != title);
        }
        states.insert(db.identity.uid.clone(), CorootState::Read(Box::new(found)));
    }
    Evidence { states }
}

pub fn shaped(at: DateTime<Utc>, shape: &str) -> Inventory {
    let mut inv = inventory(at);
    match shape {
        "not-installed" => return Inventory::unavailable(Coverage::NotServed, at),
        "refused" => return Inventory::unavailable(Coverage::Refused, at),
        "unsupported" => return Inventory::unavailable(Coverage::Unsupported, at),
        "failed" => return Inventory::unavailable(Coverage::Failed, at),
        "backup-failed" => {
            inv.backups.items[1].state = BackupState::Failed;
        }
        "no-primary" => {
            inv.databases.items[0].current_primary = Fact::Unknown;
            inv.databases.items[0].conditions.clear();
        }
        "stale" => {
            let previous = inv;
            inv = Inventory::unavailable(Coverage::Refused, at);
            inv.retain(&previous);
        }
        "empty" => {
            inv.databases.items.clear();
            inv.backups.items.clear();
            inv.schedules.items.clear();
        }
        "backup-refused" => {
            inv.backups = Read::empty(Coverage::Refused, at);
        }
        "partial" => {
            inv.backups.coverage = Coverage::Partial;
        }
        "old" => {
            inv.backups.items[0].started = Some(at - Duration::hours(27));
            inv.backups.items[0].created = inv.backups.items[0].started;
            inv.backups.items[0].completed = Some(at - Duration::hours(26));
        }
        "no-backup" => {
            inv.backups.items.clear();
        }
        "no-schedule" => {
            inv.schedules.items.clear();
        }
        "suspended" => {
            inv.schedules.items[0].suspended = Fact::Known(true);
        }
        "cert-expiring" => {
            inv.databases.items[0].certificates = Fact::Known(vec![Certificate {
                name: "orders-db-server".into(),
                expires: Some(at + Duration::days(9)),
            }]);
        }
        "switchover-stuck" => {
            let db = &mut inv.databases.items[1];
            db.target_primary = Fact::Known("ledger-db-1".into());
            db.conditions
                .retain(|condition| condition.kind != "PrimarySwitched");
            db.conditions.push(condition(
                "PrimarySwitched",
                "False",
                "SwitchPrimary",
                at - Duration::minutes(25),
            ));
            db.primary_changed = Some(at - Duration::minutes(25));
        }
        // A MariaDB with a schedule whose runs are Jobs the page does not read.
        "scheduled-only" => {
            inv.backups
                .items
                .retain(|b| b.identity.name != "ledger-manual");
        }
        "mariadb" => {
            inv.databases.items.remove(0);
            inv.backups.items.retain(|b| {
                b.cluster
                    .as_ref()
                    .is_some_and(|c| c.engine == Engine::MariaDb)
            });
            inv.schedules.items.remove(0);
            inv.engines[0].1 = Coverage::NotServed;
        }
        _ => {}
    }
    inv
}
