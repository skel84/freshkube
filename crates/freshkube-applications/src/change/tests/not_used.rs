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
        let lon = page
            .overview
            .iter()
            .find(|row| row.name == "prod-lon")
            .unwrap();
        let cell = &lon.cells[3];
        assert_eq!(cell.word.as_ref(), "Not used here");
        assert!(cell.not_used && !cell.reached);
        assert_eq!(cell.tone, Tone::Outline);
        assert_eq!(cell.hop, None, "nothing to select");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_hop_that_is_not_used_here_is_dim_and_counted_nowhere(cx: &mut TestAppContext) {
    // In the causes shape prod-ams's pods can't be read: a fix item and a
    // clause of the headline, until they are said not to be used here.
    let (handle, page) = open_overview(cx, 1280., 880., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let p = page.read(cx);
        assert!(p.fixes.iter().any(|fix| fix.hop == "prod-ams-pods"));
        let headline = p.headline.clone();
        let counts = p.counts;
        mark_unused(&page, window, cx, &["prod-ams-pods"]);
        let p = page.read(cx);
        let ams = p
            .overview
            .iter()
            .find(|row| row.name == "prod-ams")
            .unwrap();
        let running = &ams.cells[5];
        assert_eq!(running.word.as_ref(), "Not used here");
        assert_eq!(running.tone, Tone::Outline);
        assert!(running.not_used && !running.reached);
        assert_eq!(running.hop, None, "nothing to select or ring");
        // It is neither a problem nor the clock: no fix item names it, the
        // headline has no clause about it, and the chips count no worse.
        assert!(
            p.fixes.iter().all(|fix| fix.hop != "prod-ams-pods"),
            "{:?}",
            p.fixes
        );
        // It was one of the clauses the headline counts ("and 2 more").
        assert_ne!(p.headline, headline);
        assert!(!p.headline.contains("pods"), "{}", p.headline);
        assert!(p.counts[3] <= counts[3], "{:?} {counts:?}", p.counts);
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
        // The arrow keys jump over it: from Verified the next step along the
        // row that has anything to select is Running, and back.
        let selected = |cx: &App| page.read(cx).selected().map(|s| s.to_string());
        page.update(cx, |page, cx| {
            page.select("dev-verification", window, cx);
            page.focus(window, cx);
        });
        window.render_frame(cx);
        window.press("right", cx);
        assert_eq!(selected(cx).as_deref(), Some("dev-pods"));
        window.press("left", cx);
        assert_eq!(selected(cx).as_deref(), Some("dev-verification"));
        page.update(cx, |page, cx| page.select("dev-promotion", window, cx));
        window.press("right", cx);
        assert_eq!(selected(cx).as_deref(), Some("dev-verification"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_column_nothing_could_be_read_for_stays_and_says_why(cx: &mut TestAppContext) {
    // Argo CD's Applications were refused: nothing says whether a Rollout
    // runs, so the Next step column stays, with the cause, and is not
    // hidden as unused.
    let (handle, page) = open_overview(cx, 1280., 880., Shape::ArgoRefused, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let p = page.read(cx);
        assert!(visible(p.overview.iter()).contains(&Column::NextStep));
        let with_app: Vec<_> = p
            .overview
            .iter()
            .filter(|row| row.cells[3].word.as_ref() == "Can't read")
            .map(|row| row.name.to_string())
            .collect();
        assert!(!with_app.is_empty(), "some Stage shows why");
        for stage in &with_app {
            assert!(
                window
                    .find(format!("change-cell-{stage}-next-step"))
                    .visible(),
                "{stage}"
            );
        }
        assert!(
            p.overview
                .iter()
                .all(|row| !row.cells[3].not_used || row.cells[3].word.as_ref() == "Not used here")
        );
        assert!(
            p.overview
                .iter()
                .filter(|row| row.cells[3].word.as_ref() != "Can't read")
                .all(|row| !row.cells[3].reached),
            "the rest were not reached"
        );
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
fn a_listed_stage_stops_counting_as_unknown_in_the_chips(cx: &mut TestAppContext) {
    // dev is fine in the default fixture. Make its Application and pods not
    // found, so its group can't be confirmed, and then list the Stage.
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let before = page.read(cx).counts;
        let mut change = page.read(cx).change.clone();
        for hop in change
            .hops
            .iter_mut()
            .filter(|hop| ["dev-argocd", "dev-pods"].contains(&hop.key.as_str()))
        {
            hop.state = HealthIndicator::Unknown;
            hop.link = Some(Confidence::Unknown);
            hop.cause = Some(Cause::not_found("an Application that deploys to dev"));
        }
        page.update(cx, |page, cx| page.read_answer(change, window, cx));
        let unknown = page.read(cx).counts;
        assert_eq!(unknown[3], before[3] + 1, "{unknown:?} from {before:?}");
        assert_eq!(unknown[4], before[4] - 1, "the ok count drops by one");
        let fixes = page.read(cx).fixes.len();
        assert!(fixes >= 2);

        expect(&page, window, cx, &["dev"]);
        let p = page.read(cx);
        assert_eq!(p.counts, before, "dev counts as it did before");
        assert!(p.fixes.len() <= fixes - 2, "{:?}", p.fixes);
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

#[gpui_kit::test]
fn a_not_used_hop_leaves_its_stage_group_where_it_was(cx: &mut TestAppContext) {
    // dev-pods sorts last in dev's group: a dim hop there must not stand
    // for the whole group, which is fine and stays counted as ok.
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let before = page.read(cx).counts;
        mark_unused(&page, window, cx, &["dev-pods"]);
        let p = page.read(cx);
        assert_eq!(p.counts, before, "the chips count the same groups");
        let dev = p
            .groups
            .iter()
            .find(|group| group.label.as_ref() == "Stage dev")
            .unwrap();
        assert_eq!(dev.counts_as, Tone::Good);
    })
    .unwrap();
}
