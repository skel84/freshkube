//! Argo Rollouts and their AnalysisRuns, as read in an environment cluster.

use serde_json::Value;

use super::digest::text;
use super::read::{ListRequest, Reader, Scope};
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
    pub phase: Option<String>,
    pub rollout: Option<String>,
}

pub fn parse_rollout(value: &Value) -> Option<Rollout> {
    Some(Rollout {
        namespace: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        phase: text(value, "/status/phase"),
        current_pod_hash: text(value, "/status/currentPodHash"),
        stable_hash: text(value, "/status/stableRS"),
        images: value
            .pointer("/spec/template/spec/containers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|container| text(container, "/image"))
            .collect(),
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
