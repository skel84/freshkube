use super::*;

#[tokio::test(start_paused = true)]
async fn undo_goes_through_the_same_path_and_is_audited_as_its_own_write() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::scale(3)).await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    let offer = finished.undo.unwrap();
    assert_eq!(offer.window, Duration::from_secs(30));
    let before = cluster.recorder.requests().len();
    let prepared = runner
        .prepare_undo(&offer, Duration::from_secs(10))
        .await
        .unwrap();
    // Access review, read and dry run again, as for any write.
    let lines = cluster.recorder.lines();
    assert!(
        lines[before..]
            .iter()
            .any(|l| l.starts_with("POST /apis/authorization"))
    );
    assert!(lines[before..].iter().any(|l| l.contains("dryRun=All")));
    let undone = run(&runner, Box::new(prepared), Confirmation::Confirmed).await;
    assert_eq!(undone.headline(), Headline::Done);
    assert_eq!(cluster.object(fake::DEPLOYMENT)["spec"]["replicas"], 2);
    assert!(undone.undo.is_none(), "an undo is not undone");
    let read = AuditStore::new(dir.path().join("audit")).read().unwrap();
    let undo_run = read
        .records
        .iter()
        .filter(|r| r.action == "undo:test-scale")
        .collect::<Vec<_>>();
    assert_eq!(undo_run[0].phase, Phase::UndoSent);
    assert_ne!(undo_run[0].run, read.records[0].run);
}

#[tokio::test(start_paused = true)]
async fn undo_after_someone_else_changed_the_field_says_who_and_sends_nothing() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::scale(3)).await;
    let offer = run(&runner, prepared, Confirmation::Confirmed)
        .await
        .undo
        .unwrap();
    cluster.foreign_write(fake::DEPLOYMENT, "hpa", &json!({"spec": {"replicas": 5}}));
    let writes = cluster.writes().len();
    let refused = runner
        .prepare_undo(&offer, Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(
        refused,
        UndoRefused::Changed {
            object: "Deployment/web".into(),
            by: Some("hpa".into())
        }
    );
    assert_eq!(
        refused.text(),
        "Can't undo: Deployment/web was changed by hpa since."
    );
    assert_eq!(cluster.writes().len(), writes);
}

#[tokio::test(start_paused = true)]
async fn undo_is_not_sent_after_its_window_and_not_under_read_only() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::scale(3)).await;
    let offer = run(&runner, prepared, Confirmation::Confirmed)
        .await
        .undo
        .unwrap();
    assert_eq!(
        runner
            .prepare_undo(&offer, Duration::from_secs(31))
            .await
            .unwrap_err(),
        UndoRefused::Expired
    );
    let read_only = fake::runner(&cluster, dir.path(), Protection::ReadOnly);
    let before = cluster.recorder.requests().len();
    assert!(matches!(
        read_only
            .prepare_undo(&offer, Duration::from_secs(1))
            .await
            .unwrap_err(),
        UndoRefused::Blocked(Block::ReadOnly(_))
    ));
    // Reading the object to check it is allowed; no write and no review.
    assert!(
        cluster.recorder.lines()[before..]
            .iter()
            .all(|l| l.starts_with("GET"))
    );
    assert_eq!(cluster.writes().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn only_a_reversible_action_offers_undo() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::set_image("registry.example/web:2")).await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert!(finished.undo.is_none());
}

#[tokio::test(start_paused = true)]
async fn undo_after_review_puts_back_what_the_new_version_had() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    // Scale 2 -> 5 is prepared; an autoscaler sets 4 before it is sent.
    let action = fake::scale(5);
    cluster.foreign_write(fake::DEPLOYMENT, "hpa", &json!({"spec": {"replicas": 4}}));
    let Verdict::Blocked(Block::Changed {
        what: ChangedWhat::Modified,
        ..
    }) = runner.preflight(action.clone()).await
    else {
        panic!()
    };
    let rebased = runner.review_new_version(action).await.unwrap();
    let finished = run(
        &runner,
        ready(&runner, rebased).await,
        Confirmation::Confirmed,
    )
    .await;
    assert_eq!(cluster.object(fake::DEPLOYMENT)["spec"]["replicas"], 5);
    let offer = finished.undo.unwrap();
    let prepared = runner
        .prepare_undo(&offer, Duration::from_secs(1))
        .await
        .unwrap();
    run(&runner, Box::new(prepared), Confirmation::Confirmed).await;
    // Back to 4, what the new version had, not the 2 the person first saw.
    assert_eq!(cluster.object(fake::DEPLOYMENT)["spec"]["replicas"], 4);
}

#[tokio::test(start_paused = true)]
async fn undo_asks_what_any_write_asks_at_its_level() {
    for (level, asks, typed) in [
        (Protection::Open, false, false),
        (Protection::Guarded, true, false),
        (Protection::Protected, true, true),
    ] {
        let (cluster, dir) = (Cluster::new(), tmp());
        let runner = fake::runner(&cluster, dir.path(), Protection::Open);
        let offer = run(
            &runner,
            ready(&runner, fake::scale(3)).await,
            Confirmation::NotAsked,
        )
        .await
        .undo
        .unwrap();
        let runner = fake::runner(&cluster, dir.path(), level);
        let prepared = runner
            .prepare_undo(&offer, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(
            (
                prepared.requirement().confirm,
                prepared.requirement().type_name
            ),
            (asks, typed)
        );
        // A bare click is not enough where the level asks for more.
        let writes = cluster.writes().len();
        let finished = run(&runner, Box::new(prepared), Confirmation::NotAsked).await;
        assert_eq!(
            matches!(finished.status, Status::NotSent(NotSent::Unconfirmed)),
            asks,
            "{level:?}"
        );
        assert_eq!(cluster.writes().len(), writes + usize::from(!asks));
    }
    // Protected: the object's name unlocks it.
    let (cluster, dir) = (Cluster::new(), tmp());
    let open = fake::runner(&cluster, dir.path(), Protection::Open);
    let offer = run(
        &open,
        ready(&open, fake::scale(3)).await,
        Confirmation::NotAsked,
    )
    .await
    .undo
    .unwrap();
    let protected = fake::runner(&cluster, dir.path(), Protection::Protected);
    let prepared = protected
        .prepare_undo(&offer, Duration::from_secs(1))
        .await
        .unwrap();
    let undone = run(
        &protected,
        Box::new(prepared),
        Confirmation::Typed("web".into()),
    )
    .await;
    assert_eq!(undone.headline(), Headline::Done);
}
