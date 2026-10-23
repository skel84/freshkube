use super::*;

#[tokio::test]
async fn a_change_reads_from_its_freight_to_the_pods_in_one_cluster() {
    let reader = one_cluster(|_| {});
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("the Freight is read");

    assert_eq!(change.freight, "wonky-otter");
    assert_eq!(change.kargo_context, "core");
    let phases: Vec<Phase> = change.groups.iter().map(|g| g.phase.clone()).collect();
    assert_eq!(phases, [Phase::Build, Phase::Freight, Phase::Stage(0)]);

    let dev = &change.stages[0];
    assert_eq!(dev.name, "dev");
    assert_eq!(
        dev.cluster,
        Destination::Cluster("core".into()),
        "Argo CD deploys to its own cluster"
    );
    assert_eq!(change.groups[2].detail, "deploys to core");
    assert_eq!(
        (dev.state, dev.words.as_str()),
        (HealthIndicator::Healthy, "Verified")
    );
    assert_eq!(dev.eligible, Eligible::Warehouse { at: None });
    assert!(
        matches!(&dev.promotion, Promotion::Other { how, .. } if how.starts_with("Likely automatic")),
        "{:?}",
        dev.promotion
    );
    assert_eq!(dev.running, None);
    assert_eq!(dev.object.cluster, "cluster-key", "objects open by the key");
    assert_eq!(dev.object.kind, "Stage");

    let names: Vec<&str> = change.hops.iter().map(|hop| hop.name.as_str()).collect();
    assert!(names.contains(&"Run storefront-push-x"), "{names:?}");
    assert!(names.contains(&"Freight wonky-otter"), "{names:?}");
    assert!(names.contains(&"Warehouse storefront"), "{names:?}");
    for gate in ["Eligible", "Promotion", "Verification"] {
        let hop = change
            .hops
            .iter()
            .find(|hop| hop.key == format!("dev-{}", gate.to_lowercase()))
            .unwrap_or_else(|| panic!("{gate} in {names:?}"));
        assert_eq!(hop.group, 2);
        assert!(matches!(hop.shows, Shows::Stage(0)));
    }
    let pods = change
        .hops
        .iter()
        .find(|hop| hop.name == "Pods")
        .expect("the pods are reached");
    assert_eq!(pods.group, 2);
    assert_eq!(pods.state, HealthIndicator::Healthy, "the pod is ready");
    assert!(change.hops.iter().all(|hop| hop.from == "core"));
}

#[tokio::test]
async fn every_links_confidence_is_the_joins() {
    let reader = one_cluster(|_| {});
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let mut plan = plan(&[]);
    plan.followed = Some("f-new".into());
    plan.build_namespace = Ok(None);
    plan.argocd = "cluster-key".into();
    plan.environment = "cluster-key".into();
    let clusters = crate::collect::Clusters {
        kargo: &reader,
        argocd: &reader,
        tekton: &reader,
        environment: &reader,
        github: &FixtureGitHub::default(),
    };
    let trail = join(&crate::collect::collect(&clusters, &plan, observed_at()).await);
    for (key, hop) in [
        ("build-run-0", Hop::PipelineRun),
        ("dev-argocd-0", Hop::Application),
        ("dev-rollout-1", Hop::Rollout),
        ("dev-pods-2", Hop::Pod),
    ] {
        let shown = change
            .hops
            .iter()
            .find(|row| row.key == key)
            .unwrap_or_else(|| panic!("{key}"));
        let joined = trail
            .links
            .iter()
            .find(|link| link.to == hop)
            .unwrap_or_else(|| panic!("{hop:?}"));
        assert_eq!(shown.link, Some(joined.confidence), "{key}");
    }
}

#[tokio::test]
async fn a_freight_without_a_commit_still_reaches_its_pods() {
    let reader = one_cluster(|world| {
        let mut bare = freight("f-new", NEW, SHA);
        bare["commits"] = json!([]);
        world.kargo = std::mem::take(&mut world.kargo).with("freights", vec![bare]);
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let build = &change.hops[0];
    assert_eq!(build.group, 0);
    assert_eq!(build.state, HealthIndicator::Unknown);
    assert_eq!(build.link, Some(Confidence::Unknown));
    let freight = change
        .hops
        .iter()
        .find(|hop| hop.key == "freight")
        .expect("freight");
    assert_eq!(freight.state, HealthIndicator::Unknown);
    assert!(
        change.hops.iter().any(|hop| hop.name == "Pods"),
        "the followed Freight's Stage is read without a commit"
    );
    assert!(
        !reader
            .requests
            .borrow()
            .iter()
            .any(|r| r.contains("pipelineruns")),
        "no build is looked for without a commit"
    );
}

#[tokio::test]
async fn pods_of_a_freight_without_a_digest_are_unknown_not_another_digest() {
    let reader = one_cluster(|world| {
        let mut tagged = freight("f-new", NEW, SHA);
        tagged["images"][0]
            .as_object_mut()
            .expect("an image")
            .remove("digest");
        world.kargo = std::mem::take(&mut world.kargo).with("freights", vec![tagged]);
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let pods = change
        .hops
        .iter()
        .find(|hop| hop.name == "Pods")
        .expect("the pods are reached");
    let Shows::Hop(detail) = &pods.shows else {
        panic!("a hop");
    };
    assert_eq!(pods.link, Some(Confidence::Claimed));
    assert_eq!(
        (pods.state, detail.state.as_str()),
        (
            HealthIndicator::Unknown,
            "The Freight names no image digest"
        )
    );
}

#[tokio::test]
async fn a_commit_from_an_image_revision_says_why_no_build_joined() {
    let reader = one_cluster(|world| {
        let mut bare = freight("f-new", NEW, SHA);
        bare["commits"] = json!([]);
        bare["images"][0]["annotations"] = json!({"org.opencontainers.image.revision": SHA});
        world.kargo = std::mem::take(&mut world.kargo).with("freights", vec![bare]);
        world.tekton = std::mem::take(&mut world.tekton).with("pipelineruns", Vec::new());
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let freight = row(&change, "freight");
    assert!(
        freight
            .detail
            .contains("from an image's revision annotation")
            && freight.detail.contains("no PipelineRun"),
        "{}",
        freight.detail
    );
    assert!(
        !freight.detail.contains("names no commit"),
        "{}",
        freight.detail
    );
}
