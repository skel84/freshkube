//! The crash notice and Settings › About (#953): a leftover marker shows the
//! notice, Copy diagnostics holds the panic and no secret, Reveal log names
//! the crashed session's log, and a clean start says nothing.
use crate::desktop::{crash, tests::mount};
use crate::{DebugOptions, GpuiOptions};
use freshkube_core::crash_log::{CrashReport, SessionInfo, rotate};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, TestAppContext};
use std::path::{Path, PathBuf};

const SEED: &str = "s3cr3tTokenValue9f8e7d6c5b4a";

/// A session whose last run left `crash`, with that run's log as `previous`
/// and a token in it, all under a fresh temporary folder.
fn session(dir: &Path, crash: Option<CrashReport>) -> SessionInfo {
    let log = dir.join("freshkube.log");
    std::fs::write(&log, "INFO this session\n").unwrap();
    std::fs::write(
        rotate::generation(&log, 1),
        format!("INFO the crashed session\nERROR login failed token={SEED}\n"),
    )
    .unwrap();
    SessionInfo {
        version: "1.0.0".into(),
        log,
        last_crash: crash,
        marker: None,
    }
}

fn crashed(dir: &Path) -> CrashReport {
    CrashReport::started("0.9.0", &dir.join("freshkube.previous.log")).with_panic(
        &format!("index out of bounds password={SEED}"),
        Some("src/lib.rs:1:1".into()),
        "main",
    )
}

fn open(
    cx: &mut TestAppContext,
    info: SessionInfo,
    page: Option<&str>,
) -> (tokio::runtime::Runtime, AnyWindowHandle) {
    cx.update(|cx| crash::set_session(info, cx));
    let options = GpuiOptions::fixture().with_debug(DebugOptions {
        page: page.map(str::to_owned),
        ..DebugOptions::default()
    });
    let (runtime, handle, _view) = mount(cx, options, 1280., 880.);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    (runtime, handle)
}

fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn notices(cx: &mut TestAppContext, handle: AnyWindowHandle) -> usize {
    cx.update_window(handle, |_, window, cx| window.notifications(cx).len())
        .unwrap()
}

fn copied(cx: &mut TestAppContext) -> String {
    cx.read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default()
}

fn revealed() -> Option<PathBuf> {
    crash::REVEALED.with(|revealed| revealed.borrow_mut().take())
}

#[gpui_kit::test]
fn a_leftover_marker_shows_the_notice_once(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let report = crashed(dir.path());
    let (_runtime, handle) = open(cx, session(dir.path(), Some(report)), None);
    assert_eq!(notices(cx, handle), 1);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("crash-notice-reveal").is_some());
        assert!(window.try_find("crash-notice-copy").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_clean_start_shows_no_notice(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (_runtime, handle) = open(cx, session(dir.path(), None), None);
    assert_eq!(notices(cx, handle), 0);
}

#[gpui_kit::test]
fn copy_diagnostics_holds_the_panic_and_no_secret(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let report = crashed(dir.path());
    let (_runtime, handle) = open(cx, session(dir.path(), Some(report)), None);
    click(cx, handle, "crash-notice-copy");
    let text = copied(cx);
    assert!(text.contains("index out of bounds"), "{text}");
    assert!(text.contains("the crashed session"), "{text}");
    assert!(!text.contains(SEED), "{text}");
}

#[gpui_kit::test]
fn reveal_log_shows_the_crashed_sessions_log(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let report = crashed(dir.path());
    let info = session(dir.path(), Some(report));
    let previous = rotate::generation(&info.log, 1);
    let (_runtime, handle) = open(cx, info, None);
    revealed();
    click(cx, handle, "crash-notice-reveal");
    assert_eq!(revealed(), Some(previous));
}

#[gpui_kit::test]
fn about_offers_the_same_actions_without_a_crash(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let info = session(dir.path(), None);
    let log = info.log.clone();
    let (_runtime, handle) = open(cx, info, Some("settings-about"));
    assert_eq!(notices(cx, handle), 0);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("about-version").is_some());
        assert!(window.try_find("about-reveal-log").is_some());
    })
    .unwrap();
    revealed();
    click(cx, handle, "about-reveal-log");
    assert_eq!(revealed(), Some(log));
    click(cx, handle, "about-copy-diagnostics");
    let text = copied(cx);
    assert!(text.contains("Freshkube diagnostics") && text.contains("this session"));
    assert!(text.contains("No crash was recorded"));
}

#[gpui_kit::test]
fn about_after_a_crash_copies_the_crashed_log_without_the_token(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let report = crashed(dir.path());
    let (_runtime, handle) = open(
        cx,
        session(dir.path(), Some(report)),
        Some("settings-about"),
    );
    click(cx, handle, "about-copy-diagnostics");
    let text = copied(cx);
    assert!(
        text.contains("The last session ended unexpectedly"),
        "{text}"
    );
    assert!(!text.contains(SEED), "{text}");
}

#[gpui_kit::test]
fn a_settings_file_kept_aside_is_told_once_with_its_copy(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (_runtime, handle) = open(cx, session(dir.path(), None), None);
    assert_eq!(notices(cx, handle), 0);
    let notice = freshkube_prefs::Notice {
        file: dir.path().join("preferences.json"),
        kept_as: Some(dir.path().join("preferences.corrupt-20261024T101500Z.json")),
    };
    for _ in 0..2 {
        cx.update_window(handle, |_, window, cx| {
            crash::tell_notices(vec![notice.clone()], window, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }
    assert_eq!(notices(cx, handle), 1, "one line, however often it is told");
    assert_eq!(
        notice.to_string(),
        "preferences.json could not be read; the old file is kept as preferences.corrupt-20261024T101500Z.json"
    );
}
