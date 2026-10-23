use super::*;

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
            destinations: Vec::new(),
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
        destinations: Vec::new(),
    };
    let found = derive(&inputs, &split("prod-fra"));
    let left = found.find(&id(Rule::PartOf, "storefront")).unwrap();
    assert!(left.notes.is_empty(), "{:?}", left.notes);
    assert_eq!(left.members.len(), 1);
    assert_eq!(left.members[0].basis, Basis::Direct, "no longer a guess");
    let moved = found.find(&id(Rule::Manual, "prod web")).unwrap();
    assert_eq!(moved.members[0].basis, Basis::Override);
}
