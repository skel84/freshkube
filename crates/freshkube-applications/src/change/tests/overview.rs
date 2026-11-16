//! The Overview: its strip, its matrix, and how they lay out (#823).
use super::*;
use crate::change::overview::Matrix;
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
            "No Rollout",
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
                "No Rollout",
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
    assert_eq!(ams.cells[4].track(), Track::Hairline, "not reached");
    let dev = &page[0];
    // The Next step cell has no Rollout here, so it is a hairline.
    assert_eq!(dev.cells[3].track(), Track::Hairline, "no Rollout");
    assert!(
        dev.cells[..3]
            .iter()
            .chain(&dev.cells[4..6])
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
        let running = &dev.cells[5];
        // Status is the glyph's, confidence the track's.
        assert_eq!(running.tone, Tone::Good);
        assert_eq!(running.word.as_ref(), "Running");
        assert_eq!(running.track(), Track::Dashed);
        // A Confirmed link stays solid.
        assert_eq!(dev.cells[4].track(), Track::Solid);
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
            let cell = page.read(cx).overview[0].cells[6].clone();
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
        // It names the hop that was not found, not the run it shows.
        assert!(!build.detail.contains("x7k2"), "{}", build.detail);
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

#[gpui_kit::test]
fn the_strip_stays_in_the_pane_beside_the_inspector(cx: &mut TestAppContext) {
    let (handle, page) = open_overview(cx, 1280., 880., Shape::Causes, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.select("prod-ams-pods", window, cx));
        settle(window, cx);
        let pane = bounds(window, "change-overview".into());
        let detail = bounds(window, "change-detail".into());
        // The pane beside the Inspector is under 640 dp, so the strip
        // stacks there, though the page itself is wide.
        let (build, freight) = (
            bounds(window, "change-strip-build".into()),
            bounds(window, "change-strip-freight".into()),
        );
        assert!(build.bottom() <= freight.top(), "{build:?} {freight:?}");
        for card in ["pr", "build", "freight"] {
            let card = bounds(window, format!("change-strip-{card}"));
            assert!(
                card.right() <= pane.right() + px(1.),
                "{card:?} in {pane:?}"
            );
            assert!(
                card.right() <= detail.left() + px(1.),
                "{card:?} under {detail:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_focused_cell_wears_the_keyboard_ring_and_loses_it_with_the_focus(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.select("stage-pods", window, cx);
            page.focus(window, cx);
        });
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(
            window.try_find("change-cell-stage-running-ring").is_some(),
            "the focused page's selected cell has the ring"
        );
        // Clicking away: another element takes the focus, and the ring goes.
        window.blur(cx);
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(
            window.try_find("change-cell-stage-running-ring").is_none(),
            "no ring without the keyboard"
        );
        // Only the selected cell has one.
        page.update(cx, |page, cx| page.focus(window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
        let rings = Column::ALL
            .iter()
            .filter(|column| {
                window
                    .try_find(format!("change-cell-stage-{}-ring", column.id()))
                    .is_some()
            })
            .count();
        assert_eq!(rings, 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_rollouts_shape_fills_the_next_step_column_and_says_why_when_it_cant(
    cx: &mut TestAppContext,
) {
    use freshkube_ui::table::Track;
    let (handle, page) = open_overview(cx, 1280., 880., Shape::Rollouts, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let next = |cx: &App, stage: &str| {
            let page = page.read(cx);
            let row = page.overview.iter().find(|row| row.name == stage).unwrap();
            row.cells[Column::ALL
                .iter()
                .position(|c| *c == Column::NextStep)
                .unwrap()]
            .clone()
        };
        for stage in ["dev", "stage", "prod-ams", "prod-lon"] {
            assert!(
                window
                    .find(format!("change-cell-{stage}-next-step"))
                    .visible(),
                "{stage}"
            );
        }
        let (dev, stage, ams, lon) = (
            next(cx, "dev"),
            next(cx, "stage"),
            next(cx, "prod-ams"),
            next(cx, "prod-lon"),
        );
        assert_eq!((dev.word.as_ref(), dev.tone), ("Done", Tone::Good));
        assert_eq!(
            (stage.word.as_ref(), stage.tone),
            ("Step 2 of 4", Tone::Info)
        );
        assert_eq!(ams.word.as_ref(), "Not reached");
        assert_eq!(ams.track(), Track::Hairline);
        // A Stage whose Application manages no Rollout. (A Rollout that
        // couldn't be read says its cause instead: see the refused
        // Applications test.)
        assert_eq!(lon.word.as_ref(), "No Rollout");
        assert!(!lon.reached);
        assert_eq!(lon.hop, None);
        // The Stages that run one have its hop, so the cell selects it.
        assert_eq!(dev.hop.as_deref(), Some("dev-rollout"));
        assert_eq!(stage.hop.as_deref(), Some("stage-rollout"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn every_next_step_has_a_word_that_fits_its_column(cx: &mut TestAppContext) {
    use freshkube_delivery::change::NextStep;
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        for (next, word, tone) in [
            (NextStep::Done, "Done", Tone::Good),
            (
                NextStep::Step { at: 10, of: 12 },
                "Step 10 of 12",
                Tone::Info,
            ),
            (NextStep::Paused { step: None }, "Paused", Tone::Info),
            (NextStep::Progressing, "Progressing", Tone::Waiting),
            (NextStep::Aborted, "Aborted", Tone::Crit),
            (
                NextStep::Degraded { step: Some((2, 4)) },
                "Degraded",
                Tone::Warn,
            ),
            (NextStep::NotReached, "Not reached", Tone::Outline),
            (NextStep::NoRollout, "No Rollout", Tone::Outline),
            (
                NextStep::Unknown {
                    why: "the Rollout was not read".into(),
                    cause: None,
                },
                "Unknown",
                Tone::Unknown,
            ),
        ] {
            let mut change = page.read(cx).change.clone();
            change.stages[0].next_step = next;
            page.update(cx, |page, cx| page.read_answer(change, window, cx));
            let cell = page.read(cx).overview[0].cells[3].clone();
            assert_eq!(cell.word.as_ref(), word);
            assert_eq!(cell.tone, tone, "{word}");
            assert!(cell.word.chars().count() <= 14, "{word}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_stage_nothing_looked_at_says_not_reached_and_a_refused_application_says_why(
    cx: &mut TestAppContext,
) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, _, cx| {
        // prod-ams still runs other Freight: nothing looked for its
        // Application, so it is not "No Rollout".
        let p = page.read(cx);
        let ams = p
            .overview
            .iter()
            .find(|row| row.name == "prod-ams")
            .unwrap();
        let cell = &ams.cells[3];
        assert_eq!(cell.word.as_ref(), "Not reached");
        assert!(!cell.reached);
        assert!(
            cell.tooltip.contains("still runs other Freight"),
            "{}",
            cell.tooltip
        );
    })
    .unwrap();
    let (handle, page) = open_overview(cx, 1280., 880., Shape::ArgoRefused, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let p = page.read(cx);
        let dev = p.overview.iter().find(|row| row.name == "dev").unwrap();
        let cell = &dev.cells[3];
        // Argo CD's Applications were refused: whether a Rollout runs is
        // not known, and the cell says why instead of "No Rollout".
        assert_eq!(cell.word.as_ref(), "Can't read");
        assert_ne!(cell.tone, Tone::Crit);
        assert!(cell.reached);
        assert!(window.find("change-cell-dev-next-step").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_right_arrow_runs_along_the_whole_row_to_after_deploy(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| {
            page.select("dev-pods", window, cx);
            page.focus(window, cx);
        });
        window.render_frame(cx);
        // Running is the sixth step, After deploy the seventh and last.
        window.press("right", cx);
        assert_eq!(
            page.read(cx).selected().map(|s| s.to_string()).as_deref(),
            Some("dev-deployment")
        );
        window.press("right", cx);
        assert_eq!(
            page.read(cx).selected().map(|s| s.to_string()).as_deref(),
            Some("dev-deployment"),
            "the row ends there"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_ring_and_the_arrow_keys_reach_a_next_step_cell(cx: &mut TestAppContext) {
    let (handle, page) = open_overview(cx, 1280., 880., Shape::Rollouts, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let selected = |cx: &App| page.read(cx).selected().map(|s| s.to_string());
        page.update(cx, |page, cx| {
            page.select("dev-verification", window, cx);
            page.focus(window, cx);
        });
        window.render_frame(cx);
        window.press("right", cx);
        assert_eq!(selected(cx).as_deref(), Some("dev-rollout"));
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(
            window.try_find("change-cell-dev-next-step-ring").is_some(),
            "the keyboard ring is on the Next step cell"
        );
        // The Rollout is its own hop: its Inspector opens, and the keys go on.
        window.press("right", cx);
        assert_eq!(selected(cx).as_deref(), Some("dev-argocd"));
        window.press("left", cx);
        assert_eq!(selected(cx).as_deref(), Some("dev-rollout"));
    })
    .unwrap();
}

/// A Stage row scrolled under the matrix's header is cut at the header's
/// edge, not drawn half behind it (#1068). Headless cannot read pixels, so
/// this holds what makes the cut: the list that clips its rows starts where
/// the header ends, wherever the rows sit, and a row half under the header
/// still reaches into the list.
#[gpui_kit::test]
fn a_stage_row_under_the_matrix_header_is_cut_at_its_edge(cx: &mut TestAppContext) {
    use gpui_kit::base::test_support::snapshots;
    let (handle, page) = open_overview(cx, 760., 560., Shape::Whole, Duration::ZERO, 20.);
    cx.update_window(handle, |_, window, cx| {
        scroll_to_matrix(&page, window, cx);
        let scroll = page.read(cx).matrix.read(cx).table_scroll();
        let row = freshkube_ui::ui::dp_px(freshkube_ui::table::STEP_ROW, window);
        let header = freshkube_ui::ui::dp_px(freshkube_ui::table::HEADER_HEIGHT, window);
        // Half a row, then a row and a bit, under the header.
        for under in [row / 2., row + row / 4.] {
            scroll
                .0
                .borrow()
                .base_handle
                .set_offset(point(px(0.), -under));
            settle(window, cx);
            window.render_frame(cx);
            let frame = bounds(window, "change-matrix-table-scroll".into());
            let list = bounds(window, "change-matrix-list".into());
            assert!(
                (list.top() - frame.top() - header).abs() < px(1.),
                "the list starts {:?} under the table's top, the header is {header:?}",
                list.top() - frame.top()
            );
            let id = gpui_kit::ElementId::from(SharedString::from("change-matrix-list"));
            let rows: Vec<_> = snapshots(window)
                .into_iter()
                .filter(|line| {
                    line.role() == Some(gpui_kit::Role::ListBoxOption) && line.path().contains(&id)
                })
                .collect();
            assert!(rows.len() >= 2, "{rows:?}");
            let cut = rows
                .iter()
                .any(|line| line.bounds().top() < list.top() - px(1.) && line.visible());
            assert!(cut, "a row is partly under the header: {rows:?}");
            for line in rows.iter().filter(|line| line.visible()) {
                let shown = line.bounds().intersect(&list);
                assert!(
                    shown.top() >= list.top() - px(0.5)
                        && shown.bottom() <= list.bottom() + px(0.5),
                    "{:?} shows outside the list {list:?}",
                    line.bounds()
                );
            }
        }
    })
    .unwrap();
}

/// The matrix at 760×560 and text 20 shows about a row and a half (#1080):
/// arrowing down to the last Stage, right to the last column, and back,
/// keeps the selected cell inside the matrix's visible bounds, below its
/// header and right of the pinned Stage column. Each key waits for its
/// frames, as a person's do. It proves where the reveal ends up, not how
/// many frames it took, and the shapes differ in the Inspector's height
/// under the matrix.
#[gpui_kit::test]
fn arrowing_to_the_last_cell_keeps_it_inside_the_matrix(cx: &mut TestAppContext) {
    arrow_through(cx, Shape::Whole);
}

#[gpui_kit::test]
fn arrowing_to_the_last_cell_keeps_it_inside_the_matrix_with_all_four_causes(
    cx: &mut TestAppContext,
) {
    arrow_through(cx, Shape::Causes);
}

fn arrow_through(cx: &mut TestAppContext, shape: Shape) {
    let (handle, page) = open_overview(cx, 760., 560., shape, Duration::ZERO, 20.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.focus(window, cx));
        let stages: Vec<_> = page
            .read(cx)
            .overview
            .iter()
            .map(|row| row.name.to_string())
            .collect();
        assert!(stages.len() >= 3, "{stages:?}");
        let header = freshkube_ui::ui::dp_px(freshkube_ui::table::HEADER_HEIGHT, window);
        let pinned = freshkube_ui::ui::dp_px(220., window);
        let inside = |window: &Window, stage: &str, column: Column| {
            let view = bounds(window, "change-matrix-table-scroll".into());
            let cell = bounds(window, Matrix::cell_id(stage, column).to_string());
            // The row's own 1 px borders either side are not part of a cell.
            let (top, bottom) = (view.top() + header, view.bottom());
            let left = view.left() + pinned;
            assert!(
                cell.top() >= top - px(1.) && cell.bottom() <= bottom + px(1.),
                "{stage} {column:?}: {cell:?} not between {top:?} and {bottom:?}"
            );
            assert!(
                cell.left() >= left - px(1.) && cell.right() <= view.right() + px(3.),
                "{stage} {column:?}: {cell:?} not between {left:?} and {:?}",
                view.right()
            );
        };
        window.press("down", cx);
        scroll_to_matrix(&page, window, cx);
        // Down through every Stage, checking each one, then right through
        // every column of the last, and back up and left.
        for stage in &stages {
            settle(window, cx);
            inside(window, stage, Column::Eligible);
            window.press("down", cx);
        }
        let last = stages.last().unwrap().clone();
        for _ in 0..10 {
            window.press("right", cx);
            settle(window, cx);
        }
        settle(window, cx);
        inside(window, &last, Column::AfterDeploy);
        for _ in 1..stages.len() {
            window.press("up", cx);
            settle(window, cx);
        }
        for _ in 0..10 {
            window.press("left", cx);
            settle(window, cx);
        }
        settle(window, cx);
        inside(window, &stages[0], Column::Eligible);
    })
    .unwrap();
}

/// A column scrolled under the pinned Stage column is cut at its right
/// edge (#1081). Headless cannot read pixels, so this holds what the cut
/// decides: a snapshot is `visible` only while some of it lies inside the
/// content mask the table gives the passing cells. At 760×560 and text 20,
/// scrolled to the last column, every cell and word that lies wholly under
/// the pinned run is not visible, one that straddles its edge is, and none
/// shows left of it. The mask's own pixel snapping is not readable here.
#[gpui_kit::test]
fn a_scrolled_column_is_cut_at_the_pinned_stage_columns_edge(cx: &mut TestAppContext) {
    use gpui_kit::base::test_support::snapshots;
    let (handle, page) = open_overview(cx, 760., 560., Shape::Whole, Duration::ZERO, 20.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.focus(window, cx));
        window.press("down", cx);
        scroll_to_matrix(&page, window, cx);
        for _ in 0..8 {
            window.press("right", cx);
            settle(window, cx);
        }
        let edge = bounds(window, "change-stage-dev".into()).right();
        let (mut hidden, mut straddling) = (0, 0);
        for line in snapshots(window) {
            let Some(gpui_kit::ElementId::Name(name)) = line.path().last() else {
                continue;
            };
            let passing = name.starts_with("change-cell-")
                && (name.ends_with("-word") || name.matches('-').count() <= 3 + 1);
            let list = bounds(window, "change-matrix-list".into());
            let (top, bottom) = (line.bounds().top(), line.bounds().bottom());
            if !passing || top < list.top() || bottom > list.bottom() {
                continue;
            }
            let at = line.bounds();
            if at.right() <= edge {
                assert!(
                    !line.visible(),
                    "{name} shows left of the pinned edge: {at:?}"
                );
                hidden += 1;
            } else if at.left() < edge && at.right() > edge + px(1.) {
                assert!(
                    line.visible(),
                    "{name} straddles the edge and shows: {at:?}"
                );
                straddling += 1;
            }
        }
        assert!(
            hidden > 0 && straddling > 0,
            "{hidden} hidden, {straddling} straddling"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_rollout_states_shape_draws_paused_degraded_aborted_and_progressing(cx: &mut TestAppContext) {
    let (handle, page) = open_overview(cx, 1280., 880., Shape::RolloutStates, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let cell = |cx: &App, stage: &str| {
            let page = page.read(cx);
            let row = page.overview.iter().find(|row| row.name == stage).unwrap();
            row.cells[3].clone()
        };
        for (stage, word, tone) in [
            ("dev", "Paused", Tone::Info),
            ("stage", "Degraded", Tone::Warn),
            ("prod-ams", "Aborted", Tone::Crit),
            ("prod-lon", "Progressing", Tone::Waiting),
        ] {
            let cell = cell(cx, stage);
            assert_eq!((cell.word.as_ref(), cell.tone), (word, tone), "{stage}");
            assert!(cell.reached, "{stage} is reached, so its state shows");
            // Each Stage has its Rollout hop, so the cell selects it.
            assert_eq!(
                cell.hop.as_deref(),
                Some(format!("{stage}-rollout").as_str())
            );
            assert!(
                window
                    .find(format!("change-cell-{stage}-next-step"))
                    .visible()
            );
        }
        // Selecting one opens the Rollout's Inspector with its state.
        page.update(cx, |page, cx| page.select("prod-ams-rollout", window, cx));
        window.render_frame(cx);
        assert!(window.try_find("change-detail").is_some());
    })
    .unwrap();
}
