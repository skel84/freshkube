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

use std::collections::BTreeMap;

use super::argocd::{
    Application, DestinationMatch, IN_CLUSTER_NAME, ManagedObject, StageClaim, StageNaming,
};
use super::digest::{Digest, repository, tag};
use super::github::PullRequest;
use super::kargo::{Freight, KargoRead, Promotion, Stage};
use super::pods::RunningImage;
use super::rollouts::{AnalysisRun, ReplicaSet, Rollout};
use super::source::{Source, Truncation, cap_note, redact_message};
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

fn pull_request_links(evidence: &Evidence, builds: &[Build], freight: &[Freight]) -> Vec<Link> {
    let sha = &evidence.sha;
    let prs = match &evidence.pull_requests {
        None => return Vec::new(),
        Some(read) if read.read().is_some() => read.read().unwrap(),
        Some(other) => {
            return vec![Link::new(
                Hop::PullRequest,
                Hop::Commit,
                "-",
                Key::Sha(sha.clone()),
                Confidence::Unknown,
                other.why_not_read().unwrap_or_default(),
            )];
        }
    };
    if prs.is_empty() {
        return vec![Link::new(
            Hop::PullRequest,
            Hop::Commit,
            "-",
            Key::Sha(sha.clone()),
            Confidence::Unknown,
            "GitHub read, no pull request is associated with this commit",
        )];
    }
    let deployed: Vec<Digest> = builds
        .iter()
        .flat_map(|build| build.images())
        .map(|image| image.digest)
        .collect();
    let mut links = Vec::new();
    for pr in prs {
        let subject = format!("{}#{}", pr.repo, pr.number);
        let merge_is = pr
            .merge_sha
            .as_deref()
            .is_some_and(|merge| merge.eq_ignore_ascii_case(sha));
        let head_is = pr.head_sha.eq_ignore_ascii_case(sha);
        let (confidence, how) = if merge_is {
            (Confidence::Confirmed, "its merge commit")
        } else if head_is {
            (Confidence::Confirmed, "its head commit")
        } else {
            (
                Confidence::Claimed,
                "only by ancestry, as GitHub lists it; neither its head nor its merge commit",
            )
        };
        links.push(Link::new(
            Hop::PullRequest,
            Hop::Commit,
            subject.clone(),
            Key::Sha(sha.clone()),
            confidence,
            format!(
                "the commit is {how}; {}{}",
                pr.state.as_deref().unwrap_or("state unknown"),
                if pr.merged { ", merged" } else { "" }
            ),
        ));
        if merge_is || head_is {
            for build in builds {
                let run = commit_link(sha, build, &evidence.commit_names);
                let numbered = match build.run.pull_request {
                    Some(number) if number != pr.number => {
                        format!("; the run says pull request #{number}, not #{}", pr.number)
                    }
                    _ => String::new(),
                };
                links.push(Link::new(
                    Hop::PullRequest,
                    Hop::PipelineRun,
                    run.subject,
                    Key::Sha(sha.clone()),
                    run.confidence,
                    format!("built from {how}; {}{numbered}", run.reason),
                ));
            }
        }
        if head_is {
            continue;
        }
        links.extend(head_build_links(evidence, pr, &subject, &deployed, freight));
    }
    links
}

/// What the pull request's own head commit built, and whether it is what
/// shipped: with a squash merge it usually is not.
fn head_build_links(
    evidence: &Evidence,
    pr: &PullRequest,
    subject: &str,
    deployed: &[Digest],
    freight: &[Freight],
) -> Vec<Link> {
    let head = &pr.head_sha;
    let read = evidence.pr_builds.get(&pr.number);
    let builds = match read {
        Some(source) if source.read().is_some() => source.read().unwrap(),
        Some(other) => {
            return vec![Link::new(
                Hop::PullRequest,
                Hop::PipelineRun,
                subject,
                Key::Sha(head.clone()),
                Confidence::Unknown,
                other.why_not_read().unwrap_or_default(),
            )];
        }
        None => return Vec::new(),
    };
    if builds.is_empty() {
        return vec![Link::new(
            Hop::PullRequest,
            Hop::PipelineRun,
            subject,
            Key::Sha(head.clone()),
            Confidence::Unknown,
            format!(
                "CI/CD cluster read, no PipelineRun for the pull request's head commit{}",
                cap_note(read.and_then(Source::capped))
            ),
        )];
    }
    let mut links = Vec::new();
    for build in builds {
        let run = commit_link(head, build, &evidence.commit_names);
        let digests: Vec<Digest> = build.images().into_iter().map(|i| i.digest).collect();
        let shipped = digests.iter().any(|digest| deployed.contains(digest));
        let note = if digests.is_empty() {
            "it reported no image digest".to_owned()
        } else if shipped {
            "its image digest is the one that shipped".to_owned()
        } else {
            format!(
                "its image ({}) is not the digest that shipped; the merge was rebuilt",
                digests
                    .iter()
                    .map(Digest::short)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        links.push(Link::new(
            Hop::PullRequest,
            Hop::PipelineRun,
            run.subject,
            Key::Sha(head.clone()),
            run.confidence,
            format!("built from the head commit; {note}; {}", run.reason),
        ));
        // A head build whose digest Kargo holds joins the PR to that Freight
        // on the digest, whatever the merge commit was.
        for item in freight {
            if let Some(digest) = item
                .images
                .iter()
                .filter_map(|image| image.digest.as_ref())
                .find(|digest| digests.contains(digest))
            {
                links.push(Link::new(
                    Hop::PullRequest,
                    Hop::Freight,
                    format!("{}/{}", item.project, item.name),
                    Key::Digest(digest.clone()),
                    Confidence::Confirmed,
                    "Kargo holds the image the pull request's head commit built",
                ));
            }
        }
    }
    links
}

fn commit_link(sha: &str, build: &Build, names: &CommitNames) -> Link {
    let subject = id(&build.run.namespace, &build.run.name);
    let outcome = match (&build.run.succeeded, &build.run.reason) {
        (Some(status), Some(reason)) => format!("Succeeded={status} ({reason})"),
        _ => "no Succeeded condition".to_owned(),
    };
    let outcome = match &build.tasks_unread {
        Some(why) => format!("{outcome}; its TaskRuns were not read ({why})"),
        None => outcome,
    };
    match build.witnessed_commit(names) {
        Some(witness) if witness.eq_ignore_ascii_case(sha) => Link::new(
            Hop::Commit,
            Hop::PipelineRun,
            subject,
            Key::Sha(sha.to_owned()),
            Confidence::Confirmed,
            format!("PaC label and the run's own revision agree; {outcome}"),
        ),
        Some(_) => Link::new(
            Hop::Commit,
            Hop::PipelineRun,
            subject,
            Key::Sha(sha.to_owned()),
            Confidence::Claimed,
            format!("the run's own revision differs from its PaC label; {outcome}"),
        ),
        None => Link::new(
            Hop::Commit,
            Hop::PipelineRun,
            subject,
            Key::Sha(sha.to_owned()),
            Confidence::Claimed,
            format!("only the PaC label says so; {outcome}"),
        ),
    }
}

fn supply_chain_links(
    build: &Build,
    evidence_result: Option<&EvidenceResult>,
    names: &CommitNames,
) -> Vec<Link> {
    let verdict = match build.conforma() {
        Some(conforma) => format!(
            "Conforma {} ({} failures, {} warnings)",
            conforma.outcome, conforma.failures, conforma.warnings
        ),
        None => "no Conforma result".to_owned(),
    };
    let verdict = match &build.tasks_unread {
        Some(why) => {
            format!("its TaskRuns were not read ({why}), so their results are unknown; {verdict}")
        }
        None => verdict,
    };
    let images = build.images();
    let subject = id(&build.run.namespace, &build.run.name);
    let verdict = match evidence_result.map(|config| build.evidence_record(config)) {
        None => format!(
            "{verdict}; no evidence result configured{}",
            run_results(build)
        ),
        Some(None) => format!("{verdict}; the configured evidence result is not on the build"),
        Some(Some(record)) => {
            let same_commit = record.commit.as_deref().is_some_and(|sha| {
                build
                    .witnessed_commit(names)
                    .or_else(|| build.run.sha.clone())
                    .is_some_and(|seen| seen.eq_ignore_ascii_case(sha))
            });
            let same_digest = record
                .image_digest
                .as_ref()
                .is_some_and(|digest| images.iter().any(|image| &image.digest == digest));
            format!(
                "{verdict}; the pipeline's evidence record {} the build's commit and digest",
                if same_commit && same_digest {
                    "agrees with"
                } else {
                    "disagrees with"
                }
            )
        }
    };
    if images.is_empty() {
        return vec![Link::new(
            Hop::PipelineRun,
            Hop::SupplyChain,
            subject,
            Key::None,
            Confidence::Unknown,
            format!("the build reported no image digest; {verdict}"),
        )];
    }
    images
        .into_iter()
        .map(|image| {
            let (confidence, chains) = match build.chains_state().as_deref() {
                Some("true") => (Confidence::Confirmed, "signed by Chains".to_owned()),
                Some(state) => (
                    Confidence::Unknown,
                    format!("Chains reports signed={state}"),
                ),
                None => (Confidence::Unknown, "no Chains annotation".to_owned()),
            };
            Link::new(
                Hop::PipelineRun,
                Hop::SupplyChain,
                subject.clone(),
                Key::Digest(image.digest.clone()),
                confidence,
                format!(
                    "{} {}; {chains}; {verdict}",
                    repository(&image.url),
                    tag(&image.url)
                        .map(|t| format!("(tag {t})"))
                        .unwrap_or_default()
                ),
            )
        })
        .collect()
}

/// The names of the run's own results, which a configured evidence result
/// could be one of.
fn run_results(build: &Build) -> String {
    const SHOWN: usize = 8;
    let names: Vec<&str> = build.run.results.keys().map(String::as_str).collect();
    match names.len() {
        0 => " (the run has no results)".to_owned(),
        n if n > SHOWN => format!(
            " (the run's results: {}, and {} more)",
            names[..SHOWN].join(", "),
            n - SHOWN
        ),
        _ => format!(" (the run's results: {})", names.join(", ")),
    }
}

fn freight_summary(freight: &Freight, sha: &str) -> String {
    let images: Vec<String> = freight
        .images
        .iter()
        .map(|image| {
            let tag = image
                .tag
                .as_deref()
                .map(|tag| format!(" (tag {tag})"))
                .unwrap_or_default();
            let revision = image
                .revision
                .as_deref()
                .map(|revision| {
                    let whose = match same_commit(revision, sha) {
                        Some(true) => "this change's commit",
                        Some(false) => "not this change's commit",
                        None => "which can't be compared with this change's commit",
                    };
                    format!(
                        "; its OCI revision annotation names {}, {whose}",
                        short_commit(revision)
                    )
                })
                .unwrap_or_default();
            format!("{}{tag}{revision}", repository(&image.repo_url))
        })
        .collect();
    let alias = freight
        .alias
        .as_deref()
        .map(|alias| format!("alias {alias}; "))
        .unwrap_or_default();
    format!("{alias}{}", images.join(", "))
}

fn stage_links(evidence: &Evidence, freight: &Freight) -> Vec<Link> {
    let subject = id(&freight.project, &freight.name);
    let mut links = Vec::new();
    match evidence.kargo.promotions.read() {
        Some(promotions) => {
            for promotion in promotions
                .iter()
                .filter(|p| p.freight.as_deref() == Some(freight.name.as_str()))
            {
                links.push(promotion_link(promotion, freight));
            }
        }
        None => links.push(Link::new(
            Hop::Freight,
            Hop::Promotion,
            subject.clone(),
            Key::None,
            Confidence::Unknown,
            evidence.kargo.promotions.why_not_read().unwrap_or_default(),
        )),
    }
    let Some(stages) = evidence.kargo.stages.read() else {
        links.push(Link::new(
            Hop::Freight,
            Hop::Stage,
            subject,
            Key::None,
            Confidence::Unknown,
            evidence.kargo.stages.why_not_read().unwrap_or_default(),
        ));
        return links;
    };
    let current: Vec<&Stage> = stages
        .iter()
        .filter(|stage| stage.current_freight.contains(&freight.name))
        .collect();
    if current.is_empty() {
        links.push(Link::new(
            Hop::Freight,
            Hop::Stage,
            subject,
            Key::None,
            Confidence::Unknown,
            format!(
                "no Stage reports it as current (read {} Stages){}",
                stages.len(),
                cap_note(evidence.kargo.stages.capped())
            ),
        ));
    }
    for stage in current {
        let health = stage_health(stage);
        links.push(match shared_digest(freight, &stage.current_digests) {
            Some(digest) => Link::new(
                Hop::Freight,
                Hop::Stage,
                id(&stage.project, &stage.name),
                Key::Digest(digest.clone()),
                Confidence::Confirmed,
                format!(
                    "the Stage's record of its current Freight carries the digest; health {health}"
                ),
            ),
            None => Link::new(
                Hop::Freight,
                Hop::Stage,
                id(&stage.project, &stage.name),
                Key::Name(freight.name.clone()),
                Confidence::Claimed,
                format!("the Stage names this Freight only; health {health}"),
            ),
        });
        links.extend(application_links(evidence, freight, stage));
    }
    links
}

/// The Stage's health, with Kargo's reasons when it gives any.
fn stage_health(stage: &Stage) -> String {
    let status = stage.health.as_deref().unwrap_or("unreported");
    if stage.health_issues.is_empty() {
        return status.to_owned();
    }
    let issues: Vec<String> = stage
        .health_issues
        .iter()
        .map(|issue| redact_message(issue))
        .collect();
    format!("{status} ({})", issues.join("; "))
}

/// The first digest of `freight` that `held` also holds.
fn shared_digest<'a>(freight: &'a Freight, held: &[Digest]) -> Option<&'a Digest> {
    freight
        .images
        .iter()
        .filter_map(|image| image.digest.as_ref())
        .find(|digest| held.contains(digest))
}

fn promotion_link(promotion: &Promotion, freight: &Freight) -> Link {
    let pushed = match promotion.pushed_commits.as_slice() {
        [] => "it recorded no pushed commit".to_owned(),
        commits => format!(
            "it pushed {}",
            commits
                .iter()
                .map(|commit| short_commit(commit))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let note = format!(
        "to Stage {}: {}{}; {}; {pushed}",
        promotion.stage.as_deref().unwrap_or("?"),
        promotion.phase.as_deref().unwrap_or("no phase"),
        promotion
            .message
            .as_deref()
            .map(|message| format!(" ({})", redact_message(message)))
            .unwrap_or_default(),
        promotion.creator.describe()
    );
    let subject = id(&promotion.project, &promotion.name);
    match shared_digest(freight, &promotion.freight_digests) {
        Some(digest) => Link::new(
            Hop::Freight,
            Hop::Promotion,
            subject,
            Key::Digest(digest.clone()),
            Confidence::Confirmed,
            format!("the Promotion's status carries the Freight's digest; {note}"),
        ),
        None => Link::new(
            Hop::Freight,
            Hop::Promotion,
            subject,
            Key::Name(promotion.freight.clone().unwrap_or_default()),
            Confidence::Claimed,
            format!("named by the Promotion's spec only; {note}"),
        ),
    }
}

fn application_links(evidence: &Evidence, freight: &Freight, stage: &Stage) -> Vec<Link> {
    let stage_id = id(&stage.project, &stage.name);
    let Some(apps) = evidence.applications.read() else {
        return vec![Link::new(
            Hop::Stage,
            Hop::Application,
            stage_id,
            Key::None,
            Confidence::Unknown,
            evidence.applications.why_not_read().unwrap_or_default(),
        )];
    };
    let owned: Vec<&Application> = apps
        .iter()
        .filter(|app| {
            app.claims_stage(&stage.project, &stage.name, evidence.stage_naming.as_ref())
                .is_some()
        })
        .collect();
    if owned.is_empty() {
        return vec![Link::new(
            Hop::Stage,
            Hop::Application,
            stage_id,
            Key::None,
            Confidence::Unknown,
            format!(
                "no Application names this Stage (read {}){}",
                apps.len(),
                cap_note(evidence.applications.capped())
            ),
        )];
    }
    let mut links = Vec::new();
    for app in owned {
        let app_id = id(&app.namespace, &app.name);
        let destination = evidence
            .destinations
            .get(&app_id)
            .map(describe_destination)
            .unwrap_or_else(|| "destination not matched".to_owned());
        let (confidence, key, proof) = stage_application_proof(evidence, freight, stage, app, apps);
        links.push(Link::new(
            Hop::Stage,
            Hop::Application,
            app_id.clone(),
            key.unwrap_or_else(|| Key::Name(format!("{}:{}", stage.project, stage.name))),
            confidence,
            format!(
                "{}; {proof}; sync {}, health {}; {destination}",
                app.claims_stage(&stage.project, &stage.name, evidence.stage_naming.as_ref())
                    .map_or("claimed", StageClaim::describe),
                app.sync.as_deref().unwrap_or("?"),
                app.health.as_deref().unwrap_or("?")
            ),
        ));
        links.extend(rollout_links(evidence, freight, app));
    }
    links
}

/// Whether a succeeded Promotion of this Freight to this Stage pushed the very
/// commit the Application synced. That is a SHA join, and the only thing that
/// confirms the Application receives this Stage's changes. It stays a claim
/// when the revision moved on, when no Promotion recorded a commit, or when an
/// Application of another stage in the project is at the same revision (they
/// may share a branch, so the commit alone would not tell them apart).
fn stage_application_proof(
    evidence: &Evidence,
    freight: &Freight,
    stage: &Stage,
    app: &Application,
    apps: &[Application],
) -> (Confidence, Option<Key>, String) {
    let promotions: Vec<&Promotion> = evidence
        .kargo
        .promotions
        .read()
        .map(|all| {
            all.iter()
                .filter(|p| {
                    p.project == stage.project
                        && p.stage.as_deref() == Some(stage.name.as_str())
                        && p.freight.as_deref() == Some(freight.name.as_str())
                        && p.phase.as_deref() == Some("Succeeded")
                })
                .collect()
        })
        .unwrap_or_default();
    let pushed: Vec<&String> = promotions
        .iter()
        .flat_map(|p| p.pushed_commits.iter())
        .collect();
    if pushed.is_empty() {
        return (
            Confidence::Claimed,
            None,
            format!(
                "no succeeded Promotion recorded a pushed commit{}",
                cap_note(evidence.kargo.promotions.capped())
            ),
        );
    }
    let at = |application: &Application, commit: &str| {
        application
            .sync_revisions
            .iter()
            .any(|revision| revision.eq_ignore_ascii_case(commit))
    };
    let Some(commit) = pushed.iter().find(|commit| at(app, commit)) else {
        return (
            Confidence::Claimed,
            None,
            format!(
                "the Promotion pushed {} but the Application's synced revision is {}",
                short_commit(pushed[0]),
                app.sync_revisions
                    .first()
                    .map_or("unknown".to_owned(), |r| short_commit(r))
            ),
        );
    };
    let shared = apps.iter().any(|other| {
        (other.namespace != app.namespace || other.name != app.name)
            && at(other, commit)
            && other
                .claims_stage(&stage.project, &stage.name, evidence.stage_naming.as_ref())
                .is_none()
            && other.is_of_project(&stage.project, evidence.stage_naming.as_ref())
    });
    if let Some(truncation) = evidence.applications.capped() {
        // An Application of another stage at the same commit may be among
        // those not read, so the commit can't single this one out.
        return (
            Confidence::Claimed,
            Some(Key::Sha((*commit).clone())),
            format!(
                "the Application synced the commit the Promotion pushed ({}), but the Application listing stopped at the page cap after {} items, so another stage's Application may be at it too",
                short_commit(commit),
                truncation.read
            ),
        );
    }
    if shared {
        return (
            Confidence::Claimed,
            Some(Key::Sha((*commit).clone())),
            format!(
                "the Application synced the commit the Promotion pushed ({}), but another stage's Application is at it too",
                short_commit(commit)
            ),
        );
    }
    (
        Confidence::Confirmed,
        Some(Key::Sha((*commit).clone())),
        format!(
            "the Application synced the commit the Promotion pushed ({})",
            short_commit(commit)
        ),
    )
}

/// Whether `revision`, a full or abbreviated commit id, names the commit
/// `sha`. An abbreviation of seven or more characters is the start of the
/// full id, case aside. `None` when the two can't be compared: a shorter
/// abbreviation, an id longer than the commit's (another hash algorithm),
/// or one that differs from a SHA-256 commit, which may also be known by a
/// SHA-1 id.
fn same_commit(revision: &str, sha: &str) -> Option<bool> {
    const SHORTEST: usize = 7;
    let (revision, sha) = (revision.as_bytes(), sha.as_bytes());
    if revision.len() < SHORTEST || revision.len() > sha.len() {
        None
    } else if revision.eq_ignore_ascii_case(&sha[..revision.len()]) {
        Some(true)
    } else if revision.len() == sha.len() || sha.len() == 40 {
        Some(false)
    } else {
        None
    }
}

fn short_commit(commit: &str) -> String {
    commit.chars().take(12).collect()
}

fn describe_destination(destination: &DestinationMatch) -> String {
    match destination {
        DestinationMatch::One(context) => format!("destination is context {context}"),
        DestinationMatch::Ambiguous(contexts) => format!(
            "destination matches several contexts ({}); the user must choose",
            contexts.join(", ")
        ),
        DestinationMatch::ArgoCd(context) => {
            format!("destination is Argo CD's own cluster ({IN_CLUSTER_NAME}), context {context}")
        }
        DestinationMatch::Named { context, .. } => {
            format!("destination by a cluster name, mapped to context {context}")
        }
        DestinationMatch::None => "destination matches no known context".to_owned(),
        DestinationMatch::ByName(_) => {
            "destination by a cluster name no context is mapped to".to_owned()
        }
        DestinationMatch::Unspecified => "no destination".to_owned(),
    }
}

/// The namespace a managed Rollout runs in: its own, else the
/// Application's destination namespace. `None` when neither names one, and
/// then it is not read.
fn rollout_namespace(app: &Application, managed: &ManagedObject) -> Option<String> {
    managed
        .namespace
        .clone()
        .or_else(|| app.destination_namespace.clone())
        .filter(|namespace| !namespace.is_empty())
}

/// Why nothing was read in the environment cluster for an Application, and
/// how the user would say otherwise when its destination is that cluster.
fn not_the_environment(evidence: &Evidence, app_id: &str) -> String {
    let env = &evidence.environment;
    let why = match evidence.destinations.get(app_id) {
        Some(DestinationMatch::ByName(_)) => format!(
            "its cluster name is mapped to no context; if it is the environment cluster, map the name the Application gives with --known-as-name {env}=<cluster name>"
        ),
        Some(DestinationMatch::None) => format!(
            "its server matches no context; if it is the environment cluster's, add it with --known-as {env}=<server>"
        ),
        Some(DestinationMatch::Ambiguous(contexts)) => format!(
            "it matches several contexts ({}); the user must choose",
            contexts.join(", ")
        ),
        Some(DestinationMatch::Unspecified) => "the Application names no destination".to_owned(),
        Some(matched) => format!(
            "it is context {}, and the environment is {env}",
            matched.context().unwrap_or("?")
        ),
        None => "its destination was not matched".to_owned(),
    };
    format!(
        "the destination is not known to be the environment cluster, so nothing was read there for it: {why}"
    )
}

fn rollout_links(evidence: &Evidence, freight: &Freight, app: &Application) -> Vec<Link> {
    let app_id = id(&app.namespace, &app.name);
    let managed: Vec<_> = app.managed.iter().filter(|m| m.kind == "Rollout").collect();
    if managed.is_empty() {
        return vec![workload_pod_link(evidence, freight, app, &app_id)];
    }
    if !evidence.deploys_to_environment(&app_id) {
        let why = not_the_environment(evidence, &app_id);
        return vec![Link::new(
            Hop::Application,
            Hop::Rollout,
            app_id,
            Key::None,
            Confidence::Unknown,
            why,
        )];
    }
    let Some(rollouts) = evidence.rollouts.read() else {
        return vec![Link::new(
            Hop::Application,
            Hop::Rollout,
            app_id,
            Key::None,
            Confidence::Unknown,
            evidence.rollouts.why_not_read().unwrap_or_default(),
        )];
    };
    let mut links = Vec::new();
    for object in managed {
        let Some(namespace) = rollout_namespace(app, object) else {
            links.push(Link::new(
                Hop::Application,
                Hop::Rollout,
                format!("{app_id}: {}", object.name),
                Key::None,
                Confidence::Unknown,
                "neither the Rollout nor the Application's destination names a namespace, so it was not read",
            ));
            continue;
        };
        let rollout_id = id(&namespace, &object.name);
        let Some(rollout) = rollouts
            .iter()
            .find(|r| r.namespace == namespace && r.name == object.name)
        else {
            links.push(Link::new(
                Hop::Application,
                Hop::Rollout,
                rollout_id,
                Key::None,
                Confidence::Unknown,
                format!(
                    "the Application lists it but it was not found in the environment cluster{}",
                    cap_note(evidence.rollouts.capped())
                ),
            ));
            continue;
        };
        let pinned = rollout
            .images
            .iter()
            .filter_map(|image| Digest::from_reference(image))
            .find(|digest| {
                freight
                    .images
                    .iter()
                    .any(|image| image.digest.as_ref() == Some(digest))
            });
        let state = rollout_state(rollout, evidence.analysis_runs.read());
        links.push(match pinned {
            Some(digest) => Link::new(
                Hop::Application,
                Hop::Rollout,
                rollout_id.clone(),
                Key::Digest(digest),
                Confidence::Confirmed,
                format!("the Rollout's spec pins the Freight's digest; {state}"),
            ),
            None => Link::new(
                Hop::Application,
                Hop::Rollout,
                rollout_id.clone(),
                Key::Name(rollout.name.clone()),
                Confidence::Claimed,
                format!("managed by the Application; its spec does not pin the digest; {state}"),
            ),
        });
        links.push(pod_link(evidence, freight, rollout, &rollout_id));
    }
    links
}

fn rollout_state(rollout: &Rollout, analysis: Option<&Vec<AnalysisRun>>) -> String {
    let mut parts = vec![format!(
        "phase {}",
        rollout.phase.as_deref().unwrap_or("unreported")
    )];
    if rollout.aborted {
        parts.push("aborted".into());
    }
    if rollout.paused {
        parts.push("paused".into());
    }
    if let Some(runs) = analysis {
        let own: Vec<String> = runs
            .iter()
            .filter(|run| run.rollout.as_deref() == Some(rollout.name.as_str()))
            .map(|run| format!("{}={}", run.name, run.phase.as_deref().unwrap_or("?")))
            .collect();
        if !own.is_empty() {
            parts.push(format!("analysis {}", own.join(", ")));
        }
    }
    parts.join(", ")
}

fn pod_link(evidence: &Evidence, freight: &Freight, rollout: &Rollout, rollout_id: &str) -> Link {
    let subject = rollout_id.to_owned();
    let read = evidence.pods.get(rollout_id);
    let (pods, which) = match read {
        Some(read) if read.pods.read().is_some() => (
            read.pods.read().unwrap(),
            which_pods(evidence, rollout, &read.set),
        ),
        Some(other) => {
            return Link::new(
                Hop::Rollout,
                Hop::Pod,
                subject,
                Key::None,
                Confidence::Unknown,
                other.pods.why_not_read().unwrap_or_default(),
            );
        }
        None => {
            return Link::new(
                Hop::Rollout,
                Hop::Pod,
                subject,
                Key::None,
                Confidence::Unknown,
                if rollout.current_pod_hash.is_none() {
                    "the Rollout reports no current pod hash"
                } else {
                    "pods were not read"
                },
            );
        }
    };
    let mut link = judge_pods(
        Hop::Rollout,
        subject,
        pods,
        read.and_then(|read| read.pods.capped()),
        freight,
    );
    link.reason = format!("{}; {which}", link.reason);
    link
}

/// Which pods were judged, and which of the Rollout's other ReplicaSets with
/// pods were not.
fn which_pods(evidence: &Evidence, rollout: &Rollout, set: &PodSet) -> String {
    let read = evidence.replica_sets.read().map(Vec::as_slice);
    let which = match set {
        PodSet::Pinned(sets) => format!(
            "the pods of ReplicaSet {}, whose template pins the Freight's digest{}",
            sets.iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            cap_note(evidence.replica_sets.capped())
        ),
        PodSet::Current(hash) => {
            let why = match evidence.replica_sets.why_not_read() {
                Some(why) => format!("its ReplicaSets were not read ({why})"),
                None => format!(
                    "no ReplicaSet of it pins the Freight's digest{}",
                    cap_note(evidence.replica_sets.capped())
                ),
            };
            format!("the pods of the Rollout's current pod hash {hash}; {why}")
        }
    };
    let judged = set.hashes();
    let unlike = match set {
        PodSet::Pinned(_) => ", whose template does not pin it",
        PodSet::Current(_) => "",
    };
    let others: Vec<&str> = owned_replica_sets(rollout, read)
        .filter(|other| {
            other.replicas > 0
                && other
                    .pod_hash
                    .as_deref()
                    .is_none_or(|hash| !judged.contains(&hash))
        })
        .map(|other| other.name.as_str())
        .collect();
    if others.is_empty() {
        which
    } else {
        format!(
            "{which}; not judged: the pods of ReplicaSet {}{unlike}",
            others.join(", ")
        )
    }
}

/// The Application's pods when it manages no Rollout: whatever runs in its
/// destination namespace, judged by digest like a Rollout's pods. Argo CD's
/// own image summary is reported but is not what joins.
fn workload_pod_link(
    evidence: &Evidence,
    freight: &Freight,
    app: &Application,
    app_id: &str,
) -> Link {
    let unknown = |reason: String| {
        Link::new(
            Hop::Application,
            Hop::Pod,
            app_id.to_owned(),
            Key::None,
            Confidence::Unknown,
            reason,
        )
    };
    if !evidence.deploys_to_environment(app_id) {
        return unknown(not_the_environment(evidence, app_id));
    }
    match evidence.namespace_pods.get(app_id) {
        Some(source) if source.read().is_some() => {
            let pods = source.read().unwrap();
            let summary = if app
                .digests()
                .iter()
                .any(|d| freight.images.iter().any(|i| i.digest.as_ref() == Some(d)))
            {
                "Argo CD's image summary lists the Freight's digest"
            } else {
                "Argo CD's image summary does not list the Freight's digest"
            };
            let mut link = judge_pods(
                Hop::Application,
                app_id.to_owned(),
                pods,
                source.capped(),
                freight,
            );
            link.reason = format!("{summary}; {}", link.reason);
            link
        }
        Some(other) => unknown(other.why_not_read().unwrap_or_default()),
        None => unknown(
            "the pods of the destination namespace were not read (the destination has no namespace)"
                .to_owned(),
        ),
    }
}

fn judge_pods(
    from: Hop,
    subject: String,
    pods: &[RunningImage],
    capped: Option<Truncation>,
    freight: &Freight,
) -> Link {
    let wanted: Vec<&Digest> = freight
        .images
        .iter()
        .filter_map(|image| image.digest.as_ref())
        .collect();
    let repos: Vec<String> = freight
        .images
        .iter()
        .map(|image| repository(&image.repo_url))
        .collect();
    let mut matching = Vec::new();
    let mut other: Vec<(String, Option<Digest>)> = Vec::new();
    for pod in pods {
        match &pod.digest {
            Some(digest) if wanted.contains(&digest) => matching.push((pod, digest)),
            digest => {
                let same_repo = pod
                    .image
                    .as_deref()
                    .is_some_and(|image| repos.contains(&repository(image)));
                if same_repo {
                    other.push((pod.pod.clone(), digest.clone()));
                }
            }
        }
    }
    let ready = matching.iter().filter(|(pod, _)| pod.ready).count();
    if let Some((_, digest)) = matching.first() {
        let extra = if other.is_empty() {
            String::new()
        } else {
            format!(
                "; {} other container(s) of the same image run a different digest",
                other.len()
            )
        };
        Link::new(
            from,
            Hop::Pod,
            subject,
            Key::Digest((*digest).clone()),
            Confidence::Confirmed,
            format!(
                "{} container(s) run the Freight's digest, {ready} ready of {} pod container(s) read{extra}",
                matching.len(),
                pods.len()
            ),
        )
    } else if !other.is_empty() {
        let digests: Vec<String> = other
            .iter()
            .map(|(_, digest)| {
                digest
                    .as_ref()
                    .map_or("no digest".to_owned(), Digest::short)
            })
            .collect();
        Link::new(
            from,
            Hop::Pod,
            subject,
            Key::None,
            Confidence::Claimed,
            format!(
                "the Stage claims this Freight but the pods run another digest ({}); a moved tag is not a match{}",
                digests.join(", "),
                cap_note(capped)
            ),
        )
    } else {
        Link::new(
            from,
            Hop::Pod,
            subject,
            Key::None,
            Confidence::Unknown,
            format!(
                "read {} pod container(s); none runs the Freight's image{}",
                pods.len(),
                cap_note(capped)
            ),
        )
    }
}

/// A trail as text, one link per line, then its summary.
pub fn render(trail: &Trail) -> String {
    let mut out = format!("change {}\n", trail.sha);
    for link in &trail.links {
        out.push_str(&format!(
            "  {:<9} {} -> {}  [{}]  {}  ({})\n",
            link.confidence.word(),
            link.from.word(),
            link.to.word(),
            link.key,
            link.subject,
            link.reason
        ));
    }
    out.push_str(&format!("summary: {}\n", trail.summary()));
    out
}
