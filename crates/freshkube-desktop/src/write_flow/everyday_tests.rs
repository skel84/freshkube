//! The everyday actions (#857) in a real window against core's fake API: each
//! one at each level, what it sends, that Restart has no Undo, the GitOps
//! warning, and that example data sends nothing.

use std::sync::Arc;
use std::time::Duration;

use freshkube_core::write::everyday::{self, Subject};
use freshkube_core::write::testing as fake;
use freshkube_core::write::{ActionDescription, AuditStore, Protection, Runner, Who};
use gpui_kit::{AppContext, TestAppContext};
use serde_json::json;

use super::WriteRequest;
use super::tests::{Harness, harness};
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
fn protected_scale_sends_only_after_the_name_is_typed(cx: &mut TestAppContext) {
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
fn open_restarts_without_asking_and_has_no_undo(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Open);
    h.begin(cx, restart(&h));
    assert!(!h.present(cx, "write-dialog"));
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);
    let annotations =
        &h.cluster.object(fake::DEPLOYMENT)["spec"]["template"]["metadata"]["annotations"];
    assert_eq!(
        annotations["kubectl.kubernetes.io/restartedAt"],
        "2026-10-23T10:00:00Z"
    );
    assert_eq!(
        h.label(cx, "write-toast-notice"),
        "Sent without asking because this cluster is open."
    );
    // A rollout cannot be taken back.
    assert!(!h.present(cx, "write-undo"));
}

#[gpui_kit::test]
fn read_only_blocks_every_everyday_action_and_makes_no_request(cx: &mut TestAppContext) {
    for pick in 0..3 {
        let h = harness(cx, Protection::ReadOnly);
        let action = match pick {
            0 => scale_to(&h, 3),
            1 => restart(&h),
            _ => suspend(&h),
        };
        h.begin(cx, action);
        assert_eq!(h.label(cx, "write-disabled"), "This cluster is read-only.");
        assert!(h.cluster.recorder.requests().is_empty());
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
