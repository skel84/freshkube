//! A change read across the workspace: the Freight a Kargo Stage runs,
//! followed through what Kargo, Argo CD, Tekton and the workloads report,
//! each part where [`Place::sources`] says it is.
//!
//! [`read_in`] reads the Freight first, from Kargo's entry, for the commit
//! it names; then the builds from the builds' entry, Argo CD's
//! Applications from its entry, and each Stage's workloads and pods from
//! the entry its Application's destination maps to ([`Mapping`]), through
//! a [`Pool`] that connects to each entry once ([`delivery::entry::pool`]).
//! [`derive`] turns what was read into the page's [`Change`]. Every link's
//! confidence is the join's ([`join`](crate::join::join) and
//! [`freight_links`](crate::join::freight_links)); the gates
//! word what Kargo's Stages, Freight and Promotions record.
//!
//! The workspace of one ([`read`]) reads everything from the open
//! connection, as before. A destination that wasn't read, because its entry
//! couldn't be reached or, in the workspace of one, isn't the open cluster,
//! leaves its Application's workloads and pods Unknown, never missing: the
//! Stage names it, and the workloads Argo CD reports there are listed as
//! Argo CD's claim, to open on that cluster.
//!
//! [`delivery::entry::pool`]: crate::entry::pool

#[cfg(test)]
mod across_tests;
#[cfg(test)]
mod after_tests;
#[cfg(test)]
mod build_namespace_tests;
#[cfg(test)]
mod cause_tests;
mod derive;
mod gates;
mod history;
mod links;
pub(crate) mod pages;
#[cfg(test)]
mod tests;

use std::future::Future;
use std::time::Duration;

use chrono::{DateTime, Utc};

use super::{Change, Gap, Missing};
use crate::applications::ArgoFound;
use crate::applications::read::{
    argo_namespace_note, argo_namespaces, read_applications_in, read_argo_workloads,
};
use crate::argocd::MapHint;
use crate::collect::{Plan, Upstream, read_environment, upstream_with_history};
use crate::entry::pool::{Open, Pool, PoolReader, Read, Unread, Until};
use crate::github::{GitHub, PullRequest};
use crate::join::{Evidence, candidate_applications, candidate_deployments, candidate_rollouts};
use crate::kargo::{Freight, read_freight};
use crate::read::{ListRequest, Listing, Reader, Resource, ServedGroup};
use crate::source::printable;
use crate::words;
use freshkube_core::resources::{Failure, FailureKind};
use freshkube_core::workspace::{self, Chosen, Entry, Key, Part, Sources};

pub use derive::derive;
pub use pages::Pages;

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
    /// The namespace used for Argo CD's page address. Applications are
    /// read in the namespaces discovered on Argo CD's own entry.
    pub argocd_namespace: String,
    /// Which workspace cluster each Argo CD destination is.
    pub mapping: Mapping,
    /// Where Kargo, the builds and Argo CD are read; the workspace of one's
    /// reads everything from this cluster.
    pub sources: Sources,
    /// The top-level setup's public key, opened afresh off the UI thread.
    pub public_key_file: Option<std::path::PathBuf>,
    /// Whether the log must hold each verified signature. Off by default.
    pub transparency_log: bool,
    /// The setup's log address; absent means read Chains' configuration.
    pub transparency_url: Option<String>,
    /// The pipeline record configured in the same setup.
    pub evidence_result: Option<freshkube_core::setup::Evidence>,
    /// The build namespace or `{project}` template, from the same setup. A build is
    /// searched for by image digest only in it.
    pub build_namespace: Option<String>,
    /// How an Application with no authorized-stage annotation names its
    /// Stage, from the same setup.
    pub stage_naming: Option<freshkube_core::setup::StageNaming>,
}

impl Place {
    /// This cluster's name among the destinations: the open workspace
    /// entry's id, else its key. A destination mapped to the open entry is
    /// this cluster, and its workloads are read here.
    fn here(&self) -> &str {
        self.mapping.open.as_deref().unwrap_or(&self.cluster)
    }

    /// The open cluster, as its objects open and the page names it.
    fn open(&self) -> At {
        At {
            cluster: self.cluster.clone(),
            label: self.label.clone(),
            missing: None,
        }
    }

    /// Where `part` is read: this cluster for the workspace of one and the
    /// open entry, else its entry, by id.
    fn at(&self, part: Part) -> At {
        let chosen = self.sources.get(part);
        match chosen {
            Chosen::Entry { entry, .. } if !self.is_here(&entry.id) => At {
                cluster: entry.id.clone(),
                label: entry.id.clone(),
                missing: None,
            },
            Chosen::Entry { .. } | Chosen::Open => self.open(),
            Chosen::NoEntry(_) | Chosen::Gone(_) | Chosen::Skipped(_) => At {
                cluster: String::new(),
                label: crate::applications::across::WORKSPACE.into(),
                missing: chosen.missing(),
            },
        }
    }

    /// Where the environment `context`, a workspace entry or this
    /// cluster's key, is.
    fn at_context(&self, context: &str) -> At {
        match self.is_here(context) {
            true => self.open(),
            false => At {
                cluster: context.to_owned(),
                label: context.to_owned(),
                missing: None,
            },
        }
    }

    fn is_here(&self, context: &str) -> bool {
        context == self.here() || context == self.cluster
    }

    /// Argo CD's own cluster, which it calls `in-cluster`: its entry, else
    /// this one.
    fn argocd(&self) -> String {
        match self.sources.get(Part::ArgoCd) {
            Chosen::Entry { entry, .. } => entry.id.clone(),
            _ => self.here().to_owned(),
        }
    }
}

/// Where one part of a change was read: the cluster its objects open on,
/// and its name on the page, or why no entry holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct At {
    /// The open connection's key, or the workspace entry's id.
    pub cluster: String,
    pub label: String,
    /// Why no entry holds the part, when none does.
    pub missing: Option<String>,
}

impl At {
    /// A group's words: `Tekton on cicd-fra`, or why the part has no entry.
    fn on(&self, what: &str) -> String {
        match &self.missing {
            Some(why) => format!("{what}: {why}"),
            None => format!("{what} on {}", self.label),
        }
    }
}

/// The parts whose entry couldn't be read at all, by the words that say
/// why in its name. A destination's are in
/// [`Evidence::environments`](crate::join::Evidence::environments).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reached {
    pub kargo: Option<Unread>,
    pub builds: Option<Unread>,
    pub argocd: Option<Unread>,
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

/// How long a change's read may take across the workspace, past which an
/// entry not yet read is left out with why.
pub const BUDGET: Duration = Duration::from_secs(45);

/// The change `place` names, read through `reader` alone: the workspace of
/// one, whatever [`Place::sources`] says. An error says why the Freight
/// itself couldn't be read; anything after it that couldn't be read is part
/// of the change, as Unknown.
pub async fn read<R: Reader>(
    reader: &R,
    place: &Place,
    observed_at: DateTime<Utc>,
) -> Result<Change, Failure> {
    let alone = Place {
        sources: Sources::default(),
        ..place.clone()
    };
    let pool = alone_pool(reader, &alone);
    read_in(&pool, &alone, observed_at).await
}

/// The change of `freight`, already read, through `reader` alone.
pub async fn follow<R: Reader>(
    reader: &R,
    place: &Place,
    freight: &Freight,
    observed_at: DateTime<Utc>,
) -> Change {
    let alone = Place {
        sources: Sources::default(),
        ..place.clone()
    };
    let pool = alone_pool(reader, &alone);
    follow_in(&pool, &alone, freight, observed_at).await
}

/// A pool of the open connection alone.
type Alone<'a, R> = Pool<'a, R, Never, fn(Entry) -> std::future::Ready<Result<Never, Failure>>>;

fn alone_pool<'a, R: Reader>(reader: &'a R, place: &'a Place) -> Alone<'a, R> {
    Pool::new(
        &place.sources,
        Some(Open {
            entry: place.mapping.open.as_deref(),
            reader: Ok(reader),
        }),
        |_| {
            std::future::ready(Err(Failure::new(
                FailureKind::Config,
                "the workspace of one reads the open cluster alone",
            )))
        },
        BUDGET,
    )
}

/// The change `place` names, each part read from its entry through `pool`.
/// An error says why the Freight itself couldn't be read, its entry's
/// name in it; anything after it that couldn't be read is part of the
/// change, as Unknown, in its entry's name.
pub async fn read_in<R, C, F, Fut>(
    pool: &Pool<'_, R, C, F>,
    place: &Place,
    observed_at: DateTime<Utc>,
) -> Result<Change, Failure>
where
    R: Reader,
    C: Reader,
    F: Fn(Entry) -> Fut,
    Fut: Future<Output = Result<C, Failure>>,
{
    let couldnt = |kind: FailureKind, why: String| {
        Failure::new(
            kind,
            format!(
                "Freight {} of {} couldn't be read: {why}",
                place.freight, place.project
            ),
        )
    };
    let kargo = match part(pool, place, Part::Kargo).await {
        Ok((kargo, _)) => kargo,
        Err(unread) => return Err(couldnt(unread.kind(), unread.words())),
    };
    let at = place.at(Part::Kargo);
    // The Freight is the first round's too, so it ends with the budget.
    let until = pool.first();
    let read = until.bound(read_freight(&kargo, &place.project, &place.freight));
    let freight = match read.await {
        Some(Ok(freight)) => freight,
        Some(Err(failure)) => {
            let why = printable(&failure);
            return Err(match failure.kind {
                FailureKind::NotFound => couldnt(failure.kind, words::wasnt_found_on(&at.label)),
                FailureKind::Forbidden | FailureKind::Unauthorized => {
                    couldnt(failure.kind, words::refused_on(&at.label, &why))
                }
                FailureKind::Config
                | FailureKind::Unreachable
                | FailureKind::Timeout
                | FailureKind::Other => Failure::new(
                    failure.kind,
                    format!(
                        "Freight {} of {} couldn't be read on {}: {why}",
                        place.freight, place.project, at.label
                    ),
                ),
            });
        }
        None => {
            let unread = Unread::unfinished(at.label, &until);
            return Err(couldnt(unread.kind(), unread.words()));
        }
    };
    Ok(follow_in(pool, place, &freight, observed_at).await)
}

/// The change of `freight`, already read, each part read from its entry
/// through `pool`: what the Changes list follows for each Freight from the
/// project's own listing.
pub async fn follow_in<R, C, F, Fut>(
    pool: &Pool<'_, R, C, F>,
    place: &Place,
    freight: &Freight,
    observed_at: DateTime<Utc>,
) -> Change
where
    R: Reader,
    C: Reader,
    F: Fn(Entry) -> Fut,
    Fut: Future<Output = Result<C, Failure>>,
{
    let (kargo, builds, argocd) = futures::join!(
        part(pool, place, Part::Kargo),
        part(pool, place, Part::Builds),
        part(pool, place, Part::ArgoCd),
    );
    let reached = Reached {
        kargo: kargo.as_ref().err().cloned(),
        builds: builds.as_ref().err().cloned(),
        argocd: argocd.as_ref().err().cloned(),
    };
    // Every request of the first round ends with the budget; what it
    // was reading names the entry.
    let until = pool.first();
    let (kargo, builds, argocd) = (
        Reaching::of(kargo, until),
        Reaching::of(builds, until),
        Reaching::of(argocd, until),
    );
    let plan = plan(place, freight);
    let upstream_readers = Upstream {
        kargo: &kargo,
        argocd: &argocd,
        tekton: &builds,
        github: &NoGitHub,
    };
    let history_until = until
        .at()
        .unwrap_or_else(|| tokio::time::Instant::now() + BUDGET);
    let history = match &plan.build_namespace {
        Ok(_) => pool.results(history_until).await,
        Err(why) => Err(crate::results::Error::BuildNamespace(why.clone())),
    };
    let mut scope = (Vec::new(), ArgoFound::Default);
    let applications = async {
        scope = argo_namespaces(&read_argo_workloads(&argocd).await);
        read_applications_in(&argocd, &scope.0).await
    };
    let (mut evidence, pages) = futures::join!(
        upstream_with_history(
            &upstream_readers,
            &plan,
            observed_at,
            applications,
            history,
            history_until
        ),
        pages::read(&kargo, &argocd, &place.argocd_namespace),
    );
    read_environments(pool, place, &mut evidence).await;
    // Derivation uses this field only to describe where Applications were
    // looked for; the page address still uses the installation's namespace.
    let described = Place {
        argocd_namespace: scope.0.join(", "),
        ..place.clone()
    };
    let mut change = derive(&described, &evidence, freight, &pages, &reached);
    if let Some(note) = argo_namespace_note(&place.at(Part::ArgoCd).label, &scope.0, &scope.1) {
        change.missing.push(Missing::new(Gap::Unreadable, note));
    }
    change
}

/// The reader of `part`, an entry it couldn't read named as the page
/// names its place: the open cluster by its context.
async fn part<'p, R, C, F, Fut>(
    pool: &'p Pool<'_, R, C, F>,
    place: &Place,
    part: Part,
) -> Result<(PoolReader<'p, R, C>, Read), Unread>
where
    R: Reader,
    C: Reader,
    F: Fn(Entry) -> Fut,
    Fut: Future<Output = Result<C, Failure>>,
{
    let label = place.at(part).label;
    pool.part(part).await.map_err(|unread| unread.named(label))
}

/// Each environment the change's Stages deploy to, read from its entry
/// under a destination's deadline, all at once: the pool connects at most
/// a few at a time. A destination no entry the pool reaches holds, as
/// another entry in the workspace of one, isn't asked: it is named, never
/// read.
async fn read_environments<R, C, F, Fut>(
    pool: &Pool<'_, R, C, F>,
    place: &Place,
    evidence: &mut Evidence,
) where
    R: Reader,
    C: Reader,
    F: Fn(Entry) -> Fut,
    Fut: Future<Output = Result<C, Failure>>,
{
    let mut contexts: Vec<&str> = evidence
        .destinations
        .values()
        .filter_map(|destination| destination.context())
        .filter(|context| place.is_here(context) || pool.sources().entry(context).is_some())
        .collect();
    contexts.sort_unstable();
    contexts.dedup();
    // One with nothing to read for the change is read, empty, with no
    // request: so the join says what wasn't there, never that it is
    // another cluster. Another entry where Argo CD reports a workload this
    // read doesn't look for stays unread, so its Stage shows what Argo CD
    // reports, Claimed.
    let (contexts, empty): (Vec<&str>, Vec<&str>) = contexts.into_iter().partition(|context| {
        !candidate_rollouts(evidence, context).is_empty()
            || !candidate_deployments(evidence, context).is_empty()
            || !candidate_applications(evidence, context, None).is_empty()
    });
    let empty: Vec<String> = empty
        .into_iter()
        .filter(|context| place.is_here(context) || !derive::reported_on(evidence, context))
        .map(str::to_owned)
        .collect();
    evidence.environments.extend(
        empty
            .into_iter()
            .map(|context| (context, Ok(Default::default()))),
    );
    let read = futures::future::join_all(contexts.into_iter().map(|context| {
        let evidence = &*evidence;
        async move {
            let until = pool.destination();
            let reader = match place.is_here(context) {
                true => pool.open(&until).await,
                false => pool.entry(context, &until).await,
            };
            let workloads = match reader {
                Ok((reader, _)) => until
                    .bound(read_environment(&reader, evidence, context))
                    .await
                    .ok_or_else(|| {
                        let entry = place.at_context(context).label;
                        let unread = Unread::unfinished(entry, &until);
                        Failure::new(unread.kind(), unread.words())
                    }),
                Err(unread) => Err(match unread {
                    // The open cluster is named as the page names it.
                    Unread::Unreachable { why, .. } if place.is_here(context) => {
                        let unread = Unread::Unreachable {
                            entry: place.label.clone(),
                            why,
                        };
                        Failure::new(unread.kind(), unread.words())
                    }
                    unread => Failure::new(unread.kind(), unread.words()),
                }),
            };
            (context.to_owned(), workloads)
        }
    }))
    .await;
    evidence.environments.extend(read);
}

/// A part's reader, each request ending by the first round's deadline,
/// or the words why its entry wasn't read, which every request then fails
/// with, so each of its rows says so.
enum Reaching<R> {
    Read { reader: R, until: Until },
    Unread(String),
}

impl<R: Reader> Reaching<R> {
    fn of<W>(reached: Result<(R, W), Unread>, until: Until) -> Self {
        match reached {
            Ok((reader, _)) => Reaching::Read { reader, until },
            Err(unread) => Reaching::Unread(unread.words()),
        }
    }

    fn failure(why: &str) -> Failure {
        Failure::new(FailureKind::Other, why)
    }

    /// `ask` of the reader by the deadline, or why not. A request that
    /// didn't finish doesn't name its entry: what it was reading does.
    async fn ask<'r, T, Fut>(&'r self, ask: impl FnOnce(&'r R) -> Fut) -> Result<T, Failure>
    where
        Fut: Future<Output = Result<T, Failure>>,
    {
        match self {
            Reaching::Read { reader, until } => {
                until.bound(ask(reader)).await.unwrap_or_else(|| {
                    Err(Self::failure(&format!(
                        "not finished within {}",
                        until.within()
                    )))
                })
            }
            Reaching::Unread(why) => Err(Self::failure(why)),
        }
    }
}

impl<R: Reader> Reader for Reaching<R> {
    async fn get(
        &self,
        resource: &Resource,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<serde_json::Value, Failure> {
        self.ask(|reader| reader.get(resource, namespace, name))
            .await
    }
    async fn list(&self, request: &ListRequest) -> Result<Listing, Failure> {
        self.ask(|reader| reader.list(request)).await
    }
    async fn probe(&self, request: &ListRequest) -> Result<(), Failure> {
        self.ask(|reader| reader.probe(request)).await
    }
    async fn groups(&self) -> Result<Vec<ServedGroup>, Failure> {
        self.ask(|reader| reader.groups()).await
    }
    async fn plurals(&self, group: &str, version: &str) -> Result<Vec<String>, Failure> {
        self.ask(|reader| reader.plurals(group, version)).await
    }
}

/// A client never made: the workspace of one connects to nothing.
pub enum Never {}

impl Reader for Never {
    async fn get(
        &self,
        _: &Resource,
        _: Option<&str>,
        _: &str,
    ) -> Result<serde_json::Value, Failure> {
        match *self {}
    }
    async fn list(&self, _: &ListRequest) -> Result<Listing, Failure> {
        match *self {}
    }
    async fn groups(&self) -> Result<Vec<ServedGroup>, Failure> {
        match *self {}
    }
    async fn plurals(&self, _: &str, _: &str) -> Result<Vec<String>, Failure> {
        match *self {}
    }
}

/// `reader` in every role, without GitHub.
#[cfg(test)]
fn clusters<R: Reader>(reader: &R) -> crate::collect::Clusters<'_, R, R, R, R, NoGitHub> {
    crate::collect::Clusters {
        kargo: reader,
        argocd: reader,
        tekton: reader,
        environment: reader,
        github: &NoGitHub,
    }
}

/// What [`upstream_with_history`] reads for `freight` at `place`.
fn plan(place: &Place, freight: &Freight) -> Plan {
    let (contexts, cluster_names) = place.mapping.pairs();
    Plan {
        sha: commit_of(freight).unwrap_or_default().to_owned(),
        digests: freight
            .images
            .iter()
            .filter_map(|image| image.digest.clone())
            .collect(),
        followed: Some(freight.name.clone()),
        kargo_project: place.project.clone(),
        argocd_namespace: place.argocd_namespace.clone(),
        build_namespace: place
            .build_namespace
            .as_deref()
            .map(|template| {
                freshkube_core::setup::resolve_build_namespace(template, &place.project)
            })
            .transpose(),
        github_repo: None,
        contexts,
        cluster_names,
        argocd: place.argocd(),
        environment: place.here().to_owned(),
        evidence_result: place.evidence_result.as_ref().map(|record| {
            crate::tekton::EvidenceResult {
                result: record.result.clone(),
                commit_pointer: record.commit.clone(),
                digest_pointer: record.digest.clone(),
            }
        }),
        public_key_file: place.public_key_file.clone(),
        transparency_log: place.transparency_log,
        transparency_url: place.transparency_url.clone(),
        commit_names: Default::default(),
        stage_naming: place.stage_naming.as_ref().map(Into::into),
        map_hint: match place.mapping.settings {
            true => MapHint::Settings {
                listed: place.mapping.entries.clone(),
            },
            false => MapHint::Flags,
        },
        // This cluster is the open entry, else the connection's key, which
        // nobody reads: its words are the name the page shows. Every other
        // entry is named by its id.
        names: (place.here() != place.label)
            .then(|| (place.here().to_owned(), place.label.clone()))
            .into_iter()
            .chain(
                (place.cluster != place.label)
                    .then(|| (place.cluster.clone(), place.label.clone())),
            )
            .collect(),
    }
}
