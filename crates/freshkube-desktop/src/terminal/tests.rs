use std::cell::RefCell;
use std::rc::Rc;

use alacritty_terminal::grid::Dimensions;
use gpui_kit::{
    AnyWindowHandle, AppContext, ClipboardItem, Entity, InputEvent, KeyBinding, MouseButton,
    MouseDownEvent, MouseUpEvent, Pixels, Point, ScrollDelta, Subscription, TestAppContext,
    component::{Root, Theme, ThemeMode},
    point, px, size,
    test::TestWindowExt,
};

use super::{RESIZE_INTERVAL, SCROLLBACK, TerminalEvent, TerminalSize, TerminalView, streams};

gpui_kit::actions!(terminal_tests, [AppKey]);

struct Mounted {
    window: AnyWindowHandle,
    view: Entity<TerminalView>,
    events: Rc<RefCell<Vec<TerminalEvent>>>,
    app_keys: Rc<RefCell<usize>>,
    _subscription: Subscription,
}

impl Mounted {
    /// Bytes the terminal sent since the last call, joined.
    fn output(&self) -> Vec<u8> {
        self.events
            .borrow_mut()
            .extract_if(.., |event| matches!(event, TerminalEvent::Output(_)))
            .flat_map(|event| match event {
                TerminalEvent::Output(bytes) => bytes,
                _ => Vec::new(),
            })
            .collect()
    }

    /// The other events since the last call.
    fn take_events(&self) -> Vec<TerminalEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }

    fn feed(&self, cx: &mut TestAppContext, bytes: &[u8]) {
        cx.update(|cx| self.view.update(cx, |view, cx| view.feed(bytes, cx)));
        self.frame(cx);
    }

    fn frame(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(self.window, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    fn press(&self, cx: &mut TestAppContext, key: &str) {
        cx.update_window(self.window, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn input(&self, cx: &mut TestAppContext, text: &str) {
        cx.update_window(self.window, |_, window, cx| window.input(text, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn screen(&self, cx: &mut TestAppContext) -> Vec<String> {
        cx.read(|cx| self.view.read(cx).screen_text())
    }

    /// The window position of a cell's centre.
    fn cell(&self, cx: &mut TestAppContext, line: usize, column: usize) -> Point<Pixels> {
        cx.read(|cx| {
            let geometry = self.view.read(cx).geometry;
            point(
                geometry.origin.x + geometry.cell.width * (column as f32 + 0.5),
                geometry.origin.y + geometry.cell.height * (line as f32 + 0.5),
            )
        })
    }

    fn click(&self, cx: &mut TestAppContext, at: Point<Pixels>, clicks: usize) {
        cx.update_window(self.window, |_, window, cx| {
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position: at,
                    click_count: clicks,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position: at,
                    click_count: clicks,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
        self.frame(cx);
    }

    fn copy(&self, cx: &mut TestAppContext) -> Option<String> {
        self.press(cx, "cmd-c");
        cx.read(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
    }

    fn display_offset(&self, cx: &mut TestAppContext) -> usize {
        cx.read(|cx| self.view.read(cx).term.grid().display_offset())
    }

    fn size(&self, cx: &mut TestAppContext) -> TerminalSize {
        cx.read(|cx| self.view.read(cx).size())
    }

    fn resize_window(&self, cx: &mut TestAppContext, width: f32, height: f32) {
        cx.simulate_window_resize(self.window, size(px(width), px(height)));
        self.frame(cx);
    }
}

/// A window with only a focused terminal. The app binds Tab and Escape, as
/// the shell's pages do, to show that the terminal takes them first.
fn mount(cx: &mut TestAppContext) -> Mounted {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        Theme::change(ThemeMode::Light, None, cx);
        cx.set_reduce_motion(true);
        cx.bind_keys([
            KeyBinding::new("tab", AppKey, None),
            KeyBinding::new("escape", AppKey, None),
            KeyBinding::new("ctrl-tab", AppKey, None),
            KeyBinding::new("cmd-a", AppKey, None),
        ]);
    });
    let app_keys = Rc::new(RefCell::new(0));
    let counter = app_keys.clone();
    cx.update(|cx| {
        cx.on_action(move |_: &AppKey, _| *counter.borrow_mut() += 1);
    });
    let mut view = None;
    let window = cx.open_window(size(px(800.), px(500.)), |window, cx| {
        let terminal = cx.new(|cx| TerminalView::new(window, cx));
        let focus = terminal.read(cx).focus.clone();
        focus.focus(window, cx);
        view = Some(terminal.clone());
        Root::new(terminal, window, cx)
    });
    let view = view.unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    let subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, event: &TerminalEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
    });
    let mounted = Mounted {
        window: window.into(),
        view,
        events,
        app_keys,
        _subscription: subscription,
    };
    mounted.frame(cx);
    mounted.take_events();
    mounted
}

#[gpui_kit::test]
fn bytes_draw_on_the_grid_and_rebuild_rows_once_per_cycle(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    let snapshots = crate::desktop::probe::count("terminal.snapshot");
    cx.update(|cx| {
        terminal.view.update(cx, |view, cx| {
            for chunk in [&b"hello "[..], b"\x1b[1;31mworld\x1b[0m", b"\r\nnext line"] {
                view.feed(chunk, cx);
            }
        })
    });
    terminal.frame(cx);
    assert_eq!(
        crate::desktop::probe::count("terminal.snapshot"),
        snapshots + 1
    );
    let screen = terminal.screen(cx);
    assert_eq!(&screen[..2], ["hello world", "next line"]);
    // The red word is its own run, drawn in the theme's red.
    let red = cx.read(|cx| {
        let snapshot = terminal.view.read(cx).snapshot.clone();
        snapshot.rows[0]
            .iter()
            .find(|run| run.text.as_ref() == "world")
            .map(|run| (run.style.foreground, run.style.bold))
    });
    let expected = super::snapshot::hsla(super::snapshot::palette_rgb(0xB02727));
    assert_eq!(red, Some((expected, true)));
    // Redrawing without new bytes rebuilds nothing.
    terminal.frame(cx);
    terminal.frame(cx);
    assert_eq!(
        crate::desktop::probe::count("terminal.snapshot"),
        snapshots + 1
    );
}

#[gpui_kit::test]
fn keys_are_encoded_from_the_terminal_modes(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    terminal.press(cx, "up");
    terminal.press(cx, "ctrl-c");
    terminal.press(cx, "backspace");
    terminal.press(cx, "enter");
    assert_eq!(terminal.output(), b"\x1b[A\x03\x7f\r");
    // Application cursor keys, as less and vim ask for.
    terminal.feed(cx, b"\x1b[?1h");
    terminal.press(cx, "up");
    terminal.press(cx, "shift-left");
    assert_eq!(terminal.output(), b"\x1bOA\x1b[1;2D");
}

#[gpui_kit::test]
fn the_terminal_takes_keys_before_the_apps_bindings(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    terminal.press(cx, "tab");
    terminal.press(cx, "escape");
    terminal.press(cx, "ctrl-tab");
    assert_eq!(terminal.output(), b"\t\x1b\t");
    assert_eq!(*terminal.app_keys.borrow(), 0);
    // Command shortcuts other than the terminal's own reach the app.
    terminal.press(cx, "cmd-a");
    assert_eq!(terminal.output(), b"");
    assert_eq!(*terminal.app_keys.borrow(), 1);
    // Command-Escape hands the keyboard back.
    terminal.press(cx, "cmd-escape");
    assert_eq!(terminal.take_events(), [TerminalEvent::Leave]);
}

#[gpui_kit::test]
fn typed_text_goes_to_the_program(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    terminal.input(cx, "ls -l é→");
    assert_eq!(terminal.output(), "ls -l é→".as_bytes());
}

#[gpui_kit::test]
fn dragging_selects_and_command_c_copies(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    terminal.feed(cx, b"hello world\r\nsecond line");
    let from = terminal_cell_left(terminal.cell(cx, 0, 0));
    let to = terminal.cell(cx, 0, 4);
    // End on the right half of the fifth cell, so it is included.
    let to = point(to.x + px(2.), to.y);
    cx.update_window(terminal.window, |_, window, cx| window.drag(from, to, cx))
        .unwrap();
    terminal.frame(cx);
    assert_eq!(terminal.copy(cx).as_deref(), Some("hello"));
    // The selection stays after copying, and shows on screen.
    let selected = cx.read(|cx| terminal.view.read(cx).snapshot.selected.clone());
    assert_eq!(selected, [(0, 0..5)]);
    // A drag across lines.
    let to = terminal.cell(cx, 1, 5);
    let to = point(to.x + px(2.), to.y);
    cx.update_window(terminal.window, |_, window, cx| window.drag(from, to, cx))
        .unwrap();
    assert_eq!(terminal.copy(cx).as_deref(), Some("hello world\nsecond"));
}

/// A point on the left half of a cell, where a selection includes it.
fn terminal_cell_left(centre: Point<Pixels>) -> Point<Pixels> {
    point(centre.x - px(2.), centre.y)
}

#[gpui_kit::test]
fn double_click_selects_a_word_and_triple_click_a_line(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    terminal.feed(cx, b"kubectl get pods\r\nnext");
    let at = terminal.cell(cx, 0, 9);
    terminal.click(cx, at, 2);
    assert_eq!(terminal.copy(cx).as_deref(), Some("get"));
    terminal.click(cx, at, 3);
    assert_eq!(terminal.copy(cx).as_deref(), Some("kubectl get pods\n"));
    // A single click clears the selection; copying then changes nothing.
    terminal.click(cx, at, 1);
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("kept".into())));
    assert_eq!(terminal.copy(cx).as_deref(), Some("kept"));
}

#[gpui_kit::test]
fn paste_is_bracketed_only_when_the_program_asks(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("echo a\necho b".into())));
    terminal.press(cx, "cmd-v");
    assert_eq!(terminal.output(), b"echo a\recho b");
    terminal.feed(cx, b"\x1b[?2004h");
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("x\x1b[201~y".into())));
    terminal.press(cx, "cmd-v");
    assert_eq!(terminal.output(), b"\x1b[200~x[201~y\x1b[201~");
}

#[gpui_kit::test]
fn the_wheel_scrolls_back_and_typing_returns_to_the_bottom(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    terminal.feed(cx, &streams::plain(0, 200));
    let delta = ScrollDelta::Lines(point(0., 1.));
    cx.update_window(terminal.window, |_, window, cx| {
        window.scroll("terminal", delta, cx)
    })
    .unwrap();
    terminal.frame(cx);
    assert_eq!(terminal.display_offset(cx), 3);
    assert!(cx.read(|cx| terminal.view.read(cx).snapshot.scrolled.is_some()));
    // Trackpad pixels add up to whole lines.
    let line = cx.read(|cx| terminal.view.read(cx).geometry.cell.height);
    for _ in 0..4 {
        let delta = ScrollDelta::Pixels(point(px(0.), line * 0.5));
        cx.update_window(terminal.window, |_, window, cx| {
            window.scroll("terminal", delta, cx)
        })
        .unwrap();
    }
    assert_eq!(terminal.display_offset(cx), 5);
    // New output keeps the view where it is.
    let top = terminal.screen(cx)[0].clone();
    terminal.feed(cx, &streams::plain(200, 10));
    assert_eq!(terminal.display_offset(cx), 15);
    assert_eq!(terminal.screen(cx)[0], top);
    // A key goes back to the bottom.
    terminal.input(cx, "q");
    terminal.frame(cx);
    assert_eq!(terminal.display_offset(cx), 0);
    assert!(cx.read(|cx| terminal.view.read(cx).snapshot.scrolled.is_none()));
}

#[gpui_kit::test]
fn the_wheel_sends_arrows_on_the_alternate_screen(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    terminal.feed(cx, &streams::plain(0, 100));
    terminal.feed(cx, &streams::top_frame(0, 80, 24));
    terminal.output();
    let delta = ScrollDelta::Lines(point(0., -1.));
    cx.update_window(terminal.window, |_, window, cx| {
        window.scroll("terminal", delta, cx)
    })
    .unwrap();
    assert_eq!(terminal.output(), b"\x1b[B\x1b[B\x1b[B");
    assert_eq!(terminal.display_offset(cx), 0);
}

#[gpui_kit::test]
fn scrollback_keeps_ten_thousand_lines(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    terminal.feed(cx, &streams::coloured(0, SCROLLBACK + 2_000));
    let history = cx.read(|cx| terminal.view.read(cx).term.grid().history_size());
    assert_eq!(history, SCROLLBACK);
}

#[gpui_kit::test]
fn the_grid_follows_the_window_and_resizes_at_most_every_interval(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    let first = terminal.size(cx);
    assert!(first.columns > 60 && first.rows > 15, "{first:?}");
    // The first change after a quiet interval applies at once; the ones
    // after it wait out the interval, and only the last is sent.
    cx.executor().advance_clock(RESIZE_INTERVAL);
    terminal.resize_window(cx, 600., 400.);
    let second = terminal.size(cx);
    assert!(second.columns < first.columns && second.rows < first.rows);
    assert_eq!(terminal.take_events(), [TerminalEvent::Resize(second)]);
    terminal.resize_window(cx, 500., 380.);
    terminal.resize_window(cx, 400., 300.);
    assert_eq!(terminal.size(cx), second);
    assert_eq!(terminal.take_events(), []);
    cx.executor().advance_clock(RESIZE_INTERVAL);
    terminal.frame(cx);
    let last = terminal.size(cx);
    assert!(last.columns < second.columns);
    assert_eq!(terminal.take_events(), [TerminalEvent::Resize(last)]);
    let rows = cx.read(|cx| terminal.view.read(cx).term.screen_lines());
    assert_eq!(rows, usize::from(last.rows));
}

#[gpui_kit::test]
fn a_larger_text_size_makes_larger_cells(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    let before = terminal.size(cx);
    let cell = cx.read(|cx| terminal.view.read(cx).geometry.cell);
    cx.update(|cx| crate::text_size::set(20., cx));
    terminal.frame(cx);
    cx.executor().advance_clock(RESIZE_INTERVAL);
    terminal.frame(cx);
    let larger = cx.read(|cx| terminal.view.read(cx).geometry.cell);
    assert!(larger.width > cell.width && larger.height > cell.height);
    let after = terminal.size(cx);
    assert!(after.columns < before.columns && after.rows < before.rows);
    assert_eq!(terminal.take_events(), [TerminalEvent::Resize(after)]);
}

#[gpui_kit::test]
fn colours_follow_the_appearance(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    terminal.feed(cx, b"\x1b[31mred\x1b[0m");
    let red = |cx: &mut TestAppContext| {
        cx.read(|cx| terminal.view.read(cx).snapshot.rows[0][0].style.foreground)
    };
    assert_eq!(
        red(cx),
        super::snapshot::hsla(super::snapshot::palette_rgb(0xB02727))
    );
    cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    terminal.frame(cx);
    assert_eq!(
        red(cx),
        super::snapshot::hsla(super::snapshot::palette_rgb(0xE05A5A))
    );
    // A program asking for the background hears the theme's.
    terminal.feed(cx, b"\x1b]11;?\x07");
    assert_eq!(terminal.output(), b"\x1b]11;rgb:1515/1a1a/2121\x07");
}

#[gpui_kit::test]
fn programs_reach_the_clipboard_and_set_the_title(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    // OSC 52 writes...
    terminal.feed(cx, b"\x1b]52;c;aGVsbG8=\x07");
    let clipboard = cx.read(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(clipboard.as_deref(), Some("hello"));
    // ...and reads, as the user chose.
    terminal.feed(cx, b"\x1b]52;c;?\x07");
    assert_eq!(terminal.output(), b"\x1b]52;c;aGVsbG8=\x07");
    // Replies to queries go back to the program.
    terminal.feed(cx, b"\x1b[6n");
    assert_eq!(terminal.output(), b"\x1b[1;1R");
    terminal.feed(cx, b"\x1b]2;root@web-0: /\x07");
    assert_eq!(
        terminal.take_events(),
        [TerminalEvent::Title(Some("root@web-0: /".into()))]
    );
    let title = cx.read(|cx| terminal.view.read(cx).title().cloned());
    assert_eq!(title.as_deref(), Some("root@web-0: /"));
}

#[gpui_kit::test]
fn wide_and_combining_characters_keep_their_columns(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    terminal.feed(cx, "日本 e\u{301}x".as_bytes());
    let runs = cx.read(|cx| {
        terminal.view.read(cx).snapshot.rows[0]
            .iter()
            .map(|run| (run.column, run.cells, run.text.to_string()))
            .collect::<Vec<_>>()
    });
    assert_eq!(
        runs,
        [
            (0, 2, "日".to_owned()),
            (2, 2, "本".to_owned()),
            (5, 1, "e\u{301}".to_owned()),
            (6, 1, "x".to_owned()),
        ]
    );
}

#[gpui_kit::test]
fn wrapped_unicode_lines_stay_inside_the_grid(cx: &mut TestAppContext) {
    let terminal = mount(cx);
    terminal.feed(cx, &streams::unicode(60));
    let (columns, rows) = cx.read(|cx| {
        let view = terminal.view.read(cx);
        (view.term.columns(), view.snapshot.rows.clone())
    });
    for runs in &rows {
        let mut end = 0;
        for run in runs {
            assert!(run.column >= end, "overlapping runs: {runs:?}");
            end = run.column + run.cells;
        }
        assert!(end <= columns, "a run passes the edge: {runs:?}");
    }
    assert!(
        rows.iter()
            .any(|runs| runs.iter().any(|run| run.text.as_ref() == "🚀"))
    );
}

#[gpui_kit::test]
fn composed_text_is_sent_only_once_committed(cx: &mut TestAppContext) {
    use gpui_kit::EntityInputHandler;
    let terminal = mount(cx);
    let view = terminal.view.clone();
    // Option-e marks an accent, then e commits é, as a dead key does.
    cx.update_window(terminal.window, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.replace_and_mark_text_in_range(None, "´", None, window, cx)
        })
    })
    .unwrap();
    assert_eq!(terminal.output(), b"");
    assert_eq!(
        cx.read(|cx| view.read(cx).marked.clone()).as_deref(),
        Some("´")
    );
    cx.update_window(terminal.window, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.replace_text_in_range(None, "é", window, cx)
        })
    })
    .unwrap();
    assert_eq!(terminal.output(), "é".as_bytes());
    assert_eq!(cx.read(|cx| view.read(cx).marked.clone()), None);
}
