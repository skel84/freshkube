//! Passive frame sampling. The shell's render marks when a frame starts and
//! painting when it ends; only this small indicator is notified once a
//! second, and only when its reading changes.
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui_kit::component::{h_flex, tooltip::Tooltip};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::{
    palette::palette,
    ui::{self, Tone, dp},
};

const SAMPLE_PERIOD: Duration = Duration::from_secs(1);
/// A frame that starts this soon after the last paint was wanted while that
/// one drew: scrolling, an animation, a flood of lines. The gap between the
/// two paints is then the frame rate.
const BACK_TO_BACK: Duration = Duration::from_millis(20);
/// A frame whose own work, from the shell's render to this paint, takes
/// longer than this counts however long the app waited before it, so slow
/// frames read red without input. Only sparse, quick frames read idle.
const SLOW_FRAME: Duration = Duration::from_millis(33);
/// The counted time a second needs before it reads anything: about nine
/// frames at 60 FPS. A followed log's few pairs of frames, a new line
/// measured and then drawn, stay idle. A slow frame reads whatever the
/// time, so a lone hitch shows.
const ACTIVE: Duration = Duration::from_millis(150);

#[derive(Default)]
struct Frames {
    started: Option<Instant>,
    last: Option<Instant>,
    elapsed: Duration,
    intervals: u32,
    slow: bool,
}

impl Frames {
    fn start(&mut self, now: Instant) {
        self.started = Some(now);
    }

    fn paint(&mut self, now: Instant) {
        let started = self.started.take();
        let Some(last) = self.last.replace(now) else {
            return;
        };
        let gap = now.saturating_duration_since(last);
        let work = started.map(|started| now.saturating_duration_since(started));
        #[cfg(feature = "stress")]
        {
            crate::perf::value("frame.gap", gap.as_secs_f64() * 1000.);
            if let Some(work) = work {
                crate::perf::value("frame.cpu", work.as_secs_f64() * 1000.);
            }
        }
        // A followed log redraws every 100 ms or so in a few milliseconds:
        // sparse, quick frames say nothing about the frame rate. Ignore
        // duplicate paints at one instant in headless tests, too.
        let waited = started.map_or(gap, |started| started.saturating_duration_since(last));
        let slow = work.is_some_and(|work| work > SLOW_FRAME);
        self.slow |= slow;
        let interval = match work {
            _ if waited < BACK_TO_BACK => gap,
            Some(work) if slow => work,
            _ => return,
        };
        if !interval.is_zero() {
            self.elapsed += interval;
            self.intervals += 1;
        }
    }

    fn sample(&mut self) -> Option<u32> {
        let fps = (self.elapsed >= ACTIVE || self.slow)
            .then(|| (f64::from(self.intervals) / self.elapsed.as_secs_f64()).round() as u32);
        self.elapsed = Duration::ZERO;
        self.intervals = 0;
        self.slow = false;
        fps
    }
}

fn tone(fps: Option<u32>) -> Tone {
    match fps {
        Some(55..) => Tone::Good,
        Some(30..55) => Tone::Warn,
        Some(_) => Tone::Crit,
        None => Tone::Unknown,
    }
}

pub(in crate::desktop) struct Fps {
    frames: Rc<RefCell<Frames>>,
    fps: Option<u32>,
    label: SharedString,
    _task: Task<()>,
}

impl Fps {
    pub(in crate::desktop) fn new(cx: &mut Context<Self>) -> Self {
        let task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SAMPLE_PERIOD).await;
                if this
                    .update(cx, |this: &mut Self, cx| {
                        let fps = this.frames.borrow_mut().sample();
                        // A stress run records each reading, idle as 0.
                        #[cfg(feature = "stress")]
                        crate::perf::value("frame.meter", f64::from(fps.unwrap_or(0)));
                        if this.fps != fps {
                            this.fps = fps;
                            this.label = fps.map_or_else(
                                || "FPS idle".into(),
                                |fps| format!("{fps} FPS").into(),
                            );
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            frames: Rc::default(),
            fps: None,
            label: "FPS idle".into(),
            _task: task,
        }
    }

    /// Marks the start of a frame; the shell renders first in every frame.
    pub(in crate::desktop) fn frame_started(&self, now: Instant) {
        self.frames.borrow_mut().start(now);
    }
}

impl Render for Fps {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let tone = tone(self.fps);
        let color = match tone {
            Tone::Good => p.good_ink,
            Tone::Warn => p.warn_ink,
            Tone::Crit => p.crit_ink,
            _ => p.muted,
        };
        let frames = self.frames.clone();
        // Keep this entity uncached: its canvas observes every painted frame,
        // even when the expensive page views reuse their cached scene.
        h_flex()
            .id("frame-rate")
            .test_support()
            .aria_label(self.label.clone())
            .relative()
            .flex_none()
            .w(dp(78.))
            .gap(dp(5.))
            .text_color(color)
            .children(ui::status_glyph(tone, cx))
            .child(self.label.clone())
            .child(canvas(|_, _, _| {}, move |_, _, _, cx| {
                frames.borrow_mut().paint(cx.background_executor().now());
            }).absolute().size_full())
            .tooltip(|window, cx| Tooltip::new(
                "Frame rate while drawing back to back or slowly: green 55+, amber 30–54, red below 30."
            ).build(window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{Fps, Frames, SAMPLE_PERIOD, tone};
    use crate::ui::Tone;
    use gpui_kit::{AppContext, TestAppContext, component::Root, px, size, test::TestWindowExt};
    use std::{
        cell::Cell,
        rc::Rc,
        time::{Duration, Instant},
    };

    /// Paints ten frames `every` apart, each taking `work`, and samples them.
    fn sampled(every: u64, work: u64) -> Option<u32> {
        let mut frames = Frames::default();
        let start = Instant::now();
        frames.paint(start);
        for n in 1..=10 {
            let painted = start + Duration::from_millis(n * every);
            frames.start(painted - Duration::from_millis(work));
            frames.paint(painted);
        }
        frames.sample()
    }

    #[test]
    fn frames_drawn_back_to_back_read_their_rate() {
        for (every, expected, color) in [
            (16, 63, Tone::Good),
            (25, 40, Tone::Warn),
            (50, 20, Tone::Crit),
            (80, 13, Tone::Crit),
        ] {
            // Each frame starts a millisecond after the last one painted.
            let fps = sampled(every, every - 1);
            assert_eq!(fps, Some(expected));
            assert_eq!(tone(fps), color);
        }
        assert_eq!(tone(None), Tone::Unknown);
        assert_eq!(tone(Some(55)), Tone::Good);
        assert_eq!(tone(Some(30)), Tone::Warn);
    }

    #[test]
    fn a_sparse_quick_followed_log_reads_idle() {
        // Lines arrive every 100 ms and each frame takes 6 ms (#312).
        assert_eq!(sampled(100, 6), None);
        // A held key repeating every 30 ms, drawn in 5 ms.
        assert_eq!(sampled(30, 5), None);
        // Duplicate paints at one instant, as headless tests draw.
        assert_eq!(sampled(0, 0), None);
        // Five lines in a second, each measured and then drawn: five pairs of
        // frames 17 ms apart.
        let mut frames = Frames::default();
        let start = Instant::now();
        for n in 0..5 {
            let first = start + Duration::from_millis(n * 200);
            frames.start(first - Duration::from_millis(6));
            frames.paint(first);
            frames.start(first + Duration::from_millis(11));
            frames.paint(first + Duration::from_millis(17));
        }
        assert_eq!(frames.intervals, 5);
        assert_eq!(frames.sample(), None);
    }

    #[test]
    fn slow_frames_without_input_read_red() {
        // A frame every 100 ms that takes 60 ms of it, waiting 40 ms first.
        let fps = sampled(100, 60);
        assert_eq!(fps, Some(17));
        assert_eq!(tone(fps), Tone::Crit);
        // Slower than the old 250 ms cut: a 400 ms frame after a pause.
        let mut frames = Frames::default();
        let start = Instant::now();
        frames.paint(start);
        for n in 1..=3 {
            let painted = start + Duration::from_millis(n * 500);
            frames.start(painted - Duration::from_millis(400));
            frames.paint(painted);
        }
        assert_eq!(frames.sample(), Some(3));
        assert_eq!(frames.sample(), None);
    }

    #[test]
    fn one_slow_frame_in_a_quiet_second_reads_red() {
        let mut frames = Frames::default();
        let start = Instant::now();
        frames.paint(start);
        frames.start(start + Duration::from_millis(400));
        frames.paint(start + Duration::from_millis(520));
        let fps = frames.sample();
        assert_eq!(fps, Some(8));
        assert_eq!(tone(fps), Tone::Crit);
        // Two frames just over the line read too.
        frames.start(start + Duration::from_millis(1100));
        frames.paint(start + Duration::from_millis(1134));
        frames.start(start + Duration::from_millis(1500));
        frames.paint(start + Duration::from_millis(1534));
        assert_eq!(frames.sample(), Some(29));
        assert_eq!(frames.sample(), None);
    }

    #[test]
    fn a_hitch_every_second_reads_red_every_second() {
        // A timer that redraws once a second in 50 ms, among quick frames
        // a followed log draws every 100 ms.
        let mut frames = Frames::default();
        let start = Instant::now();
        frames.paint(start);
        for second in 0..5 {
            for tick in 1..=10 {
                let painted = start + Duration::from_millis(second * 1000 + tick * 100);
                let work = if tick == 5 { 50 } else { 6 };
                frames.start(painted - Duration::from_millis(work));
                frames.paint(painted);
            }
            let fps = frames.sample();
            assert_eq!(fps, Some(20), "second {second}");
            assert_eq!(tone(fps), Tone::Crit);
        }
    }

    #[gpui_kit::test]
    fn the_shell_marks_each_frame_start(cx: &mut TestAppContext) {
        let (_runtime, handle, view) = crate::desktop::tests::fixture(cx, 1280., 880.);
        let frames = view.read_with(cx, |view, cx| view.fps.read(cx).frames.clone());
        // A stale start 90 ms ago would make the next frame a slow one; the
        // shell's render replaces it with the frame's own, a quick one.
        let now = cx.executor().now();
        frames.borrow_mut().last = Some(now - Duration::from_millis(100));
        frames.borrow_mut().start(now - Duration::from_millis(90));
        let before = frames.borrow().intervals;
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        assert_eq!(frames.borrow().intervals, before);
        assert!(frames.borrow().started.is_none());
        // Counts the intervals four frames add, reading around each frame so
        // the meter's own sampling between them doesn't matter.
        let draw = |cx: &mut TestAppContext, every: u64| {
            let mut counted = 0;
            for _ in 0..4 {
                cx.executor().advance_clock(Duration::from_millis(every));
                let before = frames.borrow().intervals;
                cx.update_window(handle, |_, window, cx| window.render_frame(cx))
                    .unwrap();
                counted += frames.borrow().intervals - before;
            }
            counted
        };
        draw(cx, 16);
        // Headless frames take no time: sparse ones read idle, close ones count.
        assert_eq!(draw(cx, 100), 0);
        assert_eq!(draw(cx, 16), 4);
    }

    #[gpui_kit::test]
    fn status_bar_observes_paints_and_an_idle_tick_requests_no_frame(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            cx.set_reduce_motion(true);
        });
        let indicator = cx.new(Fps::new);
        let handle = cx.open_window(size(px(420.), px(80.)), |window, cx| {
            Root::new(indicator.clone(), window, cx)
        });
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.find("frame-rate");
            assert!(indicator.read(cx).frames.borrow().last.is_some());
        })
        .unwrap();
        cx.run_until_parked();
        let notices = Rc::new(Cell::new(0));
        let seen = notices.clone();
        let _subscription =
            cx.update(|cx| cx.observe(&indicator, move |_, _| seen.set(seen.get() + 1)));
        let last = cx.update(|cx| indicator.read(cx).frames.borrow().last);
        cx.executor().advance_clock(SAMPLE_PERIOD);
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            assert_eq!(window.simulate_next_frame(cx), 0);
            assert_eq!(indicator.read(cx).fps, None);
            assert_eq!(indicator.read(cx).frames.borrow().last, last);
        })
        .unwrap();
        assert_eq!(notices.get(), 0);
        // The first frame follows an idle second; ten intervals of 16 ms read.
        for _ in 0..11 {
            cx.executor().advance_clock(Duration::from_millis(16));
            cx.update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
            })
            .unwrap();
        }
        cx.executor().advance_clock(SAMPLE_PERIOD);
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            assert_eq!(indicator.read(cx).fps, Some(63));
            window.render_frame(cx);
            assert_eq!(window.find("frame-rate").label(), Some("63 FPS"));
        })
        .unwrap();
        assert_eq!(notices.get(), 1);
        cx.executor().advance_clock(SAMPLE_PERIOD);
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| indicator.read(cx).fps), None);
        assert_eq!(notices.get(), 2);
    }

    #[gpui_kit::test]
    fn frame_rate_fits_the_small_window_at_largest_text_size(cx: &mut TestAppContext) {
        let (_runtime, handle, _view) = crate::desktop::tests::fixture(cx, 760., 560.);
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(20., cx);
            window.render_frame(cx);
            let bounds = window.find("frame-rate").bounds();
            assert!(bounds.left() >= px(0.));
            assert!(bounds.right() <= window.viewport_size().width);
            assert!(bounds.bottom() <= window.viewport_size().height);
        })
        .unwrap();
    }
}
