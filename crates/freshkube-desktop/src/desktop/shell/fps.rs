//! Passive frame sampling. Painting records a timestamp; only this small
//! indicator is notified once a second, and only when its reading changes.
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
const IDLE_GAP: Duration = Duration::from_millis(250);

#[derive(Default)]
struct Frames {
    last: Option<Instant>,
    elapsed: Duration,
    intervals: u32,
}

impl Frames {
    fn paint(&mut self, now: Instant) {
        if let Some(last) = self.last.replace(now) {
            let elapsed = now.saturating_duration_since(last);
            // Sparse event-driven redraws do not indicate slow animation.
            // Ignore duplicate paints at one instant in headless tests, too.
            if !elapsed.is_zero() && elapsed < IDLE_GAP {
                self.elapsed += elapsed;
                self.intervals += 1;
            }
        }
    }

    fn sample(&mut self) -> Option<u32> {
        let fps = (self.intervals >= 3)
            .then(|| (f64::from(self.intervals) / self.elapsed.as_secs_f64()).round() as u32);
        self.elapsed = Duration::ZERO;
        self.intervals = 0;
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
                "Rendered frames per second during activity: green 55+, amber 30–54, red below 30. Idle means redraws are sparse."
            ).build(window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{Fps, Frames, SAMPLE_PERIOD, tone};
    use crate::ui::Tone;
    use gpui_kit::{
        AppContext, TestAppContext,
        component::Root,
        px, size,
        test::{TestAppContextExt, TestWindowExt},
    };
    use std::{
        cell::Cell,
        rc::Rc,
        time::{Duration, Instant},
    };

    #[test]
    fn activity_colors_and_idle_do_not_confuse_sparse_redraws_with_slow_frames() {
        for (millis, expected, color) in [
            (16, 63, Tone::Good),
            (25, 40, Tone::Warn),
            (50, 20, Tone::Crit),
        ] {
            let mut frames = Frames::default();
            let start = Instant::now();
            for n in 0..=10 {
                frames.paint(start + Duration::from_millis(n * millis));
            }
            let fps = frames.sample();
            assert_eq!(fps, Some(expected));
            assert_eq!(tone(fps), color);
            assert_eq!(frames.sample(), None);
            frames.paint(start + Duration::from_secs(3));
            frames.paint(start + Duration::from_secs(4));
            assert_eq!(frames.sample(), None);
        }
        assert_eq!(tone(None), Tone::Unknown);
        assert_eq!(tone(Some(55)), Tone::Good);
        assert_eq!(tone(Some(30)), Tone::Warn);
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
        for _ in 0..4 {
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
