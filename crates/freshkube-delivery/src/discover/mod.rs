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
mod builds;
#[cfg(test)]
mod builds_tests;
mod destinations;
pub mod example;
#[cfg(test)]
mod tests;

pub use builds::{
    BuildFindings, BuildItem, BuildRead, BuildSuggestions, BuildVerdict, MAX_LISTED, MAX_PROBED,
    NamespaceCounts, Probe, Projects, count_namespaces, find_builds, suggest_build_namespace,
};
pub(crate) use destinations::parse_secret_list;
pub use destinations::{
    ClusterSecret, ClusterSecrets, DestinationFindings, Field, Found, Item, NamespaceRead, Note,
    Read, Suggestions, Verdict, find_destinations, suggest_destinations,
};

/// The findings a Secret list body gives for `entry`, matched against
/// `servers` (context, server) as a find matches them. For other crates'
/// tests, which must prove nothing but the two fields comes through.
#[cfg(any(test, feature = "testing"))]
pub fn findings_from_body(
    entry: &str,
    namespace: &str,
    body: &[u8],
    servers: &[(&str, &str)],
) -> Result<DestinationFindings, freshkube_core::resources::Failure> {
    let servers: Vec<(String, String)> = servers
        .iter()
        .map(|(context, server)| ((*context).to_owned(), (*server).to_owned()))
        .collect();
    let found = parse_secret_list(namespace, body)?
        .items
        .into_iter()
        .map(|secret| Found {
            contexts: destinations::contexts_of(&servers, &secret),
            secret,
        })
        .collect();
    Ok(DestinationFindings {
        entry: entry.to_owned(),
        read: Read::Done {
            namespaces: vec![(namespace.to_owned(), NamespaceRead::Found(found))],
            skipped: Vec::new(),
            no_servers: None,
        },
    })
}
