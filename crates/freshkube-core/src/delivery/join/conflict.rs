//! A SHA-joined Freight whose image digest no read build of the commit
//! reports. Two builds of one commit usually report different digests, and
//! the build that made the Freight may be pruned or outside what was read,
//! so this is never a contradiction: the link stays Claimed and carries the
//! digests, for a person to judge.

use super::*;
use crate::delivery::digest::repository;

/// One image of a Freight that the read builds of its commit report under
/// other digests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigestConflict {
    /// The image's repository, as [`repository`] gives it.
    pub repository: String,
    /// The digest the Freight holds.
    pub freight: Digest,
    /// Each build that reported another digest for the repository.
    pub builds: Vec<BuiltDigest>,
}

/// A digest a build reported for an image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltDigest {
    /// The PipelineRun, `namespace/name`.
    pub build: String,
    pub digest: Digest,
    /// Its `status.completionTime`, as written.
    pub completed: Option<String>,
}

/// The Freight's images for whose repository a build of `sha` reports
/// another digest. No read build reports the Freight's own, or
/// [`matching_freight`] would have joined it on that digest.
///
/// None when the builds were capped or a build's TaskRuns weren't read,
/// since an unread build may report the Freight's digest. A build counts
/// only when a result reports the commit and the run succeeded; a Freight
/// image with no digest is never compared.
pub(super) fn digest_conflicts(
    builds: &[Build],
    names: &CommitNames,
    sha: &str,
    freight: &Freight,
    builds_capped: bool,
) -> Vec<DigestConflict> {
    if builds_capped || builds.iter().any(|build| build.tasks_unread.is_some()) {
        return Vec::new();
    }
    let reporting: Vec<&Build> = builds
        .iter()
        .filter(|build| {
            build.run.succeeded.as_deref() == Some("True")
                && build.witness(names, sha).is_some_and(|witness| {
                    witness.reported() && witness.commit.eq_ignore_ascii_case(sha)
                })
        })
        .collect();
    let mut conflicts = Vec::new();
    for image in &freight.images {
        let Some(held) = &image.digest else {
            continue;
        };
        let wanted = repository(&image.repo_url);
        let others: Vec<BuiltDigest> = reporting
            .iter()
            .flat_map(|build| {
                build
                    .images()
                    .into_iter()
                    .filter(|built| repository(&built.url) == wanted)
                    .map(|built| BuiltDigest {
                        build: id(&build.run.namespace, &build.run.name),
                        digest: built.digest,
                        completed: build.run.completed.clone(),
                    })
            })
            .collect();
        if !others.is_empty() {
            conflicts.push(DigestConflict {
                repository: wanted,
                freight: held.clone(),
                builds: others,
            });
        }
    }
    conflicts
}

/// The reason of a link with conflicts: which digests the Freight holds and
/// what the builds reported instead.
pub(super) fn conflict_reason(sha: &str, conflicts: &[DigestConflict]) -> String {
    let short: String = sha.chars().take(7).collect();
    conflicts
        .iter()
        .map(|conflict| {
            let builds: Vec<String> = conflict
                .builds
                .iter()
                .map(|built| {
                    let completed = built
                        .completed
                        .as_deref()
                        .map(|time| format!(", completed {time}"))
                        .unwrap_or_default();
                    format!(
                        "{} reported {}{completed}",
                        built.build,
                        built.digest.short()
                    )
                })
                .collect();
            format!(
                "no build read for {short} reports this Freight's digest ({}@{}); {}",
                conflict.repository,
                conflict.freight.short(),
                builds.join("; ")
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}
