use super::stories::data_table::{DataTableStory, Sort, State};
use super::stories::dock::{self as dock_story, DockStory};
use super::stories::drawer::DrawerStory;
use super::stories::graph::{GraphStory, Shape};
use super::stories::motion::flash::{BURST, FlashStory, PODS, Rate};
use super::stories::motion::loading::LoadingStory;
use super::{STORIES, Workbench, window_size};
use freshkube_probe::probe;
use freshkube_ui::motion::{self, Choice};
use freshkube_ui::table::SortOrder;
use freshkube_ui::table::{Look, Reduced};
use freshkube_ui::text_size;
use gpui_kit::component::{ActiveTheme, Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, ScrollDelta, TestAppContext, point, px, size};
use std::time::Duration;

/// The workbench on `story` in a 1280 × 880 window, light, at text 13.
fn open(cx: &mut TestAppContext, story: usize) -> (AnyWindowHandle, Entity<Workbench>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        freshkube_ui::theme::install(cx);
        text_size::install(None, cx);
        Theme::change(ThemeMode::Light, None, cx);
        // Dialogs animate on the real clock, which the test clock can't
        // advance; reduced motion opens and closes them at once.
        cx.set_reduce_motion(true);
    });
    let mut view = None;
    let window = cx.open_window(size(px(1280.), px(880.)), |window, cx| {
        let workbench = cx.new(|cx| Workbench::new(story, window, cx));
        view = Some(workbench.clone());
        Root::new(workbench, window, cx)
    });
    cx.run_until_parked();
    (window.into(), view.unwrap())
}

fn data_table(workbench: &Entity<Workbench>, cx: &gpui_kit::App) -> Entity<DataTableStory> {
    workbench
        .read(cx)
        .view()
        .clone()
        .downcast::<DataTableStory>()
        .unwrap()
}

#[gpui_kit::test]
fn every_story_draws_in_both_themes_at_text_13_and_20(cx: &mut TestAppContext) {
    for (ix, story) in STORIES.iter().enumerate() {
        let (handle, _) = open(cx, ix);
        for dark in [false, true] {
            for text in [13., 20.] {
                cx.update_window(handle, |_, window, cx| {
                    let mode = if dark {
                        ThemeMode::Dark
                    } else {
                        ThemeMode::Light
                    };
                    Theme::change(mode, Some(window), cx);
                    text_size::set(text, cx);
                    window.render_frame(cx);
                    assert_eq!(cx.theme().mode.is_dark(), dark);
                    assert_eq!(window.rem_size(), px(text));
                    let page = window.find(format!("{}-page", story.slug));
                    assert!(page.visible(), "{} at {text}, dark {dark}", story.slug);
                    assert!(window.find(format!("story-{}", story.slug)).visible());
                })
                .unwrap();
            }
        }
    }
}

#[gpui_kit::test]
fn the_strip_changes_theme_text_size_and_width(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, 0);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("workbench-theme-dark", cx);
        assert!(cx.theme().mode.is_dark());
        window.click("workbench-theme-light", cx);
        assert!(!cx.theme().mode.is_dark());

        window.click("workbench-text-20", cx);
        window.render_frame(cx);
        assert_eq!(text_size::current(cx), 20.);
        assert_eq!(window.rem_size(), px(20.));
        window.click("workbench-text-13", cx);
        assert_eq!(text_size::current(cx), 13.);

        window.click("workbench-width-760", cx);
        assert_eq!(window.bounds().size.width, px(760.));
        window.click("workbench-width-1280", cx);
        assert_eq!(window.bounds().size.width, px(1280.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_data_table_story_shows_each_state_and_returns_to_its_rows(cx: &mut TestAppContext) {
    let (handle, workbench) = open(cx, 0);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let story = data_table(&workbench, cx);
        assert!(window.find(("data-table-group", 0usize)).visible());
        for (id, state) in [
            ("loading", State::Loading),
            ("empty", State::Empty),
            ("failed", State::Failed),
        ] {
            window.click(format!("data-table-state-{id}"), cx);
            window.render_frame(cx);
            assert_eq!(story.read(cx).state(), state);
            assert!(window.find(format!("data-table-{id}")).visible());
            assert!(window.try_find(("data-table-group", 0usize)).is_none());
        }
        window.click("data-table-retry", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).state(), State::Rows);
        assert!(window.find(("data-table-group", 0usize)).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_data_table_story_sorts_and_holds_two_thousand_rows(cx: &mut TestAppContext) {
    let (handle, workbench) = open(cx, 0);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let story = data_table(&workbench, cx);
        assert_eq!(story.read(cx).rows(), 24);
        assert_eq!(
            story.read(cx).sorted_by(),
            (Sort::Name, SortOrder::Ascending)
        );
        let ascending = story.read(cx).names();
        // Column 1 is Name; a second press reverses it.
        window.click(("pods-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(
            story.read(cx).sorted_by(),
            (Sort::Name, SortOrder::Descending)
        );
        let descending = story.read(cx).names();
        assert_ne!(ascending, descending);
        assert_eq!(
            sorted(ascending),
            sorted(descending),
            "the same pods, in another order"
        );
        window.click("data-table-count-2000", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).rows(), 2_000);
        assert_eq!(story.read(cx).names().len(), 2_000);
        assert!(window.find(("data-table-group", 0usize)).visible());
    })
    .unwrap();
}

fn sorted(mut names: Vec<gpui_kit::SharedString>) -> Vec<gpui_kit::SharedString> {
    names.sort();
    names
}

#[test]
fn window_sizes_read_as_the_app_reads_them() {
    assert_eq!(window_size("1280x880"), Some(size(px(1280.), px(880.))));
    assert_eq!(window_size("700x880"), None);
    assert_eq!(window_size("wide"), None);
}

#[gpui_kit::test]
fn the_list_switches_stories(cx: &mut TestAppContext) {
    let (handle, workbench) = open(cx, 0);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("story-data-table").selected(), Some(true));
        window.click("story-graph", cx);
        window.render_frame(cx);
        assert_eq!(STORIES[workbench.read(cx).story()].slug, "graph");
        assert_eq!(window.find("story-graph").selected(), Some(true));
        assert!(window.find("graph-page").visible());
        assert!(window.try_find("data-table-page").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_graph_story_lays_out_each_shape(cx: &mut TestAppContext) {
    let graph = super::stories::find("graph").unwrap();
    let (handle, workbench) = open(cx, graph);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let story = workbench
            .read(cx)
            .view()
            .clone()
            .downcast::<GraphStory>()
            .unwrap();
        let canvas = window.find("graph-graph").bounds();
        // shop-web calls shop-api: the caller sits left of the callee.
        let web = window.find("graph-node-0").bounds();
        let api = window.find("graph-node-1").bounds();
        assert!(web.right() < api.left());
        assert!(canvas.contains(&web.origin) && canvas.contains(&api.origin));
        for (shape, id) in [
            (Shape::FanOut, "fan-out"),
            (Shape::Cycle, "cycle"),
            (Shape::Loose, "loose"),
            (Shape::Shop, "shop"),
        ] {
            window.click(format!("graph-shape-{id}"), cx);
            window.render_frame(cx);
            assert_eq!(story.read(cx).shape(), shape);
            let positions = story.read(cx).positions();
            for (ix, (label, x, y)) in positions.iter().enumerate() {
                assert!(
                    positions[ix + 1..]
                        .iter()
                        .all(|(_, x2, y2)| (x, y) != (x2, y2)),
                    "{label} shares its place in {id}"
                );
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_graph_story_routes_its_fan_out_without_crossings(cx: &mut TestAppContext) {
    let graph = super::stories::find("graph").unwrap();
    let (handle, workbench) = open(cx, graph);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let story = workbench
            .read(cx)
            .view()
            .clone()
            .downcast::<GraphStory>()
            .unwrap();
        window.click("graph-shape-fan-out", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).shape(), Shape::FanOut);
        assert_eq!(story.read(cx).crossings(), 0);
        // The wrapped column is drawn too.
        assert!(window.find("graph-node-12").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_graph_story_selects_a_call_and_filters_problems(cx: &mut TestAppContext) {
    use freshkube_ui::graph::GraphSource;
    let graph = super::stories::find("graph").unwrap();
    let (handle, workbench) = open(cx, graph);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let story = workbench
            .read(cx)
            .view()
            .clone()
            .downcast::<GraphStory>()
            .unwrap();
        // checkout calls payments-db, and that call is failing.
        window.click("graph-call-2--6", cx);
        assert_eq!(story.read(cx).graph().selected(), Some(&(2, 6)));
        window.render_frame(cx);
        window.click("graph-problems", cx);
        window.render_frame(cx);
        let graph = story.read(cx).graph();
        assert_eq!(graph.visible_count(), 2);
        assert_eq!(graph.placed().count(), 8);
        assert!(window.try_find("graph-call-0--1").is_none());
        assert!(window.try_find("graph-call-2--6").is_some());
    })
    .unwrap();
}

fn story<T: 'static>(workbench: &Entity<Workbench>, cx: &gpui_kit::App) -> Entity<T> {
    workbench.read(cx).view().clone().downcast::<T>().unwrap()
}

/// The workbench on the change flash with its timer off, at `motion`.
fn open_flash(cx: &mut TestAppContext, motion: Choice) -> (AnyWindowHandle, Entity<FlashStory>) {
    let (handle, workbench) = open(cx, super::stories::find("change-flash").unwrap());
    cx.update_window(handle, |_, window, cx| {
        motion::choose(motion, cx);
        window.render_frame(cx);
        window.click("change-flash-rate-off", cx);
    })
    .unwrap();
    let story = cx.read(|cx| story::<FlashStory>(&workbench, cx));
    cx.read(|cx| assert_eq!(story.read(cx).rate(), Rate::Off));
    (handle, story)
}

/// A frame's worth of time on the test clock.
const FRAME: Duration = Duration::from_millis(16);

/// Moves the clock on a frame, delivers the next frame and draws it;
/// returns how many frame requests it answered.
fn next_frame(handle: AnyWindowHandle, cx: &mut TestAppContext) -> usize {
    cx.background_executor.advance_clock(FRAME);
    let asked = cx
        .update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    cx.run_until_parked();
    asked
}

/// Moves the clock on one fade step and draws what that asks; returns
/// how many times the flash layer drew.
fn next_step(cx: &mut TestAppContext) -> usize {
    let layer = probe::count("table.flash-layer");
    cx.background_executor.advance_clock(motion::FLASH_STEP);
    cx.run_until_parked();
    probe::count("table.flash-layer") - layer
}

fn strength(story: &Entity<FlashStory>, cx: &mut TestAppContext) -> Option<f32> {
    cx.read(|cx| {
        let layer = story.read(cx).layer().read(cx);
        let key = *layer.flashing(cx).first()?;
        layer.strength(&key, cx)
    })
}

#[gpui_kit::test]
fn the_strip_overrides_reduced_motion(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, 0);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("workbench-motion-full", cx);
        assert_eq!(motion::choice(cx), Choice::Full);
        assert!(!cx.reduce_motion());
        window.click("workbench-motion-reduced", cx);
        assert!(cx.reduce_motion());
        // Nothing read the OS here, so System keeps motion on.
        window.click("workbench-motion-system", cx);
        assert_eq!(motion::choice(cx), Choice::System);
        assert!(!cx.reduce_motion());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_flash_draws_only_its_layer_and_the_views_above(cx: &mut TestAppContext) {
    let (handle, story) = open_flash(cx, Choice::Full);
    cx.update_window(handle, |_, _, cx| {
        story.update(cx, |story, cx| story.change(1, cx));
    })
    .unwrap();
    assert_eq!(strength(&story, cx), Some(1.));
    cx.run_until_parked();
    // The change itself drew the table once; the fade's steps don't.
    let table = probe::count("workbench.change-flash-table");
    let root = probe::count("workbench.change-flash");
    // The fade asks for no frames: a timer draws each step, once.
    assert_eq!(next_frame(handle, cx), 0);
    let steps = motion::FLASH_STEPS as usize;
    for step in 1..steps {
        assert_eq!(next_step(cx), 1, "step {step}");
        let left = strength(&story, cx).unwrap();
        let expected = motion::fade_left(motion::FLASH_STEP * step as u32);
        assert!((left - expected).abs() < 0.05, "{left} at step {step}");
    }
    assert_eq!(probe::count("workbench.change-flash-table"), table);
    // The layer's ancestors draw with it: the story's root, and the window's.
    assert!(probe::count("workbench.change-flash") >= root + steps - 1);
    // The fade ends, and with it the steps.
    assert!(next_step(cx) >= 1, "the last step clears it");
    assert_eq!(strength(&story, cx), None);
    cx.read(|cx| assert!(!story.read(cx).layer().read(cx).stepping()));
    assert_eq!(next_step(cx), 0);
    assert_eq!(next_frame(handle, cx), 0);
}

/// A flash that starts between two steps still clears when its fade
/// ends, not at the next step.
#[gpui_kit::test]
fn a_flash_between_steps_clears_on_time(cx: &mut TestAppContext) {
    let (handle, story) = open_flash(cx, Choice::Full);
    let change = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, _, cx| {
            story.update(cx, |story, cx| story.change(1, cx));
        })
        .unwrap();
        cx.run_until_parked();
    };
    let flashing =
        |cx: &mut TestAppContext| cx.read(|cx| story.read(cx).layer().read(cx).flashing(cx).len());
    change(cx);
    let offset = motion::FLASH_STEP / 2;
    cx.background_executor.advance_clock(offset);
    cx.run_until_parked();
    change(cx);
    assert_eq!(flashing(cx), 2);
    // The first ends on a step; the second half a step later, between two.
    cx.background_executor.advance_clock(motion::FADE - offset);
    cx.run_until_parked();
    assert_eq!(flashing(cx), 1);
    let layer = probe::count("table.flash-layer");
    cx.background_executor.advance_clock(offset);
    cx.run_until_parked();
    assert_eq!(flashing(cx), 0);
    assert_eq!(
        probe::count("table.flash-layer"),
        layer + 1,
        "cleared at its end"
    );
    assert_eq!(next_step(cx), 0, "and nothing steps after");
}

#[gpui_kit::test]
fn a_heavy_root_still_leaves_the_table_alone(cx: &mut TestAppContext) {
    let (handle, story) = open_flash(cx, Choice::Full);
    cx.update_window(handle, |_, window, cx| {
        window.click("change-flash-root-heavy", cx);
        story.update(cx, |story, cx| story.change(1, cx));
    })
    .unwrap();
    cx.read(|cx| assert!(story.read(cx).heavy()));
    cx.run_until_parked();
    let table = probe::count("workbench.change-flash-table");
    for _ in 0..3 {
        assert_eq!(next_step(cx), 1);
    }
    assert_eq!(probe::count("workbench.change-flash-table"), table);
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("change-flash-heavy").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn under_reduced_motion_a_flash_asks_no_frames(cx: &mut TestAppContext) {
    let (handle, story) = open_flash(cx, Choice::Reduced);
    cx.update_window(handle, |_, _, cx| {
        story.update(cx, |story, cx| story.change(1, cx));
    })
    .unwrap();
    assert_eq!(next_frame(handle, cx), 0);
    cx.run_until_parked();
    assert_eq!(next_step(cx), 0, "nor steps");
    // The change is still recorded, and with No tint draws nothing.
    let flashing = cx.read(|cx| story.read(cx).layer().read(cx).flashing(cx));
    assert_eq!(flashing.len(), 1);
    assert_eq!(strength(&story, cx), None);
}

#[gpui_kit::test]
fn a_held_tint_clears_at_once_when_the_fade_would_end(cx: &mut TestAppContext) {
    let (handle, story) = open_flash(cx, Choice::Reduced);
    cx.update_window(handle, |_, window, cx| {
        window.click("change-flash-reduced-held", cx);
        story.update(cx, |story, cx| story.change(1, cx));
    })
    .unwrap();
    let layer = cx.read(|cx| story.read(cx).layer().clone());
    cx.read(|cx| assert_eq!(layer.read(cx).reduced(), Reduced::Held));
    assert_eq!(next_frame(handle, cx), 0, "a held tint doesn't animate");
    assert_eq!(strength(&story, cx), Some(1.));
    let renders = probe::count("table.flash-layer");
    cx.background_executor.advance_clock(motion::FADE);
    cx.run_until_parked();
    cx.read(|cx| assert!(layer.read(cx).flashing(cx).is_empty()));
    // One redraw clears it.
    assert_eq!(probe::count("table.flash-layer"), renders + 1);
    assert_eq!(next_frame(handle, cx), 0);
}

#[gpui_kit::test]
fn full_motion_mid_flash_takes_the_fade_up_where_it_is(cx: &mut TestAppContext) {
    let (handle, story) = open_flash(cx, Choice::Reduced);
    cx.update_window(handle, |_, window, cx| {
        window.click("change-flash-reduced-held", cx);
        story.update(cx, |story, cx| story.change(1, cx));
    })
    .unwrap();
    cx.background_executor.advance_clock(motion::FADE / 2);
    cx.update(|cx| motion::choose(Choice::Full, cx));
    let left = strength(&story, cx).unwrap();
    assert!(
        (left - motion::fade_step(motion::FADE / 2)).abs() < 1e-3,
        "{left}"
    );
    cx.run_until_parked();
    assert!(next_step(cx) > 0, "and fades on from there");
}

#[gpui_kit::test]
fn full_motion_after_no_tint_shows_the_fade_where_it_is(cx: &mut TestAppContext) {
    let (handle, story) = open_flash(cx, Choice::Reduced);
    cx.update_window(handle, |_, _, cx| {
        story.update(cx, |story, cx| story.change(1, cx));
    })
    .unwrap();
    assert_eq!(strength(&story, cx), None, "no tint while reduced");
    cx.background_executor.advance_clock(motion::FADE / 2);
    cx.update(|cx| motion::choose(Choice::Full, cx));
    let left = strength(&story, cx).unwrap();
    assert!(
        (left - motion::fade_step(motion::FADE / 2)).abs() < 1e-3,
        "{left}"
    );
    cx.run_until_parked();
    assert!(next_step(cx) > 0, "and fades on from there");
}

#[gpui_kit::test]
fn a_burst_changes_rows_without_flashing_them(cx: &mut TestAppContext) {
    let (handle, story) = open_flash(cx, Choice::Full);
    let before = cx.read(|cx| story.read(cx).table().read(cx).states());
    cx.update_window(handle, |_, window, cx| {
        window.click("change-flash-burst", cx);
    })
    .unwrap();
    cx.read(|cx| {
        let after = story.read(cx).table().read(cx).states();
        assert_eq!(
            before.iter().zip(&after).filter(|(a, b)| a != b).count(),
            BURST.min(PODS)
        );
        assert!(story.read(cx).layer().read(cx).flashing(cx).is_empty());
    });
    // Right after a burst a single change is still part of it.
    cx.update_window(handle, |_, window, cx| window.click("change-flash-one", cx))
        .unwrap();
    cx.read(|cx| assert!(story.read(cx).layer().read(cx).flashing(cx).is_empty()));
    cx.background_executor.advance_clock(motion::FADE * 2);
    cx.update_window(handle, |_, window, cx| window.click("change-flash-one", cx))
        .unwrap();
    cx.read(|cx| assert_eq!(story.read(cx).layer().read(cx).flashing(cx).len(), 1));
}

#[gpui_kit::test]
fn the_flash_finds_its_rows_after_both_scrolls(cx: &mut TestAppContext) {
    let (handle, story) = open_flash(cx, Choice::Full);
    cx.update_window(handle, |_, window, cx| {
        // At text 20 the columns are wider than the table, so it scrolls
        // sideways.
        text_size::set(20., cx);
        window.render_frame(cx);
        window.scroll(
            "flash-pods-list",
            ScrollDelta::Pixels(point(px(0.), px(-150.))),
            cx,
        );
        window.scroll(
            "flash-pods-table-scroll",
            ScrollDelta::Pixels(point(px(-120.), px(0.))),
            cx,
        );
        window.render_frame(cx);
        let table = story.read(cx).table().read(cx);
        let rows = table.rows_at();
        let list = window.find("flash-pods-list").bounds();
        // What shows of the rows: the list, inside the sideways scroll.
        let shown = window
            .find("flash-pods-table-scroll")
            .bounds()
            .intersect(&list);
        let (first, _) = rows.line(0, window).unwrap();
        assert!(first.top() < list.top(), "the rows scrolled down");
        let mut seen = 0;
        for line in 0..PODS {
            let id = table.pod_at(line).unwrap();
            let Some(row) = window.try_find(("flash-pods-row", id)) else {
                continue;
            };
            let row = row.bounds();
            let (tint, viewport) = rows.line(line, window).unwrap();
            assert!(row.left() < viewport.left(), "the rows scrolled sideways");
            assert!(
                (f32::from(tint.top() - row.top())).abs() < 0.5,
                "line {line}"
            );
            assert!(
                (f32::from(tint.size.height - row.size.height)).abs() < 0.5,
                "line {line}"
            );
            assert_eq!(viewport, shown, "line {line}");
            assert_eq!(tint.left(), shown.left());
            assert_eq!(tint.size.width, shown.size.width);
            seen += 1;
        }
        assert!(seen > 5, "{seen} rows checked");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_loading_motion_leaves_the_table_alone(cx: &mut TestAppContext) {
    let (handle, workbench) = open(cx, super::stories::find("loading").unwrap());
    let story = cx.read(|cx| story::<LoadingStory>(&workbench, cx));
    let motion = cx.read(|cx| story.read(cx).motion().clone());
    for (look, id) in [(Look::Pulse, "pulse"), (Look::Shimmer, "shimmer")] {
        cx.update_window(handle, |_, window, cx| {
            motion::choose(Choice::Reduced, cx);
            window.click(format!("loading-look-{id}"), cx);
            assert_eq!(motion.read(cx).look(), look);
            assert!(window.find("loading-pods-loading").visible());
            // The table's real header shows above the rows.
            assert!(window.find(("loading-pods-sort", 1usize)).visible());
        })
        .unwrap();
        // The frame asked before reduced motion ran out; none follows it.
        next_frame(handle, cx);
        assert_eq!(next_frame(handle, cx), 0, "{id} stands still");
        cx.update(|cx| motion::choose(Choice::Full, cx));
        cx.run_until_parked();
        next_frame(handle, cx);
        let table = probe::count("workbench.loading-table");
        let moved = probe::count("table.loading-motion");
        for _ in 0..4 {
            assert!(next_frame(handle, cx) > 0, "{id} moves");
        }
        assert_eq!(probe::count("table.loading-motion"), moved + 4);
        assert_eq!(
            probe::count("workbench.loading-table"),
            table,
            "{id}'s frames leave the table alone"
        );
    }
}

#[gpui_kit::test]
fn the_story_answers_with_the_loading_rows_gone(cx: &mut TestAppContext) {
    let (handle, workbench) = open(cx, super::stories::find("loading").unwrap());
    let story = cx.read(|cx| story::<LoadingStory>(&workbench, cx));
    let table = cx.read(|cx| story.read(cx).table().clone());
    cx.update(|cx| table.update(cx, |table, cx| table.answer(cx)));
    cx.run_until_parked();
    // Under reduced motion nothing asks frames, so the story's own Kit
    // skeleton doesn't hide what the motion does; freshkube-ui's table
    // tests count the motion's frames on their own.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("loading-pods-loading").is_none());
        // The wake the table asked when the rows first showed, then none.
        window.simulate_next_frame(cx);
        assert_eq!(window.simulate_next_frame(cx), 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_dock_story_adds_closes_resizes_and_fits_its_tabs(cx: &mut TestAppContext) {
    let (handle, workbench) = open(cx, super::stories::find("dock").unwrap());
    let story = cx.update(|cx| story::<DockStory>(&workbench, cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = window.find("dock-page").bounds();
        let dock = window.find("dock").bounds();
        // The dock spans the story under its page, at the default height.
        assert!((dock.bottom() - page.bottom()).abs() <= px(1.), "{dock:?}");
        assert!((dock.size.width - page.size.width).abs() <= px(1.));
        assert!((dock.size.height - px(freshkube_ui::dock::DEFAULT_HEIGHT)).abs() <= px(1.));
        window.click("dock-add", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).tabs().len(), 3);
        assert_eq!(window.find("dock-tab-2").selected(), Some(true));
        window.click("dock-tab-0-close", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).tabs().len(), 2);

        // Below the least height the dock minimizes to its bar.
        story.update(cx, |story, cx| story.resize(60., cx));
        window.render_frame(cx);
        assert_eq!(story.read(cx).state(), dock_story::State::Minimized);
        assert!(window.try_find("dock-body").is_none());
        story.update(cx, |story, cx| story.resize(220., cx));
        window.render_frame(cx);
        let dock = window.find("dock").bounds();
        assert!((dock.size.height - px(220.)).abs() <= px(1.), "{dock:?}");

        // Fit to window takes the page's room.
        window.click("dock-state-fitted", cx);
        window.render_frame(cx);
        let page = window.find("dock-page").bounds();
        let dock = window.find("dock").bounds();
        assert!(
            (dock.size.height - page.size.height).abs() <= px(1.),
            "{dock:?} {page:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_drawer_story_opens_swaps_resizes_and_closes(cx: &mut TestAppContext) {
    use freshkube_ui::drawer::{MIN_WIDTH, WIDTH};
    let (handle, workbench) = open(cx, super::stories::find("drawer").unwrap());
    let story = cx.update(|cx| story::<DrawerStory>(&workbench, cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("drawer-frame").is_none());
        window.click("drawer-row-2", cx);
        window.render_frame(cx);
        window.render_frame(cx);
        let body = window.find("drawer-body").bounds();
        let frame = window.find("drawer-frame").bounds();
        assert!((frame.right() - body.right()).abs() <= px(1.), "{frame:?}");
        assert!((frame.top() - body.top()).abs() <= px(1.), "{frame:?}");
        assert!((frame.size.width - px(WIDTH)).abs() <= px(1.), "{frame:?}");
        assert_eq!(
            window.find("drawer-object").label(),
            Some("worker-5c8d7-hq4tn")
        );

        // Another row swaps what it shows; the edge resizes within bounds.
        window.click_at("drawer-row-0", point(px(40.), px(8.)), cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).selected(), Some(0));
        assert!(story.read(cx).is_open());
        let y = frame.center().y;
        window.drag(
            point(frame.left() + px(2.), y),
            point(frame.left() + px(200.), y),
            cx,
        );
        window.render_frame(cx);
        // The edge follows the pointer.
        let width = story.read(cx).width();
        assert!((width - (WIDTH - 200.)).abs() <= 1., "{width}");
        story.update(cx, |story, cx| story.resize(10., window, cx));
        assert_eq!(story.read(cx).width(), MIN_WIDTH);

        // A click beside the rows closes it.
        let list = window.find("drawer-list").bounds();
        window.click_at(
            "drawer-list",
            point(px(40.), list.size.height - px(10.)),
            cx,
        );
        window.render_frame(cx);
        assert!(!story.read(cx).is_open());
        assert!(window.try_find("drawer-frame").is_none());
    })
    .unwrap();
}

/// The squares story draws a row per state, and Many draws a pod whose
/// squares the cell cuts at eight and counts the rest.
#[gpui_kit::test]
fn the_squares_story_shows_each_state_and_a_crowded_pod(cx: &mut TestAppContext) {
    use super::stories::squares::SquaresStory;
    let ix = super::stories::find("squares").unwrap();
    let (handle, workbench) = open(cx, ix);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let story = workbench
            .read(cx)
            .view()
            .clone()
            .downcast::<SquaresStory>()
            .unwrap();
        for pod in ["app-a", "app-e", "app-h", "shop-api"] {
            assert!(window.find(format!("squares-{pod}")).visible(), "{pod}");
        }
        window.click("squares-many", cx);
        window.render_frame(cx);
        assert!(story.read(cx).many());
        let crowded = window.find("squares-shop-mesh").bounds();
        assert!(window.try_find("squares-app-a").is_none());
        assert!(crowded.size.width > px(0.));
        window.click("squares-states", cx);
        window.render_frame(cx);
        assert!(!story.read(cx).many());
        assert!(window.find("squares-app-a").visible());
    })
    .unwrap();
}

/// The change page opens on prod-ams waiting for promotion, with the
/// Stages that are fully fine folded; a row shows its hop, and an action
/// says where it would lead, naming the cluster.
#[gpui_kit::test]
fn the_change_story_opens_on_the_waiting_stage_and_shows_each_hop(cx: &mut TestAppContext) {
    use super::stories::change::{ChangeStory, FIRST};
    let (handle, workbench) = open(cx, super::stories::find("change-trail").unwrap());
    let story = cx.update(|cx| story::<ChangeStory>(&workbench, cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(story.read(cx).selected(), Some(FIRST));
        assert_eq!(
            window.find("change-trail-detail-title").label(),
            Some("Kargo Stage prod-ams: Waiting for promotion")
        );
        let shown = story.read(cx).shown();
        assert!(!shown.contains(&"dev-pods") && !shown.contains(&"stage-pods"));
        assert!(shown.contains(&"prod-ams-pods") && shown.contains(&"prod-fra-pods"));

        // The pruned run's logs can't be opened: pressing does nothing.
        window.click("change-trail-hop-pr-run", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).selected(), Some("pr-run"));
        assert_eq!(
            window.find("change-trail-detail-title").label(),
            Some("PipelineRun checkout-pr-418-m2q8: Pruned")
        );
        window.click("change-trail-action-0", cx);
        assert!(story.read(cx).opened().is_none());

        // The pods prod-ams won't list open in Resources on prod-ams.
        window.click("change-trail-hop-prod-ams-pods", cx);
        window.render_frame(cx);
        window.click("change-trail-action-0", cx);
        assert_eq!(
            story.read(cx).opened().map(|text| text.as_ref()),
            Some("Would open the pods in Resources on prod-ams.")
        );
    })
    .unwrap();
}

/// A Stage's chevron folds and unfolds it, Show all unfolds every Stage,
/// and a status chip shows only its rows, in every group.
#[gpui_kit::test]
fn the_change_story_folds_stages_and_filters_by_status(cx: &mut TestAppContext) {
    use super::stories::change::ChangeStory;
    let (handle, workbench) = open(cx, super::stories::find("change-trail").unwrap());
    let story = cx.update(|cx| story::<ChangeStory>(&workbench, cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let folded = story.read(cx).shown().len();
        // prod-ams is group 4.
        window.click("change-trail-fold-4", cx);
        window.render_frame(cx);
        assert!(!story.read(cx).shown().contains(&"prod-ams-pods"));
        window.click("change-trail-fold-4", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).shown().len(), folded);

        window.click("change-trail-hops-show-all", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).shown().len(), 27);
        assert!(window.try_find("change-trail-hops-show-all").is_none());

        window.click("change-trail-tally-failing", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).shown(), vec!["prod-fra-verification"]);
        // A Freight approved by hand past stage has the blue dot, and
        // counts as ok.
        window.click("change-trail-tally-failing", cx);
        window.click("change-trail-tally-ok", cx);
        window.render_frame(cx);
        assert!(story.read(cx).shown().contains(&"prod-fra-eligible"));
        window.click("change-trail-tally-ok", cx);
        window.click("change-trail-tally-failing", cx);
        window.render_frame(cx);
        window.click("change-trail-tally-waiting", cx);
        window.render_frame(cx);
        assert_eq!(
            story.read(cx).shown(),
            vec![
                "pr-run",
                "prod-ams-promotion",
                "prod-ams-verification",
                "prod-ams-pods"
            ]
        );
        window.click("change-trail-tally-waiting", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).shown().len(), 27);
    })
    .unwrap();
}
