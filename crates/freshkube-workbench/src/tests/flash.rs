use super::*;

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
