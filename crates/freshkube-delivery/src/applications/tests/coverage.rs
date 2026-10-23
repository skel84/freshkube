use super::*;

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
