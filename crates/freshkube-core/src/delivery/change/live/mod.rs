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
//! Application's workloads and pods are Unknown, never missing. Where the
//! person mapped the destination to another workspace cluster
//! ([`Mapping`]), the Stage names it, and the workloads Argo CD reports
//! there are listed as Argo CD's claim, to open on that cluster.

mod derive;
mod gates;
mod links;
mod pages;
#[cfg(test)]
mod tests;

use chrono::{DateTime, Utc};

use super::Change;
use crate::delivery::argocd::MapHint;
use crate::delivery::collect::{Clusters, Plan, collect};
use crate::delivery::github::{GitHub, PullRequest};
use crate::delivery::kargo::{Freight, read_freight};
use crate::delivery::read::Reader;
use crate::delivery::source::Source;
use crate::resources::{Failure, FailureKind};
use crate::workspace::{self, Key};

pub use derive::derive;
pub use pages::{KARGO_NAMESPACE, Pages};

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
    /// Which workspace cluster each Argo CD destination is.
    pub mapping: Mapping,
}

impl Place {
    /// This cluster's name among the destinations: the open workspace
    /// entry's id, else its key. A destination mapped to the open entry is
    /// this cluster, and its workloads are read here.
    fn here(&self) -> &str {
        self.mapping.open.as_deref().unwrap_or(&self.cluster)
    }
}

/// `(entry, server or name)`, as [`Plan`] takes contexts.
type Pairs = Vec<(String, String)>;

/// What the workspace says about where Argo CD deploys: the rows the person
/// mapped, the entries it lists, and the one open. Nothing else is matched:
/// kubeconfig servers are never compared.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mapping {
    /// The workspace entry open in the window, when it opened one.
    pub open: Option<String>,
    /// Every entry the workspace lists, by id.
    pub entries: Vec<String>,
    pub destinations: Vec<workspace::Destination>,
    /// Whether Settings › Workspace can map a destination, so the reasons
    /// beside an unmatched one say to map it there; until it can, they keep
    /// the delivery spike's words.
    pub settings: bool,
}

impl Mapping {
    /// From a workspace, with `open` the entry open in the window.
    pub fn of(workspace: &workspace::Workspace, open: Option<&str>) -> Self {
        Self {
            open: open.map(str::to_owned),
            entries: workspace.clusters.iter().map(|e| e.id.clone()).collect(),
            destinations: workspace.destinations.clone(),
            settings: true,
        }
    }

    /// Whether the workspace lists the entry.
    fn lists(&self, entry: &str) -> bool {
        self.entries.iter().any(|listed| listed == entry)
    }

    /// `(entry, server)` and `(entry, name)` of every row, as the
    /// destination match takes contexts. A row whose entry the workspace no
    /// longer lists is matched too, so the Stage can say so.
    fn pairs(&self) -> (Pairs, Pairs) {
        let mut servers = Vec::new();
        let mut names = Vec::new();
        for row in &self.destinations {
            match row.key() {
                Key::Server(server) => servers.push((row.entry.clone(), server)),
                Key::Name(name) => names.push((row.entry.clone(), name)),
            }
        }
        (servers, names)
    }
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
    let (contexts, cluster_names) = place.mapping.pairs();
    let plan = Plan {
        sha: commit_of(&freight).unwrap_or_default().to_owned(),
        followed: Some(freight.name.clone()),
        kargo_project: place.project.clone(),
        argocd_namespace: place.argocd_namespace.clone(),
        build_namespace: None,
        github_repo: None,
        contexts,
        cluster_names,
        argocd: place.here().to_owned(),
        environment: place.here().to_owned(),
        evidence_result: None,
        commit_names: Default::default(),
        stage_naming: None,
        map_hint: match place.mapping.settings {
            true => MapHint::Settings {
                listed: place.mapping.entries.clone(),
            },
            false => MapHint::Flags,
        },
    };
    let clusters = Clusters {
        kargo: reader,
        argocd: reader,
        tekton: reader,
        environment: reader,
        github: &NoGitHub,
    };
    let (evidence, pages) = futures::join!(
        collect(&clusters, &plan, observed_at),
        pages::read(reader, &place.argocd_namespace),
    );
    Ok(derive(place, &evidence, &freight, &pages))
}
