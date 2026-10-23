use super::*;

// ---- #855: create, the typed name for several subjects, no confirmation ----

#[tokio::test(start_paused = true)]
async fn a_create_has_no_object_to_get_and_is_checked_by_review_and_dry_run() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::create_notes(&["stage-a"])).await;
    // No GET of the new object (it would be a 404), no write; the dry run is
    // a POST to the collection, and the access review names no object.
    cluster.assert_nothing_written();
    cluster.recorder.assert_lines(&[
        "POST /apis/authorization.k8s.io/v1/selfsubjectaccessreviews",
        "POST /api/v1/namespaces/shop/configmaps?&dryRun=All&fieldManager=freshkube",
    ]);
    assert!(
        cluster.recorder.requests()[0]
            .body
            .contains("\"verb\":\"create\""),
        "the review asks about create"
    );
    assert!(!cluster.recorder.requests()[0].body.contains("stage-a.1"));
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Done);
    let writes = cluster.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0].method, "POST");
    assert!(writes[0].query.contains("fieldManager=freshkube"));
    // A new object carries no version to have seen.
    assert!(!writes[0].body.contains("resourceVersion"));
    assert!(cluster.exists("/api/v1/namespaces/shop/configmaps/stage-a.1"));
}

#[tokio::test(start_paused = true)]
async fn a_create_whose_name_is_taken_is_refused_by_the_dry_run_and_sends_nothing() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Open);
    let first = ready(&runner, fake::create_notes(&["stage-a"])).await;
    run(&runner, first, Confirmation::Confirmed).await;
    let writes = cluster.writes().len();
    match runner.preflight(fake::create_notes(&["stage-a"])).await {
        Verdict::Blocked(Block::DryRunFailed { message, .. }) => {
            assert!(message.contains("already exists"), "{message}")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(cluster.writes().len(), writes);
}

#[tokio::test(start_paused = true)]
async fn a_create_that_is_denied_is_blocked_before_the_dry_run() {
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.deny_access();
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    match runner.preflight(fake::create_notes(&["stage-a"])).await {
        Verdict::Blocked(Block::AccessDenied { verb, .. }) => assert_eq!(verb, "create"),
        other => panic!("{other:?}"),
    }
    assert_eq!(cluster.recorder.requests().len(), 1);
}
