use super::*;

#[tokio::test]
async fn a_deployment_is_joined_through_its_current_replica_set_to_its_pods() {
    let world = without_rollouts();
    let trail = run_configured(&world, &ENV, &FixtureGitHub::default(), None, |plan| {
        plan.stage_naming = Some(naming())
    })
    .await;
    let stage = one(&trail, Hop::Stage, Hop::Application);
    // Named by convention is weaker than the annotation, and says so.
    assert_eq!(stage.confidence, Confidence::Claimed);
    assert!(stage.reason.contains("no authorized-stage annotation"));
    let deployment = one(&trail, Hop::Application, Hop::Deployment);
    assert_eq!(
        deployment.confidence,
        Confidence::Confirmed,
        "{deployment:#?}"
    );
    assert_eq!(deployment.subject, "shop/storefront");
    let pods = one(&trail, Hop::Deployment, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Confirmed, "{pods:#?}");
    assert!(matches!(&pods.key, Key::Digest(d) if d.as_str() == NEW));
    assert!(
        pods.reason.contains("ReplicaSet storefront-6fdf"),
        "{}",
        pods.reason
    );
    assert_eq!(trail.running(), Some(Confidence::Confirmed));
    // Only reads: the namespace's Deployments and ReplicaSets, and the pods
    // of the current ReplicaSet's hash.
    let requests = world.environment.requests.borrow();
    assert!(
        requests.iter().all(|r| r.starts_with("LIST")),
        "{requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|r| r.contains("pods") && r.contains("pod-template-hash=6fdf")),
        "{requests:?}"
    );
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
    let trail = join(&collect(&clusters, &plan, observed_at()).await);
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
async fn only_the_current_revisions_pods_are_judged() {
    // The Rollout's status reports the ReplicaSet at the Freight's digest as
    // current: its pods are judged, and the older one beside it is not.
    let world = beside_a_stale_replica_set("5d9c");
    let trail = run(&world, &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Confirmed);
    assert!(matches!(&pods.key, Key::Digest(d) if d.as_str() == NEW));
    for words in [
        "2 containers run the Freight's digest, 2 ready of 2 pod containers read",
        "the pods of the Rollout's current pod hash 5d9c, ReplicaSet storefront-5d9c",
        "not judged: the pods of ReplicaSet storefront-7f3b",
    ] {
        assert!(pods.reason.contains(words), "{words}: {}", pods.reason);
    }
    assert!(!pods.reason.contains("other container"), "{}", pods.reason);
    let environment = world.environment.requests.borrow().join("\n");
    assert!(environment.contains("rollouts-pod-template-hash=5d9c"));
    assert!(!environment.contains("rollouts-pod-template-hash=7f3b"));
    assert!(environment.contains(
        "LIST apps/v1/replicasets ns=Some(\"shop\") selector=Some(\"rollouts-pod-template-hash\")"
    ));

    // It reports the older one as current, as during a canary's first step
    // or with a status that lags its spec: the current revision runs another
    // digest, and the pods at the Freight's digest are not judged.
    let world = beside_a_stale_replica_set("7f3b");
    let trail = run(&world, &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Claimed);
    for words in [
        "the current revision's pods run another digest (sha256:",
        "the pods of the Rollout's current pod hash 7f3b, ReplicaSet storefront-7f3b",
        "not judged: the pods of ReplicaSet storefront-5d9c",
    ] {
        assert!(pods.reason.contains(words), "{words}: {}", pods.reason);
    }
    let environment = world.environment.requests.borrow().join("\n");
    assert!(environment.contains("rollouts-pod-template-hash=7f3b"));
    assert!(!environment.contains("rollouts-pod-template-hash=5d9c"));
}

#[tokio::test]
async fn pods_without_an_owner_chain_to_the_rollout_only_claim() {
    let trail = run(&healthy(), &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Confirmed);
    assert!(
        pods.reason.contains(
            "the pods of the Rollout's current pod hash 5d9c, ReplicaSet storefront-5d9c"
        ),
        "{}",
        pods.reason
    );

    // ReplicaSets that can't be read leave the pods readable, but nothing
    // ties them to the Rollout but a label.
    let mut world = healthy();
    world.environment = world.environment.refusing("replicasets");
    let trail = run(&world, &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Claimed);
    for words in [
        "its ReplicaSets were not read",
        "so no owner reference ties the pods to the Rollout",
        "1 container runs the Freight's digest",
    ] {
        assert!(pods.reason.contains(words), "{words}: {}", pods.reason);
    }

    // The Rollout owns no ReplicaSet with its current pod hash.
    let mut world = healthy();
    world.environment = world.environment.with(
        "replicasets",
        vec![replica_set("7f3b", &format!("{REPO}:v1.4.0"), 1, 1)],
    );
    let trail = run(&world, &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Claimed);
    assert!(
        pods.reason
            .contains("no ReplicaSet the Rollout owns has its current pod hash 5d9c"),
        "{}",
        pods.reason
    );

    // A pod with the hash label that another ReplicaSet owns, as another
    // Rollout's could, is not this Rollout's.
    let mut world = healthy();
    let mut stray = pod(
        "storefront-5d9c-x",
        &format!("{REPO}:v1.4.0"),
        &format!("docker-pullable://{REPO}@{NEW}"),
    );
    stray["metadata"]["ownerReferences"][0]["uid"] =
        serde_json::json!("0f0e0d0c-0000-4000-8000-0000000000ff");
    world.environment = world.environment.with("pods", vec![stray]);
    let trail = run(&world, &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Claimed);
    for words in [
        "no pod of the current revision reports a digest yet",
        "1 pod container with that hash label belongs to another owner",
    ] {
        assert!(pods.reason.contains(words), "{words}: {}", pods.reason);
    }
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
async fn a_capped_replica_set_listing_says_so_when_the_current_one_is_missing() {
    let mut world = beside_a_stale_replica_set("5d9c");
    world.environment = world
        .environment
        .with(
            "replicasets",
            vec![replica_set("7f3b", &format!("{REPO}@{OLD}"), 1, 0)],
        )
        .capped("replicasets");
    let trail = run(&world, &ENV).await;
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Claimed);
    assert!(
        pods.reason.contains(
            "no ReplicaSet the Rollout owns has its current pod hash 5d9c; the listing stopped at the page cap"
        ),
        "{}",
        pods.reason
    );
}
