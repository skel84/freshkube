//! The join: from one commit SHA to the pods running its digest.
//!
//! Only two keys join records from different tools, the commit SHA and the
//! image digest. Tags are carried for display and never compared. Records
//! that belong together by name or annotation only (a Stage listing a
//! Freight, an Application naming the Stage that may sync it) are *claims*
//! until a SHA or digest matches on both sides; a source that couldn't be
//! read is *unknown*, with the reason, and is never the same as empty.
//!
//! This module reads nothing: [`join`] is a pure function of [`Evidence`].

mod argo;
mod build;
mod cause;
mod conflict;
mod deployment;
mod kargo;
mod observe;
mod render;
mod workload;

use argo::rollout_namespace;
use build::{commit_link, digest_links, pull_request_links, supply_chain_links};
pub(crate) use cause::{PROMOTIONS, STAGES, unread};
#[cfg(test)]
pub(crate) use conflict::BuiltDigest;
pub(crate) use conflict::DigestConflict;
use conflict::{conflict_reason, digest_conflicts};
use kargo::{freight_summary, stage_links, warehouse_link};
pub use render::render;

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use super::argocd::{
    Application, DestinationMatch, ManagedObject, MapHint, StageClaim, StageNaming,
};
use super::change::{Cause, SettingsField};
use super::deployments::{Deployment, DeploymentSet};
use super::digest::Digest;
use super::github::PullRequest;
use super::kargo::{Freight, KargoRead, Stage};
use super::observation::{Fact, Observation};
use super::pods::RunningImage;
use super::rekor::LogRead;
use super::rollouts::{AnalysisRun, ReplicaSet, Rollout};
use super::source::{Source, cap_note};
use super::tekton::{Build, CommitNames, EvidenceResult, FoundBy};
use super::verify::Trust;
use freshkube_core::resources::Failure;

/// How sure a link is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// The same SHA or digest is on both sides.
    Confirmed,
    /// Only a label, annotation, name or status says so.
    Claimed,
    /// A side couldn't be read, or nothing joins.
    Unknown,
}

impl Confidence {
    pub fn word(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Claimed => "claimed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hop {
    PullRequest,
    Commit,
    /// A Freight's image, named by its digest: the start of a build found
    /// by that digest when no commit is known.
    Image,
    PipelineRun,
    SupplyChain,
    Freight,
    Warehouse,
    Promotion,
    Stage,
    Application,
    Rollout,
    Deployment,
    Pod,
}

impl Hop {
    pub fn word(self) -> &'static str {
        match self {
            Self::PullRequest => "pull request",
            Self::Commit => "commit",
            Self::Image => "image",
            Self::PipelineRun => "PipelineRun",
            Self::SupplyChain => "supply chain",
            Self::Freight => "Freight",
            Self::Warehouse => "Warehouse",
            Self::Promotion => "Promotion",
            Self::Stage => "Stage",
            Self::Application => "Application",
            Self::Rollout => "Rollout",
            Self::Deployment => "Deployment",
            Self::Pod => "pods",
        }
    }
}

/// What a link was joined on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Sha(String),
    Digest(Digest),
    /// A name, label or annotation: never a join key, only a claim.
    Name(String),
    None,
}

impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sha(sha) => write!(f, "sha {}", sha.chars().take(12).collect::<String>()),
            Self::Digest(digest) => write!(f, "{}", digest.short()),
            Self::Name(name) => write!(f, "name {name}"),
            Self::None => f.write_str("-"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub from: Hop,
    pub to: Hop,
    /// What the link is about: an object, `namespace/name`.
    pub subject: String,
    pub key: Key,
    pub confidence: Confidence,
    pub reason: String,
    /// What was read to say so, from which objects. Empty when nothing was
    /// read: a source that couldn't be read leaves its links with none.
    pub evidence: Vec<Observation>,
    /// A SHA-joined Freight's images that no read build reports by their
    /// digest while a build of the commit reports another; empty otherwise.
    /// Never a failure: rebuilds of one commit report different digests.
    pub conflicts: Vec<DigestConflict>,
    /// Whether the supply-chain link's signature is marked failing: true
    /// exactly when the key the person named can't be used, the signature
    /// doesn't verify with it, the attestation can't be read, or no subject
    /// of it is the build's digest. A link with no key, no attestation found,
    /// or a verified signature leaves it false. Shown as the hop's Warn
    /// status, the way a digest conflict is.
    pub signature_warn: bool,
    /// Why the link is Unknown, when the site that made it knew from the
    /// source's answer or the setup; `None` for a link that isn't Unknown,
    /// or whose cause only a sentence would tell.
    pub cause: Option<Cause>,
}

impl Link {
    fn new(
        from: Hop,
        to: Hop,
        subject: impl Into<String>,
        key: Key,
        confidence: Confidence,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            from,
            to,
            subject: subject.into(),
            key,
            confidence,
            reason: reason.into(),
            evidence: Vec::new(),
            conflicts: Vec::new(),
            signature_warn: false,
            cause: None,
        }
    }

    /// The link with the cause of its being Unknown.
    fn because(mut self, cause: Option<Cause>) -> Self {
        self.cause = cause;
        self
    }

    /// The link with its signature marked failing, or not.
    fn warned(mut self, warn: bool) -> Self {
        self.signature_warn = warn;
        self
    }

    /// The link with what it was read from.
    fn observed(mut self, evidence: Vec<Observation>) -> Self {
        self.evidence = evidence;
        self
    }
}

/// The search of the build namespace for builds whose results report the
/// Freight's image digests, and how it went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DigestSearch {
    pub digests: Vec<Digest>,
    pub outcome: DigestOutcome,
    /// What the archive read of the same search came back as; `None` when it
    /// wasn't asked.
    pub archive: Option<Result<crate::results::History, crate::results::Error>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DigestOutcome {
    /// No build namespace is set, so nothing was listed.
    NoNamespace,
    Read(crate::tekton::Searched),
    Failed(String),
}

impl DigestSearch {
    /// Why no build was found by digest: not looked for without a build
    /// namespace, or looked for and not there. A failed search says only
    /// its words.
    pub(crate) fn cause(&self) -> Option<Cause> {
        match &self.outcome {
            DigestOutcome::NoNamespace => Some(Cause::not_set_up(
                "a build namespace to search for a build that reports the image digest",
                SettingsField::BUILD_NAMESPACE,
            )),
            DigestOutcome::Failed(_) => None,
            DigestOutcome::Read(_) => Some(Cause::not_found(
                "a PipelineRun or TaskRun result that reports the image digest",
            )),
        }
    }

    /// Why no build was found by digest, or none was looked for.
    pub(crate) fn why_none(&self) -> String {
        let digests = self
            .digests
            .iter()
            .map(Digest::short)
            .collect::<Vec<_>>()
            .join(", ");
        match &self.outcome {
            DigestOutcome::NoNamespace => {
                "no build namespace is set (delivery.build_namespace in freshkube.yaml), so builds can't be searched by image digest".to_owned()
            }
            DigestOutcome::Failed(why) => {
                format!("builds couldn't be searched by image digest ({digests}): {why}")
            }
            DigestOutcome::Read(searched) => format!(
                "no PipelineRun or TaskRun result reports image digest {digests} ({})",
                searched.words()
            ),
        }
    }
}

/// What a signature is checked against, and what the transparency log said:
/// the evidence's, handed to the links that can confirm.
pub(crate) struct Proof<'a> {
    pub(super) trust: &'a Trust,
    pub(super) log: &'a LogRead,
}

impl Proof<'static> {
    /// No key and no log: nothing verifies. The pull request links use it, so
    /// a verified provenance, which is the commit's own witness, cannot
    /// raise them.
    pub(super) fn none() -> Self {
        static NO_TRUST: Trust = Trust::Unset;
        static NO_LOG: LogRead = LogRead::Off;
        Proof {
            trust: &NO_TRUST,
            log: &NO_LOG,
        }
    }
}

/// Everything read for one change, each part with its own outcome.
pub struct Evidence {
    pub sha: String,
    /// A Freight followed by name: its Stages' workloads count as reached
    /// whether or not it matches the commit.
    pub followed: Option<String>,
    /// When the caller read it all. The caller's clock, never read here.
    pub observed_at: DateTime<Utc>,
    /// Where the pipeline's own evidence record is; `None` when the caller
    /// configured none.
    pub evidence_result: Option<EvidenceResult>,
    /// What signatures are verified against. [`Trust::Unset`] verifies
    /// nothing.
    pub(crate) trust: Trust,
    /// What the transparency log said, when the person turned it on.
    pub(crate) log: LogRead,
    /// Parameter and result names, besides upstream's, that name a build's
    /// commit.
    pub commit_names: CommitNames,
    /// How Applications are tied to stages where they carry no annotation;
    /// `None` when the caller configured none.
    pub stage_naming: Option<StageNaming>,
    /// The namespace Argo CD's Applications were read in.
    pub argocd_namespace: String,
    /// The namespace the builds were read in, when the setup names one;
    /// without it they are read across all of them.
    pub build_namespace: Option<String>,
    /// How a destination no context matches says it could be matched.
    pub map_hint: MapHint,
    /// The words for a context, by its key, where they differ ([`Evidence::name`]).
    pub names: BTreeMap<String, String>,
    pub builds: Source<Vec<Build>>,
    /// What the search of builds by image digest read, when one was asked.
    pub(crate) digest_search: Option<DigestSearch>,
    pub(crate) build_history: Result<crate::results::History, crate::results::Error>,
    pub kargo: KargoRead,
    pub applications: Source<Vec<Application>>,
    /// Where each Application's destination points, by `namespace/name`.
    pub destinations: BTreeMap<String, DestinationMatch>,
    /// Each environment cluster asked, by context: its Rollouts,
    /// Deployments and pods, or why it couldn't be read. An Application's
    /// workloads are read, and judged, only in its own destination's.
    pub environments: BTreeMap<String, Result<Workloads, Failure>>,
    /// Pull requests GitHub associates with the commit; `None` when GitHub
    /// wasn't asked.
    pub pull_requests: Option<Source<Vec<PullRequest>>>,
    /// Builds on a pull request's head commit, by PR number, for the PRs whose
    /// head is not the commit itself (a squash or rebase merge).
    pub pr_builds: BTreeMap<u64, Source<Vec<Build>>>,
}

impl Evidence {
    /// How the Application claims the Stage. A claim by name only that more
    /// than one Application makes for the Stage is no claim: the name can't
    /// say which is the Stage's.
    pub(crate) fn claims_stage(&self, app: &Application, stage: &Stage) -> Option<StageClaim> {
        let claim = app.claims_stage(&stage.project, &stage.name, self.stage_naming.as_ref())?;
        (claim != StageClaim::NamedOnly || self.name_only_claimants(stage) <= 1).then_some(claim)
    }

    /// How many Applications read claim the Stage by name only.
    pub(crate) fn name_only_claimants(&self, stage: &Stage) -> usize {
        self.applications.read().map_or(0, |apps| {
            apps.iter()
                .filter(|app| {
                    app.claims_stage(&stage.project, &stage.name, self.stage_naming.as_ref())
                        == Some(StageClaim::NamedOnly)
                })
                .count()
        })
    }

    /// What signatures are checked against, for the links that can confirm.
    pub(super) fn proof(&self) -> Proof<'_> {
        Proof {
            trust: &self.trust,
            log: &self.log,
        }
    }

    /// How a context is named in words: the name given for its key, else
    /// the key itself, which a kubeconfig context or a workspace entry is.
    pub fn name<'a>(&'a self, context: &'a str) -> &'a str {
        self.names.get(context).map_or(context, String::as_str)
    }

    /// The context the Application's destination is, when it is one.
    pub fn destination_of(&self, app_id: &str) -> Option<&str> {
        self.destinations
            .get(app_id)
            .and_then(DestinationMatch::context)
    }

    /// What the Application's destination holds: `None` when it is no
    /// environment cluster asked, so nothing was read there for it; the
    /// why when it was asked but couldn't be read. A workload of the same
    /// name in another cluster says nothing about this one.
    pub fn workloads_of(&self, app_id: &str) -> Option<Result<&Workloads, &str>> {
        let context = self.destination_of(app_id)?;
        let read = self.environments.get(context)?;
        Some(read.as_ref().map_err(|why| why.message.as_str()))
    }

    /// Whether the Application's destination is an environment cluster
    /// that was read.
    pub fn deploys_to_environment(&self, app_id: &str) -> bool {
        matches!(self.workloads_of(app_id), Some(Ok(_)))
    }

    /// The environment cluster, when exactly one was asked: the
    /// command-line read's, whose words name it.
    pub fn environment(&self) -> Option<&str> {
        match self.environments.len() {
            1 => self.environments.keys().next().map(String::as_str),
            _ => None,
        }
    }
}

/// What one environment cluster holds of a change.
#[derive(Clone, Debug)]
pub struct Workloads {
    pub rollouts: Source<Vec<Rollout>>,
    pub analysis_runs: Source<Vec<AnalysisRun>>,
    /// The ReplicaSets of the Rollouts' namespaces.
    pub replica_sets: Source<Vec<ReplicaSet>>,
    /// Pods read for each Rollout, by `namespace/name`.
    pub pods: BTreeMap<String, RolloutPods>,
    /// The Deployments of the namespaces the change reaches.
    pub deployments: Source<Vec<Deployment>>,
    /// The ReplicaSets those namespaces' Deployments own.
    pub deployment_sets: Source<Vec<DeploymentSet>>,
    /// Pods read for each Deployment, by `namespace/name`: those of the
    /// current ReplicaSet's pod-template hash.
    pub deployment_pods: BTreeMap<String, RolloutPods>,
    /// Pods read in an Application's destination namespace, by the
    /// Application's `namespace/name`, for workloads that aren't Rollouts.
    pub namespace_pods: BTreeMap<String, Source<Vec<RunningImage>>>,
}

/// Nothing asked yet: every kind read and empty.
impl Default for Workloads {
    fn default() -> Self {
        Self {
            rollouts: Source::Read(Vec::new()),
            analysis_runs: Source::Read(Vec::new()),
            replica_sets: Source::Read(Vec::new()),
            pods: BTreeMap::new(),
            deployments: Source::Read(Vec::new()),
            deployment_sets: Source::Read(Vec::new()),
            deployment_pods: BTreeMap::new(),
            namespace_pods: BTreeMap::new(),
        }
    }
}

/// The pods read for one Rollout: those labelled with the pod-template hash
/// its status reports as current. During a canary or a blue-green rollout
/// the stable ReplicaSet's pods run the previous revision, so only the
/// current revision's are judged.
#[derive(Clone, Debug)]
pub struct RolloutPods {
    pub hash: String,
    pub pods: Source<Vec<RunningImage>>,
}

pub(super) fn owned_replica_sets<'a>(
    rollout: &'a Rollout,
    replica_sets: Option<&'a [ReplicaSet]>,
) -> impl Iterator<Item = &'a ReplicaSet> {
    replica_sets.into_iter().flatten().filter(|set| {
        set.namespace == rollout.namespace
            && rollout.meta.uid.is_some()
            && set.owner_uid == rollout.meta.uid
    })
}

/// A Rollout the change reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WantedRollout {
    pub namespace: String,
    pub name: String,
}

/// The trail of one change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trail {
    pub sha: String,
    /// When the evidence was read, as [`Evidence::observed_at`].
    pub observed_at: DateTime<Utc>,
    pub links: Vec<Link>,
}

impl Trail {
    /// The best confidence of any link into the pods, if the chain got there.
    pub fn running(&self) -> Option<Confidence> {
        self.links
            .iter()
            .filter(|link| link.to == Hop::Pod)
            .map(|link| link.confidence)
            .min()
    }

    /// One line: how many links of each confidence, whether the change was
    /// found running, and, when not, the first link on the way to the pods
    /// that stopped it.
    pub fn summary(&self) -> String {
        let count = |confidence| {
            self.links
                .iter()
                .filter(|link| link.confidence == confidence)
                .count()
        };
        let mut counts = format!(
            "{} confirmed, {} claimed, {} unknown",
            count(Confidence::Confirmed),
            count(Confidence::Claimed),
            count(Confidence::Unknown)
        );
        match self
            .links
            .iter()
            .filter(|link| !link.conflicts.is_empty())
            .count()
        {
            0 => {}
            1 => counts.push_str("; 1 link has a digest no read build reports"),
            n => counts.push_str(&format!("; {n} links have a digest no read build reports")),
        }
        let running = match self.running() {
            Some(Confidence::Confirmed) => "running, confirmed: pods run the digest".to_owned(),
            Some(Confidence::Claimed) => {
                "running only as claimed: no pod is seen running the digest".to_owned()
            }
            _ => {
                // The links the pods are reached through; a Promotion or the
                // supply chain is beside the way, and a pull request before it.
                let stop = self.links.iter().find(|link| {
                    link.confidence == Confidence::Unknown
                        && link.from != Hop::PullRequest
                        && !matches!(link.to, Hop::Promotion | Hop::SupplyChain | Hop::Warehouse)
                });
                match stop {
                    Some(link) => format!(
                        "not found running, stopped at {} -> {} {}",
                        link.from.word(),
                        link.to.word(),
                        link.subject
                    ),
                    None => "not found running".to_owned(),
                }
            }
        };
        format!("{running}; {counts}")
    }
}

fn id(namespace: &str, name: &str) -> String {
    format!("{namespace}/{name}")
}

/// The Freight a build's images or the commit lead to, with how.
fn matching_freight<'a>(
    sha: &str,
    builds: &[Build],
    freight: &'a [Freight],
) -> Vec<(&'a Freight, Key)> {
    let built: Vec<Digest> = builds
        .iter()
        .flat_map(|build| build.images())
        .map(|image| image.digest)
        .collect();
    let mut found = Vec::new();
    for item in freight {
        let by_digest = item
            .images
            .iter()
            .filter_map(|image| image.digest.as_ref())
            .find(|digest| built.contains(digest));
        if let Some(digest) = by_digest {
            found.push((item, Key::Digest(digest.clone())));
        } else if !sha.is_empty()
            && item
                .commits
                .iter()
                .any(|commit| commit.id.eq_ignore_ascii_case(sha))
        {
            found.push((item, Key::Sha(sha.to_owned())));
        }
    }
    found
}

/// The Freight whose Stages the change reached: those matching the commit
/// or a built digest, and the one followed by name.
fn reached<'a>(evidence: &Evidence, builds: &[Build], freight: &'a [Freight]) -> Vec<&'a Freight> {
    let mut reached: Vec<&Freight> = matching_freight(&evidence.sha, builds, freight)
        .into_iter()
        .map(|(item, _)| item)
        .collect();
    if let Some(followed) = &evidence.followed
        && let Some(item) = freight.iter().find(|item| &item.name == followed)
        && !reached.iter().any(|item| &item.name == followed)
    {
        reached.push(item);
    }
    reached
}

/// The link into a Freight the change reached: from the PipelineRun when the
/// join has a build to stand on, else from the commit itself.
///
/// On a digest, a build reported the image. On the SHA, only a result of a
/// build reports the commit; a build tied to it by its PaC label alone makes
/// the link a claim, and with no build at all the Freight's own commit stands
/// against the change.
///
/// A build that reports the commit and another digest for one of the
/// Freight's images, when no read build reports the Freight's, leaves the
/// link a claim with the digests as its conflicts.
fn build_freight(
    builds: &[Build],
    names: &CommitNames,
    sha: &str,
    freight: &Freight,
    key: &Key,
    builds_capped: bool,
) -> (
    Hop,
    Confidence,
    String,
    Vec<Observation>,
    Vec<DigestConflict>,
) {
    let summary = freight_summary(freight, sha);
    if let Key::Digest(digest) = key {
        let mut seen = observe::builds_side(builds, names, key);
        seen.extend(observe::freight_side(freight, key));
        return digest_freight(
            builds,
            names,
            sha,
            freight,
            digest,
            builds_capped,
            summary,
            seen,
        );
    }
    // A build found by an image digest says nothing of the commit's builds.
    let of_commit: Vec<Build>;
    let builds = if builds.iter().all(|b| b.found_by == FoundBy::Commit) {
        builds
    } else {
        of_commit = builds
            .iter()
            .filter(|b| b.found_by == FoundBy::Commit)
            .cloned()
            .collect();
        &of_commit
    };
    let mut seen = observe::builds_side(builds, names, key);
    if builds.is_empty() {
        let mut seen = vec![observe::change(sha)];
        seen.extend(observe::freight_side(freight, key));
        return (
            Hop::Commit,
            Confidence::Confirmed,
            summary,
            seen,
            Vec::new(),
        );
    }
    let reported = seen.iter().any(|seen| seen.fact == Fact::Reported);
    seen.extend(observe::freight_side(freight, key));
    let conflicts = digest_conflicts(
        &builds.iter().collect::<Vec<_>>(),
        names,
        sha,
        freight,
        builds_capped,
    );
    if !conflicts.is_empty() {
        seen.retain(|seen| seen.fact == Fact::Reported);
        seen.extend(observe::conflict_sides(builds, freight, &conflicts));
        let reason = format!("{}; {summary}", conflict_reason(sha, &conflicts));
        (
            Hop::PipelineRun,
            Confidence::Claimed,
            reason,
            seen,
            conflicts,
        )
    } else if reported {
        seen.retain(|seen| seen.fact == Fact::Reported);
        (
            Hop::PipelineRun,
            Confidence::Confirmed,
            summary,
            seen,
            Vec::new(),
        )
    } else {
        let tie = if builds
            .iter()
            .any(|build| !build.reported_witnesses(names).is_empty())
        {
            "that a result of the build reports another commit"
        } else if builds.iter().any(|build| {
            build
                .witness(names, sha)
                .is_some_and(|witness| !witness.reported())
        }) {
            "the PaC label and the revision parameter, both declared"
        } else {
            "its declared label"
        };
        (
            Hop::PipelineRun,
            Confidence::Claimed,
            format!("the Freight reports the commit; the build's tie to it is {tie}; {summary}"),
            seen,
            Vec::new(),
        )
    }
}

/// The link into a Freight joined on a digest a build reported.
///
/// Confirmed, unless only builds found by that digest report it while the
/// change names a commit and the read finds the two apart: a build of the
/// commit reports another digest for the Freight's image, or the build
/// found by the digest reports another commit. The digest still joins, and
/// the link says the commit does not agree, as a claim.
#[allow(clippy::too_many_arguments)]
fn digest_freight(
    builds: &[Build],
    names: &CommitNames,
    sha: &str,
    freight: &Freight,
    digest: &Digest,
    builds_capped: bool,
    summary: String,
    mut seen: Vec<Observation>,
) -> (
    Hop,
    Confidence,
    String,
    Vec<Observation>,
    Vec<DigestConflict>,
) {
    let reporting: Vec<&Build> = builds
        .iter()
        .filter(|build| build.images().iter().any(|image| &image.digest == digest))
        .collect();
    let only_by_digest = !reporting.is_empty()
        && reporting
            .iter()
            .all(|build| build.found_by == crate::tekton::FoundBy::Digest);
    if sha.is_empty() || !only_by_digest {
        return (
            Hop::PipelineRun,
            Confidence::Confirmed,
            summary,
            seen,
            Vec::new(),
        );
    }
    let of_commit: Vec<&Build> = builds
        .iter()
        .filter(|build| build.found_by == crate::tekton::FoundBy::Commit)
        .collect();
    let conflicts = digest_conflicts(&of_commit, names, sha, freight, builds_capped);
    let other_commits: Vec<String> = reporting
        .iter()
        .flat_map(|build| build.reported_witnesses(names))
        .map(|witness| witness.commit)
        .filter(|commit| !commit.eq_ignore_ascii_case(sha))
        .map(|commit| argo::short_commit(&commit))
        .collect();
    if conflicts.is_empty() && other_commits.is_empty() {
        return (
            Hop::PipelineRun,
            Confidence::Confirmed,
            summary,
            seen,
            Vec::new(),
        );
    }
    let mut why = Vec::new();
    if !other_commits.is_empty() {
        why.push(format!(
            "the build that reports the digest reports commit {}, not {}",
            other_commits.join(", "),
            argo::short_commit(sha)
        ));
    }
    if !conflicts.is_empty() {
        why.push(conflict_reason(sha, &conflicts));
        seen.extend(observe::conflict_sides(builds, freight, &conflicts));
    }
    (
        Hop::PipelineRun,
        Confidence::Claimed,
        format!(
            "joined on the digest, but the commit does not agree: {}; {summary}",
            why.join("; ")
        ),
        seen,
        conflicts,
    )
}

/// The Rollouts whose pods should be read in the environment `context`:
/// those an Application managed by a Stage that reports one of `freight` as
/// current points at, when the Application deploys there. Used by the
/// collector so pods are read only for the change in question.
pub(crate) fn candidate_rollouts(evidence: &Evidence, context: &str) -> Vec<WantedRollout> {
    candidate_workloads(evidence, context, ControllerKind::Rollout)
}

/// The Deployments whose pods should be read, as [`candidate_rollouts`] finds
/// Rollouts.
pub(crate) fn candidate_deployments(evidence: &Evidence, context: &str) -> Vec<WantedRollout> {
    candidate_workloads(evidence, context, ControllerKind::Deployment)
}

/// The workload controllers an Application manages that join pods by owner.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ControllerKind {
    Rollout,
    Deployment,
}

impl ControllerKind {
    fn is(self, object: &ManagedObject) -> bool {
        match self {
            Self::Rollout => object.kind == "Rollout",
            // A namesake in another group is another object.
            Self::Deployment => object.group == "apps" && object.kind == "Deployment",
        }
    }
}

/// Whether Argo CD lists a Rollout among the Application's objects.
fn manages_rollout(app: &Application) -> bool {
    app.managed.iter().any(|m| ControllerKind::Rollout.is(m))
}

fn candidate_workloads(
    evidence: &Evidence,
    context: &str,
    controller: ControllerKind,
) -> Vec<WantedRollout> {
    // Builds that couldn't be read leave the Freight joined on the commit
    // alone, as `join` joins it.
    let builds = evidence.builds.read().map_or(&[][..], Vec::as_slice);
    let (Some(freight), Some(stages), Some(apps)) = (
        evidence.kargo.freight.read(),
        evidence.kargo.stages.read(),
        evidence.applications.read(),
    ) else {
        return Vec::new();
    };
    let mut wanted: Vec<WantedRollout> = Vec::new();
    for item in reached(evidence, builds, freight) {
        for stage in stages
            .iter()
            .filter(|stage| stage.current_freight.contains(&item.name))
        {
            for app in apps.iter().filter(|app| {
                evidence.claims_stage(app, stage).is_some()
                    && evidence.destination_of(&id(&app.namespace, &app.name)) == Some(context)
            }) {
                for managed in app.managed.iter().filter(|m| controller.is(m)) {
                    let Some(namespace) = rollout_namespace(app, managed) else {
                        continue;
                    };
                    let rollout = WantedRollout {
                        namespace,
                        name: managed.name.clone(),
                    };
                    if !wanted.contains(&rollout) {
                        wanted.push(rollout);
                    }
                }
            }
        }
    }
    wanted
}

/// The Applications of a stage that holds the change's Freight and manage no
/// Rollout, nor a Deployment of the Freight's repository, with the
/// namespace their workload runs in: the pods there are read instead, when
/// the destination is the environment `context`.
///
/// `read` is what was read in `context` so far, whose Deployments say
/// whether the Deployment chain speaks for an Application; without it,
/// what `evidence` holds for `context`.
pub(crate) fn candidate_applications(
    evidence: &Evidence,
    context: &str,
    read: Option<&Workloads>,
) -> Vec<(String, String)> {
    let read = read.or_else(|| {
        evidence
            .environments
            .get(context)
            .and_then(|read| read.as_ref().ok())
    });
    // Builds that couldn't be read leave the Freight joined on the commit
    // alone, as `join` joins it.
    let builds = evidence.builds.read().map_or(&[][..], Vec::as_slice);
    let (Some(freight), Some(stages), Some(apps)) = (
        evidence.kargo.freight.read(),
        evidence.kargo.stages.read(),
        evidence.applications.read(),
    ) else {
        return Vec::new();
    };
    let mut wanted = Vec::new();
    for item in reached(evidence, builds, freight) {
        for stage in stages
            .iter()
            .filter(|stage| stage.current_freight.contains(&item.name))
        {
            for app in apps.iter().filter(|app| {
                evidence.claims_stage(app, stage).is_some()
                    && !manages_rollout(app)
                    && !deployment::speaks_for_in(read, item, app)
                    && evidence.destination_of(&id(&app.namespace, &app.name)) == Some(context)
            }) {
                let Some(namespace) = app.destination_namespace.clone() else {
                    continue;
                };
                let key = (id(&app.namespace, &app.name), namespace);
                if !wanted.contains(&key) {
                    wanted.push(key);
                }
            }
        }
    }
    wanted
}

/// The links from one Freight onwards, as [`join`] makes them for each
/// Freight it matches: its Warehouse, its Promotions, and each Stage that
/// runs it with that Stage's Application, workloads and pods. In trail
/// order; a Stage's links follow its Freight → Stage link.
pub(crate) fn freight_links(evidence: &Evidence, freight: &Freight) -> Vec<Link> {
    let mut links = vec![warehouse_link(evidence, freight)];
    links.extend(stage_links(evidence, freight));
    links
}

/// Whether builds can be found by this commit: the full SHA, short enough
/// for the label value (a SHA-256 commit is not), as `read_builds` requires.
fn is_label_sha(sha: &str) -> bool {
    crate::digest::is_full_sha(sha) && sha.len() <= 63
}

pub fn join(evidence: &Evidence) -> Trail {
    let mut links = Vec::new();
    let sha = &evidence.sha;

    // Commit -> PipelineRun, and the supply-chain facts of each build. A
    // build found only by an image digest starts at the image instead.
    let names = &evidence.commit_names;
    let by_commit = !sha.is_empty();
    let builds: &[Build] =
        match evidence.builds.read() {
            Some(builds) => {
                if by_commit && !builds.iter().any(|b| b.found_by == FoundBy::Commit) {
                    links.push(
                        Link::new(
                            Hop::Commit,
                            Hop::PipelineRun,
                            "-",
                            Key::Sha(sha.clone()),
                            Confidence::Unknown,
                            format!(
                                "CI/CD cluster read, no PipelineRun carries this sha label{}",
                                cap_note(evidence.builds.capped())
                            ),
                        )
                        .because(evidence.builds.capped().is_none().then(
                            || Cause::not_found("a PipelineRun carrying the commit's sha label"),
                        )),
                    );
                }
                // With a commit, the digest search only adds builds; it speaks
                // of itself when it is the only way a build could be found.
                if let Some(search) = evidence.digest_search.as_ref().filter(|_| !by_commit)
                    && !builds.iter().any(|b| b.found_by == FoundBy::Digest)
                {
                    links.push(
                        Link::new(
                            Hop::Image,
                            Hop::PipelineRun,
                            "-",
                            search
                                .digests
                                .first()
                                .cloned()
                                .map_or(Key::None, Key::Digest),
                            Confidence::Unknown,
                            search.why_none(),
                        )
                        .because(search.cause()),
                    );
                }
                for build in builds {
                    match build.found_by {
                        FoundBy::Commit => {
                            links.push(commit_link(sha, build, names, &evidence.proof()))
                        }
                        FoundBy::Digest => links.extend(digest_links(build, evidence)),
                    }
                    links.extend(supply_chain_links(
                        build,
                        evidence.evidence_result.as_ref(),
                        names,
                        &evidence.proof(),
                    ));
                }
                builds
            }
            None => {
                links.push(
                    Link::new(
                        if by_commit { Hop::Commit } else { Hop::Image },
                        Hop::PipelineRun,
                        "-",
                        if by_commit {
                            Key::Sha(sha.clone())
                        } else {
                            Key::None
                        },
                        Confidence::Unknown,
                        evidence.builds.why_not_read().unwrap_or_default(),
                    )
                    .because(if by_commit && !is_label_sha(sha) {
                        // The builds were never asked for: reading again
                        // can't help, since this commit can't be a label value.
                        Some(Cause::cant_read(
                            "a commit SHA of 40 or 64 digits that fits a label, to find its build",
                            None,
                            true,
                        ))
                    } else {
                        cause::unread(
                            &evidence.builds,
                            cause::PIPELINE_RUNS,
                            evidence.build_namespace.as_deref(),
                            "the CI/CD cluster's PipelineRuns, to find the build",
                        )
                    }),
                );
                &[]
            }
        };

    // PullRequest -> commit and builds.
    let freight_for_prs: &[Freight] = evidence.kargo.freight.read().map_or(&[], Vec::as_slice);
    if by_commit {
        links.extend(pull_request_links(evidence, builds, freight_for_prs));
    }

    // PipelineRun -> Freight.
    let freight_all: &[Freight] = match evidence.kargo.freight.read() {
        Some(freight) => freight,
        None => {
            links.push(
                Link::new(
                    Hop::PipelineRun,
                    Hop::Freight,
                    "-",
                    Key::None,
                    Confidence::Unknown,
                    evidence.kargo.freight.why_not_read().unwrap_or_default(),
                )
                .because(cause::unread(
                    &evidence.kargo.freight,
                    cause::FREIGHT,
                    evidence.kargo.namespace(),
                    "Kargo's Freight, to find the one built from this commit",
                )),
            );
            return Trail {
                sha: sha.clone(),
                observed_at: evidence.observed_at,
                links,
            };
        }
    };
    let matched = matching_freight(sha, builds, freight_all);
    if matched.is_empty() {
        links.push(
            Link::new(
                Hop::PipelineRun,
                Hop::Freight,
                "-",
                Key::None,
                Confidence::Unknown,
                format!(
                    "no Freight carries this sha or a built digest (read {} Freight); tags are not joined{}",
                    freight_all.len(),
                    cap_note(evidence.kargo.freight.capped())
                ),
            )
            .because(evidence.kargo.freight.capped().is_none().then(|| {
                Cause::not_found("a Freight that carries the commit or a digest the build made")
            })),
        );
    }
    let builds_capped = evidence.builds.capped().is_some();
    for (freight, key) in &matched {
        let (from, confidence, reason, seen, conflicts) =
            build_freight(builds, names, sha, freight, key, builds_capped);
        let mut link = Link::new(
            from,
            Hop::Freight,
            id(&freight.project, &freight.name),
            key.clone(),
            confidence,
            reason,
        )
        .observed(seen);
        link.conflicts = conflicts;
        links.push(link);
        links.extend(freight_links(evidence, freight));
    }
    Trail {
        sha: sha.clone(),
        observed_at: evidence.observed_at,
        links,
    }
}
