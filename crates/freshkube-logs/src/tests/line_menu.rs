use super::*;

/// Copy is enabled by any selection, and its text is built only on click:
/// a selection the filters hide, or one over 1 MiB, says why in the note
/// and leaves the clipboard as it was (#318).
#[gpui_kit::test]
fn copy_explains_a_selection_it_cant_copy_and_leaves_the_clipboard(cx: &mut TestAppContext) {
    use freshkube_core::types::LogLevel;
    let (_runtime, panel, handle) = mount(cx);
    let clicked = |cx: &mut TestAppContext| {
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("unchanged".into()));
            window.click("logs-copy", cx);
            window.render_frame(cx);
            assert!(window.find("logs-summary-text").visible());
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref(),
                Some("unchanged")
            );
            panel.read(cx).feedback.clone().unwrap()
        })
        .unwrap()
    };

    // A line selected, then hidden by the level filter.
    panel.update(cx, |view, cx| {
        view.review.select(0, false, false);
        view.review.set_level(&LogLevel::Info, false);
        cx.notify();
    });
    let feedback = clicked(cx);
    assert_eq!(feedback.text.as_ref(), "Select visible lines to copy");

    // Lines that add up to more than 1 MiB.
    panel.update(cx, |view, cx| {
        view.review.set_level(&LogLevel::Info, true);
        view.review.selected.clear();
        cx.notify();
    });
    cx.update(|cx| deliver(&panel, vec!["x".repeat(8 * 1024); 160], cx));
    panel.update(cx, |view, cx| {
        let last = view.review.visible.len() - 1;
        view.review.select(last - 159, false, false);
        view.review.select(last, true, false);
        assert_eq!(view.review.selected.len(), 160);
        cx.notify();
    });
    let feedback = clicked(cx);
    assert!(
        feedback.text.contains("exceeds 1 MiB"),
        "{:?}",
        feedback.text
    );
}

fn menu_fixture(cx: &mut TestAppContext, panel: &Entity<LogPanel>, handle: WindowHandle<Root>) {
    cx.update(|cx| cx.set_reduce_motion(true));
    cx.update_window(handle.into(), |_, window, cx| {
        panel.update(cx, |view, cx| {
            view.set_fixture(
                (0..6)
                    .map(|ix| LogEvent::new("apid", format!("info line {ix}")))
                    .collect(),
                window,
                cx,
            );
        });
    })
    .unwrap();
    settle(cx, panel, handle);
}

fn open_line_menu(
    cx: &mut TestAppContext,
    panel: &Entity<LogPanel>,
    handle: WindowHandle<Root>,
    id: u64,
) {
    let generation = cx.read(|cx| panel.read(cx).generation);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.right_click(format!("log-line-{generation}-{id}"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("popup-menu").visible());
    })
    .unwrap();
}

fn pick_line_menu(cx: &mut TestAppContext, handle: WindowHandle<Root>, ix: usize) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.within("popup-menu").click(ix, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn check_menu_selection(cx: &mut TestAppContext, disjoint: bool) {
    let (_runtime, panel, handle) = mount(cx);
    menu_fixture(cx, &panel, handle);
    panel.update(cx, |view, cx| {
        view.review.select(1, false, false);
        view.review.select(3, !disjoint, disjoint);
        cx.notify();
    });
    let (selected, anchor) = cx.read(|cx| {
        let view = panel.read(cx);
        (view.review.selected.clone(), view.review.selection_anchor)
    });
    open_line_menu(cx, &panel, handle, 1);
    cx.read(|cx| {
        let view = panel.read(cx);
        assert_eq!(view.review.selected, selected);
        assert_eq!(view.review.selection_anchor, anchor);
        assert!(!view.following);
    });
    pick_line_menu(cx, handle, 0);
    let copied = cx.read(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert_eq!(
        copied,
        if disjoint {
            "info line 1\ninfo line 3"
        } else {
            "info line 1\ninfo line 2\ninfo line 3"
        }
    );
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(panel.read(cx).focus.is_focused(window));
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("unchanged".into()));
        window.press("secondary-c", cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), copied);
    })
    .unwrap();
    open_line_menu(cx, &panel, handle, 5);
    cx.read(|cx| {
        let view = panel.read(cx);
        assert_eq!(view.review.selected, BTreeSet::from([5]));
        assert_eq!(view.review.selection_anchor, Some(5));
    });
    cx.update_window(handle.into(), |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
        assert!(panel.read(cx).focus.is_focused(window));
        assert_eq!(panel.read(cx).review.selected, BTreeSet::from([5]));
        window.press("secondary-c", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "info line 5"
        );
        window.press("escape", cx);
        assert!(panel.read(cx).review.selected.is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_line_menu_keeps_a_contiguous_selection_and_replaces_it_outside(cx: &mut TestAppContext) {
    check_menu_selection(cx, false);
}

#[gpui_kit::test]
fn a_line_menu_keeps_a_disjoint_selection_and_replaces_it_outside(cx: &mut TestAppContext) {
    check_menu_selection(cx, true);
}

#[gpui_kit::test]
fn a_line_menu_shows_the_existing_platform_keys(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    menu_fixture(cx, &panel, handle);
    open_line_menu(cx, &panel, handle, 1);
    cx.update_window(handle.into(), |_, window, _| {
        let menu = window.within("popup-menu");
        assert_eq!(menu.find(0usize).label(), Some("Copy"));
        assert_eq!(menu.find(1usize).label(), Some("Select all"));
        assert_eq!(menu.find(2usize).label(), None);
        assert_eq!(menu.find(3usize).label(), Some("Find"));
    })
    .unwrap();
    let hints = match freshkube_ui::platform::Platform::current() {
        freshkube_ui::platform::Platform::MacOs => ["kbd:cmd-c", "kbd:cmd-a", "kbd:cmd-f"],
        _ => ["kbd:ctrl-c", "kbd:ctrl-a", "kbd:ctrl-f"],
    };
    for (ix, hint) in [0usize, 1, 3].into_iter().zip(hints) {
        let hint = gpui_kit::VisualTestContext::from_window(handle.into(), cx)
            .debug_bounds(hint)
            .expect("the menu draws its existing shortcut");
        cx.update_window(handle.into(), |_, window, _| {
            assert!(
                window
                    .within("popup-menu")
                    .find(ix)
                    .bounds()
                    .contains(&hint.center())
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_line_menus_select_all_keeps_the_visible_scope_and_cap(cx: &mut TestAppContext) {
    use freshkube_core::types::LogLevel;
    let (_runtime, panel, handle) = mount(cx);
    menu_fixture(cx, &panel, handle);
    panel.update(cx, |view, cx| {
        view.ingest(vec![LogEvent::new("apid", "error hidden line")], cx);
        view.review.set_level(&LogLevel::Error, false);
        cx.notify();
    });
    open_line_menu(cx, &panel, handle, 5);
    pick_line_menu(cx, handle, 1);
    cx.read(|cx| {
        assert_eq!(panel.read(cx).review.selected, (0..6).collect());
        assert_eq!(panel.read(cx).feedback, None);
    });
    panel.update(cx, |view, cx| {
        view.ingest(
            (7..251)
                .map(|ix| LogEvent::new("apid", format!("info line {ix}")))
                .collect(),
            cx,
        );
        view.reveal_last();
    });
    settle(cx, &panel, handle);
    open_line_menu(cx, &panel, handle, 250);
    pick_line_menu(cx, handle, 1);
    cx.read(|cx| {
        let view = panel.read(cx);
        assert_eq!(view.review.selected, (51..251).collect());
        assert_eq!(
            view.feedback.as_ref().unwrap().text.as_ref(),
            "Selected the newest 200 of 250 lines"
        );
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.press("escape", cx);
        window.press("secondary-a", cx);
        assert_eq!(panel.read(cx).review.selected, (51..251).collect());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_line_menus_find_focuses_search_and_escape_returns_to_lines(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    menu_fixture(cx, &panel, handle);
    open_line_menu(cx, &panel, handle, 1);
    pick_line_menu(cx, handle, 3);
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(
            panel
                .read(cx)
                .query
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        window.input("line 3", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        assert_eq!(panel.read(cx).review.query, "line 3");
        window.press("escape", cx);
        window.press("escape", cx);
        assert!(panel.read(cx).focus.is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_wrapped_line_menu_copies_the_whole_original_line(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    menu_fixture(cx, &panel, handle);
    let original = format!("info {}", "wrapped 東京 Δ ".repeat(24));
    cx.update_window(handle.into(), |_, window, cx| {
        panel.update(cx, |view, cx| {
            view.set_fixture(vec![LogEvent::new("apid", original.clone())], window, cx);
        });
    })
    .unwrap();
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        let line = window.find("log-line-3-0").bounds();
        assert!(line.size.height > window.rem_size() * 3.);
        // The centre is in a wrapped fragment, below its first line.
        window.right_click("log-line-3-0", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    pick_line_menu(cx, handle, 0);
    cx.read(|cx| {
        assert_eq!(panel.read(cx).review.selected, BTreeSet::from([0]));
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            original.trim_end()
        );
    });
}

#[gpui_kit::test]
fn stream_notes_and_empty_space_open_no_line_menu(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    menu_fixture(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        window.right_click("logs-viewport", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
        panel.update(cx, |view, cx| {
            view.set_fixture(
                vec![LogEvent::marker(
                    "apid",
                    "2026-10-01T12:00:01Z".parse().unwrap(),
                    "Stream restarted",
                )],
                window,
                cx,
            );
        });
        window.render_frame(cx);
        window.render_frame(cx);
        window.right_click("log-marker-3-0", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
        panel.update(cx, |view, cx| {
            view.set_fixture(Vec::new(), window, cx);
        });
        window.render_frame(cx);
        window.right_click("logs-empty", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
        assert!(panel.read(cx).review.selected.is_empty());
    })
    .unwrap();
}

fn stale_line_menu(cx: &mut TestAppContext, change: &str) {
    use freshkube_core::types::LogLevel;
    let (_runtime, panel, handle) = mount(cx);
    menu_fixture(cx, &panel, handle);
    // Exercise every command before the next frame removes a vanished
    // trigger: none may act on the replacement or the remaining selection.
    for item in [0usize, 1, 3] {
        menu_fixture(cx, &panel, handle);
        open_line_menu(cx, &panel, handle, 1);
        let selected = cx
            .update_window(handle.into(), |_, window, cx| {
                let position = window.within("popup-menu").find(item).bounds().center();
                window.dispatch_event(
                    gpui_kit::MouseDownEvent {
                        button: gpui_kit::MouseButton::Left,
                        position,
                        click_count: 1,
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("unchanged".into()));
                panel.update(cx, |view, cx| match change {
                    "filter" => view.review.set_level(&LogLevel::Info, false),
                    "evict" => {
                        view.review
                            .append((0..129).map(|_| LogEvent::new("apid", "x".repeat(64 * 1024))));
                    }
                    "reset" => {
                        view.reset("", window, cx);
                        view.preload(
                            vec![
                                LogEvent::new("apid", "info replacement 0"),
                                LogEvent::new("apid", "info replacement 1"),
                            ],
                            cx,
                        );
                        view.review.select(1, false, false);
                    }
                    "selection" => view.review.select(5, false, false),
                    _ => unreachable!(),
                });
                let selected = panel.read(cx).review.selected.clone();
                window.dispatch_event(
                    gpui_kit::MouseUpEvent {
                        button: gpui_kit::MouseButton::Left,
                        position,
                        click_count: 1,
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
                assert_eq!(panel.read(cx).review.selected, selected);
                assert_eq!(
                    cx.read_from_clipboard().unwrap().text().unwrap(),
                    "unchanged"
                );
                assert!(
                    !panel
                        .read(cx)
                        .query
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
                selected
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(panel.read(cx).review.selected, selected);
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                "unchanged"
            );
            assert!(
                !panel
                    .read(cx)
                    .query
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_filtered_line_makes_its_menu_inert(cx: &mut TestAppContext) {
    stale_line_menu(cx, "filter");
}

#[gpui_kit::test]
fn an_evicted_line_makes_its_menu_inert(cx: &mut TestAppContext) {
    stale_line_menu(cx, "evict");
}

#[gpui_kit::test]
fn a_new_stream_generation_makes_the_old_line_menu_inert(cx: &mut TestAppContext) {
    stale_line_menu(cx, "reset");
}

#[gpui_kit::test]
fn a_changed_selection_makes_the_old_line_menu_inert(cx: &mut TestAppContext) {
    stale_line_menu(cx, "selection");
}
