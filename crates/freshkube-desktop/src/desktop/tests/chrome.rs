use super::*;

/// The cached chrome still draws for what it shows while the motion runs
/// beside it: a notify of the shell, the theme and the text size.
#[gpui_kit::test]
fn the_cached_chrome_redraws_for_what_it_shows(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = loading_pods(cx);
    let every = CHROME.to_vec();
    let shell = view.clone();
    assert_eq!(
        chrome_redrawn(cx, handle, |_, cx| shell.update(cx, |_, cx| cx.notify())),
        every
    );
    assert_eq!(
        chrome_redrawn(cx, handle, |window, cx| Theme::change(
            ThemeMode::Dark,
            Some(window),
            cx
        )),
        every
    );
    assert_eq!(
        chrome_redrawn(cx, handle, |_, cx| crate::text_size::set(20., cx)),
        every
    );
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.rem_size(), px(20.));
        assert!(window.find("refresh").visible());
        assert!(window.find("nav-rail").visible());
        assert!(window.find("nav-column").visible());
    })
    .unwrap();
}

/// The parts draw with the shell's own controls: a rail button and a
/// header control act on the shell through them.
#[gpui_kit::test]
fn the_cached_chrome_acts_on_the_shell(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-control-plane", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(view.read(cx).area, Area::ControlPlane));
    let dark = cx.update(|cx| cx.theme().mode.is_dark());
    cx.update_window(handle, |_, window, cx| window.click("theme-toggle", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_ne!(cx.theme().mode.is_dark(), dark));
}

/// Talos holds: Nodes waits on its loading rows while the header's
/// Refresh shows the read in flight, and nothing in the chrome repeats.
#[gpui_kit::test]
fn nothing_in_the_chrome_moves_while_talos_loads(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture().holding_talos(), 1280., 880.);
    cx.update(|cx| cx.set_reduce_motion(false));
    cx.update_window(handle, |_, window, cx| window.press("secondary-2", cx))
        .unwrap();
    cx.run_until_parked();
    for _ in 0..3 {
        motion_frame(cx, handle);
    }
    cx.update(|cx| {
        assert!(view.read(cx).loading());
        assert!(view.read(cx).nodes_wait());
    });
    let motion = probe::count("table.loading-motion");
    assert!(frames_redrawing(cx, handle, "nodes") <= 1);
    assert!(probe::count("table.loading-motion") >= motion + 60);
}

#[gpui_kit::test]
fn hidden_log_batches_do_not_redraw_the_window(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let shell_notices = Rc::new(Cell::new(0usize));
    let log_notices = Rc::new(Cell::new(0usize));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let logs = view.read(cx).logs.clone();
        let events = (0..20)
            .map(|ix| {
                freshkube_core::logs::LogEvent::new("apid", format!("level=info seed line {ix}"))
            })
            .collect();
        logs.update(cx, |logs, cx| logs.set_fixture(events, window, cx));
        window.render_frame(cx);
        draw(window, cx);
        let (counter, seen) = (shell_notices.clone(), log_notices.clone());
        cx.observe(&view, move |_, _| counter.set(counter.get() + 1))
            .detach();
        cx.observe(&logs, move |_, _| seen.set(seen.get() + 1))
            .detach();
        let (overview, panel) = (count("overview"), count("logs"));
        let (applied, _) = logs.read(cx).applied_and_held();
        for batch in 0..5 {
            let lines = (0..10).map(|ix| format!("level=info hidden {batch}-{ix}"));
            assert!(logs.update(cx, |logs, cx| logs.push_fixture_batch(lines.collect(), cx)));
            draw(window, cx);
        }
        // Held back, not applied one by one, and nothing asked for a redraw.
        let (now, held) = logs.read(cx).applied_and_held();
        assert_eq!(now, applied);
        assert_eq!(held, 50);
        // Past the coalescing interval the whole backlog lands at once.
        cx.background_executor()
            .advance_clock(std::time::Duration::from_millis(300));
        assert!(logs.update(cx, |logs, cx| {
            logs.push_fixture_batch(vec!["level=info late".into()], cx)
        }));
        draw(window, cx);
        let (now, held) = logs.read(cx).applied_and_held();
        assert_eq!((now, held), (applied + 51, 0));
        assert_eq!(log_notices.get(), 0, "hidden batches must not notify");
        assert_eq!(shell_notices.get(), 0, "the shell must not hear of them");
        assert_eq!(count("overview"), overview);
        assert_eq!(count("logs"), panel);
    })
    .unwrap();
    // Showing Logs applies anything still held and draws it.
    cx.update_window(handle, |_, window, cx| {
        let logs = view.read(cx).logs.clone();
        assert!(logs.update(cx, |logs, cx| {
            logs.push_fixture_batch(vec!["level=info held".into()], cx)
        }));
        open_node_tab(window, cx, super::super::nodes::NodeTab::Logs);
        draw(window, cx);
        assert_eq!(logs.read(cx).applied_and_held().1, 0);
        assert!(count("logs") > 0);
        let visible = count("logs");
        assert!(logs.update(cx, |logs, cx| {
            logs.push_fixture_batch(vec!["level=info shown".into()], cx)
        }));
        draw(window, cx);
        assert!(count("logs") > visible, "a visible panel redraws");
    })
    .unwrap();
}

#[gpui_kit::test]
fn tick_does_not_redraw_the_active_screen(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_node_tab(window, cx, super::super::nodes::NodeTab::Processes);
        window.render_frame(cx);
        draw(window, cx);
        draw(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let (screen, ring) = cx.update(|cx| {
        (
            count("processes"),
            view.read(cx).countdown.read(cx).remaining,
        )
    });
    assert!(screen > 0);
    for _ in 0..3 {
        tick(&view, cx);
        cx.update_window(handle, |_, window, cx| draw(window, cx))
            .unwrap();
    }
    cx.update(|cx| {
        assert_ne!(
            view.read(cx).countdown.read(cx).remaining,
            ring,
            "ring moves"
        );
    });
    assert_eq!(count("processes"), screen, "the screen stays as drawn");
}

#[gpui_kit::test]
fn tick_does_not_redraw_overview(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        draw(window, cx);
        draw(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let overview = count("overview");
    assert!(overview > 0);
    tick(&view, cx);
    cx.update_window(handle, |_, window, cx| draw(window, cx))
        .unwrap();
    assert_eq!(count("overview"), overview);
}

#[gpui_kit::test]
fn cached_screen_redraws_after_data_and_theme_change(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        open_node_tab(window, cx, super::super::nodes::NodeTab::Processes);
        window.render_frame(cx);
        draw(window, cx);
        draw(window, cx);
        let drawn = count("processes");
        draw(window, cx);
        assert_eq!(count("processes"), drawn, "nothing changed, nothing drawn");
        // New data.
        let screen = view.read(cx).active_screen().unwrap();
        screen.refresh(window, cx);
        draw(window, cx);
        let after_data = count("processes");
        assert!(after_data > drawn, "data change must redraw the screen");
        // Theme.
        let light = cx.theme().background;
        Theme::change(ThemeMode::Dark, Some(window), cx);
        draw(window, cx);
        assert_ne!(cx.theme().background, light);
        let after_theme = count("processes");
        assert!(after_theme > after_data, "theme change must redraw it");
    })
    .unwrap();
    // Font size: observers run when the update ends, before the next frame.
    let before = count("processes");
    cx.update(|cx| Theme::update(cx, |theme| theme.font_size = px(20.)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| draw(window, cx))
        .unwrap();
    assert!(count("processes") > before, "font size must redraw it");
}

#[gpui_kit::test]
fn cached_overview_redraws_after_data_and_theme_change(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        draw(window, cx);
        draw(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let drawn = count("overview");
    cx.update(|cx| view.update(cx, |pilot, cx| pilot.simulate_failure(cx)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| draw(window, cx))
        .unwrap();
    let after_data = count("overview");
    assert!(after_data > drawn, "data change must redraw the page");
    cx.update_window(handle, |_, window, cx| {
        Theme::change(ThemeMode::Dark, Some(window), cx);
        draw(window, cx);
    })
    .unwrap();
    assert!(
        count("overview") > after_data,
        "theme change must redraw it"
    );
}

/// One countdown tick as the shell's timer applies it.
fn tick(view: &Entity<Pilot>, cx: &mut TestAppContext) {
    cx.update(|cx| {
        view.update(cx, |pilot, cx| {
            pilot.elapsed += std::time::Duration::from_secs(1);
            pilot.tick_countdown(cx);
        });
    });
    cx.run_until_parked();
}
