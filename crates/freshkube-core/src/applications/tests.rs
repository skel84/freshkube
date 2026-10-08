use serde_json::{Value, json};

use super::*;
use crate::delivery::argocd::{parse_application, parse_application_set};
use crate::delivery::kargo::{parse_stage, parse_warehouse};

fn key(name: &str) -> SessionKey {
    SessionKey::new(name)
}

fn stage(project: &str, name: &str) -> Stage {
    parse_stage(&json!({"metadata": {"namespace": project, "name": name}})).unwrap()
}

fn warehouse(project: &str, name: &str) -> Warehouse {
    parse_warehouse(&json!({"metadata": {"namespace": project, "name": name}})).unwrap()
}

fn project(name: &str, stages: &[&str]) -> KargoProjectRead {
    KargoProjectRead {
        name: name.into(),
        stages: Source::Read(stages.iter().map(|s| stage(name, s)).collect()),
        warehouses: Source::Read(vec![warehouse(name, "images")]),
    }
}

fn argo(name: &str, extra: Value) -> ArgoApplication {
    let mut value = json!({
        "metadata": {"namespace": "argocd", "name": name},
        "spec": {"destination": {"server": "https://kubernetes.default.svc", "namespace": name}},
    });
    merge(&mut value, extra);
    parse_application(&value).unwrap()
}

fn merge(into: &mut Value, extra: Value) {
    match (into, extra) {
        (Value::Object(into), Value::Object(extra)) => {
            for (k, v) in extra {
                merge(into.entry(k).or_insert(Value::Null), v);
            }
        }
        (into, extra) => *into = extra,
    }
}

fn owned_by(set: &str) -> Value {
    json!({"metadata": {"ownerReferences": [
        {"kind": "ApplicationSet", "apiVersion": "argoproj.io/v1alpha1", "name": set}
    ]}})
}

fn managing(kind: &str, namespace: &str, name: &str) -> Value {
    json!({"status": {"resources": [
        {"group": "apps", "kind": kind, "namespace": namespace, "name": name}
    ]}})
}

fn workload(namespace: &str, name: &str, part_of: &str) -> LabelledWorkload {
    LabelledWorkload {
        namespace: namespace.into(),
        name: name.into(),
        kind: WorkloadKind::Deployment,
        part_of: part_of.into(),
        tracking_id: None,
        instance: None,
    }
}

fn session(name: &str) -> SessionInputs {
    SessionInputs {
        key: key(name),
        kargo: Source::NotInstalled("API group kargo.akuity.io is not served".into()),
        argo_scope: ArgoScope::AllNamespaces,
        argo_applications: Source::Read(Vec::new()),
        argo_application_sets: Source::Read(Vec::new()),
        workloads: Source::Read(Vec::new()),
    }
}

fn derived(sessions: Vec<SessionInputs>) -> Derived {
    derive(
        &Inputs {
            sessions,
            stage_naming: None,
        },
        &Override::default(),
    )
}

fn id(rule: Rule, name: &str) -> ApplicationId {
    ApplicationId::new(rule, name)
}

fn set_id(name: &str) -> ApplicationId {
    ApplicationId::argo_application_set("argocd", name)
}

fn app_id(name: &str) -> ApplicationId {
    ApplicationId::argo_application("argocd", name)
}

fn names(application: &Application) -> Vec<&str> {
    application
        .members
        .iter()
        .map(|m| m.at.name.as_str())
        .collect()
}

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
fn an_unread_source_is_unknown_and_not_installed_is_a_fact() {
    let mut core = session("core-fra");
    core.kargo = Source::NotInstalled("not served".into());
    core.argo_applications = Source::Refused("forbidden".into());
    core.argo_application_sets = Source::Unreadable("timed out".into());
    core.workloads = Source::Capped(
        vec![workload("shop", "web", "storefront")],
        Truncation { read: 1 },
    );
    let found = derived(vec![
        core,
        SessionInputs::unread(key("prod-fra"), "no route"),
    ]);
    let unknown = found.unknown();
    assert_eq!(
        unknown[&Rule::Kargo],
        [key("prod-fra")],
        "not installed is no gap"
    );
    assert_eq!(unknown[&Rule::ArgoCd], [key("core-fra"), key("prod-fra")]);
    assert_eq!(unknown[&Rule::PartOf], [key("core-fra"), key("prod-fra")]);
    // What was read is still there.
    assert_eq!(found.applications.len(), 1);
    assert!(
        found
            .coverage
            .iter()
            .any(|c| c.session == key("core-fra") && c.state == CoverageState::Capped(1))
    );
}

#[test]
fn a_projects_unread_stages_leave_it_standing_with_a_note() {
    let mut core = session("core-fra");
    let mut shop = project("checkout", &[]);
    shop.stages = Source::Refused("forbidden".into());
    core.kargo = Source::Read(vec![shop]);
    let found = derived(vec![core]);
    let app = found.find(&id(Rule::Kargo, "checkout")).unwrap();
    assert!(
        app.notes
            .iter()
            .any(|n| matches!(n, Note::MembersUnknown { why, .. } if why.starts_with("Stages")))
    );
    assert!(found.coverage.iter().any(|c| {
        c.source == SourceKind::KargoStages
            && c.project.as_deref() == Some("checkout")
            && matches!(c.state, CoverageState::Refused(_))
    }));
    assert!(found.unknown().contains_key(&Rule::Kargo));
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

fn with_override(over: Override) -> Derived {
    let mut core = session("core-fra");
    core.kargo = Source::Read(vec![project("checkout", &["dev"])]);
    core.argo_applications = Source::Read(vec![argo("catalog", json!({}))]);
    core.workloads = Source::Read(vec![workload("shop", "web", "storefront")]);
    derive(
        &Inputs {
            sessions: vec![core],
            stage_naming: None,
        },
        &over,
    )
}

#[test]
fn a_rename_changes_the_name_and_keeps_the_id() {
    let over = Override {
        renames: [(id(Rule::Kargo, "checkout"), "  Shop checkout ".to_owned())].into(),
        ..Override::default()
    };
    let found = with_override(over);
    let app = found.find(&id(Rule::Kargo, "checkout")).unwrap();
    assert_eq!(app.name, "Shop checkout");
    assert!(app.evidence.contains(&Evidence::Override("rename")));
}

#[test]
fn a_merge_moves_members_and_notes_where_they_came_from() {
    let over = Override {
        merges: vec![Merge {
            from: vec![app_id("catalog"), id(Rule::PartOf, "storefront")],
            into: id(Rule::Kargo, "checkout"),
        }],
        ..Override::default()
    };
    let found = with_override(over);
    assert_eq!(found.applications.len(), 1);
    let app = &found.applications[0];
    assert_eq!(names(app), ["dev", "images", "catalog", "web"]);
    assert!(
        app.notes
            .iter()
            .any(|n| matches!(n, Note::Merged { from } if from.len() == 2))
    );
    assert!(found.unmatched.is_empty());
}

#[test]
fn a_split_takes_one_member_into_an_application_of_its_own() {
    let member = MemberRef {
        session: key("core-fra"),
        kind: MemberKind::KargoWarehouse,
        namespace: Some("checkout".into()),
        name: "images".into(),
    };
    let over = Override {
        splits: vec![Split {
            member: member.clone(),
            name: "checkout images".into(),
        }],
        ..Override::default()
    };
    let found = with_override(over);
    let split = found.find(&id(Rule::Manual, "checkout images")).unwrap();
    assert_eq!(split.members[0].at, member);
    assert_eq!(split.members[0].basis, Basis::Override);
    let checkout = found.find(&id(Rule::Kargo, "checkout")).unwrap();
    assert_eq!(names(checkout), ["dev"]);
}

#[test]
fn a_hidden_application_is_kept_aside_to_show_again() {
    let over = Override {
        hidden: [app_id("catalog")].into(),
        ..Override::default()
    };
    let found = with_override(over);
    assert!(found.find(&app_id("catalog")).is_none());
    assert_eq!(found.hidden[0].id, app_id("catalog"));
}

#[test]
fn an_override_that_matches_nothing_is_reported_not_dropped() {
    let gone = id(Rule::Kargo, "retired");
    let member = MemberRef {
        session: key("core-fra"),
        kind: MemberKind::KargoStage,
        namespace: Some("retired".into()),
        name: "dev".into(),
    };
    let over = Override {
        renames: [(gone.clone(), "x".to_owned())].into(),
        merges: vec![
            Merge {
                from: vec![gone.clone()],
                into: id(Rule::Kargo, "checkout"),
            },
            Merge {
                from: vec![app_id("catalog")],
                into: gone.clone(),
            },
        ],
        splits: vec![Split {
            member: member.clone(),
            name: "y".into(),
        }],
        hidden: [gone.clone()].into(),
    };
    let found = with_override(over);
    assert_eq!(
        found.unmatched,
        [
            Unmatched::Split(member),
            Unmatched::MergeFrom(gone.clone()),
            Unmatched::MergeInto(gone.clone()),
            Unmatched::Rename(gone.clone()),
            Unmatched::Hide(gone),
        ]
    );
    assert!(
        found.find(&app_id("catalog")).is_some(),
        "an unmatched merge moves nothing"
    );
}

#[test]
fn an_override_round_trips_through_json() {
    let over = Override {
        renames: [(id(Rule::Kargo, "checkout"), "Shop".to_owned())].into(),
        merges: vec![Merge {
            from: vec![id(Rule::PartOf, "storefront")],
            into: id(Rule::Kargo, "checkout"),
        }],
        splits: vec![Split {
            member: MemberRef {
                session: key("core-fra"),
                kind: MemberKind::Workload(WorkloadKind::StatefulSet),
                namespace: Some("shop".into()),
                name: "db".into(),
            },
            name: "db".into(),
        }],
        hidden: [app_id("catalog")].into(),
    };
    let text = serde_json::to_string(&over).unwrap();
    assert_eq!(serde_json::from_str::<Override>(&text).unwrap(), over);
}

mod reading {
    use serde_json::json;

    use super::*;
    use crate::applications::read::{MAX_ARGO_NAMESPACES, MAX_PROJECTS, read_session};
    use crate::delivery::fixtures::FixtureReader;

    fn world() -> FixtureReader {
        let namespaced = |ns: &str, name: &str, labels: Value| json!({"metadata": {"namespace": ns, "name": name, "labels": labels}});
        FixtureReader::default()
            .serves(
                "kargo.akuity.io",
                "v1alpha1",
                &["projects", "stages", "warehouses"],
            )
            .serves(
                "argoproj.io",
                "v1alpha1",
                &["applications", "applicationsets"],
            )
            .with("projects", vec![json!({"metadata": {"name": "checkout"}})])
            .with("stages", vec![namespaced("checkout", "dev", json!({}))])
            .with(
                "warehouses",
                vec![namespaced("checkout", "images", json!({}))],
            )
            .with(
                "applications",
                vec![json!({"metadata": {"namespace": "argocd", "name": "storefront"}})],
            )
            .with(
                "applicationsets",
                vec![json!({"metadata": {"namespace": "argocd", "name": "catalog"}})],
            )
            .with(
                "deployments",
                vec![
                    namespaced(
                        "shop",
                        "web",
                        json!({"app.kubernetes.io/part-of": "storefront"}),
                    ),
                    namespaced("shop", "plain", json!({"app.kubernetes.io/name": "plain"})),
                ],
            )
            .with(
                "statefulsets",
                vec![namespaced(
                    "shop",
                    "db",
                    json!({"app.kubernetes.io/part-of": "storefront"}),
                )],
            )
    }

    #[tokio::test]
    async fn a_session_reads_every_source_and_derives_from_them() {
        let reader = world();
        let read = read_session(&reader, key("core-fra")).await;
        let found = derive(
            &Inputs {
                sessions: vec![read],
                stage_naming: None,
            },
            &Override::default(),
        );
        let found_names: Vec<_> = found.applications.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(found_names, ["catalog", "checkout", "storefront"]);
        let storefront = found.find(&app_id("storefront")).unwrap();
        assert_eq!(
            names(storefront),
            ["storefront", "web", "db"],
            "the label matches the name, kinds in order"
        );
        assert_eq!(
            found.unknown().keys().copied().collect::<Vec<_>>(),
            [Rule::ArgoCd],
            "only the Argo CD namespace was listed"
        );
    }

    #[tokio::test]
    async fn reads_are_only_gets_and_scoped_lists() {
        let reader = world();
        read_session(&reader, key("core-fra")).await;
        let requests = reader.requests.borrow();
        assert!(
            requests
                .iter()
                .all(|r| r.starts_with("GET /apis") || r.starts_with("LIST "))
        );
        assert!(
            requests
                .iter()
                .filter(|r| r.contains("/deployments"))
                .all(|r| r.contains("selector=Some(")),
            "cluster-wide lists carry a selector: {requests:?}"
        );
        assert!(
            requests
                .iter()
                .any(|r| r.contains("applications ns=Some(\"argocd\")"))
        );
    }

    #[tokio::test]
    async fn sources_that_are_not_served_or_are_refused_are_told_apart() {
        let bare = FixtureReader::default().refusing("deployments");
        let read = read_session(&bare, key("core-fra")).await;
        assert!(matches!(read.kargo, Source::NotInstalled(_)));
        assert!(matches!(read.argo_applications, Source::NotInstalled(_)));
        assert!(matches!(read.workloads, Source::Refused(_)));
        let found = derive(
            &Inputs {
                sessions: vec![read],
                stage_naming: None,
            },
            &Override::default(),
        );
        assert!(found.applications.is_empty());
        assert_eq!(
            found.unknown().keys().copied().collect::<Vec<_>>(),
            [Rule::PartOf]
        );
    }

    #[tokio::test]
    async fn projects_past_the_cap_are_not_read_and_say_so() {
        let projects = (0..MAX_PROJECTS + 2)
            .map(|n| json!({"metadata": {"name": format!("p{n:03}")}}))
            .collect();
        let reader = FixtureReader::default()
            .serves(
                "kargo.akuity.io",
                "v1alpha1",
                &["projects", "stages", "warehouses"],
            )
            .with("projects", projects);
        let read = read_session(&reader, key("core-fra")).await;
        let Source::Capped(projects, truncation) = read.kargo else {
            panic!("expected a capped read");
        };
        assert_eq!(
            (projects.len(), truncation.read),
            (MAX_PROJECTS, MAX_PROJECTS)
        );
        let stage_lists = reader
            .requests
            .borrow()
            .iter()
            .filter(|r| r.contains("/stages"))
            .count();
        assert_eq!(stage_lists, MAX_PROJECTS);
    }

    /// Argo CD's own workloads, labelled as its install labels them, in
    /// each namespace given, with an Application in each.
    fn argo_in(namespaces: &[&str]) -> FixtureReader {
        let workload = |ns: &str| {
            json!({"metadata": {"namespace": ns, "name": "argocd-server",
                "labels": {"app.kubernetes.io/part-of": "argocd"}}})
        };
        let application =
            |ns: &str| json!({"metadata": {"namespace": ns, "name": format!("shop-{ns}")}});
        FixtureReader::default()
            .serves(
                "argoproj.io",
                "v1alpha1",
                &["applications", "applicationsets"],
            )
            .with(
                "deployments",
                namespaces.iter().map(|ns| workload(ns)).collect(),
            )
            .with(
                "applications",
                namespaces.iter().map(|ns| application(ns)).collect(),
            )
    }

    fn application_names(read: &SessionInputs) -> Vec<String> {
        let mut names: Vec<String> = read
            .argo_applications
            .read()
            .into_iter()
            .flatten()
            .map(|a| a.name.clone())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn argo_cd_is_read_where_its_workloads_run() {
        let reader = argo_in(&["gitops"]);
        let read = read_session(&reader, key("core-fra")).await;
        assert_eq!(
            read.argo_scope,
            ArgoScope::Namespace {
                namespaces: vec!["gitops".into()],
                found: ArgoFound::Labelled { skipped: vec![] },
            }
        );
        assert_eq!(application_names(&read), ["shop-gitops"]);
        let requests = reader.requests.borrow();
        assert!(
            !requests.iter().any(|r| r.contains("ns=Some(\"argocd\")")),
            "the default namespace isn't read when Argo CD was found: {requests:?}"
        );
    }

    #[tokio::test]
    async fn argo_cd_in_two_namespaces_is_read_in_both() {
        let read = read_session(&argo_in(&["platform", "gitops"]), key("core-fra")).await;
        let ArgoScope::Namespace { namespaces, found } = &read.argo_scope else {
            panic!("expected a namespaced read");
        };
        assert_eq!(namespaces, &["gitops", "platform"], "by name");
        assert_eq!(found, &ArgoFound::Labelled { skipped: vec![] });
        assert_eq!(application_names(&read), ["shop-gitops", "shop-platform"]);
    }

    #[tokio::test]
    async fn with_nothing_labelled_the_default_namespace_is_read() {
        let read = read_session(&world(), key("core-fra")).await;
        assert_eq!(
            read.argo_scope,
            ArgoScope::Namespace {
                namespaces: vec!["argocd".into()],
                found: ArgoFound::Default,
            }
        );
        assert_eq!(application_names(&read), ["storefront"]);
    }

    #[tokio::test]
    async fn namespaces_past_the_cap_are_named_and_not_read() {
        let reader = argo_in(&["e-ops", "a-ops", "d-ops", "b-ops", "c-ops"]);
        let read = read_session(&reader, key("core-fra")).await;
        let ArgoScope::Namespace { namespaces, found } = &read.argo_scope else {
            panic!("expected a namespaced read");
        };
        assert_eq!(namespaces.len(), MAX_ARGO_NAMESPACES);
        assert_eq!(namespaces, &["a-ops", "b-ops", "c-ops"]);
        assert_eq!(
            found,
            &ArgoFound::Labelled {
                skipped: vec!["d-ops".into(), "e-ops".into()]
            }
        );
        assert!(
            !reader
                .requests
                .borrow()
                .iter()
                .any(|r| r.contains("d-ops") || r.contains("e-ops"))
        );
        let found = derive(
            &Inputs {
                sessions: vec![read],
                stage_naming: None,
            },
            &Override::default(),
        );
        assert!(found.coverage.iter().any(|c| matches!(
            &c.state,
            CoverageState::NamespaceOnly { found: ArgoFound::Labelled { skipped }, .. }
                if skipped.len() == 2
        )));
    }

    #[tokio::test]
    async fn an_empty_default_namespace_is_not_found_never_none() {
        // Kargo and workloads answer; Argo CD is served but nothing marks
        // it, and argocd holds no Applications.
        let reader = FixtureReader::default().serves(
            "argoproj.io",
            "v1alpha1",
            &["applications", "applicationsets"],
        );
        let read = read_session(&reader, key("core-fra")).await;
        let found = derive(
            &Inputs {
                sessions: vec![read],
                stage_naming: None,
            },
            &Override::default(),
        );
        let argo: Vec<_> = found
            .coverage
            .iter()
            .filter(|c| c.source.rule() == Rule::ArgoCd)
            .map(|c| &c.state)
            .collect();
        assert_eq!(
            argo,
            [
                &CoverageState::NamespaceNotFound("argocd".into()),
                &CoverageState::NamespaceNotFound("argocd".into()),
            ]
        );
        assert!(found.unknown().contains_key(&Rule::ArgoCd));
    }

    #[tokio::test]
    async fn a_refusal_in_one_namespace_stands_for_the_whole_read() {
        let reader = argo_in(&["gitops", "platform"]).refusing("applications");
        let read = read_session(&reader, key("core-fra")).await;
        assert!(matches!(read.argo_applications, Source::Refused(_)));
    }

    #[tokio::test]
    async fn a_listing_cut_at_the_page_cap_marks_the_source_capped() {
        let reader = world().capped("deployments");
        let read = read_session(&reader, key("core-fra")).await;
        assert!(matches!(read.workloads, Source::Capped(_, _)));
    }
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

#[test]
fn refused_kargo_projects_are_unknown_and_not_no_projects() {
    let mut core = session("core-fra");
    core.kargo = Source::Refused("forbidden".into());
    let found = derived(vec![core]);
    assert!(found.applications.is_empty());
    assert_eq!(found.unknown()[&Rule::Kargo], [key("core-fra")]);
}

#[test]
fn unread_warehouses_leave_the_stages_and_a_note() {
    let mut core = session("core-fra");
    let mut shop = project("checkout", &["dev"]);
    shop.warehouses = Source::Unreadable("timed out".into());
    core.kargo = Source::Read(vec![shop]);
    let found = derived(vec![core]);
    let app = found.find(&id(Rule::Kargo, "checkout")).unwrap();
    assert_eq!(names(app), ["dev"]);
    assert!(app.notes.iter().any(|n| matches!(
        n,
        Note::MembersUnknown { why, .. } if why.starts_with("Warehouses")
    )));
    assert!(found.coverage.iter().any(|c| {
        c.source == SourceKind::KargoWarehouses && matches!(c.state, CoverageState::Unreadable(_))
    }));
}

#[test]
fn capped_stages_keep_what_was_read_and_say_members_may_be_missing() {
    let mut core = session("core-fra");
    let mut shop = project("checkout", &[]);
    shop.stages = Source::Capped(vec![stage("checkout", "dev")], Truncation { read: 1 });
    core.kargo = Source::Read(vec![shop]);
    let found = derived(vec![core]);
    let app = found.find(&id(Rule::Kargo, "checkout")).unwrap();
    assert_eq!(names(app), ["dev", "images"]);
    assert!(app.notes.iter().any(|n| matches!(
        n,
        Note::MembersUnknown { why, .. } if why.starts_with("Stages") && why.contains("page cap")
    )));
    assert!(found.unknown().contains_key(&Rule::Kargo));
}

#[test]
fn workloads_fall_to_their_label_with_a_note_when_argo_applications_were_not_read() {
    let mut core = session("core-fra");
    core.argo_applications = Source::Refused("forbidden".into());
    core.workloads = Source::Read(vec![workload("shop", "web", "storefront")]);
    let found = derived(vec![core]);
    let app = found.find(&id(Rule::PartOf, "storefront")).unwrap();
    assert_eq!(app.members[0].basis, Basis::Direct);
    assert!(app.notes.iter().any(|n| matches!(
        n,
        Note::ManagerUnknown { member } if member.name == "web"
    )));
    assert!(found.unknown().contains_key(&Rule::ArgoCd));
    // Read, and not served, there is no manager to be unsure of.
    let mut plain = session("core-fra");
    plain.workloads = Source::Read(vec![workload("shop", "web", "storefront")]);
    let found = derived(vec![plain]);
    assert!(
        found
            .find(&id(Rule::PartOf, "storefront"))
            .unwrap()
            .notes
            .is_empty()
    );
}

#[test]
fn a_split_member_takes_its_notes_along_and_members_stay_in_order() {
    let mut core = session("core-fra");
    core.argo_applications = Source::Read(vec![
        argo(
            "cart-prod",
            json!({"spec": {"destination": {"server": "https://prod.example.test:6443"}}}),
        ),
        argo("cart-dev", json!({})),
    ]);
    let found = derived(vec![core.clone()]);
    let member = MemberRef {
        session: key("core-fra"),
        kind: MemberKind::ArgoApplication,
        namespace: Some("argocd".into()),
        name: "cart-prod".into(),
    };
    assert_eq!(found.find(&app_id("cart-prod")).unwrap().notes.len(), 1);
    let over = Override {
        splits: vec![Split {
            member: member.clone(),
            name: "prod only".into(),
        }],
        ..Override::default()
    };
    let found = derive(
        &Inputs {
            sessions: vec![core],
            stage_naming: None,
        },
        &over,
    );
    let prod = found.find(&id(Rule::Manual, "prod only")).unwrap();
    assert!(
        prod.notes
            .iter()
            .any(|n| matches!(n, Note::UnmappedDestination { member: m } if *m == member))
    );
    let left = found.find(&app_id("cart-prod")).unwrap();
    assert!(
        left.members.is_empty() && left.notes.is_empty(),
        "the note moved with it"
    );
    assert!(found.unmatched.is_empty());
}

#[test]
fn a_merge_naming_a_missing_application_twice_reports_it_once() {
    let gone = id(Rule::Kargo, "retired");
    let over = Override {
        merges: vec![Merge {
            from: vec![gone.clone(), gone.clone()],
            into: id(Rule::Kargo, "checkout"),
        }],
        ..Override::default()
    };
    assert_eq!(with_override(over).unmatched, [Unmatched::MergeFrom(gone)]);
}

#[test]
fn members_stay_in_order_after_a_merge_and_a_split() {
    let over = Override {
        merges: vec![Merge {
            from: vec![id(Rule::PartOf, "storefront")],
            into: id(Rule::Kargo, "checkout"),
        }],
        splits: vec![Split {
            member: MemberRef {
                session: key("core-fra"),
                kind: MemberKind::KargoStage,
                namespace: Some("checkout".into()),
                name: "dev".into(),
            },
            name: "dev".into(),
        }],
        ..Override::default()
    };
    let found = with_override(over);
    let checkout = found.find(&id(Rule::Kargo, "checkout")).unwrap();
    assert_eq!(names(checkout), ["images", "web"]);
}

// The override applies after every rule, to a Kargo Project's application
// as to any other.

fn kargo_world(over: Override) -> Derived {
    let mut core = session("core-fra");
    core.kargo = Source::Read(vec![
        project("checkout", &["dev", "prod"]),
        project("cart", &["dev"]),
    ]);
    core.argo_applications = Source::Read(vec![argo("catalog", json!({}))]);
    derive(
        &Inputs {
            sessions: vec![core],
            stage_naming: None,
        },
        &over,
    )
}

#[test]
fn an_override_renames_a_kargo_application() {
    let found = kargo_world(Override {
        renames: [(id(Rule::Kargo, "checkout"), "Shop".to_owned())].into(),
        ..Override::default()
    });
    let app = found.find(&id(Rule::Kargo, "checkout")).unwrap();
    assert_eq!((app.name.as_str(), app.rule), ("Shop", Rule::Kargo));
}

#[test]
fn an_override_hides_a_kargo_application() {
    let found = kargo_world(Override {
        hidden: [id(Rule::Kargo, "cart")].into(),
        ..Override::default()
    });
    assert!(found.find(&id(Rule::Kargo, "cart")).is_none());
    assert_eq!(found.hidden[0].id, id(Rule::Kargo, "cart"));
}

#[test]
fn an_override_merges_a_kargo_application_into_a_lower_rules_and_the_other_way() {
    let into_argo = kargo_world(Override {
        merges: vec![Merge {
            from: vec![id(Rule::Kargo, "cart")],
            into: app_id("catalog"),
        }],
        ..Override::default()
    });
    assert!(into_argo.find(&id(Rule::Kargo, "cart")).is_none());
    let catalog = into_argo.find(&app_id("catalog")).unwrap();
    assert_eq!(names(catalog), ["dev", "images", "catalog"]);

    let into_kargo = kargo_world(Override {
        merges: vec![Merge {
            from: vec![app_id("catalog")],
            into: id(Rule::Kargo, "checkout"),
        }],
        ..Override::default()
    });
    assert!(into_kargo.find(&app_id("catalog")).is_none());
    let checkout = into_kargo.find(&id(Rule::Kargo, "checkout")).unwrap();
    assert_eq!(checkout.rule, Rule::Kargo);
    assert_eq!(names(checkout), ["dev", "prod", "images", "catalog"]);
}

#[test]
fn an_override_splits_a_stage_out_of_a_kargo_application() {
    let stage = MemberRef {
        session: key("core-fra"),
        kind: MemberKind::KargoStage,
        namespace: Some("checkout".into()),
        name: "prod".into(),
    };
    let found = kargo_world(Override {
        splits: vec![Split {
            member: stage.clone(),
            name: "checkout prod".into(),
        }],
        ..Override::default()
    });
    assert_eq!(
        names(found.find(&id(Rule::Kargo, "checkout")).unwrap()),
        ["dev", "images"]
    );
    let split = found.find(&id(Rule::Manual, "checkout prod")).unwrap();
    assert_eq!(split.members[0].at, stage);
}

#[test]
fn applications_listed_in_one_namespace_leave_argo_cd_unknown_for_workloads() {
    let mut core = session("core-fra");
    core.argo_scope = ArgoScope::Namespace {
        namespaces: vec!["gitops".into()],
        found: ArgoFound::Labelled { skipped: vec![] },
    };
    core.workloads = Source::Read(vec![workload("shop", "web", "storefront")]);
    let found = derived(vec![core]);
    let app = found.find(&id(Rule::PartOf, "storefront")).unwrap();
    assert!(
        app.notes
            .iter()
            .any(|n| matches!(n, Note::ManagerUnknown { member } if member.name == "web"))
    );
    assert_eq!(found.unknown()[&Rule::ArgoCd], [key("core-fra")]);
    assert!(found.coverage.iter().any(|c| {
        c.source == SourceKind::ArgoApplications
            && c.state
                == CoverageState::NamespaceOnly {
                    namespaces: vec!["gitops".into()],
                    found: ArgoFound::Labelled { skipped: vec![] },
                }
    }));
}

#[test]
fn a_project_beyond_the_cap_is_noted_on_the_application_that_names_it() {
    let naming_p2 = argo(
        "p2-dev",
        json!({"metadata": {"annotations": {"kargo.akuity.io/authorized-stage": "p2:dev"}}}),
    );
    let capped = |sessions: Source<Vec<KargoProjectRead>>| {
        let mut core = session("core-fra");
        core.kargo = sessions;
        core.argo_applications = Source::Read(vec![naming_p2.clone()]);
        derived(vec![core])
    };
    let found = capped(Source::Capped(
        vec![project("p1", &["dev"])],
        Truncation { read: 1 },
    ));
    let app = found.find(&app_id("p2-dev")).unwrap();
    assert!(app.notes.iter().any(|n| matches!(
        n,
        Note::ProjectNotRead { project, session, why, .. } if project == "p2" && why.contains("cap") && session == &SessionKey::new("core-fra")
    )));
    let refused = capped(Source::Refused("forbidden".into()));
    assert!(
        refused
            .find(&app_id("p2-dev"))
            .unwrap()
            .notes
            .iter()
            .any(|n| matches!(n, Note::ProjectNotRead { .. }))
    );
    // Kargo read in full, or not served: the Project is simply not there.
    let whole = capped(Source::Read(vec![project("p1", &["dev"])]));
    assert!(whole.find(&app_id("p2-dev")).unwrap().notes.is_empty());
    let absent = capped(Source::NotInstalled("not served".into()));
    assert!(absent.find(&app_id("p2-dev")).unwrap().notes.is_empty());
}

#[test]
fn unknown_members_name_the_cluster() {
    let mut core = session("core-fra");
    let mut shop = project("checkout", &["dev"]);
    shop.stages = Source::Refused("forbidden".into());
    core.kargo = Source::Read(vec![shop]);
    let found = derived(vec![core]);
    let app = found.find(&id(Rule::Kargo, "checkout")).unwrap();
    assert!(app.notes.iter().any(|n| matches!(
        n,
        Note::MembersUnknown { session, .. } if *session == key("core-fra")
    )));
}

#[test]
fn splitting_off_the_member_that_crossed_clusters_drops_the_join_note() {
    let mut dev = session("dev-fra");
    dev.workloads = Source::Read(vec![workload("shop", "web", "storefront")]);
    let mut prod = session("prod-fra");
    prod.workloads = Source::Read(vec![workload("shop", "web", "storefront")]);
    let split = |member_session: &str| Override {
        splits: vec![Split {
            member: MemberRef {
                session: key(member_session),
                kind: MemberKind::Workload(WorkloadKind::Deployment),
                namespace: Some("shop".into()),
                name: "web".into(),
            },
            name: "prod web".into(),
        }],
        ..Override::default()
    };
    let inputs = Inputs {
        sessions: vec![dev, prod],
        stage_naming: None,
    };
    let found = derive(&inputs, &split("prod-fra"));
    let left = found.find(&id(Rule::PartOf, "storefront")).unwrap();
    assert!(left.notes.is_empty(), "{:?}", left.notes);
    assert_eq!(left.members.len(), 1);
    assert_eq!(left.members[0].basis, Basis::Direct, "no longer a guess");
    let moved = found.find(&id(Rule::Manual, "prod web")).unwrap();
    assert_eq!(moved.members[0].basis, Basis::Override);
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
