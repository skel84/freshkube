//! The Stage actions (#855) in a real window against core's recording fake
//! API holding invented Kargo objects: what each level shows, that the typed
//! name unlocks Send only when it matches, that Refresh asks nothing, and
//! that a blocked or unconfirmed action sends no request.

use std::time::Duration;

use freshkube_core::write::testing::Cluster;
use freshkube_core::write::{Phase, Protection};
use freshkube_delivery::acts::{self, Pick, StageActs};
use freshkube_delivery::change::Change;
use freshkube_delivery::change::example::{example_acts, for_stage};
use gpui_kit::TestAppContext;
use serde_json::{Value, json};

use super::tests::{Harness, harness_on, harness_sized};
use crate::mutation::set_protection;

const NS: &str = "/apis/kargo.akuity.io/v1alpha1/namespaces/checkout";

fn kargo() -> Cluster {
    let object = |kind: &str, name: &str| {
        json!({"apiVersion": "kargo.akuity.io/v1alpha1", "kind": kind,
            "metadata": {"name": name, "namespace": "checkout",
                "uid": format!("uid-{}-{name}", kind.to_lowercase()), "resourceVersion": "100"},
            "status": {}})
    };
    Cluster::with(vec![
        (format!("{NS}/stages/prod-ams"), object("Stage", "prod-ams")),
        (
            format!("{NS}/promotions/prod-ams.01j9x4k2.wonky-o"),
            object("Promotion", "prod-ams.01j9x4k2.wonky-o"),
        ),
    ])
}

fn change() -> Change {
    for_stage("checkout", "prod-ams", chrono::Utc::now()).unwrap()
}

fn promote(destination: Protection) -> freshkube_core::write::ActionDescription {
    let stage = example_acts("prod-ams");
    acts::promote(
        &change(),
        &[Pick {
            acts: &stage,
            destination: Some(("prod-ams".into(), destination)),
        }],
        "01j9x4k3",
    )
}

fn running() -> acts::Promoting {
    acts::Promoting {
        promotion: acts::Handle {
            name: "prod-ams.01j9x4k2.wonky-o".into(),
            uid: "uid-promotion-prod-ams.01j9x4k2.wonky-o".into(),
            resource_version: "100".into(),
        },
        freight: "wonky-otter".into(),
        phase: "Running".into(),
    }
}

fn audited(h: &Harness) -> Vec<Phase> {
    freshkube_core::write::AuditStore::new(h._dir.path().join("audit"))
        .read()
        .unwrap()
        .records
        .iter()
        .map(|r| r.phase)
        .collect()
}

#[gpui_kit::test]
fn promote_on_a_guarded_pair_shows_the_preview_and_asks_for_no_typing(cx: &mut TestAppContext) {
    let h = harness_on(cx, Protection::Guarded, kargo);
    h.begin(cx, promote(Protection::Guarded));
    assert!(h.present(cx, "write-dialog") && h.present(cx, "write-confirm"));
    assert!(!h.present(cx, "write-type-name"), "guarded: no typing");
    // What runs now against what would run, the change, and the way back.
    assert!(
        h.label(cx, "write-preview-0")
            .contains("Runs now: Freight brave-l")
    );
    assert!(
        h.label(cx, "write-preview-0")
            .contains("Would run: Freight wonky-o")
    );
    assert!(h.present(cx, "write-destinations"));
    assert!(
        h.label(cx, "write-destinations")
            .contains("prod-ams (Guarded)")
    );
    h.cluster.assert_nothing_written();
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    let sent = h.cluster.writes();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].method, "POST");
    assert!(sent[0].path.ends_with("/promotions"));
}

#[gpui_kit::test]
fn a_protected_destination_asks_for_the_stage_name_and_send_waits_for_it(cx: &mut TestAppContext) {
    // Kargo's own cluster is only guarded; the destination decides.
    let h = harness_on(cx, Protection::Guarded, kargo);
    h.begin(cx, promote(Protection::Protected));
    assert_eq!(h.label(cx, "write-type-prompt"), "Type prod-ams to confirm");
    assert!(
        h.label(cx, "write-destinations")
            .contains("prod-ams (Protected)")
    );
    // Empty, then wrong in each way: Send does nothing.
    for wrong in [None, Some("prod"), Some("Prod-ams")] {
        if let Some(wrong) = wrong {
            h.type_name(cx, wrong);
        }
        h.click(cx, "write-confirm");
        h.drive(cx, Duration::from_secs(1));
        h.cluster.assert_nothing_written();
        assert!(h.present(cx, "write-dialog"), "{wrong:?}");
    }
}

#[gpui_kit::test]
fn the_exact_stage_name_unlocks_the_send(cx: &mut TestAppContext) {
    let h = harness_on(cx, Protection::Guarded, kargo);
    h.begin(cx, promote(Protection::Protected));
    h.type_name(cx, "prod-ams");
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);
    assert!(
        h.cluster
            .exists(&format!("{NS}/promotions/prod-ams.01j9x4k3.wonky-o"))
    );
}

#[gpui_kit::test]
fn a_protected_kargo_cluster_asks_for_the_name_though_the_destination_is_open(
    cx: &mut TestAppContext,
) {
    let h = harness_on(cx, Protection::Protected, kargo);
    h.begin(cx, promote(Protection::Open));
    assert_eq!(h.label(cx, "write-type-prompt"), "Type prod-ams to confirm");
}

#[gpui_kit::test]
fn a_read_only_destination_blocks_a_promotion_and_asks_the_cluster_nothing(
    cx: &mut TestAppContext,
) {
    let h = harness_on(cx, Protection::Open, kargo);
    h.begin(cx, promote(Protection::ReadOnly));
    assert_eq!(h.label(cx, "write-disabled"), "This cluster is read-only.");
    assert!(!h.present(cx, "write-confirm"));
    assert!(h.cluster.recorder.requests().is_empty());
}

#[gpui_kit::test]
fn several_stages_are_typed_once_and_the_dialog_names_the_protected_one(cx: &mut TestAppContext) {
    // Two Stages' previews are long; the dialog's body scrolls, so the window is tall.
    let h = harness_sized(cx, Protection::Guarded, 1800., || Cluster::with(vec![]));
    let (dev, prod): (StageActs, StageActs) = (example_acts("dev"), example_acts("prod-ams"));
    let action = acts::promote(
        &change(),
        &[
            Pick {
                acts: &dev,
                destination: Some(("dev-fra".into(), Protection::Open)),
            },
            Pick {
                acts: &prod,
                destination: Some(("prod-ams".into(), Protection::Protected)),
            },
        ],
        "01j9x4k3",
    );
    h.begin(cx, action);
    assert_eq!(h.label(cx, "write-type-prompt"), "Type prod-fra to confirm");
    assert!(
        h.label(cx, "write-type-once")
            .contains("Protected: prod-ams")
    );
    h.type_name(cx, "prod-fra");
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 2, "one Promotion for each Stage");
}

#[gpui_kit::test]
fn refresh_asks_nothing_at_any_writable_level_says_why_and_is_audited(cx: &mut TestAppContext) {
    for level in [Protection::Protected, Protection::Guarded, Protection::Open] {
        let h = harness_on(cx, level, kargo);
        h.begin(
            cx,
            acts::refresh_stage(&example_acts("prod-ams"), "2026-10-22T10:00:00Z"),
        );
        assert!(!h.present(cx, "write-dialog"), "{level:?}");
        // Past the wait for Kargo to handle it, so the slot is free again.
        h.drive(cx, Duration::from_secs(35));
        let sent = h.cluster.writes();
        assert_eq!(sent.len(), 1, "{level:?}: {sent:?}");
        assert_eq!(
            h.label(cx, "write-toast-notice"),
            "Sent without asking: it only adds an annotation.",
            "{level:?}"
        );
        let body: Value = serde_json::from_str(&sent[0].body).unwrap();
        assert_eq!(
            body["metadata"]["annotations"]["kargo.akuity.io/refresh"],
            "2026-10-22T10:00:00Z"
        );
        assert_eq!(
            audited(&h)[..2],
            [Phase::Sending, Phase::Accepted],
            "{level:?}"
        );
    }
}

#[gpui_kit::test]
fn read_only_blocks_refresh_with_no_request(cx: &mut TestAppContext) {
    let h = harness_on(cx, Protection::ReadOnly, kargo);
    h.begin(cx, acts::refresh_stage(&example_acts("prod-ams"), "t"));
    assert_eq!(h.label(cx, "write-disabled"), "This cluster is read-only.");
    assert!(h.cluster.recorder.requests().is_empty());
    assert!(audited(&h).is_empty());
}

#[gpui_kit::test]
fn a_level_changed_in_settings_reaches_a_promote_dialog_already_open(cx: &mut TestAppContext) {
    let h = harness_on(cx, Protection::Guarded, kargo);
    h.begin(cx, promote(Protection::Guarded));
    assert!(!h.present(cx, "write-type-name"));
    // The runner follows the shared cell: stricter while the dialog is open.
    cx.update(|cx| set_protection(Protection::Protected, cx));
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(2));
    assert!(h.present(cx, "write-dialog"));
    assert_eq!(h.label(cx, "write-type-prompt"), "Type prod-ams to confirm");
    h.cluster.assert_nothing_written();
    h.type_name(cx, "prod-ams");
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(3));
    assert_eq!(h.cluster.writes().len(), 1);
}

#[gpui_kit::test]
fn abort_names_what_already_ran_counts_down_and_then_marks_the_promotion(cx: &mut TestAppContext) {
    let h = harness_on(cx, Protection::Protected, kargo);
    h.begin(
        cx,
        acts::abort(&example_acts("prod-ams"), &running(), &change()),
    );
    assert_eq!(h.label(cx, "write-type-prompt"), "Type prod-ams to confirm");
    assert!(h.present(cx, "write-impact"));
    let mut text = String::new();
    for id in ["write-preview-0", "write-preview-1", "write-preview-2"] {
        if h.present(cx, id) {
            text.push_str(&h.label(cx, id));
        }
    }
    assert!(text.contains("the next sync deploys it"), "{text}");
    h.type_name(cx, "prod-ams");
    h.click(cx, "write-confirm");
    h.drive(cx, Duration::from_secs(2));
    // Counting down: nothing sent yet, and Cancel is where the confirm was.
    assert!(h.present(cx, "write-cancel-countdown"));
    h.cluster.assert_nothing_written();
    h.drive(cx, Duration::from_secs(6));
    let sent = h.cluster.writes();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(
        sent[0]
            .path
            .ends_with("/promotions/prod-ams.01j9x4k2.wonky-o")
    );
    // The value Kargo acts on: its AbortPromotionRequest's `terminate`.
    let body: Value = serde_json::from_str(&sent[0].body).unwrap();
    assert_eq!(
        body["metadata"]["annotations"]["kargo.akuity.io/abort"],
        r#"{"action":"terminate"}"#
    );
}
