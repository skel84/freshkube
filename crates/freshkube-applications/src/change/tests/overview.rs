//! The Overview: its strip, its matrix, and how they lay out (#823).
use super::*;
use crate::change::overview::cells::Column;
use gpui_kit::Bounds;
use gpui_kit::Pixels;

/// A short window scrolls the page as a whole: the matrix, under the strip,
/// is brought into view.
fn scroll_to_matrix(page: &Entity<ChangePage>, window: &mut Window, cx: &mut App) {
    window.render_frame(cx);
    let scroll = page.read(cx).page_scroll.clone();
    scroll.set_offset(point(px(0.), -scroll.max_offset().y));
    settle(window, cx);
}

fn words(page: &Entity<ChangePage>, cx: &App, stage: &str) -> Vec<String> {
    let page = page.read(cx);
    let row = page.overview.iter().find(|row| row.name == stage).unwrap();
    row.cells.iter().map(|cell| cell.word.to_string()).collect()
}

fn bounds(window: &Window, id: String) -> Bounds<Pixels> {
    window.find(SharedString::from(id)).bounds()
}

#[gpui_kit::test]
fn the_overview_draws_the_strip_and_a_row_of_cells_per_stage(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for card in ["pr", "build", "freight"] {
            assert!(
                window.find(format!("change-strip-{card}")).visible(),
                "{card}"
            );
        }
        // Each join: its key in a pill, and its word under it.
        for into in ["build", "freight"] {
            assert!(window.find(format!("change-join-{into}-key")).visible());
            assert!(
                window
                    .try_find(format!("change-join-{into}-word"))
                    .is_some()
            );
        }
        let stages: Vec<String> = page
            .read(cx)
            .change
            .stages
            .iter()
            .map(|stage| stage.name.clone())
            .collect();
        assert_eq!(stages, ["dev", "stage", "prod-ams", "prod-lon"]);
        for stage in &stages {
            assert!(window.find(format!("change-matrix-row-{stage}")).visible());
            for column in Column::ALL {
                assert!(
                    window
                        .find(format!("change-cell-{stage}-{}", column.id()))
                        .visible(),
                    "{stage} {column:?}"
                );
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn every_cell_says_one_word_and_its_cause(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let healthy = [
            "Eligible",
            "Promoted",
            "Verified",
            "Synced",
            "Running",
            "Not looked",
        ];
        assert_eq!(words(&page, cx, "dev"), healthy);
        // Waiting is the clock, not a problem; the pods nobody may list say
        // so; what the Stage still runs says nothing of this change.
        assert_eq!(
            words(&page, cx, "prod-ams"),
            [
                "Eligible",
                "Waiting",
                "Waiting",
                "Not reached",
                "Can't read",
                "Not reached"
            ]
        );
        assert_eq!(
            words(&page, cx, "prod-lon"),
            [
                "Approved",
                "Promoted",
                "Failed",
                "Synced",
                "Running",
                "Not looked"
            ]
        );
        for row in &page.read(cx).overview {
            for cell in &row.cells {
                assert!(
                    cell.word.chars().count() <= 14,
                    "{} is wider than a column",
                    cell.word
                );
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_causes_shape_shows_all_four_causes_and_the_clock(cx: &mut TestAppContext) {
    let (handle, page) = open_overview(cx, 1280., 880., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let all: Vec<String> = page
            .read(cx)
            .overview
            .iter()
            .flat_map(|row| row.cells.iter().map(|cell| cell.word.to_string()))
            .collect();
        // The matrix carries three; the fourth, the policy result that is
        // not there, is in the start, and the fix bar counts all four.
        for cause in ["Waiting", "Can't read", "Not set up"] {
            assert!(all.iter().any(|word| word == cause), "{cause} in {all:?}");
        }
        let fixes: Vec<_> = page.read(cx).fixes.iter().map(|fix| fix.words()).collect();
        assert!(
            fixes.iter().any(|fix| fix.starts_with("Not found")),
            "{fixes:?}"
        );
        // None is drawn as a failure.
        for row in &page.read(cx).overview {
            for cell in &row.cells {
                if ["Waiting", "Can't read", "Not found", "Not set up"]
                    .contains(&cell.word.as_ref())
                {
                    assert!(
                        !matches!(cell.tone, Tone::Crit | Tone::Died),
                        "{}",
                        cell.word
                    );
                }
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_stage_that_waits_is_a_dotted_track_and_one_that_never_got_there_a_hairline(
    cx: &mut TestAppContext,
) {
    use freshkube_ui::table::Track;
    let (_handle, page) = overview_fixture(cx);
    let page = page.read_with(cx, |page, _| page.overview.clone());
    let ams = page.iter().find(|row| row.name == "prod-ams").unwrap();
    assert_eq!(ams.cells[1].track(), Track::Dotted, "waiting promotion");
    assert_eq!(ams.cells[3].track(), Track::Hairline, "not reached");
    let dev = &page[0];
    assert!(
        dev.cells[..5]
            .iter()
            .all(|cell| cell.track() == Track::Solid)
    );
    // The rail into a waiting Stage is dotted; into the others it is solid.
    assert_eq!(ams.rail[0], Track::Dotted);
    assert_eq!(dev.rail[0], Track::Solid);
    assert_eq!(dev.rail[1], Track::Solid, "dev's rail runs on into stage");
}

#[gpui_kit::test]
fn the_stage_cell_says_since_when_or_what_it_still_runs(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let lines: Vec<String> = page
            .read(cx)
            .overview
            .iter()
            .map(|row| row.line2.to_string())
            .collect();
        assert!(lines[0].starts_with("since "), "{lines:?}");
        assert!(lines[1].starts_with("since "), "{lines:?}");
        assert_eq!(lines[2], "still runs brave-lynx");
        for stage in ["dev", "prod-ams"] {
            let line = bounds(window, format!("change-stage-{stage}-since"));
            assert_eq!(
                line.size.height,
                freshkube_ui::ui::dp_px(freshkube_ui::table::LINE, window)
            );
        }
    })
    .unwrap();
}

fn assert_matrix_rows(cx: &mut TestAppContext, width: f32, height: f32, text: f32) {
    let (handle, page) = open_overview(cx, width, height, Shape::Whole, Duration::ZERO, text);
    cx.update_window(handle, |_, window, cx| {
        scroll_to_matrix(&page, window, cx);
        // The rows are 40 dp and the header stays 26 dp.
        let rows = layout_check::assert_table_rows(
            window,
            cx,
            &layout_check::Table {
                table: Some("change-matrix-table-scroll"),
                list: "change-matrix-list",
            },
            freshkube_ui::table::STEP_ROW,
        );
        assert!(rows.header.is_some());
        // The track runs on across a row: each cell starts where the one
        // before ended, its track fills it, and every track is one line.
        let mut y = None;
        let mut edge = None;
        for column in Column::ALL {
            let cell = bounds(window, format!("change-cell-dev-{}", column.id()));
            let track_in = bounds(window, format!("change-cell-dev-{}-track-in", column.id()));
            let track_out = bounds(window, format!("change-cell-dev-{}-track-out", column.id()));
            let glyph = bounds(window, format!("change-cell-dev-{}-glyph", column.id()));
            if let Some(edge) = edge {
                assert_eq!(
                    cell.left(),
                    edge,
                    "{column:?} starts where the one before ended"
                );
            }
            assert_eq!(track_in.left(), cell.left(), "{column:?} track in");
            assert_eq!(
                track_in.right(),
                glyph.left(),
                "{column:?} glyph on the track"
            );
            assert_eq!(
                glyph.right(),
                track_out.left(),
                "{column:?} glyph on the track"
            );
            assert_eq!(track_out.right(), cell.right(), "{column:?} track out");
            let centre = track_in.center().y;
            assert!((centre - *y.get_or_insert(centre)).abs() < px(0.5));
            edge = Some(cell.right());
            // Both lines are inset like the header's label.
            let word = bounds(window, format!("change-cell-dev-{}-word", column.id()));
            assert!(word.left() >= cell.left());
            assert!(word.top() >= glyph.bottom() - px(1.));
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_matrix_rows_are_40_dp_and_the_track_is_continuous(cx: &mut TestAppContext) {
    assert_matrix_rows(cx, 1280., 880., 13.);
}

#[gpui_kit::test]
fn the_matrix_rows_are_40_dp_and_the_track_continuous_at_760_by_560(cx: &mut TestAppContext) {
    assert_matrix_rows(cx, 760., 560., 13.);
}

#[gpui_kit::test]
fn the_matrix_rows_are_40_dp_and_the_track_continuous_at_text_20(cx: &mut TestAppContext) {
    assert_matrix_rows(cx, 760., 560., 20.);
}

#[gpui_kit::test]
fn the_overview_keeps_the_frame_and_the_inspector_beside_and_stacked(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.select("prod-ams-pods", window, cx));
        settle(window, cx);
        layout_check::settle_header(window, cx);
        layout_check::assert_edge_frame_from(
            window,
            cx,
            &layout_check::PageFrame {
                page: "change-page",
                title: "change-title",
                title_text: "wonky-otter",
                content: "change-split",
            },
            "change-back",
        );
        // Beside, at 1280: the matrix and the strip on the left.
        layout_check::assert_inspector(
            window,
            cx,
            "change-split",
            "change-overview",
            "change-detail",
            "change-detail-title",
        );
    })
    .unwrap();
    let (handle, page) = open_overview(cx, 760., 560., Shape::Whole, Duration::ZERO, 20.);
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| page.select("prod-ams-pods", window, cx));
        settle(window, cx);
        layout_check::settle_header(window, cx);
        // Stacked, at 760 and text 20: the Inspector is under the matrix.
        layout_check::assert_inspector(
            window,
            cx,
            "change-split",
            "change-overview",
            "change-detail",
            "change-detail-title",
        );
        assert!(
            window.find("change-detail").bounds().top()
                >= window.find("change-overview").bounds().bottom() - px(1.)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_selected_cell_beyond_the_edge_scrolls_into_view_beside_the_pinned_stage(
    cx: &mut TestAppContext,
) {
    let (handle, page) = open_overview(cx, 760., 560., Shape::Whole, Duration::ZERO, 20.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| {
            page.select("prod-lon-deployment", window, cx)
        });
        scroll_to_matrix(&page, window, cx);
        let view = bounds(window, "change-matrix-table-scroll".into());
        let cell = bounds(window, "change-cell-prod-lon-after-deploy".into());
        // The row's own 1 px borders either side are not part of a cell.
        assert!(
            cell.right() <= view.right() + px(3.),
            "{cell:?} in {view:?}"
        );
        assert!(cell.left() >= view.left() - px(1.), "{cell:?} in {view:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_strip_stacks_into_one_column_under_640_dp(cx: &mut TestAppContext) {
    let (handle, _page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let pr = bounds(window, "change-strip-pr".into());
        let build = bounds(window, "change-strip-build".into());
        assert!(
            (pr.center().y - build.center().y).abs() < px(1.),
            "a wide page keeps one row"
        );
    })
    .unwrap();
    let (handle, _page) = open_overview(cx, 600., 700., Shape::Whole, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let pr = bounds(window, "change-strip-pr".into());
        let build = bounds(window, "change-strip-build".into());
        let freight = bounds(window, "change-strip-freight".into());
        assert!(pr.bottom() <= build.top() && build.bottom() <= freight.top());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_fix_chip_selects_its_cell_in_the_overview(cx: &mut TestAppContext) {
    let (handle, page) = open_overview(cx, 1280., 880., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The first chip that shows.
        let key = page
            .read(cx)
            .fixes
            .iter()
            .map(|fix| fix.hop.clone())
            .find(|key| window.try_find(format!("change-fixbar-{key}")).is_some())
            .expect("a chip shows");
        window.click(format!("change-fixbar-{key}"), cx);
        assert_eq!(
            page.read(cx).selected().map(|s| s.to_string()),
            Some(key.clone())
        );
        window.render_frame(cx);
        assert!(window.try_find("change-detail").is_some());
        // The hop is a cell of the matrix, or a card of the strip.
        let p = page.read(cx);
        let in_matrix = crate::change::overview::cells::locate(&p.overview, &key).is_some();
        let in_strip = p
            .strip
            .cards
            .iter()
            .any(|card| card.hops.iter().any(|hop| hop.as_ref() == key));
        assert!(in_matrix || in_strip, "{key} is shown in the Overview");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_claimed_link_is_a_dashed_track_under_a_healthy_glyph(cx: &mut TestAppContext) {
    use freshkube_ui::table::Track;
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        let mut change = page.read(cx).change.clone();
        let pods = change
            .hops
            .iter_mut()
            .find(|hop| hop.key == "dev-pods")
            .unwrap();
        pods.link = Some(Confidence::Claimed);
        if let Shows::Hop(detail) = &mut pods.shows
            && let Some(link) = detail.link.as_mut()
        {
            link.confidence = Confidence::Claimed;
        }
        page.update(cx, |page, cx| page.read_answer(change, window, cx));
        let dev = page.read(cx).overview[0].clone();
        let running = &dev.cells[4];
        // Status is the glyph's, confidence the track's.
        assert_eq!(running.tone, Tone::Good);
        assert_eq!(running.word.as_ref(), "Running");
        assert_eq!(running.track(), Track::Dashed);
        // A Confirmed link stays solid.
        assert_eq!(dev.cells[3].track(), Track::Solid);
    })
    .unwrap();
}

#[gpui_kit::test]
fn after_deploy_says_why_coroot_has_no_verdict(cx: &mut TestAppContext) {
    use freshkube_ui::table::Track;
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, _, cx| {
        for (why, word) in [
            (Some("Not set up"), "Not set up"),
            (Some("Not found"), "Not found"),
            (Some("Can't read"), "Can't read"),
            (None, "Not looked"),
        ] {
            page.update(cx, |page, _| {
                for row in page
                    .rows
                    .iter_mut()
                    .filter(|row| row.key == "dev-deployment")
                {
                    row.coroot_why = why;
                }
                page.derive_overview();
            });
            let cell = page.read(cx).overview[0].cells[5].clone();
            assert_eq!(cell.word.as_ref(), word);
            // A cause is not a failure.
            assert_ne!(cell.tone, Tone::Crit);
            assert_eq!(
                cell.track(),
                if why.is_some() {
                    Track::Dotted
                } else {
                    Track::Hairline
                }
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn not_found_is_drawn_on_the_builds_card_under_the_causes_shape(cx: &mut TestAppContext) {
    let (handle, page) = open_overview(cx, 1280., 880., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let build = page
            .read(cx)
            .strip
            .cards
            .iter()
            .find(|card| card.id == "build")
            .unwrap()
            .clone();
        assert!(build.detail.contains("Not found"), "{}", build.detail);
        let label = window
            .find("change-strip-build")
            .label()
            .unwrap()
            .to_owned();
        assert!(label.contains("Not found"), "{label}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_cell_keys_reach_what_is_behind_the_page_in_all_hops_only(cx: &mut TestAppContext) {
    use crate::change::{NextCell, PreviousCell};
    let (handle, page) = overview_fixture(cx);
    let reached = std::rc::Rc::new(std::cell::Cell::new(0));
    cx.update(|cx| {
        let seen = reached.clone();
        cx.on_action(move |_: &NextCell, _| seen.set(seen.get() + 1));
        let seen = reached.clone();
        cx.on_action(move |_: &PreviousCell, _| seen.set(seen.get() + 1));
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.focus(window, cx));
        window.press("right", cx);
        window.press("left", cx);
        assert_eq!(reached.get(), 0, "the Overview takes them");
        hops(&page, window, cx);
        page.update(cx, |page, cx| page.focus(window, cx));
        window.press("right", cx);
        window.press("left", cx);
        assert_eq!(reached.get(), 2, "All hops lets them through");
    })
    .unwrap();
}
