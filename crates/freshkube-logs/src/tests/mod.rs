use super::{LogSource, LogView, review::Mark};
use freshkube_core::logs::{LogEvent, ServiceId};
use gpui_kit::{
    AnyElement, AppContext, Context, Entity, Focusable, InputEvent, InteractiveElement,
    IntoElement, ScrollDelta, SharedString, Styled, TestAppContext, TestSupportExt, Window,
    WindowHandle,
    component::{Root, Theme},
    point, px, size,
    test::TestWindowExt,
};
use std::collections::{BTreeMap, BTreeSet};
use tokio::runtime::{Builder, Handle, Runtime};

mod download;
mod follow;
mod levels;
mod line_menu;
mod measure;
mod search;
mod source;

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
