//! Allowlisted projection of k8s.mariadb.com/v1alpha1 (mariadb-operator
//! v26.10.1, `api/v1alpha1`): MariaDB, Backup and MaxScale. Unknown fields,
//! Secret references, passwords and arbitrary configuration are skipped.
//! The sources of each field are cited in `example.rs`.
use super::cnpg::{Meta, RawCondition, conditions, named};
use super::*;
use serde::Deserialize;
use std::collections::BTreeMap;

pub(super) const GROUP: &str = "k8s.mariadb.com";
pub(super) const VERSION: &str = "v1alpha1";

#[derive(Default, Deserialize)]
#[serde(default)]
struct ObjectRef {
    name: Option<String>,
    namespace: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct MariaDbObject {
    metadata: Option<Meta>,
    spec: Option<MariaDbSpec>,
    status: Option<MariaDbStatus>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct MariaDbSpec {
    image: Option<String>,
    replicas: Option<u32>,
    replication: Option<ReplicationSpec>,
    max_scale_ref: Option<ObjectRef>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct ReplicationSpec {
    primary: Option<PrimarySpec>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct PrimarySpec {
    pod_index: Option<u32>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct MariaDbStatus {
    current_primary: Option<String>,
    default_version: Option<String>,
    replication: Option<ReplicationStatus>,
    conditions: Option<Vec<RawCondition>>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct ReplicationStatus {
    roles: Option<BTreeMap<String, String>>,
    replicas: Option<BTreeMap<String, ReplicaStatus>>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ReplicaStatus {
    #[serde(rename = "slaveIORunning")]
    io_running: Option<bool>,
    #[serde(rename = "slaveSQLRunning")]
    sql_running: Option<bool>,
    seconds_behind_master: Option<f64>,
}

/// The MaxScale a MariaDB names, by namespace and name.
pub(super) type MaxScaleRef = (String, String);

/// A MariaDB with the MaxScale it names, for the join with the MaxScale list.
pub(super) struct Projected {
    pub database: Database,
    pub max_scale: Option<MaxScaleRef>,
}

/// "11.4.2" and "11.4" are "11.4"; "11" is "11"; a digest or `latest` is none.
fn version_of(image: Option<&str>) -> Option<String> {
    let tag = image?.split('@').next()?.rsplit_once(':')?.1;
    let version: String = tag
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mut parts = version.split('.').filter(|part| !part.is_empty());
    let major = parts.next()?;
    Some(match parts.next() {
        Some(minor) => format!("{major}.{minor}"),
        None => major.to_owned(),
    })
}

impl MariaDbObject {
    pub(super) fn project(self) -> Option<Projected> {
        let meta = self.metadata?;
        let identity = meta.identity_in(GROUP, VERSION, "MariaDB", "mariadbs")?;
        let spec = self.spec.unwrap_or_default();
        let status = self.status.unwrap_or_default();
        let replication = status.replication.unwrap_or_default();
        let roles = replication.roles.unwrap_or_default();
        let replicas = replication.replicas.unwrap_or_default();
        let current_primary = named(status.current_primary);
        let mut names: Vec<&String> = roles.keys().chain(replicas.keys()).collect();
        if let Some(primary) = current_primary.known() {
            names.push(primary);
        }
        names.sort();
        names.dedup();
        let instances = names
            .into_iter()
            .map(|name| {
                let role = match roles.get(name).map(String::as_str) {
                    Some("Primary") => Fact::Known(Role::Primary),
                    Some("Replica" | "PrimaryReplica") => Fact::Known(Role::Replica),
                    Some(_) => Fact::Unknown,
                    None if current_primary.known() == Some(name) => Fact::Known(Role::Primary),
                    None => Fact::Unknown,
                };
                let replica = replicas.get(name);
                let running = |value: Option<bool>| match value {
                    Some(true) => "running",
                    Some(false) => "stopped",
                    None => "not reported",
                };
                Instance {
                    role,
                    replication: replica
                        .map(|replica| {
                            Fact::Known(format!(
                                "IO {} · SQL {}",
                                running(replica.io_running),
                                running(replica.sql_running)
                            ))
                        })
                        .unwrap_or(Fact::Unknown),
                    lag: replica
                        .and_then(|replica| replica.seconds_behind_master)
                        .filter(|seconds| seconds.is_finite() && *seconds >= 0.)
                        .map(|seconds| Lag {
                            seconds,
                            source: LagSource::Operator,
                        })
                        .into(),
                    ..Instance::named(name.clone())
                }
            })
            .collect();
        let conditions = conditions(status.conditions, meta.generation());
        // Switching shows as PrimarySwitched=False; its transition is when
        // the operator began. Any other state gives no start to measure from.
        let primary_changed = conditions
            .iter()
            .find(|condition| condition.kind == "PrimarySwitched" && condition.status == "False")
            .and_then(|condition| condition.transition_time);
        let target_primary = spec
            .replication
            .and_then(|replication| replication.primary)
            .and_then(|primary| primary.pod_index)
            .map(|index| format!("{}-{index}", identity.name))
            .into();
        let max_scale = spec.max_scale_ref.and_then(|reference| {
            Some((
                reference
                    .namespace
                    .unwrap_or_else(|| identity.namespace.clone()),
                reference.name?,
            ))
        });
        Some(Projected {
            max_scale,
            database: Database {
                created: meta.creation(),
                identity,
                engine: Engine::MariaDb,
                desired_instances: spec.replicas.into(),
                // The CRD has no ready count.
                ready_instances: Fact::Unknown,
                current_primary,
                target_primary,
                phase: Fact::Unknown,
                instances,
                conditions,
                // Backups name the MariaDB; the MariaDB names no backup.
                backup_configuration: Fact::Unknown,
                version: version_of(spec.image.as_deref())
                    .or_else(|| status.default_version.filter(|v| !v.is_empty()))
                    .into(),
                proxy: Fact::Unknown,
                certificates: Fact::Unknown,
                primary_changed,
            },
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
#[serde(default, rename_all = "camelCase")]
struct BackupSpec {
    maria_db_ref: Option<ObjectRef>,
    schedule: Option<ScheduleSpec>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct ScheduleSpec {
    cron: Option<String>,
    suspend: Option<bool>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct BackupStatus {
    conditions: Option<Vec<RawCondition>>,
}

/// A Backup with a schedule is the recurring definition; without one it is
/// a single run. The operator's status holds no time of a scheduled run.
pub(super) enum Item {
    Run(Backup),
    Schedule(Schedule),
}

impl BackupObject {
    pub(super) fn project(self) -> Option<Item> {
        let meta = self.metadata?;
        let identity = meta.identity_in(GROUP, VERSION, "Backup", "backups")?;
        let spec = self.spec.unwrap_or_default();
        let cluster = spec.maria_db_ref.and_then(|reference| {
            Some(meta.reference_to(
                Engine::MariaDb,
                &format!("{GROUP}/{VERSION}"),
                "MariaDB",
                reference.namespace,
                reference.name?,
            ))
        });
        if let Some(schedule) = spec.schedule {
            return Some(Item::Schedule(Schedule {
                created: meta.creation(),
                identity,
                cluster,
                schedule: named(schedule.cron),
                suspended: Fact::Known(schedule.suspend.unwrap_or(false)),
                method: Fact::Unknown,
                last: None,
                next: None,
            }));
        }
        let conditions = conditions(
            self.status.and_then(|status| status.conditions),
            meta.generation(),
        );
        let complete = conditions
            .iter()
            .find(|condition| condition.kind == "Complete");
        let (state, completed) = match complete.map(|c| (c.status.as_str(), c.reason.as_str())) {
            Some(("True", "JobComplete")) => (
                BackupState::Completed,
                complete.and_then(|c| c.transition_time),
            ),
            // The operator sets Complete=True on a failed Job too.
            Some(("True", "JobFailed")) => (BackupState::Failed, None),
            Some(("False", "JobRunning")) => (BackupState::Running, None),
            Some(("False", "JobSuspended")) => (BackupState::Pending, None),
            _ => (BackupState::Unknown, None),
        };
        Some(Item::Run(Backup {
            identity,
            cluster,
            state,
            method: Fact::Unknown,
            started: None,
            completed,
            created: meta.creation(),
        }))
    }
}
/// A MaxScale's size and readiness, joined to the MariaDB that names it.
pub(super) struct MaxScaleFacts {
    pub namespace: String,
    pub name: String,
    pub running: Option<u32>,
    pub desired: Option<u32>,
    pub ready: Fact<bool>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct MaxScaleObject {
    metadata: Option<Meta>,
    spec: Option<MaxScaleSpec>,
    status: Option<MaxScaleStatus>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct MaxScaleSpec {
    replicas: Option<u32>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct MaxScaleStatus {
    replicas: Option<u32>,
    conditions: Option<Vec<RawCondition>>,
}
impl MaxScaleObject {
    pub(super) fn project(self) -> Option<MaxScaleFacts> {
        let meta = self.metadata?;
        let identity = meta.identity_in(GROUP, VERSION, "MaxScale", "maxscales")?;
        let status = self.status.unwrap_or_default();
        let ready = conditions(status.conditions, meta.generation())
            .iter()
            .find(|condition| condition.kind == "Ready" && condition.current)
            .and_then(|condition| match condition.status.as_str() {
                "True" => Some(true),
                "False" => Some(false),
                _ => None,
            })
            .into();
        Some(MaxScaleFacts {
            namespace: identity.namespace,
            name: identity.name,
            running: status.replicas,
            desired: self.spec.and_then(|spec| spec.replicas),
            ready,
        })
    }
}

/// The proxy fact for a MariaDB: Unknown unless the MaxScale list was read
/// completely and the named MaxScale reports its sizes.
pub(super) fn proxy(
    reference: Option<&MaxScaleRef>,
    listed: &Read<MaxScaleFacts>,
) -> Fact<Option<Proxy>> {
    let Some((namespace, name)) = reference else {
        return if listed.complete() {
            Fact::Known(None)
        } else {
            Fact::Unknown
        };
    };
    let found = listed
        .items
        .iter()
        .find(|facts| facts.namespace == *namespace && facts.name == *name);
    match found {
        Some(MaxScaleFacts {
            running: Some(running),
            desired: Some(desired),
            ready,
            ..
        }) => Fact::Known(Some(Proxy {
            kind: ProxyKind::MaxScale,
            running: *running,
            desired: *desired,
            ready: ready.clone(),
        })),
        _ => Fact::Unknown,
    }
}
