//! Setup discovery: what the clusters already show, found once when asked
//! and turned into suggestions for `freshkube.yaml` that the person accepts
//! or ignores (#821).
//!
//! A finder reads and keeps its *findings*. The *suggestions* are a pure
//! function of the findings and the setup as it is now, so a save, a Reload
//! or a hand edit derives them again without another read. Nothing here
//! writes.
//!
//! [`builds`] finds the namespaces PipelineRuns run in.
mod builds;
pub mod example;
#[cfg(test)]
mod tests;

pub use builds::{
    BuildFindings, BuildItem, BuildRead, BuildSuggestions, BuildVerdict, MAX_LISTED,
    NamespaceCounts, Note, count_namespaces, find_builds, suggest_build_namespace,
};
