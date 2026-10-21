//! Prepared rows, fields and state sentences; render does no collection work.
use super::*;
const MAX_DETAIL_FIELDS: usize = 128;
use freshkube_core::HasHealth;
use freshkube_core::databases::{BackupState, Coverage, Fact, Identity, LinkState};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Clusters,
    Backups,
    Schedules,
}
impl Mode {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Clusters => "Clusters",
            Self::Backups => "Backups",
            Self::Schedules => "Schedules",
        }
    }
}
pub(crate) struct Row {
    pub key: SharedString,
    pub id: SharedString,
    pub label: SharedString,
    pub identity: Identity,
    pub cells: [SharedString; 6],
    pub tone: ui::Tone,
    pub fields: Vec<(String, String)>,
}
pub(super) struct Display {
    pub rows: Vec<Row>,
    pub summary: SharedString,
    pub notice: Option<SharedString>,
    pub empty: SharedString,
    pub columns: (Vec<table::Column>, f32),
}
impl Default for Display {
    fn default() -> Self {
        Self::new(None, Mode::Clusters)
    }
}
fn fact<T: std::fmt::Display>(value: &Fact<T>) -> String {
    value
        .known()
        .map(ToString::to_string)
        .unwrap_or_else(|| "Unknown".into())
}
fn time(value: Option<chrono::DateTime<Utc>>) -> String {
    value
        .map(|at| {
            format!(
                "{} {}",
                at.with_timezone(&chrono::Local).format("%Y-%m-%d"),
                ui::clock(at.into())
            )
        })
        .unwrap_or_else(|| "Unknown".into())
}
fn reference(value: Option<&databases::ClusterRef>, inventory: &Inventory) -> String {
    match databases::link(value, &inventory.databases) {
        LinkState::Matched => "Cluster UID matches".into(),
        LinkState::NameOnly => "Cluster name matches; incarnation unverified".into(),
        LinkState::Orphaned => "Cluster missing (or replaced)".into(),
        LinkState::Unknown => "Cluster relationship unknown".into(),
    }
}
impl Row {
    fn new(
        identity: &Identity,
        cells: [String; 6],
        tone: ui::Tone,
        mut fields: Vec<(String, String)>,
    ) -> Self {
        if fields.len() > MAX_DETAIL_FIELDS {
            let remaining = fields.len() - MAX_DETAIL_FIELDS;
            fields.truncate(MAX_DETAIL_FIELDS);
            fields.push(("More evidence".into(), format!("{remaining} additional fields are not shown; open the object in Resources for its full status")));
        }
        let key: SharedString = format!(
            "{}/{}/{}/{}",
            identity.group, identity.namespace, identity.kind, identity.uid
        )
        .into();
        let id = format!(
            "databases-row-{}-{}-{}",
            identity.kind, identity.namespace, identity.uid
        )
        .into();
        let label = cells.join(" · ").into();
        Self {
            key,
            id,
            label,
            identity: identity.clone(),
            cells: cells.map(Into::into),
            tone,
            fields,
        }
    }
}
impl Display {
    pub(super) fn new(inventory: Option<&Inventory>, mode: Mode) -> Self {
        let mut rows = Vec::new();
        let mut notices = Vec::new();
        let mut empty = "Not connected".to_owned();
        if let Some(inv) = inventory {
            for (name, coverage, stale, at) in [
                (
                    "Clusters",
                    &inv.databases.coverage,
                    inv.databases.stale,
                    inv.databases.observed_at,
                ),
                (
                    "Backups",
                    &inv.backups.coverage,
                    inv.backups.stale,
                    inv.backups.observed_at,
                ),
                (
                    "Schedules",
                    &inv.schedules.coverage,
                    inv.schedules.stale,
                    inv.schedules.observed_at,
                ),
            ] {
                if *coverage != Coverage::Complete {
                    notices.push(format!(
                        "{name}: {}{}",
                        coverage.label(),
                        if stale {
                            format!(" · last read {}", time(Some(at)))
                        } else {
                            String::new()
                        }
                    ));
                }
            }
            let coverage = match mode {
                Mode::Clusters => &inv.databases.coverage,
                Mode::Backups => &inv.backups.coverage,
                Mode::Schedules => &inv.schedules.coverage,
            };
            empty = if inv.discovery == Coverage::NotServed {
                "CloudNativePG is not installed (API not served)".into()
            } else if *coverage == Coverage::Complete {
                format!("No {} found in this scope", mode.label().to_lowercase())
            } else {
                format!("{}: {} · contents unknown", mode.label(), coverage.label())
            };
            match mode {
                Mode::Clusters => {
                    rows.extend(inv.databases.items.iter().map(|db| database_row(db, inv)))
                }
                Mode::Backups => rows.extend(
                    inv.backups
                        .items
                        .iter()
                        .map(|backup| backup_row(backup, inv)),
                ),
                Mode::Schedules => rows.extend(
                    inv.schedules
                        .items
                        .iter()
                        .map(|schedule| schedule_row(schedule, inv)),
                ),
            }
        }
        rows.sort_by(|a, b| {
            (&a.identity.namespace, &a.identity.name)
                .cmp(&(&b.identity.namespace, &b.identity.name))
        });
        let summary = format!(
            "{} {} · {}",
            rows.len(),
            mode.label().to_lowercase(),
            if notices.is_empty() {
                "read-only"
            } else {
                "coverage incomplete"
            }
        )
        .into();
        let columns = table::columns(mode, &rows);
        Self {
            rows,
            summary,
            notice: (!notices.is_empty()).then(|| notices.join(" · ").into()),
            empty: empty.into(),
            columns,
        }
    }
}

fn database_row(db: &databases::Database, inv: &Inventory) -> Row {
    let found = databases::coverage(db, inv, inv.backups.observed_at);
    let tone = ui::health_tone(if inv.databases.stale {
        freshkube_core::HealthIndicator::Unknown
    } else {
        db.health()
    })
    .0;
    let mut fields = vec![
        ("Phase".into(), fact(&db.phase)),
        ("Current primary".into(), fact(&db.current_primary)),
        ("Target primary".into(), fact(&db.target_primary)),
        (
            "Backup configuration".into(),
            match db.backup_configuration {
                Fact::Known(true) => "Reported configured; external prerequisites unverified",
                Fact::Known(false) => "No backup or WAL archiver configuration reported",
                Fact::Unknown => "Unknown",
            }
            .into(),
        ),
        (
            "Newest observed attempt".into(),
            found
                .newest
                .map(|b| format!("{} · {}", b.identity.name, b.state.label()))
                .unwrap_or_else(|| {
                    if inv.backups.complete() {
                        "None confirmed for this incarnation".into()
                    } else {
                        "Unknown".into()
                    }
                }),
        ),
        (
            "Last completed backup".into(),
            found
                .completed
                .map(|b| format!("{} · {}", b.identity.name, time(b.completed)))
                .unwrap_or_else(|| "Unknown / none confirmed".into()),
        ),
        ("Freshness".into(), found.freshness.label().into()),
        (
            "Schedules".into(),
            if found.no_schedule {
                "No schedule".into()
            } else if !inv.schedules.complete() {
                "Unknown: schedule read is incomplete or stale".into()
            } else {
                format!(
                    "{} confirmed; see Schedules for name references and read coverage",
                    found.schedules.len()
                )
            },
        ),
        ("Restore prerequisites".into(), found.restore.label().into()),
        (
            "Replication".into(),
            "Unknown: no replication measurement read".into(),
        ),
        ("Observed".into(), time(Some(inv.databases.observed_at))),
    ];
    for instance in &db.instances {
        fields.push((
            instance.name.clone(),
            format!(
                "{} · {} · replication Unknown",
                match instance.role {
                    Fact::Known(databases::Role::Primary) => "Primary",
                    Fact::Known(databases::Role::Replica) => "Replica",
                    _ => "Role unknown",
                },
                fact(&instance.state)
            ),
        ));
    }
    for condition in &db.conditions {
        fields.push((
            condition.kind.clone(),
            format!(
                "{} · {} · {} · transition {} · generation {}",
                condition.status,
                condition.reason,
                if condition.current {
                    "current"
                } else {
                    "freshness unknown / outdated"
                },
                time(condition.transition_time),
                condition
                    .observed_generation
                    .map(|generation| generation.to_string())
                    .unwrap_or_else(|| "Not reported".into())
            ),
        ));
    }
    Row::new(
        &db.identity,
        [
            db.identity.name.clone(),
            db.engine.label().into(),
            db.identity.namespace.clone(),
            format!(
                "{} / {} ready",
                fact(&db.ready_instances),
                fact(&db.desired_instances)
            ),
            fact(&db.current_primary),
            found.freshness.label().into(),
        ],
        tone,
        fields,
    )
}

fn backup_row(backup: &databases::Backup, inv: &Inventory) -> Row {
    let relationship = reference(backup.cluster.as_ref(), inv);
    let fields = vec![
        (
            "Cluster".into(),
            backup
                .cluster
                .as_ref()
                .map(|r| r.name.clone())
                .unwrap_or_else(|| "Unknown".into()),
        ),
        ("Relationship".into(), relationship.clone()),
        ("Status".into(), backup.state.label().into()),
        ("Method".into(), fact(&backup.method)),
        ("Started".into(), time(backup.started)),
        ("Finished".into(), time(backup.completed)),
        ("Observed".into(), time(Some(inv.backups.observed_at))),
        (
            "Restore prerequisites".into(),
            if databases::link(backup.cluster.as_ref(), &inv.databases) == LinkState::Orphaned {
                "Cluster is missing or replaced".into()
            } else {
                "Unknown: external prerequisites are unverified".into()
            },
        ),
    ];
    Row::new(
        &backup.identity,
        [
            backup.identity.name.clone(),
            "Backup".into(),
            backup.identity.namespace.clone(),
            backup.state.label().into(),
            fact(&backup.method),
            relationship,
        ],
        match backup.state {
            BackupState::Completed => ui::Tone::Good,
            BackupState::Failed => ui::Tone::Crit,
            _ => ui::Tone::Unknown,
        },
        fields,
    )
}

fn schedule_row(schedule: &databases::Schedule, inv: &Inventory) -> Row {
    let fields = vec![
        (
            "Cluster".into(),
            schedule
                .cluster
                .as_ref()
                .map(|r| r.name.clone())
                .unwrap_or_else(|| "Unknown".into()),
        ),
        (
            "Relationship".into(),
            reference(schedule.cluster.as_ref(), inv),
        ),
        ("Schedule".into(), fact(&schedule.schedule)),
        ("Suspended".into(), fact(&schedule.suspended)),
        ("Last scheduled".into(), time(schedule.last)),
        ("Next scheduled".into(), time(schedule.next)),
        ("Method".into(), fact(&schedule.method)),
        ("Observed".into(), time(Some(inv.schedules.observed_at))),
    ];
    Row::new(
        &schedule.identity,
        [
            schedule.identity.name.clone(),
            "ScheduledBackup".into(),
            schedule.identity.namespace.clone(),
            fact(&schedule.schedule),
            fact(&schedule.suspended),
            reference(schedule.cluster.as_ref(), inv),
        ],
        ui::Tone::Unknown,
        fields,
    )
}
