//! Read-only delivery joins: a commit to the pods that run it (#158).
//!
//! This is a spike. It reads Kargo, Argo CD, Argo Rollouts, Pipelines as
//! Code and Tekton (with Chains and Conforma results), and pods, and joins
//! them only on commit SHA and image digest, saying for each link whether it
//! is confirmed, claimed or unknown. Every cluster call goes through
//! [`read::ReadOnlyClient`], which can only GET; nothing here writes.

pub mod argocd;
pub mod change;
pub mod collect;
pub mod deployments;
pub mod digest;
pub mod github;
pub mod join;
pub mod kargo;
pub mod observation;
pub mod pods;
pub mod read;
pub mod rollouts;
pub mod source;
pub mod tekton;
mod versions;

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

pub use argocd::StageNaming;
pub use collect::{Clusters, Plan, collect, commit_of_pull_request};
pub use digest::Digest;
pub use github::{GhCli, GitHub, PullRequest};
pub use join::{Confidence, Evidence, Hop, Key, Link, Trail, join, render};
pub use observation::{Fact, Meta, ObjectRef, Observation};
pub use read::{ListRequest, ReadOnlyClient, Reader, Resource, Scope};
pub use source::{
    Source, printable, redact_body, redact_identity, redact_location, redact_message, shown,
};
pub use tekton::{CommitNames, EvidenceResult};
