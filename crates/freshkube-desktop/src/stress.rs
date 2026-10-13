//! Drives and reports a run of the `stress` binary: a key script, a
//! main-thread stall monitor, and once a second the timings `crate::perf`
//! collected, with CPU and memory. Built only with the `stress` feature.
//!
//! The knobs are read by the `stress` binary alone into a [`StressOptions`]
//! ([`install`]ed before the app starts); nothing here reads the environment.
//!
//! - `FRESHKUBE_STRESS_KEYS`: space-separated keystrokes as GPUI parses them
//!   (`down`, `enter`, `secondary-}`), `wait:<ms>` to pause, and
//!   `type:<text>` to type into whatever has focus. `hover:<x0>,<y0>,<x1>,<y1>,<ms>`
//!   moves the mouse in a straight line over that long, a step a frame, and
//!   `scroll:<x>,<y>,<dy>` scrolls there; positions are fractions of the
//!   window, so a script fits any window size.
//! - `FRESHKUBE_STRESS_SECONDS`: quit after this long (default 30).
//! - `FRESHKUBE_STRESS_WARMUP`: seconds left out of the summary (default 5).
//! - `FRESHKUBE_STRESS_APPEARANCE`: `light` or `dark` for the terminal window.
//! - `FRESHKUBE_STRESS_TALOS_RATE`, `FRESHKUBE_STRESS_WORKLOAD_RATE`: lines a
//!   second of example Talos and workload logs.
//!
//! Output goes to stderr, numbers only: `perf <second> <name> …` lines, then
//! `summary <name> …` lines over the run after the warm-up.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui_kit::{Context, Keystroke, Window};

use crate::perf;

/// How often the stall monitor asks to run. Anything later than this is time
/// the main thread spent on something else.
const STALL_TICK: Duration = Duration::from_millis(4);

/// The window a terminal run asks for, from `FRESHKUBE_STRESS_APPEARANCE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

/// The `FRESHKUBE_STRESS_*` knobs, as plain data. A missing or bad value takes
/// the default it always had.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StressOptions {
    /// `FRESHKUBE_STRESS_KEYS`: the key script; empty when unset or not UTF-8.
    pub keys: String,
    /// `FRESHKUBE_STRESS_SECONDS`: quit after this long.
    pub seconds: u64,
    /// `FRESHKUBE_STRESS_WARMUP`: seconds left out of the summary.
    pub warmup: u64,
    /// `FRESHKUBE_STRESS_APPEARANCE`: anything but `light` or `dark` leaves
    /// the theme alone.
    pub appearance: Option<Appearance>,
    /// `FRESHKUBE_STRESS_TALOS_RATE`: a positive number, else none.
    pub talos_rate: Option<u32>,
    /// `FRESHKUBE_STRESS_WORKLOAD_RATE`: a positive number, else none.
    pub workload_rate: Option<u32>,
}

impl Default for StressOptions {
    fn default() -> Self {
        Self::parse(|_| None)
    }
}

impl StressOptions {
    /// Reads the knobs from the process environment. Only the `stress` binary
    /// calls this.
    pub fn from_env() -> Self {
        Self::parse(|name| std::env::var_os(name))
    }

    /// Parses the knobs from `var`, which answers a name with its value.
    pub fn parse(var: impl Fn(&str) -> Option<OsString>) -> Self {
        let text = |name: &str| var(name).and_then(|value| value.into_string().ok());
        let number = |name: &str, default: u64| {
            text(name)
                .and_then(|value| value.parse().ok())
                .unwrap_or(default)
        };
        let rate = |name: &str| {
            text(name)
                .and_then(|rate| rate.parse().ok())
                .filter(|rate| *rate > 0)
        };
        Self {
            keys: text("FRESHKUBE_STRESS_KEYS").unwrap_or_default(),
            seconds: number("FRESHKUBE_STRESS_SECONDS", 30),
            warmup: number("FRESHKUBE_STRESS_WARMUP", 5),
            appearance: match text("FRESHKUBE_STRESS_APPEARANCE").as_deref() {
                Some("light") => Some(Appearance::Light),
                Some("dark") => Some(Appearance::Dark),
                _ => None,
            },
            talos_rate: rate("FRESHKUBE_STRESS_TALOS_RATE"),
            workload_rate: rate("FRESHKUBE_STRESS_WORKLOAD_RATE"),
        }
    }
}

static OPTIONS: OnceLock<StressOptions> = OnceLock::new();

/// Installs the run's options; the first call wins. The log floods sit under
/// the example sources, so one set-once value stands in for a parameter on
/// each of them, as core's `Ambient` does.
pub fn install(options: StressOptions) {
    let _ = OPTIONS.set(options);
}

fn options() -> StressOptions {
    OPTIONS.get().cloned().unwrap_or_default()
}

pub(crate) fn start<V: 'static>(window: &mut Window, cx: &mut Context<V>) {
    let StressOptions {
        keys,
        seconds,
        warmup,
        ..
    } = options();

    cx.spawn_in(window, async move |_, cx| {
        for step in keys.split_whitespace() {
            if let Some(ms) = step.strip_prefix("wait:") {
                let ms = ms.parse().unwrap_or(0);
                cx.background_executor()
                    .timer(Duration::from_millis(ms))
                    .await;
                continue;
            }
            if let Some(args) = step.strip_prefix("hover:") {
                let args = numbers(args);
                if let [x0, y0, x1, y1, ms] = args[..] {
                    let steps = (ms / 16.).max(1.) as usize;
                    for n in 0..=steps {
                        let t = n as f32 / steps as f32;
                        let _ = cx.update(|window, cx| {
                            let size = window.viewport_size();
                            let position = gpui_kit::point(
                                size.width * (x0 + (x1 - x0) * t),
                                size.height * (y0 + (y1 - y0) * t),
                            );
                            window.dispatch_event(
                                gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                                    position,
                                    pressed_button: None,
                                    modifiers: gpui_kit::Modifiers::default(),
                                }),
                                cx,
                            );
                        });
                        cx.background_executor()
                            .timer(Duration::from_millis(16))
                            .await;
                    }
                }
                continue;
            }
            if let Some(args) = step.strip_prefix("scroll:") {
                if let [x, y, dy] = numbers(args)[..] {
                    let _ = cx.update(|window, cx| {
                        let size = window.viewport_size();
                        window.dispatch_event(
                            gpui_kit::PlatformInput::ScrollWheel(gpui_kit::ScrollWheelEvent {
                                position: gpui_kit::point(size.width * x, size.height * y),
                                delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                                    gpui_kit::px(0.),
                                    gpui_kit::px(dy),
                                )),
                                modifiers: gpui_kit::Modifiers::default(),
                                touch_phase: gpui_kit::TouchPhase::Moved,
                            }),
                            cx,
                        );
                    });
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                }
                continue;
            }
            let strokes: Vec<String> = match step.strip_prefix("type:") {
                Some(text) => text.chars().map(String::from).collect(),
                None => vec![step.to_owned()],
            };
            for stroke in strokes {
                let _ = cx.update(|window, cx| {
                    if let Ok(keystroke) = Keystroke::parse(&stroke) {
                        let handled = window.dispatch_keystroke(keystroke, cx);
                        eprintln!("stress key {stroke} handled={handled}");
                    }
                });
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
            }
        }
    })
    .detach();

    cx.spawn(async move |_, cx| {
        let mut last = Instant::now();
        loop {
            cx.background_executor().timer(STALL_TICK).await;
            let now = Instant::now();
            let late = now.duration_since(last).saturating_sub(STALL_TICK);
            perf::value("main.stall", late.as_secs_f64() * 1000.);
            last = now;
        }
    })
    .detach();

    cx.spawn(async move |_, cx| {
        let started = Instant::now();
        let mut kept: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
        let mut usage = Usage::read();
        let mut cpu = Vec::new();
        for second in 1..=seconds {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let now = Usage::read();
            let busy = now.cpu.saturating_sub(usage.cpu).as_secs_f64();
            let share = 100. * busy / now.at.duration_since(usage.at).as_secs_f64();
            usage = now;
            for (name, samples) in perf::drain() {
                eprintln!("perf {second} {name} {}", stats(&samples));
                if second > warmup {
                    kept.entry(name).or_default().extend(samples);
                }
            }
            eprintln!(
                "perf {second} process cpu={share:.0}% rss={:.0}MB",
                usage.resident as f64 / 1e6
            );
            if second > warmup {
                cpu.push(share);
            }
        }
        for (name, samples) in &kept {
            eprintln!("summary {name} {}", stats(samples));
        }
        eprintln!(
            "summary process cpu {} rss_end={:.0}MB rss_max={:.0}MB seconds={:.1}",
            stats(&cpu),
            usage.resident as f64 / 1e6,
            usage.resident_max as f64 / 1e6,
            started.elapsed().as_secs_f64()
        );
        cx.update(|cx| cx.quit());
    })
    .detach();
}

fn numbers(text: &str) -> Vec<f32> {
    text.split(',').filter_map(|n| n.parse().ok()).collect()
}

/// `n=… p50=… p99=… max=…`, and the sum when it is a time.
fn stats(samples: &[f64]) -> String {
    if samples.is_empty() {
        return "n=0".into();
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    format!(
        "n={} p50={:.2} p99={:.2} max={:.2} sum={:.0}",
        sorted.len(),
        at(0.5),
        at(0.99),
        sorted[sorted.len() - 1],
        sorted.iter().sum::<f64>()
    )
}

/// The process's CPU time and memory, from the kernel.
struct Usage {
    at: Instant,
    cpu: Duration,
    resident: u64,
    resident_max: u64,
}

impl Usage {
    // libc points to the `mach2` crate for Mach calls; one call doesn't
    // warrant another dependency.
    #[cfg(target_os = "macos")]
    #[allow(deprecated)]
    fn read() -> Self {
        let mut info: libc::mach_task_basic_info = unsafe { std::mem::zeroed() };
        let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
        // SAFETY: `info` is a `mach_task_basic_info` and `count` its size in
        // words, as `MACH_TASK_BASIC_INFO` expects.
        let status = unsafe {
            libc::task_info(
                libc::mach_task_self(),
                libc::MACH_TASK_BASIC_INFO,
                (&raw mut info).cast(),
                &mut count,
            )
        };
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: `usage` is a valid `rusage` to fill.
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        let time =
            |value: libc::timeval| Duration::new(value.tv_sec as u64, value.tv_usec as u32 * 1000);
        let (resident, resident_max) = if status == 0 {
            (info.resident_size, info.resident_size_max)
        } else {
            (0, 0)
        };
        Self {
            at: Instant::now(),
            cpu: time(usage.ru_utime) + time(usage.ru_stime),
            resident,
            resident_max,
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn read() -> Self {
        Self {
            at: Instant::now(),
            cpu: Duration::ZERO,
            resident: 0,
            resident_max: 0,
        }
    }
}

/// What the terminal-only window draws.
#[derive(Clone, Copy, Debug)]
pub enum TerminalWorkload {
    /// Coloured log lines at this many a second, sent every 10 ms.
    Flood { lines_per_second: u32 },
    /// A `top`-like redraw of the whole screen, about 60 times a second.
    Top,
    /// One screen of colours, styles and wide characters, for looking at.
    Sample,
}

/// Opens a window with only a terminal and feeds it `workload` from another
/// thread, as an exec stream would arrive. `FRESHKUBE_STRESS_APPEARANCE`
/// (`light` or `dark`) picks the appearance; a debug build also takes
/// `FRESHKUBE_TEXT_SIZE`.
pub(crate) fn run_terminal(
    workload: TerminalWorkload,
    debug: crate::DebugOptions,
) -> color_eyre::Result<()> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use futures::StreamExt;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::{AppContext, Focusable, TitlebarOptions, WindowBounds, WindowOptions, px, size};

    use crate::terminal::{TerminalEvent, TerminalView, streams};

    let appearance = options().appearance;
    gpui_kit::application()
        .with_assets(crate::theme::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install_with(None, debug.text_size, cx);
            match appearance {
                Some(Appearance::Light) => Theme::change(ThemeMode::Light, None, cx),
                Some(Appearance::Dark) => Theme::change(ThemeMode::Dark, None, cx),
                None => {}
            }
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1150.), px(790.)), cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Freshkube terminal".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let Ok((_, view)) = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    let view = TerminalView::new(window, cx);
                    view.focus_handle(cx).focus(window, cx);
                    start(window, cx);
                    view
                })
            }) else {
                return cx.quit();
            };
            cx.activate(true);

            // The producer follows the grid's size, as a program would.
            let grid = Arc::new(AtomicU32::new(0));
            let size = view.read(cx).size();
            grid.store(
                u32::from(size.columns) << 16 | u32::from(size.rows),
                Ordering::Relaxed,
            );
            let seen = grid.clone();
            cx.subscribe(&view, move |_, event: &TerminalEvent, _| {
                if let TerminalEvent::Resize(size) = event {
                    seen.store(
                        u32::from(size.columns) << 16 | u32::from(size.rows),
                        Ordering::Relaxed,
                    );
                }
            })
            .detach();

            let (sender, mut receiver) = futures::channel::mpsc::unbounded::<Vec<u8>>();
            std::thread::spawn(move || {
                let send = |bytes| sender.unbounded_send(bytes).is_ok();
                match workload {
                    TerminalWorkload::Flood { lines_per_second } => {
                        let per_tick = (lines_per_second as usize / 100).max(1);
                        let mut from = 0;
                        while send(streams::coloured(from, per_tick)) {
                            from += per_tick;
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    }
                    TerminalWorkload::Top => {
                        for frame in 0.. {
                            let size = grid.load(Ordering::Relaxed);
                            let (columns, rows) = ((size >> 16) as usize, (size & 0xffff) as usize);
                            if !send(streams::top_frame(frame, columns, rows)) {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(16));
                        }
                    }
                    TerminalWorkload::Sample => {
                        send(streams::sample());
                    }
                }
            });
            cx.spawn(async move |cx| {
                while let Some(bytes) = receiver.next().await {
                    view.update(cx, |view, cx| {
                        view.feed(&bytes, cx);
                        // Whatever else has arrived joins the same frame.
                        while let Ok(bytes) = receiver.try_recv() {
                            view.feed(&bytes, cx);
                        }
                    });
                }
            })
            .detach();
        });
    Ok(())
}

/// `FRESHKUBE_STRESS_TALOS_RATE`: lines a second for example Talos logs.
pub(crate) fn talos_rate() -> Option<u32> {
    options().talos_rate
}

/// Writes `rate` example lines a second, spread over `services` and stamped
/// with the time each was due, so a backlog anywhere shows as lag.
pub(crate) async fn talos_flood(
    rate: u32,
    sender: tokio::sync::mpsc::Sender<crate::backend::StreamEvent>,
    target: crate::backend::Target,
    services: Vec<freshkube_core::logs::ServiceId>,
) {
    let started_at = chrono::Local::now();
    let started = Instant::now();
    let mut sent = 0u64;
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    loop {
        tick.tick().await;
        let due = (started.elapsed().as_secs_f64() * f64::from(rate)) as u64;
        while sent < due {
            let at = started_at
                + chrono::Duration::nanoseconds((sent as f64 * 1e9 / f64::from(rate)) as i64);
            let service = services[sent as usize % services.len()].clone();
            let line = format!(
                "{} level=info msg=\"stress line\" service={} seq={sent}",
                at.format("%Y-%m-%dT%H:%M:%S%.6f%:z"),
                service.as_str()
            );
            let event = crate::backend::StreamEvent {
                target: target.clone(),
                service,
                result: Ok(line),
            };
            if sender.send(event).await.is_err() {
                return;
            }
            sent += 1;
        }
    }
}

/// The workload logs flood's lines a second, from `FRESHKUBE_STRESS_WORKLOAD_RATE`.
pub(crate) fn workload_rate() -> Option<u32> {
    options().workload_rate
}

/// Writes `rate` pod log lines a second, as Kubernetes stamps them, spread
/// round `streams` and stamped with the time each was due. `send` takes the
/// stream's index and the line, and answers false when nobody listens.
pub(crate) async fn line_flood<F, Fut>(rate: u32, streams: usize, send: F)
where
    F: Fn(usize, String) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let started_at = chrono::Utc::now();
    let started = Instant::now();
    let mut sent = 0u64;
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    loop {
        tick.tick().await;
        let due = (started.elapsed().as_secs_f64() * f64::from(rate)) as u64;
        while sent < due {
            let at = started_at
                + chrono::Duration::nanoseconds((sent as f64 * 1e9 / f64::from(rate)) as i64);
            let line = format!(
                "{} level=info msg=\"stress line\" seq={sent}",
                at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
            );
            if !send(sent as usize % streams, line).await {
                return;
            }
            sent += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(pairs: &'static [(&'static str, &'static str)]) -> StressOptions {
        StressOptions::parse(|name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    #[test]
    fn nothing_set_takes_the_defaults() {
        let options = parse(&[]);
        assert_eq!(options.keys, "");
        assert_eq!((options.seconds, options.warmup), (30, 5));
        assert_eq!(options.appearance, None);
        assert_eq!((options.talos_rate, options.workload_rate), (None, None));
        assert_eq!(options, StressOptions::default());
    }

    #[test]
    fn keys_are_taken_as_written() {
        assert_eq!(
            parse(&[("FRESHKUBE_STRESS_KEYS", "down wait:5")]).keys,
            "down wait:5"
        );
    }

    #[test]
    fn seconds_and_warmup_fall_back_when_bad() {
        let good = parse(&[
            ("FRESHKUBE_STRESS_SECONDS", "12"),
            ("FRESHKUBE_STRESS_WARMUP", "0"),
        ]);
        assert_eq!((good.seconds, good.warmup), (12, 0));
        for bad in ["", "soon", "-3", "1.5"] {
            let options = StressOptions::parse(|name| {
                matches!(name, "FRESHKUBE_STRESS_SECONDS" | "FRESHKUBE_STRESS_WARMUP")
                    .then(|| OsString::from(bad))
            });
            assert_eq!((options.seconds, options.warmup), (30, 5), "{bad:?}");
        }
    }

    #[test]
    fn appearance_knows_light_and_dark_only() {
        let appearance = |value: &'static str| {
            StressOptions::parse(|name| {
                (name == "FRESHKUBE_STRESS_APPEARANCE").then(|| OsString::from(value))
            })
            .appearance
        };
        assert_eq!(appearance("light"), Some(Appearance::Light));
        assert_eq!(appearance("dark"), Some(Appearance::Dark));
        assert_eq!(appearance("Light"), None);
        assert_eq!(appearance(""), None);
    }

    #[test]
    fn rates_must_be_positive_numbers() {
        let rates = |value: &'static str| {
            let options = StressOptions::parse(|name| {
                matches!(
                    name,
                    "FRESHKUBE_STRESS_TALOS_RATE" | "FRESHKUBE_STRESS_WORKLOAD_RATE"
                )
                .then(|| OsString::from(value))
            });
            (options.talos_rate, options.workload_rate)
        };
        assert_eq!(rates("500"), (Some(500), Some(500)));
        for bad in ["0", "-1", "fast", "", "2.5"] {
            assert_eq!(rates(bad), (None, None), "{bad:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_value_that_is_not_utf8_counts_as_unset() {
        use std::os::unix::ffi::OsStringExt;
        let options = StressOptions::parse(|_| Some(OsString::from_vec(vec![0xff, 0xfe])));
        assert_eq!(options, StressOptions::default());
    }
}
