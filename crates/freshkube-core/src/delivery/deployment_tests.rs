//! The Deployment controller chain: Application -> Deployment -> pods, with
//! one rule per outcome. Invented names throughout.

use serde_json::{Value, json};

use super::fixtures::*;
use super::join::{Confidence, Hop, Key, Link, Trail, render};
use super::tests::{ENV, naming, one, run_configured};

fn pinned_image() -> String {
    format!("{REPO}@{NEW}")
}

fn image_id(image: &str) -> String {
    format!("docker-pullable://{image}")
}

/// The Deployment chain at rest: the Deployment pins the digest, its current
/// revision is ReplicaSet `6fdf`, whose one pod runs it.
fn parts() -> (Value, Vec<Value>, Vec<Value>) {
    let image = pinned_image();
    (
        deployment(&image),
        vec![deployment_set("6fdf", "2", &image, 1, 1)],
        vec![deployment_pod(
            "storefront-6fdf-x",
            "6fdf",
            &image,
            &image_id(&image),
            true,
        )],
    )
}

fn world((deployment, sets, pods): (Value, Vec<Value>, Vec<Value>)) -> World {
    let mut world = without_rollouts();
    world.environment = FixtureReader::default()
        .with("deployments", vec![deployment])
        .with("replicasets", sets)
        .with("pods", pods);
    world
}

async fn trail(world: &World) -> Trail {
    run_configured(world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.stage_naming = Some(naming())
    })
    .await
}

async fn chain(world: &World) -> (Link, Link) {
    let trail = trail(world).await;
    (
        one(&trail, Hop::Application, Hop::Deployment).clone(),
        one(&trail, Hop::Deployment, Hop::Pod).clone(),
    )
}

// ---- Confirmed ---------------------------------------------------------

#[tokio::test]
async fn a_pinned_deployment_whose_current_pods_run_the_digest_is_confirmed() {
    let world = world(parts());
    let trail = trail(&world).await;
    let (app, pods) = (
        one(&trail, Hop::Application, Hop::Deployment),
        one(&trail, Hop::Deployment, Hop::Pod),
    );
    assert_eq!(app.confidence, Confidence::Confirmed, "{app:#?}");
    assert_eq!(pods.confidence, Confidence::Confirmed, "{pods:#?}");
    assert!(matches!(&pods.key, Key::Digest(d) if d.as_str() == NEW));
    let text = render(&trail);
    // The tie is listed step by step, each reported by a controller.
    for needle in [
        "ReplicaSet.apps shop/storefront-6fdf",
        "/metadata/ownerReferences",
        "Deployment.apps shop/storefront",
        "deployment.kubernetes.io~1revision",
        "pod -> ReplicaSet -> Deployment by owner UID",
    ] {
        assert!(text.contains(needle), "{needle} in {text}");
    }
}

#[tokio::test]
async fn a_deployment_joins_a_freight_on_another_spelling_of_its_repository() {
    // Docker Hub, spelled three ways: the Freight names its legacy host,
    // Argo CD no registry, and the Deployment and its pod docker.io.
    let image = format!("docker.io/acme/storefront@{NEW}");
    let mut world = world((
        deployment(&image),
        vec![deployment_set("6fdf", "2", &image, 1, 1)],
        vec![deployment_pod(
            "storefront-6fdf-x",
            "6fdf",
            &image,
            &image_id(&image),
            true,
        )],
    ));
    world.kargo = world
        .kargo
        .with("freights", freights_on("index.docker.io/acme/storefront"));
    world.argocd = world.argocd.with(
        "applications",
        vec![summarised(
            unannotated_application(),
            &[&format!("acme/storefront@{NEW}")],
        )],
    );
    let (app, pods) = chain(&world).await;
    assert_eq!(app.confidence, Confidence::Confirmed, "{app:#?}");
    assert_eq!(pods.confidence, Confidence::Confirmed, "{pods:#?}");
    assert!(matches!(&pods.key, Key::Digest(d) if d.as_str() == NEW));
}

#[tokio::test]
async fn a_sidecar_on_another_digest_does_not_count_against_the_pinned_container() {
    let (deployment, sets, mut pods) = parts();
    let other = format!("registry.example/acme/proxy@{OLD}");
    pods[0]["status"]["containerStatuses"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "proxy", "image": other, "imageID": image_id(&other), "ready": true}));
    let (_, pods) = chain(&world((deployment, sets, pods))).await;
    assert_eq!(pods.confidence, Confidence::Confirmed, "{pods:#?}");
}

#[tokio::test]
async fn pods_of_an_older_revision_are_not_judged() {
    let (deployment, mut sets, mut pods) = parts();
    let stale = format!("{REPO}@{OLD}");
    sets.push(deployment_set("7f3b", "1", &stale, 1, 1));
    pods.push(deployment_pod(
        "storefront-7f3b-a",
        "7f3b",
        &stale,
        &image_id(&stale),
        true,
    ));
    let world = world((deployment, sets, pods));
    // Only the current hash's pods are asked for.
    let (_, link) = chain(&world).await;
    assert_eq!(link.confidence, Confidence::Confirmed, "{link:#?}");
    // A rolling update is seen, and is no reason for a claim.
    assert!(!link.reason.contains("7f3b"), "{}", link.reason);
    assert!(
        link.evidence.iter().any(|seen| seen.object.name
            == "rolling: 1 pod(s) of the previous revision, ReplicaSet storefront-7f3b"),
        "{:#?}",
        link.evidence
    );
}

// ---- Claimed -----------------------------------------------------------

#[tokio::test]
async fn a_spec_that_names_a_tag_only_is_claimed_while_its_pods_still_confirm() {
    let (mut deployment, sets, pods) = parts();
    deployment["spec"]["template"]["spec"]["containers"][0]["image"] =
        json!(format!("{REPO}:v1.4.0"));
    let (app, pods) = chain(&world((deployment, sets, pods))).await;
    assert_eq!(app.confidence, Confidence::Claimed);
    assert!(
        app.reason.contains("does not pin the digest"),
        "{}",
        app.reason
    );
    assert_eq!(pods.confidence, Confidence::Confirmed);
}

#[tokio::test]
async fn a_pin_the_image_summary_does_not_list_is_declared_only() {
    let mut world = world(parts());
    let mut app = unannotated_application();
    app["status"]["summary"] = json!({"images": [format!("{REPO}:v1.4.0")]});
    world.argocd = world.argocd.with("applications", vec![app]);
    let (app, pods) = chain(&world).await;
    assert_eq!(app.confidence, Confidence::Claimed);
    assert!(app.reason.contains("declared only"), "{}", app.reason);
    assert!(
        app.reason
            .contains("image summary lists tag v1.4.0 instead"),
        "{}",
        app.reason
    );
    assert_eq!(pods.confidence, Confidence::Confirmed);
}

#[tokio::test]
async fn pods_that_run_another_digest_make_both_links_claims_naming_it() {
    let (deployment, sets, mut pods) = parts();
    let moved = format!("{REPO}@{OLD}");
    pods[0]["status"]["containerStatuses"][0]["imageID"] = json!(image_id(&moved));
    let (app, pods) = chain(&world((deployment, sets, pods))).await;
    assert_eq!(pods.confidence, Confidence::Claimed);
    assert!(pods.reason.contains("another digest"), "{}", pods.reason);
    assert!(pods.reason.contains(&OLD[7..19]), "{}", pods.reason);
    assert_eq!(app.confidence, Confidence::Claimed);
}

#[tokio::test]
async fn a_capped_pod_listing_only_claims() {
    let mut world = world(parts());
    world.environment = world.environment.capped("pods");
    let (app, pods) = chain(&world).await;
    assert_eq!(pods.confidence, Confidence::Claimed, "{pods:#?}");
    assert!(pods.reason.contains("page cap"), "{}", pods.reason);
    assert_eq!(app.confidence, Confidence::Claimed);
}

#[tokio::test]
async fn a_deployment_that_has_not_observed_its_latest_spec_only_claims() {
    let (mut deployment, sets, pods) = parts();
    deployment["status"]["observedGeneration"] = json!(2);
    let (_, pods) = chain(&world((deployment, sets, pods))).await;
    assert_eq!(pods.confidence, Confidence::Claimed, "{pods:#?}");
    assert_eq!(pods.key, Key::None);
    assert!(
        pods.reason.contains("observedGeneration 2, generation 3"),
        "{}",
        pods.reason
    );
}

#[tokio::test]
async fn a_pod_that_is_not_ready_yet_only_claims() {
    let (deployment, sets, mut pods) = parts();
    pods[0]["status"]["containerStatuses"][0]["ready"] = json!(false);
    let (_, pods) = chain(&world((deployment, sets, pods))).await;
    assert_eq!(pods.confidence, Confidence::Claimed);
    assert!(pods.reason.contains("none ready yet"), "{}", pods.reason);
}

#[tokio::test]
async fn a_pod_with_the_hash_label_and_another_owner_is_not_the_deployments() {
    let (deployment, sets, mut pods) = parts();
    pods[0]["metadata"]["ownerReferences"][0]["uid"] =
        json!("0f0e0d0c-0000-4000-8000-0000000000ff");
    let (_, pods) = chain(&world((deployment, sets, pods))).await;
    assert_eq!(pods.confidence, Confidence::Claimed, "{pods:#?}");
    assert!(
        pods.reason
            .contains("belong to another owner and were not judged"),
        "{}",
        pods.reason
    );
}

// ---- Unknown -----------------------------------------------------------

#[tokio::test]
async fn deployments_that_cannot_be_read_are_unknown_with_the_reason() {
    let mut world = world(parts());
    world.environment = world.environment.refusing("deployments");
    let trail = trail(&world).await;
    let app = one(&trail, Hop::Application, Hop::Deployment);
    assert_eq!(app.confidence, Confidence::Unknown);
    assert!(app.reason.contains("forbidden by RBAC"), "{}", app.reason);
    assert!(app.evidence.is_empty());
    assert!(super::tests::link(&trail, Hop::Deployment, Hop::Pod).is_empty());
}

#[tokio::test]
async fn a_deployment_the_application_lists_but_the_cluster_lacks_is_unknown() {
    let mut world = world(parts());
    world.environment = world
        .environment
        .with("deployments", vec![])
        .capped("deployments");
    let (app, _) = trail_links(&world).await;
    assert_eq!(app.confidence, Confidence::Unknown);
    assert!(app.reason.contains("not found"), "{}", app.reason);
    assert!(app.reason.contains("page cap"), "{}", app.reason);
}

async fn trail_links(world: &World) -> (Link, Vec<Link>) {
    let trail = trail(world).await;
    (
        one(&trail, Hop::Application, Hop::Deployment).clone(),
        super::tests::link(&trail, Hop::Deployment, Hop::Pod)
            .into_iter()
            .cloned()
            .collect(),
    )
}

#[tokio::test]
async fn replica_sets_that_cannot_be_read_leave_the_pods_unknown() {
    let mut world = world(parts());
    world.environment = world.environment.refusing("replicasets");
    let (app, pods) = chain(&world).await;
    assert_eq!(pods.confidence, Confidence::Unknown);
    assert!(
        pods.reason
            .contains("its ReplicaSets were not read (not readable (refused): forbidden by RBAC)"),
        "{}",
        pods.reason
    );
    assert_eq!(
        app.confidence,
        Confidence::Claimed,
        "declared only without pods"
    );
}

#[tokio::test]
async fn pods_that_cannot_be_read_are_unknown() {
    let mut world = world(parts());
    world.environment = world.environment.refusing("pods");
    let (_, pods) = chain(&world).await;
    assert_eq!(pods.confidence, Confidence::Unknown);
    assert!(pods.reason.contains("forbidden by RBAC"), "{}", pods.reason);
}

#[tokio::test]
async fn no_current_revision_is_unknown_not_a_guess() {
    let (mut deployment, sets, pods) = parts();
    deployment["metadata"]["annotations"] = json!({});
    let (_, pods) = chain(&world((deployment, sets, pods))).await;
    assert_eq!(pods.confidence, Confidence::Unknown);
    assert!(
        pods.reason.contains("deployment.kubernetes.io/revision"),
        "{}",
        pods.reason
    );
}

#[tokio::test]
async fn a_current_replica_set_among_those_not_read_is_unknown() {
    let (deployment, mut sets, pods) = parts();
    sets[0]["metadata"]["annotations"] = json!({"deployment.kubernetes.io/revision": "1"});
    let mut world = world((deployment, sets, pods));
    world.environment = world.environment.capped("replicasets");
    let (_, pods) = chain(&world).await;
    assert_eq!(pods.confidence, Confidence::Unknown);
    assert!(
        pods.reason.contains("carries its current revision 2"),
        "{}",
        pods.reason
    );
    assert!(pods.reason.contains("page cap"), "{}", pods.reason);
}

// ---- What is not a Deployment ------------------------------------------

#[tokio::test]
async fn a_namesake_in_another_group_is_not_the_deployment() {
    let mut world = world(parts());
    let mut app = unannotated_application();
    app["status"]["resources"][0]["group"] = json!("example.test");
    world.argocd = world.argocd.with("applications", vec![app]);
    let trail = trail(&world).await;
    assert!(super::tests::link(&trail, Hop::Application, Hop::Deployment).is_empty());
    // Managing no known controller, its namespace's pods are judged as before.
    assert_eq!(
        one(&trail, Hop::Application, Hop::Pod).confidence,
        Confidence::Confirmed
    );
}

#[tokio::test]
async fn a_deployment_that_pins_another_repository_is_not_part_of_the_hop() {
    let mut world = world(parts());
    let mut proxy = deployment_proxy();
    proxy["metadata"]["name"] = json!("storefront-proxy");
    let mut app = unannotated_application();
    app["status"]["resources"]
        .as_array_mut()
        .unwrap()
        .push(json!({"group": "apps", "kind": "Deployment", "namespace": "shop", "name": "storefront-proxy"}));
    world.argocd = world.argocd.with("applications", vec![app]);
    world.environment = world
        .environment
        .with("deployments", vec![deployment(&pinned_image()), proxy]);
    let trail = trail(&world).await;
    let links = super::tests::link(&trail, Hop::Application, Hop::Deployment);
    assert_eq!(links.len(), 1, "{links:#?}");
    assert_eq!(links[0].subject, "shop/storefront");
    assert_eq!(links[0].confidence, Confidence::Confirmed);
}

fn deployment_proxy() -> Value {
    deployment(&format!("registry.example/acme/proxy@{OLD}"))
}

#[tokio::test]
async fn an_application_none_of_whose_deployments_pin_the_repository_falls_back_to_the_namespace_pods()
 {
    let (_, sets, pods) = parts();
    let other = deployment(&format!("registry.example/acme/proxy@{OLD}"));
    let trail = trail(&world((other, sets, pods))).await;
    assert!(super::tests::link(&trail, Hop::Application, Hop::Deployment).is_empty());
    let pods = one(&trail, Hop::Application, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Confirmed, "{pods:#?}");
}

#[tokio::test]
async fn a_deployment_beside_a_stateful_set_does_not_hide_the_namespace_pods() {
    // The Freight is of the API, which a StatefulSet runs; the Deployment is
    // another image's.
    let mut world = world(parts());
    let mut app = unannotated_application();
    app["status"]["resources"] = json!([
        {"group": "apps", "kind": "Deployment", "namespace": "shop", "name": "redis"},
        {"group": "apps", "kind": "StatefulSet", "namespace": "shop", "name": "storefront"}]);
    world.argocd = world.argocd.with("applications", vec![app]);
    let mut redis = deployment("registry.example/acme/redis:7");
    redis["metadata"]["name"] = json!("redis");
    world.environment = world.environment.with("deployments", vec![redis]);
    let trail = trail(&world).await;
    assert!(super::tests::link(&trail, Hop::Application, Hop::Deployment).is_empty());
    assert_eq!(
        one(&trail, Hop::Application, Hop::Pod).confidence,
        Confidence::Confirmed
    );
}

#[tokio::test]
async fn a_rollout_and_a_deployment_under_one_application_are_both_joined() {
    let mut world = world(parts());
    let mut app = unannotated_application();
    app["status"]["resources"] = json!([
        {"group": "argoproj.io", "kind": "Rollout", "namespace": "shop", "name": "storefront"},
        {"group": "apps", "kind": "Deployment", "namespace": "shop", "name": "storefront"}]);
    world.argocd = world.argocd.with("applications", vec![app]);
    let image = pinned_image();
    world.environment = FixtureReader::default()
        .serves("argoproj.io", "v1alpha1", &["rollouts", "analysisruns"])
        .with("rollouts", vec![rollout("registry.example/acme/web:v1")])
        .with("deployments", vec![deployment(&image)])
        .with(
            "replicasets",
            vec![
                replica_set("5d9c", "registry.example/acme/web:v1", 1, 1),
                deployment_set("6fdf", "2", &image, 1, 1),
            ],
        )
        .with(
            "pods",
            vec![deployment_pod(
                "storefront-6fdf-x",
                "6fdf",
                &image,
                &image_id(&image),
                true,
            )],
        );
    let trail = trail(&world).await;
    assert_eq!(
        one(&trail, Hop::Application, Hop::Rollout).confidence,
        Confidence::Claimed
    );
    let deployment = one(&trail, Hop::Application, Hop::Deployment);
    assert_eq!(
        deployment.confidence,
        Confidence::Confirmed,
        "{deployment:#?}"
    );
    assert_eq!(
        one(&trail, Hop::Deployment, Hop::Pod).confidence,
        Confidence::Confirmed
    );
}

#[tokio::test]
async fn a_current_replica_set_found_in_a_capped_listing_only_claims() {
    let mut world = world(parts());
    world.environment = world.environment.capped("replicasets");
    let (app, pods) = chain(&world).await;
    assert_eq!(pods.confidence, Confidence::Claimed, "{pods:#?}");
    assert_eq!(pods.key, Key::None);
    assert!(
        pods.reason
            .contains("another ReplicaSet with the same revision"),
        "{}",
        pods.reason
    );
    assert_eq!(app.confidence, Confidence::Claimed);
}

#[tokio::test]
async fn two_replica_sets_with_the_current_revision_are_unknown() {
    let (deployment, mut sets, pods) = parts();
    sets.push(deployment_set("7f3b", "2", &pinned_image(), 1, 1));
    let (_, pods) = chain(&world((deployment, sets, pods))).await;
    assert_eq!(pods.confidence, Confidence::Unknown);
    assert!(
        pods.reason
            .contains("several ReplicaSets the Deployment owns carry its current revision 2"),
        "{}",
        pods.reason
    );
}
