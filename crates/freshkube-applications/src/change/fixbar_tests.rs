//! The headline, the fix bar and the Why / Needs / Fix block (#823).
use super::tests::open;
use super::{ChangePage, TONES};
use freshkube_core::indicators::HealthIndicator;
use freshkube_delivery::change::example::Shape;
use freshkube_delivery::change::{Cause, Eligible, Promotion, Reach};
use freshkube_delivery::join::Confidence;
use freshkube_ui::layout_check;
use freshkube_ui::ui::Tone;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext, Window};
use std::time::Duration;

const FOUR: &str = "Runs in dev, stage and prod-lon. prod-lon failed (verification failed); \
    the build can't be confirmed (run not found, policy not found); \
    stage can't be confirmed (delivery.stage_naming not set); and 2 more.";

/// The page's change with nothing unconfirmed, waiting or failing.
fn healthy(page: &Entity<ChangePage>, window: &mut Window, cx: &mut gpui_kit::App) {
    let mut change = page.read(cx).change.clone();
    for hop in &mut change.hops {
        hop.cause = None;
        hop.state = HealthIndicator::Healthy;
    }
    for stage in &mut change.stages {
        stage.state = HealthIndicator::Healthy;
        stage.reach = Reach::Linked(Confidence::Confirmed);
        stage.eligible = Eligible::Warehouse { at: None };
        stage.promotion = Promotion::Automatic { at: None };
    }
    page.update(cx, |page, cx| page.read_answer(change, window, cx));
    window.render_frame(cx);
}

fn count(page: &ChangePage, tone: Tone) -> usize {
    let ix = TONES.iter().position(|(t, ..)| *t == tone).unwrap();
    page.counts[ix]
}

#[gpui_kit::test]
fn a_healthy_change_says_where_it_runs_and_has_no_fix_bar(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        healthy(&page, window, cx);
        assert_eq!(
            page.read(cx).headline(),
            "Runs in dev, stage, prod-ams and prod-lon."
        );
        assert_eq!(
            window.find("change-headline").label(),
            Some("Runs in dev, stage, prod-ams and prod-lon.")
        );
        assert!(window.try_find("change-fixbar").is_none());
        let p = page.read(cx);
        assert_eq!((count(p, Tone::Good), count(p, Tone::Unknown)), (5, 0));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_change_with_all_four_causes_has_the_headline_and_a_chip_for_each_that_can_not_be_confirmed(
    cx: &mut TestAppContext,
) {
    // Wide enough for every chip.
    let (handle, page) = open(cx, 1700., 880., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(page.read(cx).headline(), FOUR);
        assert_eq!(window.find("change-fixbar").label(), None);
        // The count, then a chip for each cause that is a problem.
        for (key, words) in [
            ("prod-ams-pods", "Can't read: prod-ams pods"),
            ("stage-argocd", "Not set up: stage checkout-stage"),
            ("policy", "Not found: Policy acme-prod"),
            ("pr-run", "Not found: Run checkout-pr-418-m2q8"),
        ] {
            let chip = window.find(format!("change-fixbar-{key}"));
            assert!(chip.visible(), "{key}");
            assert_eq!(chip.label(), Some(words), "{key}");
        }
        // Waiting is no problem: prod-ams waits, and has no chip.
        assert!(
            window
                .try_find("change-fixbar-prod-ams-promotion")
                .is_none()
        );
        assert!(page.read(cx).fixes.len() == 4);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_chip_selects_its_hop_and_the_inspector_leads_with_its_cause(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1700., 880., Shape::Causes, Duration::ZERO, 13.);
    for (key, word) in [
        ("prod-ams-pods", "Can't read"),
        ("stage-argocd", "Not set up"),
        ("policy", "Not found"),
    ] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(format!("change-fixbar-{key}"), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(page.read(cx).selected().map(|s| s.as_ref()), Some(key));
            assert_eq!(window.find("change-fix-cause").label(), Some(word), "{key}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_failing_change_leads_with_what_failed(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        healthy(&page, window, cx);
        let mut change = page.read(cx).change.clone();
        let lon = change
            .stages
            .iter_mut()
            .find(|s| s.name == "prod-lon")
            .unwrap();
        lon.state = HealthIndicator::Error;
        lon.words = "Verification failed".into();
        page.update(cx, |page, cx| page.read_answer(change, window, cx));
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).headline(),
            "Runs in dev, stage, prod-ams and prod-lon. prod-lon failed (verification failed)."
        );
        assert!(window.try_find("change-fixbar").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn chips_that_do_not_fit_fold_into_a_menu_that_selects_the_rest(cx: &mut TestAppContext) {
    // A larger text size leaves less room in dp, as a narrower window would.
    let (handle, page) = open(cx, 760., 560., Shape::Causes, Duration::ZERO, 20.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("change-fixbar-pr-run").visible());
        // Not every chip fits: the rest is behind +N.
        let more = window.find("change-fixbar-more");
        assert!(more.visible());
        assert!(window.try_find("change-fixbar-prod-ams-pods").is_none());
        window.click("change-fixbar-more", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menu = window.within("popup-menu");
        let items: Vec<String> = (0usize..)
            .map_while(|ix| menu.try_find(ix))
            .filter_map(|item| item.label().map(str::to_owned))
            .collect();
        assert!(!items.is_empty());
        assert!(
            items.iter().any(|item| item.starts_with("Not found")),
            "{items:?}"
        );
        let last = items.len() - 1;
        window.within("popup-menu").click(last, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let selected = page.read(cx).selected().cloned().expect("a selection");
        assert!(
            page.read(cx)
                .fixes
                .iter()
                .any(|f| f.hop == selected.as_ref())
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_headline_stays_within_two_lines_at_the_narrowest_window_and_text_20(
    cx: &mut TestAppContext,
) {
    let (handle, _page) = open(cx, 760., 560., Shape::Causes, Duration::ZERO, 20.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let headline = window.find("change-headline").bounds();
        // 12.5 dp text at 20 on 1.4 line height, two lines at most.
        let line = 12.5 * (20. / 13.) * 1.6;
        assert!(
            f32::from(headline.size.height) <= 2. * line + 2.,
            "{:?}",
            headline.size.height
        );
        assert!(window.find("change-fixbar").visible());
        // Neither runs past the page's column.
        let column = window.find("change-page").bounds().right();
        for id in ["change-headline", "change-fixbar"] {
            let right = window.find(id).bounds().right();
            assert!(right <= column, "{id} ends at {right:?}, past {column:?}");
        }
        assert!(column <= window.viewport_size().width);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_waiting_chip_shows_the_waiting_stage_and_nothing_of_the_start(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        healthy(&page, window, cx);
        let mut change = page.read(cx).change.clone();
        // prod-ams waits for its promotion and nothing else is wrong.
        for hop in change
            .hops
            .iter_mut()
            .filter(|hop| hop.key.starts_with("prod-ams"))
        {
            if hop.key.ends_with("promotion") || hop.key.ends_with("verification") {
                hop.state = HealthIndicator::Pending;
                hop.cause = Some(Cause::waiting("a Promotion of this Freight"));
            }
        }
        change.stages[2].promotion = Promotion::Waiting;
        page.update(cx, |page, cx| page.read_answer(change, window, cx));
        window.render_frame(cx);
        assert_eq!(count(page.read(cx), Tone::Waiting), 1);
        window.click("change-tally-waiting", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let lines = page.read(cx).lines_text();
        assert!(lines.contains(&"# Stage prod-ams".to_owned()), "{lines:?}");
        assert!(
            lines.iter().any(|line| line == "prod-ams-promotion"),
            "{lines:?}"
        );
        // Nothing from the start group, nor from the other Stages.
        assert!(
            !lines.iter().any(|l| l == "# Build" || l == "# Freight"),
            "{lines:?}"
        );
        assert!(
            !lines
                .iter()
                .any(|l| l.starts_with("# Stage") && l != "# Stage prod-ams")
        );
        assert!(
            lines
                .iter()
                .all(|l| l.starts_with('#') || l.starts_with("prod-ams"))
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn copy_access_request_copies_exactly_a_read_only_role(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.select("prod-ams-pods", window, cx));
        window.render_frame(cx);
        window.click("change-fix-copy", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(
        copied.as_deref(),
        Some(
            "apiVersion: rbac.authorization.k8s.io/v1\nkind: Role\nmetadata:\n  \
             name: freshkube-read-pods\n  namespace: checkout\nrules:\n  \
             - apiGroups: [\"\"]\n    resources: [\"pods\"]\n    \
             verbs: [\"get\", \"list\", \"watch\"]\n"
        )
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("change-fix-copy").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_inspector_keeps_its_frame_with_the_block_beside_and_stacked(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (handle, page) = open(cx, width, height, Shape::Causes, Duration::ZERO, 13.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            page.update(cx, |page, cx| page.select("prod-ams-pods", window, cx));
            window.render_frame(cx);
            layout_check::assert_inspector(
                window,
                cx,
                "change-split",
                "change-table",
                "change-detail",
                "change-detail-title",
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn copy_access_request_says_copied_after_it_copies(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.select("prod-ams-pods", window, cx));
        window.render_frame(cx);
        assert_eq!(
            window.find("change-fix-copy").label(),
            Some("Copy access request")
        );
        window.click("change-fix-copy", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("change-fix-copy").label(), Some("Copied"));
        // Another hop's button is not Copied.
        page.update(cx, |page, cx| page.select("policy", window, cx));
        window.render_frame(cx);
        assert!(window.try_find("change-fix-copy").is_none());
        page.update(cx, |page, cx| page.select("prod-ams-pods", window, cx));
        window.render_frame(cx);
        assert_eq!(window.find("change-fix-copy").label(), Some("Copied"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_chip_clears_a_filter_that_hides_its_hop(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1700., 880., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("change-tally-ok", cx);
        window.render_frame(cx);
        let lines = page.read(cx).lines_text();
        assert!(!lines.contains(&"prod-ams-pods".to_owned()), "{lines:?}");
        window.click("change-fixbar-prod-ams-pods", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let p = page.read(cx);
        assert_eq!(p.selected().map(|s| s.as_ref()), Some("prod-ams-pods"));
        assert!(p.filter.is_none(), "the filter that hid the row is cleared");
        assert!(p.lines_text().contains(&"prod-ams-pods".to_owned()));
        // A filter that shows the row stays.
        window.click("change-tally-unknown", cx);
        window.render_frame(cx);
        window.click("change-fixbar-prod-ams-pods", cx);
        window.render_frame(cx);
        assert!(page.read(cx).filter.is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_menu_clears_a_filter_that_hides_its_hop_too(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 760., 560., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("change-tally-ok", cx);
        window.render_frame(cx);
        window.click("change-fixbar-more", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menu = window.within("popup-menu");
        let last = (0usize..)
            .take_while(|&ix| menu.try_find(ix).is_some())
            .count()
            - 1;
        window.within("popup-menu").click(last, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let p = page.read(cx);
        let key = p.selected().cloned().expect("a selection");
        assert!(p.filter.is_none());
        assert!(p.lines_text().contains(&key.to_string()));
    })
    .unwrap();
}

#[gpui_kit::test]
fn retry_shows_reading_and_holds_while_the_change_is_read(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let mut change = page.read(cx).change.clone();
        let policy = change.hops.iter_mut().find(|h| h.key == "policy").unwrap();
        policy.cause = Some(Cause::cant_read("the policy results", None, false));
        page.update(cx, |page, cx| page.read_answer(change, window, cx));
        page.update(cx, |page, cx| page.select("policy", window, cx));
        window.render_frame(cx);
        assert_eq!(window.find("change-fix-retry").label(), Some("Retry"));
        window.click("change-fix-retry", cx);
        window.render_frame(cx);
        // The read is in flight: the button says so and holds.
        assert_eq!(window.find("change-fix-retry").label(), Some("Reading…"));
        assert!(page.read(cx).loader.is_loading());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!page.read(cx).loader.is_loading());
        // The answer is the example's again, so the hop's cause is back to
        // what the example says; whatever it is, the button no longer
        // says Reading….
        if let Some(button) = window.try_find("change-fix-retry") {
            assert_ne!(button.label(), Some("Reading…"));
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn check_sources_and_open_settings_open_their_section(cx: &mut TestAppContext) {
    use freshkube_ui::platform::OpenSourcesSettings;
    use std::cell::Cell;
    use std::rc::Rc;
    let (handle, page) = open(cx, 1280., 880., Shape::Causes, Duration::ZERO, 13.);
    let opened = Rc::new(Cell::new(0));
    cx.update(|cx| {
        let opened = opened.clone();
        cx.on_action(move |_: &OpenSourcesSettings, _| opened.set(opened.get() + 1));
    });
    for key in ["policy", "stage-argocd"] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            page.update(cx, |page, cx| page.select(key, window, cx));
            window.render_frame(cx);
            let id = if key == "policy" {
                "change-fix-sources"
            } else {
                "change-fix-settings"
            };
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    assert_eq!(opened.get(), 2);
}

#[gpui_kit::test]
fn open_in_kargo_opens_the_address_and_is_greyed_out_without_one(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.select("prod-ams-promotion", window, cx));
        window.render_frame(cx);
        assert_eq!(window.find("change-fix-cause").label(), Some("Waiting"));
        window.click("change-fix-kargo", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let address = cx.opened_url().expect("Kargo's page opens");
    assert!(
        address.starts_with("https://kargo.example.test"),
        "{address}"
    );
    // Without an address nothing more opens.
    let (handle, page) = open(cx, 1280., 880., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let mut change = page.read(cx).change.clone();
        change.page = Err("the cluster records no address for Kargo".into());
        page.update(cx, |page, cx| page.read_answer(change, window, cx));
        page.update(cx, |page, cx| page.select("prod-ams-promotion", window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    let before = cx.opened_url();
    cx.update_window(handle, |_, window, cx| window.click("change-fix-kargo", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.opened_url(), before);
}

#[gpui_kit::test]
fn another_hop_opens_its_inspector_at_the_top(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 760., 560., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.select("policy", window, cx));
        window.render_frame(cx);
        page.update(cx, |page, _| {
            page.detail_scroll
                .set_offset(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-40.)));
        });
        page.update(cx, |page, cx| page.select("prod-ams-pods", window, cx));
        assert_eq!(page.read(cx).detail_scroll.offset().y, gpui_kit::px(0.));
    })
    .unwrap();
}
