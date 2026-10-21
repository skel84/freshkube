//! Allowlisted projection of postgresql.cnpg.io/v1 (upstream release-1.28 schema).
//! Unknown JSON fields, Secret selectors, arbitrary plugin config and output are skipped.
use super::*;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::BTreeMap;

pub(super) const GROUP: &str = "postgresql.cnpg.io";
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct Meta {
    name: String,
    namespace: String,
    uid: String,
    generation: Option<i64>,
    creation_timestamp: Option<String>,
    owner_references: Option<Vec<Owner>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Owner {
    api_version: String,
    kind: String,
    name: String,
    uid: String,
}
impl Meta {
    fn identity(&self, kind: &str, plural: &str) -> Option<Identity> {
        if self.name.is_empty() || self.namespace.is_empty() || self.uid.is_empty() {
            return None;
        }
        Some(Identity {
            group: GROUP.into(),
            version: "v1".into(),
            kind: kind.into(),
            plural: plural.into(),
            namespace: self.namespace.clone(),
            name: self.name.clone(),
            uid: self.uid.clone(),
        })
    }
    fn reference(&self, cluster: Option<Reference>) -> Option<ClusterRef> {
        let name = cluster?.name?;
        let uid = self
            .owner_references
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|owner| {
                owner.api_version == format!("{GROUP}/v1")
                    && owner.kind == "Cluster"
                    && owner.name == name
            })
            .map(|owner| owner.uid.clone());
        Some(ClusterRef {
            engine: Engine::CloudNativePg,
            namespace: self.namespace.clone(),
            name,
            uid,
        })
    }
}
fn time(value: Option<String>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.as_deref()?)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}
fn named(value: Option<String>) -> Fact<String> {
    value
        .filter(|value| !value.is_empty() && value != "-" && value != "pending")
        .into()
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Reference {
    name: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct Cluster {
    metadata: Option<Meta>,
    spec: Option<ClusterSpec>,
    status: Option<ClusterStatus>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct ClusterSpec {
    instances: Option<u32>,
    backup: Option<BackupConfig>,
    plugins: Option<Vec<Plugin>>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct BackupConfig {
    barman_object_store: Option<serde::de::IgnoredAny>,
    volume_snapshot: Option<serde::de::IgnoredAny>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Plugin {
    is_wal_archiver: Option<bool>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ClusterStatus {
    ready_instances: Option<u32>,
    current_primary: Option<String>,
    target_primary: Option<String>,
    phase: Option<String>,
    instances_status: Option<BTreeMap<String, Vec<String>>>,
    instances_reported_state: Option<BTreeMap<String, ReportedInstance>>,
    conditions: Option<Vec<RawCondition>>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct RawCondition {
    #[serde(rename = "type")]
    kind: String,
    status: String,
    reason: String,
    last_transition_time: Option<String>,
    observed_generation: Option<i64>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ReportedInstance {
    is_primary: Option<bool>,
}
impl Cluster {
    pub(super) fn project(self) -> Option<Database> {
        let meta = self.metadata?;
        let identity = meta.identity("Cluster", "clusters")?;
        let spec = self.spec.unwrap_or_default();
        let status = self.status.unwrap_or_default();
        let current_primary = named(status.current_primary);
        let mut states: BTreeMap<String, Fact<String>> = status
            .instances_status
            .unwrap_or_default()
            .into_iter()
            .flat_map(|(state, names)| {
                names
                    .into_iter()
                    .map(move |name| (name, Fact::Known(state.clone())))
            })
            .collect();
        let reported = status.instances_reported_state.unwrap_or_default();
        for name in reported.keys() {
            states.entry(name.clone()).or_insert(Fact::Unknown);
        }
        let instances = states
            .into_iter()
            .map(|(name, state)| {
                let role = match reported.get(&name).and_then(|instance| instance.is_primary) {
                    Some(true) => Fact::Known(Role::Primary),
                    Some(false) => Fact::Known(Role::Replica),
                    None if current_primary.known() == Some(&name) => Fact::Known(Role::Primary),
                    None => Fact::Unknown,
                };
                Instance {
                    name,
                    role,
                    state,
                    replication: Fact::Unknown,
                }
            })
            .collect();
        let conditions = status
            .conditions
            .unwrap_or_default()
            .into_iter()
            .map(|condition| Condition {
                kind: condition.kind,
                status: condition.status,
                reason: condition.reason,
                transition_time: time(condition.last_transition_time),
                observed_generation: condition.observed_generation,
                current: meta
                    .generation
                    .zip(condition.observed_generation)
                    .is_some_and(|(generation, observed)| observed >= generation),
            })
            .collect();
        let backup_configuration = if spec
            .backup
            .as_ref()
            .is_some_and(|b| b.barman_object_store.is_some() || b.volume_snapshot.is_some())
            || spec.plugins.as_ref().is_some_and(|plugins| {
                plugins
                    .iter()
                    .any(|plugin| plugin.is_wal_archiver == Some(true))
            }) {
            Fact::Known(true)
        } else {
            Fact::Unknown
        };
        Some(Database {
            created: time(meta.creation_timestamp),
            identity,
            engine: Engine::CloudNativePg,
            desired_instances: spec.instances.into(),
            ready_instances: status.ready_instances.into(),
            current_primary,
            target_primary: named(status.target_primary),
            phase: named(status.phase),
            instances,
            conditions,
            backup_configuration,
        })
    }
}
#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct BackupObject {
    metadata: Option<Meta>,
    spec: Option<BackupSpec>,
    status: Option<BackupStatus>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct BackupSpec {
    cluster: Option<Reference>,
    method: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct BackupStatus {
    phase: Option<String>,
    method: Option<String>,
    started_at: Option<String>,
    stopped_at: Option<String>,
}
impl BackupObject {
    pub(super) fn project(self) -> Option<Backup> {
        let meta = self.metadata?;
        let spec = self.spec.unwrap_or_default();
        let status = self.status.unwrap_or_default();
        let state = match status.phase.as_deref() {
            Some("completed") => BackupState::Completed,
            Some("failed") => BackupState::Failed,
            Some("running" | "started" | "walArchiving") => BackupState::Running,
            Some("pending") => BackupState::Pending,
            _ => BackupState::Unknown,
        };
        Some(Backup {
            identity: meta.identity("Backup", "backups")?,
            cluster: meta.reference(spec.cluster),
            state,
            method: named(status.method.or(spec.method)),
            started: time(status.started_at),
            completed: time(status.stopped_at),
            created: time(meta.creation_timestamp),
        })
    }
}
#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct ScheduleObject {
    metadata: Option<Meta>,
    spec: Option<ScheduleSpec>,
    status: Option<ScheduleStatus>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct ScheduleSpec {
    cluster: Option<Reference>,
    schedule: Option<String>,
    suspend: Option<bool>,
    method: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ScheduleStatus {
    last_schedule_time: Option<String>,
    next_schedule_time: Option<String>,
}
impl ScheduleObject {
    pub(super) fn project(self) -> Option<Schedule> {
        let meta = self.metadata?;
        let spec = self.spec.unwrap_or_default();
        let status = self.status.unwrap_or_default();
        Some(Schedule {
            created: time(meta.creation_timestamp.clone()),
            identity: meta.identity("ScheduledBackup", "scheduledbackups")?,
            cluster: meta.reference(spec.cluster),
            schedule: named(spec.schedule),
            suspended: spec.suspend.into(),
            method: named(spec.method),
            last: time(status.last_schedule_time),
            next: time(status.next_schedule_time),
        })
    }
}
