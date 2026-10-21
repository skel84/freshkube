//! A Freight that names an image and no commit is joined to its build by the
//! image digest a result of the build reports.
use serde_json::{Value, json};

use super::digest::Digest;
use super::fixtures::*;
use super::join::{Confidence, Hop, Key, Link, Trail};
use super::tests::{link, one, run_configured};

const CONTEXTS: &[(&str, &str)] = &[("env-a", "https://env-a.example:6443")];

/// The Freight with its commit taken away, as a Warehouse that subscribes to
/// an image repository alone makes it.
fn image_only(name: &str, digest: &str) -> Value {
    let mut item = freight(name, digest, SHA);
    item["commits"] = json!([]);
    item
}

/// A run no Pipelines as Code label or revision parameter names a commit for,
/// reporting `digest` among its results when it is given.
fn bare_run(name: &str, digest: Option<&str>) -> Value {
    let mut results = Vec::new();
    if let Some(digest) = digest {
        results.push(json!({"name": "IMAGE_URL", "value": format!("{REPO}:v1.4.0")}));
        results.push(json!({"name": "IMAGE_DIGEST", "value": digest}));
    }
    json!({
        "metadata": {"name": name, "namespace": "acme-builds"},
        "status": {"conditions": [{"type": "Succeeded", "status": "True", "reason": "Succeeded"}],
                    "results": results}
    })
}

/// A TaskRun of `run` that reports `digest`, as a build task does.
fn building_task(run: &str, digest: &str) -> Value {
    json!({
        "metadata": {"name": format!("{run}-build"), "namespace": "acme-builds",
                      "labels": {"tekton.dev/pipelineRun": run}},
        "status": {"results": [
            {"name": "IMAGE_URL", "value": format!("{REPO}:v1.4.0")},
            {"name": "IMAGE_DIGEST", "value": digest}]}
    })
}

fn image_only_world(runs: Vec<Value>, tasks: Vec<Value>) -> World {
    let mut world = healthy();
    world.kargo = world.kargo.with(
        "freights",
        vec![image_only("f-new", NEW), image_only("f-old", OLD)],
    );
    world.tekton = world
        .tekton
        .with("pipelineruns", runs)
        .with("taskruns", tasks);
    world
}

async fn by_digest(world: &World, namespace: Option<&str>) -> Trail {
    run_configured(world, CONTEXTS, &FixtureGitHub::default(), None, |plan| {
        plan.sha = String::new();
        plan.digests = vec![Digest::parse(NEW).unwrap()];
        plan.build_namespace = Ok(namespace.map(str::to_owned));
    })
    .await
}

fn of_freight<'a>(trail: &'a Trail, subject: &str) -> &'a Link {
    link(trail, Hop::PipelineRun, Hop::Freight)
        .into_iter()
        .find(|link| link.subject == subject)
        .unwrap_or_else(|| panic!("no link into {subject} in {trail:#?}"))
}

fn image_link(trail: &Trail) -> &Link {
    one(trail, Hop::Image, Hop::PipelineRun)
}

#[tokio::test]
async fn an_image_only_freight_joins_the_run_that_reports_its_digest() {
    let world = image_only_world(vec![bare_run("digest-run", Some(NEW))], vec![]);
    let trail = by_digest(&world, Some("acme-builds")).await;
    let built = image_link(&trail);
    assert_eq!(built.confidence, Confidence::Confirmed);
    assert_eq!(built.subject, "acme-builds/digest-run");
    assert!(matches!(&built.key, Key::Digest(d) if d.as_str() == NEW));
    assert!(
        built.reason.contains("a result of the PipelineRun"),
        "{}",
        built.reason
    );
    // No commit is known, so none is claimed.
    assert!(link(&trail, Hop::Commit, Hop::PipelineRun).is_empty());
    let freight = one(&trail, Hop::PipelineRun, Hop::Freight);
    assert_eq!(freight.confidence, Confidence::Confirmed);
    assert_eq!(freight.subject, "storefront/f-new");
    assert!(matches!(&freight.key, Key::Digest(d) if d.as_str() == NEW));
}

#[tokio::test]
async fn the_same_through_a_taskrun_result() {
    let world = image_only_world(
        vec![bare_run("digest-run", None)],
        vec![building_task("digest-run", NEW)],
    );
    let trail = by_digest(&world, Some("acme-builds")).await;
    let built = image_link(&trail);
    assert_eq!(built.confidence, Confidence::Confirmed);
    assert_eq!(built.subject, "acme-builds/digest-run");
    assert!(
        built.reason.contains("one of its TaskRuns"),
        "{}",
        built.reason
    );
    assert_eq!(
        one(&trail, Hop::PipelineRun, Hop::Freight).confidence,
        Confidence::Confirmed
    );
}

#[tokio::test]
async fn no_match_says_what_was_read_and_joins_nothing() {
    let world = image_only_world(vec![bare_run("other-run", Some(OLD))], vec![]);
    let trail = by_digest(&world, Some("acme-builds")).await;
    let none = image_link(&trail);
    assert_eq!(none.confidence, Confidence::Unknown);
    assert_eq!(none.subject, "-");
    assert!(
        none.reason
            .contains("no PipelineRun or TaskRun result reports image digest sha256:111111111111")
            && none.reason.contains("read 1 PipelineRuns and 0 TaskRuns"),
        "{}",
        none.reason
    );
    assert!(
        link(&trail, Hop::PipelineRun, Hop::Freight)
            .iter()
            .all(|link| link.confidence == Confidence::Unknown)
    );
}

#[tokio::test]
async fn without_a_build_namespace_nothing_is_listed() {
    let world = image_only_world(vec![bare_run("digest-run", Some(NEW))], vec![]);
    let trail = by_digest(&world, None).await;
    let none = image_link(&trail);
    assert_eq!(none.confidence, Confidence::Unknown);
    assert!(
        none.reason.contains("no build namespace is set"),
        "{}",
        none.reason
    );
    assert!(
        !world
            .tekton
            .requests
            .borrow()
            .iter()
            .any(|request| request.starts_with("LIST tekton.dev")),
        "{:?}",
        world.tekton.requests
    );
}

#[tokio::test]
async fn the_search_lists_one_namespace_twice_and_never_by_selector() {
    let world = image_only_world(vec![bare_run("digest-run", Some(NEW))], vec![]);
    by_digest(&world, Some("acme-builds")).await;
    let lists: Vec<String> = world
        .tekton
        .requests
        .borrow()
        .iter()
        .filter(|request| request.starts_with("LIST tekton.dev"))
        .cloned()
        .collect();
    assert_eq!(lists.len(), 2, "{lists:?}");
    assert!(
        lists
            .iter()
            .all(|list| list.contains("ns=Some(\"acme-builds\") selector=None")),
        "{lists:?}"
    );
}

#[tokio::test]
async fn a_capped_listing_says_the_build_may_be_among_those_not_read() {
    let mut world = image_only_world(vec![bare_run("other-run", Some(OLD))], vec![]);
    world.tekton = world.tekton.capped("pipelineruns");
    let trail = by_digest(&world, Some("acme-builds")).await;
    let none = image_link(&trail);
    assert_eq!(none.confidence, Confidence::Unknown);
    assert!(
        none.reason.contains("page cap") && none.reason.contains("may be among those not read"),
        "{}",
        none.reason
    );
}

/// A Freight that names the commit and the digest, built twice: the commit's
/// own run reports another digest, and a run no label ties to the commit
/// reports the Freight's.
fn disagreeing_world(digest_run_commit: Option<&str>) -> World {
    let mut world = healthy();
    let mut commit_run = pipeline_run(SHA, true, Some(OLD));
    commit_run["metadata"]["name"] = json!("storefront-push-x");
    let mut tasks = vec![clone_task_run("storefront-push-x", SHA), task_run()];
    let mut digest_run = bare_run("digest-run", Some(NEW));
    if let Some(commit) = digest_run_commit {
        tasks.push(clone_task_run("digest-run", commit));
        digest_run["status"]["childReferences"] =
            json!([{"kind": "TaskRun", "name": "digest-run-clone"}]);
    }
    world.tekton = world
        .tekton
        .with("pipelineruns", vec![commit_run, digest_run])
        .with("taskruns", tasks);
    world
}

async fn by_both(world: &World) -> Trail {
    run_configured(world, CONTEXTS, &FixtureGitHub::default(), None, |plan| {
        plan.digests = vec![Digest::parse(NEW).unwrap()];
    })
    .await
}

#[tokio::test]
async fn a_commit_run_reporting_another_digest_makes_the_digest_join_a_claim() {
    let world = disagreeing_world(None);
    let trail = by_both(&world).await;
    // Both builds are shown, each by its own key.
    let by_commit = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert!(matches!(&by_commit.key, Key::Sha(sha) if sha == SHA));
    assert_eq!(by_commit.subject, "acme-builds/storefront-push-x");
    let by_image = image_link(&trail);
    assert_eq!(by_image.subject, "acme-builds/digest-run");
    assert_eq!(by_image.confidence, Confidence::Confirmed);
    let freight = of_freight(&trail, "storefront/f-new");
    assert!(matches!(&freight.key, Key::Digest(d) if d.as_str() == NEW));
    assert_eq!(freight.confidence, Confidence::Claimed);
    assert!(
        freight.reason.contains("the commit does not agree"),
        "{}",
        freight.reason
    );
    assert_eq!(freight.conflicts.len(), 1);
    assert_eq!(freight.conflicts[0].builds[0].digest.as_str(), OLD);
}

#[tokio::test]
async fn a_digest_run_reporting_another_commit_makes_the_digest_join_a_claim() {
    let world = disagreeing_world(Some(OTHER_SHA));
    let trail = by_both(&world).await;
    let freight = of_freight(&trail, "storefront/f-new");
    assert_eq!(freight.confidence, Confidence::Claimed);
    assert!(
        freight
            .reason
            .contains("reports commit bbbbbbbbbbbb, not aaaaaaaaaaaa"),
        "{}",
        freight.reason
    );
}

#[tokio::test]
async fn a_run_found_by_both_keys_is_the_commits() {
    let world = healthy();
    let trail = by_both(&world).await;
    assert!(link(&trail, Hop::Image, Hop::PipelineRun).is_empty());
    assert_eq!(
        one(&trail, Hop::PipelineRun, Hop::Freight).confidence,
        Confidence::Confirmed
    );
}

fn with_url(mut run: Value, url: &str) -> Value {
    run["status"]["results"][0]["value"] = json!(url);
    run
}

#[tokio::test]
async fn a_digest_result_alone_counts() {
    let mut run = bare_run("digest-run", Some(NEW));
    run["status"]["results"] = json!([{"name": "IMAGE_DIGEST", "value": NEW}]);
    let world = image_only_world(vec![run], vec![]);
    let trail = by_digest(&world, Some("acme-builds")).await;
    assert_eq!(image_link(&trail).confidence, Confidence::Confirmed);
    assert_eq!(
        one(&trail, Hop::PipelineRun, Hop::Freight).confidence,
        Confidence::Confirmed
    );
}

#[tokio::test]
async fn prefixed_type_hints_count() {
    let mut run = bare_run("digest-run", None);
    run["status"]["results"] = json!([
        {"name": "build-IMAGE_URL", "value": format!("{REPO}:v1.4.0")},
        {"name": "build-IMAGE_DIGEST", "value": NEW}
    ]);
    let world = image_only_world(vec![run], vec![]);
    let trail = by_digest(&world, Some("acme-builds")).await;
    assert_eq!(image_link(&trail).confidence, Confidence::Confirmed);
}

#[tokio::test]
async fn the_same_digest_under_another_repository_stays_confirmed() {
    let run = with_url(
        bare_run("digest-run", Some(NEW)),
        "mirror.example/acme/storefront:v1",
    );
    let world = image_only_world(vec![run], vec![]);
    let trail = by_digest(&world, Some("acme-builds")).await;
    assert_eq!(image_link(&trail).confidence, Confidence::Confirmed);
    assert_eq!(
        one(&trail, Hop::PipelineRun, Hop::Freight).confidence,
        Confidence::Confirmed
    );
}

#[tokio::test]
async fn a_failed_digest_read_leaves_the_commit_builds_and_skips_the_archive() {
    use super::collect::{Clusters, collect};
    use super::join::DigestOutcome;
    let mut world = healthy();
    world.tekton = world.tekton.refusing_namespace_lists("pipelineruns");
    let github = FixtureGitHub::default();
    let clusters = Clusters {
        kargo: &world.kargo,
        argocd: &world.argocd,
        tekton: &world.tekton,
        environment: &world.environment,
        github: &github,
    };
    let mut plan = super::tests::plan(CONTEXTS);
    plan.digests = vec![Digest::parse(NEW).unwrap()];
    let evidence = collect(&clusters, &plan, super::tests::observed_at()).await;
    let search = evidence.digest_search.as_ref().unwrap();
    assert!(
        matches!(search.outcome, DigestOutcome::Failed(_)),
        "{search:?}"
    );
    assert!(search.archive.is_none());
    assert_eq!(evidence.builds.read().unwrap().len(), 1);
    let trail = super::join::join(&evidence);
    // The commit still joins; the failed digest search adds no link beside it.
    assert!(link(&trail, Hop::Image, Hop::PipelineRun).is_empty());
    assert_eq!(
        one(&trail, Hop::PipelineRun, Hop::Freight).confidence,
        Confidence::Confirmed
    );
}
