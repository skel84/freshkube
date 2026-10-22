//! The framework end to end on the recording fake: what is sent, in what
//! order, and what is never sent.

use std::time::Duration;

use serde_json::json;

use super::testing::{self as fake, Cluster};
use super::*;

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

async fn ready(runner: &Runner, action: ActionDescription) -> Box<Prepared> {
    match runner.preflight(action).await {
        Verdict::Ready(prepared) => prepared,
        Verdict::Blocked(block) => panic!("blocked: {block:?}"),
    }
}

async fn run(runner: &Runner, prepared: Box<Prepared>, confirmation: Confirmation) -> Finished {
    runner
        .execute(*prepared, confirmation, || false, |_| {})
        .await
}

fn phases(store: &AuditStore) -> Vec<Phase> {
    store
        .read()
        .unwrap()
        .records
        .iter()
        .map(|r| r.phase)
        .collect()
}

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
async fn a_refused_access_review_blocks_before_anything_else_is_asked() {
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.deny_access();
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    match runner.preflight(fake::scale(3)).await {
        Verdict::Blocked(Block::AccessDenied { verb, reason, .. }) => {
            assert_eq!(verb, "patch");
            assert!(reason.contains("RBAC"));
        }
        other => panic!("{other:?}"),
    }
    cluster
        .recorder
        .assert_lines(&["POST /apis/authorization.k8s.io/v1/selfsubjectaccessreviews"]);
}

#[tokio::test(start_paused = true)]
async fn an_access_review_that_fails_is_unknown_not_allowed() {
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.break_access_review();
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    assert!(matches!(
        runner.preflight(fake::scale(3)).await,
        Verdict::Blocked(Block::AccessUnknown { .. })
    ));
    cluster.assert_nothing_written();
}

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
    cluster.refuse_writes(
        409,
        "Operation cannot be fulfilled on deployments.apps \"web\": the object has been modified",
    );
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Changed);
    assert!(finished.undo.is_none());
    // One attempt and no second try, with or without force.
    let writes = cluster.writes();
    assert_eq!(writes.len(), 1);
    assert!(!writes[0].query.contains("force"));
}

#[tokio::test(start_paused = true)]
async fn a_dry_run_the_cluster_refuses_blocks_and_is_audited_but_nothing_is_written() {
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.refuse_dry_runs(
        422,
        "admission webhook \"policy.example\" denied the request",
    );
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    match runner.preflight(fake::set_image("bad")).await {
        Verdict::Blocked(Block::DryRunFailed { message, .. }) => {
            assert!(message.contains("denied"))
        }
        other => panic!("{other:?}"),
    }
    cluster.assert_nothing_written();
    assert_eq!(
        phases(&AuditStore::new(dir.path().join("audit"))),
        [Phase::DryRunFailed]
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
async fn the_sending_line_comes_first_and_if_it_cannot_be_written_nothing_is_sent() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let blocker = dir.path().join("audit");
    std::fs::write(&blocker, "a file where the directory should be").unwrap();
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::scale(3)).await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert!(matches!(
        finished.status,
        Status::NotSent(NotSent::AuditUnavailable(_))
    ));
    cluster.assert_nothing_written();
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
    let events = std::cell::RefCell::new(vec![]);
    let started = tokio::time::Instant::now();
    let finished = runner
        .execute(
            *prepared,
            Confirmation::Confirmed,
            || started.elapsed() >= Duration::from_secs(2),
            |event| events.borrow_mut().push(event),
        )
        .await;
    assert_eq!(finished.status, Status::Cancelled);
    assert_eq!(finished.headline(), Headline::Cancelled);
    assert_eq!(
        events.borrow()[0],
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

#[tokio::test(start_paused = true)]
async fn the_diff_of_a_spec_change_is_the_fields_that_move() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::set_image("registry.example/web:2")).await;
    let diff = &prepared.diffs()[0].changes;
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].path, "spec.template.spec.containers[0].image");
    assert_eq!(diff[0].before.as_deref(), Some("registry.example/web:1"));
    assert_eq!(diff[0].after.as_deref(), Some("registry.example/web:2"));
}

#[tokio::test(start_paused = true)]
async fn a_secret_is_diffed_and_audited_by_key_names_never_values() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::rotate_password()).await;
    let diff = &prepared.diffs()[0].changes;
    assert_eq!(diff[0].path, "data.password");
    assert_eq!(
        (diff[0].before.as_ref(), diff[0].after.as_ref()),
        (None, None)
    );
    run(&runner, prepared, Confirmation::Confirmed).await;
    let text = std::fs::read_to_string(dir.path().join("audit/writes.jsonl")).unwrap();
    assert!(text.contains("data.password"));
    for value in ["aHVudGVyMg", "bmV3cGFzc3dvcmQ", "hunter2"] {
        assert!(!text.contains(value), "{value} leaked into the audit log");
    }
}

#[tokio::test(start_paused = true)]
async fn the_audit_log_names_fields_and_context_and_holds_no_values() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::scale(3)).await;
    run(&runner, prepared, Confirmation::Confirmed).await;
    let read = AuditStore::new(dir.path().join("audit")).read().unwrap();
    let first = &read.records[0];
    assert_eq!(
        (first.entry.as_str(), first.context.as_str()),
        ("prod-fra", "ctx-a")
    );
    assert_eq!(
        (
            first.actor.as_str(),
            first.protection.as_str(),
            first.tier.as_str()
        ),
        ("tester", "guarded", "reversible")
    );
    assert_eq!(first.objects[0].fields, ["spec.replicas"]);
    assert_eq!(first.objects[0].uid, "uid-web");
    let runs: std::collections::BTreeSet<_> = read.records.iter().map(|r| &r.run).collect();
    assert_eq!(runs.len(), 1, "one write is one run");
}

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
    let undone = run(&runner, Box::new(prepared), Confirmation::NotAsked).await;
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
async fn a_prepared_action_is_consumed_so_a_run_is_sent_once() {
    // `execute` takes the prepared action by value: sending it again does not
    // compile. What can be done is to prepare again, which is a new run.
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let first = ready(&runner, fake::scale(3)).await;
    let second = ready(&runner, fake::scale(3)).await;
    assert_ne!(first.run(), second.run());
}
