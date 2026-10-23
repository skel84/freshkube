use super::*;

#[tokio::test(start_paused = true)]
async fn guarded_scale_asks_then_sends_one_apply_and_is_observed() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::scale(3)).await;
    assert!(prepared.requirement().confirm && prepared.requirement().undo);
    // Preflight changed nothing and sent no write.
    cluster.assert_nothing_written();
    cluster.recorder.assert_lines(&[
        "POST /apis/authorization.k8s.io/v1/selfsubjectaccessreviews",
        "GET /apis/apps/v1/namespaces/shop/deployments/web",
        "PATCH /apis/apps/v1/namespaces/shop/deployments/web?&dryRun=All&fieldManager=freshkube",
    ]);
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Done);
    assert!(finished.succeeded() && finished.undo.is_some());
    let writes = cluster.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    // Server-side apply, our field manager, never force, with the version seen.
    assert_eq!(writes[0].method, "PATCH");
    assert!(writes[0].query.contains("fieldManager=freshkube"));
    assert!(!writes[0].query.contains("force"));
    assert!(writes[0].body.contains("\"resourceVersion\":\"100\""));
    assert_eq!(cluster.object(fake::DEPLOYMENT)["spec"]["replicas"], 3);
    assert_eq!(
        phases(&AuditStore::new(dir.path().join("audit"))),
        [
            Phase::Sending,
            Phase::Accepted,
            Phase::Observed,
            Phase::UndoOffered
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_server_side_apply_conflict_names_the_manager_and_fields_and_is_never_forced() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::scale(3)).await;
    cluster.refuse_writes(
        409,
        "Apply failed with 1 conflict: conflict with \"hpa\" using apps/v1: .spec.replicas",
    );
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Conflict);
    match &finished.objects[0].step {
        Step::Conflict(conflict) => {
            assert_eq!(conflict.managers, ["hpa"]);
            assert_eq!(conflict.fields, [".spec.replicas"]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(cluster.writes().len(), 1);
    assert!(cluster.writes().iter().all(|w| !w.query.contains("force")));
    assert!(finished.undo.is_none());
}

#[tokio::test(start_paused = true)]
async fn the_rest_of_a_write_is_not_sent_after_one_object_fails() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let mut action = fake::scale_and_delete();
    action.tier = Tier::Reversible;
    let prepared = ready(&runner, action).await;
    cluster.refuse_writes(
        409,
        "Apply failed with 1 conflict: conflict with \"hpa\" using apps/v1: .spec.replicas",
    );
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.objects[1].step, Step::NotAttempted);
    assert_eq!(cluster.writes().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_write_with_no_answer_is_unknown_and_is_never_resent() {
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.drop_the_connection_on_writes();
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::scale(3)).await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Unknown);
    assert!(!finished.succeeded() && finished.undo.is_none());
    assert_eq!(cluster.writes().len(), 1, "no replay");
    assert_eq!(
        phases(&AuditStore::new(dir.path().join("audit"))),
        [Phase::Sending, Phase::Unknown]
    );
}

#[tokio::test(start_paused = true)]
async fn a_prepared_action_is_consumed_so_a_run_is_sent_once() {
    // `execute` takes the prepared action by value: sending it again does not
    // compile. What can be done is to prepare again, which is a new run.
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let first = ready(&runner, fake::scale(3)).await;
    let second = ready(&runner, fake::scale(3)).await;
    assert_ne!(first.run(), second.run());
}
