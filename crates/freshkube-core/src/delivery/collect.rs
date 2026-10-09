//! Reads what one change needs, in the order the chain leads, and hands it to
//! [`join`](super::join::join). Each cluster has its own [`Reader`], so the
//! Kargo, Argo CD, CI/CD and environment clusters may be four contexts or
//! fewer; the caller says which is which.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use super::argocd::{
    DestinationMatch, Destinations, MapHint, StageNaming, match_destination, read_applications,
};
use super::deployments::{
    Deployment, DeploymentSet, current_set, read_deployment_sets, read_deployments,
};
use super::github::GitHub;
use super::join::{
    Evidence, RolloutPods, candidate_applications, candidate_deployments, candidate_rollouts,
};
use super::kargo::read_project;
use super::pods::{read_deployment_pods, read_namespace_pods, read_rollout_pods};
use super::read::Reader;
use super::rollouts::{
    AnalysisRun, ReplicaSet, Rollout, read_analysis_runs, read_replica_sets, read_rollouts,
};
use super::source::Source;
use super::tekton::{CommitNames, EvidenceResult, read_builds};

/// Which namespaces to read, all named by the caller.
#[derive(Clone, Debug)]
pub struct Plan {
    /// The commit; empty when the change is followed from a Freight that
    /// names none, and then no build is read.
    pub sha: String,
    /// A Freight followed by name, whose Stages' workloads and pods are
    /// read whether or not it matches the commit.
    pub followed: Option<String>,
    /// The Kargo project namespace.
    pub kargo_project: String,
    /// The namespace Argo CD's Applications live in.
    pub argocd_namespace: String,
    /// The namespace PaC runs the build in; every namespace when `None`.
    pub build_namespace: Option<String>,
    /// `owner/name` of the GitHub repository, when pull requests are read.
    pub github_repo: Option<String>,
    /// `(context name, server)` of the kubeconfig contexts a destination may
    /// be matched to. Servers are compared here and never output.
    pub contexts: Vec<(String, String)>,
    /// `(context name, cluster name)`: Argo CD cluster names the user mapped
    /// to contexts. A destination given by any other name matches nothing,
    /// except `in-cluster`. Empty unless configured.
    pub cluster_names: Vec<(String, String)>,
    /// The context name of the cluster Argo CD runs in, one of `contexts`:
    /// the cluster Argo CD calls `in-cluster`.
    pub argocd: String,
    /// The context name of the environment cluster, one of `contexts`.
    /// Rollouts and pods of an Application are read from it only when the
    /// Application's destination is that context.
    pub environment: String,
    /// Where the pipeline's own evidence record is, if the platform writes
    /// one. Not read unless configured.
    pub evidence_result: Option<EvidenceResult>,
    /// Parameter and result names, besides upstream's, under which the
    /// pipelines record the commit they built. Empty unless configured.
    pub commit_names: CommitNames,
    /// How Applications without the authorized-stage annotation are tied to
    /// stages. Off unless named.
    pub stage_naming: Option<StageNaming>,
    /// How a destination no context matches says it could be matched.
    pub map_hint: MapHint,
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
    observed_at: DateTime<Utc>,
) -> Evidence {
    let builds = if plan.sha.is_empty() {
        Source::Unreadable("the Freight names no commit to find its builds by".into())
    } else {
        read_builds(clusters.tekton, plan.build_namespace.as_deref(), &plan.sha).await
    };
    let kargo = read_project(clusters.kargo, &plan.kargo_project).await;
    let applications = read_applications(clusters.argocd, &plan.argocd_namespace).await;
    let destinations: BTreeMap<String, DestinationMatch> = applications
        .read()
        .map(|apps| {
            apps.iter()
                .map(|app| {
                    (
                        format!("{}/{}", app.namespace, app.name),
                        match_destination(
                            app,
                            Destinations {
                                contexts: &plan.contexts,
                                names: &plan.cluster_names,
                                argocd: &plan.argocd,
                            },
                        ),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let mut evidence = Evidence {
        sha: plan.sha.clone(),
        followed: plan.followed.clone(),
        observed_at,
        evidence_result: plan.evidence_result.clone(),
        commit_names: plan.commit_names.clone(),
        stage_naming: plan.stage_naming.clone(),
        map_hint: plan.map_hint,
        builds,
        kargo,
        applications,
        destinations,
        environment: plan.environment.clone(),
        rollouts: Source::Read(Vec::new()),
        analysis_runs: Source::Read(Vec::new()),
        replica_sets: Source::Read(Vec::new()),
        pods: BTreeMap::new(),
        deployments: Source::Read(Vec::new()),
        deployment_sets: Source::Read(Vec::new()),
        deployment_pods: BTreeMap::new(),
        namespace_pods: BTreeMap::new(),
        pull_requests: None,
        pr_builds: BTreeMap::new(),
    };
    read_pull_requests(clusters, plan, &mut evidence).await;

    // Only the Rollouts this change can reach are read, in their namespaces.
    let wanted = candidate_rollouts(&evidence);
    let mut namespaces: Vec<&str> = wanted.iter().map(|w| w.namespace.as_str()).collect();
    namespaces.sort_unstable();
    namespaces.dedup();
    let mut rollouts: Vec<Source<Vec<Rollout>>> = Vec::new();
    let mut analysis: Vec<Source<Vec<AnalysisRun>>> = Vec::new();
    let mut replica_sets: Vec<Source<Vec<ReplicaSet>>> = Vec::new();
    for namespace in &namespaces {
        rollouts.push(read_rollouts(clusters.environment, namespace).await);
        analysis.push(read_analysis_runs(clusters.environment, namespace).await);
        replica_sets.push(read_replica_sets(clusters.environment, namespace).await);
    }
    if !namespaces.is_empty() {
        evidence.rollouts = merge(rollouts);
        evidence.analysis_runs = merge(analysis);
        evidence.replica_sets = merge(replica_sets);
    }
    if let Some(found) = evidence.rollouts.read() {
        // A Rollout's pods are those of its current revision; the stable
        // ReplicaSet's, during a canary, run the previous one and are not read.
        let mut pods = BTreeMap::new();
        for wanted in &wanted {
            let Some(hash) = found
                .iter()
                .find(|r| r.namespace == wanted.namespace && r.name == wanted.name)
                .and_then(|rollout| rollout.current_pod_hash.clone())
            else {
                continue;
            };
            let read = read_rollout_pods(clusters.environment, &wanted.namespace, &hash).await;
            pods.insert(
                format!("{}/{}", wanted.namespace, wanted.name),
                RolloutPods { hash, pods: read },
            );
        }
        evidence.pods = pods;
    }

    read_deployment_chain(clusters.environment, &mut evidence).await;

    // Applications that manage no Rollout: read the pods of their namespace.
    // Both candidate lists hold only Applications whose destination is the
    // environment cluster.
    for (app_id, namespace) in candidate_applications(&evidence) {
        let pods = read_namespace_pods(clusters.environment, &namespace).await;
        evidence.namespace_pods.insert(app_id, pods);
    }
    evidence
}

/// The Deployments this change can reach, in their namespaces, the
/// ReplicaSets they own, and the pods of each one's current ReplicaSet.
async fn read_deployment_chain<E: Reader>(environment: &E, evidence: &mut Evidence) {
    let wanted = candidate_deployments(evidence);
    let mut namespaces: Vec<&str> = wanted.iter().map(|w| w.namespace.as_str()).collect();
    namespaces.sort_unstable();
    namespaces.dedup();
    if namespaces.is_empty() {
        return;
    }
    let mut deployments: Vec<Source<Vec<Deployment>>> = Vec::new();
    let mut sets: Vec<Source<Vec<DeploymentSet>>> = Vec::new();
    for namespace in &namespaces {
        deployments.push(read_deployments(environment, namespace).await);
        sets.push(read_deployment_sets(environment, namespace).await);
    }
    evidence.deployments = merge(deployments);
    evidence.deployment_sets = merge(sets);
    let (Some(found), Some(owned)) = (evidence.deployments.read(), evidence.deployment_sets.read())
    else {
        return;
    };
    // Only the current ReplicaSet's pods are read: the controller names it by
    // revision, and an older revision's pods run another template.
    let mut pods = BTreeMap::new();
    for wanted in &wanted {
        let Some(deployment) = found
            .iter()
            .find(|d| d.namespace == wanted.namespace && d.name == wanted.name)
        else {
            continue;
        };
        let Ok(set) = current_set(deployment, owned) else {
            continue;
        };
        let Some(hash) = set.pod_hash.clone() else {
            continue;
        };
        let read = read_deployment_pods(environment, &wanted.namespace, &hash).await;
        pods.insert(
            format!("{}/{}", wanted.namespace, wanted.name),
            RolloutPods { hash, pods: read },
        );
    }
    evidence.deployment_pods = pods;
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
                read_builds(
                    clusters.tekton,
                    plan.build_namespace.as_deref(),
                    &pr.head_sha,
                )
                .await,
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
