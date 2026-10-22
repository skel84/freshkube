//! The write flow in a real window against core's recording fake API: each
//! state of the dialog at each level, and that a blocked, cancelled or
//! unconfirmed action sends no request. The runner runs on a paused
//! current-thread runtime that the test steps, so nothing waits on a clock.

use std::sync::Arc;
use std::time::Duration;

use freshkube_core::write::testing::{self as fake, Cluster};
use freshkube_core::write::{ActionDescription, Protection, Runner};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Context, Empty, Entity, IntoElement, Render, TestAppContext,
    Window, px, size,
};
use tokio::runtime::{Builder, Runtime};

use super::{WriteFlow, WriteRequest, start};
use crate::mutation::{Operations, level_cell, set_protection};

struct Host;

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub(super) struct Harness {
    pub(super) runtime: Runtime,
    pub(super) handle: AnyWindowHandle,
    pub(super) cluster: Cluster,
    pub(super) runner: Arc<Runner>,
    pub(super) _dir: tempfile::TempDir,
}

fn harness(cx: &mut TestAppContext, level: Protection) -> Harness {
    harness_on(cx, level, Cluster::new)
}

/// The harness over the cluster `make` builds, inside its runtime.
pub(super) fn harness_on(
    cx: &mut TestAppContext,
    level: Protection,
    make: impl FnOnce() -> Cluster,
) -> Harness {
    harness_sized(cx, level, 880., make)
}

/// [`harness_on`] in a window `height` points tall: a dialog with a long
/// preview scrolls its body, and a click needs the control in view.
pub(super) fn harness_sized(
    cx: &mut TestAppContext,
    level: Protection,
    height: f32,
    make: impl FnOnce() -> Cluster,
) -> Harness {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        // Dialogs animate on the real clock; reduced motion closes them at once.
        cx.set_reduce_motion(true);
        set_protection(level, cx);
    });
    let dir = tempfile::tempdir().unwrap();
    // The client's buffer worker spawns onto the runtime it is built in.
    let cluster = {
        let _guard = runtime.enter();
        make()
    };
    let cell = cx.update(level_cell);
    let runner = Arc::new(fake::runner(&cluster, dir.path(), level).following(cell));
    let handle = cx
        .open_window(size(px(1280.), px(height)), |window, cx| {
            Root::new(cx.new(|_| Host), window, cx)
        })
        .into();
    cx.run_until_parked();
    Harness {
        runtime,
        handle,
        cluster,
        runner,
        _dir: dir,
    }
}

impl Harness {
    pub(super) fn begin(
        &self,
        cx: &mut TestAppContext,
        action: ActionDescription,
    ) -> Entity<WriteFlow> {
        let request = WriteRequest {
            runner: self.runner.clone(),
            runtime: self.runtime.handle().clone(),
            action,
            undo: None,
        };
        let flow = cx
            .update_window(self.handle, |_, window, cx| start(request, window, cx))
            .unwrap();
        self.drive(cx, Duration::from_millis(10));
        flow
    }

    /// Lets the runtime run for `time` of its virtual clock, and GPUI take
    /// what it sent.
    pub(super) fn drive(&self, cx: &mut TestAppContext, time: Duration) {
        let step = Duration::from_millis(250).min(time);
        let mut left = time;
        loop {
            self.runtime
                .block_on(async { tokio::time::sleep(step).await });
            cx.run_until_parked();
            left = left.saturating_sub(step);
            if left.is_zero() {
                break;
            }
        }
        cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    pub(super) fn present(&self, cx: &mut TestAppContext, id: &'static str) -> bool {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    pub(super) fn label(&self, cx: &mut TestAppContext, id: &'static str) -> String {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .find(id)
                .label()
                .map(str::to_owned)
                .unwrap_or_default()
        })
        .unwrap()
    }

    pub(super) fn click(&self, cx: &mut TestAppContext, id: &'static str) {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    pub(super) fn type_name(&self, cx: &mut TestAppContext, text: &'static str) {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("write-type-name", cx);
            window.input(text, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    pub(super) fn advance(&self, cx: &mut TestAppContext, time: Duration) {
        cx.executor().advance_clock(time);
        cx.run_until_parked();
    }

    pub(super) fn slot_free(&self, cx: &mut TestAppContext) -> bool {
        cx.update(|cx| Operations::current(cx).is_none())
    }

    fn replicas(&self) -> i64 {
        self.cluster.object(fake::DEPLOYMENT)["spec"]["replicas"]
            .as_i64()
            .unwrap()
    }
}

#[gpui_kit::test]
fn guarded_asks_first_and_sending_waits_for_the_confirm(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.begin(cx, fake::scale(3));
    assert!(h.present(cx, "write-dialog"));
    assert!(h.present(cx, "write-confirm"));
    // The dialog is up, access was reviewed, the dry run ran: nothing is written.
    h.cluster.assert_nothing_written();
    assert_eq!(h.replicas(), 2);
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);
    assert_eq!(h.replicas(), 3);
    assert!(!h.present(cx, "write-dialog"));
    assert!(h.label(cx, "write-toast-status").starts_with("Done."));
    assert!(h.present(cx, "write-undo"));
    assert!(h.slot_free(cx));
}

#[gpui_kit::test]
fn cancelling_the_dialog_sends_no_request(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.begin(cx, fake::scale(3));
    h.click(cx, "write-cancel");
    h.drive(cx, Duration::from_secs(1));
    assert!(!h.present(cx, "write-dialog"));
    h.cluster.assert_nothing_written();
    assert!(h.slot_free(cx));
}

#[gpui_kit::test]
fn read_only_blocks_every_tier_with_its_reason_and_makes_no_request(cx: &mut TestAppContext) {
    for action in [fake::scale(3), fake::set_image("x"), fake::delete_web()] {
        let h = harness(cx, Protection::ReadOnly);
        h.begin(cx, action);
        assert_eq!(h.label(cx, "write-disabled"), "This cluster is read-only.");
        assert!(!h.present(cx, "write-confirm"));
        assert!(h.cluster.recorder.requests().is_empty());
        assert!(h.slot_free(cx));
    }
}

#[gpui_kit::test]
fn protected_asks_for_the_name_and_a_wrong_or_empty_one_sends_nothing(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Protected);
    h.begin(cx, fake::scale(3));
    assert_eq!(h.label(cx, "write-type-prompt"), "Type web to confirm");
    // Disabled while empty.
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(1));
    h.cluster.assert_nothing_written();
    h.type_name(cx, "wrong");
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(1));
    h.cluster.assert_nothing_written();
    assert!(h.present(cx, "write-dialog"));
}

#[gpui_kit::test]
fn protected_sends_once_the_exact_name_is_typed(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Protected);
    h.begin(cx, fake::scale(3));
    h.type_name(cx, "web");
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);
}

#[gpui_kit::test]
fn protected_with_several_objects_asks_for_the_entry_id_once(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Protected);
    h.begin(cx, fake::scale_and_delete());
    assert_eq!(h.label(cx, "write-type-prompt"), "Type prod-fra to confirm");
    assert!(h.present(cx, "write-impact"));
}

#[gpui_kit::test]
fn open_sends_a_reversible_action_without_a_dialog_and_says_so(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Open);
    h.begin(cx, fake::scale(3));
    assert!(!h.present(cx, "write-dialog"));
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);
    assert_eq!(
        h.label(cx, "write-toast-notice"),
        "Sent without asking because this cluster is open."
    );
    assert!(h.present(cx, "write-undo"));
}

#[gpui_kit::test]
fn open_still_asks_for_a_spec_change_and_shows_the_diff(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Open);
    h.begin(cx, fake::set_image("registry.example/web:2"));
    assert!(h.present(cx, "write-dialog") && h.present(cx, "write-diff"));
    assert!(h.present(cx, "write-diff-spec.template.spec.containers[0].image"));
    h.cluster.assert_nothing_written();
}

#[gpui_kit::test]
fn a_diff_shows_paths_and_kinds_for_every_kind_and_never_a_value(cx: &mut TestAppContext) {
    for action in [
        fake::rotate_password(),
        fake::set_image("registry.example/web:2"),
    ] {
        let h = harness(cx, Protection::Guarded);
        let path = if action.changes[0].target.kind == "Secret" {
            "write-diff-data.password"
        } else {
            "write-diff-spec.template.spec.containers[0].image"
        };
        h.begin(cx, action);
        let line = h.label(cx, path);
        assert!(line.starts_with("~ "), "{line}");
        for value in ["aHVu", "bmV3", "registry.example"] {
            assert!(!line.contains(value), "{line}");
        }
    }
}

#[gpui_kit::test]
fn a_destructive_action_counts_down_in_the_dialog_and_cancel_stops_it(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.begin(cx, fake::delete_web());
    assert!(h.present(cx, "write-impact"));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_millis(1500));
    // The confirm became Cancel, with the time left; nothing is sent yet.
    assert!(h.present(cx, "write-dialog"));
    assert!(!h.present(cx, "write-confirm"));
    assert!(
        h.label(cx, "write-cancel-countdown")
            .starts_with("Cancel (")
    );
    assert!(h.present(cx, "write-countdown"));
    h.cluster.assert_nothing_written();
    h.click(cx, "write-cancel-countdown");
    h.drive(cx, Duration::from_secs(1));
    h.cluster.assert_nothing_written();
    assert!(h.cluster.exists(fake::DEPLOYMENT));
    assert_eq!(
        h.label(cx, "write-toast-status"),
        "Cancelled. Nothing was sent."
    );
    assert!(h.slot_free(cx));
}

#[gpui_kit::test]
fn an_uncancelled_destructive_action_is_sent_after_five_seconds(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.begin(cx, fake::delete_web());
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    h.cluster.assert_nothing_written();
    h.drive(cx, Duration::from_secs(4));
    assert_eq!(h.cluster.writes().len(), 1);
    assert!(!h.cluster.exists(fake::DEPLOYMENT));
    assert!(h.label(cx, "write-toast-status").starts_with("Done."));
    assert!(!h.present(cx, "write-undo"));
}

#[gpui_kit::test]
fn a_changed_object_offers_only_to_review_the_new_version(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.cluster.foreign_write(
        fake::DEPLOYMENT,
        "kubectl",
        &serde_json::json!({"spec": {"paused": true}}),
    );
    h.begin(cx, fake::set_image("registry.example/web:2"));
    assert!(
        h.label(cx, "write-changed")
            .contains("changed after you opened it")
    );
    assert!(h.present(cx, "write-review"));
    assert!(!h.present(cx, "write-confirm"));
    h.cluster.assert_nothing_written();
    h.click(cx, "write-review");
    h.drive(cx, Duration::from_secs(1));
    // The dry run ran again against the new version; it asks again and has sent nothing.
    assert!(h.present(cx, "write-confirm") && h.present(cx, "write-diff"));
    h.cluster.assert_nothing_written();
}

#[gpui_kit::test]
fn a_deleted_object_cannot_be_reviewed_or_sent(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    let mut action = fake::set_image("x");
    action.changes[0].target.name = "missing".into();
    h.begin(cx, action);
    assert!(h.present(cx, "write-changed"));
    assert!(!h.present(cx, "write-review") && !h.present(cx, "write-confirm"));
    h.cluster.assert_nothing_written();
}

#[gpui_kit::test]
fn a_conflict_names_the_manager_and_fields_and_has_no_force(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.cluster.refuse_dry_runs(
        409,
        "Apply failed with 1 conflict: conflict with \"hpa\" using apps/v1: .spec.replicas",
    );
    h.begin(cx, fake::scale(3));
    let text = h.label(cx, "write-conflict");
    assert!(
        text.contains("hpa") && text.contains(".spec.replicas"),
        "{text}"
    );
    assert!(text.contains("The rest of the write was not sent."));
    assert!(h.label(cx, "write-conflict-fields").contains("Owner: hpa"));
    assert!(!h.present(cx, "write-confirm") && !h.present(cx, "write-review"));
    h.cluster.assert_nothing_written();
}

#[gpui_kit::test]
fn a_refused_access_review_is_shown_and_nothing_is_sent(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.cluster.deny_access();
    h.begin(cx, fake::scale(3));
    assert!(h.label(cx, "write-refused").contains("not allowed"));
    assert!(!h.present(cx, "write-confirm"));
    h.cluster.assert_nothing_written();
}

#[gpui_kit::test]
fn an_accepted_write_never_seen_stays_not_observed_and_never_looks_like_success(
    cx: &mut TestAppContext,
) {
    let h = harness(cx, Protection::Guarded);
    h.cluster.never_reaches_the_state();
    h.begin(cx, fake::scale(3));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(10));
    // Still looking: the bar shows, and the line says accepted, not done.
    assert!(h.present(cx, "write-observation-bar"));
    assert!(h.label(cx, "write-toast-status").starts_with("Accepted."));
    h.drive(cx, Duration::from_secs(25));
    let line = h.label(cx, "write-toast-status");
    assert!(
        line.contains("not seen to take effect after 30 s"),
        "{line}"
    );
    assert!(!line.contains("Done"));
    assert!(!h.present(cx, "write-observation-bar"));
    // The toast stays: a long wait later it is still there.
    h.advance(cx, Duration::from_secs(120));
    assert!(h.present(cx, "write-toast"));
}

#[gpui_kit::test]
fn a_write_with_no_answer_says_unknown_and_is_not_sent_again(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.cluster.drop_the_connection_on_writes();
    h.begin(cx, fake::scale(3));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(40));
    let line = h.label(cx, "write-toast-status");
    assert!(line.contains("not known whether it accepted"), "{line}");
    assert!(!line.contains("Done"));
    assert_eq!(h.cluster.writes().len(), 1);
    assert!(!h.present(cx, "write-undo"));
}

#[gpui_kit::test]
fn undo_shows_its_time_left_and_goes_through_the_same_path(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.begin(cx, fake::scale(3));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.label(cx, "write-undo"), "Undo (30 s)");
    h.advance(cx, Duration::from_secs(10));
    assert_eq!(h.label(cx, "write-undo"), "Undo (20 s)");
    h.click(cx, "write-undo");
    h.drive(cx, Duration::from_secs(2));
    // Undo is a write like any other: at guarded it asks first.
    assert!(h.present(cx, "write-dialog") && h.present(cx, "write-confirm"));
    assert_eq!(h.cluster.writes().len(), 1);
    assert_eq!(
        h.label(cx, "write-undo-note"),
        "Undo continues in its own notice."
    );
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(5));
    assert_eq!(h.replicas(), 2);
    assert_eq!(h.cluster.writes().len(), 2);
    assert!(h.slot_free(cx));
}

#[gpui_kit::test]
fn undo_after_someone_else_changed_it_says_who_and_never_just_vanishes(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.begin(cx, fake::scale(3));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    h.cluster.foreign_write(
        fake::DEPLOYMENT,
        "hpa",
        &serde_json::json!({"spec": {"replicas": 5}}),
    );
    h.click(cx, "write-undo");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(
        h.label(cx, "write-undo-note"),
        "Can't undo: Deployment/web was changed by hpa since."
    );
    assert_eq!(h.cluster.writes().len(), 1);
    assert_eq!(h.replicas(), 5);
}

#[gpui_kit::test]
fn undo_that_has_expired_says_so_in_the_button_place(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.begin(cx, fake::scale(3));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    h.advance(cx, Duration::from_secs(31));
    assert!(!h.present(cx, "write-undo"));
    assert_eq!(h.label(cx, "write-undo-note"), "Undo is no longer offered.");
}

#[gpui_kit::test]
fn a_submitted_write_outlives_its_flow_and_is_sent_once(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    let flow = h.begin(cx, fake::scale(3));
    h.click(cx, "write-confirm");
    // The person double-presses and the page lets go of the flow at once.
    h.click(cx, "write-confirm");
    drop(flow);
    h.drive(cx, Duration::from_secs(5));
    assert_eq!(
        h.cluster.writes().len(),
        1,
        "{:?}",
        h.cluster.recorder.lines()
    );
    assert_eq!(h.replicas(), 3);
    assert!(
        h.slot_free(cx),
        "the slot is freed exactly once, at the end"
    );
}

#[gpui_kit::test]
fn another_operation_holding_the_slot_refuses_the_confirm_and_sends_nothing(
    cx: &mut TestAppContext,
) {
    let h = harness(cx, Protection::Guarded);
    let operations = cx.update(Operations::global);
    let ticket = cx.update(|cx| {
        operations
            .update(cx, |operations, cx| operations.begin("Drain node-1", cx))
            .ok()
            .unwrap()
    });
    h.begin(cx, fake::scale(3));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(2));
    assert!(
        h.label(cx, "write-refused")
            .contains("Drain node-1 is running")
    );
    h.cluster.assert_nothing_written();
    cx.update(|cx| operations.update(cx, |operations, cx| operations.finish(ticket, cx)));
}

#[gpui_kit::test]
fn the_level_turning_read_only_after_the_dialog_opened_sends_nothing(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.begin(cx, fake::scale(3));
    cx.update(|cx| set_protection(Protection::ReadOnly, cx));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(2));
    h.cluster.assert_nothing_written();
    assert!(h.label(cx, "write-refused").contains("read-only"));
}

#[gpui_kit::test]
fn undo_on_a_protected_cluster_asks_for_the_name(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Open);
    h.begin(cx, fake::scale(3));
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);
    // The cluster becomes protected before Undo is pressed.
    cx.update(|cx| set_protection(Protection::Protected, cx));
    h.click(cx, "write-undo");
    h.drive(cx, Duration::from_secs(2));
    assert_eq!(h.label(cx, "write-type-prompt"), "Type web to confirm");
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(2));
    assert_eq!(h.cluster.writes().len(), 1, "a bare click sends nothing");
    h.type_name(cx, "web");
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.replicas(), 2);
}

#[gpui_kit::test]
fn a_level_that_got_stricter_while_the_dialog_was_open_asks_again(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.begin(cx, fake::scale(3));
    assert!(!h.present(cx, "write-type-prompt"));
    cx.update(|cx| set_protection(Protection::Protected, cx));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(2));
    // Back in the dialog, now asking for the name; nothing was sent.
    assert!(h.present(cx, "write-dialog"));
    assert_eq!(h.label(cx, "write-type-prompt"), "Type web to confirm");
    h.cluster.assert_nothing_written();
    assert!(h.slot_free(cx));
    h.type_name(cx, "web");
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);
}

#[gpui_kit::test]
fn a_worker_that_died_reads_as_unknown_and_never_as_not_sent(_cx: &mut TestAppContext) {
    use freshkube_core::write::{Headline, Status};
    let finished = super::report::stopped();
    assert_eq!(finished.headline(), Headline::Unknown);
    assert!(!matches!(finished.status, Status::NotSent(_)));
    assert!(
        finished
            .message()
            .contains("It is not known whether it accepted the write")
    );
}

#[gpui_kit::test]
fn an_undo_asked_again_under_a_stricter_level_keeps_its_guard(cx: &mut TestAppContext) {
    let h = harness(cx, Protection::Guarded);
    h.begin(cx, fake::scale(3));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);
    h.click(cx, "write-undo");
    h.drive(cx, Duration::from_secs(2));
    assert!(h.present(cx, "write-dialog"));
    // The level gets stricter, and someone else writes before Undo is sent.
    cx.update(|cx| set_protection(Protection::Protected, cx));
    h.cluster.foreign_write(
        fake::DEPLOYMENT,
        "hpa",
        &serde_json::json!({"spec": {"replicas": 5}}),
    );
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    // Asked again, the check is Undo's own: its guard no longer holds, so
    // the old value is not offered back over the other writer's.
    assert!(h.present(cx, "write-refused") || h.present(cx, "write-changed"));
    assert!(!h.present(cx, "write-confirm"));
    assert_eq!(h.cluster.writes().len(), 1);
    assert_eq!(h.replicas(), 5);
}
