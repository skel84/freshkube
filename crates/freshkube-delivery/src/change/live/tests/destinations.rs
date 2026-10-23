use super::*;

#[tokio::test]
async fn an_application_in_a_cluster_that_isnt_read_leaves_its_pods_unknown() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).with(
            "applications",
            vec![application(Some("https://env-a.example:6443"))],
        );
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    assert_eq!(
        change.stages[0].cluster,
        Destination::Unknown(
            "server https://env-a.example:6443 isn't mapped to a workspace cluster".into()
        )
    );
    // Until Settings can map it, the reasons keep the spike's words.
    assert!(
        change
            .hops
            .iter()
            .any(|hop| hop.detail.contains("--known-as")),
        "{:#?}",
        change.hops
    );
    let change = read_mapped(Some("https://env-a.example:6443"), mapped(&[])).await;
    assert!(
        change
            .hops
            .iter()
            .any(|hop| hop.detail.contains("map it in Settings › Workspace")),
        "with Settings, the app says where to map it, not the spike's flags: {:#?}",
        change.hops
    );
    assert!(
        change
            .hops
            .iter()
            .filter(|hop| hop.group == 2 && hop.link.is_some())
            .skip(1)
            .all(|hop| hop.state == HealthIndicator::Unknown),
        "{:#?}",
        change.hops
    );
}

#[tokio::test]
async fn refused_applications_leave_the_destination_unknown_and_say_where() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).refusing("applications");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let dev = &change.stages[0];
    let Destination::Unknown(why) = &dev.cluster else {
        panic!("{:?}", dev.cluster);
    };
    assert!(
        why.starts_with("Argo CD Applications in argocd weren't read") && why.contains("forbidden"),
        "{why}"
    );
    assert!(change.groups[2].detail.starts_with("destination unknown: "));

    // The join's Unknown link names the Stage: the row says what wasn't
    // read, since the list was refused.
    let app = row(&change, "dev-argocd-0");
    assert_eq!(app.name, "Application not read");
    assert_eq!(app.state, HealthIndicator::Unknown);
    let Shows::Hop(detail) = &app.shows else {
        panic!("a hop");
    };
    assert_eq!(detail.title, "Application not read");
    assert!(
        detail
            .fields
            .iter()
            .any(|f| f.label == "Kargo Stage" && f.value == Shown::Mono("storefront/dev".into())),
        "{:?}",
        detail.fields
    );
    assert!(
        detail.fields.iter().all(|f| f.label != "Name"),
        "the Stage isn't shown as the Application: {:?}",
        detail.fields
    );
}

#[tokio::test]
async fn no_application_for_the_stage_is_unknown_not_another_cluster() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).with("applications", Vec::new());
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    assert_eq!(
        change.stages[0].cluster,
        Destination::Unknown("no Argo CD Application in argocd was found for it".into())
    );
    // The list was read, so the row says what wasn't found.
    assert_eq!(row(&change, "dev-argocd-0").name, "Application not found");
}

#[tokio::test]
async fn a_capped_application_list_says_the_application_may_be_past_the_cap() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd)
            .with("applications", Vec::new())
            .capped("applications");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let Destination::Unknown(why) = &change.stages[0].cluster else {
        panic!("{:?}", change.stages[0].cluster);
    };
    assert!(
        why.starts_with(
            "no Argo CD Application in argocd was found for it; the listing stopped at the page cap"
        ),
        "{why}"
    );
    assert_eq!(row(&change, "dev-argocd-0").name, "Application not found");
}

fn by_name() -> Key {
    Key::Name("env-dev".into())
}

#[tokio::test]
async fn a_destination_mapped_by_server_names_its_workspace_cluster() {
    let change = read_mapped(Some(ENV_A), mapped(&[(by_server(), "prod")])).await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Entry {
            entry: "prod".into(),
            via: format!("server {ENV_A}"),
        }
    );
    assert_eq!(change.groups[2].detail, "deploys to prod (mapped)");

    // What Argo CD reports there, its claim, opening on prod; the Service it
    // also lists is no workload.
    let reported: Vec<&Row> = change
        .hops
        .iter()
        .filter(|hop| hop.key.starts_with("dev-reported-"))
        .collect();
    assert_eq!(reported.len(), 1, "{:#?}", change.hops);
    let rollout = reported[0];
    assert_eq!(rollout.name, "Rollout storefront");
    assert_eq!(rollout.group, 2);
    assert_eq!(rollout.link, Some(Confidence::Claimed));
    assert_eq!(rollout.from, "core", "Argo CD was read on the open cluster");
    let Shows::Hop(detail) = &rollout.shows else {
        panic!("a hop")
    };
    assert_eq!(
        detail.link.as_ref().map(|link| link.confidence),
        Some(Confidence::Claimed)
    );
    let (entry, object) = detail
        .actions
        .iter()
        .find_map(|action| match &action.target {
            super::super::Target::OnEntry { entry, object, .. } => Some((entry, object)),
            _ => None,
        })
        .expect("it opens on its workspace cluster");
    assert_eq!(
        object.cluster, "",
        "a workspace entry is never a connection"
    );
    assert_eq!(
        (
            entry.as_str(),
            object.group.as_str(),
            object.version.as_str(),
            object.plural.as_str(),
            object.namespace.as_str(),
            object.name.as_str(),
        ),
        (
            "prod",
            "argoproj.io",
            "v1alpha1",
            "rollouts",
            "shop",
            "storefront"
        )
    );
    // Nothing on prod was read, so no row says it was not the environment.
    assert!(
        !change
            .hops
            .iter()
            .any(|hop| hop.group == 2 && hop.detail.contains("nothing was read there")),
        "{:#?}",
        change.hops
    );
}

#[tokio::test]
async fn a_destination_mapped_by_name_names_its_workspace_cluster() {
    let change = read_mapped(None, mapped(&[(by_name(), "prod")])).await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Entry {
            entry: "prod".into(),
            via: "name env-dev".into(),
        }
    );
    // A server row doesn't match a destination given by name.
    let change = read_mapped(None, mapped(&[(Key::Server(ENV_A.into()), "prod")])).await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Unknown(
            "Argo CD's cluster env-dev isn't mapped to a workspace cluster".into()
        )
    );
}

#[tokio::test]
async fn an_unmapped_destination_stays_unknown_with_why() {
    let change = read_mapped(Some(ENV_A), mapped(&[(by_name(), "prod")])).await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Unknown(format!(
            "server {ENV_A} isn't mapped to a workspace cluster"
        ))
    );
    assert!(!change.hops.iter().any(|hop| hop.key.contains("-reported-")));
    let change = read_mapped(None, Mapping::default()).await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Unknown(
            "Argo CD's cluster env-dev isn't mapped to a workspace cluster".into()
        )
    );
}

#[tokio::test]
async fn a_destination_mapped_to_a_cluster_no_longer_listed_is_unknown() {
    let change = read_mapped(Some(ENV_A), mapped(&[(by_server(), "gone")])).await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Unknown("mapped to gone, which the workspace no longer lists".into())
    );
    assert!(!change.hops.iter().any(|hop| hop.key.contains("-reported-")));
    // Its rows say so as the Stage does.
    let rows: Vec<&str> = change
        .hops
        .iter()
        .filter(|hop| hop.group == 2 && hop.detail.contains("nothing was read there"))
        .map(|hop| hop.detail.as_str())
        .collect();
    assert!(!rows.is_empty(), "{:#?}", change.hops);
    assert!(
        rows.iter()
            .all(|detail| detail
                .ends_with("it is mapped to gone, which the workspace no longer lists")),
        "{rows:#?}"
    );
}

#[tokio::test]
async fn two_destinations_mapped_to_one_cluster_both_name_it() {
    let rows = [(by_server(), "prod"), (by_name(), "prod")];
    for (server, via) in [
        (Some(ENV_A), format!("server {ENV_A}")),
        (None, "name env-dev".to_owned()),
    ] {
        let change = read_mapped(server, mapped(&rows)).await;
        assert_eq!(
            change.stages[0].cluster,
            Destination::Entry {
                entry: "prod".into(),
                via,
            }
        );
    }
}

#[tokio::test]
async fn one_destination_mapped_twice_is_never_guessed() {
    let change = read_mapped(
        Some(ENV_A),
        mapped(&[(by_server(), "stage"), (Key::Server(ENV_A.into()), "prod")]),
    )
    .await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Unknown(
            "its destination is mapped to more than one workspace cluster (prod, stage)".into()
        )
    );
}

#[tokio::test]
async fn a_destination_mapped_to_the_open_cluster_is_read_there() {
    let change = read_mapped(Some(ENV_A), mapped(&[(by_server(), "home")])).await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Cluster("core".into())
    );
    let pods = change
        .hops
        .iter()
        .find(|hop| hop.name == "Pods")
        .expect("the pods are reached");
    assert_eq!(pods.state, HealthIndicator::Healthy, "its pods were read");
    assert!(!change.hops.iter().any(|hop| hop.key.contains("-reported-")));
}

/// What Argo CD reports of a workload nobody read is its claim: Healthy is
/// never more than Info, so the Stage doesn't fold as fine.
#[tokio::test]
async fn a_reported_workload_takes_argo_cds_health_no_higher_than_info() {
    let mut app = application(Some(ENV_A));
    app["status"]["resources"] = json!([
        {"group": "apps", "version": "v1", "kind": "Deployment", "namespace": "shop", "name": "web", "health": {"status": "Healthy"}},
        {"group": "apps", "version": "v1", "kind": "StatefulSet", "namespace": "shop", "name": "db", "health": {"status": "Degraded"}},
        {"group": "apps", "version": "v1", "kind": "DaemonSet", "namespace": "shop", "name": "agent", "health": {"status": "Missing"}},
        {"group": "argoproj.io", "kind": "Rollout", "namespace": "shop", "name": "storefront", "health": {"status": "Progressing"}},
        {"group": "", "kind": "ConfigMap", "namespace": "shop", "name": "settings", "health": {"status": "Healthy"}},
        {"group": "batch", "kind": "Job", "namespace": "shop", "name": "migrate"}
    ]);
    let change = read_app(app, mapped(&[(by_server(), "prod")])).await;
    let reported: Vec<(&str, HealthIndicator)> = change
        .hops
        .iter()
        .filter(|hop| hop.key.starts_with("dev-reported-"))
        .map(|hop| (hop.name.as_str(), hop.state))
        .collect();
    assert_eq!(
        reported,
        vec![
            ("Deployment web", HealthIndicator::Info),
            ("StatefulSet db", HealthIndicator::Warning),
            ("DaemonSet agent", HealthIndicator::Warning),
            ("Rollout storefront", HealthIndicator::Info),
        ],
        "only workloads, each worded as Argo CD's claim"
    );
    assert!(
        change
            .hops
            .iter()
            .any(|hop| hop.group == 2 && hop.state != HealthIndicator::Healthy),
        "a Stage known only from Argo CD's claim isn't all fine: {:#?}",
        change.hops
    );
}

#[test]
fn a_mapping_is_the_workspace_with_the_open_entry() {
    let workspace = freshkube_core::workspace::example();
    let mapping = Mapping::of(&workspace, Some("core-fra"));
    assert_eq!(mapping.open.as_deref(), Some("core-fra"));
    assert_eq!(
        mapping.entries,
        workspace
            .clusters
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(mapping.destinations, workspace.destinations);
    assert!(mapping.settings, "the app's Settings can map them");
    assert_eq!(Mapping::of(&workspace, None).open, None);
}
