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
        assert_eq!(super::rank(Tone::Waiting), super::rank(Tone::Good));
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
        assert!(body.contains("Check the sources"), "{body}");
        let (lead, body) = show(&page, "stage-argocd", cx);
        assert_eq!(lead, "Not set up.");
        assert!(body.contains("delivery.stage_naming"), "{body}");
    })
    .unwrap();
}
