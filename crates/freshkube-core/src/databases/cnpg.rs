//! Allowlisted projection of postgresql.cnpg.io/v1 (upstream release-1.28 schema).
//! Unknown JSON fields, Secret selectors, arbitrary plugin config and output are skipped.
use super::*;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::BTreeMap;

pub(super) const GROUP: &str = "postgresql.cnpg.io";
pub(super) const VERSION: &str = "v1";
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
        self.identity_in(GROUP, VERSION, kind, plural)
    }
    pub(super) fn identity_in(
        &self,
        group: &str,
        version: &str,
        kind: &str,
        plural: &str,
    ) -> Option<Identity> {
        if self.name.is_empty() || self.namespace.is_empty() || self.uid.is_empty() {
            return None;
        }
        Some(Identity {
            group: group.into(),
            version: version.into(),
            kind: kind.into(),
            plural: plural.into(),
            namespace: self.namespace.clone(),
            name: self.name.clone(),
            uid: self.uid.clone(),
        })
    }
    fn reference(&self, cluster: Option<Reference>) -> Option<ClusterRef> {
        Some(self.reference_to(
            Engine::CloudNativePg,
            &format!("{GROUP}/{VERSION}"),
            "Cluster",
            None,
            cluster?.name?,
        ))
    }
    /// A reference to a database by name, carrying its UID only when an owner
    /// reference of the given API version and kind names it.
    pub(super) fn reference_to(
        &self,
        engine: Engine,
        api_version: &str,
        owner_kind: &str,
        namespace: Option<String>,
        name: String,
    ) -> ClusterRef {
        let namespace = namespace.unwrap_or_else(|| self.namespace.clone());
        let uid = self
            .owner_references
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|owner| {
                owner.api_version == api_version
                    && owner.kind == owner_kind
                    && owner.name == name
                    && namespace == self.namespace
            })
            .map(|owner| owner.uid.clone());
        ClusterRef {
            engine,
            namespace,
            name,
            uid,
        }
    }
    pub(super) fn creation(&self) -> Option<DateTime<Utc>> {
        time(self.creation_timestamp.clone())
    }
    pub(super) fn generation(&self) -> Option<i64> {
        self.generation
    }
}
pub(super) fn time(value: Option<String>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.as_deref()?)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}
/// The major version of a PostgreSQL image's tag: `…/postgresql:16.4-12` is 16.
fn major_of(image: Option<&str>) -> Option<String> {
    let tag = image?.split('@').next()?.rsplit_once(':')?.1;
    let major: String = tag.chars().take_while(char::is_ascii_digit).collect();
    (!major.is_empty()).then_some(major)
}
/// Go's `time.Time.String()`: `2026-01-02 15:04:05.123 +0000 UTC`.
fn go_time(text: &str) -> Option<DateTime<Utc>> {
    let mut parts = text.split_whitespace();
    let stamp = format!("{} {} {}", parts.next()?, parts.next()?, parts.next()?);
    DateTime::parse_from_str(&stamp, "%Y-%m-%d %H:%M:%S%.f %z")
        .ok()
        .map(|at| at.with_timezone(&Utc))
}
pub(super) fn named(value: Option<String>) -> Fact<String> {
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
#[serde(default, rename_all = "camelCase")]
struct ClusterSpec {
    instances: Option<u32>,
    image_name: Option<String>,
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
    #[serde(rename = "isWALArchiver")]
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
    image: Option<String>,
    pg_data_image_info: Option<ImageInfo>,
    certificates: Option<RawCertificates>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ImageInfo {
    major_version: Option<u32>,
}
/// Only the expiry map: secret names and dates, never a Secret's data.
#[derive(Default, Deserialize)]
#[serde(default)]
struct RawCertificates {
    expirations: Option<BTreeMap<String, String>>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct RawCondition {
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) status: String,
    pub(super) reason: String,
    pub(super) last_transition_time: Option<String>,
    pub(super) observed_generation: Option<i64>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ReportedInstance {
    is_primary: Option<bool>,
}
pub(super) fn conditions(
    raw: Option<Vec<RawCondition>>,
    generation: Option<i64>,
) -> Vec<Condition> {
    raw.unwrap_or_default()
        .into_iter()
        .map(|condition| Condition {
            kind: condition.kind,
            status: condition.status,
            reason: condition.reason,
            transition_time: time(condition.last_transition_time),
            observed_generation: condition.observed_generation,
            current: generation
                .zip(condition.observed_generation)
                .is_none_or(|(generation, observed)| observed >= generation),
        })
        .collect()
}
impl Cluster {
    pub(super) fn project(self) -> Option<Database> {
        let meta = self.metadata?;
        let identity = meta.identity("Cluster", "clusters")?;
        let spec_reported = self.spec.is_some();
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
                let ready = match &state {
                    Fact::Known(state) if state == "healthy" => Fact::Known(true),
                    Fact::Known(state) if state == "failed" || state == "unhealthy" => {
                        Fact::Known(false)
                    }
                    _ => Fact::Unknown,
                };
                Instance {
                    role,
                    state,
                    ready,
                    ..Instance::named(name)
                }
            })
            .collect();
        let conditions = conditions(status.conditions, meta.generation);
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
        } else if spec_reported {
            Fact::Known(false)
        } else {
            Fact::Unknown
        };
        let version = status
            .pg_data_image_info
            .and_then(|info| info.major_version)
            .map(|major| major.to_string())
            .or_else(|| major_of(status.image.as_deref().or(spec.image_name.as_deref())))
            .into();
        let certificates = match status.certificates {
            Some(RawCertificates {
                expirations: Some(expirations),
            }) => Fact::Known(
                expirations
                    .into_iter()
                    .map(|(name, text)| Certificate {
                        name,
                        expires: go_time(&text),
                    })
                    .collect(),
            ),
            _ => Fact::Unknown,
        };
        Some(Database {
            version,
            proxy: Fact::Unknown,
            certificates,
            primary_changed: None,
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
            Some("failed" | "walArchivingFailing") => BackupState::Failed,
            Some("running" | "started" | "finalizing") => BackupState::Running,
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

/// A Pooler's size, joined to its Cluster by name. `status.instances` counts
/// pods scheduled; the CRD has no ready count and no conditions.
pub(super) struct PoolerFacts {
    pub namespace: String,
    pub cluster: String,
    /// None when the status doesn't report it yet.
    pub running: Option<u32>,
    pub desired: u32,
}
#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct PoolerObject {
    metadata: Option<Meta>,
    spec: Option<PoolerSpec>,
    status: Option<PoolerStatus>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct PoolerSpec {
    cluster: Option<Reference>,
    instances: Option<u32>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct PoolerStatus {
    instances: Option<u32>,
}
impl PoolerObject {
    pub(super) fn project(self) -> Option<PoolerFacts> {
        let meta = self.metadata?;
        let spec = self.spec?;
        Some(PoolerFacts {
            namespace: meta.namespace,
            cluster: spec.cluster?.name?,
            // The CRD defaults `instances` to 1.
            desired: spec.instances.unwrap_or(1),
            running: self.status.and_then(|status| status.instances),
        })
    }
}

/// The proxy fact for a Cluster: the Poolers naming it, added up; Unknown
/// unless the list was read completely and each reports its pods.
pub(super) fn proxy(db: &Database, poolers: &Read<PoolerFacts>) -> Fact<Option<Proxy>> {
    // A Pooler kind the API doesn't serve means there are none.
    if poolers.coverage == Coverage::NotServed {
        return Fact::Known(None);
    }
    if !poolers.complete() {
        return Fact::Unknown;
    }
    let mine: Vec<_> = poolers
        .items
        .iter()
        .filter(|p| p.namespace == db.identity.namespace && p.cluster == db.identity.name)
        .collect();
    if mine.is_empty() {
        return Fact::Known(None);
    }
    let mut running = 0;
    for pooler in &mine {
        match pooler.running {
            Some(count) => running += count,
            None => return Fact::Unknown,
        }
    }
    Fact::Known(Some(Proxy {
        kind: ProxyKind::Pooler,
        running,
        desired: mine.iter().map(|p| p.desired).sum(),
        ready: Fact::Unknown,
    }))
}
