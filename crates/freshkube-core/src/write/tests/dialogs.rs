use super::*;

// ---- The dialog each state and level asks for ----

#[tokio::test(start_paused = true)]
async fn each_level_draws_the_dialog_the_table_names() {
    use Protection::*;
    // Read-only: disabled, with the reason, whatever the tier.
    for action in [fake::scale(3), fake::set_image("x"), fake::delete_web()] {
        assert_eq!(
            dialog_for(ReadOnly, action).await,
            Dialog::Disabled {
                reason: "This cluster is read-only.".into()
            }
        );
    }
    // Open: a reversible action goes with no dialog and says so.
    assert_eq!(
        dialog_for(Open, fake::scale(3)).await,
        Dialog::SendWithoutAsking {
            notice: OPEN_NOTICE
        }
    );
    // Guarded reversible: a plain confirm. Protected: the name.
    let Dialog::Ask(ask) = dialog_for(Guarded, fake::scale(3)).await else {
        panic!()
    };
    assert_eq!(
        (ask.type_name.clone(), ask.cancel_window, ask.danger),
        (None, None, false)
    );
    assert_eq!(ask.verb, "Scale");
    let Dialog::Ask(ask) = dialog_for(Protected, fake::scale(3)).await else {
        panic!()
    };
    assert_eq!(ask.type_name.as_deref(), Some("web"));
    // Spec change at every level that allows it: the diff is in the dialog.
    for level in [Protected, Guarded, Open] {
        let Dialog::Ask(ask) = dialog_for(level, fake::set_image("registry.example/web:2")).await
        else {
            panic!("{level:?}")
        };
        assert_eq!(ask.diff[0].changes.len(), 1);
        assert_eq!(ask.type_name.is_some(), level == Protected);
    }
    // Destructive: impact, a cancel window, and a dangerous confirm.
    for level in [Protected, Guarded, Open] {
        let Dialog::Ask(ask) = dialog_for(level, fake::delete_web()).await else {
            panic!()
        };
        assert_eq!(ask.impact, ["Deployment/web"]);
        assert_eq!(ask.cancel_window, Some(Duration::from_secs(5)));
        assert!(ask.danger);
        assert_eq!(ask.type_name.is_some(), level == Protected);
    }
    // Several objects: the entry id, typed once, and every object listed.
    let Dialog::Ask(ask) = dialog_for(Protected, fake::scale_and_delete()).await else {
        panic!()
    };
    assert_eq!(ask.type_name.as_deref(), Some("prod-fra"));
    assert_eq!(ask.impact, ["Deployment/web", "Secret/db"]);
}

#[tokio::test(start_paused = true)]
async fn a_changed_object_offers_only_to_review_the_new_version_and_a_deleted_one_nothing() {
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.foreign_write(
        fake::DEPLOYMENT,
        "kubectl",
        &json!({"spec": {"paused": true}}),
    );
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let verdict = runner
        .preflight(fake::set_image("registry.example/web:2"))
        .await;
    let Dialog::Changed { text, can_review } = Dialog::for_verdict(&verdict, "e") else {
        panic!()
    };
    assert!(
        can_review && text.contains("changed after you opened it"),
        "{text}"
    );
    // Reviewing re-bases the action and runs preflight again: a fresh diff
    // against the new version, still unsent and still asking.
    let Verdict::Blocked(_) = verdict else {
        panic!()
    };
    let writes = cluster.writes().len();
    let rebased = runner
        .review_new_version(fake::set_image("registry.example/web:2"))
        .await
        .unwrap();
    assert_eq!(rebased.changes[0].target.resource_version, "1001");
    let again = runner.preflight(rebased).await;
    assert!(matches!(Dialog::for_verdict(&again, "e"), Dialog::Ask(_)));
    assert_eq!(cluster.writes().len(), writes);
    // Gone: nothing to review.
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let mut action = fake::set_image("x");
    action.changes[0].target.name = "missing".into();
    let block = runner.preflight(action).await;
    let Dialog::Changed { can_review, .. } = Dialog::for_verdict(&block, "e") else {
        panic!("{block:?}")
    };
    assert!(!can_review);
}

#[tokio::test(start_paused = true)]
async fn a_conflict_dialog_names_the_manager_and_fields_and_offers_no_force() {
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.refuse_dry_runs(
        409,
        "Apply failed with 1 conflict: conflict with \"hpa\" using apps/v1: .spec.replicas",
    );
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let verdict = runner.preflight(fake::scale(3)).await;
    let Dialog::Conflict {
        managers,
        fields,
        text,
    } = Dialog::for_verdict(&verdict, "e")
    else {
        panic!("{verdict:?}")
    };
    assert_eq!(
        (managers, fields),
        (vec!["hpa".to_owned()], vec![".spec.replicas".to_owned()])
    );
    assert!(
        text.contains("The rest of the write was not sent."),
        "{text}"
    );
    assert!(!text.to_lowercase().contains("force anyway"));
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn refusals_before_the_dialog_are_refused_not_asked() {
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.deny_access();
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    assert!(matches!(
        Dialog::for_verdict(&runner.preflight(fake::scale(3)).await, "e"),
        Dialog::Refused { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn the_typed_name_is_the_subject_and_is_typed_once_for_several() {
    use Protection::*;
    let covering = |names: &[&str], protection| {
        let mut action = fake::create_notes(&["stage-a", "stage-b"]);
        action.destinations = vec![Destination {
            entry: "prod-ams".into(),
            protection,
            cell: None,
            covers: names.iter().map(|n| (*n).to_owned()).collect(),
        }];
        action
    };
    // One subject: its own name, not the generated object's.
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protected);
    let one = ready(&runner, fake::create_notes(&["stage-a"])).await;
    assert!(one.requirement().type_name);
    assert_eq!(one.expected_name("prod-fra"), "stage-a");
    // Several: typed once, for the entry; the dialog names which are protected.
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Guarded);
    let verdict = runner.preflight(covering(&["stage-b"], Protected)).await;
    let Dialog::Ask(ask) = Dialog::for_verdict(&verdict, "prod-fra") else {
        panic!("asks");
    };
    assert_eq!(ask.type_name.as_deref(), Some("prod-fra"));
    assert_eq!(ask.protected, ["stage-b"]);
    assert_eq!(ask.subjects, ["stage-a", "stage-b"]);
    assert_eq!(ask.destinations, [("prod-ams".to_owned(), Protected)]);
    let Verdict::Ready(prepared) = verdict else {
        panic!("ready")
    };
    // A guess at one Stage's name unlocks nothing; the one name unlocks both.
    let again = ready(&runner, covering(&["stage-b"], Protected)).await;
    let wrong = run(&runner, again, Confirmation::Typed("stage-b".into())).await;
    assert_eq!(wrong.status, Status::NotSent(NotSent::Unconfirmed));
    cluster.assert_nothing_written();
    let finished = run(&runner, prepared, Confirmation::Typed("prod-fra".into())).await;
    assert_eq!(finished.headline(), Headline::Done);
    assert_eq!(cluster.writes().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn guarded_on_both_clusters_shows_the_preview_without_typing() {
    use Protection::*;
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Guarded);
    let mut action = fake::create_notes(&["stage-a"]);
    action.preview = vec![Preview {
        title: "Runs now".into(),
        lines: vec!["freight-1".into()],
    }];
    action.destinations = vec![Destination {
        entry: "prod-ams".into(),
        protection: Guarded,
        cell: None,
        covers: vec!["stage-a".into()],
    }];
    let verdict = runner.preflight(action).await;
    let Dialog::Ask(ask) = Dialog::for_verdict(&verdict, "prod-fra") else {
        panic!("asks");
    };
    assert_eq!(ask.type_name, None);
    assert!(ask.protected.is_empty());
    assert_eq!(ask.preview[0].title, "Runs now");
    // And a destination stricter than the cluster it is written on wins.
    let mut stricter = fake::create_notes(&["stage-a"]);
    stricter.destinations = vec![
        Destination {
            entry: "dev-fra".into(),
            protection: Open,
            cell: None,
            covers: vec![],
        },
        Destination {
            entry: "prod-ams".into(),
            protection: Protected,
            cell: None,
            covers: vec!["stage-a".into()],
        },
    ];
    assert_eq!(stricter.level(Open), Protected);
    assert_eq!(stricter.level(ReadOnly), ReadOnly);
}

#[tokio::test(start_paused = true)]
async fn an_action_may_title_its_dialog_by_what_the_person_does() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let mut action = fake::scale(3);
    let plain = match Dialog::for_verdict(&runner.preflight(action.clone()).await, "e") {
        Dialog::Ask(ask) => ask.title,
        other => panic!("{other:?}"),
    };
    assert_eq!(plain, "Scale Deployment/web");
    action.title = Some("Scale the web tier to 3".into());
    match Dialog::for_verdict(&runner.preflight(action).await, "e") {
        Dialog::Ask(ask) => assert_eq!(ask.title, "Scale the web tier to 3"),
        other => panic!("{other:?}"),
    }
}
