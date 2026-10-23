use super::*;

#[tokio::test(start_paused = true)]
async fn an_object_that_moved_since_the_person_saw_it_blocks_preflight() {
    for (change, what) in [
        (false, ChangedWhat::Modified),
        (true, ChangedWhat::Recreated),
    ] {
        let (cluster, dir) = (Cluster::new(), tmp());
        if change {
            cluster.recreate(fake::DEPLOYMENT);
        } else {
            cluster.foreign_write(
                fake::DEPLOYMENT,
                "kubectl",
                &json!({"spec": {"paused": true}}),
            );
        }
        let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
        match runner.preflight(fake::scale(3)).await {
            Verdict::Blocked(Block::Changed { what: found, .. }) => assert_eq!(found, what),
            other => panic!("{other:?}"),
        }
        // Never a dry run on an object that is not the one seen.
        assert!(
            cluster
                .recorder
                .lines()
                .iter()
                .all(|l| !l.contains("dryRun"))
        );
        cluster.assert_nothing_written();
    }
}

#[tokio::test(start_paused = true)]
async fn an_object_that_moves_after_preflight_is_refused_by_the_server_not_forced() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::scale(3)).await;
    // Someone writes the object after preflight. The fake enforces the
    // resourceVersion in the body as a real server does; nothing is injected.
    cluster.foreign_write(
        fake::DEPLOYMENT,
        "kubectl",
        &json!({"spec": {"paused": true}}),
    );
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Changed);
    assert_eq!(
        finished.objects[0].step,
        Step::Changed(ChangedWhat::Modified)
    );
    assert!(
        cluster.object(fake::DEPLOYMENT)["spec"]
            .get("replicas")
            .is_some_and(|r| r == 2)
    );
    assert!(finished.undo.is_none());
    // One attempt and no second try, with or without force.
    let writes = cluster.writes();
    assert_eq!(writes.len(), 1);
    assert!(!writes[0].query.contains("force"));
}

#[tokio::test(start_paused = true)]
async fn a_delete_of_an_object_created_again_after_preflight_is_refused_by_its_preconditions() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::delete_web()).await;
    cluster.recreate(fake::DEPLOYMENT);
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Changed);
    assert_eq!(
        finished.objects[0].step,
        Step::Changed(ChangedWhat::Recreated)
    );
    assert!(cluster.exists(fake::DEPLOYMENT));
}

#[tokio::test(start_paused = true)]
async fn a_recreated_object_is_not_something_to_review() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    cluster.recreate(fake::DEPLOYMENT);
    let block = runner.review_new_version(fake::scale(3)).await.unwrap_err();
    assert!(matches!(
        block,
        Block::Changed {
            what: ChangedWhat::Recreated,
            ..
        }
    ));
    let verdict = Verdict::Blocked(block);
    assert!(matches!(
        Dialog::for_verdict(&verdict, "e"),
        Dialog::Changed {
            can_review: false,
            ..
        }
    ));
}
