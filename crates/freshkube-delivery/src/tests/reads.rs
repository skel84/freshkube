use super::*;

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
        let read = read_builds(&world.tekton, Some("acme-builds"), bad).await;
        assert!(matches!(read, Source::Unreadable(_)), "{bad}");
    }
    assert!(world.tekton.requests.borrow().is_empty());
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
