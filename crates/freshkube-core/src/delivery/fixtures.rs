//! A fake cluster for tests: invented names, invented digests and SHAs, and
//! a [`Reader`] that answers from a table and records what it was asked.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use super::github::{GitHub, PullRequest};
use super::read::{ListRequest, Listing, Reader, Resource, Scope, ServedGroup};
use super::source::Truncation;
use super::tekton::EvidenceResult;
use crate::resources::{Failure, FailureKind};

pub const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const OTHER_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub const NEW: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
pub const OLD: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";
pub const PUSHED: &str = "dddddddddddddddddddddddddddddddddddddddd";
pub const REPO: &str = "registry.example/acme/storefront";
/// An invented UID for the storefront Rollout.
pub const ROLLOUT_UID: &str = "0f0e0d0c-0000-4000-8000-000000000001";

#[derive(Default)]
pub struct FixtureReader {
    groups: Vec<ServedGroup>,
    plurals: HashMap<(String, String), Vec<String>>,
    lists: HashMap<String, Result<Vec<Value>, Failure>>,
    /// Kinds whose listing reports that it stopped at the page cap.
    capped: HashSet<String>,
    /// Kinds refused for one label selector, by plural and selector.
    refused_selectors: HashSet<(String, String)>,
    /// Every request, as a path-like description.
    pub requests: RefCell<Vec<String>>,
}

impl FixtureReader {
    pub fn serves(mut self, group: &str, version: &str, plurals: &[&str]) -> Self {
        match self.groups.iter_mut().find(|g| g.name == group) {
            Some(existing) => existing.versions.push(version.into()),
            None => self.groups.push(ServedGroup {
                name: group.into(),
                versions: vec![version.into()],
            }),
        }
        self.plurals.insert(
            (group.into(), version.into()),
            plurals.iter().map(|p| (*p).to_owned()).collect(),
        );
        self
    }

    pub fn with(mut self, plural: &str, items: Vec<Value>) -> Self {
        self.lists.insert(plural.into(), Ok(items));
        self
    }

    /// The kind's listing says it stopped at the page cap.
    pub fn capped(mut self, plural: &str) -> Self {
        self.capped.insert(plural.into());
        self
    }

    pub fn refusing(mut self, plural: &str) -> Self {
        self.lists.insert(
            plural.into(),
            Err(Failure::new(FailureKind::Forbidden, "forbidden by RBAC")),
        );
        self
    }

    /// The kind is refused for one label selector only; its other listings
    /// answer. RBAC can't refuse by selector, but this is how one build's
    /// TaskRuns fail while another's are read.
    pub fn refusing_selector(mut self, plural: &str, selector: &str) -> Self {
        self.refused_selectors
            .insert((plural.into(), selector.into()));
        self
    }

    pub fn unreachable(mut self, plural: &str) -> Self {
        self.lists.insert(
            plural.into(),
            Err(Failure::new(FailureKind::Unreachable, "connection refused")),
        );
        self
    }
}

/// The object with an invented `uid` and `resourceVersion` where it has
/// none, as a server's answer always has them.
fn stamped(mut item: Value, plural: &str, index: usize) -> Value {
    if let Some(metadata) = item.get_mut("metadata").and_then(Value::as_object_mut) {
        let name = metadata
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unnamed")
            .to_owned();
        metadata
            .entry("uid")
            .or_insert_with(|| Value::from(format!("uid-{plural}-{name}")));
        metadata
            .entry("resourceVersion")
            .or_insert_with(|| Value::from(format!("{}", 1000 + index)));
    }
    item
}

/// `key=value` terms, and a bare `key` for a label that exists.
fn matches(item: &Value, selector: &str) -> bool {
    let label = |key: &str| {
        item.pointer("/metadata/labels")
            .and_then(|labels| labels.get(key))
            .and_then(Value::as_str)
    };
    selector.split(',').all(|term| match term.split_once('=') {
        Some((key, value)) => label(key) == Some(value),
        None => label(term).is_some(),
    })
}

impl Reader for FixtureReader {
    async fn get(
        &self,
        resource: &Resource,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<Value, Failure> {
        self.requests.borrow_mut().push(format!(
            "GET {}/{} {namespace:?} {name}",
            resource.group, resource.plural
        ));
        Err(Failure::new(FailureKind::NotFound, "not in the fixture"))
    }

    async fn list(&self, request: &ListRequest) -> Result<Listing, Failure> {
        let (namespace, selector) = match &request.scope {
            Scope::Namespace(namespace) => (Some(namespace.clone()), None),
            Scope::Labels {
                namespace,
                selector,
            } => (namespace.clone(), Some(selector.clone())),
        };
        self.requests.borrow_mut().push(format!(
            "LIST {}/{}/{} ns={namespace:?} selector={selector:?}",
            request.resource.group, request.resource.version, request.resource.plural
        ));
        if let Some(selector) = &selector
            && self
                .refused_selectors
                .contains(&(request.resource.plural.clone(), selector.clone()))
        {
            return Err(Failure::new(FailureKind::Forbidden, "forbidden by RBAC"));
        }
        let items = self
            .lists
            .get(&request.resource.plural)
            .cloned()
            // A kind the fixture has nothing for is an empty, readable list.
            .unwrap_or(Ok(Vec::new()))?;
        let items: Vec<Value> = items
            .into_iter()
            .filter(|item| {
                namespace.as_deref().is_none_or(|namespace| {
                    item.pointer("/metadata/namespace").and_then(Value::as_str) == Some(namespace)
                })
            })
            .filter(|item| selector.as_deref().is_none_or(|s| matches(item, s)))
            .enumerate()
            .map(|(index, item)| stamped(item, &request.resource.plural, index))
            .collect();
        let truncated = self
            .capped
            .contains(&request.resource.plural)
            .then_some(Truncation { read: items.len() });
        Ok(Listing { items, truncated })
    }

    async fn groups(&self) -> Result<Vec<ServedGroup>, Failure> {
        self.requests.borrow_mut().push("GET /apis".into());
        Ok(self.groups.clone())
    }

    async fn plurals(&self, group: &str, version: &str) -> Result<Vec<String>, Failure> {
        self.requests
            .borrow_mut()
            .push(format!("GET /apis/{group}/{version}"));
        self.plurals
            .get(&(group.to_owned(), version.to_owned()))
            .cloned()
            .ok_or_else(|| Failure::new(FailureKind::NotFound, "version not served"))
    }
}

// ---- one healthy change, field by field -------------------------------

pub fn freight(name: &str, digest: &str, commit: &str) -> Value {
    json!({
        "metadata": {"name": name, "namespace": "storefront",
                      "labels": {"kargo.akuity.io/alias": "wonky-otter"}},
        "origin": {"kind": "Warehouse", "name": "storefront"},
        "commits": [{"repoURL": "https://git.example/acme/storefront.git", "id": commit}],
        "images": [{"repoURL": REPO, "tag": "v1.4.0", "digest": digest}],
        "status": {"verifiedIn": {"dev": {}}}
    })
}

pub fn stage(current: &[&str]) -> Value {
    let items: serde_json::Map<String, Value> = current
        .iter()
        .map(|name| {
            (
                format!("Warehouse/storefront/{name}"),
                json!({"name": name}),
            )
        })
        .collect();
    json!({
        "metadata": {"name": "dev", "namespace": "storefront"},
        "spec": {"requestedFreight": [{"origin": {"kind": "Warehouse", "name": "storefront"}}]},
        "status": {"freightHistory": [{"id": "x", "items": items}],
                    "health": {"status": "Healthy"}}
    })
}

/// A Promotion whose status records the Freight's image and the commit its
/// last step pushed, as Kargo writes them.
pub fn promotion_pushing(freight: &str, digest: &str, phase: &str, pushed: &str) -> Value {
    let images = json!([{"repoURL": REPO, "tag": "v1.4.0", "digest": digest}]);
    json!({
        "metadata": {"name": "dev.02.def", "namespace": "storefront"},
        "spec": {"stage": "dev", "freight": freight},
        "status": {
            "phase": phase,
            "freight": {"name": freight, "images": images},
            "state": {
                "step-1": {"commits": {"./repo": SHA}},
                "step-2": {"commitMessage": "x"},
                "step-3": {"commit": pushed, "branch": "main"}}}
    })
}

/// A Stage whose record of its current Freight carries the image digest.
pub fn stage_holding(current: &str, digest: &str) -> Value {
    json!({
        "metadata": {"name": "dev", "namespace": "storefront"},
        "spec": {"requestedFreight": [{"origin": {"kind": "Warehouse", "name": "storefront"}}]},
        "status": {"freightHistory": [{"id": "x", "items": {
            "Warehouse/storefront": {"name": current,
                                "images": [{"repoURL": REPO, "digest": digest}]}}}],
                    "health": {"status": "Healthy"}}
    })
}

pub fn promotion(freight: &str) -> Value {
    json!({
        "metadata": {"name": "dev.01.abc", "namespace": "storefront"},
        "spec": {"stage": "dev", "freight": freight},
        "status": {"phase": "Succeeded"}
    })
}

pub fn application(server: Option<&str>) -> Value {
    let destination = match server {
        Some(server) => json!({"server": server, "namespace": "shop"}),
        None => json!({"name": "env-dev", "namespace": "shop"}),
    };
    json!({
        "metadata": {"name": "storefront-dev", "namespace": "argocd",
                      "annotations": {"kargo.akuity.io/authorized-stage": "storefront:dev"}},
        "spec": {"project": "storefront", "destination": destination},
        "status": {"sync": {"status": "Synced", "revision": "cccccccccccccccccccccccccccccccccccccccc"},
                    "health": {"status": "Healthy"},
                    "resources": [
                        {"group": "argoproj.io", "kind": "Rollout", "namespace": "shop", "name": "storefront"},
                        {"group": "", "kind": "Service", "namespace": "shop", "name": "storefront"}]}
    })
}

pub fn rollout(image: &str) -> Value {
    json!({
        "metadata": {"name": "storefront", "namespace": "shop", "uid": ROLLOUT_UID},
        "spec": {"template": {"spec": {"containers": [{"name": "app", "image": image}]}}},
        "status": {"phase": "Healthy", "currentPodHash": "5d9c", "stableRS": "5d9c"}
    })
}

pub fn analysis_run() -> Value {
    json!({
        "metadata": {"name": "storefront-5d9c-1", "namespace": "shop",
                      "ownerReferences": [{"kind": "Rollout", "name": "storefront"}]},
        "status": {"phase": "Successful"}
    })
}

pub fn pod(name: &str, image: &str, image_id: &str) -> Value {
    json!({
        "metadata": {"name": name, "namespace": "shop",
                      "labels": {"rollouts-pod-template-hash": "5d9c"}},
        "status": {"containerStatuses": [
            {"name": "app", "image": image, "imageID": image_id, "ready": true}]}
    })
}

/// A pod of one ReplicaSet of the Rollout, by its pod-template hash.
pub fn pod_of(name: &str, hash: &str, image: &str, image_id: &str, ready: bool) -> Value {
    json!({
        "metadata": {"name": name, "namespace": "shop",
                      "labels": {"rollouts-pod-template-hash": hash}},
        "status": {"containerStatuses": [
            {"name": "app", "image": image, "imageID": image_id, "ready": ready}]}
    })
}

/// A ReplicaSet the storefront Rollout owns.
pub fn replica_set(hash: &str, image: &str, replicas: u64, ready: u64) -> Value {
    json!({
        "metadata": {"name": format!("storefront-{hash}"), "namespace": "shop",
                      "labels": {"rollouts-pod-template-hash": hash},
                      "ownerReferences": [{"apiVersion": "argoproj.io/v1alpha1", "kind": "Rollout",
                                           "name": "storefront", "uid": ROLLOUT_UID,
                                           "controller": true}]},
        "spec": {"template": {"spec": {"containers": [{"name": "app", "image": image}]}}},
        "status": {"replicas": replicas, "readyReplicas": ready}
    })
}

/// The Rollout at the Freight's digest, running as ReplicaSet `5d9c`, beside
/// `7f3b`: an older ReplicaSet of it that never became healthy, was never
/// promoted, and still has a crash-looping pod of another image. The
/// Rollout's status reports `current` as its current pod hash.
pub fn beside_a_stale_replica_set(current: &str) -> World {
    let mut world = healthy();
    let pinned = format!("{REPO}@{NEW}");
    let stale = format!("{REPO}@{OLD}");
    let mut rollout = rollout(&pinned);
    rollout["status"]["currentPodHash"] = json!(current);
    world.environment = world
        .environment
        .with("rollouts", vec![rollout])
        .with(
            "replicasets",
            vec![
                replica_set("7f3b", &stale, 1, 0),
                replica_set("5d9c", &pinned, 2, 2),
            ],
        )
        .with(
            "pods",
            vec![
                pod_of(
                    "storefront-7f3b-a",
                    "7f3b",
                    &stale,
                    &format!("docker-pullable://{stale}"),
                    false,
                ),
                pod_of(
                    "storefront-5d9c-a",
                    "5d9c",
                    &pinned,
                    &format!("docker-pullable://{pinned}"),
                    true,
                ),
                pod_of(
                    "storefront-5d9c-b",
                    "5d9c",
                    &pinned,
                    &format!("docker-pullable://{pinned}"),
                    true,
                ),
            ],
        );
    world
}

pub fn pipeline_run(sha: &str, with_revision: bool, digest: Option<&str>) -> Value {
    let mut params =
        vec![json!({"name": "git-url", "value": "https://git.example/acme/storefront.git"})];
    if with_revision {
        params.push(json!({"name": "revision", "value": sha}));
    }
    let mut results = Vec::new();
    if let Some(digest) = digest {
        results.push(json!({"name": "IMAGE_URL", "value": format!("{REPO}:v1.4.0")}));
        results.push(json!({"name": "IMAGE_DIGEST", "value": digest}));
    }
    json!({
        "metadata": {"name": "storefront-push-x", "namespace": "acme-builds",
                      "labels": {"pipelinesascode.tekton.dev/sha": sha,
                                  "pipelinesascode.tekton.dev/repository": "storefront"},
                      "annotations": {"chains.tekton.dev/signed": "true"}},
        "spec": {"params": params},
        "status": {"conditions": [{"type": "Succeeded", "status": "True", "reason": "Succeeded"}],
                    "results": results,
                    "childReferences": [{"kind": "TaskRun", "name": "storefront-push-x-build"}]}
    })
}

/// An invented place for a pipeline's own record: a result name and the
/// pointers into its JSON.
pub fn evidence_result() -> EvidenceResult {
    EvidenceResult {
        result: "MADE_UP_RECORD".into(),
        commit_pointer: "/commit".into(),
        digest_pointer: "/digest".into(),
    }
}

/// A task run that, besides the Conforma result, writes an evidence record
/// naming `sha` and `digest`.
pub fn evidence_task_run(sha: &str, digest: &str) -> Value {
    let mut run = task_run();
    let record = json!({"commit": sha, "digest": digest});
    run["status"]["results"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": evidence_result().result, "value": record.to_string()}));
    run
}

pub fn task_run() -> Value {
    json!({
        "metadata": {"name": "storefront-push-x-verify", "namespace": "acme-builds",
                      "labels": {"tekton.dev/pipelineRun": "storefront-push-x"}},
        "status": {"results": [
            {"name": "TEST_OUTPUT", "value": "{\"result\":\"SUCCESS\",\"failures\":0,\"warnings\":1}"}]}
    })
}

/// One healthy change, with a reader for each role the join reads from:
/// Kargo, Argo CD, the builds and the environment the pods run in.
pub struct World {
    pub kargo: FixtureReader,
    pub argocd: FixtureReader,
    pub tekton: FixtureReader,
    pub environment: FixtureReader,
}

pub fn healthy() -> World {
    let image_id = format!("docker-pullable://{REPO}@{NEW}");
    World {
        kargo: FixtureReader::default()
            .serves(
                "kargo.akuity.io",
                "v1alpha1",
                &["warehouses", "freights", "stages", "promotions"],
            )
            .with(
                "freights",
                vec![freight("f-new", NEW, SHA), freight("f-old", OLD, OTHER_SHA)],
            )
            .with("stages", vec![stage(&["f-new"])])
            .with("promotions", vec![promotion("f-new")]),
        argocd: FixtureReader::default()
            .serves("argoproj.io", "v1alpha1", &["applications"])
            .with(
                "applications",
                vec![application(Some("https://env-a.example:6443"))],
            ),
        tekton: FixtureReader::default()
            .serves("tekton.dev", "v1", &["pipelineruns", "taskruns"])
            .with("pipelineruns", vec![pipeline_run(SHA, true, Some(NEW))])
            .with("taskruns", vec![task_run()]),
        environment: FixtureReader::default()
            .serves("argoproj.io", "v1alpha1", &["rollouts", "analysisruns"])
            .with("rollouts", vec![rollout(&format!("{REPO}:v1.4.0"))])
            .with("analysisruns", vec![analysis_run()])
            .with(
                "pods",
                vec![pod(
                    "storefront-5d9c-x",
                    &format!("{REPO}:v1.4.0"),
                    &image_id,
                )],
            ),
    }
}

/// A setup without Argo Rollouts: the Application carries no authorized-stage
/// annotation, names its Kargo project in an annotation of the setup's choosing
/// and its stage in its own name, and manages a plain Deployment, which the
/// join doesn't look inside. The environment cluster serves only core kinds.
pub fn without_rollouts() -> World {
    without_rollouts_with(unannotated_application())
}

/// The Application of [`without_rollouts`].
pub fn unannotated_application() -> Value {
    json!({
        "metadata": {"name": "app-storefront-in-dev", "namespace": "argocd",
                      "annotations": {"example.test/project": "storefront"}},
        "spec": {"destination": {"server": "https://env-a.example:6443", "namespace": "shop"}},
        "status": {"sync": {"status": "Synced"}, "health": {"status": "Healthy"},
                    "summary": {"images": [format!("{REPO}@{NEW}")]},
                    "resources": [{"group": "apps", "kind": "Deployment",
                                   "namespace": "shop", "name": "storefront"}]}
    })
}

/// [`without_rollouts`] with another Application.
pub fn without_rollouts_with(application: Value) -> World {
    let mut world = healthy();
    world.argocd = FixtureReader::default()
        .serves("argoproj.io", "v1alpha1", &["applications"])
        .with("applications", vec![application]);
    world.environment = FixtureReader::default().with(
        "pods",
        vec![pod(
            "storefront-6fdf-x",
            &format!("{REPO}@{NEW}"),
            &format!("docker-pullable://{REPO}@{NEW}"),
        )],
    );
    world
}

pub const GH_REPO: &str = "acme/storefront";

pub fn pull_request(number: u64, head: &str, merge: Option<&str>) -> PullRequest {
    PullRequest {
        repo: GH_REPO.into(),
        number,
        title: Some("Add the thing".into()),
        state: Some(if merge.is_some() { "closed" } else { "open" }.into()),
        merged: merge.is_some(),
        head_sha: head.into(),
        merge_sha: merge.map(str::to_owned),
        base_branch: Some("main".into()),
    }
}

/// A build PaC started for a pull request's head commit.
pub fn pr_pipeline_run(sha: &str, number: u64, digest: &str) -> Value {
    let mut run = pipeline_run(sha, true, Some(digest));
    run["metadata"]["name"] = json!("storefront-pr-y");
    run["metadata"]["annotations"]["pipelinesascode.tekton.dev/pull-request"] =
        json!(number.to_string());
    run["metadata"]["labels"]["pipelinesascode.tekton.dev/event-type"] = json!("pull_request");
    run
}

#[derive(Default)]
pub struct FixtureGitHub {
    pub pulls: Option<Result<Vec<PullRequest>, Failure>>,
    pub requests: RefCell<Vec<String>>,
}

impl FixtureGitHub {
    pub fn with(pulls: Vec<PullRequest>) -> Self {
        Self {
            pulls: Some(Ok(pulls)),
            ..Self::default()
        }
    }

    pub fn refusing() -> Self {
        Self {
            pulls: Some(Err(Failure::new(
                FailureKind::Forbidden,
                "gh: Forbidden (HTTP 403)",
            ))),
            ..Self::default()
        }
    }
}

impl GitHub for FixtureGitHub {
    async fn pulls_for_commit(&self, repo: &str, sha: &str) -> Result<Vec<PullRequest>, Failure> {
        self.requests
            .borrow_mut()
            .push(format!("pulls {repo} {sha}"));
        self.pulls.clone().unwrap_or(Ok(Vec::new()))
    }

    async fn pull(&self, repo: &str, number: u64) -> Result<PullRequest, Failure> {
        self.requests
            .borrow_mut()
            .push(format!("pull {repo} {number}"));
        self.pulls
            .clone()
            .unwrap_or(Ok(Vec::new()))?
            .into_iter()
            .find(|pr| pr.number == number)
            .ok_or_else(|| Failure::new(FailureKind::NotFound, "gh: Not Found (HTTP 404)"))
    }
}
