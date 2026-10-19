use super::*;

/// A stack trace in one message: 200 lines, the first naming the panic
/// and the last naming the function the search looks for (#225).
fn stack_trace() -> String {
    let mut lines = vec!["error panic: settlement queue closed".to_owned()];
    lines.extend((1..199).map(|ix| format!("    frame {ix}(0x0, 0x0, 0xc000123456)")));
    lines.push("    payments/worker.drainSettlementQueue()".to_owned());
    lines.join("\n")
}

/// A search moves to the matched line within a row taller than the list,
/// not to the row's middle, wrapped or not; a match on the row's first
/// line shows the row's top.
#[gpui_kit::test]
fn a_search_shows_the_matched_line_within_a_tall_row(cx: &mut TestAppContext) {
    for wrapped in [true, false] {
        let (_runtime, panel, handle) = mount(cx);
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            deliver(&panel, vec![stack_trace()], cx);
            if !wrapped {
                window.click("logs-wrap", cx);
            }
            window.render_frame(cx);
        })
        .unwrap();
        settle(cx, &panel, handle);
        for (query, end) in [
            ("drainSettlementQueue", true),
            ("settlement queue closed", false),
        ] {
            cx.update_window(handle.into(), |_, window, cx| {
                window.click("logs-search", cx);
                window.press("secondary-a", cx);
                window.input(query, cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| {
                window.click("logs-search-next", cx);
                window.render_frame(cx);
                window.render_frame(cx);
                let view = panel.read(cx);
                let id = view.review.current_match.expect("a match").id;
                let row = window
                    .find(SharedString::from(format!("log-line-1-{id}")))
                    .bounds();
                let list = window.find("logs-viewport").bounds();
                let case = format!("wrapped {wrapped}, {query}");
                assert!(
                    row.size.height > list.size.height * 3.,
                    "{case}: the row {row:?} isn't tall"
                );
                if end {
                    // The matched last line ends the row: its bottom shows.
                    assert!(
                        row.bottom() > list.top() && row.bottom() <= list.bottom(),
                        "{case}: the row's end {:?} is outside the list {list:?}",
                        row.bottom()
                    );
                } else {
                    assert!(
                        row.top() >= list.top() && row.top() < list.bottom(),
                        "{case}: the row's top {:?} is outside the list {list:?}",
                        row.top()
                    );
                }
            })
            .unwrap();
        }
    }
}

/// A stack trace in one message, 200 lines, that names the function the
/// search looks for on three of them: the 61st, the 121st and the last.
fn trace_with_three_matches() -> String {
    let mut lines = vec!["error panic: settlement queue closed".to_owned()];
    lines.extend((1..199).map(|ix| match ix {
        60 | 120 => format!("    payments/ledger.retryLedger(0x{ix:x})"),
        _ => format!("    frame {ix}(0x0, 0x0, 0xc000123456)"),
    }));
    lines.push("    payments/ledger.retryLedger.func1()".to_owned());
    lines.join("\n")
}

/// Next visits every matching line of a row taller than the list, each
/// shown in the list, before it moves on to the next row; Previous comes
/// back to the row's last match. Each step within the tall row lays the
/// row out again to find its line (#281).
#[gpui_kit::test]
fn next_shows_every_matched_line_of_a_tall_row(cx: &mut TestAppContext) {
    let reveals = || freshkube_probe::probe::count("logs.reveal_line");
    for wrapped in [true, false] {
        let (_runtime, panel, handle) = mount(cx);
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            deliver(
                &panel,
                vec![
                    trace_with_three_matches(),
                    "info retryLedger finished".to_owned(),
                ],
                cx,
            );
            if !wrapped {
                window.click("logs-wrap", cx);
            }
            window.render_frame(cx);
            window.click("logs-search", cx);
            window.input("retryledger", cx);
        })
        .unwrap();
        settle(cx, &panel, handle);
        let trace = SharedString::from("log-line-1-120");
        let mut offsets = Vec::new();
        let mut line_ends: Vec<gpui_kit::Pixels> = Vec::new();
        for (step, line) in [(1, 60), (2, 120), (3, 199)] {
            let case = format!("wrapped {wrapped}, step {step}");
            let before = reveals();
            cx.update_window(handle.into(), |_, window, cx| {
                window.click("logs-search-next", cx);
                window.render_frame(cx);
            })
            .unwrap();
            assert!(reveals() > before, "{case}: the row wasn't laid out again");
            // Nothing a later frame asks for moves the line away.
            cx.update_window(handle.into(), |_, window, cx| {
                window.simulate_next_frame(cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                let hit = panel.read(cx).review.current_match.expect("a match");
                assert_eq!((hit.id, hit.line), (120, Some(line)), "{case}");
                assert_eq!(panel.read(cx).review.current_position(), Some(step - 1));
                assert_eq!(panel.read(cx).review.match_count(), 4);
                // Laying a row out works only while a frame draws; read
                // what the reveal measured then.
                let (id, revealed, line_end) =
                    panel.read(cx).revealed_line().expect("a revealed line");
                assert_eq!((id, revealed), (120, line), "{case}");
                if let Some(&earlier) = line_ends.last() {
                    assert!(
                        line_end > earlier,
                        "{case}: line {line} ends above the last"
                    );
                }
                line_ends.push(line_end);
                let row = window.find(trace.clone()).bounds();
                let list = window.find("logs-viewport").bounds();
                assert!(
                    row.size.height > list.size.height * 3.,
                    "{case}: the row {row:?} isn't tall"
                );
                let shown = row.top() + line_end;
                assert!(
                    shown > list.top() + window.rem_size() && shown <= list.bottom(),
                    "{case}: line {line} ends at {shown:?}, outside the list {list:?}"
                );
                assert!(
                    window
                        .find("logs-search-count")
                        .label()
                        .is_some_and(|label| label.starts_with("4 matching lines")),
                    "{case}: the count doesn't say it counts lines"
                );
                offsets.push(panel.read(cx).scroll.offset().y);
            })
            .unwrap();
        }
        assert!(
            offsets[0] > offsets[1] && offsets[1] > offsets[2],
            "wrapped {wrapped}: each step moved down the row: {offsets:?}"
        );
        // After the row's last match Next moves on to the short row, which
        // shows whole and needs no line found.
        let before = reveals();
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("logs-search-next", cx);
            window.render_frame(cx);
            let hit = panel.read(cx).review.current_match.expect("a match");
            assert_eq!(hit.id, 121);
            assert!(window.find(SharedString::from("log-line-1-121")).visible());
        })
        .unwrap();
        assert_eq!(reveals(), before);
        // Previous comes back to the trace's last match.
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("logs-search-prev", cx);
            window.render_frame(cx);
            let hit = panel.read(cx).review.current_match.expect("a match");
            assert_eq!((hit.id, hit.line), (120, Some(199)));
        })
        .unwrap();
        assert!(reveals() > before);
        // An arrow key after a search reveals its own row, not the line
        // the search had matched: here the trace's line 120.
        panel.update(cx, |view, cx| {
            view.search(false, cx);
            assert_eq!(view.reveal_matched_line, Some(120));
            view.navigate(1, false, cx);
            assert_eq!(view.reveal_matched_line, None);
        });
    }
}

/// In a message of a few lines, short enough to show whole, Next moves the
/// current mark from one matching line to the next within the row, and the
/// row as a whole isn't marked, so the step shows though nothing scrolls
/// (#281).
#[gpui_kit::test]
fn next_moves_the_current_line_within_a_short_row(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let trace = "error retry failed: ledger busy\n    at retryLedger(1)\n    \
                     at main()\n    at retryLedger(2)";
        deliver(&panel, vec![trace.to_owned()], cx);
        window.render_frame(cx);
        window.click("logs-search", cx);
        window.input("retryledger", cx);
    })
    .unwrap();
    settle(cx, &panel, handle);
    for (step, line) in [(1, 1), (2, 3)] {
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("logs-search-next", cx);
            window.render_frame(cx);
            let view = panel.read(cx);
            let hit = view.review.current_match.expect("a match");
            assert_eq!((hit.id, hit.line), (120, Some(line)));
            let ix = view.review.row_for_id(120).unwrap();
            let marks = view.review.marks(ix);
            assert_eq!(marks.row, None, "step {step}: the whole row is marked");
            let current: Vec<bool> = (marks.lines.iter())
                .map(|(_, mark)| *mark == Mark::Current)
                .collect();
            assert_eq!(current, [step == 1, step == 2], "step {step}");
            assert_eq!(view.review.search_count().text, format!("{step} of 2"));
            assert!(window.find(SharedString::from("log-line-1-120")).visible());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn searches_reveal_inner_rows_and_keyboard_copies_complete_selected_lines(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    let row = SharedString::from("log-line-1-10");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let toolbar = window.find("logs-toolbar").bounds();
        assert!(toolbar.size.height > window.rem_size());
        let viewport = window.find("logs-viewport").bounds();
        let search = window.find("logs-search").bounds();
        assert!(search.bottom() <= viewport.top());
        assert!(viewport.top() - search.bottom() <= window.rem_size() * 2.);
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("unchanged".into()));
        let feedback = panel.read(cx).feedback.clone();
        window.click("logs-copy", cx);
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("unchanged")
        );
        assert_eq!(panel.read(cx).feedback, feedback);
        assert!(window.try_find(row.clone()).is_none());
        window.click("logs-search", cx);
        window.input("needle", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        let toolbar = window.find("logs-search").bounds();
        window.click("logs-search-next", cx);
        let viewport = window.find("logs-viewport").bounds();
        let line = window.find(row.clone());
        assert!(line.visible());
        assert!(line.bounds().top() >= viewport.top());
        assert!(line.bounds().bottom() <= viewport.bottom());
        assert_eq!(window.find("logs-search").bounds(), toolbar);
        assert_eq!(panel.read(cx).review.visible.len(), 120);
        assert!(!panel.read(cx).following);
        assert_eq!(
            panel.read(cx).review.current_match.map(|hit| hit.id),
            Some(10)
        );
        window.click(row, cx);
        assert_eq!(panel.read(cx).review.selected.len(), 1);
        assert!(panel.read(cx).focus.is_focused(window));
        window.press("shift-down", cx);
        assert_eq!(panel.read(cx).review.selected.len(), 2);
        window.press("secondary-c", cx);
        let text = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap();
        assert_eq!(
            text,
            "error needle first 東京\ninfo ordinary complete line 11"
        );
        assert!(!text.contains("[apid]"));
        window.press("escape", cx);
        assert!(panel.read(cx).review.selected.is_empty());
        window.click("logs-search-next", cx);
        assert_eq!(
            panel.read(cx).review.current_match.map(|hit| hit.id),
            Some(90)
        );
        assert!(window.find(SharedString::from("log-line-1-90")).visible());
        window.click("logs-search-prev", cx);
        assert_eq!(
            panel.read(cx).review.current_match.map(|hit| hit.id),
            Some(10)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn escape_in_the_search_clears_it_then_hands_the_keyboard_to_the_lines(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("logs-search", cx);
        window.input("needle", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(panel.read(cx).review.query, "needle");
        window.press("escape", cx);
        window.render_frame(cx);
        // The first Escape clears the search and stays in it.
        assert!(panel.read(cx).review.query.is_empty());
        assert!(panel.read(cx).query.read(cx).value().is_empty());
        assert!(!panel.read(cx).focus.is_focused(window));
        window.press("escape", cx);
        window.render_frame(cx);
        // The second hands the keyboard to the lines.
        assert!(panel.read(cx).focus.is_focused(window));
        window.press("down", cx);
        assert_eq!(panel.read(cx).review.selected.len(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn command_f_g_and_a_find_and_select_from_the_lines_or_the_search(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let lines = panel.read(cx).focus.clone();
        window.focus(&lines, cx);
        // Command-A selects every visible line; the fixture is under the
        // limit, so nothing is said.
        window.press("secondary-a", cx);
        assert_eq!(panel.read(cx).review.selected.len(), 120);
        assert_eq!(panel.read(cx).feedback, None);
        window.press("escape", cx);
        assert!(panel.read(cx).review.selected.is_empty());
        // Command-F goes from the lines to the search.
        window.press("secondary-f", cx);
        assert!(!panel.read(cx).focus.is_focused(window));
        window.input("needle", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let current =
            |cx: &mut gpui_kit::App| panel.read(cx).review.current_match.map(|hit| hit.id);
        // From the search, Command-G and Shift-Command-G step through the
        // matches, as F3 and Shift-F3 do from the lines.
        window.press("secondary-g", cx);
        assert_eq!(current(cx), Some(10));
        window.press("secondary-g", cx);
        assert_eq!(current(cx), Some(90));
        window.press("secondary-shift-g", cx);
        assert_eq!(current(cx), Some(10));
        let lines = panel.read(cx).focus.clone();
        window.focus(&lines, cx);
        window.press("f3", cx);
        assert_eq!(current(cx), Some(90));
        window.press("shift-f3", cx);
        assert_eq!(current(cx), Some(10));
        window.press("secondary-g", cx);
        assert_eq!(current(cx), Some(90));
    })
    .unwrap();
}

/// Enter in the search steps to the next match and Shift-Enter to the
/// previous one, and the keyboard stays in the search, as in a browser
/// (#316). Escape's steps don't change.
#[gpui_kit::test]
fn enter_in_the_search_steps_through_the_matches_and_keeps_the_keyboard(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let lines = panel.read(cx).focus.clone();
        window.focus(&lines, cx);
        window.press("secondary-f", cx);
        window.input("needle", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let current = |cx: &mut gpui_kit::App| panel.read(cx).review.current_match.map(|hit| hit.id);
    let in_search = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        let query = gpui_kit::Focusable::focus_handle(panel.read(cx).query.read(cx), cx);
        query.is_focused(window)
    };
    for (key, expected) in [
        ("enter", 10),
        ("enter", 90),
        ("enter", 10),
        ("shift-enter", 90),
        ("shift-enter", 10),
    ] {
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert_eq!(current(cx), Some(expected), "after {key}");
            assert!(
                in_search(window, cx),
                "{key} keeps the keyboard in the search"
            );
        })
        .unwrap();
    }
    cx.update_window(handle.into(), |_, window, cx| {
        // Escape clears the search and stays; a second hands the keyboard
        // to the lines.
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(panel.read(cx).review.query.is_empty());
        assert!(in_search(window, cx));
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(panel.read(cx).focus.is_focused(window));
    })
    .unwrap();
}
