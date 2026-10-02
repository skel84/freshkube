//! Drives and reports a run of the `stress` binary: a key script, a
//! main-thread stall monitor, and once a second the timings `crate::perf`
//! collected, with CPU and memory. Built only with the `stress` feature.
//!
//! - `FRESHKUBE_STRESS_KEYS`: space-separated keystrokes as GPUI parses them
//!   (`down`, `enter`, `secondary-}`), `wait:<ms>` to pause, and
//!   `type:<text>` to type into whatever has focus.
//! - `FRESHKUBE_STRESS_SECONDS`: quit after this long (default 30).
//! - `FRESHKUBE_STRESS_WARMUP`: seconds left out of the summary (default 5).
//!
//! Output goes to stderr, numbers only: `perf <second> <name> …` lines, then
//! `summary <name> …` lines over the run after the warm-up.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use gpui_kit::{Context, Keystroke, Window};

use crate::perf;

/// How often the stall monitor asks to run. Anything later than this is time
/// the main thread spent on something else.
const STALL_TICK: Duration = Duration::from_millis(4);

pub(crate) fn start<V: 'static>(window: &mut Window, cx: &mut Context<V>) {
    let seconds = env_u64("FRESHKUBE_STRESS_SECONDS", 30);
    let warmup = env_u64("FRESHKUBE_STRESS_WARMUP", 5);
    let keys = std::env::var("FRESHKUBE_STRESS_KEYS").unwrap_or_default();

    cx.spawn_in(window, async move |_, cx| {
        for step in keys.split_whitespace() {
            if let Some(ms) = step.strip_prefix("wait:") {
                let ms = ms.parse().unwrap_or(0);
                cx.background_executor()
                    .timer(Duration::from_millis(ms))
                    .await;
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

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
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
pub(crate) fn run_terminal(workload: TerminalWorkload) -> color_eyre::Result<()> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use futures::StreamExt;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::{AppContext, Focusable, TitlebarOptions, WindowBounds, WindowOptions, px, size};

    use crate::terminal::{TerminalEvent, TerminalView, streams};

    gpui_kit::application()
        .with_assets(crate::theme::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            match std::env::var("FRESHKUBE_STRESS_APPEARANCE").as_deref() {
                Ok("light") => Theme::change(ThemeMode::Light, None, cx),
                Ok("dark") => Theme::change(ThemeMode::Dark, None, cx),
                _ => {}
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
    std::env::var("FRESHKUBE_STRESS_TALOS_RATE")
        .ok()
        .and_then(|rate| rate.parse().ok())
        .filter(|rate| *rate > 0)
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
