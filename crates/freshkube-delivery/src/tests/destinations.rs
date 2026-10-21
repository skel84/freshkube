use super::*;

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
    let trail = join(&collect(&clusters, &plan, observed_at()).await);
    let deployment = one(&trail, Hop::Application, Hop::Deployment);
    assert_eq!(deployment.confidence, Confidence::Unknown);
    assert!(world.environment.requests.borrow().is_empty());
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
