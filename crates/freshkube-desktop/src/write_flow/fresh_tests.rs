//! Each writer's dialog, with `freshkube.yaml` edited while it is open (#1055):
//! pressing Confirm reads the file again, and a stricter level, or a file that
//! can't say, sends nothing. The writers differ in what they describe; they
//! share the one send step, so each is driven through it here.

use std::time::Duration;

use freshkube_core::write::testing::{self as fake, Cluster};
use freshkube_core::write::{Phase, Protection};
use gpui_kit::TestAppContext;

use super::tests::{Harness, harness_rechecked};

fn audited(h: &Harness) -> Vec<Phase> {
    freshkube_core::write::AuditStore::new(h._dir.path().join("audit"))
        .read()
        .unwrap()
        .records
        .iter()
        .map(|r| r.phase)
        .collect()
}

/// Confirms after the edit and sees that nothing was written or audited as
/// sent, and the operation slot is free again.
fn nothing_sent(h: &Harness, cx: &mut TestAppContext) {
    h.cluster.assert_nothing_written();
    assert!(!audited(h).contains(&Phase::Sending), "{:?}", audited(h));
    assert!(h.slot_free(cx));
}

#[gpui_kit::test]
fn safety_g28_an_everyday_action_sends_nothing_when_the_file_got_stricter(cx: &mut TestAppContext) {
    for to in [
        Some(Protection::ReadOnly),
        Some(Protection::Protected),
        None,
    ] {
        let h = harness_rechecked(cx, Protection::Guarded, Cluster::new);
        h.begin(cx, fake::scale(3));
        h.edit_setup(to);
        h.click(cx, "write-confirm");
        h.drive(cx, Duration::from_secs(3));
        assert_eq!(h.replicas(), 2, "{to:?}");
        nothing_sent(&h, cx);
        if to == Some(Protection::Protected) {
            // Review again: the dialog is asked under the level the file holds,
            // and so is the window's level, for the header and the next write.
            assert!(h.present(cx, "write-type-name"));
            assert_eq!(cx.read(crate::mutation::level), Protection::Protected);
        }
    }
}

#[gpui_kit::test]
fn safety_g28_an_edit_of_the_yaml_sends_nothing_when_the_file_got_stricter(
    cx: &mut TestAppContext,
) {
    let h = harness_rechecked(cx, Protection::Guarded, Cluster::new);
    h.begin(cx, fake::set_image("x"));
    h.edit_setup(Some(Protection::ReadOnly));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    nothing_sent(&h, cx);
}

/// A delete is read again once more after its countdown: a file edited while
/// the five seconds ran stops it.
#[gpui_kit::test]
fn safety_g28_a_delete_sends_nothing_when_the_file_got_stricter_even_during_its_countdown(
    cx: &mut TestAppContext,
) {
    // Edited before Confirm.
    let h = harness_rechecked(cx, Protection::Guarded, Cluster::new);
    h.begin(cx, fake::delete_web());
    h.edit_setup(Some(Protection::ReadOnly));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(8));
    assert!(h.cluster.exists(fake::DEPLOYMENT));
    nothing_sent(&h, cx);
    // Edited while the countdown ran.
    let h = harness_rechecked(cx, Protection::Guarded, Cluster::new);
    h.begin(cx, fake::delete_web());
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_millis(1500));
    assert!(h.present(cx, "write-countdown"));
    h.edit_setup(Some(Protection::ReadOnly));
    h.drive(cx, Duration::from_secs(8));
    assert!(h.cluster.exists(fake::DEPLOYMENT));
    nothing_sent(&h, cx);
}

#[gpui_kit::test]
fn safety_g28_an_undo_sends_nothing_when_the_file_went_read_only(cx: &mut TestAppContext) {
    let h = harness_rechecked(cx, Protection::Guarded, Cluster::new);
    h.begin(cx, fake::scale(3));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);
    h.click(cx, "write-undo");
    h.drive(cx, Duration::from_secs(2));
    assert!(h.present(cx, "write-confirm"));
    h.edit_setup(Some(Protection::ReadOnly));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1, "the undo wrote nothing");
    assert_eq!(h.replicas(), 3);
}

/// A file that is unchanged, or looser, still sends: it only tightens. The
/// same setup then refuses a file that can't be read, which proves the file
/// was read at all.
#[gpui_kit::test]
fn a_looser_file_changes_nothing_for_a_write_until_reviewed_again(cx: &mut TestAppContext) {
    let h = harness_rechecked(cx, Protection::Guarded, Cluster::new);
    h.begin(cx, fake::scale(3));
    h.edit_setup(Some(Protection::Open));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);
    // The guard is live: the same harness, a file that can't be read, and the
    // next write sends nothing.
    h.begin(cx, fake::scale(4));
    h.edit_setup(None);
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);
}
