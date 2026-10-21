//! The Backups and Schedules views: the objects themselves, so an orphan
//! stays inspectable.
use super::format::{cron, tone_of};
use super::*;
use freshkube_core::HealthIndicator;
use freshkube_core::databases::{self, Backup, BackupState, ClusterRef, LinkState, Schedule};

fn reference(value: Option<&ClusterRef>, inventory: &Inventory) -> &'static str {
    match databases::link(value, &inventory.databases) {
        LinkState::Matched => "Cluster UID matches",
        LinkState::NameOnly => "Cluster name matches; incarnation unverified",
        LinkState::Orphaned => "Cluster missing (or replaced)",
        LinkState::Unknown => "Cluster relationship unknown",
    }
}

fn database_name(value: Option<&ClusterRef>) -> String {
    value.map_or_else(|| "Unknown".into(), |r| r.name.clone())
}

pub(super) fn backups(inv: &Inventory) -> Vec<Row> {
    inv.backups.items.iter().map(|b| backup(b, inv)).collect()
}

fn backup(backup: &Backup, inv: &Inventory) -> Row {
    let relationship = reference(backup.cluster.as_ref(), inv);
    let orphan = databases::orphan(backup.cluster.as_ref(), inv);
    let operator = backup
        .cluster
        .as_ref()
        .map_or("Unknown", |r| r.engine.label());
    let (key, id) = identity_key(&backup.identity);
    let mut tone = tone_of(freshkube_core::HasHealth::health(&backup.state));
    let mut headline = backup.state.label().to_string();
    let mut why = Vec::new();
    let mut next = Vec::new();
    if let Some(finding) = &orphan {
        // A completed backup of a missing database is not a pass.
        tone = ui::Tone::Warn;
        headline = finding.why.clone();
        why.push(finding.why.clone().into());
        next.extend(finding.next.clone().map(SharedString::from));
    } else if backup.state == BackupState::Failed {
        why.push("The backup attempt failed".into());
    }
    let fields = vec![
        Field::prose("Database", database_name(backup.cluster.as_ref())),
        Field::prose("Relationship", relationship),
        Field::prose("Status", backup.state.label()),
        Field::fact("Method", &backup.method),
        Field::time("Started", backup.started),
        Field::time("Finished", backup.completed),
        Field::time("Observed", Some(inv.backups.observed_at)),
        Field::prose(
            "Restore prerequisites",
            if orphan.is_some() {
                "Cluster is missing or replaced"
            } else {
                "Unknown: external prerequisites are unverified"
            },
        ),
        Field::prose("Operator", operator),
    ];
    let finished = backup
        .completed
        .map_or_else(|| "—".to_owned(), |at| time(Some(at)));
    Row {
        key,
        id,
        label: format!("{} · {}", backup.identity.name, backup.state.label()).into(),
        identity: backup.identity.clone(),
        cells: vec![
            backup.identity.name.clone().into(),
            database_name(backup.cluster.as_ref()).into(),
            backup.state.label().into(),
            finished.into(),
        ],
        tips: vec![None, Some(relationship.into()), None, None],
        tone,
        chip: None,
        group: None,
        search: format!(
            "{} {} {} {}",
            backup.identity.name,
            backup.identity.namespace,
            database_name(backup.cluster.as_ref()),
            backup.state.label()
        )
        .to_lowercase(),
        engine: operator.into(),
        verdict: Verdict {
            tone,
            headline: headline.into(),
            source: None,
            why,
            next,
        },
        sections: vec![Section::new("backup", "Backup", fields)],
        observe: None,
    }
    .finish()
}

pub(super) fn schedules(inv: &Inventory) -> Vec<Row> {
    inv.schedules
        .items
        .iter()
        .map(|s| schedule(s, inv))
        .collect()
}

fn schedule(schedule: &Schedule, inv: &Inventory) -> Row {
    let engine = schedule
        .cluster
        .as_ref()
        .map_or(databases::Engine::CloudNativePg, |r| r.engine);
    let (words, raw) = cron(engine, &schedule.schedule);
    let state = match schedule.suspended {
        Fact::Known(true) => "Suspended",
        Fact::Known(false) => "Active",
        Fact::Unknown => "Unknown",
    };
    let tone = match schedule.suspended {
        Fact::Known(false) => tone_of(HealthIndicator::Healthy),
        _ => ui::Tone::Unknown,
    };
    let relationship = reference(schedule.cluster.as_ref(), inv);
    let mut fields = vec![
        Field::prose("Database", database_name(schedule.cluster.as_ref())),
        Field::prose("Relationship", relationship),
        Field::prose("When", words.clone()),
    ];
    if let Some(raw) = &raw {
        fields.push(Field::mono("Expression", raw.clone()));
    }
    fields.extend([
        Field::prose("State", state),
        Field::time("Last scheduled", schedule.last),
        Field::time("Next scheduled", schedule.next),
        Field::fact("Method", &schedule.method),
        Field::time("Observed", Some(inv.schedules.observed_at)),
    ]);
    let (key, id) = identity_key(&schedule.identity);
    Row {
        key,
        id,
        label: format!("{} · {state} · {words}", schedule.identity.name).into(),
        identity: schedule.identity.clone(),
        cells: vec![
            schedule.identity.name.clone().into(),
            database_name(schedule.cluster.as_ref()).into(),
            words.clone().into(),
            state.into(),
        ],
        tips: vec![None, Some(relationship.into()), raw.map(Into::into), None],
        tone,
        chip: None,
        group: None,
        search: format!(
            "{} {} {} {state}",
            schedule.identity.name,
            schedule.identity.namespace,
            database_name(schedule.cluster.as_ref())
        )
        .to_lowercase(),
        engine: engine.label().into(),
        verdict: Verdict {
            tone,
            headline: format!("{state} · {words}").into(),
            source: None,
            why: Vec::new(),
            next: Vec::new(),
        },
        sections: vec![Section::new("schedule", "Schedule", fields)],
        observe: None,
    }
    .finish()
}
