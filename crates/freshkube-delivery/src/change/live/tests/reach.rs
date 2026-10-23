use super::*;

/// One cluster whose dev Stage runs `f-new` through an Application that
/// deploys the storefront Deployment there, pinned to the Freight's digest,
/// with its current pod running it. `edit` changes the Deployment.
fn on_deployment(edit: impl FnOnce(&mut Value)) -> World {
    on_deployment_at(IN_CLUSTER_SERVER, edit)
}

async fn reach_of(world: World) -> Vec<(String, Reach)> {
    let change = read(&world.one_cluster(), &place("f-new"), observed_at())
        .await
        .expect("read");
    change
        .stages
        .iter()
        .map(|stage| (stage.name.clone(), stage.reach.clone()))
        .collect()
}

fn dev(reach: Reach) -> Vec<(String, Reach)> {
    vec![("dev".into(), reach)]
}

#[tokio::test]
async fn a_digest_that_reaches_the_pods_is_confirmed() {
    assert_eq!(
        reach_of(on_deployment(|_| {})).await,
        dev(Reach::Linked(Confidence::Confirmed))
    );
}

#[tokio::test]
async fn a_deployment_that_names_its_image_by_tag_only_is_claimed() {
    let world = on_deployment(|deployment| {
        deployment["spec"]["template"]["spec"]["containers"][0]["image"] =
            json!(format!("{REPO}:v1.4.0"));
    });
    assert_eq!(
        reach_of(world).await,
        dev(Reach::Linked(Confidence::Claimed))
    );
}

#[tokio::test]
async fn a_deployment_that_hasnt_observed_its_spec_is_claimed() {
    let world = on_deployment(|deployment| {
        deployment["status"]["observedGeneration"] = json!(2);
    });
    assert_eq!(
        reach_of(world).await,
        dev(Reach::Linked(Confidence::Claimed))
    );
}

#[tokio::test]
async fn refused_pods_are_refused_not_unknown() {
    let mut world = on_deployment(|_| {});
    world.environment = std::mem::take(&mut world.environment).refusing("pods");
    assert_eq!(reach_of(world).await, dev(Reach::Refused("Pods".into())));
}

#[tokio::test]
async fn refused_applications_are_refused_not_unknown() {
    let mut world = on_deployment(|_| {});
    world.argocd = std::mem::take(&mut world.argocd).refusing("applications");
    assert_eq!(
        reach_of(world).await,
        dev(Reach::Refused("Argo CD Applications".into()))
    );
}

#[tokio::test]
async fn a_stage_that_runs_it_without_an_application_is_unknown() {
    let mut world = on_deployment(|_| {});
    world.argocd = std::mem::take(&mut world.argocd).with("applications", Vec::new());
    assert_eq!(
        reach_of(world).await,
        dev(Reach::Linked(Confidence::Unknown))
    );
}

#[tokio::test]
async fn a_stage_that_doesnt_run_it_is_not_reached() {
    let mut world = on_deployment(|_| {});
    world.kargo = std::mem::take(&mut world.kargo).with(
        "stages",
        vec![dev_holding_new(), stage_of("staging", &["dev"], &["f-old"])],
    );
    assert_eq!(
        reach_of(world).await,
        vec![
            ("dev".into(), Reach::Linked(Confidence::Confirmed)),
            ("staging".into(), Reach::NotReached),
        ]
    );
}

#[tokio::test]
async fn refused_names_only_the_kinds_its_own_stage_reaches() {
    // dev's Rollout couldn't be read; staging's Deployments neither. Each
    // Stage names its own.
    let mut world = on_deployment(|_| {});
    let mut staging = application(Some(IN_CLUSTER_SERVER));
    staging["metadata"]["name"] = json!("storefront-staging");
    staging["metadata"]["annotations"]["kargo.akuity.io/authorized-stage"] =
        json!("storefront:staging");
    staging["status"]["resources"] = json!([
        {"group": "apps", "kind": "Deployment", "namespace": "shop", "name": "storefront"}
    ]);
    world.argocd = std::mem::take(&mut world.argocd).with(
        "applications",
        vec![application(Some(IN_CLUSTER_SERVER)), staging],
    );
    world.kargo = std::mem::take(&mut world.kargo).with(
        "stages",
        vec![
            stage_of("dev", &[], &["f-new"]),
            stage_of("staging", &["dev"], &["f-new"]),
        ],
    );
    world.environment = std::mem::take(&mut world.environment)
        .serves("argoproj.io", "v1alpha1", &["rollouts", "analysisruns"])
        .refusing("rollouts")
        .refusing("deployments");
    assert_eq!(
        reach_of(world).await,
        vec![
            ("dev".into(), Reach::Refused("Rollouts".into())),
            ("staging".into(), Reach::Refused("Deployments".into())),
        ]
    );
}

#[tokio::test]
async fn a_stage_on_another_entry_is_claimed_as_argo_cd_reports_it() {
    let change = read_mapped(Some(ENV_A), mapped(&[(by_server(), "prod")])).await;
    assert_eq!(
        change.stages[0].reach,
        Reach::Linked(Confidence::Claimed),
        "the reported Rollout stands for the cluster nothing read"
    );
}

#[tokio::test]
async fn a_stage_on_another_entry_that_reports_no_workload_is_unknown() {
    let mut app = application(Some(ENV_A));
    app["status"]["resources"] =
        json!([{"group": "", "kind": "Service", "namespace": "shop", "name": "storefront"}]);
    let change = read_app(app, mapped(&[(by_server(), "prod")])).await;
    assert_eq!(change.stages[0].reach, Reach::Linked(Confidence::Unknown));
}

/// A run through an Application read here, which reached no pods, and one
/// on another entry that Argo CD reports, stays Unknown: the one elsewhere
/// stands only for its own links. No join leaves a local Application without
/// a Pod link, so the run is built by hand.
#[tokio::test]
async fn a_local_application_without_pods_stays_unknown_beside_one_elsewhere() {
    let mut local = application(Some(IN_CLUSTER_SERVER));
    local["metadata"]["name"] = json!("storefront-local");
    let mut remote = application(Some(ENV_A));
    remote["metadata"]["name"] = json!("storefront-prod");
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).with("applications", vec![local, remote]);
    });
    let mut place = place("f-new");
    place.mapping = mapped(&[(by_server(), "prod")]);
    let freight = read_freight(&reader, &place.project, &place.freight)
        .await
        .expect("freight");
    let evidence = collect(
        &clusters(&reader),
        &plan_of(&place, &freight),
        observed_at(),
    )
    .await;
    let link = |from, to, subject: &str, confidence| Link {
        cause: None,
        from,
        to,
        subject: subject.into(),
        key: JoinKey::None,
        confidence,
        reason: String::new(),
        evidence: Vec::new(),
        conflicts: Vec::new(),
        signature_warn: false,
    };
    let stage = link(
        Hop::Freight,
        Hop::Stage,
        "storefront/dev",
        Confidence::Confirmed,
    );
    let to_remote = link(
        Hop::Stage,
        Hop::Application,
        "argocd/storefront-prod",
        Confidence::Claimed,
    );
    let reported = link(
        Hop::Application,
        Hop::Rollout,
        "argocd/storefront-prod",
        Confidence::Unknown,
    );
    let to_local = link(
        Hop::Stage,
        Hop::Application,
        "argocd/storefront-local",
        Confidence::Claimed,
    );
    assert_eq!(
        reach(&place, &evidence, true, &[&stage, &to_remote, &reported]),
        Reach::Linked(Confidence::Claimed),
        "alone, the one Argo CD reports is Claimed"
    );
    assert_eq!(
        reach(
            &place,
            &evidence,
            true,
            &[&stage, &to_remote, &reported, &to_local]
        ),
        Reach::Linked(Confidence::Unknown)
    );
}

/// The why names this cluster as the page does, never by the connection's
/// key, which nobody reads; objects still open by the key.
#[tokio::test]
async fn a_stage_names_this_cluster_by_its_context_not_its_key() {
    const KEY: &str = "access:4f1e9c0d2b7a";
    let reader = one_cluster(|_| {});
    let mut place = place("f-new");
    place.cluster = KEY.into();
    let change = read(&reader, &place, observed_at()).await.expect("read");
    let argocd = change
        .hops
        .iter()
        .find(|hop| hop.key == "dev-argocd-0")
        .expect("the Stage's Application");
    assert!(
        argocd
            .detail
            .contains("destination is Argo CD's own cluster (in-cluster), context core"),
        "{}",
        argocd.detail
    );
    let texts = format!("{change:?}");
    let words: Vec<&str> = change
        .hops
        .iter()
        .map(|hop| hop.detail.as_str())
        .chain(change.groups.iter().map(|group| group.detail.as_str()))
        .collect();
    assert!(words.iter().all(|text| !text.contains(KEY)), "{words:#?}");
    assert!(
        texts.contains(&format!("cluster: \"{KEY}\"")),
        "objects open by the key"
    );
}
