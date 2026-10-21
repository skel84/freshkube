//! The words and the glyph of a cause in today's List and Chain (#823).
use super::tests::open;
use super::{ChangePage, TONES};
use freshkube_delivery::change::example::Shape;
use freshkube_ui::ui::Tone;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use std::time::Duration;

fn count(page: &ChangePage, tone: Tone) -> usize {
    let ix = TONES.iter().position(|(t, ..)| *t == tone).unwrap();
    page.counts[ix]
}

fn row_tone(page: &ChangePage, key: &str) -> Tone {
    page.rows.iter().find(|row| row.key == key).unwrap().tone
}

fn show(page: &Entity<ChangePage>, key: &str, cx: &mut gpui_kit::App) -> (String, String) {
    page.update(cx, |page, cx| {
        page.selected = Some(key.to_owned().into());
        cx.notify();
    });
    let detail = page.read(cx).derive_detail().expect("a detail");
    let (_, lead, body) = detail.banner.expect("a banner");
    (lead.to_string(), body.to_string())
}

#[gpui_kit::test]
fn waiting_has_its_own_glyph_and_chip_and_counts_with_no_problem(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let p = page.read(cx);
        // prod-ams's promotion and verification have not been reached.
        assert_eq!(row_tone(p, "prod-ams-promotion"), Tone::Waiting);
        assert_eq!(row_tone(p, "prod-ams-verification"), Tone::Waiting);
        assert_eq!(count(p, Tone::Waiting), 2);
        // The unreadable pods and the pruned run are the unknowns; waiting
        // adds to neither them nor the warnings nor the failures.
        assert_eq!(count(p, Tone::Unknown), 2);
        assert_eq!(count(p, Tone::Warn), 1);
        assert_eq!(count(p, Tone::Crit), 1);
        assert!(window.find("change-tally-waiting").visible());
        assert!(window.find("change-tally-unknown").visible());
        // The group of a waiting Stage is no worse than its other rows:
        // the clock ranks with what is fine.
        assert!(super::rank(Tone::Waiting) > super::rank(Tone::Good));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_link_column_says_the_cause_in_place_of_unknown(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let p = page.read(cx);
        assert_eq!(
            count(p, Tone::Unknown),
            4,
            "the pruned run is Can't read too"
        );
        for (key, word) in [
            ("prod-ams-pods", "Can't read"),
            ("policy", "Not found"),
            ("stage-argocd", "Not set up"),
        ] {
            let row = window.find(format!("change-hop-{key}"));
            let label = row.label().unwrap();
            assert!(label.contains(&format!("link {word}")), "{key}: {label}");
            let cell = window.find(format!("change-hop-{key}-link"));
            assert_eq!(cell.label(), Some(word), "{key}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_inspector_leads_with_the_cause_what_confirms_it_and_the_step(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let (lead, body) = show(&page, "prod-ams-pods", cx);
        assert_eq!(lead, "Can't read.");
        assert!(body.contains("Needs the pods of checkout-api"), "{body}");
        assert!(
            body.contains("Ask for read access to pods in checkout."),
            "{body}"
        );
        let (lead, body) = show(&page, "policy", cx);
        assert_eq!(lead, "Not found.");
        assert!(body.contains("Check sources in Settings"), "{body}");
        let (lead, body) = show(&page, "stage-argocd", cx);
        assert_eq!(lead, "Not set up.");
        assert!(body.contains("delivery.stage_naming"), "{body}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_verification_nothing_recorded_after_its_promotion_ended_is_not_found_not_waiting(
    cx: &mut TestAppContext,
) {
    use freshkube_delivery::change::Cause;
    use freshkube_delivery::join::Confidence;
    let (handle, page) = open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // dev holds the Freight, its Promotion ended, and Kargo has no
        // verification of it: Pending, with a Not found cause.
        let mut change = page.read(cx).change.clone();
        let gate = change
            .hops
            .iter_mut()
            .find(|hop| hop.key == "dev-verification")
            .unwrap();
        gate.state = freshkube_core::indicators::HealthIndicator::Pending;
        gate.detail = "Not verified yet".into();
        gate.cause = Some(Cause::not_found(
            "an AnalysisRun in the Stage's verification history",
        ));
        let _ = Confidence::Unknown;
        let (waiting, unknown) = {
            let p = page.read(cx);
            (count(p, Tone::Waiting), count(p, Tone::Unknown))
        };
        page.update(cx, |page, cx| page.read_answer(change, window, cx));
        window.render_frame(cx);
        let p = page.read(cx);
        // The dashed ring, counted unknown; the clock stays on the rows that wait.
        assert_eq!(row_tone(p, "dev-verification"), Tone::Unknown);
        assert_eq!(count(p, Tone::Unknown), unknown + 1);
        assert_eq!(count(p, Tone::Waiting), waiting);
        let (lead, body) = show(&page, "dev-verification", cx);
        assert_eq!(lead, "Not found.");
        assert!(body.contains("Check sources in Settings"), "{body}");
        assert!(!body.contains("settings-"), "no slug on screen: {body}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_hop_in_progress_is_not_waiting(cx: &mut TestAppContext) {
    use freshkube_core::indicators::HealthIndicator;
    let (handle, page) = open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // A pod set that isn't all ready, a build that runs: Pending with
        // no cause draws the dashed ring, as before.
        let mut change = page.read(cx).change.clone();
        for key in ["dev-pods", "push-run"] {
            let hop = change.hops.iter_mut().find(|hop| hop.key == key).unwrap();
            hop.state = HealthIndicator::Pending;
            hop.cause = None;
        }
        page.update(cx, |page, cx| page.read_answer(change, window, cx));
        let p = page.read(cx);
        assert_eq!(row_tone(p, "dev-pods"), Tone::Unknown);
        assert_eq!(row_tone(p, "push-run"), Tone::Unknown);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_waiting_stage_does_not_fold_and_its_group_is_not_ok(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let p = page.read(cx);
        let ams = p
            .groups
            .iter()
            .find(|group| group.label.contains("prod-ams"))
            .expect("prod-ams's group");
        // Whatever order its rows come in, a Stage with a waiting gate is
        // never as fine as ok, so it stays open.
        assert_ne!(ams.tone, Tone::Good);
        assert!(super::rank(ams.tone) > super::rank(Tone::Good));
        assert!(
            !p.folded
                .iter()
                .any(|&ix| p.groups[ix].label.contains("prod-ams"))
        );
    })
    .unwrap();
}
