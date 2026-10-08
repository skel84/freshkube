use serde_json::{Value, json};

use super::super::example::{CORE, acme};
use super::super::read::parse_labelled_workload;
use super::super::*;
use super::*;
use crate::delivery::argocd::parse_application;
use crate::delivery::kargo::{parse_stage, parse_warehouse};
use crate::delivery::observation::Fact;
use crate::delivery::source::Source;
use crate::workloads::WorkloadKind;

fn session(name: &str) -> SessionInputs {
    SessionInputs {
        key: SessionKey::new(name),
        kargo: Source::NotInstalled("API group kargo.akuity.io is not served".into()),
        argo_scope: ArgoScope::AllNamespaces,
        argo_applications: Source::Read(Vec::new()),
        argo_application_sets: Source::Read(Vec::new()),
        workloads: Source::Read(Vec::new()),
    }
}

fn argo(name: &str, metadata: Value) -> crate::delivery::argocd::Application {
    let mut meta = json!({"namespace": "argocd", "name": name});
    if let (Some(into), Some(extra)) = (meta.as_object_mut(), metadata.as_object()) {
        into.extend(extra.clone());
    }
    parse_application(&json!({
        "metadata": meta,
        "spec": {"destination": {"server": "https://kubernetes.default.svc", "namespace": name}},
    }))
    .unwrap()
}

fn labelled(namespace: &str, name: &str, part_of: &str) -> LabelledWorkload {
    parse_labelled_workload(
        &json!({"metadata": {
            "namespace": namespace, "name": name,
            "labels": {"app.kubernetes.io/part-of": part_of},
        }}),
        WorkloadKind::Deployment,
    )
    .unwrap()
}

fn project(name: &str) -> KargoProjectRead {
    KargoProjectRead {
        name: name.into(),
        stages: Source::Read(
            parse_stage(&json!({"metadata": {"namespace": name, "name": "dev"}}))
                .into_iter()
                .collect(),
        ),
        warehouses: Source::Read(
            parse_warehouse(&json!({"metadata": {"namespace": name, "name": "images"}}))
                .into_iter()
                .collect(),
        ),
    }
}

fn of(inputs: &Inputs, name: &str) -> Claims {
    of_with(inputs, &Override::default(), name)
}

fn of_with(inputs: &Inputs, wishes: &Override, name: &str) -> Claims {
    let derived = derive(inputs, wishes);
    let app = derived
        .applications
        .iter()
        .find(|app| app.name == name)
        .unwrap_or_else(|| panic!("no application {name}"));
    claims(&derived, app)
}

fn inputs(sessions: Vec<SessionInputs>) -> Inputs {
    Inputs {
        sessions,
        stage_naming: None,
    }
}

fn link<'a>(claims: &'a Claims, name: &str) -> &'a Claim {
    claims
        .links
        .iter()
        .find(|claim| claim.member.name == name)
        .unwrap_or_else(|| panic!("no link for {name}"))
}

#[test]
fn acme_checkouts_stages_and_warehouse_are_confirmed_and_its_labels_claimed() {
    let checkout = of(&acme(), "checkout");
    let count = |kind: MemberKind, confidence: Confidence| {
        checkout
            .links
            .iter()
            .filter(|c| c.member.kind == kind && c.confidence == confidence)
            .count()
    };
    assert_eq!(count(MemberKind::KargoStage, Confidence::Confirmed), 4);
    assert_eq!(count(MemberKind::KargoWarehouse, Confidence::Confirmed), 1);
    let worker = link(&checkout, "checkout-worker");
    assert_eq!(worker.confidence, Confidence::Claimed);
    assert_eq!(worker.lower, [(Rule::PartOf, "checkout".to_owned())]);
    assert_eq!(worker.member_side.fact, Some(Fact::Declared));
    assert!(checkout.gaps.is_empty());
    // Kind order: Stages, the Warehouse, Applications, then workloads.
    let ranks: Vec<u8> = checkout
        .links
        .iter()
        .map(|c| c.member.kind.rank())
        .collect();
    assert!(ranks.is_sorted());
    let project = &checkout.links[0].app_side;
    assert_eq!(project.text, "Project checkout, read");
    assert_eq!(project.session, Some(SessionKey::new(CORE)));
}

#[test]
fn an_application_naming_a_project_is_claimed() {
    let checkout = of(&acme(), "checkout");
    let dev = link(&checkout, "checkout-dev");
    assert_eq!(dev.confidence, Confidence::Claimed);
    assert_eq!(dev.member_side.fact, Some(Fact::Declared));
}

#[test]
fn an_application_under_a_set_that_was_read_is_confirmed() {
    let catalog = of(&acme(), "catalog");
    let dev = link(&catalog, "catalog-dev");
    assert_eq!(dev.confidence, Confidence::Confirmed);
    assert_eq!(dev.app_side.fact, Some(Fact::Reported));
}

#[test]
fn an_application_under_a_set_nobody_found_is_claimed_and_one_nobody_could_read_unknown() {
    let owned = json!({"ownerReferences": [{
        "kind": "ApplicationSet", "apiVersion": "argoproj.io/v1alpha1", "name": "catalog",
    }]});
    let mut core = session("core-fra");
    core.argo_applications = Source::Read(vec![argo("catalog-dev", owned)]);
    let found = of(&inputs(vec![core.clone()]), "catalog");
    let dev = link(&found, "catalog-dev");
    assert_eq!(dev.confidence, Confidence::Claimed);
    assert_eq!(dev.app_side.fact, None, "the set wasn't read");

    core.argo_application_sets = Source::Refused("forbidden".into());
    let found = of(&inputs(vec![core]), "catalog");
    assert_eq!(link(&found, "catalog-dev").confidence, Confidence::Unknown);
}

#[test]
fn a_plain_application_is_itself_confirmed() {
    let status = of(&acme(), "status-page");
    let app = status
        .links
        .iter()
        .find(|c| c.member.kind == MemberKind::ArgoApplication)
        .unwrap();
    assert_eq!(app.confidence, Confidence::Confirmed);
}

#[test]
fn a_managed_workload_is_confirmed_only_when_it_names_the_application_back() {
    let status = of(&acme(), "status-page");
    let deployment = status
        .links
        .iter()
        .find(|c| matches!(c.member.kind, MemberKind::Workload(_)))
        .unwrap();
    assert_eq!(deployment.confidence, Confidence::Confirmed);
    assert_eq!(
        deployment.lower,
        [(Rule::PartOf, "public-status".to_owned())]
    );

    let mut core = session("core-fra");
    core.argo_applications = Source::Read(vec![
        parse_application(&json!({
            "metadata": {"namespace": "argocd", "name": "cart"},
            "spec": {"destination": {"server": "https://kubernetes.default.svc"}},
            "status": {"resources": [
                {"group": "apps", "kind": "Deployment", "namespace": "shop", "name": "web"}
            ]},
        }))
        .unwrap(),
    ]);
    core.workloads = Source::Read(vec![labelled("shop", "web", "storefront")]);
    let cart = of(&inputs(vec![core]), "cart");
    let web = link(&cart, "web");
    assert_eq!(web.confidence, Confidence::Claimed);
    assert_eq!(web.member_side.fact, Some(Fact::Declared));
}

#[test]
fn a_label_alone_is_claimed() {
    let loyalty = of(&acme(), "loyalty");
    let api = link(&loyalty, "loyalty-api");
    assert_eq!(api.confidence, Confidence::Claimed);
    assert!(api.lower.is_empty());
}

#[test]
fn a_name_joined_across_clusters_is_claimed_and_says_which() {
    let mut dev = session("dev-fra");
    dev.workloads = Source::Read(vec![labelled("loyalty", "loyalty-api", "loyalty")]);
    let mut prod = session("prod-fra");
    prod.workloads = Source::Read(vec![labelled("loyalty", "loyalty-api", "loyalty")]);
    let loyalty = of(&inputs(vec![dev, prod]), "loyalty");
    assert_eq!(loyalty.links.len(), 2);
    for claim in &loyalty.links {
        assert_eq!(claim.confidence, Confidence::Claimed);
        assert_eq!(claim.member_side.fact, Some(Fact::Derived));
        assert!(claim.member_side.text.contains("across 2 clusters"));
    }
}

#[test]
fn a_part_the_override_placed_is_claimed() {
    let derived = derive(&acme(), &Override::default());
    let worker = derived
        .find(&ApplicationId::new(Rule::Kargo, "checkout"))
        .unwrap()
        .members
        .iter()
        .find(|m| m.at.name == "checkout-worker")
        .unwrap()
        .at
        .clone();
    let wishes = Override {
        splits: vec![Split {
            member: worker,
            name: "workers".into(),
        }],
        ..Override::default()
    };
    let workers = of_with(&acme(), &wishes, "workers");
    let claim = &workers.links[0];
    assert_eq!(claim.confidence, Confidence::Claimed);
    assert_eq!(claim.why, "Placed by your override");
}

#[test]
fn a_label_where_argo_cd_was_not_checked_is_claimed_and_says_so() {
    let mut core = session("core-fra");
    core.argo_applications = Source::Refused("forbidden".into());
    core.workloads = Source::Read(vec![labelled("loyalty", "loyalty-api", "loyalty")]);
    let loyalty = of(&inputs(vec![core]), "loyalty");
    let api = link(&loyalty, "loyalty-api");
    // The label was read: the link is the label's, and what could still
    // say otherwise is named, not shown as a side that wasn't read.
    assert_eq!(api.confidence, Confidence::Claimed);
    assert_eq!(api.member_side.fact, Some(Fact::Declared));
    assert_eq!(api.unchecked.as_deref(), Some("Argo CD not checked here"));
    assert!(api.why.contains("one may manage it"), "{}", api.why);
}

#[test]
fn a_label_where_argo_cd_was_read_in_part_says_in_part() {
    let mut core = session("core-fra");
    core.argo_applications = Source::Read(Vec::new());
    core.argo_scope = ArgoScope::Namespace {
        namespaces: vec!["gitops".into()],
        found: ArgoFound::Labelled { skipped: vec![] },
    };
    core.workloads = Source::Read(vec![labelled("loyalty", "loyalty-api", "loyalty")]);
    let loyalty = of(&inputs(vec![core]), "loyalty");
    let api = link(&loyalty, "loyalty-api");
    assert_eq!(api.confidence, Confidence::Claimed);
    assert_eq!(
        api.unchecked.as_deref(),
        Some("Argo CD checked in part here")
    );
}

#[test]
fn a_kargo_project_beside_argo_cd_read_in_part_may_miss_applications() {
    let mut read = acme();
    read.sessions[0].argo_scope = ArgoScope::Namespace {
        namespaces: vec!["gitops".into()],
        found: ArgoFound::Labelled { skipped: vec![] },
    };
    let checkout = of(&read, "checkout");
    let gap = checkout
        .gaps
        .iter()
        .find(|g| g.kinds == [MemberKind::ArgoApplication])
        .expect("a gap for Argo CD Applications");
    assert_eq!(gap.session, SessionKey::new(CORE));
    assert_eq!(
        gap.why,
        "Argo CD Applications its Stages promote to may be missing: read in gitops only"
    );
    // Read in full, there is nothing to miss.
    assert!(
        of(&acme(), "checkout")
            .gaps
            .iter()
            .all(|g| g.kinds != [MemberKind::ArgoApplication])
    );
}

#[test]
fn an_application_naming_a_project_nobody_read_is_unknown() {
    let mut core = session("core-fra");
    core.kargo = Source::Refused("forbidden".into());
    core.argo_applications = Source::Read(vec![argo(
        "checkout-dev",
        json!({"annotations": {"kargo.akuity.io/authorized-stage": "checkout:dev"}}),
    )]);
    let found = of(&inputs(vec![core]), "checkout-dev");
    assert_eq!(link(&found, "checkout-dev").confidence, Confidence::Unknown);
}

#[test]
fn stages_that_did_not_answer_are_a_gap_not_an_empty_group() {
    let mut core = session("core-fra");
    let mut checkout = project("checkout");
    checkout.stages = Source::Unreadable("timed out".into());
    core.kargo = Source::Read(vec![checkout]);
    let found = of(&inputs(vec![core]), "checkout");
    assert!(
        found
            .links
            .iter()
            .all(|c| c.member.kind != MemberKind::KargoStage)
    );
    assert_eq!(found.gaps.len(), 1);
    let gap = &found.gaps[0];
    assert_eq!(gap.session, SessionKey::new("core-fra"));
    assert_eq!(gap.kinds[0], MemberKind::KargoStage);
    assert!(gap.why.contains("timed out"), "{}", gap.why);
}

#[test]
fn an_unread_cluster_gives_only_unknown() {
    let mut read = acme();
    read.sessions.push(SessionInputs::unread(
        SessionKey::new("edge-fra"),
        "connection refused",
    ));
    let checkout = of(&read, "checkout");
    assert!(
        checkout
            .links
            .iter()
            .all(|c| c.member.session != SessionKey::new("edge-fra"))
    );
    assert_eq!(checkout.gaps.len(), 1);
    let gap = &checkout.gaps[0];
    assert_eq!(gap.session, SessionKey::new("edge-fra"));
    assert!(matches!(gap.kinds[0], MemberKind::Workload(_)));
    assert!(gap.why.contains("connection refused"));
}

#[test]
fn a_read_of_one_namespace_is_no_gap() {
    let mut read = acme();
    read.sessions[0].argo_scope = ArgoScope::Namespace {
        namespaces: vec!["argocd".into()],
        found: ArgoFound::Default,
    };
    let catalog = of(&read, "catalog");
    assert!(catalog.gaps.is_empty());
    assert!(
        catalog
            .links
            .iter()
            .filter(|c| c.member.kind == MemberKind::ArgoApplication)
            .all(|c| c.confidence == Confidence::Confirmed)
    );
}
