//! The delivery chain, read-only: a commit to the pods that run it (#158),
//! and the applications derived from it (#521).
//!
//! It reads Kargo, Argo CD, Argo Rollouts, Pipelines as Code and Tekton
//! (with Chains and Conforma results), and pods, across the workspace's
//! cluster entries, and joins them only on commit SHA and image digest,
//! saying for each link whether it is confirmed, claimed or unknown
//! ([`join`], [`change`], [`changes`]). [`applications`] derives what a
//! cluster runs from Kargo Projects, Argo CD and the `part-of` label, and
//! says how far each was read. Every cluster call goes through
//! [`read::ReadOnlyClient`], which can only GET; nothing here writes.
//!
//! The crate uses `freshkube-core` and `talos-rs` and no UI types. Only the
//! modules a screen reaches are public; example data and parsers that
//! other crates' tests read are behind the `testing` feature
//! (`freshkube_delivery::testing`).

pub(crate) mod address;
pub mod applications;
pub mod argocd;
pub(crate) mod chains;
pub mod change;
pub mod changes;
pub(crate) mod collect;
pub(crate) mod deployments;
pub(crate) mod digest;
pub mod entry;
pub(crate) mod github;
pub mod join;
pub mod kargo;
pub(crate) mod observation;
pub(crate) mod pods;
pub mod read;
pub(crate) mod results;
pub(crate) mod rollouts;
pub(crate) mod shared;
pub mod source;
pub(crate) mod tekton;
pub(crate) mod verify;
pub(crate) mod versions;
pub mod words;

#[cfg(test)]
mod deployment_tests;
#[cfg(test)]
pub(crate) mod fixtures;
#[cfg(test)]
mod provenance;
#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
mod warehouse_tests;

pub use address::Address;
pub use argocd::StageNaming;
pub use collect::{Clusters, Plan, collect, commit_of_pull_request};
pub use digest::is_full_sha;
pub use github::GhCli;
pub use join::{Confidence, join, render};
pub use read::{ListRequest, ReadOnlyClient, Reader, Resource, Scope};
pub use source::{Source, printable, shown};
pub use tekton::{CommitNames, EvidenceResult};

/// The example data and parsers that other crates' tests read. Compiled
/// only for this crate's tests and for dependents' dev-dependencies
/// (`features = ["testing"]`); nothing shipped names them.
#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use crate::address::Address;
    use crate::change::Change;
    use crate::kargo::Stage;
    use chrono::{DateTime, Utc};
    use serde_json::Value;

    /// Argo CD's own Deployments in the example's `core-fra`.
    pub const ARGOCD_WORKLOADS: [&str; 2] = crate::applications::example::ARGOCD_WORKLOADS;
    /// Where the example release pipeline checks policy.
    pub const RELEASE_NAMESPACE: &str = crate::change::example::RELEASE_NAMESPACE;
    /// The example release run's policy TaskRun.
    pub const VERIFY_TASK: &str = crate::change::example::VERIFY_TASK;

    /// The example Freight's page in Kargo.
    pub fn kargo_page() -> Address {
        crate::change::example::kargo_page()
    }

    /// The example change as the change page shows it.
    pub fn change(now: DateTime<Utc>) -> Change {
        crate::change::example::change(now)
    }

    /// A Kargo Stage as the reads parse it.
    pub fn parse_stage(value: &Value) -> Option<Stage> {
        crate::kargo::parse_stage(value)
    }
}
