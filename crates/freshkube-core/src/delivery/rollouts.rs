//! Argo Rollouts, their ReplicaSets and their AnalysisRuns, as read in an
//! environment cluster.

use serde_json::Value;

use super::digest::{Digest, text};
use super::observation::{Meta, ObjectRef};
use super::read::{ListRequest, Reader, Resource, Scope};
use super::source::{Source, Truncation};
use super::versions::resolve;
use crate::resources::Failure;

pub const GROUP: &str = "argoproj.io";
const VERSIONS: &[&str] = &["v1alpha1"];
/// The label every pod of a Rollout's ReplicaSet carries.
pub const POD_HASH_LABEL: &str = "rollouts-pod-template-hash";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rollout {
    pub namespace: String,
    pub name: String,
    /// `meta.uid` is what its ReplicaSets' owner references carry.
    /// `metadata.uid` and `metadata.resourceVersion`.
    pub meta: Meta,
    pub phase: Option<String>,
    pub current_pod_hash: Option<String>,
    pub stable_hash: Option<String>,
    /// `spec.template.spec.containers[].image`, as written.
    pub images: Vec<String>,
    pub aborted: bool,
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnalysisRun {
    pub namespace: String,
    pub name: String,
    /// `metadata.uid` and `metadata.resourceVersion`.
    pub meta: Meta,
    pub phase: Option<String>,
    pub rollout: Option<String>,
}

/// A ReplicaSet a Rollout owns: one pod template, one pod-template hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaSet {
    pub namespace: String,
    pub name: String,
    /// `metadata.uid` and `metadata.resourceVersion`.
    pub meta: Meta,
    /// The UID of the Rollout its controller owner reference names.
    pub owner_uid: Option<String>,
    /// Its `rollouts-pod-template-hash` label, which its pods carry too.
    pub pod_hash: Option<String>,
    /// `spec.template.spec.containers[].image`, as written.
    pub images: Vec<String>,
    pub replicas: u64,
    pub ready_replicas: u64,
}

impl AnalysisRun {
    /// The object these facts were read from.
    pub fn object_ref(&self) -> ObjectRef {
        ObjectRef::new(
            GROUP,
            "AnalysisRun",
            Some(&self.namespace),
            &self.name,
            &self.meta,
        )
    }
}

impl Rollout {
    /// The object these facts were read from.
    pub fn object_ref(&self) -> ObjectRef {
        ObjectRef::new(
            GROUP,
            "Rollout",
            Some(&self.namespace),
            &self.name,
            &self.meta,
        )
    }
}

impl ReplicaSet {
    /// The object these facts were read from.
    pub fn object_ref(&self) -> ObjectRef {
        ObjectRef::new(
            "apps",
            "ReplicaSet",
            Some(&self.namespace),
            &self.name,
            &self.meta,
        )
    }

    /// Whether its pod template pins one of `digests`.
    pub fn pins(&self, digests: &[Digest]) -> bool {
        self.images
            .iter()
            .filter_map(|image| Digest::from_reference(image))
            .any(|digest| digests.contains(&digest))
    }
}

fn containers(value: &Value) -> Vec<String> {
    value
        .pointer("/spec/template/spec/containers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|container| text(container, "/image"))
        .collect()
}

fn count(value: &Value, pointer: &str) -> u64 {
    value.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
}

pub fn parse_replica_set(value: &Value) -> Option<ReplicaSet> {
    let owner_uid = value
        .pointer("/metadata/ownerReferences")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|owner| {
            text(owner, "/kind").as_deref() == Some("Rollout")
                && text(owner, "/apiVersion").is_some_and(|version| {
                    version
                        .split_once('/')
                        .is_some_and(|(group, _)| group == GROUP)
                })
                && owner.get("controller").and_then(Value::as_bool) == Some(true)
        })
        .and_then(|owner| text(owner, "/uid"));
    Some(ReplicaSet {
        namespace: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        meta: Meta::parse(value),
        owner_uid,
        pod_hash: value
            .pointer("/metadata/labels")
            .and_then(|labels| labels.get(POD_HASH_LABEL))
            .and_then(Value::as_str)
            .filter(|hash| !hash.is_empty())
            .map(str::to_owned),
        images: containers(value),
        replicas: count(value, "/status/replicas"),
        ready_replicas: count(value, "/status/readyReplicas"),
    })
}

pub fn parse_rollout(value: &Value) -> Option<Rollout> {
    Some(Rollout {
        namespace: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        meta: Meta::parse(value),
        phase: text(value, "/status/phase"),
        current_pod_hash: text(value, "/status/currentPodHash"),
        stable_hash: text(value, "/status/stableRS"),
        images: containers(value),
        aborted: value
            .pointer("/status/abort")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        paused: value
            .pointer("/status/pauseConditions")
            .and_then(Value::as_array)
            .is_some_and(|conditions| !conditions.is_empty())
            || value
                .pointer("/spec/paused")
                .and_then(Value::as_bool)
                .unwrap_or(false),
    })
}

pub fn parse_analysis_run(value: &Value) -> Option<AnalysisRun> {
    let rollout = value
        .pointer("/metadata/ownerReferences")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|owner| text(owner, "/kind").as_deref() == Some("Rollout"))
        .and_then(|owner| text(owner, "/name"));
    Some(AnalysisRun {
        namespace: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        meta: Meta::parse(value),
        phase: text(value, "/status/phase"),
        rollout,
    })
}

async fn read_namespace<R: Reader, T>(
    reader: &R,
    plural: &str,
    namespace: &str,
    parse: fn(&Value) -> Option<T>,
) -> Result<(Vec<T>, Option<Truncation>), Failure> {
    let resource = resolve(reader, GROUP, plural, VERSIONS, true).await?;
    let listing = reader
        .list(&ListRequest {
            resource,
            scope: Scope::Namespace(namespace.to_owned()),
        })
        .await?;
    Ok(listing.parse(parse))
}

pub async fn read_rollouts<R: Reader>(reader: &R, namespace: &str) -> Source<Vec<Rollout>> {
    Source::from_listing(read_namespace(reader, "rollouts", namespace, parse_rollout).await)
}

pub async fn read_analysis_runs<R: Reader>(
    reader: &R,
    namespace: &str,
) -> Source<Vec<AnalysisRun>> {
    Source::from_listing(
        read_namespace(reader, "analysisruns", namespace, parse_analysis_run).await,
    )
}

/// The ReplicaSets of one namespace that carry the Rollouts' pod-template
/// hash label, so only those a Rollout made; the caller keeps those its
/// Rollouts own.
pub async fn read_replica_sets<R: Reader>(reader: &R, namespace: &str) -> Source<Vec<ReplicaSet>> {
    async fn run<R: Reader>(
        reader: &R,
        namespace: &str,
    ) -> Result<(Vec<ReplicaSet>, Option<Truncation>), Failure> {
        let listing = reader
            .list(&ListRequest {
                resource: Resource::new("apps", "v1", "replicasets", true),
                // A constant key, so nothing read from an object reaches the
                // selector.
                scope: Scope::Labels {
                    namespace: Some(namespace.to_owned()),
                    selector: POD_HASH_LABEL.to_owned(),
                },
            })
            .await?;
        Ok(listing.parse(parse_replica_set))
    }
    Source::from_listing(run(reader, namespace).await)
}
