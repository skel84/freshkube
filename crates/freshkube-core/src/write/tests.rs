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
