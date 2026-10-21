use super::*;

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

/// A setup that names no project key: the name template alone.
fn name_only() -> StageNaming {
    StageNaming {
        project_key: None,
        ..naming()
    }
}

/// [`unannotated_application`] without the project it names, as the setup of
/// the name-only tests has it.
fn application_without_project() -> serde_json::Value {
    let mut app = unannotated_application();
    app["metadata"]["annotations"] = serde_json::json!({});
    app
}

async fn stage_application_link(world: &World, naming: Option<StageNaming>) -> super::join::Link {
    let trail = run_configured(world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.stage_naming = naming
    })
    .await;
    one(&trail, Hop::Stage, Hop::Application).clone()
}

#[tokio::test]
async fn a_template_alone_claims_the_stage_by_name_only() {
    let world = without_rollouts_with(application_without_project());
    let link = stage_application_link(&world, Some(name_only())).await;
    // Claimed, never Confirmed, and in words that tell it from a project key's.
    assert_eq!(link.confidence, Confidence::Claimed, "{link:#?}");
    assert!(
        link.reason
            .contains("matched by the configured name template only"),
        "{}",
        link.reason
    );
    assert!(!link.reason.contains("under the configured key"));
}

#[tokio::test]
async fn the_template_must_match_the_whole_name() {
    let mut app = application_without_project();
    app["metadata"]["name"] = serde_json::json!("app-storefront-in-dev-extra");
    let link = stage_application_link(&without_rollouts_with(app), Some(name_only())).await;
    assert_eq!(link.confidence, Confidence::Unknown);
    assert!(link.reason.contains("no Application names this Stage"));
}

#[tokio::test]
async fn the_authorized_stage_annotation_still_wins_over_the_name() {
    let mut app = application_without_project();
    app["metadata"]["annotations"] =
        serde_json::json!({"kargo.akuity.io/authorized-stage": "storefront:dev"});
    app["metadata"]["name"] = serde_json::json!("unrelated");
    let link = stage_application_link(&without_rollouts_with(app), Some(name_only())).await;
    assert!(
        link.reason.starts_with("authorized-stage annotation"),
        "{}",
        link.reason
    );
}

#[tokio::test]
async fn an_annotation_for_another_stage_blocks_the_name() {
    let mut app = application_without_project();
    app["metadata"]["annotations"] =
        serde_json::json!({"kargo.akuity.io/authorized-stage": "storefront:prod"});
    let link = stage_application_link(&without_rollouts_with(app), Some(name_only())).await;
    assert_eq!(link.confidence, Confidence::Unknown);
}

#[tokio::test]
async fn a_set_project_key_is_still_required() {
    let world = without_rollouts_with(application_without_project());
    let link = stage_application_link(&world, Some(naming())).await;
    assert_eq!(link.confidence, Confidence::Unknown);
}

#[tokio::test]
async fn a_name_only_sibling_at_the_same_commit_keeps_the_claim_a_claim() {
    let mut world = promoted(NEW, "Succeeded", PUSHED);
    // The setup names no project key, and a later stage's Application is
    // named for its own stage, at the commit the promotion pushed.
    let mut dev = application_without_project();
    dev["status"]["sync"]["revision"] = serde_json::json!(PUSHED);
    let mut prod = application_without_project();
    prod["metadata"]["name"] = serde_json::json!("app-storefront-in-prod");
    prod["status"]["sync"]["revision"] = serde_json::json!(PUSHED);
    world.argocd = world.argocd.with("applications", vec![dev, prod]);
    let mut prod_stage = stage(&[]);
    prod_stage["metadata"]["name"] = serde_json::json!("prod");
    world.kargo = world
        .kargo
        .with("stages", vec![stage_holding("f-new", NEW), prod_stage]);
    let link = stage_application_link(&world, Some(name_only())).await;
    assert_eq!(link.confidence, Confidence::Claimed, "{link:#?}");
    assert!(
        link.reason
            .contains("another stage's Application is at it too"),
        "{}",
        link.reason
    );
}

#[tokio::test]
async fn a_name_only_claim_stays_claimed_when_the_push_and_the_sync_match() {
    let mut world = promoted(NEW, "Succeeded", PUSHED);
    let mut app = application_without_project();
    app["status"]["sync"]["revision"] = serde_json::json!(PUSHED);
    world.argocd = world.argocd.with("applications", vec![app]);
    let link = stage_application_link(&world, Some(name_only())).await;
    assert_eq!(link.confidence, Confidence::Claimed, "{link:#?}");
    assert!(
        link.reason.contains("found by the naming template only"),
        "{}",
        link.reason
    );
    // The same proof under the project key is Confirmed, so the cap is the
    // claim's, not the proof's.
    let mut keyed = unannotated_application();
    keyed["status"]["sync"]["revision"] = serde_json::json!(PUSHED);
    world.argocd = world.argocd.with("applications", vec![keyed]);
    let link = stage_application_link(&world, Some(naming())).await;
    assert_eq!(link.confidence, Confidence::Confirmed, "{link:#?}");
}

#[tokio::test]
async fn two_applications_named_for_one_stage_are_linked_to_neither() {
    let world = without_rollouts_with(application_without_project());
    let clusters = Clusters {
        kargo: &world.kargo,
        argocd: &world.argocd,
        tekton: &world.tekton,
        environment: &world.environment,
        github: &FixtureGitHub::default(),
    };
    let mut plan = plan(&ENV);
    plan.stage_naming = Some(name_only());
    let mut evidence = collect(&clusters, &plan, observed_at()).await;
    // The same name in another namespace Argo CD reads Applications from.
    let first = evidence.applications.read().unwrap()[0].clone();
    let mut twin = first.clone();
    twin.namespace = "argocd-b".into();
    let mut annotated = application_without_project();
    annotated["metadata"]["name"] = serde_json::json!("storefront-dev");
    annotated["metadata"]["annotations"] =
        serde_json::json!({"kargo.akuity.io/authorized-stage": "storefront:dev"});
    let annotated = parse_application(&annotated).unwrap();
    evidence.applications = Source::Read(vec![first, twin, annotated]);
    let trail = join(&evidence);
    let links = link(&trail, Hop::Stage, Hop::Application);
    let of = |subject: &str| links.iter().find(|l| l.subject == subject).expect(subject);
    for subject in [
        "argocd/app-storefront-in-dev",
        "argocd-b/app-storefront-in-dev",
    ] {
        let link = of(subject);
        assert_eq!(link.confidence, Confidence::Unknown, "{link:#?}");
        assert!(
            link.reason
                .contains("2 Applications match the naming template for this Stage"),
            "{}",
            link.reason
        );
    }
    // An annotation's claim is not part of the ambiguity.
    assert_ne!(of("argocd/storefront-dev").confidence, Confidence::Unknown);
}
