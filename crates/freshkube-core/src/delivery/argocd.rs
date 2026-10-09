//! Argo CD Applications: sync, health, destination, the Kargo stage that is
//! allowed to sync one, and the objects it manages.

use std::collections::BTreeMap;

use serde_json::Value;

use super::digest::text;
use super::observation::{Meta, ObjectRef};
use super::read::{ListRequest, Reader, Scope};
use super::source::{Source, Truncation, redact_message};
use super::versions::resolve;
use crate::resources::Failure;

pub const GROUP: &str = "argoproj.io";
const VERSIONS: &[&str] = &["v1alpha1"];
/// The annotation Kargo reads to know which Stage may sync an Application:
/// `<project>:<stage>`.
pub const AUTHORIZED_STAGE: &str = "kargo.akuity.io/authorized-stage";

/// How an Application ties itself to a Kargo stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageClaim {
    Annotation,
    NamedByConvention,
}

impl StageClaim {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Annotation => "authorized-stage annotation",
            Self::NamedByConvention => {
                "named by the configured template, with the Kargo project under the configured key; no authorized-stage annotation"
            }
        }
    }
}

/// How a setup that doesn't set the authorized-stage annotation ties an
/// Application to a Kargo stage, written by whoever runs the join: an
/// annotation the setup names (or a label of that key) on the Application,
/// holding the Kargo project, and a name template using `{project}` and
/// `{stage}`. Off unless configured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageNaming {
    pub project_key: String,
    pub name_template: String,
}

impl StageNaming {
    pub fn name_for(&self, project: &str, stage: &str) -> String {
        self.name_template
            .replace("{project}", project)
            .replace("{stage}", stage)
    }
}

/// One entry of `status.resources`, the objects Argo CD manages for an
/// Application, with what it last compared about each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedObject {
    pub group: String,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
    pub version: Option<String>,
    /// `Synced`, `OutOfSync` or `Unknown`, as Argo CD writes it.
    pub sync: Option<String>,
    pub health: Option<String>,
    /// The health message, redacted.
    pub health_message: Option<String>,
}

/// `spec.syncPolicy`: what the Application asks of Argo CD. Configuration
/// alone says nothing about what Argo CD is doing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncPolicy {
    /// `syncPolicy.automated`, when the block is present.
    pub automated: Option<Automated>,
    /// `syncPolicy.retry`, when the block is present.
    pub retry: Option<RetryPolicy>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Automated {
    /// `enabled`, which newer Argo CD versions read; absent means enabled.
    pub enabled: Option<bool>,
    pub prune: Option<bool>,
    pub self_heal: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetryPolicy {
    /// `limit` as written. What a zero or negative limit means is Argo CD's
    /// business; it is reported, never interpreted as "unlimited".
    pub limit: Option<i64>,
    pub backoff: Option<Backoff>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Backoff {
    pub duration: Option<String>,
    pub factor: Option<i64>,
    pub max_duration: Option<String>,
}

/// `status.operationState`: the last or running sync operation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Operation {
    /// `Running`, `Terminating`, `Failed`, `Error` or `Succeeded`.
    pub phase: Option<String>,
    /// The operation's message, redacted.
    pub message: Option<String>,
    pub retry_count: Option<i64>,
    /// `syncResult.revisions`, else `syncResult.revision`; without a sync
    /// result (an operation that failed before it synced), the revisions
    /// the operation asked for, `operation.sync`.
    pub revisions: Vec<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

/// One source of an Application. Its `repoURL` is not kept: it may hold a
/// host or credentials, and nothing here prints it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplicationSource {
    pub target_revision: Option<String>,
    pub path: Option<String>,
    pub chart: Option<String>,
    /// `ref`, the name other sources use for a values-only source.
    pub reference: Option<String>,
}

/// `spec.source`, or `spec.sources` when that is set, as Argo CD reads them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sources {
    Single(ApplicationSource),
    Multiple(Vec<ApplicationSource>),
}

impl Sources {
    pub fn as_slice(&self) -> &[ApplicationSource] {
        match self {
            Self::Single(source) => std::slice::from_ref(source),
            Self::Multiple(sources) => sources,
        }
    }
}

/// The last entry of `status.history`: what Argo CD last deployed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Deployment {
    /// `revisions`, else `revision`.
    pub revisions: Vec<String>,
    pub deployed_at: Option<String>,
}

/// One of `status.conditions`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Condition {
    pub kind: String,
    /// The condition's message, redacted.
    pub message: Option<String>,
}

/// The three revisions an Application names: what its spec asks for, what
/// Argo CD last compared the cluster against, and what it last deployed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Revisions {
    /// Each source's `targetRevision`, in the sources' order; `None` where a
    /// source names none. Empty when the spec has no source.
    pub requested: Vec<Option<String>>,
    /// `status.sync.revisions`, else `status.sync.revision`.
    pub compared: Vec<String>,
    /// The last history entry's revisions.
    pub deployed: Vec<String>,
}

/// Whether the Application asks Argo CD to sync it on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoSync {
    /// No `automated` block: syncs only when someone asks.
    Manual,
    /// An `automated` block with `enabled: false`.
    Disabled,
    /// An `automated` block, enabled. Each flag is true only when the spec
    /// says so.
    Enabled { prune: bool, self_heal: bool },
}

/// What the operation state shows of retries. Only an operation's own
/// `retryCount` and phase say a retry happened; a retry policy alone says
/// none did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Retries {
    /// No operation reports a retry.
    NoneReported,
    /// The running operation has retried `count` times.
    Retrying { count: i64 },
    /// The operation ended `Failed` or `Error` after `count` retries, as
    /// many as the configured positive limit.
    Exhausted { count: i64 },
    /// The operation ended in `phase` after `count` retries, short of a
    /// positive limit or without one known.
    Ended { count: i64, phase: Option<String> },
}

/// Counts over `status.resources`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InventoryCounts {
    pub objects: usize,
    pub out_of_sync: usize,
    pub degraded: usize,
    pub missing: usize,
}

/// What an Application reports about reconciling itself, ready to be put
/// into words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reconciliation {
    pub auto_sync: AutoSync,
    /// The configured retry limit, as written; `None` without a retry block
    /// or a limit in it.
    pub retry_limit: Option<i64>,
    /// What the operation shows of retries. Read it with
    /// [`Reconciliation::superseded`]: the operation state is the last
    /// operation's, and stays after it ends.
    pub retries: Retries,
    /// The last operation's phase.
    pub phase: Option<String>,
    /// When the last operation finished; `None` while it runs.
    pub operation_finished_at: Option<String>,
    /// The revisions the last operation synced.
    pub operation_revisions: Vec<String>,
    /// The last operation has ended, and Argo CD has since compared the
    /// cluster against other revisions: its phase and retries are history,
    /// not the current state.
    pub superseded: bool,
    pub multiple_sources: bool,
    pub revisions: Revisions,
    /// `None` when Argo CD reported no inventory: unknown, not empty.
    pub inventory: Option<InventoryCounts>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub namespace: String,
    pub name: String,
    /// `metadata.uid` and `metadata.resourceVersion`.
    pub meta: Meta,
    pub project: Option<String>,
    /// `spec.destination.server`, when the destination is given by URL.
    pub destination_server: Option<String>,
    /// `spec.destination.name`, when given by cluster name (a cluster Secret
    /// the app does not read).
    pub destination_name: Option<String>,
    pub destination_namespace: Option<String>,
    pub sync: Option<String>,
    /// `status.sync.revision` followed by `status.sync.revisions`, as the
    /// join matches them against a Promotion's pushed commit: a commit
    /// listed in either field counts. It is not `compared_revisions`, which
    /// reads one field or the other the way an operation's and a history
    /// entry's revisions are read; to compare with those, use
    /// `compared_revisions`.
    pub sync_revisions: Vec<String>,
    /// The pointer each of `sync_revisions` was read from.
    pub sync_revisions_at: Vec<&'static str>,
    pub health: Option<String>,
    /// `<project>:<stage>` from the Kargo annotation, a claim.
    pub authorized_stage: Option<(String, String)>,
    /// The Application's own `metadata.annotations` and `metadata.labels`,
    /// where a setup may name the Kargo project.
    pub metadata_annotations: BTreeMap<String, String>,
    pub metadata_labels: BTreeMap<String, String>,
    /// The name of the ApplicationSet that generated it, by an owner
    /// reference of kind `ApplicationSet` in this API group.
    pub owner_application_set: Option<String>,
    /// `status.resources`; empty both when Argo CD lists none and when it
    /// reported no inventory, which [`Application::inventory`] tells apart.
    pub managed: Vec<ManagedObject>,
    /// Whether `status.resources` was there at all.
    pub inventory_reported: bool,
    /// `status.sync.revisions`, else `status.sync.revision`, read as the
    /// operation's and history's revisions are.
    pub compared_revisions: Vec<String>,
    /// Images Argo CD summarises, shown only.
    pub images: Vec<String>,
    pub sync_policy: Option<SyncPolicy>,
    pub operation: Option<Operation>,
    pub sources: Option<Sources>,
    pub last_deployed: Option<Deployment>,
    pub conditions: Vec<Condition>,
}

pub fn parse_application(value: &Value) -> Option<Application> {
    let mut found: Vec<(String, &'static str)> = Vec::new();
    if let Some(revision) = text(value, "/status/sync/revision") {
        found.push((revision, "/status/sync/revision"));
    }
    found.extend(
        value
            .pointer("/status/sync/revisions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|revision| (revision.to_owned(), "/status/sync/revisions")),
    );
    found.dedup_by(|a, b| a.0 == b.0);
    let (revisions, revisions_at): (Vec<String>, Vec<&'static str>) = found.into_iter().unzip();
    let metadata_annotations = strings(value, "/metadata/annotations");
    let authorized_stage = metadata_annotations
        .get(AUTHORIZED_STAGE)
        .and_then(|stage| stage.split_once(':'))
        .map(|(project, stage)| (project.to_owned(), stage.to_owned()));
    Some(Application {
        namespace: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        meta: Meta::parse(value),
        project: text(value, "/spec/project"),
        destination_server: text(value, "/spec/destination/server"),
        destination_name: text(value, "/spec/destination/name"),
        destination_namespace: text(value, "/spec/destination/namespace"),
        sync: text(value, "/status/sync/status"),
        sync_revisions: revisions,
        sync_revisions_at: revisions_at,
        health: text(value, "/status/health/status"),
        authorized_stage,
        metadata_annotations,
        metadata_labels: strings(value, "/metadata/labels"),
        owner_application_set: owning_application_set(value),
        managed: value
            .pointer("/status/resources")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(parse_managed)
            .collect(),
        inventory_reported: value
            .pointer("/status/resources")
            .is_some_and(Value::is_array),
        compared_revisions: object(value, "/status/sync")
            .map(revisions_of)
            .unwrap_or_default(),
        images: value
            .pointer("/status/summary/images")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        sync_policy: object(value, "/spec/syncPolicy").map(parse_sync_policy),
        operation: object(value, "/status/operationState").map(parse_operation),
        sources: parse_sources(value),
        last_deployed: value
            .pointer("/status/history")
            .and_then(Value::as_array)
            .and_then(|history| history.last())
            .map(|entry| Deployment {
                revisions: revisions_of(entry),
                deployed_at: text(entry, "/deployedAt"),
            }),
        conditions: value
            .pointer("/status/conditions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|condition| {
                Some(Condition {
                    kind: text(condition, "/type")?,
                    message: redacted(condition, "/message"),
                })
            })
            .collect(),
    })
}

fn parse_managed(resource: &Value) -> Option<ManagedObject> {
    Some(ManagedObject {
        group: text(resource, "/group").unwrap_or_default(),
        kind: text(resource, "/kind")?,
        namespace: text(resource, "/namespace"),
        name: text(resource, "/name")?,
        version: text(resource, "/version"),
        sync: text(resource, "/status"),
        health: text(resource, "/health/status"),
        health_message: redacted(resource, "/health/message"),
    })
}

fn parse_sync_policy(policy: &Value) -> SyncPolicy {
    SyncPolicy {
        automated: object(policy, "/automated").map(|automated| Automated {
            enabled: flag(automated, "/enabled"),
            prune: flag(automated, "/prune"),
            self_heal: flag(automated, "/selfHeal"),
        }),
        retry: object(policy, "/retry").map(|retry| RetryPolicy {
            limit: number(retry, "/limit"),
            backoff: object(retry, "/backoff").map(|backoff| Backoff {
                duration: text(backoff, "/duration"),
                factor: number(backoff, "/factor"),
                max_duration: text(backoff, "/maxDuration"),
            }),
        }),
    }
}

fn parse_operation(state: &Value) -> Operation {
    Operation {
        phase: text(state, "/phase"),
        message: redacted(state, "/message"),
        retry_count: number(state, "/retryCount"),
        revisions: [
            object(state, "/syncResult"),
            object(state, "/operation/sync"),
        ]
        .into_iter()
        .flatten()
        .map(revisions_of)
        .find(|revisions| !revisions.is_empty())
        .unwrap_or_default(),
        started_at: text(state, "/startedAt"),
        finished_at: text(state, "/finishedAt"),
    }
}

/// `spec.sources` when it lists any, as Argo CD prefers it, else
/// `spec.source`.
fn parse_sources(value: &Value) -> Option<Sources> {
    let source = |source: &Value| ApplicationSource {
        target_revision: text(source, "/targetRevision"),
        path: text(source, "/path"),
        chart: text(source, "/chart"),
        reference: text(source, "/ref"),
    };
    match value.pointer("/spec/sources").and_then(Value::as_array) {
        Some(sources) if !sources.is_empty() => {
            Some(Sources::Multiple(sources.iter().map(source).collect()))
        }
        _ => value
            .pointer("/spec/source")
            .filter(|source| source.is_object())
            .map(|found| Sources::Single(source(found))),
    }
}

/// `revisions` where it lists any, else `revision`, of a sync status, sync
/// result or history entry.
fn revisions_of(value: &Value) -> Vec<String> {
    let many: Vec<String> = value
        .get("revisions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|revision| !revision.is_empty())
        .map(str::to_owned)
        .collect();
    if many.is_empty() {
        text(value, "/revision").into_iter().collect()
    } else {
        many
    }
}

/// The object at `pointer`; a null, as a block left empty in YAML reads, is
/// no block, as Argo CD reads it.
fn object<'a>(value: &'a Value, pointer: &str) -> Option<&'a Value> {
    value.pointer(pointer).filter(|found| found.is_object())
}

fn redacted(value: &Value, pointer: &str) -> Option<String> {
    text(value, pointer).map(|message| redact_message(&message))
}

fn flag(value: &Value, pointer: &str) -> Option<bool> {
    value.pointer(pointer).and_then(Value::as_bool)
}

fn number(value: &Value, pointer: &str) -> Option<i64> {
    value.pointer(pointer).and_then(Value::as_i64)
}

/// The ApplicationSet an owner reference names.
fn owning_application_set(value: &Value) -> Option<String> {
    value
        .pointer("/metadata/ownerReferences")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|owner| {
            text(owner, "/kind").as_deref() == Some("ApplicationSet")
                && text(owner, "/apiVersion").is_some_and(|version| {
                    version
                        .split_once('/')
                        .is_some_and(|(group, _)| group == GROUP)
                })
        })
        .and_then(|owner| text(owner, "/name"))
}

/// The string values of the object at `pointer`.
fn strings(value: &Value, pointer: &str) -> BTreeMap<String, String> {
    value
        .pointer(pointer)
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
        .collect()
}

impl Application {
    /// The object these facts were read from.
    pub fn object_ref(&self) -> ObjectRef {
        ObjectRef::new(
            GROUP,
            "Application",
            Some(&self.namespace),
            &self.name,
            &self.meta,
        )
    }

    /// The Kargo project under the setup's key: the annotation, else the
    /// label.
    fn named_project(&self, naming: &StageNaming) -> Option<&str> {
        self.metadata_annotations
            .get(&naming.project_key)
            .or_else(|| self.metadata_labels.get(&naming.project_key))
            .map(String::as_str)
    }

    /// How this Application claims the Kargo stage `project:stage`, if it
    /// does. The annotation names the stage outright. Where `naming` is
    /// configured, an Application whose annotation names the project and whose
    /// name is the template's is a weaker claim, reported as such.
    pub fn claims_stage(
        &self,
        project: &str,
        stage: &str,
        naming: Option<&StageNaming>,
    ) -> Option<StageClaim> {
        match &self.authorized_stage {
            Some((p, s)) if p == project && s == stage => Some(StageClaim::Annotation),
            Some(_) => None,
            None => {
                let naming = naming?;
                (self.named_project(naming) == Some(project)
                    && self.name == naming.name_for(project, stage))
                .then_some(StageClaim::NamedByConvention)
            }
        }
    }

    /// Whether the Application is of the Kargo project, by the authorized-stage
    /// annotation or under the configured key.
    pub fn is_of_project(&self, project: &str, naming: Option<&StageNaming>) -> bool {
        self.authorized_stage
            .as_ref()
            .is_some_and(|(p, _)| p == project)
            || naming.is_some_and(|naming| self.named_project(naming) == Some(project))
    }

    /// Digests of the images Argo CD's summary lists, which it writes as
    /// `repo@sha256:…` once an image is pinned.
    pub fn digests(&self) -> Vec<super::digest::Digest> {
        self.images
            .iter()
            .filter_map(|image| super::digest::Digest::from_reference(image))
            .collect()
    }

    /// The managed objects, or `None` when Argo CD reported no inventory.
    pub fn inventory(&self) -> Option<&[ManagedObject]> {
        self.inventory_reported.then_some(self.managed.as_slice())
    }

    /// The managed object of exactly this group, kind, namespace and name;
    /// a namesake in another group is another object.
    pub fn managed_object(
        &self,
        group: &str,
        kind: &str,
        namespace: Option<&str>,
        name: &str,
    ) -> Option<&ManagedObject> {
        self.managed.iter().find(|object| {
            object.group == group
                && object.kind == kind
                && object.namespace.as_deref() == namespace
                && object.name == name
        })
    }

    pub fn revisions(&self) -> Revisions {
        Revisions {
            requested: self
                .sources
                .iter()
                .flat_map(Sources::as_slice)
                .map(|source| source.target_revision.clone())
                .collect(),
            compared: self.compared_revisions.clone(),
            deployed: self
                .last_deployed
                .as_ref()
                .map(|deployment| deployment.revisions.clone())
                .unwrap_or_default(),
        }
    }

    pub fn reconciliation(&self) -> Reconciliation {
        let automated = self.sync_policy.as_ref().and_then(|p| p.automated.as_ref());
        let auto_sync = match automated {
            None => AutoSync::Manual,
            Some(automated) if automated.enabled == Some(false) => AutoSync::Disabled,
            Some(automated) => AutoSync::Enabled {
                prune: automated.prune == Some(true),
                self_heal: automated.self_heal == Some(true),
            },
        };
        let retry_limit = self
            .sync_policy
            .as_ref()
            .and_then(|policy| policy.retry.as_ref())
            .and_then(|retry| retry.limit);
        let operation = self.operation.as_ref();
        Reconciliation {
            auto_sync,
            retry_limit,
            retries: retries(operation, retry_limit),
            phase: operation.and_then(|op| op.phase.clone()),
            operation_finished_at: operation.and_then(|op| op.finished_at.clone()),
            operation_revisions: operation.map(|op| op.revisions.clone()).unwrap_or_default(),
            superseded: operation.is_some_and(|op| {
                op.has_ended()
                    && !op.revisions.is_empty()
                    && !self.compared_revisions.is_empty()
                    && op.revisions != self.compared_revisions
            }),
            multiple_sources: matches!(self.sources, Some(Sources::Multiple(_))),
            revisions: self.revisions(),
            inventory: self.inventory().map(|objects| InventoryCounts {
                objects: objects.len(),
                out_of_sync: count(objects, |o| o.sync.as_deref() == Some("OutOfSync")),
                degraded: count(objects, |o| o.health.as_deref() == Some("Degraded")),
                missing: count(objects, |o| o.health.as_deref() == Some("Missing")),
            }),
        }
    }
}

impl Operation {
    /// Whether the operation has ended: it has a finish time, or a phase
    /// Argo CD writes only at the end.
    pub fn has_ended(&self) -> bool {
        self.finished_at.is_some()
            || matches!(
                self.phase.as_deref(),
                Some("Succeeded" | "Failed" | "Error")
            )
    }
}

fn count(objects: &[ManagedObject], test: impl Fn(&ManagedObject) -> bool) -> usize {
    objects.iter().filter(|object| test(object)).count()
}

/// What the operation shows of retries: its own count and phase, read
/// against the configured limit only to say a failure used them all.
fn retries(operation: Option<&Operation>, limit: Option<i64>) -> Retries {
    let Some(operation) = operation else {
        return Retries::NoneReported;
    };
    let count = operation.retry_count.unwrap_or(0);
    if count <= 0 {
        return Retries::NoneReported;
    }
    match operation.phase.as_deref() {
        Some("Running") => Retries::Retrying { count },
        Some("Failed" | "Error") if limit.is_some_and(|limit| limit > 0 && count >= limit) => {
            Retries::Exhausted { count }
        }
        _ => Retries::Ended {
            count,
            phase: operation.phase.clone(),
        },
    }
}

/// The Applications of one Argo CD namespace.
pub async fn read_applications<R: Reader>(reader: &R, namespace: &str) -> Source<Vec<Application>> {
    async fn run<R: Reader>(
        reader: &R,
        namespace: &str,
    ) -> Result<(Vec<Application>, Option<Truncation>), Failure> {
        let resource = resolve(reader, GROUP, "applications", VERSIONS, true).await?;
        let listing = reader
            .list(&ListRequest {
                resource,
                scope: Scope::Namespace(namespace.to_owned()),
            })
            .await?;
        Ok(listing.parse(parse_application))
    }
    Source::from_listing(run(reader, namespace).await)
}

/// An ApplicationSet: the generator of a family of Applications, such as one
/// per environment. Only what names and groups it is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationSet {
    pub namespace: String,
    pub name: String,
    /// `metadata.uid` and `metadata.resourceVersion`.
    pub meta: Meta,
    pub metadata_labels: BTreeMap<String, String>,
    /// `spec.template.spec.project`, the AppProject its Applications use.
    pub project: Option<String>,
}

pub fn parse_application_set(value: &Value) -> Option<ApplicationSet> {
    Some(ApplicationSet {
        namespace: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        meta: Meta::parse(value),
        metadata_labels: strings(value, "/metadata/labels"),
        project: text(value, "/spec/template/spec/project"),
    })
}

impl ApplicationSet {
    /// The object these facts were read from.
    pub fn object_ref(&self) -> ObjectRef {
        ObjectRef::new(
            GROUP,
            "ApplicationSet",
            Some(&self.namespace),
            &self.name,
            &self.meta,
        )
    }
}

/// The ApplicationSets of one Argo CD namespace.
pub async fn read_application_sets<R: Reader>(
    reader: &R,
    namespace: &str,
) -> Source<Vec<ApplicationSet>> {
    async fn run<R: Reader>(
        reader: &R,
        namespace: &str,
    ) -> Result<(Vec<ApplicationSet>, Option<Truncation>), Failure> {
        let resource = resolve(reader, GROUP, "applicationsets", VERSIONS, true).await?;
        let listing = reader
            .list(&ListRequest {
                resource,
                scope: Scope::Namespace(namespace.to_owned()),
            })
            .await?;
        Ok(listing.parse(parse_application_set))
    }
    Source::from_listing(run(reader, namespace).await)
}

/// The name Argo CD gives the cluster it runs in.
pub const IN_CLUSTER_NAME: &str = "in-cluster";
/// The address Argo CD gives the cluster it runs in, as seen from inside it.
pub const IN_CLUSTER_SERVER: &str = "https://kubernetes.default.svc";

/// How a destination that matches no context says it could be: by the
/// spike's flags, or in the app's Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapHint {
    /// `--known-as` and `--known-as-name`.
    #[default]
    Flags,
    /// Settings › Workspace, where the contexts are workspace clusters.
    Settings,
}

/// How an Application's destination matches the kubeconfig contexts. Several
/// matches are never resolved by guessing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DestinationMatch {
    /// The destination server is this context's.
    One(String),
    /// The destination is Argo CD's own cluster, `in-cluster` or
    /// `https://kubernetes.default.svc`: the context Argo CD was read from.
    ArgoCd(String),
    /// The destination's cluster name, mapped by the user to this context.
    Named {
        name: String,
        context: String,
    },
    /// Candidates, in the order given; the user must choose.
    Ambiguous(Vec<String>),
    None,
    /// The destination is a cluster name no context is mapped to. Its cluster
    /// Secret, which holds the address, is not read.
    ByName(String),
    /// The Application has no destination at all.
    Unspecified,
}

impl DestinationMatch {
    /// The one context the destination is, if it is known.
    pub fn context(&self) -> Option<&str> {
        match self {
            Self::One(context) | Self::ArgoCd(context) | Self::Named { context, .. } => {
                Some(context)
            }
            _ => None,
        }
    }
}

/// Where destinations may point: the `(context name, server)` pairs of the
/// kubeconfig contexts, the `(context name, cluster name)` pairs the user
/// mapped, and the context Argo CD runs in.
#[derive(Clone, Copy, Debug)]
pub struct Destinations<'a> {
    pub contexts: &'a [(String, String)],
    pub names: &'a [(String, String)],
    pub argocd: &'a str,
}

/// `https://Host:443/` and `https://host` name the same server.
pub fn normalize_server(server: &str) -> String {
    let lower = server.trim().to_ascii_lowercase();
    let trimmed = lower.trim_end_matches('/');
    trimmed
        .strip_suffix(":443")
        .filter(|rest| rest.starts_with("https://"))
        .unwrap_or(trimmed)
        .to_owned()
}

/// Matches a destination against the contexts. The servers stay inside this
/// function; only context names come out. Argo CD's own names for its
/// cluster match the Argo CD context, as they do in Argo CD; any other
/// cluster name matches only the contexts the user mapped it to.
pub fn match_destination(application: &Application, to: Destinations<'_>) -> DestinationMatch {
    let (mut found, own) = if let Some(server) = &application.destination_server {
        let wanted = normalize_server(server);
        let found: Vec<String> = to
            .contexts
            .iter()
            .filter(|(_, server)| normalize_server(server) == wanted)
            .map(|(name, _)| name.clone())
            .collect();
        (found, wanted == IN_CLUSTER_SERVER)
    } else if let Some(name) = &application.destination_name {
        let found: Vec<String> = to
            .names
            .iter()
            .filter(|(_, mapped)| mapped == name)
            .map(|(context, _)| context.clone())
            .collect();
        (found, name == IN_CLUSTER_NAME)
    } else {
        return DestinationMatch::Unspecified;
    };
    if own {
        found.push(to.argocd.to_owned());
    }
    // A context may be reachable at more than one address, or mapped under
    // a name more than once, so it may be found more than once.
    found.sort();
    found.dedup();
    match (found.len(), &application.destination_server) {
        (0, Some(_)) => DestinationMatch::None,
        (0, None) => {
            DestinationMatch::ByName(application.destination_name.clone().unwrap_or_default())
        }
        (1, _) if own => DestinationMatch::ArgoCd(found.remove(0)),
        (1, Some(_)) => DestinationMatch::One(found.remove(0)),
        (1, None) => DestinationMatch::Named {
            name: application.destination_name.clone().unwrap_or_default(),
            context: found.remove(0),
        },
        _ => DestinationMatch::Ambiguous(found),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    const COMMIT_A: &str = "1111111111111111111111111111111111111111";
    const COMMIT_B: &str = "2222222222222222222222222222222222222222";
    const COMMIT_C: &str = "3333333333333333333333333333333333333333";

    fn app(spec: Value, status: Value) -> Application {
        parse_application(&json!({
            "metadata": {"namespace": "argocd", "name": "storefront"},
            "spec": spec,
            "status": status,
        }))
        .expect("an Application")
    }

    #[test]
    fn application_set_owner_is_read_from_its_owner_reference() {
        let owned = |kind: &str, api_version: &str| {
            parse_application(&json!({
                "metadata": {
                    "namespace": "argocd",
                    "name": "storefront-dev",
                    "ownerReferences": [{"kind": kind, "apiVersion": api_version, "name": "storefront"}],
                },
            }))
            .expect("an Application")
            .owner_application_set
        };
        assert_eq!(
            owned("ApplicationSet", "argoproj.io/v1alpha1").as_deref(),
            Some("storefront")
        );
        assert_eq!(owned("ApplicationSet", "example.test/v1"), None);
        assert_eq!(owned("Rollout", "argoproj.io/v1alpha1"), None);
        assert_eq!(app(json!({}), json!({})).owner_application_set, None);
    }

    #[test]
    fn application_set_parses_its_name_labels_and_project() {
        let set = parse_application_set(&json!({
            "metadata": {
                "namespace": "argocd",
                "name": "storefront",
                "labels": {"app.kubernetes.io/part-of": "shop"},
            },
            "spec": {"template": {"spec": {"project": "shop"}}},
        }))
        .expect("an ApplicationSet");
        assert_eq!(set.name, "storefront");
        assert_eq!(set.project.as_deref(), Some("shop"));
        assert_eq!(
            set.metadata_labels
                .get("app.kubernetes.io/part-of")
                .map(String::as_str),
            Some("shop")
        );
        assert!(parse_application_set(&json!({"metadata": {"name": "x"}})).is_none());
    }

    #[test]
    fn automated_sync_disabled_is_not_automated() {
        let disabled = app(
            json!({"syncPolicy": {"automated": {"enabled": false, "prune": true, "selfHeal": true}}}),
            json!({}),
        );
        let policy = disabled.sync_policy.as_ref().unwrap();
        assert_eq!(
            policy.automated,
            Some(Automated {
                enabled: Some(false),
                prune: Some(true),
                self_heal: Some(true),
            })
        );
        assert_eq!(disabled.reconciliation().auto_sync, AutoSync::Disabled);

        let manual = app(json!({"syncPolicy": {}}), json!({}));
        assert_eq!(manual.reconciliation().auto_sync, AutoSync::Manual);
        let no_policy = app(json!({}), json!({}));
        assert_eq!(no_policy.sync_policy, None);
        assert_eq!(no_policy.reconciliation().auto_sync, AutoSync::Manual);

        let on = app(
            json!({"syncPolicy": {"automated": {"selfHeal": true}}}),
            json!({}),
        );
        assert_eq!(
            on.reconciliation().auto_sync,
            AutoSync::Enabled {
                prune: false,
                self_heal: true,
            }
        );
    }

    #[test]
    fn a_bounded_retry_reports_its_count_and_limit() {
        let application = app(
            json!({"syncPolicy": {
                "automated": {"prune": true},
                "retry": {"limit": 5, "backoff": {"duration": "5s", "factor": 2, "maxDuration": "3m"}},
            }}),
            json!({"operationState": {
                "phase": "Running",
                "message": "Retrying attempt #2 at 10:00AM: failed to reach https://git.example.test/acme/storefront.git",
                "retryCount": 2,
                "startedAt": "2026-10-01T10:00:00Z",
                "syncResult": {"revision": COMMIT_A},
            }}),
        );
        let retry = application
            .sync_policy
            .as_ref()
            .and_then(|policy| policy.retry.clone())
            .unwrap();
        assert_eq!(retry.limit, Some(5));
        assert_eq!(
            retry.backoff,
            Some(Backoff {
                duration: Some("5s".into()),
                factor: Some(2),
                max_duration: Some("3m".into()),
            })
        );
        let operation = application.operation.as_ref().unwrap();
        assert_eq!(operation.phase.as_deref(), Some("Running"));
        assert_eq!(operation.retry_count, Some(2));
        assert_eq!(operation.revisions, vec![COMMIT_A.to_owned()]);
        assert_eq!(
            operation.started_at.as_deref(),
            Some("2026-10-01T10:00:00Z")
        );
        assert_eq!(operation.finished_at, None);
        let message = operation.message.as_deref().unwrap();
        assert!(message.starts_with("Retrying attempt #2"), "{message}");
        assert!(!message.contains("git.example.test"), "{message}");

        let reconciliation = application.reconciliation();
        assert_eq!(reconciliation.retry_limit, Some(5));
        assert_eq!(reconciliation.retries, Retries::Retrying { count: 2 });
        assert_eq!(reconciliation.phase.as_deref(), Some("Running"));
    }

    #[test]
    fn a_failed_operation_at_its_limit_has_exhausted_its_retries() {
        let application = app(
            json!({"syncPolicy": {"retry": {"limit": 5}}}),
            json!({"operationState": {
                "phase": "Failed",
                "message": "one or more objects failed to apply",
                "retryCount": 5,
                "finishedAt": "2026-10-01T10:20:00Z",
            }}),
        );
        let reconciliation = application.reconciliation();
        assert_eq!(reconciliation.retries, Retries::Exhausted { count: 5 });
        assert_eq!(reconciliation.retry_limit, Some(5));

        let short = app(
            json!({"syncPolicy": {"retry": {"limit": 5}}}),
            json!({"operationState": {"phase": "Failed", "retryCount": 3}}),
        );
        assert_eq!(
            short.reconciliation().retries,
            Retries::Ended {
                count: 3,
                phase: Some("Failed".into()),
            }
        );
    }

    #[test]
    fn configuration_alone_claims_no_retries() {
        for limit in [json!(5), json!(-1), json!(0)] {
            let application = app(
                json!({"syncPolicy": {"automated": {}, "retry": {"limit": limit}}}),
                json!({}),
            );
            let reconciliation = application.reconciliation();
            assert_eq!(reconciliation.retries, Retries::NoneReported);
            assert_eq!(reconciliation.retry_limit, limit.as_i64());
            assert_eq!(reconciliation.phase, None);
        }
        let without_limit = app(json!({"syncPolicy": {"retry": {}}}), json!({}));
        assert_eq!(without_limit.reconciliation().retry_limit, None);
        // A failure without a known positive limit is not "exhausted".
        let unbounded = app(
            json!({"syncPolicy": {"retry": {"limit": -1}}}),
            json!({"operationState": {"phase": "Failed", "retryCount": 9}}),
        );
        assert!(matches!(
            unbounded.reconciliation().retries,
            Retries::Ended { count: 9, .. }
        ));
    }

    #[test]
    fn multiple_sources_name_each_revision_and_never_a_repository_host() {
        let application = app(
            json!({
                "source": {"repoURL": "https://git.example.test/acme/ignored.git", "targetRevision": "ignored"},
                "sources": [
                    {"repoURL": "https://deploy:s3cret@git.example.test/acme/storefront.git",
                     "targetRevision": "main", "path": "deploy/env-a"},
                    {"repoURL": "https://registry.example/charts", "chart": "storefront",
                     "targetRevision": "1.4.2"},
                    {"repoURL": "git@git.example.test:acme/values.git", "ref": "values"},
                ],
            }),
            json!({
                "sync": {"status": "Synced", "revisions": [COMMIT_A, "1.4.2", COMMIT_B]},
                "history": [
                    {"id": 1, "revisions": [COMMIT_C, "1.4.1", COMMIT_B], "deployedAt": "2026-09-30T08:00:00Z"},
                    {"id": 2, "revisions": [COMMIT_A, "1.4.2", COMMIT_B], "deployedAt": "2026-10-01T09:00:00Z"},
                ],
                "operationState": {"phase": "Succeeded", "syncResult": {
                    "revision": "",
                    "revisions": [COMMIT_A, "1.4.2", COMMIT_B],
                }},
            }),
        );
        let Some(Sources::Multiple(sources)) = &application.sources else {
            panic!("multiple sources: {:?}", application.sources);
        };
        assert_eq!(sources.len(), 3);
        assert_eq!(sources[0].path.as_deref(), Some("deploy/env-a"));
        assert_eq!(sources[1].chart.as_deref(), Some("storefront"));
        assert_eq!(sources[2].reference.as_deref(), Some("values"));
        assert_eq!(
            application.revisions(),
            Revisions {
                requested: vec![Some("main".into()), Some("1.4.2".into()), None],
                compared: vec![COMMIT_A.into(), "1.4.2".into(), COMMIT_B.into()],
                deployed: vec![COMMIT_A.into(), "1.4.2".into(), COMMIT_B.into()],
            }
        );
        assert_eq!(
            application
                .last_deployed
                .as_ref()
                .unwrap()
                .deployed_at
                .as_deref(),
            Some("2026-10-01T09:00:00Z")
        );
        assert_eq!(
            application.operation.as_ref().unwrap().revisions,
            vec![COMMIT_A.to_owned(), "1.4.2".into(), COMMIT_B.into()]
        );
        assert!(application.reconciliation().multiple_sources);
        let everything = format!("{application:?}");
        for secret in ["git.example.test", "registry.example", "deploy:", "s3cret"] {
            assert!(!everything.contains(secret), "{secret} in {everything}");
        }

        let single = app(
            json!({"source": {"repoURL": "https://git.example.test/acme/storefront.git",
                              "targetRevision": "main", "path": "deploy/env-b"}}),
            json!({"sync": {"revision": COMMIT_A}, "history": [{"revision": COMMIT_C}]}),
        );
        assert!(matches!(single.sources, Some(Sources::Single(_))));
        let reconciliation = single.reconciliation();
        assert!(!reconciliation.multiple_sources);
        assert_eq!(
            reconciliation.revisions,
            Revisions {
                requested: vec![Some("main".into())],
                compared: vec![COMMIT_A.into()],
                deployed: vec![COMMIT_C.into()],
            }
        );
        assert_eq!(single.last_deployed.unwrap().deployed_at, None);
    }

    #[test]
    fn a_missing_inventory_is_unknown_not_empty() {
        let unknown = app(json!({}), json!({"sync": {"status": "Unknown"}}));
        assert!(!unknown.inventory_reported);
        assert_eq!(unknown.inventory(), None);
        assert_eq!(unknown.reconciliation().inventory, None);
        assert_eq!(unknown.revisions(), Revisions::default());
        assert_eq!(unknown.sources, None);
        assert_eq!(unknown.operation, None);
        assert_eq!(unknown.last_deployed, None);

        let empty = app(json!({}), json!({"resources": []}));
        assert_eq!(empty.inventory(), Some(&[][..]));
        assert_eq!(
            empty.reconciliation().inventory,
            Some(InventoryCounts::default())
        );
    }

    #[test]
    fn applications_of_the_same_name_in_two_namespaces_stay_apart() {
        let parse = |namespace: &str, phase: &str| {
            parse_application(&json!({
                "metadata": {"namespace": namespace, "name": "storefront"},
                "spec": {"syncPolicy": {"automated": {}}},
                "status": {"operationState": {"phase": phase}},
            }))
            .unwrap()
        };
        let first = parse("argocd", "Succeeded");
        let second = parse("argocd-acme", "Failed");
        assert_eq!(first.name, second.name);
        assert_ne!(first.namespace, second.namespace);
        assert_eq!(first.reconciliation().phase.as_deref(), Some("Succeeded"));
        assert_eq!(second.reconciliation().phase.as_deref(), Some("Failed"));
    }

    #[test]
    fn a_namesake_in_another_group_is_another_object() {
        let application = app(
            json!({}),
            json!({
                "resources": [
                    {"group": "apps", "version": "v1", "kind": "Deployment", "namespace": "shop",
                     "name": "storefront", "status": "Synced", "health": {"status": "Healthy"}},
                    {"group": "acme.example.test", "version": "v1beta1", "kind": "Deployment",
                     "namespace": "shop", "name": "storefront", "status": "OutOfSync",
                     "health": {"status": "Degraded",
                                "message": "probe to 192.0.2.10:8080 failed for User \"jane\""}},
                    {"version": "v1", "kind": "Service", "namespace": "shop", "name": "storefront",
                     "status": "Synced", "health": {"status": "Missing"}},
                    {"version": "v1", "kind": "ConfigMap", "namespace": "shop", "name": "storefront"},
                ],
                "conditions": [
                    {"type": "ComparisonError",
                     "message": "failed to load https://git.example.test/acme/storefront.git"},
                    {"message": "a condition without a type is dropped"},
                ],
            }),
        );
        let apps = application
            .managed_object("apps", "Deployment", Some("shop"), "storefront")
            .unwrap();
        assert_eq!(apps.version.as_deref(), Some("v1"));
        assert_eq!(apps.sync.as_deref(), Some("Synced"));
        assert_eq!(apps.health.as_deref(), Some("Healthy"));
        assert_eq!(apps.health_message, None);
        let custom = application
            .managed_object(
                "acme.example.test",
                "Deployment",
                Some("shop"),
                "storefront",
            )
            .unwrap();
        assert_eq!(custom.version.as_deref(), Some("v1beta1"));
        assert_eq!(custom.sync.as_deref(), Some("OutOfSync"));
        assert_eq!(custom.health.as_deref(), Some("Degraded"));
        let message = custom.health_message.as_deref().unwrap();
        assert!(!message.contains("192.0.2.10"), "{message}");
        assert!(!message.contains("jane"), "{message}");
        assert!(
            application
                .managed_object("", "Deployment", Some("shop"), "storefront")
                .is_none()
        );
        // Without a sync status or health, an object counts nowhere.
        let bare = application
            .managed_object("", "ConfigMap", Some("shop"), "storefront")
            .unwrap();
        assert_eq!((bare.sync.as_deref(), bare.health.as_deref()), (None, None));
        assert_eq!(bare.health_message, None);
        assert_eq!(
            application.reconciliation().inventory,
            Some(InventoryCounts {
                objects: 4,
                out_of_sync: 1,
                degraded: 1,
                missing: 1,
            })
        );
        assert_eq!(application.conditions.len(), 1);
        assert_eq!(application.conditions[0].kind, "ComparisonError");
        let message = application.conditions[0].message.as_deref().unwrap();
        assert!(!message.contains("git.example.test"), "{message}");
    }

    #[test]
    fn a_stale_operation_is_superseded_not_current() {
        let failed_long_ago = json!({
            "phase": "Failed",
            "message": "one or more objects failed to apply",
            "retryCount": 5,
            "startedAt": "2026-09-01T10:00:00Z",
            "finishedAt": "2026-09-01T10:20:00Z",
            "syncResult": {"revision": COMMIT_B},
        });
        let application = app(
            json!({"syncPolicy": {"automated": {}, "retry": {"limit": 5}}}),
            json!({
                "sync": {"status": "Synced", "revision": COMMIT_A},
                "health": {"status": "Healthy"},
                "operationState": failed_long_ago,
            }),
        );
        let reconciliation = application.reconciliation();
        assert!(reconciliation.superseded);
        assert_eq!(reconciliation.phase.as_deref(), Some("Failed"));
        assert_eq!(reconciliation.retries, Retries::Exhausted { count: 5 });
        assert_eq!(
            reconciliation.operation_finished_at.as_deref(),
            Some("2026-09-01T10:20:00Z")
        );
        assert_eq!(
            reconciliation.operation_revisions,
            vec![COMMIT_B.to_owned()]
        );
        assert_eq!(reconciliation.revisions.compared, vec![COMMIT_A.to_owned()]);

        // The same failure at the revision still compared is current.
        let current = app(
            json!({"syncPolicy": {"retry": {"limit": 5}}}),
            json!({
                "sync": {"status": "OutOfSync", "revision": COMMIT_B},
                "operationState": failed_long_ago,
            }),
        );
        assert!(!current.reconciliation().superseded);

        // Both fields written, on either side, are read alike.
        let both = app(
            json!({"sources": [{"targetRevision": "main"}, {"targetRevision": "main"}]}),
            json!({
                "sync": {"revision": COMMIT_A, "revisions": [COMMIT_A, COMMIT_B]},
                "operationState": {"phase": "Succeeded", "syncResult": {
                    "revision": COMMIT_A, "revisions": [COMMIT_A, COMMIT_B],
                }},
            }),
        );
        assert!(!both.reconciliation().superseded);
        assert_eq!(
            both.revisions().compared,
            vec![COMMIT_A.to_owned(), COMMIT_B.to_owned()]
        );

        // A running operation is never history, whatever it syncs.
        let running = app(
            json!({}),
            json!({
                "sync": {"status": "OutOfSync", "revision": COMMIT_A},
                "operationState": {"phase": "Running", "syncResult": {"revision": COMMIT_B}},
            }),
        );
        assert!(!running.reconciliation().superseded);

        // Without revisions on either side, nothing says it was superseded.
        let unknown = app(
            json!({}),
            json!({
                "sync": {"status": "Synced", "revision": COMMIT_A},
                "operationState": {"phase": "Failed", "finishedAt": "2026-09-01T10:20:00Z"},
            }),
        );
        assert!(!unknown.reconciliation().superseded);
        // An operation that failed before it synced names the revisions it
        // asked for, and ages out by them.
        let unsynced = app(
            json!({}),
            json!({
                "sync": {"status": "Synced", "revision": COMMIT_A},
                "operationState": {
                    "phase": "Failed",
                    "message": "rpc error: manifest generation failed",
                    "finishedAt": "2026-09-01T10:20:00Z",
                    "operation": {"sync": {"revision": COMMIT_B}},
                },
            }),
        );
        let reconciliation = unsynced.reconciliation();
        assert_eq!(
            reconciliation.operation_revisions,
            vec![COMMIT_B.to_owned()]
        );
        assert!(reconciliation.superseded);
        // A sync result that names no revision (it failed before any
        // source synced) falls back to the revisions the operation asked for.
        let empty_result = app(
            json!({}),
            json!({
                "sync": {"status": "Synced", "revision": COMMIT_A},
                "operationState": {
                    "phase": "Failed",
                    "finishedAt": "2026-09-01T10:20:00Z",
                    "syncResult": {"resources": []},
                    "operation": {"sync": {"revisions": [COMMIT_B]}},
                },
            }),
        );
        let reconciliation = empty_result.reconciliation();
        assert_eq!(
            reconciliation.operation_revisions,
            vec![COMMIT_B.to_owned()]
        );
        assert!(reconciliation.superseded);
        let uncompared = app(
            json!({}),
            json!({"operationState": {"phase": "Failed", "syncResult": {"revision": COMMIT_B}}}),
        );
        assert!(!uncompared.reconciliation().superseded);
    }

    #[test]
    fn a_null_block_is_no_block() {
        let application = app(
            json!({"syncPolicy": {"automated": null, "retry": {"limit": 3, "backoff": null}}}),
            json!({"operationState": null}),
        );
        let policy = application.sync_policy.as_ref().unwrap();
        assert_eq!(policy.automated, None);
        assert_eq!(
            policy.retry,
            Some(RetryPolicy {
                limit: Some(3),
                backoff: None,
            })
        );
        assert_eq!(application.operation, None);
        assert_eq!(application.reconciliation().auto_sync, AutoSync::Manual);

        let empty = app(json!({"syncPolicy": null}), json!({}));
        assert_eq!(empty.sync_policy, None);
        let no_retry = app(json!({"syncPolicy": {"retry": null}}), json!({}));
        assert_eq!(no_retry.sync_policy.unwrap().retry, None);
    }
}
