//! The crash log in the app (#953): the notice shown when the last session
//! ended unexpectedly, and the Reveal log and Copy diagnostics actions it
//! shares with Settings › About. The files, the marker and the redaction are
//! `freshkube_core::crash_log`; this draws and hands the clipboard and the
//! file manager what it says.

use super::*;
use crate::ui::dp;
use freshkube_core::crash_log::{CrashReport, SessionInfo};
use gpui_kit::component::notification::{Notification, NotificationType};
use gpui_kit::component::{Sizable, WindowExt as _, button::Button, h_flex, v_flex};

#[cfg(any(debug_assertions, feature = "stress"))]
use crate::debug_options::CrashShape;

/// The session that is running, for the notice and About.
#[derive(Clone)]
struct Current(SessionInfo);

impl Global for Current {}

/// Keeps the session the binary started, and with `FRESHKUBE_CRASH=notice`
/// in example data an invented crash when none was found.
pub(super) fn install(options: &GpuiOptions, cx: &mut App) {
    let Some(mut info) = options.crash_session.clone() else {
        // A window with no log of its own (tests, the stress binary): the
        // example data still has something to show.
        #[cfg(any(debug_assertions, feature = "stress"))]
        if options.starts_with_examples() && options.debug.crash == Some(CrashShape::Notice) {
            cx.set_global(Current(example_session()));
        }
        return;
    };
    #[cfg(any(debug_assertions, feature = "stress"))]
    if options.debug.crash == Some(CrashShape::Notice) && info.last_crash.is_none() {
        info.last_crash = Some(example_crash(&info.log));
    }
    #[cfg(not(any(debug_assertions, feature = "stress")))]
    let _ = &mut info;
    cx.set_global(Current(info));
}

/// The app is quitting: the session ends cleanly, which a quit that exits the
/// process without returning from `main` would not otherwise say.
pub(super) fn clean_quit(cx: &App) {
    if let Some(info) = session(cx) {
        info.clean_quit();
    }
}

/// Hands a test window the session a launch would have started.
#[cfg(test)]
pub(crate) fn set_session(info: SessionInfo, cx: &mut App) {
    cx.set_global(Current(info));
}

/// An invented session, for example data.
#[cfg(any(debug_assertions, feature = "stress"))]
fn example_session() -> SessionInfo {
    let log = std::env::temp_dir().join("freshkube.log");
    SessionInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        last_crash: Some(example_crash(&log)),
        log,
        marker: None,
    }
}

#[cfg(any(debug_assertions, feature = "stress"))]
fn example_crash(log: &std::path::Path) -> CrashReport {
    CrashReport::started(env!("CARGO_PKG_VERSION"), log).with_panic(
        "index out of bounds: the len is 3 but the index is 7",
        Some("crates/example/src/lib.rs:42:9".to_owned()),
        "main",
    )
}

/// The running session, when the window has one.
pub(crate) fn session(cx: &App) -> Option<SessionInfo> {
    cx.try_global::<Current>().map(|current| current.0.clone())
}

#[cfg(test)]
thread_local! {
    /// What `reveal_log` last asked the file manager to show: the test
    /// platform shows nothing.
    pub(crate) static REVEALED: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Shows the log in the file manager: the crashed session's after a crash.
pub(crate) fn reveal_log(window: &mut Window, cx: &mut App) {
    let Some(info) = session(cx) else {
        window.push_notification("This window keeps no log to show.", cx);
        return;
    };
    let path = info.log_to_reveal();
    if !path.exists() {
        window.push_notification("The log file isn't there any more.", cx);
        return;
    }
    // The test platform can't show a file; the test reads the path instead.
    #[cfg(test)]
    REVEALED.with(|revealed| *revealed.borrow_mut() = Some(path.clone()));
    #[cfg(not(test))]
    cx.reveal_path(&path);
}

/// Puts the redacted diagnostics on the clipboard.
pub(crate) fn copy_diagnostics(window: &mut Window, cx: &mut App) {
    let Some(info) = session(cx) else {
        window.push_notification("This window keeps no log to copy from.", cx);
        return;
    };
    cx.write_to_clipboard(ClipboardItem::new_string(info.diagnostics()));
    window.push_notification(
        "Diagnostics copied. They hold no credentials; read them before you share them.",
        cx,
    );
}

impl Pilot {
    /// Tells once, at launch, that the last session ended unexpectedly, and
    /// in a debug build forces the panic `FRESHKUBE_CRASH` asks for.
    pub(super) fn start_crash_log(
        &mut self,
        debug: &crate::DebugOptions,
        runtime: &tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(not(any(debug_assertions, feature = "stress")))]
        let _ = (debug, runtime);
        // A stress build compiles the trigger in, but
        // `DebugOptions::debug_only` clears the knob there: only a debug
        // build ever panics on request.
        // `FRESHKUBE_CRASH_SEED`: a credential-shaped value the log and the
        // forced panic carry, so a smoke can check the redaction on the real
        // path. It is never written anywhere unredacted.
        #[cfg(any(debug_assertions, feature = "stress"))]
        let seed = debug
            .crash_seed
            .as_deref()
            .map(|seed| format!(" password={seed}"))
            .unwrap_or_default();
        #[cfg(any(debug_assertions, feature = "stress"))]
        if let Some(seed) = &debug.crash_seed {
            tracing::info!("signing in with token={seed}");
        }
        #[cfg(any(debug_assertions, feature = "stress"))]
        match debug.crash {
            Some(CrashShape::Panic) => {
                cx.spawn(async move |_, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(3))
                        .await;
                    cx.update(|_| panic!("forced panic (FRESHKUBE_CRASH=panic){seed}"));
                })
                .detach();
            }
            Some(CrashShape::Task) => {
                runtime.spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    panic!("forced panic in a Tokio task (FRESHKUBE_CRASH=task){seed}");
                });
            }
            _ => {}
        }
        let Some(crash) = session(cx).and_then(|info| info.last_crash) else {
            return;
        };
        cx.defer_in(window, move |_, window, cx| {
            window.push_notification(crash_notice(&crash, cx), cx);
        });
    }
}

impl Pilot {
    /// Tells each settings file that was kept aside or left alone as a newer
    /// build's (`freshkube_prefs::take_notices`): those queued before the
    /// window opened at once, and any queued later (a file first read when
    /// its page opens, a save that is refused) as it happens.
    pub(super) fn tell_settings_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (wake, woken) = futures::channel::mpsc::unbounded();
        let _ = wake.unbounded_send(());
        // A test shares the process-wide queue with every other test.
        #[cfg(not(test))]
        {
            freshkube_prefs::watch(move || drop(wake.unbounded_send(())));
            self.watch_settings_files(woken, freshkube_prefs::take_notices, window, cx);
        }
        #[cfg(test)]
        let _ = (wake, woken, window, cx);
    }

    /// Takes the queue each time `woken` fires and tells what it holds.
    pub(super) fn watch_settings_files(
        &mut self,
        mut woken: futures::channel::mpsc::UnboundedReceiver<()>,
        take: impl Fn() -> Vec<freshkube_prefs::Notice> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use futures::StreamExt as _;
        cx.spawn_in(window, async move |this, cx| {
            while woken.next().await.is_some() {
                let notices = take();
                if this
                    .update_in(cx, |_, window, cx| tell_notices(notices, window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }
}

/// One warning per notice, kept until dismissed: it names a file and where
/// its copy went.
pub(super) fn tell_notices(
    notices: Vec<freshkube_prefs::Notice>,
    window: &mut Window,
    cx: &mut App,
) {
    for notice in notices {
        let key = notice.file.to_string_lossy().into_owned();
        window.push_notification(
            Notification::warning(notice.to_string())
                .id1::<SettingsFileNotice>(key)
                .autohide(false),
            cx,
        );
    }
}

/// The notice's identity, so a file is told once.
struct SettingsFileNotice;

/// The notice: what happened in a line, and the two actions.
fn crash_notice(crash: &CrashReport, cx: &App) -> Notification {
    let what = match &crash.panic {
        Some(message) => format!("It stopped on a panic: {message}"),
        None => "It stopped without leaving a reason.".to_owned(),
    };
    let muted = freshkube_ui::palette::palette(cx).muted;
    Notification::new()
        .id1::<CrashNotice>("crash-notice")
        .with_type(NotificationType::Warning)
        .title("The last session ended unexpectedly")
        .autohide(false)
        .content(move |_, _, _| {
            v_flex()
                .id("crash-notice")
                .test_support()
                .gap(dp(8.))
                .child(
                    div()
                        .id("crash-notice-what")
                        .test_support()
                        .text_color(muted)
                        .child(what.clone()),
                )
                .child(
                    h_flex()
                        .gap(dp(8.))
                        .child(
                            Button::new("crash-notice-reveal")
                                .outline()
                                .small()
                                .label("Reveal log")
                                .on_click(|_, window, cx| reveal_log(window, cx)),
                        )
                        .child(
                            Button::new("crash-notice-copy")
                                .outline()
                                .small()
                                .label("Copy diagnostics")
                                .on_click(|_, window, cx| copy_diagnostics(window, cx)),
                        ),
                )
                .into_any_element()
        })
}

/// The notice's identity, so it is told once.
struct CrashNotice;
