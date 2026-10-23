//! The shared team setup file (#849): a team keeps one `freshkube.yaml` in a
//! git repository or at an HTTPS address, and each person joins it. This
//! module is the rules, without a UI and without running anything: where the
//! file may come from ([`source`]), what it may set ([`file`]), which value
//! wins ([`merge`]), how a new version is reviewed before it applies
//! ([`review`]) and what joining, failing and leaving do ([`membership`]).
//!
//! Fetching runs nothing from the repository. [`source::GitFetch`] only
//! describes the git command; a later slice runs it.

pub mod example;
pub mod file;
pub mod keyfile;
pub mod membership;
pub mod merge;
pub mod review;
pub mod source;

#[cfg(test)]
mod tests;

pub use file::{Ignored, IgnoredKey, Values, Version, VersionError};
pub use keyfile::{KeyFileError, resolve_in_repo};
pub use membership::{Failure, Fetch, FetchFailed, Leave, Membership, Overrides};
pub use merge::{
    ClusterView, LevelSource, Origin, Resolved, allowed_levels, cluster_view, floor_words, resolve,
};
pub use review::{Change, ChangeKind, NotReviewed, Review, Step, TrustKind};
pub use source::{GitFetch, GitSource, HttpsSource, Refused, Source, redact};
