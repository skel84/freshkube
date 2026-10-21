//! Setup discovery: what the clusters already show, found once when asked
//! and turned into suggestions for `freshkube.yaml` that the person accepts
//! or ignores (#821).
//!
//! A finder reads and keeps its *findings*. The *suggestions* are a pure
//! function of the findings and the setup as it is now, so a save or a
//! Reload derives them again without another read. Nothing here writes.
//!
//! [`destinations`] finds Argo CD's destinations in its cluster Secrets and
//! matches their servers to the contexts of the kubeconfig. It reads
//! Secrets, so it is shaped to hold nothing but `metadata.name`,
//! `data.server` and `data.name`: see [`destinations::parse_secret_list`].
mod destinations;
pub mod example;
#[cfg(test)]
mod tests;

pub(crate) use destinations::parse_secret_list;
pub use destinations::{
    ClusterSecret, ClusterSecrets, DestinationFindings, Field, Found, Item, MAX_CLUSTER_SECRETS,
    NamespaceRead, Note, Read, Suggestions, Verdict, find_destinations, suggest_destinations,
};
