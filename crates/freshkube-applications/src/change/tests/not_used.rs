//! Not used here (#823): a step the application isn't expected to have is
//! dim, counted nowhere, and a column where every Stage is the same is
//! hidden.
use super::*;
use crate::change::overview::cells::{Column, visible};
use freshkube_delivery::change::Cause;

fn mark_unused(page: &Entity<ChangePage>, window: &mut Window, cx: &mut App, keys: &[&str]) {
    let mut change = page.read(cx).change.clone();
    for hop in change
        .hops
        .iter_mut()
        .filter(|hop| keys.contains(&hop.key.as_str()))
    {
        hop.state = HealthIndicator::Unknown;
        hop.link = Some(Confidence::Unknown);
        hop.cause = Some(Cause::not_used_here(
            "an Argo Rollout, which isn't expected here",
        ));
    }
    page.update(cx, |page, cx| page.read_answer(change, window, cx));
    window.render_frame(cx);
}

#[gpui_kit::test]
fn a_column_nothing_uses_is_hidden_and_one_a_stage_uses_is_drawn(cx: &mut TestAppContext) {
    let (handle, page) = open_overview(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // No Stage runs a Rollout, so Next step has no header and no cells.
        assert!(window.try_find("change-cell-dev-next-step").is_none());
        assert!(window.try_find("change-cell-dev-synced").is_some());
        assert!(!visible(page.read(cx).overview.iter()).contains(&Column::NextStep));
    })
    .unwrap();
    let (handle, page) = open_overview(cx, 1280., 880., Shape::Rollouts, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Some Stages run one: the column is drawn, and the Stage without
        // one is dim in it.
        assert!(window.find("change-cell-dev-next-step").visible());
        let page = page.read(cx);
        let ams = page
            .overview
            .iter()
            .find(|row| row.name == "prod-ams")
            .unwrap();
        let cell = &ams.cells[3];
        assert_eq!(cell.word.as_ref(), "Not used here");
        assert!(cell.not_used && !cell.reached);
        assert_eq!(cell.tone, Tone::Outline);
        assert_eq!(cell.hop, None, "nothing to select");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_hop_that_is_not_used_here_is_dim_and_counted_nowhere(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let before = (page.read(cx).counts, page.read(cx).fixes.len());
        mark_unused(&page, window, cx, &["dev-pods"]);
        let p = page.read(cx);
        let dev = p.overview.iter().find(|row| row.name == "dev").unwrap();
        let running = &dev.cells[5];
        assert_eq!(running.word.as_ref(), "Not used here");
        assert_eq!(running.tone, Tone::Outline);
        assert!(running.not_used && !running.reached);
        // It is neither a problem nor the clock: the fix bar has no item for
        // it, and no chip counts the group worse for it.
        assert!(p.fixes.len() <= before.1, "{:?}", p.fixes);
        assert_eq!(p.counts, before.0, "the chips count the same groups");
        assert!(
            !p.headline.contains("pods") || p.headline.contains("prod"),
            "{}",
            p.headline
        );
        // Its column stays: the other Stages still use it.
        assert!(visible(p.overview.iter()).contains(&Column::Running));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_column_where_every_stage_is_not_used_goes_and_the_keys_skip_it(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let keys: Vec<String> = page
            .read(cx)
            .change
            .hops
            .iter()
            .filter(|hop| matches!(&hop.shows, Shows::Hop(d) if d.kind == "Argo CD Application"))
            .map(|hop| hop.key.clone())
            .collect();
        assert!(!keys.is_empty());
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        mark_unused(&page, window, cx, &keys);
        let p = page.read(cx);
        let shown = visible(p.overview.iter());
        assert!(!shown.contains(&Column::Synced), "{shown:?}");
        assert!(window.try_find("change-cell-dev-synced").is_none());
        // Its cells have nothing to select or ring, so the hop belongs to
        // its Stage's row.
        for row in &p.overview {
            assert_eq!(row.cells[4].hop, None, "{}", row.name);
        }
        assert!(window.find("change-cell-dev-running").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_not_used_here_hop_explains_itself_with_nothing_to_fix(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        mark_unused(&page, window, cx, &["dev-pods"]);
        page.update(cx, |page, cx| page.select("dev-pods", window, cx));
        window.render_frame(cx);
        assert_eq!(
            window.find("change-fix-cause").label(),
            Some("Not used here")
        );
        // Nothing to fix: none of the next steps' buttons is offered.
        for id in [
            "change-fix-kargo",
            "change-fix-copy",
            "change-fix-retry",
            "change-fix-settings",
            "change-fix-sources",
        ] {
            assert!(window.try_find(id).is_none(), "{id}");
        }
    })
    .unwrap();
}

fn expect(page: &Entity<ChangePage>, window: &mut Window, cx: &mut App, stages: &[&str]) {
    let applications = [freshkube_core::setup::DeliveryApplication::new(
        "checkout", stages,
    )];
    page.update(cx, |page, cx| {
        page.set_applications(&applications);
        let raw = page.change.clone();
        page.read_answer(raw, window, cx);
    });
    window.render_frame(cx);
}

fn words(page: &Entity<ChangePage>, cx: &App, stage: &str) -> Vec<String> {
    let page = page.read(cx);
    let row = page.overview.iter().find(|row| row.name == stage).unwrap();
    row.cells.iter().map(|cell| cell.word.to_string()).collect()
}

#[gpui_kit::test]
fn dont_expect_it_turns_a_stages_not_found_into_not_used_here(cx: &mut TestAppContext) {
    let (handle, page) =
        open_overview(cx, 1280., 880., Shape::ProdLonNotFound, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let before = words(&page, cx, "prod-lon");
        assert!(before.iter().any(|w| w == "Not found"), "{before:?}");
        let fixes_before = page.read(cx).fixes.len();
        let counts_before = page.read(cx).counts;
        assert!(
            page.read(cx)
                .fixes
                .iter()
                .any(|fix| fix.hop.starts_with("prod-lon-")),
            "Not found is something to fix until the setup says otherwise"
        );

        expect(&page, window, cx, &["prod-lon"]);
        let after = words(&page, cx, "prod-lon");
        assert!(!after.iter().any(|w| w == "Not found"), "{after:?}");
        assert!(after.iter().any(|w| w == "Not used here"), "{after:?}");
        let p = page.read(cx);
        assert!(p.fixes.len() < fixes_before);
        assert!(p.fixes.iter().all(|fix| !fix.hop.starts_with("prod-lon-")));
        // prod-lon's verification failed, so its group counts as failing
        // either way: Not used here changes no count it didn't earn.
        assert_eq!(p.counts, counts_before);
        assert!(p.not_expected_note.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn only_not_found_is_changed_and_an_unknown_stage_name_is_a_warning(cx: &mut TestAppContext) {
    let (handle, page) = open_overview(cx, 1280., 880., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let before: Vec<Vec<String>> = ["dev", "stage", "prod-ams", "prod-lon"]
            .iter()
            .map(|stage| words(&page, cx, stage))
            .collect();
        // The causes shape has Waiting, Can't read and Not set up on Stages:
        // listing every Stage changes none of them. A name that is no Stage
        // of the change warns beside the setup's own warning.
        expect(
            &page,
            window,
            cx,
            &["dev", "stage", "prod-ams", "prod-lon", "prod-lan"],
        );
        let after: Vec<Vec<String>> = ["dev", "stage", "prod-ams", "prod-lon"]
            .iter()
            .map(|stage| words(&page, cx, stage))
            .collect();
        assert_eq!(after, before);
        let p = page.read(cx);
        let note = p.not_expected_note.as_deref().expect("a warning");
        assert!(
            note.contains("prod-lan") && note.contains("checkout"),
            "{note}"
        );
        let (text, _) = p
            .missing
            .clone()
            .expect("the banner of what may be missing");
        assert!(text.contains("prod-lan"), "{text}");
        // And it is only a warning: the page reads and shows as before.
        assert!(!p.change.hops.is_empty());
    })
    .unwrap();
}
