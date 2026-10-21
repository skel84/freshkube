use super::*;

#[tokio::test]
async fn a_healthy_change_joins_from_the_commit_to_the_pods() {
    let trail = run(&healthy(), &[("env-a", "https://env-a.example:6443")]).await;
    let commit = one(&trail, Hop::Commit, Hop::PipelineRun);
    // The label and the revision parameter are both declared: no result
    // reports the commit.
    assert_eq!(commit.confidence, Confidence::Claimed);
    assert!(commit.reason.contains("both declared"), "{}", commit.reason);
    assert_eq!(commit.key, Key::Sha(SHA.into()));
    let chain = one(&trail, Hop::PipelineRun, Hop::SupplyChain);
    assert_eq!(chain.confidence, Confidence::Claimed);
    assert!(
        chain
            .reason
            .contains("Conforma SUCCESS (0 failures, 1 warnings)")
    );
    let freight = one(&trail, Hop::PipelineRun, Hop::Freight);
    assert_eq!(freight.confidence, Confidence::Confirmed);
    assert_eq!(freight.subject, "storefront/f-new");
    assert!(matches!(&freight.key, Key::Digest(d) if d.as_str() == NEW));
    // Kargo's own record of what the Stage runs, its newest Freight history
    // entry, names the Freight: Confirmed by name. Other names and
    // annotations are claims.
    assert_eq!(
        one(&trail, Hop::Freight, Hop::Stage).confidence,
        Confidence::Confirmed
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
    let trail = run(&pinned(), &ENV).await;
    assert_eq!(
        one(&trail, Hop::Application, Hop::Rollout).confidence,
        Confidence::Confirmed
    );
}

#[tokio::test]
async fn a_rollout_joins_a_freight_on_another_spelling_of_its_repository() {
    // Docker Hub, spelled three ways: the Freight names no registry, the
    // Rollout pins its legacy host, and the pods and Argo CD name docker.io.
    let pin = format!("index.docker.io/acme/storefront@{NEW}");
    let running = "docker.io/acme/storefront:v1.4.0";
    let mut world = healthy();
    world.kargo = world.kargo.with("freights", freights_on("acme/storefront"));
    world.argocd = world.argocd.with(
        "applications",
        vec![summarised(
            application(Some("https://env-a.example:6443")),
            &[&format!("docker.io/acme/storefront@{NEW}")],
        )],
    );
    world.environment = world
        .environment
        .with("rollouts", vec![rollout(&pin)])
        .with("replicasets", vec![replica_set("5d9c", &pin, 1, 1)])
        .with(
            "pods",
            vec![pod(
                "storefront-5d9c-x",
                running,
                &format!("docker-pullable://docker.io/acme/storefront@{NEW}"),
            )],
        );
    let trail = run(&world, &ENV).await;
    let rollout = one(&trail, Hop::Application, Hop::Rollout);
    assert_eq!(rollout.confidence, Confidence::Confirmed, "{rollout:#?}");
    let pods = one(&trail, Hop::Rollout, Hop::Pod);
    assert_eq!(pods.confidence, Confidence::Confirmed, "{pods:#?}");
    assert!(matches!(&pods.key, Key::Digest(d) if d.as_str() == NEW));
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
    // No build is named: the change stands against the Freight's commit.
    assert!(link(&trail, Hop::PipelineRun, Hop::Freight).is_empty());
    let freight = one(&trail, Hop::Commit, Hop::Freight);
    assert_eq!(freight.confidence, Confidence::Confirmed);
    assert_eq!(freight.key, Key::Sha(SHA.into()));
    assert_eq!(freight.subject, "storefront/f-new");
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
