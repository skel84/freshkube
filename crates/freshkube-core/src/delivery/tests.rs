use super::argocd::{DestinationMatch, StageNaming, match_destination, parse_application};
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
        environment: None,
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
        annotation_key: "acme.example/kargo-project".into(),
        name_template: "{project}-{stage}".into(),
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
    let trail = run(&world, &[]).await;
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
    let trail = run(&world, &[]).await;
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
    let trail = run(&world, &[]).await;
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
    let trail = run(&world, &[]).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Claimed);
    assert!(pods.reason.contains("a moved tag is not a match"));
    assert_eq!(trail.running(), Some(Confidence::Claimed));
}

#[tokio::test]
async fn a_refused_read_is_unknown_and_says_why_never_empty() {
    let mut world = healthy();
    world.environment = world.environment.refusing("pods");
    let trail = run(&world, &[]).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Unknown);
    assert!(
        pods.reason.contains("not readable (refused)"),
        "{}",
        pods.reason
    );

    let mut world = healthy();
    world.kargo = world.kargo.unreachable("freights");
    let trail = run(&world, &[]).await;
    let freight = one(&trail, Hop::PipelineRun, Hop::Freight);
    assert_eq!(freight.confidence, Confidence::Unknown);
    assert!(freight.reason.contains("not readable: connection refused"));
    assert!(
        link(&trail, Hop::Freight, Hop::Stage).is_empty(),
        "the chain stops there"
    );
}

#[tokio::test]
async fn a_cluster_without_the_crds_is_not_installed_not_empty() {
    let mut world = healthy();
    world.tekton = FixtureReader::default();
    let trail = run(&world, &[]).await;
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
    assert_eq!(
        match_destination(&by_url, &contexts),
        DestinationMatch::One("env-a".into())
    );
    assert_eq!(match_destination(&by_url, &[]), DestinationMatch::None);
    let by_name = parse_application(&application(None)).unwrap();
    assert_eq!(
        match_destination(&by_name, &contexts),
        DestinationMatch::ByName("env-beta".into())
    );
}

#[tokio::test]
async fn every_request_is_a_get_in_a_namespace_or_with_a_selector() {
    let world = healthy();
    run(&world, &[]).await;
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
    assert_eq!(new.verified_in, ["beta"]);
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
    let trail = run_with(&squash_world(), &[], &github, Some(GH_REPO)).await;
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
    let trail = run_with(&world, &[], &github, Some(GH_REPO)).await;
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
    let trail = run_with(&healthy(), &[], &github, Some(GH_REPO)).await;
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
    let trail = run_with(&world, &[], &github, Some(GH_REPO)).await;
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
    let trail = run_with(&healthy(), &[], &FixtureGitHub::refusing(), Some(GH_REPO)).await;
    let commit = one(&trail, Hop::PullRequest, Hop::Commit);
    assert_eq!(commit.confidence, Confidence::Unknown);
    assert!(
        commit.reason.contains("not readable (refused)"),
        "{}",
        commit.reason
    );
    assert_eq!(trail.running(), Some(Confidence::Confirmed));

    // Read, but no pull request: empty is said as empty.
    let none = run_with(&healthy(), &[], &FixtureGitHub::with(vec![]), Some(GH_REPO)).await;
    assert!(
        one(&none, Hop::PullRequest, Hop::Commit)
            .reason
            .contains("no pull request is associated")
    );
}

#[tokio::test]
async fn github_is_not_asked_without_a_repository() {
    let github = FixtureGitHub::with(vec![pull_request(7, SHA, Some(SHA))]);
    let trail = run_with(&healthy(), &[], &github, None).await;
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
    plan.environment = Some("env-b".into());
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
    let mut gamma = application(Some("https://env-a.example:6443"));
    gamma["metadata"]["name"] = serde_json::json!("storefront-gamma");
    gamma["metadata"]["annotations"]["kargo.akuity.io/authorized-stage"] =
        serde_json::json!("storefront:gamma");
    gamma["status"]["sync"]["revision"] = serde_json::json!(PUSHED);
    let mut beta = application(Some("https://env-a.example:6443"));
    beta["status"]["sync"]["revision"] = serde_json::json!(PUSHED);
    world.argocd = world.argocd.with("applications", vec![beta, gamma]);
    let trail = run(&world, &ENV).await;
    let app = link(&trail, Hop::Stage, Hop::Application)
        .into_iter()
        .find(|l| l.subject == "argocd/storefront-beta")
        .expect("the beta Application is linked");
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
async fn a_name_that_is_not_the_template_is_not_claimed() {
    let world = without_rollouts();
    let trail = run_configured(&world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.stage_naming = Some(StageNaming {
            name_template: "{stage}-{project}".into(),
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

    let none = run(&world, &[]).await;
    assert!(supply_chain_reason(&none).contains("no evidence result configured"));

    let agrees = run_configured(&world, &[], &FixtureGitHub::default(), None, configured).await;
    assert!(supply_chain_reason(&agrees).contains("evidence record agrees with"));

    world.tekton = world
        .tekton
        .with("taskruns", vec![evidence_task_run(SHA, OLD)]);
    let differs = run_configured(&world, &[], &FixtureGitHub::default(), None, configured).await;
    assert!(supply_chain_reason(&differs).contains("evidence record disagrees with"));

    // A name the build does not carry is said, not guessed.
    let absent = run_configured(&healthy(), &[], &FixtureGitHub::default(), None, configured).await;
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
    let trail = run(&world, &[]).await;
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
    let trail = run(&world, &[]).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Claimed);
    assert!(pods.reason.contains("page cap"), "{}", pods.reason);

    // Uncapped, the same answers say none without the note.
    let trail = run(&healthy(), &[]).await;
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
        .push(serde_json::json!({"name": "source-sha", "value": SHA}));
    world.tekton = world.tekton.with("pipelineruns", vec![run_value]);

    let plain = run(&world, &[]).await;
    assert_eq!(
        one(&plain, Hop::Commit, Hop::PipelineRun).confidence,
        Confidence::Claimed
    );
    let configured = run_configured(&world, &[], &FixtureGitHub::default(), None, |plan| {
        plan.commit_names.params = vec!["source-sha".into()]
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
        super::tekton::result_commit(&results("acme-built-from"), &names),
        None
    );
}
