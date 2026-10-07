use super::stories::data_table::{DataTableStory, Sort, State};
use super::stories::dock::{self as dock_story, DockStory};
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
    // The change itself drew the table once; the fade's frames don't.
    let table = probe::count("workbench.change-flash-table");
    let layer = probe::count("table.flash-layer");
    let root = probe::count("workbench.change-flash");
    for _ in 0..5 {
        assert!(next_frame(handle, cx) > 0, "the fade asks for frames");
    }
    assert_eq!(probe::count("workbench.change-flash-table"), table);
    assert_eq!(probe::count("table.flash-layer"), layer + 5);
    // The layer's ancestors draw with it: the story's root, and the window's.
    assert_eq!(probe::count("workbench.change-flash"), root + 5);
    let left = strength(&story, cx).unwrap();
    let expected = motion::fade_left(FRAME * 5);
    assert!((left - expected).abs() < 1e-3, "{left} after five frames");
    // Once the fade ends, the layer stops asking.
    cx.background_executor.advance_clock(motion::FADE);
    cx.run_until_parked();
    next_frame(handle, cx);
    assert_eq!(next_frame(handle, cx), 0);
    assert_eq!(strength(&story, cx), None);
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
    let table = probe::count("workbench.change-flash-table");
    for _ in 0..3 {
        assert!(next_frame(handle, cx) > 0);
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
        (left - motion::fade_left(motion::FADE / 2)).abs() < 1e-3,
        "{left}"
    );
    assert!(next_frame(handle, cx) > 0, "and fades on from there");
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
        (left - motion::fade_left(motion::FADE / 2)).abs() < 1e-3,
        "{left}"
    );
    assert!(next_frame(handle, cx) > 0, "and fades on from there");
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
