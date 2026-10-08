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
    }
}

fn session(name: &str) -> SessionInputs {
    SessionInputs {
        key: key(name),
        kargo: Source::NotInstalled("API group kargo.akuity.io is not served".into()),
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
    let cart = found.find(&id(Rule::ArgoCd, "cart")).expect("cart");
    assert_eq!(names(cart), ["cart-dev", "cart-prod"]);
    assert!(matches!(
        cart.evidence[0],
        Evidence::ArgoApplicationSet { read: true, .. }
    ));
    let catalog = found.find(&id(Rule::ArgoCd, "catalog")).expect("catalog");
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
    assert!(
        found
            .find(&id(Rule::ArgoCd, "empty"))
            .unwrap()
            .members
            .is_empty()
    );
}

#[test]
fn an_owner_reference_names_the_set_even_when_the_sets_were_not_read() {
    let mut core = session("core-fra");
    core.argo_applications = Source::Read(vec![argo("cart-dev", owned_by("cart"))]);
    core.argo_application_sets = Source::Refused("forbidden".into());
    let found = derived(vec![core]);
    let cart = found.find(&id(Rule::ArgoCd, "cart")).expect("cart");
    assert!(matches!(
        cart.evidence[0],
        Evidence::ArgoApplicationSet { read: false, .. }
    ));
    assert!(found.unknown().contains_key(&Rule::ArgoCd));
}

#[test]
fn the_part_of_label_groups_workloads_across_clusters() {
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
    assert!(found.find(&id(Rule::ArgoCd, "checkout-dev")).is_some());
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
    let cart = found.find(&id(Rule::ArgoCd, "cart")).unwrap();
    let web = cart.members.iter().find(|m| m.at.name == "web").unwrap();
    assert_eq!(web.basis, Basis::ManagedBy);
    assert_eq!(web.also_claimed_by, [Rule::PartOf]);
    assert!(cart.notes.iter().any(|n| matches!(
        n,
        Note::LowerClaim { rule: Rule::PartOf, name, .. } if name == "storefront"
    )));
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
        let app = found.find(&id(Rule::ArgoCd, name)).unwrap();
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
            .any(|n| matches!(n, Note::MembersUnknown { why } if why.starts_with("Stages")))
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
            from: vec![id(Rule::ArgoCd, "catalog"), id(Rule::PartOf, "storefront")],
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
        hidden: [id(Rule::ArgoCd, "catalog")].into(),
        ..Override::default()
    };
    let found = with_override(over);
    assert!(found.find(&id(Rule::ArgoCd, "catalog")).is_none());
    assert_eq!(found.hidden[0].id, id(Rule::ArgoCd, "catalog"));
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
                from: vec![id(Rule::ArgoCd, "catalog")],
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
        found.find(&id(Rule::ArgoCd, "catalog")).is_some(),
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
        hidden: [id(Rule::ArgoCd, "catalog")].into(),
    };
    let text = serde_json::to_string(&over).unwrap();
    assert_eq!(serde_json::from_str::<Override>(&text).unwrap(), over);
}

mod reading {
    use serde_json::json;

    use super::*;
    use crate::applications::read::{MAX_PROJECTS, read_session};
    use crate::delivery::fixtures::FixtureReader;
    use crate::delivery::kargo;

    fn world() -> FixtureReader {
        let namespaced = |ns: &str, name: &str, labels: Value| json!({"metadata": {"namespace": ns, "name": name, "labels": labels}});
        FixtureReader::default()
            .serves("kargo.akuity.io", "v1alpha1", &["projects", "stages", "warehouses"])
            .serves("argoproj.io", "v1alpha1", &["applications", "applicationsets"])
            .with(
                "namespaces",
                vec![json!({"metadata": {"name": "checkout", "labels": {kargo::PROJECT_LABEL: "true"}}})],
            )
            .with("stages", vec![namespaced("checkout", "dev", json!({}))])
            .with("warehouses", vec![namespaced("checkout", "images", json!({}))])
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
                    namespaced("shop", "web", json!({"app.kubernetes.io/part-of": "storefront"})),
                    namespaced("shop", "plain", json!({"app.kubernetes.io/name": "plain"})),
                ],
            )
            .with(
                "statefulsets",
                vec![namespaced("shop", "db", json!({"app.kubernetes.io/part-of": "storefront"}))],
            )
    }

    #[tokio::test]
    async fn a_session_reads_every_source_and_derives_from_them() {
        let reader = world();
        let read = read_session(&reader, key("core-fra"), "argocd").await;
        let found = derive(
            &Inputs {
                sessions: vec![read],
                stage_naming: None,
            },
            &Override::default(),
        );
        let found_names: Vec<_> = found.applications.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(found_names, ["catalog", "checkout", "storefront"]);
        let storefront = found.find(&id(Rule::ArgoCd, "storefront")).unwrap();
        assert_eq!(
            names(storefront),
            ["storefront", "web", "db"],
            "the label matches the name, kinds in order"
        );
        assert!(found.unknown().is_empty());
    }

    #[tokio::test]
    async fn reads_are_only_gets_and_scoped_lists() {
        let reader = world();
        read_session(&reader, key("core-fra"), "argocd").await;
        let requests = reader.requests.borrow();
        assert!(
            requests
                .iter()
                .all(|r| r.starts_with("GET /apis") || r.starts_with("LIST "))
        );
        assert!(
            requests
                .iter()
                .filter(|r| r.contains("/deployments") || r.contains("/namespaces"))
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
        let read = read_session(&bare, key("core-fra"), "argocd").await;
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
        let namespaces = (0..MAX_PROJECTS + 2)
            .map(|n| json!({"metadata": {"name": format!("p{n:03}"), "labels": {kargo::PROJECT_LABEL: "true"}}}))
            .collect();
        let reader = FixtureReader::default()
            .serves(
                "kargo.akuity.io",
                "v1alpha1",
                &["projects", "stages", "warehouses"],
            )
            .with("namespaces", namespaces);
        let read = read_session(&reader, key("core-fra"), "argocd").await;
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

    #[tokio::test]
    async fn a_listing_cut_at_the_page_cap_marks_the_source_capped() {
        let reader = world().capped("deployments");
        let read = read_session(&reader, key("core-fra"), "argocd").await;
        assert!(matches!(read.workloads, Source::Capped(_, _)));
    }
}
