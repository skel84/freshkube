use super::*;

#[tokio::test(start_paused = true)]
async fn an_accepted_write_that_never_arrives_says_not_observed_after_the_wait() {
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.never_reaches_the_state();
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::scale(3)).await;
    let started = tokio::time::Instant::now();
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::NotObserved);
    assert!(!finished.succeeded());
    assert!(
        started.elapsed() >= Duration::from_secs(30) && started.elapsed() < Duration::from_secs(60)
    );
    match &finished.objects[0].step {
        Step::Accepted {
            observed: Observed::NotObserved { waited, .. },
            ..
        } => {
            assert_eq!(*waited, Duration::from_secs(30))
        }
        other => panic!("{other:?}"),
    }
    let phases = phases(&AuditStore::new(dir.path().join("audit")));
    assert!(phases.contains(&Phase::Accepted) && phases.contains(&Phase::NotObserved));
    assert!(!phases.contains(&Phase::Observed));
}

#[tokio::test(start_paused = true)]
async fn only_an_observed_success_is_allowed_to_go_away_by_itself() {
    // Observed: not sticky.
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let done = run(
        &runner,
        ready(&runner, fake::scale(3)).await,
        Confirmation::Confirmed,
    )
    .await;
    assert!(!done.sticky());
    assert!(done.message().starts_with("Done."));
    // Not observed, unknown, conflict, cancelled, not sent: all stay.
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.never_reaches_the_state();
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let stuck = run(
        &runner,
        ready(&runner, fake::scale(3)).await,
        Confirmation::Confirmed,
    )
    .await;
    assert!(stuck.sticky());
    assert!(
        stuck
            .message()
            .contains("not seen to take effect after 30 s")
    );
    assert!(!stuck.message().to_lowercase().contains("done"));
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.drop_the_connection_on_writes();
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let unknown = run(
        &runner,
        ready(&runner, fake::scale(3)).await,
        Confirmation::Confirmed,
    )
    .await;
    assert!(unknown.sticky() && unknown.message().contains("not known whether it accepted"));
    assert!(!unknown.message().to_lowercase().contains("done"));
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let unconfirmed = run(
        &runner,
        ready(&runner, fake::scale(3)).await,
        Confirmation::NotAsked,
    )
    .await;
    assert!(unconfirmed.sticky() && unconfirmed.message().contains("Nothing was sent"));
}

#[test]
fn a_partial_write_names_each_objects_own_outcome() {
    let accepted = |observed| Step::Accepted {
        accepted: Accepted {
            resource_version: Some("2".into()),
        },
        observed,
    };
    let finished = Finished {
        run: RunId::new(),
        action: "x".into(),
        status: Status::Sent,
        objects: vec![
            ObjectResult {
                object: "Deployment/web".into(),
                step: accepted(Observed::Reached),
            },
            ObjectResult {
                object: "Secret/db".into(),
                step: Step::Unknown("reset".into()),
            },
        ],
        undo: None,
        audit_error: None,
    };
    assert_eq!(finished.headline(), Headline::Partial);
    let message = finished.message();
    assert!(
        message.contains("Deployment/web: accepted and seen to take effect"),
        "{message}"
    );
    assert!(
        message.contains("Secret/db: no answer; not known whether the cluster took it"),
        "{message}"
    );
    assert!(!message.contains("were not changed"));
}

#[tokio::test(start_paused = true)]
async fn an_action_with_nothing_to_watch_is_told_plainly_and_does_not_stay() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Open);
    let mut action = fake::annotate_web();
    action.changes[0].observe = Observation::Unobservable;
    let finished = run(
        &runner,
        ready(&runner, action).await,
        Confirmation::NotAsked,
    )
    .await;
    assert_eq!(finished.headline(), Headline::NotObserved);
    assert!(!finished.sticky(), "nothing to watch is not a doubt");
    assert!(
        finished.message().contains("can't watch"),
        "{}",
        finished.message()
    );
    // A write whose audit line failed always stays, whatever its headline.
    let mut unlogged = finished.clone();
    unlogged.audit_error = Some("the log is full".into());
    assert!(unlogged.sticky());
    // An accepted write that was watched and never arrived still stays.
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.never_reaches_the_state();
    let runner = fake::runner(&cluster, dir.path(), Protection::Open);
    let stuck = run(
        &runner,
        ready(&runner, fake::scale(3)).await,
        Confirmation::NotAsked,
    )
    .await;
    assert_eq!(stuck.headline(), Headline::NotObserved);
    assert!(stuck.sticky());
}

#[tokio::test(start_paused = true)]
async fn a_cancel_leaves_no_doubt_so_its_report_goes_away_like_done() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::delete_web()).await;
    let finished = runner
        .execute(*prepared, Confirmation::Confirmed, || true, |_| {})
        .await;
    assert_eq!(finished.headline(), Headline::Cancelled);
    assert!(!finished.sticky());
    // A cancel records no audit result in `Finished` (`run.rs` drops it), so
    // there is no failed-audit cancel to keep.
    assert_eq!(finished.audit_error, None);
}
