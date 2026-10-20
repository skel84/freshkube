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
    cx.update(|cx| assert_eq!(view.read(cx).navigation.area, Area::ControlPlane));
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
    let (_runtime, handle, view) = mount(
        cx,
        GpuiOptions::fixture().with_debug(crate::DebugOptions::parse(|name| {
            (name == "FRESHKUBE_FIXTURE_HOLD").then(|| "talos".into())
        })),
        1280.,
        880.,
    );
    cx.update(|cx| cx.set_reduce_motion(false));
    cx.update_window(handle, |_, window, cx| window.press("secondary-2", cx))
        .unwrap();
    cx.run_until_parked();
    for _ in 0..3 {
        motion_frame(cx, handle);
    }
    cx.update(|cx| {
        assert!(view.read(cx).loading());
        assert!(view.read(cx).nodes_wait(cx));
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
        let logs = view.read(cx).pages.logs.clone();
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
        let logs = view.read(cx).pages.logs.clone();
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
            view.read(cx).chrome.countdown.read(cx).remaining,
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
            view.read(cx).chrome.countdown.read(cx).remaining,
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
        let screen = view.read(cx).active_screen(cx).unwrap();
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
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        draw(window, cx);
        draw(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let drawn = count("overview");
    cx.update_window(handle, |_, window, cx| window.click("fixture-fail", cx))
        .unwrap();
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

/// Overview is a view of its own: a notify of the shell that changes none
/// of what it shows leaves it as drawn, while its cards, its attention
/// list, a change of state, Show all, the theme and the text size redraw it
/// once each.
#[gpui_kit::test]
fn overview_redraws_for_its_own_changes(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        draw(window, cx);
        draw(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let mut redrawn = |change: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        let before = count("overview");
        cx.update_window(handle, |_, window, cx| change(window, cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| draw(window, cx))
            .unwrap();
        count("overview") - before
    };
    let shell = redrawn(&|_, cx| view.update(cx, |_, cx| cx.notify()));
    let cards = redrawn(&|_, cx| view.update(cx, |view, cx| view.rebuild_joined_nodes(cx)));
    // A refresh that fails through the shell: the observer hands Overview
    // the stale state, with the joined nodes rebuilt in the same frame.
    // Click helpers force render_frame, which bypasses caches. Queue the
    // source's answer and refresh normally to count only scheduled redraws.
    let stale = redrawn(&|window, cx| {
        view.update(cx, |view, cx| {
            view.mode.example_source_mut().unwrap().fail_next_overview();
            view.refresh(window, cx);
        })
    });
    let after_stale = redrawn(&|_, cx| view.update(cx, |_, cx| cx.notify()));
    let attention = redrawn(&|_, cx| {
        view.update(cx, |view, cx| {
            let attention = view.pages.node_workspace.read(cx).attention.clone();
            view.pages
                .overview_page
                .update(cx, |page, cx| page.set_attention(attention, cx));
        })
    });
    let state = redrawn(&|_, cx| {
        view.update(cx, |view, cx| {
            view.config_error = Some("unreadable".into());
            cx.notify();
        })
    });
    let unchanged = redrawn(&|_, cx| view.update(cx, |_, cx| cx.notify()));
    let show_all = redrawn(&|_, cx| view.update(cx, |view, cx| view.expand_attention(cx)));
    let cached = redrawn(&|_, cx| view.update(cx, |view, cx| view.pages.notify_cached(cx)));
    let text_size = redrawn(&|_, cx| Theme::update(cx, |theme| theme.font_size = px(20.)));
    assert_eq!(
        [
            shell,
            cards,
            stale,
            after_stale,
            attention,
            state,
            unchanged,
            show_all,
            cached,
            text_size
        ],
        [0, 1, 1, 0, 1, 1, 0, 1, 1, 1]
    );
    let before = count("overview");
    cx.update_window(handle, |_, window, cx| {
        Theme::change(ThemeMode::Dark, Some(window), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| draw(window, cx))
        .unwrap();
    assert_eq!(count("overview") - before, 1, "a theme change redraws it");
}

/// The pointer moving onto a card and off it again redraws Overview once
/// each way for the card's hover tint. The shell draws again as its
/// ancestor, as it did around the host; the cached header stays as drawn.
#[gpui_kit::test]
fn overview_card_hover_redraws_only_overview(cx: &mut TestAppContext) {
    use gpui_kit::{InputEvent, MouseMoveEvent};
    let (_runtime, handle, _view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        draw(window, cx);
        draw(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let mut moved = |onto_card: bool| {
        let (overview, header) = (count("overview"), count("chrome.header"));
        cx.update_window(handle, |_, window, cx| {
            let card = window.find("tile-nodes").bounds();
            let position = if onto_card {
                card.center()
            } else {
                card.origin - gpui_kit::point(gpui_kit::px(4.), gpui_kit::px(4.))
            };
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| draw(window, cx))
            .unwrap();
        (
            count("overview") - overview,
            count("chrome.header") - header,
        )
    };
    let onto = moved(true);
    let off = moved(false);
    assert_eq!([onto, off], [(1, 0), (1, 0)]);
}

/// System services is a cached view of its own: a notify of the shell
/// leaves it as drawn, while its nodes, a changed Talos reading and the
/// global redraw each draw it again.
#[gpui_kit::test]
fn cached_system_services_redraws_for_its_own_changes(cx: &mut TestAppContext) {
    use crate::screens::Reading;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.navigate(Page::SystemServices, window, cx)
        });
        window.render_frame(cx);
        draw(window, cx);
        draw(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let mut redrawn = |change: &dyn Fn(&mut Pilot, &mut gpui_kit::Context<Pilot>)| {
        let before = count("system-services");
        cx.update(|cx| view.update(cx, |view, cx| change(view, cx)));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| draw(window, cx))
            .unwrap();
        count("system-services") - before
    };
    let shell = redrawn(&|_, cx| cx.notify());
    let nodes = redrawn(&|view, cx| {
        let nodes = view.target.nodes.clone();
        view.pages
            .system_services
            .update(cx, |page, cx| page.set_nodes(&nodes, cx));
    });
    let reading = redrawn(&|view, cx| {
        let reading = Reading::Failed("no answer".into());
        view.pages
            .system_services
            .update(cx, |page, cx| page.set_reading(reading, cx));
    });
    let cached = redrawn(&|view, cx| view.pages.notify_cached(cx));
    assert_eq!([shell, nodes, reading, cached], [0, 1, 1, 1]);
}

/// One countdown tick as the shell's timer applies it.
fn tick(view: &Entity<Pilot>, cx: &mut TestAppContext) {
    cx.update(|cx| {
        view.update(cx, |pilot, cx| {
            pilot.chrome.elapsed += std::time::Duration::from_secs(1);
            pilot.tick_countdown(cx);
        });
    });
    cx.run_until_parked();
}

/// The node pane's System services tab draws inside Nodes, a view of its
/// own, so a notify of the shell leaves it as drawn; its own changes draw
/// it once. A restart's notice doesn't notify the shell, so the tab draws
/// it only because it hears the notice itself. A theme change redraws the
/// cached tab.
#[gpui_kit::test]
fn node_services_redraws_inside_nodes(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1700., 1200.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.open_node_by_name(
                "talos-wk-fra1-02",
                super::super::nodes::NodeTab::Services,
                window,
                cx,
            )
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        draw(window, cx);
        draw(window, cx);
    })
    .unwrap();
    let mut redrawn = |change: &dyn Fn(&mut Pilot, &mut gpui_kit::Context<Pilot>)| {
        let before = count("node-services");
        cx.update(|cx| view.update(cx, |view, cx| change(view, cx)));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| draw(window, cx))
            .unwrap();
        count("node-services") - before
    };
    let shell = redrawn(&|_, cx| cx.notify());
    let selection = redrawn(&|view, cx| view.pages.select_service(Some("kubelet".into()), cx));
    let services = redrawn(&|view, cx| view.push_services(cx));
    let notice = redrawn(&|view, cx| view.note_restart("kubelet", cx));
    let cached = redrawn(&|view, cx| view.pages.notify_cached(cx));
    assert_eq!(
        [shell, selection, services, notice, cached],
        [0, 1, 1, 1, 1]
    );
}
