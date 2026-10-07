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
mod kargo;
mod render;
mod workload;

use argo::rollout_namespace;
use build::{commit_link, pull_request_links, supply_chain_links};
use kargo::{freight_summary, stage_links};
pub use render::render;

use std::collections::BTreeMap;

use super::argocd::{Application, DestinationMatch, StageNaming};
use super::digest::Digest;
use super::github::PullRequest;
use super::kargo::{Freight, KargoRead};
use super::pods::RunningImage;
use super::rollouts::{AnalysisRun, ReplicaSet, Rollout};
use super::source::{Source, cap_note};
use super::tekton::{Build, CommitNames, EvidenceResult};

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
    PipelineRun,
    SupplyChain,
    Freight,
    Promotion,
    Stage,
    Application,
    Rollout,
    Pod,
}

impl Hop {
    pub fn word(self) -> &'static str {
        match self {
            Self::PullRequest => "pull request",
            Self::Commit => "commit",
            Self::PipelineRun => "PipelineRun",
            Self::SupplyChain => "supply chain",
            Self::Freight => "Freight",
            Self::Promotion => "Promotion",
            Self::Stage => "Stage",
            Self::Application => "Application",
            Self::Rollout => "Rollout",
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
        }
    }
}

/// Everything read for one change, each part with its own outcome.
pub struct Evidence {
    pub sha: String,
    /// Where the pipeline's own evidence record is; `None` when the caller
    /// configured none.
    pub evidence_result: Option<EvidenceResult>,
    /// Parameter and result names, besides upstream's, that name a build's
    /// commit.
    pub commit_names: CommitNames,
    /// How Applications are tied to stages where they carry no annotation;
    /// `None` when the caller configured none.
    pub stage_naming: Option<StageNaming>,
    pub builds: Source<Vec<Build>>,
    pub kargo: KargoRead,
    pub applications: Source<Vec<Application>>,
    /// Where each Application's destination points, by `namespace/name`.
    pub destinations: BTreeMap<String, DestinationMatch>,
    /// The context name of the environment cluster, the only cluster
    /// Rollouts and pods are read from.
    pub environment: String,
    pub rollouts: Source<Vec<Rollout>>,
    pub analysis_runs: Source<Vec<AnalysisRun>>,
    /// The ReplicaSets of the Rollouts' namespaces.
    pub replica_sets: Source<Vec<ReplicaSet>>,
    /// Pods read for each Rollout, by `namespace/name`.
    pub pods: BTreeMap<String, RolloutPods>,
    /// Pods read in an Application's destination namespace, by the
    /// Application's `namespace/name`, for workloads that aren't Rollouts.
    pub namespace_pods: BTreeMap<String, Source<Vec<RunningImage>>>,
    /// Pull requests GitHub associates with the commit; `None` when GitHub
    /// wasn't asked.
    pub pull_requests: Option<Source<Vec<PullRequest>>>,
    /// Builds on a pull request's head commit, by PR number, for the PRs whose
    /// head is not the commit itself (a squash or rebase merge).
    pub pr_builds: BTreeMap<u64, Source<Vec<Build>>>,
}

impl Evidence {
    /// Whether the Application's destination is the environment cluster.
    /// Its Rollouts and pods are read, and judged, only then: a workload of
    /// the same name in another cluster says nothing about this one.
    pub fn deploys_to_environment(&self, app_id: &str) -> bool {
        self.destinations
            .get(app_id)
            .and_then(DestinationMatch::context)
            .is_some_and(|context| context == self.environment)
    }
}

/// Which of a Rollout's pods show what it runs for the change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PodSet {
    /// The ReplicaSets, `(name, pod-template hash)`, whose pod template pins
    /// the Freight's digest. Another ReplicaSet of the Rollout, such as an
    /// older one that never became healthy, is not judged.
    Pinned(Vec<(String, String)>),
    /// The Rollout's current pod hash, when no ReplicaSet of it pins the
    /// digest or they weren't read.
    Current(String),
}

impl PodSet {
    pub fn hashes(&self) -> Vec<&str> {
        match self {
            Self::Pinned(sets) => sets.iter().map(|(_, hash)| hash.as_str()).collect(),
            Self::Current(hash) => vec![hash],
        }
    }
}

/// The pods read for one Rollout, and which.
#[derive(Clone, Debug)]
pub struct RolloutPods {
    pub set: PodSet,
    pub pods: Source<Vec<RunningImage>>,
}

/// The pods to read for a Rollout: those of its ReplicaSets that pin one of
/// `digests`, else its current pod hash. `None` when it reports none.
pub fn pod_set(
    rollout: &Rollout,
    replica_sets: Option<&[ReplicaSet]>,
    digests: &[Digest],
) -> Option<PodSet> {
    let pinned: Vec<(String, String)> = owned_replica_sets(rollout, replica_sets)
        .filter(|set| set.pins(digests))
        .filter_map(|set| Some((set.name.clone(), set.pod_hash.clone()?)))
        .collect();
    if pinned.is_empty() {
        rollout.current_pod_hash.clone().map(PodSet::Current)
    } else {
        Some(PodSet::Pinned(pinned))
    }
}

fn owned_replica_sets<'a>(
    rollout: &'a Rollout,
    replica_sets: Option<&'a [ReplicaSet]>,
) -> impl Iterator<Item = &'a ReplicaSet> {
    replica_sets.into_iter().flatten().filter(|set| {
        set.namespace == rollout.namespace && rollout.uid.is_some() && set.owner_uid == rollout.uid
    })
}

/// A Rollout the change reaches, with the digests of the Freight that lead
/// to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WantedRollout {
    pub namespace: String,
    pub name: String,
    pub digests: Vec<Digest>,
}

/// The trail of one change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trail {
    pub sha: String,
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
        let counts = format!(
            "{} confirmed, {} claimed, {} unknown",
            count(Confidence::Confirmed),
            count(Confidence::Claimed),
            count(Confidence::Unknown)
        );
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
                        && !matches!(link.to, Hop::Promotion | Hop::SupplyChain)
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
        } else if item
            .commits
            .iter()
            .any(|commit| commit.id.eq_ignore_ascii_case(sha))
        {
            found.push((item, Key::Sha(sha.to_owned())));
        }
    }
    found
}

/// The Rollouts whose pods should be read: those an Application managed by a
/// Stage that reports one of `freight` as current points at, when the
/// Application deploys to the environment cluster. Used by the collector so
/// pods are read only for the change in question.
pub fn candidate_rollouts(evidence: &Evidence) -> Vec<WantedRollout> {
    let (Some(builds), Some(freight), Some(stages), Some(apps)) = (
        evidence.builds.read(),
        evidence.kargo.freight.read(),
        evidence.kargo.stages.read(),
        evidence.applications.read(),
    ) else {
        return Vec::new();
    };
    let mut wanted: Vec<WantedRollout> = Vec::new();
    for (item, _) in matching_freight(&evidence.sha, builds, freight) {
        let digests = item.images.iter().filter_map(|image| image.digest.clone());
        let digests: Vec<Digest> = digests.collect();
        for stage in stages
            .iter()
            .filter(|stage| stage.current_freight.contains(&item.name))
        {
            for app in apps.iter().filter(|app| {
                app.claims_stage(&stage.project, &stage.name, evidence.stage_naming.as_ref())
                    .is_some()
                    && evidence.deploys_to_environment(&id(&app.namespace, &app.name))
            }) {
                for managed in app.managed.iter().filter(|m| m.kind == "Rollout") {
                    let Some(namespace) = rollout_namespace(app, managed) else {
                        continue;
                    };
                    let found = wanted
                        .iter()
                        .position(|w| w.namespace == namespace && w.name == managed.name);
                    let entry = match found {
                        Some(index) => &mut wanted[index],
                        None => {
                            wanted.push(WantedRollout {
                                namespace,
                                name: managed.name.clone(),
                                digests: Vec::new(),
                            });
                            wanted.last_mut().expect("just pushed")
                        }
                    };
                    for digest in &digests {
                        if !entry.digests.contains(digest) {
                            entry.digests.push(digest.clone());
                        }
                    }
                }
            }
        }
    }
    wanted
}

/// The Applications of a stage that holds the change's Freight and manage no
/// Rollout, with the namespace their workload runs in: the pods there are
/// read instead, when the destination is the environment cluster.
pub fn candidate_applications(evidence: &Evidence) -> Vec<(String, String)> {
    let (Some(builds), Some(freight), Some(stages), Some(apps)) = (
        evidence.builds.read(),
        evidence.kargo.freight.read(),
        evidence.kargo.stages.read(),
        evidence.applications.read(),
    ) else {
        return Vec::new();
    };
    let mut wanted = Vec::new();
    for (item, _) in matching_freight(&evidence.sha, builds, freight) {
        for stage in stages
            .iter()
            .filter(|stage| stage.current_freight.contains(&item.name))
        {
            for app in apps.iter().filter(|app| {
                app.claims_stage(&stage.project, &stage.name, evidence.stage_naming.as_ref())
                    .is_some()
                    && !app.managed.iter().any(|m| m.kind == "Rollout")
                    && evidence.deploys_to_environment(&id(&app.namespace, &app.name))
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

pub fn join(evidence: &Evidence) -> Trail {
    let mut links = Vec::new();
    let sha = &evidence.sha;

    // Commit -> PipelineRun, and the supply-chain facts of each build.
    let names = &evidence.commit_names;
    let builds: &[Build] = match evidence.builds.read() {
        Some(builds) => {
            if builds.is_empty() {
                links.push(Link::new(
                    Hop::Commit,
                    Hop::PipelineRun,
                    "-",
                    Key::Sha(sha.clone()),
                    Confidence::Unknown,
                    format!(
                        "CI/CD cluster read, no PipelineRun carries this sha label{}",
                        cap_note(evidence.builds.capped())
                    ),
                ));
            }
            for build in builds {
                links.push(commit_link(sha, build, names));
                links.extend(supply_chain_links(
                    build,
                    evidence.evidence_result.as_ref(),
                    names,
                ));
            }
            builds
        }
        None => {
            links.push(Link::new(
                Hop::Commit,
                Hop::PipelineRun,
                "-",
                Key::Sha(sha.clone()),
                Confidence::Unknown,
                evidence.builds.why_not_read().unwrap_or_default(),
            ));
            &[]
        }
    };

    // PullRequest -> commit and builds.
    let freight_for_prs: &[Freight] = evidence.kargo.freight.read().map_or(&[], Vec::as_slice);
    links.extend(pull_request_links(evidence, builds, freight_for_prs));

    // PipelineRun -> Freight.
    let freight_all: &[Freight] = match evidence.kargo.freight.read() {
        Some(freight) => freight,
        None => {
            links.push(Link::new(
                Hop::PipelineRun,
                Hop::Freight,
                "-",
                Key::None,
                Confidence::Unknown,
                evidence.kargo.freight.why_not_read().unwrap_or_default(),
            ));
            return Trail {
                sha: sha.clone(),
                links,
            };
        }
    };
    let matched = matching_freight(sha, builds, freight_all);
    if matched.is_empty() {
        links.push(Link::new(
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
        ));
    }
    for (freight, key) in &matched {
        links.push(Link::new(
            Hop::PipelineRun,
            Hop::Freight,
            id(&freight.project, &freight.name),
            key.clone(),
            Confidence::Confirmed,
            freight_summary(freight, sha),
        ));
        links.extend(stage_links(evidence, freight));
    }
    Trail {
        sha: sha.clone(),
        links,
    }
}
