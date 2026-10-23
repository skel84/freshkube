use super::*;

#[test]
fn a_kargo_project_is_an_application_of_its_stages_and_warehouses() {
    let mut core = session("core-fra");
    core.kargo = Source::Read(vec![project("checkout", &["dev", "prod"])]);
    let found = derived(vec![core]);
    let app = found.find(&id(Rule::Kargo, "checkout")).expect("checkout");
    assert_eq!(app.name, "checkout");
    assert_eq!(app.rule, Rule::Kargo);
    assert_eq!(names(app), ["dev", "prod", "images"]);
    assert!(app.members.iter().all(|m| m.basis == Basis::Direct));
    assert_eq!(found.applications.len(), 1);
}

#[test]
fn an_application_set_groups_the_applications_it_generated() {
    let mut core = session("core-fra");
    core.argo_applications = Source::Read(vec![
        argo("cart-dev", owned_by("cart")),
        argo("cart-prod", owned_by("cart")),
        argo("catalog", json!({})),
    ]);
    core.argo_application_sets = Source::Read(vec![
        parse_application_set(&json!({"metadata": {"namespace": "argocd", "name": "cart"}}))
            .unwrap(),
    ]);
    let found = derived(vec![core]);
    let cart = found.find(&set_id("cart")).expect("cart");
    assert_eq!(names(cart), ["cart-dev", "cart-prod"]);
    assert!(matches!(
        cart.evidence[0],
        Evidence::ArgoApplicationSet { read: true, .. }
    ));
    let catalog = found.find(&app_id("catalog")).expect("catalog");
    assert!(matches!(
        catalog.evidence[0],
        Evidence::ArgoApplication { .. }
    ));
}

#[test]
fn an_application_set_that_generated_nothing_is_still_an_application() {
    let mut core = session("core-fra");
    core.argo_application_sets = Source::Read(vec![
        parse_application_set(&json!({"metadata": {"namespace": "argocd", "name": "empty"}}))
            .unwrap(),
    ]);
    let found = derived(vec![core]);
    assert!(found.find(&set_id("empty")).unwrap().members.is_empty());
}

#[test]
fn an_owner_reference_names_the_set_even_when_the_sets_were_not_read() {
    let mut core = session("core-fra");
    core.argo_applications = Source::Read(vec![argo("cart-dev", owned_by("cart"))]);
    core.argo_application_sets = Source::Refused("forbidden".into());
    let found = derived(vec![core]);
    let cart = found.find(&set_id("cart")).expect("cart");
    assert!(matches!(
        cart.evidence[0],
        Evidence::ArgoApplicationSet { read: false, .. }
    ));
    assert!(found.unknown().contains_key(&Rule::ArgoCd));
}

#[test]
fn the_part_of_label_groups_workloads_across_clusters_as_an_inference() {
    let mut dev = session("dev-fra");
    dev.workloads = Source::Read(vec![workload("shop", "web", "storefront")]);
    let mut prod = session("prod-fra");
    prod.workloads = Source::Read(vec![
        workload("shop", "web", "storefront"),
        workload("shop", "worker", "storefront"),
        workload("ops", "agent", ""),
    ]);
    let found = derived(vec![dev, prod]);
    let app = found
        .find(&id(Rule::PartOf, "storefront"))
        .expect("storefront");
    assert_eq!(app.members.len(), 3);
    assert_eq!(
        found.applications.len(),
        1,
        "a blank label is no application"
    );
    assert!(app.members.iter().all(|m| m.basis == Basis::Inferred));
    assert!(app.notes.iter().any(|n| matches!(
        n,
        Note::JoinedAcrossSessions { name, sessions }
            if name == "storefront" && sessions == &[key("dev-fra"), key("prod-fra")]
    )));
    let sessions: Vec<_> = app
        .members
        .iter()
        .map(|m| m.at.session.0.as_str())
        .collect();
    assert_eq!(sessions, ["dev-fra", "prod-fra", "prod-fra"]);
}

#[test]
fn the_answer_does_not_depend_on_the_order_of_the_sessions() {
    let build = |reverse: bool| {
        let mut core = session("core-fra");
        core.kargo = Source::Read(vec![
            project("checkout", &["dev"]),
            project("cart", &["dev"]),
        ]);
        let mut prod = session("prod-fra");
        prod.workloads = Source::Read(vec![workload("shop", "web", "storefront")]);
        let mut sessions = vec![core, prod];
        if reverse {
            sessions.reverse();
        }
        derived(sessions)
    };
    let (a, b) = (build(false), build(true));
    assert_eq!(a.applications, b.applications);
    let order: Vec<_> = a.applications.iter().map(|app| app.name.as_str()).collect();
    assert_eq!(order, ["cart", "checkout", "storefront"]);
}

#[test]
fn within_one_cluster_a_shared_label_is_direct_and_needs_no_note() {
    let mut prod = session("prod-fra");
    prod.workloads = Source::Read(vec![
        workload("shop", "web", "storefront"),
        workload("shop", "worker", "storefront"),
    ]);
    let found = derived(vec![prod]);
    let app = found.find(&id(Rule::PartOf, "storefront")).unwrap();
    assert!(app.members.iter().all(|m| m.basis == Basis::Direct));
    assert!(app.notes.is_empty());
}

#[test]
fn argo_cd_applications_of_one_name_in_two_namespaces_stay_two_applications() {
    let mut core = session("core-fra");
    let in_namespace =
        |namespace: &str| argo("checkout", json!({"metadata": {"namespace": namespace}}));
    core.argo_applications = Source::Read(vec![in_namespace("team-a"), in_namespace("team-b")]);
    let found = derived(vec![core]);
    assert_eq!(found.applications.len(), 2);
    assert!(
        found
            .find(&ApplicationId::argo_application("team-a", "checkout"))
            .is_some()
    );
    assert!(
        found
            .find(&ApplicationId::argo_application("team-b", "checkout"))
            .is_some()
    );
}

#[test]
fn an_application_set_and_an_application_of_one_name_stay_apart() {
    let mut core = session("core-fra");
    core.argo_applications = Source::Read(vec![
        argo("checkout", json!({})),
        argo("checkout-dev", owned_by("checkout")),
    ]);
    let found = derived(vec![core]);
    assert!(found.find(&app_id("checkout")).is_some());
    assert!(found.find(&set_id("checkout")).is_some());
    assert_eq!(found.applications.len(), 2);
}

#[test]
fn an_owner_reference_matches_a_set_of_its_own_namespace_only() {
    let mut core = session("core-fra");
    let set = |namespace: &str| {
        parse_application_set(&json!({"metadata": {"namespace": namespace, "name": "cart"}}))
            .unwrap()
    };
    core.argo_applications = Source::Read(vec![argo("cart-dev", owned_by("cart"))]);
    // The Application is in `argocd`; only the other namespace's set was read.
    core.argo_application_sets = Source::Read(vec![set("team-b")]);
    let found = derived(vec![core]);
    let owned = found.find(&set_id("cart")).unwrap();
    assert!(matches!(
        owned.evidence[0],
        Evidence::ArgoApplicationSet { read: false, .. }
    ));
    let other = found
        .find(&ApplicationId::argo_application_set("team-b", "cart"))
        .expect("a set that generated nothing here is still an application");
    assert!(other.members.is_empty());
}
