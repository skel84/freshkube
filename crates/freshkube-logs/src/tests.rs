use std::collections::{BTreeMap, BTreeSet};

use gpui_kit::{
    AnyElement, AppContext, Context, Entity, InteractiveElement, IntoElement, ScrollDelta,
    SharedString, Styled, TestAppContext, TestSupportExt, Window, WindowHandle,
    component::{Root, Theme},
    point, px, size,
    test::TestWindowExt,
};
use tokio::runtime::{Builder, Handle, Runtime};

use freshkube_core::logs::{LogEvent, ServiceId};

use super::{LogSource, LogView, review::Mark};

/// A source with no stream of its own, for testing the view: the tests
/// hand it batches as a stream would deliver them, through `apply_batch`.
struct TestLogs {
    fixture_target: Option<Target>,
    stream_revision: u64,
    errors: BTreeMap<ServiceId, String>,
    /// The source's own words for its panel, when a test gives some.
    words: Option<(SharedString, SharedString)>,
    /// A tall control of the source's, as a pod's restart banner is.
    banner: bool,
}

/// The stream a batch came from.
#[derive(Clone, PartialEq)]
struct Target(u64);

/// One line or failure from one source's stream.
struct StreamEvent {
    target: Target,
    service: ServiceId,
    result: Result<String, String>,
}

impl LogSource for TestLogs {
    fn controls(view: &LogView<Self>, _: &mut Context<LogView<Self>>) -> Vec<AnyElement> {
        view.source()
            .banner
            .then(|| {
                gpui_kit::div()
                    .id("test-banner")
                    .test_support()
                    .h(freshkube_ui::ui::dp(72.))
                    .into_any_element()
            })
            .into_iter()
            .collect()
    }

    fn empty_message(_: &LogView<Self>) -> SharedString {
        "No lines.".into()
    }

    fn errors(&self) -> &BTreeMap<ServiceId, String> {
        &self.errors
    }

    fn follow_tooltip(view: &LogView<Self>) -> SharedString {
        match &view.source().words {
            Some((tooltip, _)) => tooltip.clone(),
            None => "Pause to review. Collection keeps running.".into(),
        }
    }

    fn panel_label(view: &LogView<Self>) -> SharedString {
        match &view.source().words {
            Some((_, label)) => label.clone(),
            None => "Live logs panel".into(),
        }
    }
}

type LogPanel = LogView<TestLogs>;

/// What the tests ask of the source, named as the Talos page names it.
trait TestPanel: Sized + 'static {
    fn new(runtime: Handle, tail: i32, window: &mut Window, cx: &mut Context<Self>) -> Self;

    fn set_fixture(&mut self, events: Vec<LogEvent>, window: &mut Window, cx: &mut Context<Self>);

    fn apply_batch(
        &mut self,
        target: &Target,
        revision: u64,
        batch: Vec<StreamEvent>,
        cx: &mut Context<Self>,
    ) -> bool;
}

impl TestPanel for LogPanel {
    fn new(_: Handle, _: i32, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let source = TestLogs {
            fixture_target: None,
            stream_revision: 0,
            errors: BTreeMap::new(),
            words: None,
            banner: false,
        };
        Self::with_source(source, window, cx)
    }

    /// Starts over on example lines, showing every source, and follows
    /// them: what the Talos page does with its example data.
    fn set_fixture(&mut self, events: Vec<LogEvent>, window: &mut Window, cx: &mut Context<Self>) {
        self.source_mut().stream_revision += 1;
        self.reset("fixture.invalid", window, cx);
        self.source_mut().fixture_target = Some(Target(self.generation()));
        let shown: BTreeSet<_> = events.iter().map(|event| event.service.clone()).collect();
        self.preload(events, cx);
        self.set_shown(shown);
        self.reveal_last();
        cx.notify();
    }

    /// Hands a batch from the current stream to the view, as a source does;
    /// a batch from an earlier stream, or a line from another, is dropped.
    fn apply_batch(
        &mut self,
        target: &Target,
        revision: u64,
        batch: Vec<StreamEvent>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.source().stream_revision != revision
            || self.source().fixture_target.as_ref() != Some(target)
        {
            return false;
        }
        let mut lines = Vec::new();
        for event in batch {
            if &event.target != target {
                continue;
            }
            match event.result {
                Ok(line) => lines.push(LogEvent::new(event.service, line)),
                Err(error) => {
                    self.source_mut().errors.insert(event.service, error);
                }
            }
        }
        self.ingest(lines, cx);
        true
    }
}

fn fixture_events() -> Vec<LogEvent> {
    (0..120)
        .map(|ix| {
            let detail = if ix == 10 {
                "error needle first 東京".to_owned()
            } else if ix == 90 {
                format!(
                    "error needle last {}",
                    "wrapped Unicode Δ 東京 🚀 ".repeat(24)
                )
            } else {
                format!("info ordinary complete line {ix}")
            };
            LogEvent::new(if ix % 2 == 0 { "apid" } else { "kubelet" }, detail)
        })
        .collect()
}

fn mount(cx: &mut TestAppContext) -> (Runtime, Entity<LogPanel>, WindowHandle<Root>) {
    mount_sized(cx, 620., 760.)
}

fn mount_sized(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (Runtime, Entity<LogPanel>, WindowHandle<Root>) {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(gpui_kit::init);
    let mut panel = None;
    let handle = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = LogPanel::new(runtime.handle().clone(), 100, window, cx);
            view.set_fixture(fixture_events(), window, cx);
            view
        });
        panel = Some(view.clone());
        Root::new(view, window, cx)
    });
    (runtime, panel.unwrap(), handle)
}

/// Lets a resize settle, then delivers frames until no row is estimated.
fn settle(cx: &mut TestAppContext, panel: &Entity<LogPanel>, handle: WindowHandle<Root>) {
    cx.executor().advance_clock(super::RESIZE_SETTLE);
    cx.run_until_parked();
    for _ in 0..64 {
        let settled = cx
            .update_window(handle.into(), |_, window, cx| {
                window.simulate_next_frame(cx);
                window.render_frame(cx);
                !panel.read(cx).row_exact.contains(&false)
            })
            .unwrap();
        if settled {
            return;
        }
    }
    panic!("estimated rows never settled");
}

#[gpui_kit::test]
fn a_source_that_is_not_a_stream_names_its_panel_and_follow_in_its_own_words(
    cx: &mut TestAppContext,
) {
    let (_runtime, panel, handle) = mount(cx);
    let label = |cx: &mut TestAppContext| {
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.find("logs-panel").label().map(str::to_owned)
        })
        .unwrap()
    };
    assert_eq!(label(cx).as_deref(), Some("Live logs panel"));
    panel.update(cx, |view, cx| {
        view.source_mut().words = Some((
            "Pause to review. Refresh adds newer messages.".into(),
            "Saved logs panel".into(),
        ));
        cx.notify();
    });
    assert_eq!(label(cx).as_deref(), Some("Saved logs panel"));
    panel.read_with(cx, |view, _| {
        assert_eq!(
            TestLogs::follow_tooltip(view),
            "Pause to review. Refresh adds newer messages."
        )
    });
}

#[gpui_kit::test]
fn a_rows_level_follows_its_severity(cx: &mut TestAppContext) {
    use freshkube_core::types::LogLevel;
    use freshkube_ui::palette::palette;

    let (_runtime, _panel, _handle) = mount(cx);
    cx.update(|cx| {
        let p = palette(cx);
        let style = |level| super::view::level_style(&level, &p);
        // An error reads as more severe than a warning, not as information.
        assert_eq!(style(LogLevel::Error), ("ERROR", p.crit_ink, p.crit));
        assert_eq!(style(LogLevel::Warning), ("WARN", p.warn_ink, p.warn));
        assert_eq!(style(LogLevel::Info).1, p.muted);
    });
}

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

/// Delivers lines as the example stream would, from one service.
fn deliver(panel: &Entity<LogPanel>, lines: Vec<String>, cx: &mut gpui_kit::App) {
    panel.update(cx, |view, cx| {
        let target = view.source.fixture_target.clone().unwrap();
        let revision = view.source.stream_revision;
        let batch = lines
            .into_iter()
            .map(|line| StreamEvent {
                target: target.clone(),
                service: ServiceId::from("apid"),
                result: Ok(line),
            })
            .collect();
        assert!(view.apply_batch(&target, revision, batch, cx));
    });
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
    cx.executor().advance_clock(super::RESIZE_SETTLE / 2);
    cx.update_window(handle.into(), |_, window, cx| {
        deliver(&panel, flood(400), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.executor().advance_clock(super::RESIZE_SETTLE * 3 / 4);
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
                    panel.read(cx).revealed_line.expect("a revealed line");
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

/// A frame that scrolls around a log, as a short node pane does.
struct Frame {
    panel: Entity<LogPanel>,
    scroll: gpui_kit::ScrollHandle,
    /// The log's height inside the frame.
    height: f32,
}

impl gpui_kit::Render for Frame {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui_kit::IntoElement {
        use gpui_kit::{TestSupportExt, prelude::*};
        gpui_kit::div()
            .id("frame")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                gpui_kit::div()
                    .id("frame-header")
                    .test_support()
                    .h(px(120.)),
            )
            .child(gpui_kit::div().h(px(self.height)).child(self.panel.clone()))
            .child(gpui_kit::div().h(px(600.)))
    }
}

/// A log `height` tall in a [`Frame`] that scrolls around it.
fn mount_framed(
    cx: &mut TestAppContext,
    height: f32,
) -> (
    Runtime,
    Entity<LogPanel>,
    gpui_kit::ScrollHandle,
    WindowHandle<Root>,
) {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(gpui_kit::init);
    let scroll = gpui_kit::ScrollHandle::new();
    let mut panel = None;
    let handle = cx.open_window(size(px(620.), px(760.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = LogPanel::new(runtime.handle().clone(), 100, window, cx);
            view.set_fixture(fixture_events(), window, cx);
            view
        });
        panel = Some(view.clone());
        let frame = cx.new(|_| Frame {
            panel: view,
            scroll: scroll.clone(),
            height,
        });
        Root::new(frame, window, cx)
    });
    (runtime, panel.unwrap(), scroll, handle)
}

#[gpui_kit::test]
fn a_wheel_over_a_short_logs_controls_scrolls_the_log_and_not_the_frame(cx: &mut TestAppContext) {
    let (_runtime, panel, scroll, handle) = mount_framed(cx, 180.);
    panel.update(cx, |view, cx| {
        view.source_mut().banner = true;
        cx.notify();
    });
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        let panel_scroll = panel.read(cx).panel_scroll.clone();
        assert!(
            panel_scroll.max_offset().y > px(0.),
            "the log doesn't scroll"
        );
        window.scroll(
            "frame-header",
            ScrollDelta::Pixels(point(px(0.), px(-60.))),
            cx,
        );
        window.render_frame(cx);
        let framed = scroll.offset().y;
        assert!(
            framed < px(-1.),
            "the header's wheel didn't scroll the frame"
        );
        // Down to the lines, then back up to the banner.
        for by in [-400., 400.] {
            window.scroll(
                "logs-toolbar",
                ScrollDelta::Pixels(point(px(0.), px(by))),
                cx,
            );
            window.render_frame(cx);
            let expected = if by < 0. {
                -panel_scroll.max_offset().y
            } else {
                px(0.)
            };
            assert_eq!(
                panel_scroll.offset().y,
                expected,
                "the log didn't scroll by {by}"
            );
            assert_eq!(
                scroll.offset().y,
                framed,
                "the log's wheel by {by} scrolled the frame"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_wheel_over_the_log_scrolls_the_log_and_not_the_frame_around_it(cx: &mut TestAppContext) {
    let (_runtime, panel, scroll, handle) = mount_framed(cx, 560.);
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(scroll.max_offset().y > px(0.), "the frame doesn't scroll");
        let down = ScrollDelta::Pixels(point(px(0.), px(-60.)));
        window.scroll("frame-header", down, cx);
        window.render_frame(cx);
        let framed = scroll.offset().y;
        assert!(
            framed < px(0.),
            "the header's wheel didn't scroll the frame"
        );
        assert!(panel.read(cx).following);
        let before = panel.read(cx).scroll.offset();
        window.scroll(
            "logs-viewport",
            ScrollDelta::Pixels(point(px(0.), px(300.))),
            cx,
        );
        window.render_frame(cx);
        assert!(!panel.read(cx).following, "the wheel didn't reach the log");
        assert_ne!(
            panel.read(cx).scroll.offset(),
            before,
            "the log didn't scroll"
        );
        assert_eq!(
            scroll.offset().y,
            framed,
            "the log's wheel scrolled the frame"
        );
    })
    .unwrap();
}

/// Delivers `count` lines and lets the executor's clock pass, as a
/// stream's next batch would.
fn deliver_more(cx: &mut TestAppContext, panel: &Entity<LogPanel>, from: usize, count: usize) {
    let lines = (from..from + count)
        .map(|ix| format!("info later line {ix}"))
        .collect();
    cx.update(|cx| deliver(panel, lines, cx));
    cx.executor().advance_clock(super::HIDDEN_APPLY_INTERVAL);
    cx.run_until_parked();
}

fn wheel(cx: &mut TestAppContext, handle: WindowHandle<Root>, x: f32, y: f32) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.scroll(
            "logs-viewport",
            ScrollDelta::Pixels(point(px(x), px(y))),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn at_the_end_a_wheel_down_or_none_at_all_keeps_following(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    let tail = cx.read(|cx| panel.read(cx).scroll.offset());
    assert!(cx.read(|cx| panel.read(cx).following));

    wheel(cx, handle, 0., -300.);
    cx.read(|cx| {
        assert!(panel.read(cx).following, "a wheel down at the end paused");
        assert_eq!(panel.read(cx).scroll.offset(), tail);
    });
    wheel(cx, handle, 0., 0.);
    assert!(
        cx.read(|cx| panel.read(cx).following),
        "an empty wheel paused"
    );

    deliver_more(cx, &panel, 0, 30);
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(panel.read(cx).following);
        assert!(
            panel.read(cx).scroll.offset().y < tail.y,
            "the tail moved on"
        );
        assert!(window.find(SharedString::from("log-line-1-149")).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_wheel_up_pauses_follow_and_new_lines_leave_the_list_where_it_is(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    let tail = cx.read(|cx| panel.read(cx).scroll.offset());

    wheel(cx, handle, 0., 300.);
    let paused = cx.read(|cx| {
        assert!(!panel.read(cx).following, "a wheel up kept following");
        panel.read(cx).scroll.offset()
    });
    assert!(paused.y > tail.y, "the list didn't scroll up");
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.find(SharedString::from("log-line-1-99")).visible());
    })
    .unwrap();

    deliver_more(cx, &panel, 0, 30);
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(!panel.read(cx).following);
        assert_eq!(panel.read(cx).scroll.offset(), paused);
        assert!(window.find(SharedString::from("log-line-1-99")).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_sideways_wheel_keeps_following(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("logs-wrap", cx);
        assert!(!panel.read(cx).wrapped);
    })
    .unwrap();
    // A wide newest line, so the list has somewhere to go sideways.
    cx.update(|cx| {
        deliver(
            &panel,
            vec![format!("info wide {}", "東京 ".repeat(200))],
            cx,
        )
    });
    settle(cx, &panel, handle);
    let tail = cx.read(|cx| panel.read(cx).scroll.offset());
    assert!(cx.read(|cx| panel.read(cx).following));

    wheel(cx, handle, -200., 0.);
    cx.read(|cx| {
        let offset = panel.read(cx).scroll.offset();
        assert!(panel.read(cx).following, "a sideways wheel paused");
        assert_eq!(offset.y, tail.y);
        assert!(offset.x < tail.x, "the wheel didn't reach the list");
    });
}

#[gpui_kit::test]
fn the_levels_menu_hides_a_level_and_says_how_many_show(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        assert_eq!(
            window.find("logs-levels").label(),
            Some("Levels: all shown")
        );
        // The tooltip counts each level, derived when the counts changed.
        assert_eq!(
            panel.read(cx).levels_tip.1.as_ref(),
            "Lines by level: Error 2 · Warn 0 · Info 118 · Debug 0 · Unknown 0"
        );
        window.click("logs-levels", cx);
    })
    .unwrap();
    cx.run_until_parked();
    // The first item, Error, from the keyboard.
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = panel.read(cx);
        let levels = &view.review.logs.buffer().filters().levels;
        assert!(!levels.accepts(&freshkube_core::types::LogLevel::Error));
        assert!(levels.accepts(&freshkube_core::types::LogLevel::Info));
        assert_eq!(
            window.find("logs-levels").label(),
            Some("Levels: 4 of 5 shown")
        );
    })
    .unwrap();
}

/// Hides the Error level from the Levels menu: 118 of the example's 120
/// lines show.
fn hide_errors(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("logs-levels", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// Opens the Download menu, checks its items' words and clicks `item`.
fn download(cx: &mut TestAppContext, handle: WindowHandle<Root>, words: [&str; 2], item: usize) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("logs-download", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let mut menu = window.within("popup-menu");
        for (ix, words) in words.into_iter().enumerate() {
            assert_eq!(menu.find(ix).label(), Some(words));
        }
        menu.click(item, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn download_saves_the_visible_lines_as_copy_copies_them(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("visible.log");
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    hide_errors(cx, handle);
    let copied = cx.update(|cx| {
        panel.update(cx, |view, cx| {
            view.select_all(cx);
            let copied = view.review.copy_text(view.copy_as()).unwrap();
            view.review.selected.clear();
            copied
        })
    });
    download(
        cx,
        handle,
        ["Visible lines (118)", "All retained lines (120)"],
        0,
    );
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|folder| {
        // The Downloads folder, else home.
        assert_eq!(folder, super::download::default_folder());
        Some(path.clone())
    });
    cx.run_until_parked();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert_eq!(saved, format!("{copied}\n"));
    assert!(!saved.contains("error needle"));
    let feedback = cx.update(|cx| panel.read(cx).feedback.clone()).unwrap();
    assert!(!feedback.failed);
    // One line names the file; its tooltip, the folder too.
    assert_eq!(feedback.text.as_ref(), "Saved 118 lines to visible.log");
    assert_eq!(
        feedback.whole.as_ref(),
        format!("Saved 118 lines to {}", path.display())
    );
    // Only the file is left: no temporary beside it.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[gpui_kit::test]
fn download_saves_every_retained_line_and_cancel_saves_nothing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("all.log");
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    hide_errors(cx, handle);
    download(
        cx,
        handle,
        ["Visible lines (118)", "All retained lines (120)"],
        1,
    );
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    cx.run_until_parked();
    let saved = std::fs::read_to_string(&path).unwrap();
    // The lines the filters hide are saved too.
    assert_eq!(saved.lines().count(), 120);
    assert!(saved.starts_with("info ordinary complete line 0\n"));
    assert!(saved.contains("error needle first 東京\n"));

    cx.update(|cx| panel.update(cx, |view, _| view.feedback = None));
    download(
        cx,
        handle,
        ["Visible lines (118)", "All retained lines (120)"],
        0,
    );
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| panel.read(cx).feedback.clone()), None);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[gpui_kit::test]
fn a_download_that_cant_be_written_says_so_and_leaves_no_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing").join("lines.log");
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    download(
        cx,
        handle,
        ["Visible lines (120)", "All retained lines (120)"],
        0,
    );
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    cx.run_until_parked();
    let feedback = cx.update(|cx| panel.read(cx).feedback.clone()).unwrap();
    assert!(feedback.failed);
    assert!(
        feedback.text.starts_with("Couldn't save lines.log: "),
        "{}",
        feedback.text
    );
    assert!(
        feedback
            .whole
            .starts_with(&format!("Couldn't save {}: ", path.display())),
        "{}",
        feedback.whole
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("logs-summary-text").visible());
    })
    .unwrap();
}

/// While a save dialog is open, another Download does nothing: the first
/// save goes through with the lines it was asked for, and once it has
/// answered Download asks again.
#[gpui_kit::test]
fn a_second_download_waits_for_the_open_save_dialog(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("first.log");
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    download(
        cx,
        handle,
        ["Visible lines (120)", "All retained lines (120)"],
        0,
    );
    assert!(cx.did_prompt_for_new_path());
    deliver_more(cx, &panel, 0, 5);
    download(
        cx,
        handle,
        ["Visible lines (125)", "All retained lines (125)"],
        1,
    );
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    cx.run_until_parked();
    assert!(!cx.did_prompt_for_new_path());
    let saved = std::fs::read_to_string(&path).unwrap();
    assert_eq!(saved.lines().count(), 120);
    assert!(!saved.contains("later line"));

    download(
        cx,
        handle,
        ["Visible lines (125)", "All retained lines (125)"],
        1,
    );
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
}

/// The menu counts its lines each time it opens, however it last closed.
#[gpui_kit::test]
fn the_download_menu_counts_again_each_time_it_opens(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount(cx);
    settle(cx, &panel, handle);
    let labels = |cx: &mut TestAppContext| {
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("logs-download", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let menu = window.within("popup-menu");
            [0, 1].map(|ix| menu.find(ix).label().unwrap_or_default().to_owned())
        })
        .unwrap()
    };
    assert_eq!(
        labels(cx),
        ["Visible lines (120)", "All retained lines (120)"]
    );
    // Closed by its button.
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("logs-download", cx);
    })
    .unwrap();
    cx.run_until_parked();
    deliver_more(cx, &panel, 0, 5);
    assert_eq!(
        labels(cx),
        ["Visible lines (125)", "All retained lines (125)"]
    );
    // Closed by Escape.
    cx.update_window(handle.into(), |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    deliver_more(cx, &panel, 5, 3);
    assert_eq!(
        labels(cx),
        ["Visible lines (128)", "All retained lines (128)"]
    );
}

/// Under 40 rem the toolbar shows its buttons' icons alone; the menus'
/// carets still fit beside them.
#[gpui_kit::test]
fn icon_only_menu_buttons_keep_room_for_their_caret(cx: &mut TestAppContext) {
    let (_runtime, panel, handle) = mount_sized(cx, 480., 760.);
    settle(cx, &panel, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("logs-levels").label().is_some());
        for id in ["logs-levels", "logs-download"] {
            let bounds = window.find(id).bounds();
            assert!(
                bounds.size.width > bounds.size.height * 1.4,
                "{id}: {:?}",
                bounds.size
            );
        }
    })
    .unwrap();
}
