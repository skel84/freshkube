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
mod conflict;
mod deployment;
mod kargo;
mod observe;
mod render;
mod workload;

use argo::rollout_namespace;
use build::{commit_link, pull_request_links, supply_chain_links};
pub use conflict::{BuiltDigest, DigestConflict};
use conflict::{conflict_reason, digest_conflicts};
use kargo::{freight_summary, stage_links, warehouse_link};
pub use render::render;

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use super::argocd::{Application, DestinationMatch, ManagedObject, StageNaming};
use super::deployments::{Deployment, DeploymentSet};
use super::digest::Digest;
use super::github::PullRequest;
use super::kargo::{Freight, KargoRead};
use super::observation::{Fact, Observation};
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
        }
    }

    /// The link with what it was read from.
    fn observed(mut self, evidence: Vec<Observation>) -> Self {
        self.evidence = evidence;
        self
    }
}

/// Everything read for one change, each part with its own outcome.
pub struct Evidence {
    pub sha: String,
    /// When the caller read it all. The caller's clock, never read here.
    pub observed_at: DateTime<Utc>,
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
pub struct WantedRollout {
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
    let mut seen = observe::builds_side(builds, names, key);
    if matches!(key, Key::Digest(_)) {
        seen.extend(observe::freight_side(freight, key));
        return (
            Hop::PipelineRun,
            Confidence::Confirmed,
            summary,
            seen,
            Vec::new(),
        );
    }
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
    let conflicts = digest_conflicts(builds, names, sha, freight, builds_capped);
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

/// The Rollouts whose pods should be read: those an Application managed by a
/// Stage that reports one of `freight` as current points at, when the
/// Application deploys to the environment cluster. Used by the collector so
/// pods are read only for the change in question.
pub fn candidate_rollouts(evidence: &Evidence) -> Vec<WantedRollout> {
    candidate_workloads(evidence, ControllerKind::Rollout)
}

/// The Deployments whose pods should be read, as [`candidate_rollouts`] finds
/// Rollouts.
pub fn candidate_deployments(evidence: &Evidence) -> Vec<WantedRollout> {
    candidate_workloads(evidence, ControllerKind::Deployment)
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

/// Whether Argo CD lists a controller of either kind among the
/// Application's objects.
fn manages_controller(app: &Application) -> bool {
    app.managed
        .iter()
        .any(|m| ControllerKind::Rollout.is(m) || ControllerKind::Deployment.is(m))
}

fn candidate_workloads(evidence: &Evidence, controller: ControllerKind) -> Vec<WantedRollout> {
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
        for stage in stages
            .iter()
            .filter(|stage| stage.current_freight.contains(&item.name))
        {
            for app in apps.iter().filter(|app| {
                app.claims_stage(&stage.project, &stage.name, evidence.stage_naming.as_ref())
                    .is_some()
                    && evidence.deploys_to_environment(&id(&app.namespace, &app.name))
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
                    && !manages_controller(app)
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
                observed_at: evidence.observed_at,
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
        links.push(warehouse_link(evidence, freight));
        links.extend(stage_links(evidence, freight));
    }
    Trail {
        sha: sha.clone(),
        observed_at: evidence.observed_at,
        links,
    }
}
