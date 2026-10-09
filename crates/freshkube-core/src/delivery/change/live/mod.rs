//! A change read from one cluster: the Freight a Kargo Stage runs, followed
//! through what Kargo, Argo CD, Tekton and the workloads report there.
//!
//! [`read`] reads the Freight first, for the commit it names, then hands
//! everything else to [`collect`](crate::delivery::collect()) with the one
//! cluster in every role, and [`derive`] turns what was read into the
//! page's [`Change`]. Every link's confidence is the join's
//! ([`join`](crate::delivery::join::join) and
//! [`freight_links`](crate::delivery::join::freight_links)); the gates
//! word what Kargo's Stages, Freight and Promotions record.
//!
//! A cluster Argo CD deploys to that isn't this one isn't read: its
//! Application's workloads and pods are Unknown, never missing.

mod derive;
mod gates;
#[cfg(test)]
mod tests;

use chrono::{DateTime, Utc};

use super::Change;
use crate::delivery::collect::{Clusters, Plan, collect};
use crate::delivery::github::{GitHub, PullRequest};
use crate::delivery::kargo::{Freight, read_freight};
use crate::delivery::read::Reader;
use crate::delivery::source::Source;
use crate::resources::{Failure, FailureKind};

pub use derive::derive;

/// Where a change is read: one cluster, one Kargo project, one Freight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// The cluster's key, which the objects to open carry.
    pub cluster: String,
    /// The cluster's name, which the page shows.
    pub label: String,
    pub project: String,
    /// The Freight's name, its content hash.
    pub freight: String,
    /// The namespace Argo CD's Applications are read in.
    pub argocd_namespace: String,
}

/// No pull requests are read: GitHub is never called from the app.
struct NoGitHub;

impl GitHub for NoGitHub {
    async fn pulls_for_commit(&self, _: &str, _: &str) -> Result<Vec<PullRequest>, Failure> {
        Err(Failure::new(
            FailureKind::Other,
            "pull requests aren't read",
        ))
    }

    async fn pull(&self, _: &str, _: u64) -> Result<PullRequest, Failure> {
        Err(Failure::new(
            FailureKind::Other,
            "pull requests aren't read",
        ))
    }
}

/// The commit a Freight names: its first commit, else an image's OCI
/// revision annotation.
pub fn commit_of(freight: &Freight) -> Option<&str> {
    freight
        .commits
        .first()
        .map(|commit| commit.id.as_str())
        .or_else(|| {
            freight
                .images
                .iter()
                .find_map(|image| image.revision.as_deref())
        })
}

/// The change `place` names, read through `reader` alone. An error says why
/// the Freight itself couldn't be read; anything after it that couldn't be
/// read is part of the change, as Unknown.
pub async fn read<R: Reader>(
    reader: &R,
    place: &Place,
    observed_at: DateTime<Utc>,
) -> Result<Change, String> {
    let freight = match read_freight(reader, &place.project, &place.freight).await {
        Ok(freight) => freight,
        Err(failure) => {
            let why = match Source::<()>::from_failure(failure) {
                Source::NotInstalled(_) => "it wasn't found".to_owned(),
                Source::Refused(why) => format!("refused: {why}"),
                Source::Unreadable(why) => why,
                Source::Read(()) | Source::Capped(..) => String::new(),
            };
            return Err(format!(
                "Freight {} of {} couldn't be read: {why}",
                place.freight, place.project
            ));
        }
    };
    let plan = Plan {
        sha: commit_of(&freight).unwrap_or_default().to_owned(),
        followed: Some(freight.name.clone()),
        kargo_project: place.project.clone(),
        argocd_namespace: place.argocd_namespace.clone(),
        build_namespace: None,
        github_repo: None,
        contexts: Vec::new(),
        cluster_names: Vec::new(),
        argocd: place.cluster.clone(),
        environment: place.cluster.clone(),
        evidence_result: None,
        commit_names: Default::default(),
        stage_naming: None,
    };
    let clusters = Clusters {
        kargo: reader,
        argocd: reader,
        tekton: reader,
        environment: reader,
        github: &NoGitHub,
    };
    let evidence = collect(&clusters, &plan, observed_at).await;
    Ok(derive(place, &evidence, &freight))
}
