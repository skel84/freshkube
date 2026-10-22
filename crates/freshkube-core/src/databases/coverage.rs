use super::*;
use chrono::{DateTime, Duration, Utc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    Matched,
    NameOnly,
    Orphaned,
    Unknown,
}
/// Never call a reference orphaned after a refused, capped or stale Cluster read.
pub fn link(reference: Option<&ClusterRef>, databases: &Read<Database>) -> LinkState {
    let Some(reference) = reference else {
        return LinkState::Unknown;
    };
    if !databases.complete() {
        return LinkState::Unknown;
    }
    let database = databases.items.iter().find(|db| {
        db.engine == reference.engine
            && db.identity.namespace == reference.namespace
            && db.identity.name == reference.name
    });
    match database {
        None => LinkState::Orphaned,
        Some(db) => match &reference.uid {
            Some(uid) if *uid == db.identity.uid => LinkState::Matched,
            Some(_) => LinkState::Orphaned,
            None => LinkState::NameOnly,
        },
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Old,
    None,
    Unknown,
}
impl Freshness {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fresh => "Within 24 hours",
            Self::Old => "Older than 24 hours",
            Self::None => "No completed backup",
            Self::Unknown => "Backup freshness unknown",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restore {
    NoCompletedBackup,
    NoConfiguration,
    Unknown,
}
impl Restore {
    pub fn label(self) -> &'static str {
        match self {
            Self::NoCompletedBackup => "No completed backup observed",
            Self::NoConfiguration => "No backup configuration reported",
            Self::Unknown => "Unknown: external restore prerequisites are unverified",
        }
    }
}
pub struct BackupCoverage<'a> {
    pub newest: Option<&'a Backup>,
    pub completed: Option<&'a Backup>,
    pub freshness: Freshness,
    pub schedules: Vec<&'a Schedule>,
    pub no_schedule: bool,
    pub restore: Restore,
}
/// Name references are applicable only when the record postdates the current
/// Cluster's creation. Older name-only records cannot cover its replacement.
fn belongs(reference: Option<&ClusterRef>, db: &Database, created: Option<DateTime<Utc>>) -> bool {
    reference.is_some_and(|r| {
        r.engine == db.engine
            && r.namespace == db.identity.namespace
            && r.name == db.identity.name
            && match &r.uid {
                Some(uid) => *uid == db.identity.uid,
                None => db
                    .created
                    .zip(created)
                    .is_some_and(|(cluster, record)| record >= cluster),
            }
    })
}
fn uncertain(
    reference: Option<&ClusterRef>,
    db: &Database,
    created: Option<DateTime<Utc>>,
) -> bool {
    reference.is_none_or(|r| {
        r.engine == db.engine
            && r.namespace == db.identity.namespace
            && r.name == db.identity.name
            && r.uid.is_none()
            && db.created.zip(created).is_none()
    })
}
pub fn coverage<'a>(
    db: &Database,
    inventory: &'a Inventory,
    now: DateTime<Utc>,
) -> BackupCoverage<'a> {
    let backups: Vec<_> = inventory
        .backups
        .items
        .iter()
        .filter(|backup| belongs(backup.cluster.as_ref(), db, backup.created))
        .collect();
    let newest = backups
        .iter()
        .copied()
        .max_by_key(|b| b.started.or(b.created));
    let completed = backups
        .iter()
        .copied()
        .filter(|b| b.state == BackupState::Completed)
        .max_by_key(|b| b.completed);
    let uncertain_backup = inventory
        .backups
        .items
        .iter()
        .any(|backup| uncertain(backup.cluster.as_ref(), db, backup.created));
    let freshness = if !inventory.backups.complete()
        || uncertain_backup
        || backups.iter().any(|backup| {
            backup.state == BackupState::Completed
                && (backup.completed.is_none()
                    || backup
                        .started
                        .zip(backup.completed)
                        .is_some_and(|(start, end)| end < start))
        }) {
        Freshness::Unknown
    } else {
        match completed {
            None => Freshness::None,
            Some(backup) => match backup.completed {
                Some(at) if at <= now && now - at <= Duration::hours(24) => Freshness::Fresh,
                Some(at) if at <= now => Freshness::Old,
                _ => Freshness::Unknown,
            },
        }
    };
    let schedules = inventory
        .schedules
        .items
        .iter()
        .filter(|s| belongs(s.cluster.as_ref(), db, s.created))
        .collect::<Vec<_>>();
    let uncertain_schedule = inventory
        .schedules
        .items
        .iter()
        .any(|schedule| uncertain(schedule.cluster.as_ref(), db, schedule.created));
    // The MariaDB operator records a scheduled Backup's runs as Jobs, which
    // are not read, and its status keeps no time of a run: with a schedule
    // and no run Backup the last good backup is unknown, not absent.
    let freshness =
        if freshness == Freshness::None && db.engine == Engine::MariaDb && !schedules.is_empty() {
            Freshness::Unknown
        } else {
            freshness
        };
    let no_schedule = schedules.is_empty() && inventory.schedules.complete() && !uncertain_schedule;
    let restore = if freshness == Freshness::None {
        Restore::NoCompletedBackup
    } else if db.backup_configuration == Fact::Known(false) && no_schedule {
        Restore::NoConfiguration
    } else {
        Restore::Unknown
    };
    BackupCoverage {
        newest,
        completed,
        freshness,
        schedules,
        no_schedule,
        restore,
    }
}

/// Whether WAL is archived, from the operator's `ContinuousArchiving`
/// condition joined with Coroot's check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Archiving {
    Ok,
    Failing,
    /// The operator and Coroot answer differently.
    Disagree,
    Unknown,
}
impl Archiving {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ok => "WAL archiving working",
            Self::Failing => "WAL archiving failing",
            Self::Disagree => "The operator and Coroot disagree on WAL archiving",
            Self::Unknown => "WAL archiving unknown",
        }
    }
}

/// The operator's side: a current `ContinuousArchiving` condition.
pub(super) fn operator_archiving(db: &Database) -> Option<bool> {
    let condition = db
        .conditions
        .iter()
        .find(|condition| condition.kind == "ContinuousArchiving" && condition.current)?;
    match condition.status.as_str() {
        "True" => Some(true),
        "False" => Some(false),
        _ => None,
    }
}
pub(super) fn coroot_archiving(report: Option<&super::coroot::DatabaseReport>) -> Option<bool> {
    use crate::coroot::Status;
    match report?.archiving? {
        Status::Ok | Status::Info => Some(true),
        Status::Warning | Status::Critical => Some(false),
        Status::Unknown => None,
    }
}
/// Joins the two answers. One alone stands; both must agree to be Ok.
pub fn archiving(db: &Database, coroot: Option<&super::coroot::DatabaseReport>) -> Archiving {
    match (operator_archiving(db), coroot_archiving(coroot)) {
        (Some(a), Some(b)) if a != b => Archiving::Disagree,
        (Some(true), _) | (None, Some(true)) => Archiving::Ok,
        (Some(false), _) | (None, Some(false)) => Archiving::Failing,
        (None, None) => Archiving::Unknown,
    }
}

/// How far a restore can reach: from the oldest completed backup observed
/// to the newest, or to now while WAL is known to be archived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestoreWindow {
    pub from: DateTime<Utc>,
    pub until: DateTime<Utc>,
    /// Reaches the present through archived WAL.
    pub to_now: bool,
}
pub fn restore_window(
    db: &Database,
    inventory: &Inventory,
    found: &BackupCoverage,
    archiving: Archiving,
    now: DateTime<Utc>,
) -> Option<RestoreWindow> {
    let latest = found.completed?.completed?;
    if found.freshness == Freshness::Unknown {
        return None;
    }
    let from = inventory
        .backups
        .items
        .iter()
        .filter(|backup| {
            backup.state == BackupState::Completed
                && belongs(backup.cluster.as_ref(), db, backup.created)
        })
        .filter_map(|backup| backup.completed)
        .min()?;
    let to_now = archiving == Archiving::Ok && latest <= now;
    Some(RestoreWindow {
        from,
        until: if to_now { now } else { latest },
        to_now,
    })
}
