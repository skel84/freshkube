use super::*;

#[gpui_kit::test]
fn the_first_frame_redraws_once_the_list_knows_its_width(cx: &mut TestAppContext) {
    // Opening the window draws the first frame, with rows measured against
    // the window because the list's width is learned in that prepaint.
    let (_runtime, panel, handle) = mount(cx);
    let notified = std::rc::Rc::new(std::cell::Cell::new(0));
    let _observer = cx.update({
        let notified = notified.clone();
        |cx| cx.observe(&panel, move |_, _| notified.set(notified.get() + 1))
    });
    cx.run_until_parked();
    let before = notified.get();
    // A notify from prepaint schedules nothing; the next frame must ask
    // for the redraw itself, with no input event to prompt it.
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(window.simulate_next_frame(cx) > 0);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(notified.get() > before);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = panel.read(cx);
        assert_eq!(view.measured.as_ref().map(|key| key.width), view.width);
    })
    .unwrap();
}

#[gpui_kit::test]
fn live_resize_lays_out_rows_on_screen_and_settles_the_rest_afterwards(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    let measured = || freshkube_probe::probe::count("logs.measure");
    let tall = SharedString::from("log-line-1-90");
    let mut original = Vec::new();
    let mut before = 0;
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    // Rows off screen are measured once the first lines have held.
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        // Review from the top, so the tall row 90 is off screen.
        panel.update(cx, |view, cx| view.navigate(isize::MIN, false, cx));
        window.render_frame(cx);
        window.render_frame(cx);
        let view = panel.read(cx);
        assert!(!view.following);
        assert_eq!(view.scroll.offset().y, px(0.));
        assert!(window.try_find(tall.clone()).is_none());
        original = view.sizes.iter().map(|row| row.height).collect::<Vec<_>>();
        before = measured();
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(900.), px(760.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let view = panel.read(cx);
        let laid_out = measured() - before;
        assert!(
            laid_out > 0 && laid_out < original.len() / 2,
            "a resize frame laid out {laid_out} of {} rows",
            original.len()
        );
        assert!(!view.settled);
        // Off screen, the tall row keeps its last height until the
        // width holds.
        assert!(!view.row_exact[90]);
        assert_eq!(view.sizes[90].height, original[90]);
        assert_eq!(view.scroll.offset().y, px(0.));
        let mut shown = 0;
        for ix in 0..view.sizes.len() {
            let id = SharedString::from(format!("log-line-1-{}", view.review.id(ix)));
            if window.try_find(id).is_some_and(|row| row.visible()) {
                assert!(view.row_exact[ix], "row {ix} is on screen with an estimate");
                shown += 1;
            }
        }
        assert!(shown > 10);
        // Rows measured at the new width and rows still estimated add up
        // to the height the list is given.
        assert!(view.wrapped);
        assert_total_height(view);
    })
    .unwrap();
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, _, cx| {
        let view = panel.read(cx);
        assert!(view.settled);
        assert_total_height(view);
        assert!(view.sizes[90].height < original[90]);
        // Settling rows below the review position never moves it.
        assert_eq!(view.scroll.offset().y, px(0.));
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(620.), px(760.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, _, cx| {
        let heights: Vec<_> = panel.read(cx).sizes.iter().map(|row| row.height).collect();
        assert_eq!(
            heights, original,
            "settled heights differ from a fresh layout"
        );
        assert_total_height(panel.read(cx));
    })
    .unwrap();
}

/// The total height kept beside `sizes` is their heights added up.
fn assert_total_height<S: LogSource>(view: &LogView<S>) {
    let sum: f64 = (view.sizes.iter())
        .map(|row| f64::from(f32::from(row.height)))
        .sum();
    let kept = view.sizes_height;
    assert!(
        (kept - sum).abs() < 1e-6,
        "kept {kept}, rows add up to {sum}"
    );
}

#[gpui_kit::test]
fn a_flood_lays_out_the_rows_on_screen_and_the_rest_once_it_stops(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    let measured = || freshkube_probe::probe::count("logs.measure");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx, &panel, handle);
    let flood = |from: usize| {
        (from..from + 400)
            .map(|ix| format!("info flood line {ix}"))
            .collect()
    };
    let mut before = 0;
    cx.update_window(handle.into(), |_, window, cx| {
        before = measured();
        deliver(&panel, flood(0), cx);
        window.render_frame(cx);
        window.render_frame(cx);
        let view = panel.read(cx);
        assert!(view.following);
        let laid_out = measured() - before;
        assert!(
            laid_out > 0 && laid_out < 100,
            "a frame laid out {laid_out} of 400 new rows"
        );
        assert!(!view.settled);
        assert!(view.row_exact.contains(&false));
        let last = view.sizes.len() - 1;
        assert!(
            view.row_exact[last],
            "the newest row is on screen with an estimate"
        );
        assert!(
            window
                .find(SharedString::from(format!(
                    "log-line-1-{}",
                    view.review.id(last)
                )))
                .visible()
        );
    })
    .unwrap();
    // More lines before the stream has held keep rows off screen waiting.
    cx.executor().advance_clock(super::super::RESIZE_SETTLE / 2);
    cx.update_window(handle.into(), |_, window, cx| {
        deliver(&panel, flood(400), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.executor()
        .advance_clock(super::super::RESIZE_SETTLE * 3 / 4);
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, _, cx| assert!(!panel.read(cx).settled))
        .unwrap();
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        let view = panel.read(cx);
        assert!(view.settled);
        assert_eq!(view.sizes.len(), 920);
        assert_total_height(view);
        // Following keeps the newest row in view after the estimates settle.
        let last = view.review.id(view.sizes.len() - 1);
        assert!(
            window
                .find(SharedString::from(format!("log-line-1-{last}")))
                .visible()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn evicted_lines_drop_their_measurements(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(panel.read(cx).row_measurements.contains_key(&0));
        // 200 lines of 60 KiB pass the 8 MiB the view retains.
        for batch in 0..4 {
            let lines = (0..50)
                .map(|ix| format!("info {batch} {ix} {}", "x".repeat(60 * 1024)))
                .collect();
            deliver(&panel, lines, cx);
            window.render_frame(cx);
            assert_total_height(panel.read(cx));
        }
        let view = panel.read(cx);
        assert!(view.review.evicted > 120);
        let retained: std::collections::BTreeSet<u64> = view
            .review
            .logs
            .buffer()
            .entries()
            .iter()
            .map(|entry| entry.sequence())
            .collect();
        assert!(!view.row_measurements.is_empty());
        assert!(view.row_measurements.keys().all(|id| retained.contains(id)));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_short_panel_keeps_its_controls_whole_and_scrolls_to_the_list(cx: &mut TestAppContext) {
    // 240 px holds the toolbar, the banner and some list only when the
    // toolbar is cut; 760 px holds them all.
    for height in [240., 760.] {
        let (_runtime, panel, handle) = mount_sized(cx, 620., height);
        panel.update(cx, |view, cx| {
            view.source_mut().banner = true;
            // More errors than the notices' share of the panel holds.
            for ix in 0..12 {
                view.source_mut().errors.insert(
                    format!("service-{ix:02}").into(),
                    "connection refused".into(),
                );
            }
            cx.notify();
        });
        cx.update_window(handle.into(), |_, window, cx| {
            for _ in 0..3 {
                window.render_frame(cx);
            }
            let shown = window.find("logs-panel").bounds();
            let toolbar = window.find("logs-toolbar").bounds();
            let notices = window.find("logs-notices").bounds();
            let viewport = window.find("logs-viewport").bounds();
            for id in ["test-banner", "logs-search", "logs-follow"] {
                let control = window.find(id).bounds();
                assert!(
                    control.top() >= toolbar.top() && control.bottom() <= toolbar.bottom(),
                    "{height}: {id} at {control:?} is cut by the toolbar at {toolbar:?}"
                );
            }
            // The notices take at most a quarter of the panel and scroll
            // inside it.
            assert!(
                notices.size.height <= shown.size.height * 0.25 + px(0.5),
                "{height}: the notices {notices:?} take more than a quarter of {shown:?}"
            );
            // The list keeps its least height, 6 rem, at either height.
            let least = window.rem_size() * 6.;
            assert!(
                viewport.size.height >= least - px(0.5),
                "{height}: the list {viewport:?} is shorter than {least:?}"
            );
            let short = height < 300.;
            if !short {
                // A tall panel's list takes all the room left.
                assert!(
                    (viewport.bottom() - shown.bottom()).abs() <= px(0.5),
                    "{height}: the list {viewport:?} doesn't reach the panel's end {shown:?}"
                );
            }
            let scroll = panel.read(cx).panel_scroll.clone();
            assert_eq!(
                scroll.max_offset().y > px(0.),
                short,
                "{height}: the panel scrolls by {:?}",
                scroll.max_offset()
            );
            // At its end the panel shows the whole list.
            scroll.set_offset(point(px(0.), -scroll.max_offset().y));
            window.render_frame(cx);
            let viewport = window.find("logs-viewport").bounds();
            assert!(
                viewport.bottom() <= shown.bottom() + px(0.5),
                "{height}: the list {viewport:?} ends below the panel {shown:?}"
            );
        })
        .unwrap();
        // Another source's lines start at the panel's top again.
        panel.update(cx, |view, _| view.reset_lines("another"));
        assert_eq!(
            panel.read_with(cx, |view, _| view.panel_scroll.offset()),
            point(px(0.), px(0.))
        );
    }
}

#[gpui_kit::test]
fn wrap_and_rem_remeasure_real_rows_and_wheel_pauses_follow(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    let mut original = px(0.);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        original = panel.read(cx).sizes[90].height;
        assert!(original > panel.read(cx).sizes[89].height);
        let toolbar = window.find("logs-follow").bounds();
        let before = panel.read(cx).scroll.offset();
        let viewport = window.find("logs-viewport").bounds();
        // Use the toolkit's visible native thumb, not the model seam.
        window.drag(
            point(viewport.right() - px(8.), viewport.bottom() - px(24.)),
            point(viewport.right() - px(8.), viewport.top() + px(40.)),
            cx,
        );
        assert!(!panel.read(cx).following);
        assert_ne!(panel.read(cx).scroll.offset(), before);
        window.click("logs-follow", cx);
        assert!(panel.read(cx).following);
        window.scroll(
            "logs-viewport",
            ScrollDelta::Pixels(point(px(0.), px(900.))),
            cx,
        );
        assert!(!panel.read(cx).following);
        assert_ne!(panel.read(cx).scroll.offset(), before);
        assert_eq!(window.find("logs-follow").bounds(), toolbar);
        window.click("logs-wrap", cx);
        assert!(!panel.read(cx).wrapped);
        let unwrapped = panel.read(cx).sizes[90].height;
        assert!(unwrapped < original);
        assert_eq!(unwrapped, panel.read(cx).sizes[89].height);
        window.click("logs-wrap", cx);
        assert_eq!(panel.read(cx).sizes[90].height, original);
        Theme::update(cx, |theme| theme.font_size = px(20.));
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    // Off screen, the tall row is laid out again once the change holds.
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(panel.read(cx).sizes[90].height > original);
        window.click("logs-follow", cx);
        assert!(panel.read(cx).following);
        assert!(window.find(SharedString::from("log-line-1-119")).visible());
        assert_eq!(window.find("logs-wrap").checked(), Some(true));
        panel.update(cx, |view, cx| {
            let target = view.source.fixture_target.clone().unwrap();
            assert!(view.apply_batch(
                &target,
                view.source.stream_revision,
                vec![StreamEvent {
                    target: target.clone(),
                    service: ServiceId::from("apid"),
                    result: Ok(format!(
                        "info tall final row {}",
                        "Unicode 東京 latest text ".repeat(500)
                    )),
                }],
                cx
            ));
        });
        window.render_frame(cx);
        let viewport = window.find("logs-viewport").bounds();
        let last = window.find(SharedString::from("log-line-1-120")).bounds();
        assert!(last.size.height > viewport.size.height);
        assert!(last.bottom() <= viewport.bottom());
        assert!(last.bottom() >= viewport.bottom() - px(2.));
        let total: gpui_kit::Pixels = panel.read(cx).sizes.iter().map(|row| row.height).sum();
        assert_eq!(
            panel.read(cx).scroll.offset().y,
            viewport.size.height - px(2.) - total
        );
    })
    .unwrap();
}
