//! The everyday actions (#857) in a real window against core's fake API: each
//! one at each level, what it sends, that Restart has no Undo, the GitOps
//! warning, and that example data sends nothing.

use std::sync::Arc;
use std::time::Duration;

use freshkube_core::write::everyday::{self, Row, Subject};
use freshkube_core::write::testing::{self as fake, Cluster};
use freshkube_core::write::{ActionDescription, AuditStore, Protection, Runner, Who};
use gpui_kit::{AppContext, TestAppContext};
use serde_json::json;

use super::WriteRequest;
use super::tests::{Harness, harness, harness_on};
use crate::mutation::level_cell;

fn subject(h: &Harness, path: &str) -> Subject {
    Subject::from_object(&h.cluster.object(path)).unwrap()
}

fn scale_to(h: &Harness, count: u64) -> ActionDescription {
    everyday::scale(&subject(h, fake::DEPLOYMENT), count).unwrap()
}

fn restart(h: &Harness) -> ActionDescription {
    everyday::restart(&subject(h, fake::DEPLOYMENT), "2026-10-23T10:00:00Z").unwrap()
}

fn suspend(h: &Harness) -> ActionDescription {
    everyday::suspend(&subject(h, fake::CRONJOB)).unwrap()
}

/// The Deployment `web` as the page read it, with its revisions.
fn rollback_to(h: &Harness, revision: u64) -> ActionDescription {
    let row = Row {
        group: "apps".into(),
        version: "v1".into(),
        kind: "Deployment".into(),
        resource: "deployments".into(),
        namespace: Some("shop".into()),
        name: "web".into(),
        uid: "uid-web".into(),
    };
    let subject = h
        .runtime
        .block_on(Subject::read(&h.cluster.client, &row))
        .unwrap();
    everyday::rollback(&subject, revision).unwrap()
}

fn run_now(h: &Harness) -> ActionDescription {
    everyday::run_now(&subject(h, fake::CRONJOB), "k7x2q").unwrap()
}

#[gpui_kit::test]
fn guarded_scale_confirms_sends_one_apply_and_offers_undo(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.begin(cx, scale_to(&h, 5));
    assert!(h.present(cx, "write-confirm") && !h.present(cx, "write-type-name"));
    assert!(h.label(cx, "write-sentence").contains("from 2 to 5"));
    h.cluster.assert_nothing_written();
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);
    assert_eq!(h.replicas(), 5);
    assert!(h.label(cx, "write-toast-status").starts_with("Done."));
    assert!(h.present(cx, "write-undo"));
}

#[gpui_kit::test]
fn safety_g02_protected_scale_sends_only_after_the_name_is_typed(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Protected);
    h.begin(cx, scale_to(&h, 5));
    assert_eq!(h.label(cx, "write-type-prompt"), "Type web to confirm");
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(1));
    h.cluster.assert_nothing_written();
    h.type_name(cx, "web");
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.replicas(), 5);
}

#[gpui_kit::test]
fn open_still_asks_before_a_restart_and_it_has_no_undo(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Open);
    h.begin(cx, restart(&h));
    // No way back, so no skipped confirmation, even on an open cluster.
    assert!(h.present(cx, "write-dialog") && h.present(cx, "write-confirm"));
    assert!(
        h.label(cx, "write-sentence")
            .contains("This can't be undone")
    );
    h.cluster.assert_nothing_written();
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);
    let annotations =
        &h.cluster.object(fake::DEPLOYMENT)["spec"]["template"]["metadata"]["annotations"];
    assert_eq!(
        annotations["kubectl.kubernetes.io/restartedAt"],
        "2026-10-23T10:00:00Z"
    );
    assert!(!h.present(cx, "write-undo"));
}

#[gpui_kit::test]
fn open_sends_a_scale_without_asking_and_says_so(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Open);
    h.begin(cx, scale_to(&h, 4));
    assert!(!h.present(cx, "write-dialog"));
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.replicas(), 4);
    assert_eq!(
        h.label(cx, "write-toast-notice"),
        "Sent without asking because this cluster is open."
    );
    assert!(h.present(cx, "write-undo"));
}

#[gpui_kit::test]
fn safety_g01_read_only_blocks_every_everyday_action_and_makes_no_request(cx: &mut TestAppContext) {
    for pick in 0..5 {
        let h = harness_on(cx, Protection::ReadOnly, Cluster::with_history);
        let action = match pick {
            0 => scale_to(&h, 3),
            1 => restart(&h),
            2 => suspend(&h),
            3 => rollback_to(&h, 1),
            _ => run_now(&h),
        };
        // Reading the object for the description is the test's own; what the
        // write flow adds after it must be nothing.
        let read = h.cluster.recorder.requests().len();
        h.begin(cx, action);
        assert_eq!(h.label(cx, "write-disabled"), "This cluster is read-only.");
        assert_eq!(h.cluster.recorder.requests().len(), read);
    }
}

#[gpui_kit::test]
fn protected_suspend_types_the_cronjob_and_undo_resumes(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Protected);
    h.begin(cx, suspend(&h));
    assert_eq!(h.label(cx, "write-type-prompt"), "Type nightly to confirm");
    h.type_name(cx, "nightly");
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.object(fake::CRONJOB)["spec"]["suspend"], true);
    assert!(h.present(cx, "write-undo"));
}

#[gpui_kit::test]
fn a_gitops_managed_object_says_so_in_the_dialog(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.cluster.foreign_write(
        fake::DEPLOYMENT,
        "argocd",
        &json!({"metadata": {"annotations": {"argocd.argoproj.io/tracking-id": "shop:web"}}}),
    );
    h.begin(cx, scale_to(&h, 4));
    let sentence = h.label(cx, "write-sentence");
    assert!(
        sentence.contains("Argo CD manages this object"),
        "{sentence}"
    );
    assert!(sentence.contains("OutOfSync"), "{sentence}");
}

#[gpui_kit::test]
fn example_data_shows_the_dialog_and_sends_nothing(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    let cell = cx.update(level_cell);
    let who = Who {
        entry: "prod-fra".into(),
        context: "ctx-a".into(),
        protection: Protection::Guarded,
        actor: "tester".into(),
    };
    let audit = AuditStore::new(h._dir.path().join("example-audit"));
    let runner = Arc::new(Runner::example(h.cluster.client.clone(), audit, who).following(cell));
    let request = WriteRequest {
        runner,
        runtime: h.runtime.handle().clone(),
        action: scale_to(&h, 5),
        undo: None,
    };
    cx.update_window(h.handle, |_, window, cx| super::start(request, window, cx))
        .unwrap();
    h.drive(cx, Duration::from_millis(10));
    assert!(h.present(cx, "write-confirm"));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert!(h.cluster.recorder.requests().is_empty());
    assert_eq!(h.replicas(), 2);
    assert!(
        h.label(cx, "write-toast-status")
            .to_lowercase()
            .contains("example data")
    );
    assert!(h.cluster.writes().is_empty());
}

#[gpui_kit::test]
fn the_x_is_hidden_while_a_destructive_write_counts_down(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.begin(cx, fake::delete_web());
    // Before confirming, the ✕ closes the dialog like Cancel does.
    assert!(h.present(cx, "close"));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_millis(1500));
    assert!(h.present(cx, "write-cancel-countdown"));
    // In the cancel window only its own Cancel (or the send) ends the
    // dialog: a ✕ would close it and leave the write running.
    assert!(!h.present(cx, "close"));
    h.cluster.assert_nothing_written();
}

#[gpui_kit::test]
fn guarded_roll_back_shows_the_diff_asks_once_and_has_no_undo(cx: &mut TestAppContext) {
    let h = harness_on(cx, Protection::Guarded, Cluster::with_history);
    h.begin(cx, rollback_to(&h, 1));
    assert!(h.present(cx, "write-confirm") && !h.present(cx, "write-type-name"));
    assert!(h.label(cx, "write-sentence").contains("to revision 1"));
    // The dry run's diff: paths, never the images or the env value.
    assert!(h.present(cx, "write-diff"));
    assert!(h.present(cx, "write-diff-spec.template.spec.containers[0].image"));
    h.cluster.assert_nothing_written();
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);
    let template = &h.cluster.object(fake::DEPLOYMENT)["spec"]["template"];
    assert_eq!(
        template["spec"]["containers"][0]["image"],
        "registry.example/web:1"
    );
    assert!(h.label(cx, "write-toast-status").starts_with("Done."));
    assert!(!h.present(cx, "write-undo"));
}

#[gpui_kit::test]
fn protected_roll_back_types_the_name_and_open_still_asks(cx: &mut TestAppContext) {
    let h = harness_on(cx, Protection::Protected, Cluster::with_history);
    h.begin(cx, rollback_to(&h, 2));
    assert_eq!(h.label(cx, "write-type-prompt"), "Type web to confirm");
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(1));
    h.cluster.assert_nothing_written();
    h.type_name(cx, "web");
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);

    let h = harness_on(cx, Protection::Open, Cluster::with_history);
    h.begin(cx, rollback_to(&h, 2));
    assert!(h.present(cx, "write-confirm") && h.present(cx, "write-diff"));
    assert!(!h.present(cx, "write-type-name"));
    h.cluster.assert_nothing_written();
}

#[gpui_kit::test]
fn safety_g11_run_now_asks_at_every_level_with_a_preview_and_no_typing_or_countdown(
    cx: &mut TestAppContext,
) {
    for level in [Protection::Protected, Protection::Guarded, Protection::Open] {
        let h = harness(cx, level);
        h.begin(cx, run_now(&h));
        assert!(h.present(cx, "write-confirm"), "{level:?}");
        assert!(!h.present(cx, "write-type-name"), "{level:?}");
        let preview = h.label(cx, "write-preview-0");
        assert!(
            preview.contains("Job nightly-manual-k7x2q in shop"),
            "{preview}"
        );
        assert!(preview.contains("registry.example/report:3"), "{preview}");
        assert!(!preview.contains("s3cr3t-token"), "{preview}");
        h.cluster.assert_nothing_written();
        h.click(cx, "write-confirm");
        h.drive(cx, Duration::from_secs(3));
        // One create, at once: no cancel window, and nothing to undo.
        assert_eq!(h.cluster.writes().len(), 1, "{level:?}");
        assert!(
            h.cluster
                .exists("/apis/batch/v1/namespaces/shop/jobs/nightly-manual-k7x2q")
        );
        assert!(!h.present(cx, "write-undo"), "{level:?}");
    }
}

#[gpui_kit::test]
fn a_gitops_managed_cronjob_says_the_job_is_made_beside_it(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.cluster.foreign_write(
        fake::CRONJOB,
        "argocd-controller",
        &json!({"metadata": {"labels": {"app": "nightly"}}}),
    );
    h.begin(cx, run_now(&h));
    let sentence = h.label(cx, "write-sentence");
    assert!(
        sentence.contains("Argo CD manages this CronJob"),
        "{sentence}"
    );
}
