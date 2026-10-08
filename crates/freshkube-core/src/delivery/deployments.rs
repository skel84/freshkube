//! Deployments and the ReplicaSets they own, as read in an environment
//! cluster: the controller chain between an Application and its pods.
//!
//! The Deployment controller reports which ReplicaSet is current: it writes
//! `deployment.kubernetes.io/revision` on the Deployment and on the
//! ReplicaSet that revision made, and the ReplicaSet's controller owner
//! reference names the Deployment's UID. The pod template's images stay
//! declared; only the pods report a digest.

use serde_json::Value;

use super::digest::text;
use super::observation::{Meta, ObjectRef};
use super::read::{ListRequest, Reader, Resource, Scope};
use super::rollouts::Container;
use super::source::{Source, Truncation};
use crate::resources::Failure;

pub const GROUP: &str = "apps";
/// The annotation the Deployment controller writes on a Deployment and on
/// the ReplicaSet of each of its revisions.
pub const REVISION: &str = "deployment.kubernetes.io/revision";
/// The label the Deployment controller puts on a ReplicaSet and its pods.
pub const TEMPLATE_HASH_LABEL: &str = "pod-template-hash";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deployment {
    pub namespace: String,
    pub name: String,
    /// `metadata.uid` and `metadata.resourceVersion`; the `uid` is what its
    /// ReplicaSets' owner references carry.
    pub meta: Meta,
    /// `metadata.generation`: the latest spec.
    pub generation: Option<i64>,
    /// `status.observedGeneration`: the spec the controller last acted on.
    pub observed_generation: Option<i64>,
    /// The revision the controller reports as current.
    pub revision: Option<String>,
    pub replicas: u64,
    pub updated_replicas: u64,
    pub available_replicas: u64,
    pub paused: bool,
    /// `spec.template.spec.containers[].image`, as written.
    pub images: Vec<String>,
    /// `spec.template.spec.containers[]`, by name, with the image each pins.
    pub containers: Vec<Container>,
}

/// A ReplicaSet a Deployment owns: one revision, one pod-template hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentSet {
    pub namespace: String,
    pub name: String,
    /// `metadata.uid` and `metadata.resourceVersion`.
    pub meta: Meta,
    /// The UID of the Deployment its controller owner reference names.
    pub owner_uid: Option<String>,
    /// The revision the controller wrote on it.
    pub revision: Option<String>,
    /// Its `pod-template-hash` label, which its pods carry too.
    pub pod_hash: Option<String>,
    pub replicas: u64,
    pub ready_replicas: u64,
}

impl Deployment {
    /// The object these facts were read from.
    pub fn object_ref(&self) -> ObjectRef {
        ObjectRef::new(
            GROUP,
            "Deployment",
            Some(&self.namespace),
            &self.name,
            &self.meta,
        )
    }

    /// Whether the controller has acted on the latest spec. `None` when
    /// either number is missing from what was read.
    pub fn observed_latest(&self) -> Option<bool> {
        Some(self.observed_generation? == self.generation?)
    }
}

impl DeploymentSet {
    /// The object these facts were read from.
    pub fn object_ref(&self) -> ObjectRef {
        ObjectRef::new(
            GROUP,
            "ReplicaSet",
            Some(&self.namespace),
            &self.name,
            &self.meta,
        )
    }
}

fn count(value: &Value, pointer: &str) -> u64 {
    value.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
}

fn annotation(value: &Value, key: &str) -> Option<String> {
    value
        .pointer("/metadata/annotations")
        .and_then(|annotations| annotations.get(key))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub fn parse_deployment(value: &Value) -> Option<Deployment> {
    let containers: Vec<Container> = value
        .pointer("/spec/template/spec/containers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|container| {
            Some(Container {
                name: text(container, "/name")?,
                image: text(container, "/image")?,
            })
        })
        .collect();
    Some(Deployment {
        namespace: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        meta: Meta::parse(value),
        generation: value
            .pointer("/metadata/generation")
            .and_then(Value::as_i64),
        observed_generation: value
            .pointer("/status/observedGeneration")
            .and_then(Value::as_i64),
        revision: annotation(value, REVISION),
        replicas: count(value, "/status/replicas"),
        updated_replicas: count(value, "/status/updatedReplicas"),
        available_replicas: count(value, "/status/availableReplicas"),
        paused: value
            .pointer("/spec/paused")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        images: containers.iter().map(|c| c.image.clone()).collect(),
        containers,
    })
}

/// A ReplicaSet whose controller owner reference is an `apps` Deployment;
/// any other is not read as one.
pub fn parse_deployment_set(value: &Value) -> Option<DeploymentSet> {
    let owner_uid = value
        .pointer("/metadata/ownerReferences")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|owner| {
            text(owner, "/kind").as_deref() == Some("Deployment")
                && text(owner, "/apiVersion").as_deref() == Some("apps/v1")
                && owner.get("controller").and_then(Value::as_bool) == Some(true)
        })
        .and_then(|owner| text(owner, "/uid"))?;
    Some(DeploymentSet {
        namespace: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        meta: Meta::parse(value),
        owner_uid: Some(owner_uid),
        revision: annotation(value, REVISION),
        pod_hash: value
            .pointer("/metadata/labels")
            .and_then(|labels| labels.get(TEMPLATE_HASH_LABEL))
            .and_then(Value::as_str)
            .filter(|hash| !hash.is_empty())
            .map(str::to_owned),
        replicas: count(value, "/status/replicas"),
        ready_replicas: count(value, "/status/readyReplicas"),
    })
}

/// The ReplicaSet the controller reports as `deployment`'s current one: that
/// the Deployment owns by UID, and that carries the Deployment's revision.
/// Why there is none otherwise.
pub fn current_set<'a>(
    deployment: &Deployment,
    sets: &'a [DeploymentSet],
) -> Result<&'a DeploymentSet, String> {
    let Some(uid) = deployment.meta.uid.as_deref() else {
        return Err(
            "the Deployment reports no UID, so no owner reference ties a ReplicaSet to it".into(),
        );
    };
    let Some(revision) = deployment.revision.as_deref() else {
        return Err(format!(
            "the Deployment reports no {REVISION} annotation, so no ReplicaSet is known to be its current one"
        ));
    };
    let mut found = sets.iter().filter(|set| {
        set.namespace == deployment.namespace
            && set.owner_uid.as_deref() == Some(uid)
            && set.meta.uid.is_some()
            && set.revision.as_deref() == Some(revision)
            && set.pod_hash.is_some()
    });
    match (found.next(), found.next()) {
        (Some(set), None) => Ok(set),
        (None, _) => Err(format!(
            "no ReplicaSet the Deployment owns carries its current revision {revision}"
        )),
        (Some(_), Some(_)) => Err(format!(
            "several ReplicaSets the Deployment owns carry its current revision {revision}"
        )),
    }
}

async fn list<R: Reader, T>(
    reader: &R,
    plural: &str,
    namespace: &str,
    parse: fn(&Value) -> Option<T>,
) -> Result<(Vec<T>, Option<Truncation>), Failure> {
    let listing = reader
        .list(&ListRequest {
            resource: Resource::new(GROUP, "v1", plural, true),
            scope: Scope::Namespace(namespace.to_owned()),
        })
        .await?;
    Ok(listing.parse(parse))
}

pub async fn read_deployments<R: Reader>(reader: &R, namespace: &str) -> Source<Vec<Deployment>> {
    Source::from_listing(list(reader, "deployments", namespace, parse_deployment).await)
}

/// The ReplicaSets of one namespace that a Deployment owns. Other
/// ReplicaSets are read and left out, so a cap counts them: a Deployment's
/// may be among those not read.
pub async fn read_deployment_sets<R: Reader>(
    reader: &R,
    namespace: &str,
) -> Source<Vec<DeploymentSet>> {
    Source::from_listing(list(reader, "replicasets", namespace, parse_deployment_set).await)
}
