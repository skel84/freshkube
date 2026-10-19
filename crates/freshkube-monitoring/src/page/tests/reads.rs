use super::*;

/// Opens the stress run's 30 panels from a folder of its own, which the
/// caller removes.
fn open_thirty(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    page: &Entity<MonitoringPage>,
) -> std::path::PathBuf {
    let folder = std::env::temp_dir().join(format!(
        "freshkube-monitoring-thirty-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("thirty.json"),
        // The stress binary's dashboard (`mon-30`); desktop owns the file.
        include_str!("../../../../freshkube-desktop/src/bin/stress/dashboards/thirty.json"),
    )
    .unwrap();
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.saved.folder = Some(folder.clone());
            page.read_folder(cx);
        })
    });
    cx.run_until_parked();
    let id = cx.read(|cx| match &page.read(cx).catalog.folder {
        FolderState::Read { entries, .. } => entries[0].id.clone(),
        _ => panic!("the folder wasn't read"),
    });
    show(cx, handle, page);
    cx.update(|cx| page.update(cx, |page, cx| page.open(id, cx)));
    cx.run_until_parked();
    // The header folds from what it measured on the frame before; a page
    // that never stops asking for frames is the caller's to find.
    cx.update_window(handle, |_, window, cx| {
        for _ in 0..4 {
            window.render_frame(cx);
            window.simulate_next_frame(cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(slot_count(cx, page), 30);
    folder
}

#[gpui_kit::test]
fn a_hidden_page_reads_nothing_and_showing_answers_the_panels_in_view(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    cx.read(|cx| {
        let page = page.read(cx);
        assert!(page.board.is_none(), "a hidden page opened a dashboard");
        assert!(matches!(page.connection, Connection::None));
    });

    show(cx, handle, &page);
    assert!(shown(cx, handle, "monitoring-title"));
    assert!(shown(cx, handle, "monitoring-variable-node"));
    assert!(shown(cx, handle, "monitoring-variable-namespace"));
    let answered = ready(cx, &page);
    assert!(answered.contains(&"Ready nodes".to_owned()), "{answered:?}");
    assert!(
        answered.contains(&"CPU usage by node".to_owned()),
        "{answered:?}"
    );
    assert_eq!(answered.len(), slot_count(cx, &page));
}

#[gpui_kit::test]
fn a_variable_change_reads_the_variables_and_panels_again(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let (index, before, choice) = cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        let control = board
            .controls
            .iter()
            .find(|control| control.id.as_ref() == "monitoring-variable-node")
            .unwrap();
        (
            control.index,
            control.value.clone(),
            control.options[1].clone(),
        )
    });
    assert_eq!(before.as_ref(), "All");
    let generation = cx.read(|cx| page.read(cx).generation);
    cx.update(|cx| page.update(cx, |page, cx| page.set_variable(index, choice.clone(), cx)));
    cx.run_until_parked();
    frame(cx, handle);
    cx.read(|cx| {
        let page = page.read(cx);
        assert!(page.generation > generation);
        let board = page.board.as_ref().unwrap();
        let control = board.controls.iter().find(|c| c.index == index).unwrap();
        assert_eq!(control.value, choice);
        assert_eq!(control.options[0].as_ref(), "All");
        assert_eq!(
            control
                .options
                .iter()
                .filter(|o| o.as_ref() == "All")
                .count(),
            1
        );
    });
    assert_eq!(ready(cx, &page).len(), slot_count(cx, &page));
}

#[gpui_kit::test]
fn a_time_range_change_asks_every_panel_over_the_new_window(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let (generation, span) = cx.read(|cx| {
        let page = page.read(cx);
        (page.generation, page.board.as_ref().unwrap().span)
    });
    assert_eq!(span, 6 * 3600);
    cx.update(|cx| page.update(cx, |page, cx| page.set_range(3600, cx)));
    cx.run_until_parked();
    frame(cx, handle);
    cx.read(|cx| {
        let page = page.read(cx);
        let board = page.board.as_ref().unwrap();
        assert_eq!(page.generation, generation + 1);
        assert_eq!(board.range_label.as_ref(), "Last 1 hour");
        assert_eq!(board.window.unwrap().span, 3600);
        assert!(
            board
                .slots
                .iter()
                .filter(|slot| slot.view.read(cx).is_ready())
                .all(|slot| slot.asked == Some(page.generation))
        );
    });
}

#[gpui_kit::test]
fn the_cursor_on_one_chart_shows_on_the_others_and_redraws_no_panel(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let (plot, others) = cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        let cpu = board
            .slots
            .iter()
            .position(|slot| slot.spec.title == "CPU usage by node")
            .unwrap();
        let others: Vec<SharedString> = board
            .slots
            .iter()
            .enumerate()
            .filter(|(index, slot)| *index != cpu && slot.view.read(cx).is_chart())
            .map(|(_, slot)| slot.view.read(cx).part("crosshair"))
            .collect();
        (board.slots[cpu].view.read(cx).part("plot"), others)
    });
    assert!(others.len() >= 2, "{others:?}");
    // Real moves and real frames, with the view caches on.
    cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    let move_to = |cx: &mut TestAppContext, at: fn(Bounds<Pixels>) -> Point<Pixels>| {
        let plot = plot.clone();
        cx.update_window(handle, |_, window, cx| {
            let bounds = window.find(plot).bounds();
            window.dispatch_event(
                gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                    position: at(bounds),
                    pressed_button: None,
                    modifiers: Default::default(),
                }),
                cx,
            );
            window.draw(cx).clear(cx);
        })
        .unwrap();
    };
    move_to(cx, |plot| plot.center());
    let counts = || {
        [
            probe::count("monitoring-panel"),
            probe::count("monitoring-page"),
            probe::count("monitoring-derive"),
        ]
    };
    let [panels, pages, derived] = counts();
    move_to(cx, |plot| plot.center() + point(px(40.), px(0.)));
    move_to(cx, |plot| plot.center() + point(px(80.), px(0.)));
    // No panel draws again, not even the one under the pointer: its cursor
    // is a view of its own. The page draws with the shell's frames to place
    // the crosshairs, from what it derived before: a move derives nothing.
    let [panels_now, pages_now, derived_now] = counts();
    assert_eq!(panels_now, panels, "a move redrew a panel");
    assert!(pages_now > pages);
    assert_eq!(derived_now, derived, "a move derived");
    for id in &others {
        assert!(shown(cx, handle, id), "no crosshair on {id}");
    }

    // Leaving the plot, for its card's title, takes them away.
    move_to(cx, |plot| point(plot.center().x, plot.top() - px(16.)));
    for id in &others {
        assert!(!shown(cx, handle, id), "{id} kept its crosshair");
    }
}

#[gpui_kit::test]
fn only_panels_in_reach_draw_and_one_scrolled_in_takes_its_card_place(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    let folder = open_thirty(cx, handle, &page);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Each slot is its panel or an empty card, never both or neither.
        let drawn: Vec<bool> = (0..30)
            .map(|n| {
                let panel = window
                    .try_find(SharedString::from(format!("monitoring-panel-{n}")))
                    .is_some();
                let card = window
                    .try_find(SharedString::from(format!("monitoring-placeholder-{n}")))
                    .is_some();
                assert_ne!(panel, card, "slot {n}");
                panel
            })
            .collect();
        assert!(drawn[0], "the first panel isn't drawn");
        let far = drawn
            .iter()
            .position(|drawn| !drawn)
            .expect("every panel is drawn");
        let grid = window.find("monitoring-grid").bounds();
        let card = window
            .find(format!("monitoring-placeholder-{far}"))
            .bounds();
        let first = slot_bounds(window, 0);
        // Scrolled until the card's top meets the grid's: drawn in the same
        // frame, at the card's place and size, with nothing else moved.
        let by = card.top() - grid.top();
        wheel(window, ScrollDelta::Pixels(point(px(0.), -by)), cx);
        let panel = window.find(format!("monitoring-panel-{far}")).bounds();
        assert_eq!(panel.size, card.size);
        assert!(
            (panel.top() - grid.top()).abs() < px(0.5),
            "{panel:?} after scrolling {by:?} to {grid:?}"
        );
        assert!(
            (slot_bounds(window, 0).top() - (first.top() - by)).abs() < px(0.5),
            "the first slot moved from {first:?} to {:?}",
            slot_bounds(window, 0)
        );
        assert!(
            window.try_find("monitoring-panel-0").is_none(),
            "the first panel is still drawn far above the view"
        );
    })
    .unwrap();
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn with_the_panels_in_reach_answered_the_page_asks_for_no_frame(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    // A loading panel pulses only with motion on.
    cx.update(|cx| cx.set_reduce_motion(false));
    let folder = open_thirty(cx, handle, &page);
    let generation = cx.read(|cx| page.read(cx).generation);
    let ready = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let board = page.read(cx).board.as_ref().unwrap();
            // What is drawn is what was asked, and the rest is still loading.
            for (n, slot) in board.slots.iter().enumerate() {
                let drawn = window
                    .try_find(SharedString::from(format!("monitoring-panel-{n}")))
                    .is_some();
                assert_eq!(drawn, slot.asked == Some(generation), "slot {n}");
                assert_eq!(drawn, slot.view.read(cx).is_ready(), "slot {n}");
            }
            board
                .slots
                .iter()
                .filter(|slot| slot.asked.is_some())
                .count()
        })
        .unwrap();
    assert!((1..30).contains(&ready), "{ready} of 30 answered");
    cx.update_window(handle, |_, window, cx| {
        // The header folds from what it measured on the frame before.
        for _ in 0..4 {
            window.draw(cx).clear(cx);
            window.simulate_next_frame(cx);
        }
        let panels = probe::count("monitoring-panel");
        for _ in 0..3 {
            window.draw(cx).clear(cx);
            assert_eq!(window.simulate_next_frame(cx), 0, "a frame is asked for");
        }
        assert_eq!(probe::count("monitoring-panel"), panels);
    })
    .unwrap();
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn hiding_mid_request_drops_the_reads_and_showing_asks_again(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    // Show and hide before any read can answer.
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_visible(true, cx);
            assert!(page.board.as_ref().unwrap().resolving.is_some());
            page.set_visible(false, cx);
        })
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let page = page.read(cx);
        let board = page.board.as_ref().unwrap();
        assert!(board.resolving.is_none());
        assert!(board.variables.is_none(), "an answer arrived after hiding");
        assert!(page.refresh_task.is_none());
    });
    assert!(ready(cx, &page).is_empty());

    // Panels asked, then hidden: their reads go and nothing lands.
    show(cx, handle, &page);
    let before = cx.read(|cx| page.read(cx).generation);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.ask_again(cx);
            page.set_visible(false, cx);
            let board = page.board.as_ref().unwrap();
            assert!(board.slots.iter().all(|slot| slot.request.is_none()));
            assert!(board.slots.iter().all(|slot| slot.asked.is_none()));
        })
    });
    cx.run_until_parked();
    assert!(cx.read(|cx| page.read(cx).generation) > before);

    show(cx, handle, &page);
    assert_eq!(ready(cx, &page).len(), slot_count(cx, &page));
    cx.read(|cx| {
        let page = page.read(cx);
        let generation = page.generation;
        assert!(
            page.board
                .as_ref()
                .unwrap()
                .slots
                .iter()
                .all(|slot| slot.asked == Some(generation))
        );
    });
}

#[gpui_kit::test]
fn a_folded_row_shows_its_count_and_unfolding_asks_its_panels(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.open(EntryId::Builtin("freshkube-workloads"), cx)
        })
    });
    cx.run_until_parked();
    frame(cx, handle);
    let network: Vec<String> = vec!["Received by pod".into(), "Sent by pod".into()];
    let answered = ready(cx, &page);
    assert!(answered.contains(&"CPU by pod".to_owned()), "{answered:?}");
    assert!(network.iter().all(|title| !answered.contains(title)));
    let row = cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        let (section, header) = board
            .rows
            .iter()
            .enumerate()
            .find_map(|(index, row)| {
                row.as_ref()
                    .filter(|row| row.title.as_ref() == "Network")
                    .map(|row| (index, row))
            })
            .unwrap();
        assert!(board.is_collapsed(section));
        assert_eq!(header.count.as_ref(), "2 panels");
        header.id.clone()
    });
    cx.update_window(handle, |_, window, cx| window.click(row, cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, handle);
    let answered = ready(cx, &page);
    assert!(
        network.iter().all(|title| answered.contains(title)),
        "{answered:?}"
    );
}
