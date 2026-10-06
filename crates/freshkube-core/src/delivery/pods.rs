//! The images pods actually run: `containerStatuses[].imageID`, which names
//! the digest the runtime pulled, whatever the tag says.

use serde_json::Value;

use super::digest::{Digest, text};
use super::read::{ListRequest, Listing, Reader, Resource, Scope, label_equals};
use super::rollouts::POD_HASH_LABEL;
use super::source::{Source, Truncation};
use crate::resources::Failure;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningImage {
    pub pod: String,
    pub container: String,
    /// The tag or reference the container was asked to run, shown only.
    pub image: Option<String>,
    pub digest: Option<Digest>,
    pub ready: bool,
}

fn running(listing: &Listing) -> (Vec<RunningImage>, Option<Truncation>) {
    (
        listing.items.iter().flat_map(parse_pod).collect(),
        listing.truncated,
    )
}

pub fn parse_pod(value: &Value) -> Vec<RunningImage> {
    let Some(pod) = text(value, "/metadata/name") else {
        return Vec::new();
    };
    // Init containers don't keep running; only the app containers count.
    value
        .pointer("/status/containerStatuses")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|status| {
            Some(RunningImage {
                pod: pod.clone(),
                container: text(status, "/name")?,
                image: text(status, "/image"),
                digest: text(status, "/imageID").and_then(|id| Digest::from_reference(&id)),
                ready: status
                    .pointer("/ready")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

/// Reads a Rollout's pods by its pod-template-hash label, in its namespace,
/// and returns what each container runs.
pub async fn read_rollout_pods<R: Reader>(
    reader: &R,
    namespace: &str,
    pod_hash: &str,
) -> Source<Vec<RunningImage>> {
    async fn run<R: Reader>(
        reader: &R,
        namespace: &str,
        pod_hash: &str,
    ) -> Result<(Vec<RunningImage>, Option<Truncation>), Failure> {
        let listing = reader
            .list(&ListRequest {
                resource: Resource::new("", "v1", "pods", true),
                scope: Scope::Labels {
                    namespace: Some(namespace.to_owned()),
                    selector: label_equals(POD_HASH_LABEL, pod_hash)?,
                },
            })
            .await?;
        Ok(running(&listing))
    }
    Source::from_listing(run(reader, namespace, pod_hash).await)
}

/// Reads every pod of one namespace, for a workload that is not a Rollout.
/// The list is scoped to the namespace and bounded like every other.
pub async fn read_namespace_pods<R: Reader>(
    reader: &R,
    namespace: &str,
) -> Source<Vec<RunningImage>> {
    async fn run<R: Reader>(
        reader: &R,
        namespace: &str,
    ) -> Result<(Vec<RunningImage>, Option<Truncation>), Failure> {
        let listing = reader
            .list(&ListRequest {
                resource: Resource::new("", "v1", "pods", true),
                scope: Scope::Namespace(namespace.to_owned()),
            })
            .await?;
        Ok(running(&listing))
    }
    Source::from_listing(run(reader, namespace).await)
}
