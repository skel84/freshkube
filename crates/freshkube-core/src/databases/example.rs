//! Invented records only; MariaDB exercises the shared model without a live adapter.
use super::*;
use chrono::{DateTime, Duration, Utc};
pub fn inventory(at: DateTime<Utc>) -> Inventory {
    let identity = |kind: &str, plural: &str, name: &str| Identity {
        group: "postgresql.cnpg.io".into(),
        version: "v1".into(),
        kind: kind.into(),
        plural: plural.into(),
        namespace: "shop".into(),
        name: name.into(),
        uid: format!("invented-{name}"),
    };
    let mut db = Database {
        created: Some(at - Duration::days(10)),
        identity: identity("Cluster", "clusters", "orders-db"),
        engine: Engine::CloudNativePg,
        desired_instances: Fact::Known(3),
        ready_instances: Fact::Known(3),
        current_primary: Fact::Known("orders-db-1".into()),
        target_primary: Fact::Known("orders-db-1".into()),
        phase: Fact::Known("Cluster in healthy state".into()),
        backup_configuration: Fact::Known(true),
        conditions: vec![Condition {
            kind: "Ready".into(),
            status: "True".into(),
            reason: "ClusterIsReady".into(),
            transition_time: Some(at - Duration::hours(2)),
            observed_generation: Some(2),
            current: true,
        }],
        instances: vec![],
    };
    db.instances = (1..=3)
        .map(|i| Instance {
            name: format!("orders-db-{i}"),
            role: Fact::Known(if i == 1 { Role::Primary } else { Role::Replica }),
            state: Fact::Known("healthy".into()),
            replication: Fact::Unknown,
        })
        .collect();
    let reference = ClusterRef {
        engine: db.engine,
        namespace: "shop".into(),
        name: "orders-db".into(),
        uid: Some(db.identity.uid.clone()),
    };
    let backup = Backup {
        identity: identity("Backup", "backups", "orders-morning"),
        cluster: Some(reference.clone()),
        state: BackupState::Completed,
        method: Fact::Known("plugin".into()),
        started: Some(at - Duration::hours(2)),
        completed: Some(at - Duration::hours(1)),
        created: Some(at - Duration::hours(2)),
    };
    let mut running = backup.clone();
    running.identity = identity("Backup", "backups", "orders-evening");
    running.state = BackupState::Running;
    running.started = Some(at - Duration::minutes(5));
    running.created = running.started;
    running.completed = None;
    let mut orphan = backup.clone();
    orphan.identity = identity("Backup", "backups", "ledger-old");
    orphan.cluster = Some(ClusterRef {
        engine: Engine::CloudNativePg,
        namespace: "shop".into(),
        name: "ledger-db".into(),
        uid: Some("invented-ledger-old".into()),
    });
    let schedule = Schedule {
        created: Some(at - Duration::days(9)),
        identity: identity("ScheduledBackup", "scheduledbackups", "orders-daily"),
        cluster: Some(reference),
        schedule: Fact::Known("0 0 2 * * *".into()),
        suspended: Fact::Known(false),
        method: Fact::Known("plugin".into()),
        last: Some(at - Duration::hours(2)),
        next: Some(at + Duration::hours(22)),
    };
    let read = |items| Read {
        items,
        coverage: Coverage::Complete,
        observed_at: at,
        stale: false,
    };
    Inventory {
        discovery: Coverage::Complete,
        databases: read(vec![db]),
        backups: Read {
            items: vec![backup, running, orphan],
            coverage: Coverage::Complete,
            observed_at: at,
            stale: false,
        },
        schedules: Read {
            items: vec![schedule],
            coverage: Coverage::Complete,
            observed_at: at,
            stale: false,
        },
    }
}
pub fn mariadb(at: DateTime<Utc>) -> Database {
    let mut db = inventory(at).databases.items.remove(0);
    db.engine = Engine::MariaDb;
    db.identity.group = "k8s.mariadb.com".into();
    db.identity.kind = "MariaDB".into();
    db.identity.plural = "mariadbs".into();
    db.identity.name = "ledger-db".into();
    db.identity.uid = "invented-ledger".into();
    db.current_primary = Fact::Unknown;
    db.target_primary = Fact::Unknown;
    db.instances.clear();
    db.conditions.clear();
    db.backup_configuration = Fact::Unknown;
    db
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
        "mariadb-model" => {
            inv.databases.items.push(mariadb(at));
        }
        _ => {}
    }
    inv
}
