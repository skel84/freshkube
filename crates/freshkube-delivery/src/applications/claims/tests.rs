use serde_json::{Value, json};

use super::super::example::{CICD, CORE, acme};
use super::super::read::parse_labelled_workload;
use super::super::*;
use super::*;
use crate::argocd::parse_application;
use crate::kargo::{parse_stage, parse_warehouse};
use crate::observation::Fact;
use crate::source::Source;
use freshkube_core::workloads::WorkloadKind;

fn session(name: &str) -> SessionInputs {
    SessionInputs {
        key: SessionKey::new(name),
        kargo: Source::NotInstalled("API group kargo.akuity.io is not served".into()),
        argo_scope: ArgoScope::AllNamespaces,
        argo_applications: Source::Read(Vec::new()),
        argo_application_sets: Source::Read(Vec::new()),
        workloads: Source::Read(Vec::new()),
        asked: crate::applications::Asked::ALL,
        absent: None,
    }
}

fn argo(name: &str, metadata: Value) -> crate::argocd::Application {
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
        destinations: Vec::new(),
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
    // Kargo is read from the CI/CD entry.
    assert_eq!(project.session, Some(SessionKey::new(CICD)));
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
    // An absence found by reading the object: derived, not declared.
    assert_eq!(web.member_side.fact, Some(Fact::Derived));
}

/// An Application in `core-fra` that lists Deployment shop/web, with
/// `metadata` merged in, and that workload naming it back by its instance
/// label.
fn listing_web(name: &str, metadata: Value, destination: Value) -> SessionInputs {
    let mut meta = json!({"namespace": "argocd", "name": name});
    if let (Some(into), Some(extra)) = (meta.as_object_mut(), metadata.as_object()) {
        into.extend(extra.clone());
    }
    let mut core = session("core-fra");
    core.argo_applications = Source::Read(vec![
        parse_application(&json!({
            "metadata": meta,
            "spec": {"destination": destination},
            "status": {"resources": [
                {"group": "apps", "kind": "Deployment", "namespace": "shop", "name": "web"}
            ]},
        }))
        .unwrap(),
    ]);
    core.workloads = Source::Read(vec![
        parse_labelled_workload(
            &json!({"metadata": {
                "namespace": "shop", "name": "web",
                "labels": {
                    "app.kubernetes.io/part-of": "storefront",
                    "app.kubernetes.io/instance": name,
                },
            }}),
            WorkloadKind::Deployment,
        )
        .unwrap(),
    ]);
    core
}

fn in_cluster() -> Value {
    json!({"server": "https://kubernetes.default.svc"})
}

#[test]
fn a_tracked_workload_is_no_surer_than_the_application_that_lists_it() {
    // checkout-dev names Project checkout by annotation alone: Claimed. A
    // workload naming checkout-dev back can't be Confirmed on checkout.
    let mut core = listing_web(
        "checkout-dev",
        json!({"annotations": {"kargo.akuity.io/authorized-stage": "checkout:dev"}}),
        in_cluster(),
    );
    core.kargo = Source::Read(vec![project("checkout")]);
    let checkout = of(&inputs(vec![core]), "checkout");
    assert_eq!(
        link(&checkout, "checkout-dev").confidence,
        Confidence::Claimed
    );
    let web = link(&checkout, "web");
    assert_eq!(web.confidence, Confidence::Claimed);
    assert!(
        web.why
            .contains("but that Application's own link is claimed"),
        "{}",
        web.why
    );
    assert_eq!(
        web.app_side.text,
        "Argo CD Application argocd/checkout-dev's inventory lists it"
    );
    assert_eq!(web.app_side.session, Some(SessionKey::new("core-fra")));

    // On the Application's own application, it is Confirmed.
    let core = listing_web("cart", json!({}), in_cluster());
    let cart = of(&inputs(vec![core]), "cart");
    assert_eq!(link(&cart, "web").confidence, Confidence::Confirmed);
}

#[test]
fn an_application_deploying_elsewhere_confirms_nothing_here() {
    let core = listing_web(
        "cart",
        json!({}),
        json!({"server": "https://edge.example.test:6443"}),
    );
    let derived = derive(&inputs(vec![core]), &Override::default());
    let storefront = derived
        .applications
        .iter()
        .find(|app| app.name == "storefront")
        .expect("the workload stays with its label");
    assert!(
        storefront
            .members
            .iter()
            .all(|m| m.basis != Basis::Tracked && m.basis != Basis::ManagedBy)
    );
}

#[test]
fn a_tracking_id_copied_from_another_object_does_not_name_back() {
    let mut core = listing_web("cart", json!({}), in_cluster());
    core.workloads = Source::Read(vec![
        parse_labelled_workload(
            &json!({"metadata": {
                "namespace": "shop", "name": "web",
                "labels": {"app.kubernetes.io/part-of": "storefront"},
                "annotations": {
                    "argocd.argoproj.io/tracking-id": "cart:apps/Deployment:shop/api",
                },
            }}),
            WorkloadKind::Deployment,
        )
        .unwrap(),
    ]);
    let cart = of(&inputs(vec![core.clone()]), "cart");
    assert_eq!(link(&cart, "web").confidence, Confidence::Claimed);
    // Naming this object, it does.
    core.workloads = Source::Read(vec![
        parse_labelled_workload(
            &json!({"metadata": {
                "namespace": "shop", "name": "web",
                "labels": {"app.kubernetes.io/part-of": "storefront"},
                "annotations": {
                    "argocd.argoproj.io/tracking-id": "cart:apps/Deployment:shop/web",
                },
            }}),
            WorkloadKind::Deployment,
        )
        .unwrap(),
    ]);
    let cart = of(&inputs(vec![core]), "cart");
    assert_eq!(link(&cart, "web").confidence, Confidence::Confirmed);
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
    // prod-lon's Argo CD Applications were read on core-fra, and none is
    // matched to its workloads: the override says where, and the caveat
    // stays.
    assert!(
        claim
            .why
            .starts_with("Placed by your override. No Argo CD Application mapped"),
        "{}",
        claim.why
    );
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
    assert_eq!(api.unchecked, Some(Unchecked::NotRead));
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
        api.unchecked,
        Some(Unchecked::Namespaces(vec!["gitops".into()]))
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
fn an_unread_cluster_gives_gaps_and_no_parts() {
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
    // Its Applications may name checkout's Project, and its workloads may
    // carry the label: both may be missing.
    assert_eq!(checkout.gaps.len(), 2, "{:?}", checkout.gaps);
    for gap in &checkout.gaps {
        assert_eq!(gap.session, SessionKey::new("edge-fra"));
        assert!(gap.why.contains("connection refused"));
    }
    assert_eq!(checkout.gaps[0].kinds, [MemberKind::ArgoApplication]);
    assert!(matches!(checkout.gaps[1].kinds[0], MemberKind::Workload(_)));
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

/// A labelled part on an entry Argo CD wasn't asked of: its Argo CD is the
/// one read on its own entry, named, and never the last word until its
/// inventories are matched to that cluster.
#[test]
fn a_label_on_another_entry_says_how_argo_cd_was_read_on_its_own() {
    let loyalty = of(&acme(), "loyalty");
    let api = link(&loyalty, "loyalty-api");
    assert_eq!(api.member.session, SessionKey::new("dev-fra"));
    assert_eq!(api.argo_on, Some(SessionKey::new(CORE)));
    // Read in full on core-fra, its inventories still aren't matched to
    // dev-fra.
    assert_eq!(api.unchecked, Some(Unchecked::NotMatched));
    assert!(
        api.why
            .contains("No Argo CD Application mapped to its cluster lists it")
            && !api.why.contains("weren't all read"),
        "{}",
        api.why
    );

    // Read in argocd only there, it says so.
    let mut inputs = acme();
    let core = inputs
        .sessions
        .iter_mut()
        .find(|s| s.key.0 == CORE)
        .unwrap();
    core.argo_scope = ArgoScope::Namespace {
        namespaces: vec!["argocd".into()],
        found: ArgoFound::Default,
    };
    let api = link(&of(&inputs, "loyalty"), "loyalty-api").clone();
    assert_eq!(
        api.unchecked,
        Some(Unchecked::Namespaces(vec!["argocd".into()]))
    );

    // Refused there, it wasn't read for dev-fra either.
    let mut inputs = acme();
    let core = inputs
        .sessions
        .iter_mut()
        .find(|s| s.key.0 == CORE)
        .unwrap();
    core.argo_applications = Source::Refused("forbidden".into());
    let loyalty = of(&inputs, "loyalty");
    let api = link(&loyalty, "loyalty-api");
    assert_eq!(api.argo_on, Some(SessionKey::new(CORE)));
    assert_eq!(api.unchecked, Some(Unchecked::NotRead));
    assert!(api.why.contains("one may manage it"), "{}", api.why);
}

/// An Application deploys to an entry whose labelled workloads weren't
/// read: its application says its parts there may be missing.
#[test]
fn an_application_deploying_to_an_unread_entry_may_be_missing_parts() {
    let mut inputs = acme();
    let dev = inputs
        .sessions
        .iter_mut()
        .find(|s| s.key.0 == "dev-fra")
        .unwrap();
    *dev = SessionInputs::absent(
        dev.key.clone(),
        dev.asked,
        Absent::Unreachable("connection refused".into()),
    );
    let derived = derive(&inputs, &Override::default());
    for name in ["cart", "checkout"] {
        let app = derived
            .applications
            .iter()
            .find(|a| a.name == name)
            .unwrap();
        assert!(
            app.notes.iter().any(|note| matches!(
                note,
                Note::DestinationUnread { session, why, .. }
                    if session.0 == "dev-fra" && why == "couldn't be reached: connection refused"
            )),
            "{name}: {:?}",
            app.notes
        );
    }
    // Read, the entry leaves no such note, and a mapped destination isn't
    // said to be unmapped.
    let derived = derive(&acme(), &Override::default());
    let cart = derived
        .applications
        .iter()
        .find(|a| a.name == "cart")
        .unwrap();
    assert!(
        !cart.notes.iter().any(|note| matches!(
            note,
            Note::DestinationUnread { member, .. } | Note::UnmappedDestination { member }
                if member.name.ends_with("-dev")
        )),
        "{:?}",
        cart.notes
    );
}

/// An Application read on Argo CD's entry manages what its inventory lists
/// on the entry its destination is mapped to; on a cluster no row maps, its
/// inventory names nothing the workspace read there.
#[test]
fn an_inventory_is_matched_on_the_entry_its_destination_maps_to() {
    let derived = derive(&acme(), &Override::default());
    let part = |app: &str, session: &str, name: &str| {
        let app = derived.applications.iter().find(|a| a.name == app).unwrap();
        let member = app
            .members
            .iter()
            .find(|m| m.at.session.0 == session && m.at.name == name)
            .unwrap_or_else(|| panic!("{name} on {session}"))
            .clone();
        let caveat = app
            .notes
            .iter()
            .any(|note| matches!(note, Note::ManagerUnknown { member: at } if *at == member.at));
        (member, caveat)
    };
    let (api, caveat) = part("cart", "dev-fra", "cart-api");
    assert_eq!(api.basis, Basis::Tracked);
    assert_eq!(
        api.via.as_ref().map(|via| via.name.as_str()),
        Some("cart-dev")
    );
    assert_eq!(api.via.map(|via| via.session), Some(SessionKey::new(CORE)));
    assert!(!caveat, "its manager was found");
    // Through an Application that only names its Project, it is no surer
    // than that Application, whichever entry it runs on.
    let api = link(&of(&acme(), "cart"), "cart-api").clone();
    assert_eq!(api.member.session, SessionKey::new("dev-fra"));
    assert_eq!(api.confidence, Confidence::Claimed);
    assert!(
        api.why
            .contains("but that Application's own link is claimed"),
        "{}",
        api.why
    );
    let (_, caveat) = part("catalog", "prod-lon", "catalog-api");
    assert!(caveat, "prod-lon isn't mapped");
    let (_, caveat) = part("loyalty", "dev-fra", "loyalty-api");
    assert!(caveat, "no inventory lists it");
}

/// Argo CD on `core`, its Application `storefront` deploying to `dev`'s
/// server and listing Deployment shop/web; `dev` read for its labelled
/// workloads alone, with `web` there carrying `labels`.
fn storefront_across(labels: Value, mapped: bool) -> Inputs {
    const DEV: &str = "https://dev.example.test:6443";
    let mut core = session("core");
    core.argo_applications = Source::Read(vec![
        parse_application(&json!({
            "metadata": {"namespace": "argocd", "name": "storefront"},
            "spec": {"destination": {"server": DEV}},
            "status": {"resources": [
                {"group": "apps", "kind": "Deployment", "namespace": "shop", "name": "web"}
            ]},
        }))
        .unwrap(),
    ]);
    let mut dev = SessionInputs::asking(
        SessionKey::new("dev"),
        Asked {
            workloads: true,
            ..Asked::NONE
        },
    );
    dev.workloads = Source::Read(vec![
        parse_labelled_workload(
            &json!({"metadata": {"namespace": "shop", "name": "web", "labels": labels}}),
            WorkloadKind::Deployment,
        )
        .unwrap(),
    ]);
    Inputs {
        destinations: match mapped {
            true => vec![freshkube_core::workspace::Destination::new(
                freshkube_core::workspace::Key::Server(DEV.into()),
                "dev",
            )],
            false => Vec::new(),
        },
        ..inputs(vec![core, dev])
    }
}

/// A workload matched to an Argo CD Application on another entry has the
/// confidence it would have beside it: Confirmed when it names the
/// Application back, Claimed when only the inventory lists it, and nothing
/// from the inventory when the destination isn't mapped to its entry.
#[test]
fn a_workload_matched_across_entries_is_as_sure_as_beside_its_application() {
    let web = |labels: Value, mapped: bool| {
        let inputs = storefront_across(labels, mapped);
        let derived = derive(&inputs, &Override::default());
        let app = derived
            .applications
            .iter()
            .find(|app| app.name == "storefront")
            .expect("storefront");
        let basis = app
            .members
            .iter()
            .find(|member| member.at.name == "web")
            .map(|member| member.basis);
        let claim = claims(&derived, app)
            .links
            .into_iter()
            .find(|claim| claim.member.name == "web");
        (basis, claim)
    };
    let named_back = json!({
        "app.kubernetes.io/part-of": "storefront",
        "app.kubernetes.io/instance": "storefront",
    });
    let (basis, claim) = web(named_back.clone(), true);
    assert_eq!(basis, Some(Basis::Tracked));
    let claim = claim.expect("web's link");
    assert_eq!(claim.member.session, SessionKey::new("dev"));
    assert_eq!(claim.confidence, Confidence::Confirmed, "{}", claim.why);
    assert_eq!(claim.app_side.session, Some(SessionKey::new("core")));

    let (basis, claim) = web(json!({"app.kubernetes.io/part-of": "storefront"}), true);
    assert_eq!(basis, Some(Basis::ManagedBy));
    assert_eq!(claim.expect("web's link").confidence, Confidence::Claimed);

    let (basis, _) = web(named_back, false);
    assert!(
        !matches!(basis, Some(Basis::Tracked | Basis::ManagedBy)),
        "{basis:?}"
    );
}
