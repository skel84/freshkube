use super::*;

// ---- Review of #855: a create reads what it depends on ----

#[tokio::test(start_paused = true)]
async fn a_create_reads_the_object_it_depends_on_and_asks_about_its_verb() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let _ = ready(&runner, fake::create_note_after_web()).await;
    cluster.assert_nothing_written();
    let lines = cluster.recorder.lines();
    assert_eq!(
        lines,
        [
            "POST /apis/authorization.k8s.io/v1/selfsubjectaccessreviews",
            "POST /apis/authorization.k8s.io/v1/selfsubjectaccessreviews",
            "GET /apis/apps/v1/namespaces/shop/deployments/web",
            "POST /api/v1/namespaces/shop/configmaps?&dryRun=All&fieldManager=freshkube",
        ]
    );
    let bodies: Vec<String> = cluster
        .recorder
        .requests()
        .iter()
        .take(2)
        .map(|r| r.body.clone())
        .collect();
    assert!(bodies[0].contains("\"verb\":\"create\""));
    assert!(bodies[1].contains("\"verb\":\"promote\"") && bodies[1].contains("\"name\":\"web\""));
}

#[tokio::test(start_paused = true)]
async fn a_dependency_that_moved_blocks_the_create_and_cannot_be_reviewed_away() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Open);
    // The page showed two replicas; someone has scaled it since.
    cluster.foreign_write(
        fake::DEPLOYMENT,
        "kubectl",
        &json!({"spec": {"replicas": 5}}),
    );
    let verdict = runner.preflight(fake::create_note_after_web()).await;
    assert!(matches!(
        &verdict,
        Verdict::Blocked(Block::Changed {
            what: ChangedWhat::Moved,
            ..
        })
    ));
    // It offers to close, not to review: reading it again would not help.
    assert!(matches!(
        Dialog::for_verdict(&verdict, "prod-fra"),
        Dialog::Changed {
            can_review: false,
            ..
        }
    ));
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn a_recreated_or_deleted_dependency_blocks_the_create() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    cluster.recreate(fake::DEPLOYMENT);
    assert!(matches!(
        runner.preflight(fake::create_note_after_web()).await,
        Verdict::Blocked(Block::Changed {
            what: ChangedWhat::Recreated,
            ..
        })
    ));
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn a_verb_the_dependency_refuses_blocks_the_create_before_anything_is_written() {
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.deny_verb("promote");
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    match runner.preflight(fake::create_note_after_web()).await {
        Verdict::Blocked(Block::AccessDenied { verb, object, .. }) => {
            assert_eq!((verb, object.as_str()), ("promote", "Deployment/web"));
        }
        other => panic!("{other:?}"),
    }
    cluster.assert_nothing_written();
    assert!(
        cluster
            .recorder
            .lines()
            .iter()
            .all(|l| !l.contains("dryRun")),
        "no dry run after a refusal"
    );
}

#[tokio::test(start_paused = true)]
async fn a_dependency_that_moves_after_the_dialog_stops_the_send() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::create_note_after_web()).await;
    // The dialog is open while someone else changes what the page showed.
    cluster.foreign_write(
        fake::DEPLOYMENT,
        "kubectl",
        &json!({"spec": {"replicas": 5}}),
    );
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    match &finished.status {
        Status::NotSent(NotSent::Moved(what)) => {
            assert!(what.contains("no longer as the page showed it"), "{what}")
        }
        other => panic!("{other:?}"),
    }
    assert!(finished.message().contains("nothing was sent"));
    cluster.assert_nothing_written();
    assert!(phases(&AuditStore::new(dir.path().join("audit"))).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_dependency_that_is_unchanged_is_checked_again_and_the_send_goes_ahead() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::create_note_after_web()).await;
    let reads_before = cluster.recorder.requests().len();
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Done);
    // The Stage-like object was read again just before the send.
    let after: Vec<String> = cluster
        .recorder
        .lines()
        .into_iter()
        .skip(reads_before)
        .collect();
    let read = after
        .iter()
        .position(|l| l == "GET /apis/apps/v1/namespaces/shop/deployments/web")
        .expect("read again");
    let send = after
        .iter()
        .position(|l| l.starts_with("POST /api/v1/namespaces/shop/configmaps"))
        .expect("sent");
    assert!(read < send, "{after:?}");
}

#[tokio::test(start_paused = true)]
async fn a_conflict_answering_a_read_is_no_answer_not_a_panic() {
    use crate::fake_api;
    let (client, _recorder) = fake_api::client(|request| {
        if request.path.ends_with("/selfsubjectaccessreviews") {
            return Some(fake_api::json(
                201,
                json!({"apiVersion": "authorization.k8s.io/v1", "kind": "SelfSubjectAccessReview",
                    "metadata": {}, "spec": {}, "status": {"allowed": true}}),
            ));
        }
        // A proxy answers every read with a 409 that reads as a conflict.
        Some(fake_api::json(
            409,
            json!({"kind": "Status", "apiVersion": "v1", "status": "Failure", "code": 409,
                "reason": "Conflict",
                "message": "Apply failed with 1 conflict: conflict with \"proxy\" using v1: .spec.x"}),
        ))
    });
    let dir = tmp();
    let runner = Runner::new(
        client,
        AuditStore::new(dir.path().join("audit")),
        Who {
            entry: "e".into(),
            context: "c".into(),
            protection: Protection::Guarded,
            actor: "t".into(),
        },
    );
    // Both the dependency's read and an ordinary object's read.
    for action in [fake::create_note_after_web(), fake::scale(3)] {
        match runner.preflight(action).await {
            Verdict::Blocked(Block::NoAnswer { message, .. }) => {
                assert!(message.contains("conflict"), "{message}")
            }
            other => panic!("{other:?}"),
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_recheck_that_could_not_be_made_says_so_and_sends_nothing() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::create_note_after_web()).await;
    // The access review fails when asked again: nothing is known.
    cluster.break_access_review();
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    match &finished.status {
        Status::NotSent(NotSent::Unchecked(text)) => {
            assert!(text.contains("couldn't be checked again"), "{text}");
            assert!(!text.contains("changed after you were asked"), "{text}");
        }
        other => panic!("{other:?}"),
    }
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn a_cancel_pressed_during_the_recheck_is_honoured() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::create_note_after_web()).await;
    // Not cancelled when first asked; cancelled by the time the recheck is done.
    let asked = AtomicUsize::new(0);
    let finished = runner
        .execute(
            *prepared,
            Confirmation::Confirmed,
            || asked.fetch_add(1, Ordering::SeqCst) >= 1,
            |_| {},
        )
        .await;
    assert_eq!(finished.status, Status::Cancelled);
    cluster.assert_nothing_written();
}
