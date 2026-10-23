use super::*;

#[test]
fn a_kargo_project_beats_an_argo_application_that_names_it() {
    let mut core = session("core-fra");
    core.kargo = Source::Read(vec![project("checkout", &["dev"])]);
    core.argo_applications = Source::Read(vec![argo(
        "checkout-dev",
        json!({"metadata": {"annotations": {"kargo.akuity.io/authorized-stage": "checkout:dev"}}}),
    )]);
    let found = derived(vec![core]);
    assert_eq!(found.applications.len(), 1);
    let app = &found.applications[0];
    assert_eq!(app.id, id(Rule::Kargo, "checkout"));
    let argo = app
        .members
        .iter()
        .find(|m| m.at.kind == MemberKind::ArgoApplication)
        .expect("the Application is a member");
    assert_eq!(argo.basis, Basis::NamesProject);
    assert_eq!(argo.also_claimed_by, [Rule::ArgoCd]);
    assert!(app.notes.iter().any(|n| matches!(
        n,
        Note::LowerClaim { rule: Rule::ArgoCd, name, .. } if name == "checkout-dev"
    )));
}

#[test]
fn an_application_naming_a_project_nobody_read_stays_with_argo_cd() {
    let mut core = session("core-fra");
    core.kargo = Source::Refused("forbidden".into());
    core.argo_applications = Source::Read(vec![argo(
        "checkout-dev",
        json!({"metadata": {"annotations": {"kargo.akuity.io/authorized-stage": "checkout:dev"}}}),
    )]);
    let found = derived(vec![core]);
    assert!(found.find(&app_id("checkout-dev")).is_some());
    assert!(found.unknown().contains_key(&Rule::Kargo));
}

#[test]
fn a_workload_argo_cd_manages_goes_to_that_application_with_the_label_noted() {
    let mut core = session("core-fra");
    core.argo_applications =
        Source::Read(vec![argo("cart", managing("Deployment", "shop", "web"))]);
    core.workloads = Source::Read(vec![workload("shop", "web", "storefront")]);
    let found = derived(vec![core]);
    assert!(found.find(&id(Rule::PartOf, "storefront")).is_none());
    let cart = found.find(&app_id("cart")).unwrap();
    let web = cart.members.iter().find(|m| m.at.name == "web").unwrap();
    assert_eq!(web.basis, Basis::ManagedBy);
    assert_eq!(web.also_claimed_by, [Rule::PartOf]);
    assert!(cart.notes.iter().any(|n| matches!(
        n,
        Note::LowerClaim { rule: Rule::PartOf, name, .. } if name == "storefront"
    )));
}

#[test]
fn a_managed_workload_that_names_its_application_back_is_tracked() {
    let mut core = session("core-fra");
    core.argo_applications =
        Source::Read(vec![argo("cart", managing("Deployment", "shop", "web"))]);
    let named = |tracking_id: Option<&str>, instance: Option<&str>| {
        let mut web = workload("shop", "web", "storefront");
        web.tracking_id = tracking_id.map(str::to_owned);
        web.instance = instance.map(str::to_owned);
        web
    };
    let basis = |web: LabelledWorkload| {
        let mut core = core.clone();
        core.workloads = Source::Read(vec![web]);
        let found = derived(vec![core]);
        let cart = found.find(&app_id("cart")).unwrap();
        cart.members
            .iter()
            .find(|m| m.at.name == "web")
            .unwrap()
            .basis
    };
    let tracked = named(Some("cart:apps/Deployment:shop/web"), None);
    assert_eq!(basis(tracked), Basis::Tracked);
    assert_eq!(basis(named(None, Some("cart"))), Basis::Tracked);
    assert_eq!(basis(named(None, Some("argocd_cart"))), Basis::Tracked);
    // Another Application's name, or none, leaves the inventory's word alone.
    let other = named(Some("orders:apps/Deployment:shop/web"), Some("orders"));
    assert_eq!(basis(other), Basis::ManagedBy);
    assert_eq!(basis(named(None, None)), Basis::ManagedBy);
}

#[test]
fn a_label_that_is_a_higher_applications_name_joins_it_as_a_guess() {
    let mut core = session("core-fra");
    core.kargo = Source::Read(vec![project("checkout", &["dev"])]);
    core.workloads = Source::Read(vec![workload("shop", "web", "checkout")]);
    let found = derived(vec![core]);
    assert!(found.find(&id(Rule::PartOf, "checkout")).is_none());
    let app = found.find(&id(Rule::Kargo, "checkout")).unwrap();
    let web = app.members.iter().find(|m| m.at.name == "web").unwrap();
    assert_eq!(web.basis, Basis::SameName);
}

#[test]
fn an_application_deploying_to_another_cluster_stays_a_member_unmapped() {
    let mut core = session("core-fra");
    core.argo_applications = Source::Read(vec![
        argo(
            "cart-prod",
            json!({"spec": {"destination": {"server": "https://prod.example.test:6443"}}}),
        ),
        argo(
            "cart-by-name",
            json!({"spec": {"destination": {"server": null, "name": "prod-ams"}}}),
        ),
        argo(
            "cart-local",
            json!({"spec": {"destination": {"server": null, "name": "in-cluster"}}}),
        ),
        argo("cart-nowhere", json!({"spec": {"destination": null}})),
    ]);
    let found = derived(vec![core]);
    let destination = |name: &str| {
        let app = found.find(&app_id(name)).unwrap();
        assert_eq!(app.members[0].at.session, key("core-fra"));
        (app.members[0].destination.clone().unwrap(), app.notes.len())
    };
    assert_eq!(
        destination("cart-prod"),
        (
            Destination::Other {
                server: Some("https://prod.example.test:6443".into()),
                name: None
            },
            1
        )
    );
    assert!(matches!(
        destination("cart-by-name"),
        (Destination::Other { .. }, 1)
    ));
    assert_eq!(destination("cart-local"), (Destination::Local, 0));
    assert_eq!(destination("cart-nowhere"), (Destination::Missing, 0));
}

#[test]
fn an_argo_application_joins_a_project_read_in_another_cluster_as_an_inference() {
    let mut kargo = session("core-fra");
    kargo.kargo = Source::Read(vec![project("checkout", &["dev"])]);
    let mut argo_cluster = session("ops-fra");
    argo_cluster.argo_applications = Source::Read(vec![argo(
        "checkout-dev",
        json!({"metadata": {"annotations": {"kargo.akuity.io/authorized-stage": "checkout:dev"}}}),
    )]);
    let found = derived(vec![kargo, argo_cluster]);
    let app = found.find(&id(Rule::Kargo, "checkout")).unwrap();
    let basis = |name: &str| {
        app.members
            .iter()
            .find(|m| m.at.name == name)
            .unwrap()
            .basis
    };
    assert_eq!(basis("dev"), Basis::Direct, "the Project's own Stage");
    assert_eq!(basis("checkout-dev"), Basis::Inferred);
    assert!(app.notes.iter().any(|n| matches!(
        n,
        Note::JoinedAcrossSessions { sessions, .. }
            if sessions == &[key("core-fra"), key("ops-fra")]
    )));
}

#[test]
fn a_workloads_tracking_annotation_and_instance_label_are_read() {
    let web = read::parse_labelled_workload(
        &json!({"metadata": {
            "namespace": "shop", "name": "web",
            "labels": {"app.kubernetes.io/part-of": "storefront", "app.kubernetes.io/instance": " cart "},
            "annotations": {"argocd.argoproj.io/tracking-id": "cart:apps/Deployment:shop/web"},
        }}),
        WorkloadKind::Deployment,
    )
    .unwrap();
    assert_eq!(web.instance.as_deref(), Some("cart"));
    assert_eq!(
        web.tracking_id.as_deref(),
        Some("cart:apps/Deployment:shop/web")
    );
    let bare = read::parse_labelled_workload(
        &json!({"metadata": {
            "namespace": "shop", "name": "web",
            "labels": {"app.kubernetes.io/part-of": "storefront", "app.kubernetes.io/instance": ""},
        }}),
        WorkloadKind::Deployment,
    )
    .unwrap();
    assert_eq!((bare.instance, bare.tracking_id), (None, None));
}
