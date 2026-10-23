use super::*;

#[tokio::test(start_paused = true)]
async fn read_only_blocks_every_tier_and_makes_no_request_at_all() {
    for action in [fake::scale(3), fake::set_image("x"), fake::delete_web()] {
        let (cluster, dir) = (Cluster::new(), tmp());
        let runner = fake::runner(&cluster, dir.path(), Protection::ReadOnly);
        match runner.preflight(action).await {
            Verdict::Blocked(Block::ReadOnly(reason)) => {
                assert_eq!(reason, "This cluster is read-only.")
            }
            other => panic!("{other:?}"),
        }
        assert!(cluster.recorder.requests().is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn a_prepared_action_is_still_refused_by_a_read_only_runner() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let guarded = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&guarded, fake::scale(3)).await;
    let read_only = fake::runner(&cluster, dir.path(), Protection::ReadOnly);
    let finished = run(&read_only, prepared, Confirmation::Confirmed).await;
    assert_eq!(
        finished.status,
        Status::NotSent(NotSent::ReadOnly("This cluster is read-only."))
    );
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn nothing_is_sent_without_the_confirmation_the_level_asks_for() {
    for (level, action, given) in [
        (Protection::Guarded, fake::scale(3), Confirmation::NotAsked),
        (
            Protection::Protected,
            fake::scale(3),
            Confirmation::Confirmed,
        ),
        (
            Protection::Protected,
            fake::scale(3),
            Confirmation::Typed("Web".into()),
        ),
        (
            Protection::Guarded,
            fake::set_image("x"),
            Confirmation::NotAsked,
        ),
        (
            Protection::Protected,
            fake::delete_web(),
            Confirmation::Typed("shop".into()),
        ),
    ] {
        let (cluster, dir) = (Cluster::new(), tmp());
        let runner = fake::runner(&cluster, dir.path(), level);
        let prepared = ready(&runner, action).await;
        let finished = run(&runner, prepared, given).await;
        assert_eq!(
            finished.status,
            Status::NotSent(NotSent::Unconfirmed),
            "{level:?}"
        );
        cluster.assert_nothing_written();
        // And nothing was audited as sent.
        assert!(
            AuditStore::new(dir.path().join("audit"))
                .read()
                .unwrap()
                .records
                .is_empty()
        );
    }
}

#[tokio::test(start_paused = true)]
async fn open_sends_a_reversible_action_without_a_dialog_but_not_a_spec_change() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Open);
    let prepared = ready(&runner, fake::scale(3)).await;
    assert!(!prepared.requirement().confirm);
    assert_eq!(
        run(&runner, prepared, Confirmation::NotAsked)
            .await
            .headline(),
        Headline::Done
    );
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Open);
    let prepared = ready(&runner, fake::set_image("registry.example/web:2")).await;
    assert!(prepared.requirement().confirm && prepared.requirement().diff);
}

#[tokio::test(start_paused = true)]
async fn protected_takes_the_typed_name_and_the_entry_id_for_several_objects() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Protected);
    let prepared = ready(&runner, fake::scale(3)).await;
    assert_eq!(prepared.expected_name("prod-fra"), "web");
    let finished = run(&runner, prepared, Confirmation::Typed("web".into())).await;
    assert_eq!(finished.headline(), Headline::Done);
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Protected);
    let prepared = ready(&runner, fake::scale_and_delete()).await;
    assert_eq!(prepared.expected_name("prod-fra"), "prod-fra");
}

#[tokio::test(start_paused = true)]
async fn a_destructive_action_waits_five_seconds_and_cancelling_sends_nothing() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::delete_web()).await;
    assert_eq!(
        prepared.requirement().cancel_window,
        Some(Duration::from_secs(5))
    );
    let events = std::sync::Mutex::new(vec![]);
    let started = tokio::time::Instant::now();
    let finished = runner
        .execute(
            *prepared,
            Confirmation::Confirmed,
            || started.elapsed() >= Duration::from_secs(2),
            |event| events.lock().unwrap().push(event),
        )
        .await;
    assert_eq!(finished.status, Status::Cancelled);
    assert_eq!(finished.headline(), Headline::Cancelled);
    assert_eq!(
        events.lock().unwrap()[0],
        Event::CancelWindow(Duration::from_secs(5))
    );
    cluster.assert_nothing_written();
    assert_eq!(
        phases(&AuditStore::new(dir.path().join("audit"))),
        [Phase::Cancelled]
    );
}

#[tokio::test(start_paused = true)]
async fn an_uncancelled_destructive_action_sends_a_delete_with_preconditions_after_the_window() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::delete_web()).await;
    let started = tokio::time::Instant::now();
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert_eq!(finished.headline(), Headline::Done);
    assert!(!cluster.exists(fake::DEPLOYMENT));
    let deletes = cluster.writes();
    assert_eq!(deletes.len(), 1);
    assert!(deletes[0].body.contains("uid-web") && deletes[0].body.contains("\"100\""));
    assert!(finished.undo.is_none());
}

// ---- Review items: levels, session, undo, values ----

#[tokio::test(start_paused = true)]
async fn the_level_is_the_strictest_of_the_cluster_and_the_destination() {
    use Protection::*;
    let with_destination = |protection| {
        let mut action = fake::scale(3);
        action.destinations = vec![Destination {
            entry: "prod-ams".into(),
            protection,
            cell: None,
            covers: vec![],
        }];
        action
    };
    // Own guarded, destination read-only: blocked, with no request at all.
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Guarded);
    assert!(matches!(
        runner.preflight(with_destination(ReadOnly)).await,
        Verdict::Blocked(Block::ReadOnly(_))
    ));
    assert!(cluster.recorder.requests().is_empty());
    // Own guarded, destination protected: the name is asked.
    let prepared = ready(&runner, with_destination(Protected)).await;
    assert!(prepared.requirement().type_name);
    // Own protected, destination open: still protected.
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protected);
    assert!(
        ready(&runner, with_destination(Open))
            .await
            .requirement()
            .type_name
    );
    // No destination: the own level.
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Open);
    assert!(!ready(&runner, fake::scale(3)).await.requirement().confirm);
    assert_eq!(fake::scale(3).level(Open), Open);
    assert_eq!(with_destination(ReadOnly).level(Open), ReadOnly);
}

#[tokio::test(start_paused = true)]
async fn a_session_action_is_refused_before_any_request() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Open);
    let mut action = fake::scale(3);
    action.tier = Tier::Session;
    assert!(matches!(
        runner.preflight(action).await,
        Verdict::Blocked(Block::Session)
    ));
    assert!(cluster.recorder.requests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_level_that_got_stricter_after_preflight_sends_nothing_and_asks_again() {
    use Protection::*;
    let (cluster, dir) = (Cluster::new(), tmp());
    let level = LevelCell::new(Guarded);
    let runner = fake::runner(&cluster, dir.path(), Guarded).following(level.clone());
    let prepared = ready(&runner, fake::scale(3)).await;
    level.set(Protected);
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(
        finished.status,
        Status::NotSent(NotSent::LevelChanged(Protected))
    );
    assert!(finished.message().contains("Nothing was sent"));
    cluster.assert_nothing_written();
    // Asked again under the new level: the name is wanted now.
    assert!(ready(&runner, fake::scale(3)).await.requirement().type_name);
    // Read-only now: refused outright.
    let prepared = ready(&runner, fake::scale(3)).await;
    level.set(ReadOnly);
    let finished = run(&runner, prepared, Confirmation::Typed("web".into())).await;
    assert!(matches!(
        finished.status,
        Status::NotSent(NotSent::ReadOnly(_))
    ));
    cluster.assert_nothing_written();
    assert!(phases(&AuditStore::new(dir.path().join("audit"))).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_level_that_changes_during_the_cancel_window_decides_the_send() {
    use Protection::*;
    for (becomes, read_only) in [(ReadOnly, true), (Protected, false)] {
        let (cluster, dir) = (Cluster::new(), tmp());
        let level = LevelCell::new(Guarded);
        let runner = fake::runner(&cluster, dir.path(), Guarded).following(level.clone());
        let prepared = ready(&runner, fake::delete_web()).await;
        let started = tokio::time::Instant::now();
        let finished = runner
            .execute(
                *prepared,
                Confirmation::Confirmed,
                || {
                    if started.elapsed() >= Duration::from_secs(2) {
                        level.set(becomes);
                    }
                    false
                },
                |_| {},
            )
            .await;
        match finished.status {
            Status::NotSent(NotSent::ReadOnly(_)) => assert!(read_only),
            Status::NotSent(NotSent::LevelChanged(now)) => assert!(!read_only && now == Protected),
            other => panic!("{other:?}"),
        }
        cluster.assert_nothing_written();
        assert!(phases(&AuditStore::new(dir.path().join("audit"))).is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn an_action_that_declares_no_confirmation_asks_nothing_at_any_writable_level() {
    for level in [Protection::Protected, Protection::Guarded, Protection::Open] {
        let (cluster, dir) = (Cluster::new(), tmp());
        let runner = fake::runner(&cluster, dir.path(), level);
        let prepared = ready(&runner, fake::annotate_web()).await;
        assert!(
            !prepared.requirement().confirm && !prepared.requirement().type_name,
            "{level:?}"
        );
        let verdict = Verdict::Ready(prepared);
        assert_eq!(
            Dialog::for_verdict(&verdict, "prod-fra"),
            Dialog::SendWithoutAsking {
                notice: ANNOTATION_NOTICE
            }
        );
        let Verdict::Ready(prepared) = verdict else {
            unreachable!()
        };
        let finished = run(&runner, prepared, Confirmation::NotAsked).await;
        assert_eq!(finished.headline(), Headline::Done, "{level:?}");
        // Still audited, from the line before the send to the observation.
        assert_eq!(
            phases(&AuditStore::new(dir.path().join("audit"))),
            [Phase::Sending, Phase::Accepted, Phase::Observed]
        );
        assert_eq!(
            cluster.object(fake::DEPLOYMENT)["metadata"]["annotations"]["example.com/refresh"],
            "t1"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn the_declared_mark_is_the_action_s_and_the_same_action_without_it_asks() {
    // Without the mark the same reversible action asks, as its tier says.
    let mut action = fake::annotate_web();
    action.no_confirmation = false;
    assert!(requirement_for(&action, Protection::Guarded).confirm);
    assert!(requirement_for(&action, Protection::Protected).type_name);
    // With it, only read-only still blocks.
    let marked = fake::annotate_web();
    assert!(!requirement_for(&marked, Protection::Protected).confirm);
    assert!(
        requirement_for(&marked, Protection::ReadOnly)
            .blocked
            .is_some()
    );
}

#[tokio::test(start_paused = true)]
async fn read_only_blocks_an_unconfirmed_action_and_a_create_with_no_request_at_all() {
    for action in [fake::annotate_web(), fake::create_notes(&["stage-a"])] {
        let (cluster, dir) = (Cluster::new(), tmp());
        let runner = fake::runner(&cluster, dir.path(), Protection::ReadOnly);
        assert!(matches!(
            runner.preflight(action).await,
            Verdict::Blocked(Block::ReadOnly(_))
        ));
        assert!(cluster.recorder.requests().is_empty());
        assert!(phases(&AuditStore::new(dir.path().join("audit"))).is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn a_destination_whose_level_changes_after_the_dialog_opened_is_followed() {
    use Protection::*;
    for (becomes, read_only) in [(Protected, false), (ReadOnly, true)] {
        let (cluster, dir) = (Cluster::new(), tmp());
        let destination = LevelCell::new(Guarded);
        let runner = fake::runner(&cluster, dir.path(), Open);
        let mut action = fake::create_notes(&["stage-a"]);
        action.destinations = vec![Destination {
            entry: "prod-ams".into(),
            protection: Guarded,
            cell: Some(destination.clone()),
            covers: vec!["stage-a".into()],
        }];
        let prepared = ready(&runner, action).await;
        assert!(!prepared.requirement().type_name);
        // Settings changes the destination's level while the dialog is open.
        destination.set(becomes);
        let finished = run(&runner, prepared, Confirmation::Confirmed).await;
        match finished.status {
            Status::NotSent(NotSent::ReadOnly(_)) => assert!(read_only),
            Status::NotSent(NotSent::LevelChanged(now)) => assert!(!read_only && now == Protected),
            other => panic!("{other:?}"),
        }
        cluster.assert_nothing_written();
        // Asked again, the dialog reads the level as it is now.
        if !read_only {
            let mut again = fake::create_notes(&["stage-a"]);
            again.destinations = vec![Destination {
                entry: "prod-ams".into(),
                protection: Guarded,
                cell: Some(destination.clone()),
                covers: vec!["stage-a".into()],
            }];
            let verdict = runner.preflight(again).await;
            let Dialog::Ask(ask) = Dialog::for_verdict(&verdict, "prod-fra") else {
                panic!("asks");
            };
            assert_eq!(ask.type_name.as_deref(), Some("stage-a"));
            assert_eq!(ask.destinations, [("prod-ams".to_owned(), Protected)]);
        }
    }
}
