//! Reads what one change needs, in the order the chain leads, and hands it to
//! [`join`](super::join::join). Each cluster has its own [`Reader`], so the
//! Kargo, Argo CD, CI/CD and environment clusters may be four contexts or
//! fewer; the caller says which is which.

use std::collections::BTreeMap;

use super::argocd::{DestinationMatch, StageNaming, match_destination, read_applications};
use super::github::GitHub;
use super::join::{Evidence, candidate_applications, candidate_rollouts};
use super::kargo::read_project;
use super::pods::{read_namespace_pods, read_rollout_pods};
use super::read::Reader;
use super::rollouts::{AnalysisRun, Rollout, read_analysis_runs, read_rollouts};
use super::source::Source;
use super::tekton::{CommitNames, EvidenceResult, read_builds};

/// Which namespaces to read, all named by the caller.
#[derive(Clone, Debug)]
pub struct Plan {
    pub sha: String,
    /// The Kargo project namespace.
    pub kargo_project: String,
    /// The namespace Argo CD's Applications live in.
    pub argocd_namespace: String,
    /// The namespace PaC runs the build in.
    pub build_namespace: String,
    /// `owner/name` of the GitHub repository, when pull requests are read.
    pub github_repo: Option<String>,
    /// `(context name, server)` of the kubeconfig contexts a destination may
    /// be matched to. Servers are compared here and never output.
    pub contexts: Vec<(String, String)>,
    /// The context name of the environment cluster. Pods of an Application
    /// are read from it only when the Application's destination is that
    /// context; `None` accepts any destination that matches one.
    pub environment: Option<String>,
    /// Where the pipeline's own evidence record is, if the platform writes
    /// one. Not read unless configured.
    pub evidence_result: Option<EvidenceResult>,
    /// Parameter and result names, besides upstream's, under which the
    /// pipelines record the commit they built. Empty unless configured.
    pub commit_names: CommitNames,
    /// How Applications without the authorized-stage annotation are tied to
    /// stages. Off unless named.
    pub stage_naming: Option<StageNaming>,
}

/// One reader per role.
pub struct Clusters<'a, K, A, T, E, G> {
    pub kargo: &'a K,
    pub argocd: &'a A,
    pub tekton: &'a T,
    pub environment: &'a E,
    pub github: &'a G,
}

/// The first part that couldn't be read, else every part's items together,
/// capped when any part was.
fn merge<T>(parts: Vec<Source<Vec<T>>>) -> Source<Vec<T>> {
    let mut all = Vec::new();
    let mut truncated = None;
    for part in parts {
        match part {
            Source::Read(items) => all.extend(items),
            Source::Capped(items, truncation) => {
                all.extend(items);
                truncated = truncated.or(Some(truncation));
            }
            other => return other,
        }
    }
    match truncated {
        Some(truncation) => Source::Capped(all, truncation),
        None => Source::Read(all),
    }
}

pub async fn collect<K: Reader, A: Reader, T: Reader, E: Reader, G: GitHub>(
    clusters: &Clusters<'_, K, A, T, E, G>,
    plan: &Plan,
) -> Evidence {
    let builds = read_builds(clusters.tekton, &plan.build_namespace, &plan.sha).await;
    let kargo = read_project(clusters.kargo, &plan.kargo_project).await;
    let applications = read_applications(clusters.argocd, &plan.argocd_namespace).await;
    let destinations: BTreeMap<String, DestinationMatch> = applications
        .read()
        .map(|apps| {
            apps.iter()
                .map(|app| {
                    (
                        format!("{}/{}", app.namespace, app.name),
                        match_destination(app, &plan.contexts),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let mut evidence = Evidence {
        sha: plan.sha.clone(),
        evidence_result: plan.evidence_result.clone(),
        commit_names: plan.commit_names.clone(),
        stage_naming: plan.stage_naming.clone(),
        builds,
        kargo,
        applications,
        destinations,
        rollouts: Source::Read(Vec::new()),
        analysis_runs: Source::Read(Vec::new()),
        pods: BTreeMap::new(),
        namespace_pods: BTreeMap::new(),
        pull_requests: None,
        pr_builds: BTreeMap::new(),
    };
    read_pull_requests(clusters, plan, &mut evidence).await;

    // Only the Rollouts this change can reach are read, in their namespaces.
    let wanted = candidate_rollouts(&evidence);
    let mut namespaces: Vec<&str> = wanted.iter().map(|(ns, _)| ns.as_str()).collect();
    namespaces.sort_unstable();
    namespaces.dedup();
    let mut rollouts: Vec<Source<Vec<Rollout>>> = Vec::new();
    let mut analysis: Vec<Source<Vec<AnalysisRun>>> = Vec::new();
    for namespace in &namespaces {
        rollouts.push(read_rollouts(clusters.environment, namespace).await);
        analysis.push(read_analysis_runs(clusters.environment, namespace).await);
    }
    if !namespaces.is_empty() {
        evidence.rollouts = merge(rollouts);
        evidence.analysis_runs = merge(analysis);
    }
    if let Some(found) = evidence.rollouts.read() {
        let mut pods = BTreeMap::new();
        for (namespace, name) in &wanted {
            let Some(hash) = found
                .iter()
                .find(|r| r.namespace == *namespace && r.name == *name)
                .and_then(|r| r.current_pod_hash.clone())
            else {
                continue;
            };
            pods.insert(
                format!("{namespace}/{name}"),
                read_rollout_pods(clusters.environment, namespace, &hash).await,
            );
        }
        evidence.pods = pods;
    }

    // Applications that manage no Rollout: read the pods of their namespace,
    // but only where the destination is the environment cluster.
    for (app_id, namespace) in candidate_applications(&evidence) {
        let here = match evidence.destinations.get(&app_id) {
            Some(DestinationMatch::One(context)) => plan
                .environment
                .as_deref()
                .is_none_or(|environment| environment == context),
            _ => false,
        };
        if here {
            let pods = read_namespace_pods(clusters.environment, &namespace).await;
            evidence.namespace_pods.insert(app_id, pods);
        }
    }
    evidence
}

/// Pull requests for the commit, and the builds of each one's head commit
/// when that is a different commit.
async fn read_pull_requests<K: Reader, A: Reader, T: Reader, E: Reader, G: GitHub>(
    clusters: &Clusters<'_, K, A, T, E, G>,
    plan: &Plan,
    evidence: &mut Evidence,
) {
    let Some(repo) = &plan.github_repo else {
        return;
    };
    let read = Source::from_result(clusters.github.pulls_for_commit(repo, &plan.sha).await);
    if let Some(prs) = read.read() {
        for pr in prs
            .iter()
            .filter(|pr| !pr.head_sha.eq_ignore_ascii_case(&plan.sha))
        {
            evidence.pr_builds.insert(
                pr.number,
                read_builds(clusters.tekton, &plan.build_namespace, &pr.head_sha).await,
            );
        }
    }
    evidence.pull_requests = Some(read);
}

/// The commit a pull request leads to: its merge commit once merged, its head
/// commit before. The caller learns which.
pub async fn commit_of_pull_request<G: GitHub>(
    github: &G,
    repo: &str,
    number: u64,
) -> Source<(String, bool)> {
    Source::from_result(
        github
            .pull(repo, number)
            .await
            .map(|pr| match pr.merge_sha {
                Some(merge) => (merge, true),
                None => (pr.head_sha, false),
            }),
    )
}
