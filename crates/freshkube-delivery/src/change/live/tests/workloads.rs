use super::*;

#[tokio::test]
async fn a_degraded_rollout_and_unready_pods_are_shown_as_they_are() {
    let image_id = format!("docker-pullable://{REPO}@{NEW}");
    let reader = one_cluster(|world| {
        let mut degraded = rollout(&format!("{REPO}:v1.4.0"));
        degraded["status"]["phase"] = json!("Degraded");
        world.environment = std::mem::take(&mut world.environment)
            .with("rollouts", vec![degraded])
            .with(
                "pods",
                vec![pod_of(
                    "storefront-5d9c-a",
                    "5d9c",
                    &format!("{REPO}:v1.4.0"),
                    &image_id,
                    false,
                )],
            );
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let rollout = row(&change, "dev-rollout-1");
    assert_eq!(
        rollout.state,
        HealthIndicator::Error,
        "joined, and degraded all the same"
    );
    assert_ne!(rollout.link, Some(Confidence::Unknown));
    let pods = row(&change, "dev-pods-2");
    let Shows::Hop(detail) = &pods.shows else {
        panic!("a hop");
    };
    assert_eq!(
        (pods.state, detail.state.as_str()),
        (HealthIndicator::Warning, "None ready")
    );
}

/// [`one_cluster`] with its Application deploying the storefront
/// Deployment, whose current ReplicaSet is `sets`' revision `2`.
fn deployed(sets: Vec<Value>) -> FixtureReader {
    deployed_running(sets, NEW)
}

/// [`deployed`], its pod reporting `running` as its image's digest.
fn deployed_running(sets: Vec<Value>, running: &str) -> FixtureReader {
    let image = format!("{REPO}@{NEW}");
    let id = format!("docker-pullable://{REPO}@{running}");
    one_cluster(|world| {
        let mut app = application(Some(IN_CLUSTER_SERVER));
        app["status"]["resources"] = json!([
            {"group": "apps", "kind": "Deployment", "namespace": "shop", "name": "storefront"}]);
        world.argocd = std::mem::take(&mut world.argocd).with("applications", vec![app]);
        world.environment = FixtureReader::default()
            .with("deployments", vec![deployment(&image)])
            .with("replicasets", sets)
            .with(
                "pods",
                vec![deployment_pod(
                    "storefront-6fdf-x",
                    "6fdf",
                    &image,
                    &id,
                    true,
                )],
            );
    })
}

#[tokio::test]
async fn application_pods_name_the_destination_namespace_instead_of_the_application() {
    let reader = one_cluster(|world| {
        let mut app = application(Some(IN_CLUSTER_SERVER));
        app["status"]["resources"] = json!([]);
        world.argocd = std::mem::take(&mut world.argocd).with("applications", vec![app]);
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let pods = change.hop("dev-pods-1").expect("Application's pods");
    assert_eq!(field(pods, "Namespace"), Some(&Shown::Mono("shop".into())));
    assert_eq!(field(pods, "Name"), None);
    assert_eq!(pods.state, HealthIndicator::Healthy);
    assert_eq!(pods.after.as_deref(), Some("dev-argocd-0"));
}

#[tokio::test]
async fn controller_pods_keep_the_controllers_namespace_and_name() {
    let image = format!("{REPO}@{NEW}");
    for reader in [
        one_cluster(|_| {}),
        deployed(vec![deployment_set("6fdf", "2", &image, 1, 1)]),
    ] {
        let change = read(&reader, &place("f-new"), observed_at())
            .await
            .expect("read");
        let pods = change.hop("dev-pods-2").expect("controller's pods");
        assert_eq!(field(pods, "Namespace"), Some(&Shown::Mono("shop".into())));
        assert_eq!(field(pods, "Name"), Some(&Shown::Mono("storefront".into())));
    }
}

/// The hop's field labelled `label`, as shown.
fn field<'a>(hop: &'a Row, label: &str) -> Option<&'a Shown> {
    let Shows::Hop(detail) = &hop.shows else {
        return None;
    };
    detail
        .fields
        .iter()
        .find(|field| field.label == label)
        .map(|field| &field.value)
}

#[tokio::test]
async fn a_deployment_names_coroots_revision_by_its_current_replica_sets_hash() {
    let image = format!("{REPO}@{NEW}");
    let reader = deployed(vec![
        deployment_set("4b1c", "1", &image, 0, 0),
        deployment_set("6fdf", "2", &image, 1, 1),
    ]);
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);
    let (key, _, to, disabled) = link_out(&links, "Coroot revision");
    assert_eq!(
        to.as_deref(),
        Some("cluster-key/shop/storefront Some(\"6fdf\")"),
        "the current revision's hash, not an older one's"
    );
    assert_eq!(disabled, &None);
    let hop = change.hop(key).expect("the Deployment's hop");
    assert!(hop.name.starts_with("Deployment"), "{}", hop.name);
    assert_eq!(
        field(hop, "Pod template hash"),
        Some(&Shown::Mono("6fdf".into()))
    );
}

#[tokio::test]
async fn a_deployment_whose_pods_run_another_digest_does_not_offer_its_revision() {
    let image = format!("{REPO}@{NEW}");
    let reader = deployed_running(vec![deployment_set("6fdf", "2", &image, 1, 1)], OLD);
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);
    let (key, _, to, disabled) = link_out(&links, "Coroot revision");
    assert_eq!(
        to.as_deref(),
        Some("cluster-key/shop/storefront Some(\"6fdf\")")
    );
    assert_eq!(
        disabled.as_deref(),
        Some(
            "Its current revision isn't confirmed to run this Freight: see its Pods row. \
             Coroot's revision of it may be another change."
        ),
        "the spec pins the digest, but the pods say otherwise"
    );
    let hop = change.hop(key).expect("the Deployment's hop");
    assert_eq!(
        field(hop, "Pod template hash"),
        Some(&Shown::Mono("6fdf".into()))
    );
}

#[tokio::test]
async fn a_deployment_without_a_current_replica_set_says_why_its_revision_isnt_known() {
    let image = format!("{REPO}@{NEW}");
    let reader = deployed(vec![deployment_set("4b1c", "1", &image, 0, 0)]);
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);
    let (key, _, to, disabled) = link_out(&links, "Coroot revision");
    assert_eq!(to.as_deref(), Some("cluster-key/shop/storefront None"));
    assert_eq!(
        disabled.as_deref(),
        Some(
            "No ReplicaSet the Deployment owns carries its current revision 2, so Coroot's \
             revision of it isn't known."
        )
    );
    let hop = change.hop(key).expect("the Deployment's hop");
    assert_eq!(field(hop, "Pod template hash"), None);
}

#[tokio::test]
async fn a_rollouts_revision_is_greyed_out_since_coroot_keeps_deployments_only() {
    let reader = one_cluster(|_| {});
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);
    let (key, _, _, disabled) = link_out(&links, "Coroot revision");
    assert_eq!(key, "dev-rollout-1");
    assert_eq!(
        disabled.as_deref(),
        Some("Coroot keeps revisions of Deployments only, and this is an Argo Rollout.")
    );
}
