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

#[tokio::test(start_paused = true)]
async fn the_diff_of_a_spec_change_is_the_fields_that_move() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::set_image("registry.example/web:2")).await;
    let diff = &prepared.diffs()[0].changes;
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].path, "spec.template.spec.containers[0].image");
    assert_eq!(diff[0].kind, ChangeKind::Changed);
    // Paths and kinds only, never what the fields hold.
    assert!(!format!("{diff:?}").contains("registry.example"));
}

#[tokio::test(start_paused = true)]
async fn a_secret_is_diffed_and_audited_by_key_names_never_values() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::rotate_password()).await;
    let diff = &prepared.diffs()[0].changes;
    assert_eq!(diff[0].path, "data.password");
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
async fn a_prepared_action_is_consumed_so_a_run_is_sent_once() {
    // `execute` takes the prepared action by value: sending it again does not
    // compile. What can be done is to prepare again, which is a new run.
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let first = ready(&runner, fake::scale(3)).await;
    let second = ready(&runner, fake::scale(3)).await;
    assert_ne!(first.run(), second.run());
}

// ---- The dialog each state and level asks for ----

async fn dialog_for(level: Protection, action: ActionDescription) -> Dialog {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), level);
    Dialog::for_verdict(&runner.preflight(action).await, "prod-fra")
}

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
async fn the_audit_line_records_the_level_at_send_time() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let level = LevelCell::new(Protection::Guarded);
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded).following(level.clone());
    let prepared = ready(&runner, fake::scale(3)).await;
    // Looser now: the stricter requirement it was prepared under still
    // applies, and the line says what the level was when it was sent.
    level.set(Protection::Open);
    run(&runner, prepared, Confirmation::Confirmed).await;
    let read = AuditStore::new(dir.path().join("audit")).read().unwrap();
    assert!(
        read.records.iter().all(|r| r.protection == "open"),
        "{:?}",
        read.records
    );
}

#[tokio::test(start_paused = true)]
async fn an_error_that_echoes_a_value_reaches_neither_the_dialog_nor_the_audit_log() {
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.refuse_dry_runs(
        422,
        "Deployment.apps \"web\" is invalid: spec.template.spec.containers[0].env[0].value: Invalid value: \"hunter2-pass\": must not contain a secret",
    );
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let verdict = runner.preflight(fake::set_image("x")).await;
    let Dialog::Refused { text } = Dialog::for_verdict(&verdict, "e") else {
        panic!("{verdict:?}")
    };
    assert!(
        text.contains("env[0].value") && !text.contains("hunter2"),
        "{text}"
    );
    let log = std::fs::read_to_string(dir.path().join("audit/writes.jsonl")).unwrap();
    assert!(!log.contains("hunter2"), "{log}");
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
async fn an_example_runner_shows_each_level_s_dialog_and_makes_no_request() {
    use Protection::*;
    for (level, asks, typed) in [(Guarded, true, false), (Protected, true, true)] {
        let (cluster, dir) = (Cluster::new(), tmp());
        let runner = Runner::example(
            cluster.client.clone(),
            AuditStore::new(dir.path().join("audit")),
            Who {
                entry: "prod-fra".into(),
                context: "ctx".into(),
                protection: level,
                actor: "tester".into(),
            },
        );
        let verdict = runner.preflight(fake::create_notes(&["stage-a"])).await;
        let Dialog::Ask(ask) = Dialog::for_verdict(&verdict, "prod-fra") else {
            panic!("{level:?} asks: {asks}");
        };
        assert_eq!(ask.type_name.is_some(), typed);
        let Verdict::Ready(prepared) = verdict else {
            panic!()
        };
        let finished = runner
            .execute(
                *prepared,
                Confirmation::Typed("stage-a".into()),
                || false,
                |_| {},
            )
            .await;
        assert_eq!(finished.status, Status::NotSent(NotSent::Example));
        assert!(finished.message().contains("Example data"));
        // Not a request of any kind, and nothing audited.
        assert!(cluster.recorder.requests().is_empty(), "{level:?}");
        assert!(phases(&AuditStore::new(dir.path().join("audit"))).is_empty());
    }
    // Read-only still blocks, as for any runner.
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = Runner::example(
        cluster.client.clone(),
        AuditStore::new(dir.path().join("audit")),
        Who {
            entry: "e".into(),
            context: "c".into(),
            protection: ReadOnly,
            actor: "t".into(),
        },
    );
    assert!(matches!(
        runner.preflight(fake::annotate_web()).await,
        Verdict::Blocked(Block::ReadOnly(_))
    ));
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

#[tokio::test(start_paused = true)]
async fn a_cancel_on_example_data_writes_nothing_to_any_audit_log() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = Runner::example(
        cluster.client.clone(),
        AuditStore::new(dir.path().join("audit")),
        Who {
            entry: "e".into(),
            context: "c".into(),
            protection: Protection::Guarded,
            actor: "t".into(),
        },
    );
    let prepared = ready(&runner, fake::delete_web()).await;
    // Cancelled during the countdown.
    let finished = runner
        .execute(*prepared, Confirmation::Confirmed, || true, |_| {})
        .await;
    assert_eq!(finished.status, Status::Cancelled);
    assert!(cluster.recorder.requests().is_empty());
    assert!(phases(&AuditStore::new(dir.path().join("audit"))).is_empty());
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
