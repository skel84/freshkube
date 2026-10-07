use super::argocd::{
    DestinationMatch, Destinations, StageNaming, match_destination, parse_application,
};
use super::collect::{Clusters, Plan, collect, commit_of_pull_request};
use super::fixtures::*;
use super::join::{Confidence, Hop, Key, Trail, join};
use super::kargo::{parse_freight, parse_stage};
use super::source::Source;
use super::tekton::{built_images, conforma, read_builds};

fn plan(contexts: &[(&str, &str)]) -> Plan {
    Plan {
        sha: SHA.into(),
        kargo_project: "storefront".into(),
        argocd_namespace: "argocd".into(),
        build_namespace: "acme-builds".into(),
        github_repo: None,
        environment: "env-a".into(),
        argocd: "core".into(),
        cluster_names: Vec::new(),
        evidence_result: None,
        stage_naming: None,
        commit_names: Default::default(),
        contexts: contexts
            .iter()
            .map(|(name, server)| ((*name).into(), (*server).into()))
            .collect(),
    }
}

async fn run(world: &World, contexts: &[(&str, &str)]) -> Trail {
    run_with(world, contexts, &FixtureGitHub::default(), None).await
}

async fn run_with(
    world: &World,
    contexts: &[(&str, &str)],
    github: &FixtureGitHub,
    repo: Option<&str>,
) -> Trail {
    run_configured(world, contexts, github, repo, |_| {}).await
}

/// An invented naming a setup without the authorized-stage annotation might
/// configure.
fn naming() -> StageNaming {
    StageNaming {
        project_key: "example.test/project".into(),
        name_template: "app-{project}-in-{stage}".into(),
    }
}

async fn run_configured(
    world: &World,
    contexts: &[(&str, &str)],
    github: &FixtureGitHub,
    repo: Option<&str>,
    configure: impl FnOnce(&mut Plan),
) -> Trail {
    let clusters = Clusters {
        kargo: &world.kargo,
        argocd: &world.argocd,
        tekton: &world.tekton,
        environment: &world.environment,
        github,
    };
    let mut plan = plan(contexts);
    plan.github_repo = repo.map(str::to_owned);
    configure(&mut plan);
    join(&collect(&clusters, &plan).await)
}

fn link(trail: &Trail, from: Hop, to: Hop) -> Vec<&super::join::Link> {
    trail
        .links
        .iter()
        .filter(|l| l.from == from && l.to == to)
        .collect()
}

fn one(trail: &Trail, from: Hop, to: Hop) -> &super::join::Link {
    let found = link(trail, from, to);
    assert_eq!(found.len(), 1, "{from:?}->{to:?} in {trail:#?}");
    found[0]
}

#[tokio::test]
async fn a_healthy_change_joins_from_the_commit_to_the_pods() {
    let trail = run(&healthy(), &[("env-a", "https://env-a.example:6443")]).await;
    let commit = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(commit.confidence, Confidence::Confirmed);
    assert_eq!(commit.key, Key::Sha(SHA.into()));
    let chain = one(&trail, Hop::PipelineRun, Hop::SupplyChain);
    assert_eq!(chain.confidence, Confidence::Confirmed);
    assert!(
        chain
            .reason
            .contains("Conforma SUCCESS (0 failures, 1 warnings)")
    );
    let freight = one(&trail, Hop::PipelineRun, Hop::Freight);
    assert_eq!(freight.confidence, Confidence::Confirmed);
    assert_eq!(freight.subject, "storefront/f-new");
    assert!(matches!(&freight.key, Key::Digest(d) if d.as_str() == NEW));
    // Names and annotations are claims.
    assert_eq!(
        one(&trail, Hop::Freight, Hop::Stage).confidence,
        Confidence::Claimed
    );
    assert_eq!(
        one(&trail, Hop::Freight, Hop::Promotion).confidence,
        Confidence::Claimed
    );
    let app = one(&trail, Hop::Stage, Hop::Application);
    assert_eq!(app.confidence, Confidence::Claimed);
    assert!(app.reason.contains("destination is context env-a"));
    // The Rollout's spec uses a tag, so the digest isn't pinned there.
    let rollout = one(&trail, Hop::Application, Hop::Rollout);
    assert_eq!(rollout.confidence, Confidence::Claimed);
    assert!(
        rollout
            .reason
            .contains("analysis storefront-5d9c-1=Successful")
    );
    // The pods close the chain on the digest.
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Confirmed);
    assert!(matches!(&pods.key, Key::Digest(d) if d.as_str() == NEW));
    assert_eq!(trail.running(), Some(Confidence::Confirmed));
}

#[tokio::test]
async fn a_rollout_that_pins_the_digest_is_confirmed() {
    let mut world = healthy();
    world.environment = world
        .environment
        .with("rollouts", vec![rollout(&format!("{REPO}@{NEW}"))]);
    let trail = run(&world, &ENV).await;
    assert_eq!(
        one(&trail, Hop::Application, Hop::Rollout).confidence,
        Confidence::Confirmed
    );
}

#[tokio::test]
async fn only_a_label_is_claimed_and_leads_nowhere() {
    let mut world = healthy();
    // The run has the PaC label but names no revision and builds no digest,
    // and no Freight carries the commit.
    world.tekton = world
        .tekton
        .with("pipelineruns", vec![pipeline_run(SHA, false, None)])
        .with("taskruns", vec![]);
    world.kargo = world
        .kargo
        .with("freights", vec![freight("f-old", OLD, OTHER_SHA)]);
    let trail = run(&world, &ENV).await;
    let commit = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(commit.confidence, Confidence::Claimed);
    assert!(commit.reason.contains("only the PaC label"));
    assert_eq!(
        one(&trail, Hop::PipelineRun, Hop::SupplyChain).confidence,
        Confidence::Unknown
    );
    let freight = one(&trail, Hop::PipelineRun, Hop::Freight);
    assert_eq!(freight.confidence, Confidence::Unknown);
    assert!(freight.reason.contains("read 1 Freight"));
    assert_eq!(trail.running(), None);
}

#[tokio::test]
async fn a_freight_can_join_on_the_commit_alone() {
    let mut world = healthy();
    world.tekton = world.tekton.with("pipelineruns", vec![]);
    let trail = run(&world, &ENV).await;
    assert_eq!(
        one(&trail, Hop::Commit, Hop::PipelineRun).confidence,
        Confidence::Unknown
    );
    let freight = one(&trail, Hop::PipelineRun, Hop::Freight);
    assert_eq!(freight.confidence, Confidence::Confirmed);
    assert_eq!(freight.key, Key::Sha(SHA.into()));
}

#[tokio::test]
async fn a_moved_tag_is_not_a_match() {
    let mut world = healthy();
    // The pods run the same tag but the old digest.
    let image_id = format!("docker-pullable://{REPO}@{OLD}");
    world.environment = world.environment.with(
        "pods",
        vec![pod(
            "storefront-5d9c-x",
            &format!("{REPO}:v1.4.0"),
            &image_id,
        )],
    );
    let trail = run(&world, &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Claimed);
    assert!(pods.reason.contains("a moved tag is not a match"));
    assert_eq!(trail.running(), Some(Confidence::Claimed));
}

#[tokio::test]
async fn a_refused_read_is_unknown_and_says_why_never_empty() {
    let mut world = healthy();
    world.environment = world.environment.refusing("pods");
    let trail = run(&world, &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Unknown);
    assert!(
        pods.reason.contains("not readable (refused)"),
        "{}",
        pods.reason
    );

    let mut world = healthy();
    world.kargo = world.kargo.unreachable("freights");
    let trail = run(&world, &ENV).await;
    let freight = one(&trail, Hop::PipelineRun, Hop::Freight);
    assert_eq!(freight.confidence, Confidence::Unknown);
    assert!(
        freight
            .reason
            .contains("not readable: the server could not be reached")
    );
    assert!(
        link(&trail, Hop::Freight, Hop::Stage).is_empty(),
        "the chain stops there"
    );
}

#[tokio::test]
async fn a_cluster_without_the_crds_is_not_installed_not_empty() {
    let mut world = healthy();
    world.tekton = FixtureReader::default();
    let trail = run(&world, &ENV).await;
    let commit = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(commit.confidence, Confidence::Unknown);
    assert!(
        commit.reason.contains("not installed there"),
        "{}",
        commit.reason
    );
}

#[tokio::test]
async fn an_ambiguous_destination_is_reported_not_guessed() {
    let shared = "https://proxy.example:443/";
    let trail = run(
        &{
            let mut world = healthy();
            world.argocd = world
                .argocd
                .with("applications", vec![application(Some(shared))]);
            world
        },
        &[
            ("env-a", "https://PROXY.example"),
            ("env-b", shared),
            ("env-c", "https://env-c.example"),
        ],
    )
    .await;
    let app = one(&trail, Hop::Stage, Hop::Application);
    assert!(
        app.reason
            .contains("several contexts (env-a, env-b); the user must choose"),
        "{}",
        app.reason
    );
}

#[test]
fn destinations_match_one_none_or_by_name() {
    let by_url = parse_application(&application(Some("https://env-a.example:6443"))).unwrap();
    let contexts = vec![("env-a".to_owned(), "https://env-a.example:6443/".to_owned())];
    let to = |contexts| Destinations {
        contexts,
        names: &[],
        argocd: "core",
    };
    assert_eq!(
        match_destination(&by_url, to(&contexts)),
        DestinationMatch::One("env-a".into())
    );
    assert_eq!(match_destination(&by_url, to(&[])), DestinationMatch::None);
    let by_name = parse_application(&application(None)).unwrap();
    assert_eq!(
        match_destination(&by_name, to(&contexts)),
        DestinationMatch::ByName("env-dev".into())
    );
}

#[tokio::test]
async fn every_request_is_a_get_in_a_namespace_or_with_a_selector() {
    let world = healthy();
    run(&world, &ENV).await;
    for reader in [
        &world.kargo,
        &world.argocd,
        &world.tekton,
        &world.environment,
    ] {
        for request in reader.requests.borrow().iter() {
            if request.starts_with("LIST") {
                assert!(
                    !request.contains("ns=None selector=None"),
                    "unscoped list: {request}"
                );
                assert!(!request.contains("secrets"), "{request}");
            } else {
                assert!(request.starts_with("GET /apis"), "{request}");
            }
        }
    }
    // The build lists are selected by the PaC label and the run name, pods by
    // the Rollout's hash.
    let tekton = world.tekton.requests.borrow().join("\n");
    assert!(tekton.contains(&format!("pipelinesascode.tekton.dev/sha={SHA}")));
    assert!(tekton.contains("tekton.dev/pipelineRun=storefront-push-x"));
    let environment = world.environment.requests.borrow().join("\n");
    assert!(environment.contains("rollouts-pod-template-hash=5d9c"));
}

#[tokio::test]
async fn a_selector_cannot_be_injected_through_the_sha() {
    let world = healthy();
    for bad in ["abc,env=prod", "zzzzzzz", "a b", "abc"] {
        let read = read_builds(&world.tekton, "acme-builds", bad).await;
        assert!(matches!(read, Source::Unreadable(_)), "{bad}");
    }
    assert!(world.tekton.requests.borrow().is_empty());
}

#[tokio::test]
async fn an_older_tekton_version_is_used_when_it_is_all_that_is_served() {
    let world = healthy();
    let tekton = FixtureReader::default()
        .serves("tekton.dev", "v1beta1", &["pipelineruns", "taskruns"])
        .with("pipelineruns", vec![pipeline_run(SHA, true, Some(NEW))]);
    let read = read_builds(&tekton, "acme-builds", SHA).await;
    assert_eq!(read.read().map(Vec::len), Some(1));
    assert!(
        tekton
            .requests
            .borrow()
            .iter()
            .any(|r| r.contains("tekton.dev/v1beta1/pipelineruns"))
    );
    drop(world);
}

#[test]
fn freight_and_stage_are_read_in_both_kargo_shapes() {
    let new = parse_freight(&freight("f-new", NEW, SHA)).unwrap();
    assert_eq!(new.alias.as_deref(), Some("wonky-otter"));
    assert_eq!(new.images[0].tag.as_deref(), Some("v1.4.0"));
    assert_eq!(new.images[0].digest.as_ref().map(|d| d.as_str()), Some(NEW));
    assert_eq!(new.verified_in, ["dev"]);
    // Older Kargo: contents under `spec`, one currentFreight.
    let old = parse_freight(&serde_json::json!({
        "metadata": {"name": "f", "namespace": "storefront"},
        "spec": {"origin": {"name": "storefront"}, "images": [{"repoURL": REPO, "digest": OLD}]}
    }))
    .unwrap();
    assert_eq!(old.warehouse.as_deref(), Some("storefront"));
    assert_eq!(old.images.len(), 1);
    let stage = parse_stage(&serde_json::json!({
        "metadata": {"name": "s", "namespace": "storefront"},
        "status": {"currentFreight": {"name": "f"}}
    }))
    .unwrap();
    assert_eq!(stage.current_freight, ["f"]);
    // Nothing but a name is tolerated.
    assert!(
        parse_stage(&serde_json::json!({"metadata": {"name": "s", "namespace": "n"}})).is_some()
    );
    assert!(parse_freight(&serde_json::json!({})).is_none());
}

#[test]
fn build_results_pair_urls_with_digests_and_conforma_is_parsed() {
    let results = [
        ("IMAGE_URL", format!("{REPO}:v1")),
        ("IMAGE_DIGEST", NEW.to_owned()),
        (
            "sidecar_IMAGE_URL",
            "registry.example/acme/sidecar:v1".to_owned(),
        ),
        ("sidecar_IMAGE_DIGEST", "not-a-digest".to_owned()),
        (
            "TEST_OUTPUT",
            "{\"result\":\"FAILURE\",\"failures\":2}".to_owned(),
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    let images = built_images(&results);
    assert_eq!(
        images.len(),
        1,
        "a result without a real digest is not an image"
    );
    assert_eq!(images[0].digest.as_str(), NEW);
    let verdict = conforma(&results).unwrap();
    assert_eq!(
        (verdict.outcome.as_str(), verdict.failures, verdict.warnings),
        ("FAILURE", 2, 0)
    );
}

// ---- GitHub ------------------------------------------------------------

/// A squash merge: the pull request's head commit is not the commit on main.
/// Main's build is what shipped; the head's build produced another digest,
/// which Kargo also holds as older freight.
fn squash_world() -> World {
    let mut world = healthy();
    world.tekton = world.tekton.with(
        "pipelineruns",
        vec![
            pipeline_run(SHA, true, Some(NEW)),
            pr_pipeline_run(OTHER_SHA, 7, OLD),
        ],
    );
    world
}

#[tokio::test]
async fn a_squash_merged_pull_request_joins_through_its_merge_commit() {
    let github = FixtureGitHub::with(vec![pull_request(7, OTHER_SHA, Some(SHA))]);
    let trail = run_with(&squash_world(), &ENV, &github, Some(GH_REPO)).await;
    let commit = one(&trail, Hop::PullRequest, Hop::Commit);
    assert_eq!(commit.confidence, Confidence::Confirmed);
    assert_eq!(commit.subject, "acme/storefront#7");
    assert!(
        commit.reason.contains("its merge commit"),
        "{}",
        commit.reason
    );
    // Two PipelineRuns hang off the pull request: main's and the head's.
    let runs = link(&trail, Hop::PullRequest, Hop::PipelineRun);
    assert_eq!(runs.len(), 2, "{trail:#?}");
    let merged_run = runs.iter().find(|l| l.key == Key::Sha(SHA.into())).unwrap();
    assert_eq!(merged_run.confidence, Confidence::Confirmed);
    let head_run = runs
        .iter()
        .find(|l| l.key == Key::Sha(OTHER_SHA.into()))
        .unwrap();
    assert!(
        head_run.reason.contains("is not the digest that shipped"),
        "{}",
        head_run.reason
    );
    // The head's image is old freight in Kargo: joined on the digest.
    let freight = one(&trail, Hop::PullRequest, Hop::Freight);
    assert_eq!(freight.confidence, Confidence::Confirmed);
    assert_eq!(freight.subject, "storefront/f-old");
    assert!(matches!(&freight.key, Key::Digest(d) if d.as_str() == OLD));
    // The rest of the chain is untouched.
    assert_eq!(trail.running(), Some(Confidence::Confirmed));
}

#[tokio::test]
async fn a_pull_request_whose_head_is_the_commit_reads_no_second_build() {
    let github = FixtureGitHub::with(vec![pull_request(7, SHA, Some(SHA))]);
    let world = healthy();
    let trail = run_with(&world, &ENV, &github, Some(GH_REPO)).await;
    assert_eq!(
        one(&trail, Hop::PullRequest, Hop::Commit).confidence,
        Confidence::Confirmed
    );
    assert_eq!(link(&trail, Hop::PullRequest, Hop::PipelineRun).len(), 1);
    let sha_selectors = world
        .tekton
        .requests
        .borrow()
        .iter()
        .filter(|r| r.contains("pipelinesascode.tekton.dev/sha=") && r.contains("pipelineruns"))
        .count();
    assert_eq!(sha_selectors, 1);
}

#[tokio::test]
async fn a_pull_request_listed_by_ancestry_only_is_a_claim() {
    let github = FixtureGitHub::with(vec![pull_request(
        7,
        OTHER_SHA,
        Some("cccccccccccccccccccccccccccccccccccccccc"),
    )]);
    let trail = run_with(&healthy(), &ENV, &github, Some(GH_REPO)).await;
    let commit = one(&trail, Hop::PullRequest, Hop::Commit);
    assert_eq!(commit.confidence, Confidence::Claimed);
    assert!(commit.reason.contains("only by ancestry"));
    assert!(
        link(&trail, Hop::PullRequest, Hop::PipelineRun)
            .iter()
            .all(|l| l.key != Key::Sha(SHA.into()))
    );
}

#[tokio::test]
async fn a_run_that_names_another_pull_request_is_flagged() {
    let mut world = healthy();
    let mut run = pipeline_run(SHA, true, Some(NEW));
    run["metadata"]["annotations"]["pipelinesascode.tekton.dev/pull-request"] =
        serde_json::json!("9");
    world.tekton = world.tekton.with("pipelineruns", vec![run]);
    let github = FixtureGitHub::with(vec![pull_request(7, SHA, Some(SHA))]);
    let trail = run_with(&world, &ENV, &github, Some(GH_REPO)).await;
    let runs = link(&trail, Hop::PullRequest, Hop::PipelineRun);
    assert!(
        runs[0]
            .reason
            .contains("the run says pull request #9, not #7"),
        "{}",
        runs[0].reason
    );
}

#[tokio::test]
async fn github_that_cannot_be_read_is_unknown_and_the_rest_still_joins() {
    let trail = run_with(&healthy(), &ENV, &FixtureGitHub::refusing(), Some(GH_REPO)).await;
    let commit = one(&trail, Hop::PullRequest, Hop::Commit);
    assert_eq!(commit.confidence, Confidence::Unknown);
    assert!(
        commit.reason.contains("not readable (refused)"),
        "{}",
        commit.reason
    );
    assert_eq!(trail.running(), Some(Confidence::Confirmed));

    // Read, but no pull request: empty is said as empty.
    let none = run_with(
        &healthy(),
        &ENV,
        &FixtureGitHub::with(vec![]),
        Some(GH_REPO),
    )
    .await;
    assert!(
        one(&none, Hop::PullRequest, Hop::Commit)
            .reason
            .contains("no pull request is associated")
    );
}

#[tokio::test]
async fn github_is_not_asked_without_a_repository() {
    let github = FixtureGitHub::with(vec![pull_request(7, SHA, Some(SHA))]);
    let trail = run_with(&healthy(), &ENV, &github, None).await;
    assert!(github.requests.borrow().is_empty());
    assert!(link(&trail, Hop::PullRequest, Hop::Commit).is_empty());
}

#[tokio::test]
async fn a_pull_request_number_leads_to_its_merge_commit_or_its_head() {
    let github = FixtureGitHub::with(vec![
        pull_request(7, OTHER_SHA, Some(SHA)),
        pull_request(8, OTHER_SHA, None),
    ]);
    assert_eq!(
        commit_of_pull_request(&github, GH_REPO, 7).await,
        Source::Read((SHA.to_owned(), true))
    );
    assert_eq!(
        commit_of_pull_request(&github, GH_REPO, 8).await,
        Source::Read((OTHER_SHA.to_owned(), false))
    );
    assert!(matches!(
        commit_of_pull_request(&github, GH_REPO, 99).await,
        Source::NotInstalled(_)
    ));
}

#[tokio::test]
async fn without_rollouts_the_pods_of_the_destination_namespace_are_judged_by_digest() {
    let world = without_rollouts();
    let trail = run_configured(&world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.stage_naming = Some(naming())
    })
    .await;
    let stage = one(&trail, Hop::Stage, Hop::Application);
    // Named by convention is weaker than the annotation, and says so.
    assert_eq!(stage.confidence, Confidence::Claimed);
    assert!(stage.reason.contains("no authorized-stage annotation"));
    let pods = one(&trail, Hop::Application, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Confirmed);
    assert!(matches!(&pods.key, Key::Digest(d) if d.as_str() == NEW));
    assert!(
        pods.reason
            .contains("image summary lists the Freight's digest")
    );
    assert_eq!(trail.running(), Some(Confidence::Confirmed));
    // The only environment request is one LIST of pods in the namespace.
    let requests = world.environment.requests.borrow();
    assert!(requests.iter().all(|r| r.contains("pods")), "{requests:?}");
    assert!(
        requests.iter().any(|r| r.contains("ns=Some(\"shop\")")),
        "{requests:?}"
    );
}

#[tokio::test]
async fn task_runs_that_cannot_be_read_leave_the_build_standing() {
    let mut world = healthy();
    world.tekton = world.tekton.refusing("taskruns");
    let trail = run(&world, &ENV).await;
    let commit = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(commit.confidence, Confidence::Confirmed);
    assert!(
        commit.reason.contains("its TaskRuns were not read"),
        "{}",
        commit.reason
    );
    let supply = one(&trail, Hop::PipelineRun, Hop::SupplyChain);
    assert!(
        supply
            .reason
            .contains("its TaskRuns were not read (forbidden by RBAC)"),
        "{}",
        supply.reason
    );
    assert_eq!(
        one(&trail, Hop::PipelineRun, Hop::Freight).confidence,
        Confidence::Confirmed,
        "the run's own image result still joins"
    );
}

/// Two builds of the same commit, `storefront-push-x` and
/// `storefront-push-y`, each with its own TaskRun.
fn two_builds() -> World {
    let mut second = pipeline_run(SHA, true, Some(NEW));
    second["metadata"]["name"] = serde_json::json!("storefront-push-y");
    second["status"]["childReferences"][0]["name"] = serde_json::json!("storefront-push-y-verify");
    let mut second_task = task_run();
    second_task["metadata"]["name"] = serde_json::json!("storefront-push-y-verify");
    second_task["metadata"]["labels"]["tekton.dev/pipelineRun"] =
        serde_json::json!("storefront-push-y");
    let mut world = healthy();
    world.tekton = world
        .tekton
        .with(
            "pipelineruns",
            vec![pipeline_run(SHA, true, Some(NEW)), second],
        )
        .with("taskruns", vec![task_run(), second_task]);
    world
}

fn of<'a>(links: &[&'a super::join::Link], run: &str) -> &'a super::join::Link {
    let subject = format!("acme-builds/{run}");
    let found: Vec<_> = links.iter().filter(|l| l.subject == subject).collect();
    assert_eq!(found.len(), 1, "{subject} in {links:#?}");
    found[0]
}

#[tokio::test]
async fn one_builds_refused_task_runs_leave_the_other_build_whole() {
    let mut world = two_builds();
    world.tekton = world
        .tekton
        .refusing_selector("taskruns", "tekton.dev/pipelineRun=storefront-push-y");

    let read = read_builds(&world.tekton, "acme-builds", SHA).await;
    let Source::Read(builds) = &read else {
        panic!("the builds were read: {read:?}");
    };
    assert_eq!(builds.len(), 2);
    let read_one = builds
        .iter()
        .find(|b| b.run.name == "storefront-push-x")
        .unwrap();
    assert_eq!(read_one.tasks.len(), 1);
    assert_eq!(read_one.tasks_unread, None);
    let refused = builds
        .iter()
        .find(|b| b.run.name == "storefront-push-y")
        .unwrap();
    assert!(refused.tasks.is_empty());
    assert_eq!(refused.tasks_unread.as_deref(), Some("forbidden by RBAC"));

    let trail = run(&world, &ENV).await;
    let commits = link(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(commits.len(), 2, "{trail:#?}");
    let x = of(&commits, "storefront-push-x");
    assert_eq!(x.confidence, Confidence::Confirmed);
    assert!(!x.reason.contains("not read"), "{}", x.reason);
    let y = of(&commits, "storefront-push-y");
    assert_eq!(
        y.confidence,
        Confidence::Confirmed,
        "the run's own revision still confirms it"
    );
    assert!(
        y.reason
            .contains("its TaskRuns were not read (forbidden by RBAC)"),
        "{}",
        y.reason
    );

    let supply = link(&trail, Hop::PipelineRun, Hop::SupplyChain);
    let x = of(&supply, "storefront-push-x");
    assert!(
        x.reason
            .contains("Conforma SUCCESS (0 failures, 1 warnings)"),
        "the other build's TaskRun result still joins: {}",
        x.reason
    );
    assert!(!x.reason.contains("not read"), "{}", x.reason);
    let y = of(&supply, "storefront-push-y");
    assert!(
        y.reason
            .contains("its TaskRuns were not read (forbidden by RBAC), so their results are unknown; no Conforma result"),
        "{}",
        y.reason
    );
    assert!(!y.reason.contains("Conforma FAILURE"), "{}", y.reason);
}

#[tokio::test]
async fn tekton_is_discovered_once_for_every_build_it_reads() {
    let world = two_builds();
    let read = read_builds(&world.tekton, "acme-builds", SHA).await;
    assert_eq!(read.read().map(Vec::len), Some(2));
    let requests = world.tekton.requests.borrow();
    let discovery: Vec<&str> = requests
        .iter()
        .map(String::as_str)
        .filter(|r| r.starts_with("GET /apis"))
        .collect();
    assert_eq!(discovery, ["GET /apis", "GET /apis/tekton.dev/v1"]);
    let count = |prefix: &str| requests.iter().filter(|r| r.starts_with(prefix)).count();
    assert_eq!(count("LIST tekton.dev/v1/pipelineruns"), 1, "{requests:#?}");
    assert_eq!(count("LIST tekton.dev/v1/taskruns"), 2, "{requests:#?}");
}

#[tokio::test]
async fn task_runs_that_are_not_served_leave_the_build_standing() {
    let tekton = FixtureReader::default()
        .serves("tekton.dev", "v1", &["pipelineruns"])
        .with("pipelineruns", vec![pipeline_run(SHA, true, Some(NEW))])
        .with("taskruns", vec![task_run()]);
    let read = read_builds(&tekton, "acme-builds", SHA).await;
    let builds = read.read().expect("the builds were read");
    assert_eq!(builds.len(), 1);
    assert_eq!(
        builds[0].tasks_unread.as_deref(),
        Some("taskruns.tekton.dev is not served at any version")
    );
    assert!(
        tekton
            .requests
            .borrow()
            .iter()
            .all(|r| !r.contains("taskruns")),
        "nothing is listed at a version that doesn't serve it"
    );
}

#[tokio::test]
async fn a_short_sha_finds_no_build() {
    let world = healthy();
    let trail = run_configured(&world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.sha = SHA[..12].into()
    })
    .await;
    let builds = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(builds.confidence, Confidence::Unknown);
    assert!(
        builds.reason.contains("40 or 64 hexadecimal digits"),
        "{}",
        builds.reason
    );
    assert!(world.tekton.requests.borrow().is_empty());
}

#[tokio::test]
async fn rollouts_are_not_judged_in_a_cluster_that_is_not_the_destination() {
    // The Application deploys to env-a, but the environment cluster read is
    // env-b. It runs a Rollout of the same name and namespace with the
    // Freight's digest, which would have confirmed a deployment that is not
    // this Application's.
    let world = healthy();
    let clusters = Clusters {
        kargo: &world.kargo,
        argocd: &world.argocd,
        tekton: &world.tekton,
        environment: &world.environment,
        github: &FixtureGitHub::default(),
    };
    let mut plan = plan(&[
        ("env-a", "https://env-a.example:6443"),
        ("env-b", "https://env-b.example:6443"),
    ]);
    plan.environment = "env-b".into();
    let trail = join(&collect(&clusters, &plan).await);
    let rollout = one(&trail, Hop::Application, Hop::Rollout);
    assert_eq!(rollout.confidence, Confidence::Unknown);
    assert!(
        rollout
            .reason
            .contains("not known to be the environment cluster")
    );
    assert!(link(&trail, Hop::Rollout, Hop::Pod).is_empty());
    assert_eq!(trail.running(), None);
    assert!(world.environment.requests.borrow().is_empty());

    // A destination that matches no context is not the environment either.
    let trail = run(&healthy(), &[]).await;
    assert_eq!(
        one(&trail, Hop::Application, Hop::Rollout).confidence,
        Confidence::Unknown
    );
}

#[tokio::test]
async fn of_two_applications_with_the_same_rollout_only_the_environments_links_to_it() {
    // Both manage shop/storefront; only env-a is the environment cluster, so
    // only the Application deploying there reaches the Rollout and its pods.
    let mut world = healthy();
    let mut elsewhere = application(Some("https://env-b.example:6443"));
    elsewhere["metadata"]["name"] = serde_json::json!("storefront-dev-b");
    world.argocd = world.argocd.with(
        "applications",
        vec![application(Some("https://env-a.example:6443")), elsewhere],
    );
    let contexts = [
        ("env-a", "https://env-a.example:6443"),
        ("env-b", "https://env-b.example:6443"),
    ];
    let trail = run(&world, &contexts).await;
    assert_eq!(link(&trail, Hop::Stage, Hop::Application).len(), 2);
    let rollouts = link(&trail, Hop::Application, Hop::Rollout);
    assert_eq!(rollouts.len(), 2, "{rollouts:#?}");
    let other = rollouts
        .iter()
        .find(|l| l.subject == "argocd/storefront-dev-b")
        .expect("the other Application's link");
    assert_eq!(other.confidence, Confidence::Unknown);
    assert!(
        other
            .reason
            .contains("not known to be the environment cluster")
    );
    let ours = rollouts
        .iter()
        .find(|l| l.subject == "shop/storefront")
        .expect("the environment Application's link");
    assert_ne!(ours.confidence, Confidence::Unknown);
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.subject, "shop/storefront");
    assert_eq!(pods.confidence, Confidence::Confirmed);
}

#[tokio::test]
async fn a_rollout_without_a_namespace_leaves_the_others_readable() {
    let mut world = healthy();
    let mut app = application(Some("https://env-a.example:6443"));
    app["spec"]["destination"]
        .as_object_mut()
        .unwrap()
        .remove("namespace");
    app["status"]["resources"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"group": "argoproj.io", "kind": "Rollout", "name": "nowhere"}));
    world.argocd = world.argocd.with("applications", vec![app]);
    let trail = run(&world, &ENV).await;
    let rollouts = link(&trail, Hop::Application, Hop::Rollout);
    let nowhere = rollouts
        .iter()
        .find(|l| l.subject.ends_with("nowhere"))
        .expect("the Rollout without a namespace");
    assert_eq!(nowhere.confidence, Confidence::Unknown);
    assert!(
        nowhere.reason.contains("names a namespace"),
        "{}",
        nowhere.reason
    );
    let named = rollouts
        .iter()
        .find(|l| l.subject == "shop/storefront")
        .expect("the Rollout with a namespace");
    assert_ne!(named.confidence, Confidence::Unknown, "{}", named.reason);
    assert_eq!(
        one(&trail, Hop::Rollout, Hop::Pod).confidence,
        Confidence::Confirmed
    );
}

#[tokio::test]
async fn a_promotion_message_is_printed_without_addresses_or_identities() {
    let mut world = healthy();
    let mut promotion = promotion("f-new");
    promotion["status"]["message"] = serde_json::json!(
        r#"push to https://git.example.test/acme/config.git failed: User "someone" denied; see /var/run/x, 192.0.2.7:443"#
    );
    world.kargo = world.kargo.with("promotions", vec![promotion]);
    let trail = run(&world, &ENV).await;
    let reason = &one(&trail, Hop::Freight, Hop::Promotion).reason;
    for word in ["git.example.test", "someone", "/var/run", "192.0.2.7"] {
        assert!(!reason.contains(word), "{reason}");
    }
    assert!(reason.contains("push to <url> failed"), "{reason}");
}

#[tokio::test]
async fn a_sha_256_commit_says_why_no_build_is_found() {
    let world = healthy();
    let trail = run_configured(&world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.sha = "b".repeat(64)
    })
    .await;
    let builds = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(builds.confidence, Confidence::Unknown);
    assert!(builds.reason.contains("SHA-256"), "{}", builds.reason);
    assert!(world.tekton.requests.borrow().is_empty());
}

#[tokio::test]
async fn pods_are_not_read_from_a_cluster_that_is_not_the_destination() {
    let world = without_rollouts();
    let clusters = Clusters {
        kargo: &world.kargo,
        argocd: &world.argocd,
        tekton: &world.tekton,
        environment: &world.environment,
        github: &FixtureGitHub::default(),
    };
    let mut plan = plan(&[("env-a", "https://env-a.example:6443")]);
    plan.stage_naming = Some(naming());
    plan.environment = "env-b".into();
    let trail = join(&collect(&clusters, &plan).await);
    let pods = one(&trail, Hop::Application, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Unknown);
    assert!(world.environment.requests.borrow().is_empty());
}

/// The healthy world, with the Promotion and the Stage recording what Kargo
/// writes, and the Application synced at `revision`.
fn promoted(digest: &str, phase: &str, revision: &str) -> World {
    let mut world = healthy();
    world.kargo = world
        .kargo
        .with("stages", vec![stage_holding("f-new", digest)])
        .with(
            "promotions",
            vec![promotion_pushing("f-new", digest, phase, PUSHED)],
        );
    let mut app = application(Some("https://env-a.example:6443"));
    app["status"]["sync"]["revision"] = serde_json::json!(revision);
    world.argocd = world.argocd.with("applications", vec![app]);
    world
}

const ENV: [(&str, &str); 1] = [("env-a", "https://env-a.example:6443")];

#[tokio::test]
async fn the_pushed_commit_the_application_synced_confirms_promotion_stage_and_application() {
    let trail = run(&promoted(NEW, "Succeeded", PUSHED), &ENV).await;
    let promotion = one(&trail, Hop::Freight, Hop::Promotion);
    assert_eq!(promotion.confidence, Confidence::Confirmed);
    assert!(matches!(&promotion.key, Key::Digest(d) if d.as_str() == NEW));
    let stage = one(&trail, Hop::Freight, Hop::Stage);
    assert_eq!(stage.confidence, Confidence::Confirmed);
    assert!(matches!(&stage.key, Key::Digest(d) if d.as_str() == NEW));
    let app = one(&trail, Hop::Stage, Hop::Application);
    assert_eq!(app.confidence, Confidence::Confirmed);
    assert!(matches!(&app.key, Key::Sha(sha) if sha == PUSHED));
    assert!(
        app.reason
            .contains("synced the commit the Promotion pushed")
    );
}

#[tokio::test]
async fn a_revision_that_is_not_the_pushed_commit_stays_claimed() {
    // The Application is at another commit: it moved on, or never took it.
    let trail = run(&promoted(NEW, "Succeeded", OTHER_SHA), &ENV).await;
    let app = one(&trail, Hop::Stage, Hop::Application);
    assert_eq!(app.confidence, Confidence::Claimed);
    assert!(matches!(&app.key, Key::Name(_)));
    assert!(
        app.reason
            .contains("but the Application's synced revision is")
    );
    // The digest evidence still stands on its own.
    assert_eq!(
        one(&trail, Hop::Freight, Hop::Promotion).confidence,
        Confidence::Confirmed
    );
}

#[tokio::test]
async fn a_promotion_that_did_not_succeed_confirms_nothing() {
    let trail = run(&promoted(NEW, "Failed", PUSHED), &ENV).await;
    let app = one(&trail, Hop::Stage, Hop::Application);
    assert_eq!(app.confidence, Confidence::Claimed);
    assert!(
        app.reason
            .contains("no succeeded Promotion recorded a pushed commit")
    );
}

#[tokio::test]
async fn records_that_carry_another_digest_are_not_a_match() {
    // The Promotion and the Stage hold a different image than the Freight.
    let trail = run(&promoted(OLD, "Succeeded", PUSHED), &ENV).await;
    assert_eq!(
        one(&trail, Hop::Freight, Hop::Promotion).confidence,
        Confidence::Claimed
    );
    assert_eq!(
        one(&trail, Hop::Freight, Hop::Stage).confidence,
        Confidence::Claimed
    );
}

#[tokio::test]
async fn another_stages_application_at_the_same_revision_keeps_it_claimed() {
    let mut world = promoted(NEW, "Succeeded", PUSHED);
    // A later stage's Application of the same project, on the same branch, is at
    // the same commit: the commit cannot tell the two stages apart.
    let mut prod = application(Some("https://env-a.example:6443"));
    prod["metadata"]["name"] = serde_json::json!("storefront-prod");
    prod["metadata"]["annotations"]["kargo.akuity.io/authorized-stage"] =
        serde_json::json!("storefront:prod");
    prod["status"]["sync"]["revision"] = serde_json::json!(PUSHED);
    let mut dev = application(Some("https://env-a.example:6443"));
    dev["status"]["sync"]["revision"] = serde_json::json!(PUSHED);
    world.argocd = world.argocd.with("applications", vec![dev, prod]);
    let trail = run(&world, &ENV).await;
    let app = link(&trail, Hop::Stage, Hop::Application)
        .into_iter()
        .find(|l| l.subject == "argocd/storefront-dev")
        .expect("the dev Application is linked");
    assert_eq!(app.confidence, Confidence::Claimed);
    assert!(
        app.reason
            .contains("another stage's Application is at it too")
    );
}

#[test]
fn a_promotions_steps_give_pushed_and_checked_out_commits_apart() {
    let promotion =
        super::kargo::parse_promotion(&promotion_pushing("f-new", NEW, "Succeeded", PUSHED))
            .unwrap();
    assert_eq!(promotion.pushed_commits, [PUSHED]);
    assert_eq!(promotion.source_commits, [SHA]);
    assert_eq!(promotion.freight_digests.len(), 1);
    // A value that is not a commit id is ignored, never matched.
    let mut value = promotion_pushing("f-new", NEW, "Succeeded", "not-a-commit");
    value["status"]["state"]["step-4"] = serde_json::json!({"commit": "zz"});
    let parsed = super::kargo::parse_promotion(&value).unwrap();
    assert!(parsed.pushed_commits.is_empty());
}

#[tokio::test]
async fn the_naming_fallback_is_off_unless_configured() {
    let world = without_rollouts();
    let trail = run(&world, &ENV).await;
    let app = one(&trail, Hop::Stage, Hop::Application);
    assert_eq!(app.confidence, Confidence::Unknown);
    assert!(app.reason.contains("no Application names this Stage"));
    // Nothing was read in the destination namespace for it either.
    assert!(world.environment.requests.borrow().is_empty());
}

#[tokio::test]
async fn the_project_key_is_read_from_the_applications_labels_too() {
    let mut app = unannotated_application();
    let project = app["metadata"]["annotations"]
        .as_object_mut()
        .unwrap()
        .remove("example.test/project")
        .unwrap();
    app["metadata"]["labels"] = serde_json::json!({"example.test/project": project});
    let world = without_rollouts_with(app);
    let trail = run_configured(&world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.stage_naming = Some(naming())
    })
    .await;
    assert_eq!(
        one(&trail, Hop::Stage, Hop::Application).confidence,
        Confidence::Claimed
    );
}

#[tokio::test]
async fn a_name_that_is_not_the_template_is_not_claimed() {
    let world = without_rollouts();
    let trail = run_configured(&world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.stage_naming = Some(StageNaming {
            name_template: "other-{project}-{stage}".into(),
            ..naming()
        })
    })
    .await;
    assert_eq!(
        one(&trail, Hop::Stage, Hop::Application).confidence,
        Confidence::Unknown
    );
}

fn supply_chain_reason(trail: &Trail) -> String {
    link(trail, Hop::PipelineRun, Hop::SupplyChain)[0]
        .reason
        .clone()
}

#[tokio::test]
async fn an_evidence_record_is_checked_only_when_its_result_is_configured() {
    let mut world = healthy();
    world.tekton = world
        .tekton
        .with("taskruns", vec![evidence_task_run(SHA, NEW)]);
    let configured = |plan: &mut Plan| plan.evidence_result = Some(evidence_result());

    let none = run(&world, &ENV).await;
    assert!(supply_chain_reason(&none).contains("no evidence result configured"));

    let agrees = run_configured(&world, &ENV, &FixtureGitHub::default(), None, configured).await;
    assert!(supply_chain_reason(&agrees).contains("evidence record agrees with"));

    world.tekton = world
        .tekton
        .with("taskruns", vec![evidence_task_run(SHA, OLD)]);
    let differs = run_configured(&world, &ENV, &FixtureGitHub::default(), None, configured).await;
    assert!(supply_chain_reason(&differs).contains("evidence record disagrees with"));

    // A name the build does not carry is said, not guessed.
    let absent = run_configured(
        &healthy(),
        &ENV,
        &FixtureGitHub::default(),
        None,
        configured,
    )
    .await;
    assert!(
        supply_chain_reason(&absent).contains("configured evidence result is not on the build")
    );
}

#[tokio::test]
async fn a_capped_listing_never_says_none() {
    // The only Freight read doesn't match, but the listing stopped at the cap.
    let mut world = healthy();
    world.kargo = world
        .kargo
        .with("freights", vec![freight("f-old", OLD, OTHER_SHA)])
        .capped("freights");
    let trail = run(&world, &ENV).await;
    let freight = one(&trail, Hop::PipelineRun, Hop::Freight);
    assert_eq!(freight.confidence, Confidence::Unknown);
    assert!(
        freight
            .reason
            .contains("stopped at the page cap after 1 items"),
        "{}",
        freight.reason
    );

    // Pods that run another digest, from a capped listing: the right pod may
    // be among those not read.
    let mut world = healthy();
    let image_id = format!("docker-pullable://{REPO}@{OLD}");
    world.environment = world
        .environment
        .with(
            "pods",
            vec![pod(
                "storefront-5d9c-x",
                &format!("{REPO}:v1.4.0"),
                &image_id,
            )],
        )
        .capped("pods");
    let trail = run(&world, &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Claimed);
    assert!(pods.reason.contains("page cap"), "{}", pods.reason);

    // Uncapped, the same answers say none without the note.
    let trail = run(&healthy(), &ENV).await;
    assert!(trail.links.iter().all(|l| !l.reason.contains("page cap")));
}

#[tokio::test]
async fn a_capped_application_listing_cannot_confirm_by_commit() {
    // The pushed commit would confirm the Application, but another stage's
    // Application at the same commit may be among those not read.
    let mut world = promoted(NEW, "Succeeded", PUSHED);
    world.argocd = world.argocd.capped("applications");
    let trail = run(&world, &ENV).await;
    let app = one(&trail, Hop::Stage, Hop::Application);
    assert_eq!(app.confidence, Confidence::Claimed);
    assert!(app.reason.contains("page cap"), "{}", app.reason);
}

#[tokio::test]
async fn only_upstream_commit_names_are_read_unless_more_are_configured() {
    // The run names its revision under a parameter upstream doesn't define.
    let mut world = healthy();
    let mut run_value = pipeline_run(SHA, false, Some(NEW));
    run_value["spec"]["params"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"name": "made-up-param", "value": SHA}));
    world.tekton = world.tekton.with("pipelineruns", vec![run_value]);

    let plain = run(&world, &ENV).await;
    assert_eq!(
        one(&plain, Hop::Commit, Hop::PipelineRun).confidence,
        Confidence::Claimed
    );
    let configured = run_configured(&world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.commit_names.params = vec!["made-up-param".into()]
    })
    .await;
    assert_eq!(
        one(&configured, Hop::Commit, Hop::PipelineRun).confidence,
        Confidence::Confirmed
    );

    // Results: upstream's `commit` counts, an unknown name doesn't.
    let names = super::tekton::CommitNames::default();
    let results = |key: &str| {
        [(key.to_owned(), SHA.to_owned())]
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    assert_eq!(
        super::tekton::result_commit(&results("commit"), &names).as_deref(),
        Some(SHA)
    );
    assert_eq!(
        super::tekton::result_commit(&results("CHAINS-GIT_COMMIT"), &names).as_deref(),
        Some(SHA)
    );
    assert_eq!(
        super::tekton::result_commit(&results("made-up-result"), &names),
        None
    );
}

// ---- cluster names, ReplicaSets and the output gaps of #330 -------------

/// The healthy world with its Application's destination given as the
/// cluster name `name` instead of a server.
fn destined_to(name: &str) -> World {
    let mut world = healthy();
    let mut app = application(None);
    app["spec"]["destination"]["name"] = serde_json::json!(name);
    world.argocd = world.argocd.with("applications", vec![app]);
    world
}

#[test]
fn argo_cds_own_names_match_its_context_and_other_names_only_when_mapped() {
    let contexts = vec![
        ("core".to_owned(), "https://core.example:6443".to_owned()),
        ("env-a".to_owned(), "https://env-a.example:6443".to_owned()),
    ];
    let names = vec![("env-a".to_owned(), "workloads".to_owned())];
    let to = Destinations {
        contexts: &contexts,
        names: &names,
        argocd: "core",
    };
    let by = |destination: serde_json::Value| {
        let mut app = application(None);
        app["spec"]["destination"] = destination;
        match_destination(&parse_application(&app).unwrap(), to)
    };
    use serde_json::json;
    assert_eq!(
        by(json!({"name": "in-cluster"})),
        DestinationMatch::ArgoCd("core".into())
    );
    assert_eq!(
        by(json!({"server": "https://kubernetes.default.svc:443/"})),
        DestinationMatch::ArgoCd("core".into())
    );
    assert_eq!(
        by(json!({"name": "workloads"})),
        DestinationMatch::Named {
            name: "workloads".into(),
            context: "env-a".into()
        }
    );
    // No default for any other name, and a name is never matched as a server.
    assert_eq!(
        by(json!({"name": "env-a"})),
        DestinationMatch::ByName("env-a".into())
    );
    assert_eq!(
        by(json!({"namespace": "shop"})),
        DestinationMatch::Unspecified
    );

    // A name mapped to two contexts, or in-cluster mapped elsewhere, is not
    // guessed between.
    let twice = vec![
        ("core".to_owned(), "workloads".to_owned()),
        ("env-a".to_owned(), "workloads".to_owned()),
        ("env-a".to_owned(), "in-cluster".to_owned()),
    ];
    let to = Destinations {
        names: &twice,
        ..to
    };
    let app = |name: &str| {
        let mut app = application(None);
        app["spec"]["destination"]["name"] = json!(name);
        parse_application(&app).unwrap()
    };
    for name in ["workloads", "in-cluster"] {
        assert_eq!(
            match_destination(&app(name), to),
            DestinationMatch::Ambiguous(vec!["core".into(), "env-a".into()]),
            "{name}"
        );
    }
    // Mapped by the user to Argo CD's context too, in-cluster is still one.
    let same = vec![("core".to_owned(), "in-cluster".to_owned())];
    assert_eq!(
        match_destination(&app("in-cluster"), Destinations { names: &same, ..to }),
        DestinationMatch::ArgoCd("core".into())
    );
}

#[tokio::test]
async fn in_cluster_reaches_the_pods_when_argo_cd_runs_in_the_environment() {
    let world = destined_to("in-cluster");
    let trail = run_configured(&world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.argocd = "env-a".into();
    })
    .await;
    let app = one(&trail, Hop::Stage, Hop::Application);
    assert!(
        app.reason
            .contains("destination is Argo CD's own cluster (in-cluster), context env-a"),
        "{}",
        app.reason
    );
    assert_ne!(
        one(&trail, Hop::Application, Hop::Rollout).confidence,
        Confidence::Unknown
    );
    assert_eq!(
        one(&trail, Hop::Rollout, Hop::Pod).confidence,
        Confidence::Confirmed
    );
    assert_eq!(trail.running(), Some(Confidence::Confirmed));
}

#[tokio::test]
async fn in_cluster_is_not_the_environment_when_argo_cd_runs_elsewhere() {
    let world = destined_to("in-cluster");
    let trail = run(&world, &ENV).await;
    let rollout = one(&trail, Hop::Application, Hop::Rollout);
    assert_eq!(rollout.confidence, Confidence::Unknown);
    assert!(
        rollout
            .reason
            .contains("it is context core, and the environment is env-a"),
        "{}",
        rollout.reason
    );
    assert!(world.environment.requests.borrow().is_empty());
}

#[tokio::test]
async fn an_unmapped_cluster_name_says_how_to_map_it() {
    let world = destined_to("workloads");
    let trail = run(&world, &ENV).await;
    let rollout = one(&trail, Hop::Application, Hop::Rollout);
    assert_eq!(rollout.confidence, Confidence::Unknown);
    assert!(
        rollout.reason.contains(
            "map the name the Application gives with --known-as-name env-a=<cluster name>"
        ),
        "{}",
        rollout.reason
    );
    assert!(
        one(&trail, Hop::Stage, Hop::Application)
            .reason
            .contains("destination by a cluster name no context is mapped to")
    );
    // A cluster name may carry a cloud account or project; it is never printed.
    for link in &trail.links {
        assert!(!link.reason.contains("workloads"), "{}", link.reason);
    }
    assert!(world.environment.requests.borrow().is_empty());
    assert!(
        trail.summary().starts_with(
            "not found running, stopped at Application -> Rollout argocd/storefront-dev"
        ),
        "{}",
        trail.summary()
    );
}

#[tokio::test]
async fn a_mapped_cluster_name_reaches_the_pods() {
    let world = destined_to("workloads");
    let trail = run_configured(&world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.cluster_names = vec![("env-a".into(), "workloads".into())];
    })
    .await;
    assert!(
        one(&trail, Hop::Stage, Hop::Application)
            .reason
            .contains("destination by a cluster name, mapped to context env-a")
    );
    for link in &trail.links {
        assert!(!link.reason.contains("workloads"), "{}", link.reason);
    }
    assert_eq!(
        one(&trail, Hop::Rollout, Hop::Pod).confidence,
        Confidence::Confirmed
    );
}

#[tokio::test]
async fn only_the_replica_set_at_the_promoted_digest_is_judged() {
    // The Rollout's status reports the running ReplicaSet as current, or
    // still the stale one (a status that lags its spec): the ReplicaSet whose
    // template pins the Freight's digest is judged either way.
    for current in ["5d9c", "7f3b"] {
        let world = beside_a_stale_replica_set(current);
        let trail = run(&world, &ENV).await;
        let pods = one(&trail, Hop::Rollout, Hop::Pod);
        assert_eq!(pods.confidence, Confidence::Confirmed, "{current}");
        assert!(matches!(&pods.key, Key::Digest(d) if d.as_str() == NEW));
        assert!(
            pods.reason.contains(
                "2 container(s) run the Freight's digest, 2 ready of 2 pod container(s) read"
            ),
            "{current}: {}",
            pods.reason
        );
        assert!(!pods.reason.contains("other container"), "{}", pods.reason);
        assert!(
            pods.reason.contains(
                "the pods of ReplicaSet storefront-5d9c, whose template pins the Freight's digest"
            ),
            "{}",
            pods.reason
        );
        assert!(
            pods.reason.contains(
                "not judged: the pods of ReplicaSet storefront-7f3b, whose template does not pin it"
            ),
            "{}",
            pods.reason
        );
        let environment = world.environment.requests.borrow().join("\n");
        assert!(environment.contains("rollouts-pod-template-hash=5d9c"));
        assert!(!environment.contains("rollouts-pod-template-hash=7f3b"));
        assert!(environment.contains(
            "LIST apps/v1/replicasets ns=Some(\"shop\") selector=Some(\"rollouts-pod-template-hash\")"
        ));
    }
}

#[tokio::test]
async fn without_a_replica_set_at_the_digest_the_current_pod_hash_is_read() {
    // The healthy Rollout uses a tag, so no template pins the digest.
    let trail = run(&healthy(), &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Confirmed);
    assert!(
        pods.reason.contains(
            "the pods of the Rollout's current pod hash 5d9c; no ReplicaSet of it pins the Freight's digest"
        ),
        "{}",
        pods.reason
    );

    // ReplicaSets that can't be read leave the pods readable, and say so.
    let mut world = beside_a_stale_replica_set("5d9c");
    world.environment = world.environment.refusing("replicasets");
    let trail = run(&world, &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Confirmed);
    assert!(
        pods.reason.contains("its ReplicaSets were not read"),
        "{}",
        pods.reason
    );
}

fn promotion_by(actor: Option<&str>) -> World {
    let mut world = promoted(NEW, "Succeeded", PUSHED);
    let mut promotion = promotion_pushing("f-new", NEW, "Succeeded", PUSHED);
    if let Some(actor) = actor {
        promotion["metadata"]["annotations"] =
            serde_json::json!({"kargo.akuity.io/create-actor": actor});
    }
    world.kargo = world.kargo.with("promotions", vec![promotion]);
    world
}

#[tokio::test]
async fn a_promotion_says_auto_or_manual_and_the_commit_it_pushed() {
    for (actor, words) in [
        (None, "likely auto-promoted"),
        (
            Some("controller:kargo-controller"),
            "created by a Kargo controller",
        ),
        (Some("email:someone@example.test"), "promoted by hand"),
        (Some("kubernetes:someone"), "promoted by hand"),
        (Some("admin"), "promoted by hand"),
        (
            Some("kubernetes:system:serviceaccount:delivery:someone"),
            "created by a service account, an API client",
        ),
        (Some("unknown actor"), "its creator is not known"),
    ] {
        let trail = run(&promotion_by(actor), &ENV).await;
        let reason = &one(&trail, Hop::Freight, Hop::Promotion).reason;
        assert!(reason.contains(words), "{actor:?}: {reason}");
        assert!(reason.contains("it pushed dddddddddddd"), "{reason}");
        // The actor is an identity and is never printed.
        assert!(!reason.contains("someone"), "{reason}");
    }
    let trail = run(&healthy(), &ENV).await;
    assert!(
        one(&trail, Hop::Freight, Hop::Promotion)
            .reason
            .contains("it recorded no pushed commit")
    );
}

#[tokio::test]
async fn an_unhealthy_stage_says_why() {
    let mut world = healthy();
    let mut stage = stage(&["f-new"]);
    stage["status"]["health"] = serde_json::json!({
        "status": "Unhealthy",
        "issues": [
            "Argo CD Application \"argocd/storefront-dev\" is synced to eeeeeeeeeeee, not the desired revision dddddddddddd",
            "fetching https://git.example.test/acme/config.git failed"
        ]
    });
    world.kargo = world.kargo.with("stages", vec![stage]);
    let trail = run(&world, &ENV).await;
    let reason = &one(&trail, Hop::Freight, Hop::Stage).reason;
    assert!(
        reason.contains("health Unhealthy (Argo CD Application \"argocd/storefront-dev\" is synced to eeeeeeeeeeee, not the desired revision dddddddddddd; fetching <url> failed)"),
        "{reason}"
    );
    assert!(!reason.contains("git.example.test"), "{reason}");
}

#[tokio::test]
async fn the_freight_says_whether_its_image_revision_is_the_commit() {
    let sha_256 = "a".repeat(64);
    for (revision, words) in [
        (SHA, "names aaaaaaaaaaaa, this change's commit"),
        (OTHER_SHA, "names bbbbbbbbbbbb, not this change's commit"),
        // An abbreviation is compared as the start of the full id, case aside.
        ("AAAAAAA", "names aaaaaaa, this change's commit"),
        ("aaaaaaab", "names aaaaaaab, not this change's commit"),
        // A longer id than the commit's is another hash algorithm's.
        (
            sha_256.as_str(),
            "names aaaaaaaaaaaa, which can't be compared with this change's commit",
        ),
    ] {
        let mut world = healthy();
        let mut item = freight("f-new", NEW, SHA);
        item["images"][0]["annotations"] =
            serde_json::json!({"org.opencontainers.image.revision": revision});
        world.kargo = world.kargo.with("freights", vec![item]);
        let trail = run(&world, &ENV).await;
        let reason = &one(&trail, Hop::PipelineRun, Hop::Freight).reason;
        assert!(reason.contains(words), "{reason}");
    }
    let trail = run(&healthy(), &ENV).await;
    assert!(
        !one(&trail, Hop::PipelineRun, Hop::Freight)
            .reason
            .contains("OCI revision")
    );
}

#[tokio::test]
async fn without_an_evidence_result_the_hint_names_the_runs_results() {
    let trail = run(&healthy(), &ENV).await;
    assert!(
        supply_chain_reason(&trail)
            .contains("no evidence result configured (the run's results: IMAGE_DIGEST, IMAGE_URL)"),
        "{}",
        supply_chain_reason(&trail)
    );
}

#[tokio::test]
async fn the_summary_says_whether_the_change_runs() {
    let trail = run(&healthy(), &ENV).await;
    assert!(
        trail
            .summary()
            .starts_with("running, confirmed: pods run the digest; "),
        "{}",
        trail.summary()
    );
    let rendered = super::join::render(&trail);
    assert!(
        rendered
            .lines()
            .last()
            .is_some_and(|line| line.starts_with("summary: running, confirmed")),
        "{rendered}"
    );
    let counts = |c| trail.links.iter().filter(|l| l.confidence == c).count();
    assert!(trail.summary().ends_with(&format!(
        "{} confirmed, {} claimed, {} unknown",
        counts(Confidence::Confirmed),
        counts(Confidence::Claimed),
        counts(Confidence::Unknown)
    )));

    // A moved tag: the pods run another digest, a claim at most.
    let mut world = healthy();
    world.environment = world.environment.with(
        "pods",
        vec![pod(
            "storefront-5d9c-x",
            &format!("{REPO}:v1.4.0"),
            &format!("docker-pullable://{REPO}@{OLD}"),
        )],
    );
    let trail = run(&world, &ENV).await;
    assert!(
        trail.summary().starts_with("running only as claimed"),
        "{}",
        trail.summary()
    );
}

#[tokio::test]
async fn a_replica_set_of_another_rollout_with_the_same_name_is_not_its_own() {
    // The pinned ReplicaSet's owner is a Rollout of the same name and kind
    // that was deleted and made again: another UID, so not this Rollout's.
    let mut world = beside_a_stale_replica_set("7f3b");
    let mut orphan = replica_set("5d9c", &format!("{REPO}@{NEW}"), 2, 2);
    orphan["metadata"]["ownerReferences"][0]["uid"] =
        serde_json::json!("0f0e0d0c-0000-4000-8000-000000000002");
    let mut other_group = replica_set("6e1a", &format!("{REPO}@{NEW}"), 1, 1);
    other_group["metadata"]["ownerReferences"][0]["apiVersion"] =
        serde_json::json!("example.test/v1");
    world.environment = world.environment.with(
        "replicasets",
        vec![
            replica_set("7f3b", &format!("{REPO}@{OLD}"), 1, 0),
            orphan,
            other_group,
        ],
    );
    let trail = run(&world, &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert!(
        pods.reason
            .contains("the pods of the Rollout's current pod hash 7f3b"),
        "{}",
        pods.reason
    );
    assert!(!pods.reason.contains("storefront-5d9c"), "{}", pods.reason);
    assert!(!pods.reason.contains("storefront-6e1a"), "{}", pods.reason);
}

#[tokio::test]
async fn a_capped_replica_set_listing_says_so_when_one_pins_the_digest() {
    let mut world = beside_a_stale_replica_set("5d9c");
    world.environment = world.environment.capped("replicasets");
    let trail = run(&world, &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert!(
        pods.reason.contains(
            "whose template pins the Freight's digest; the listing stopped at the page cap"
        ),
        "{}",
        pods.reason
    );
}
