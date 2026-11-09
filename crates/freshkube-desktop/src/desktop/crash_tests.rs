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

fn corrupt(dir: &Path) -> freshkube_prefs::Notice {
    freshkube_prefs::Notice {
        file: dir.join("preferences.json"),
        why: freshkube_prefs::Why::Corrupt {
            kept_as: dir.join("preferences.corrupt-20261024T101500Z.json"),
        },
    }
}

#[gpui_kit::test]
fn a_settings_file_kept_aside_is_told_once_with_its_copy(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (_runtime, handle) = open(cx, session(dir.path(), None), None);
    assert_eq!(notices(cx, handle), 0);
    let notice = corrupt(dir.path());
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

/// A notice queued after the window is open (a file first read when its page
/// opens, a save over a newer file) is told when the queue wakes the
/// watcher, and not only at launch.
#[gpui_kit::test]
fn a_notice_queued_after_launch_is_told_when_the_queue_wakes(cx: &mut TestAppContext) {
    use std::cell::RefCell;
    use std::rc::Rc;
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| crash::set_session(session(dir.path(), None), cx));
    let (_runtime, handle, view) = mount(cx, GpuiOptions::fixture(), 1280., 880.);
    cx.run_until_parked();
    assert_eq!(notices(cx, handle), 0);

    let queue = Rc::new(RefCell::new(Vec::new()));
    let (wake, woken) = futures::channel::mpsc::unbounded();
    let taken = queue.clone();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.watch_settings_files(woken, move || taken.take(), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(notices(cx, handle), 0, "nothing queued, nothing told");

    queue.borrow_mut().push(corrupt(dir.path()));
    wake.unbounded_send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(notices(cx, handle), 1);

    queue.borrow_mut().push(freshkube_prefs::Notice {
        file: dir.path().join("follows.json"),
        why: freshkube_prefs::Why::Newer { version: 2 },
    });
    wake.unbounded_send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(notices(cx, handle), 2, "a later notice is told too");
}

/// In a narrow window at a large text size the kept copy's name sits on its
/// own line and breaks only between its parts, after a `-` or a `.`: every
/// part is drawn whole and inside the window.
#[gpui_kit::test]
fn the_kept_copys_name_breaks_only_between_its_parts(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| crash::set_session(session(dir.path(), None), cx));
    let (_runtime, handle, _view) = mount(cx, GpuiOptions::fixture(), 760., 560.);
    let name = "preferences.corrupt-20261024T101500Z.json";
    assert_eq!(
        crash::name_parts(name),
        ["preferences.", "corrupt-", "20261024T101500Z.", "json"]
    );
    for text_size in [18., 20.] {
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(text_size, cx);
            crash::tell_notices(vec![corrupt(dir.path())], window, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            let size = window.viewport_size();
            let mut rows = Vec::new();
            for ix in 0..4usize {
                let part = window.find(("settings-notice-copy-part", ix));
                let bounds = part.bounds();
                assert!(
                    bounds.left() >= gpui_kit::px(0.) && bounds.right() <= size.width,
                    "part {ix} at {text_size}: {bounds:?} in {size:?}"
                );
                // One line: a part is never itself wrapped.
                assert!(
                    bounds.size.height < gpui_kit::px(text_size * 2.),
                    "part {ix} at {text_size} is wrapped: {bounds:?}"
                );
                rows.push(bounds.top());
            }
            // The parts follow each other, so rows only go down.
            assert!(rows.windows(2).all(|pair| pair[0] <= pair[1]), "{rows:?}");
            // At the larger size the name is wider than the toast, so it
            // really does wrap.
            if text_size >= 20. {
                assert!(rows.last() > rows.first(), "{rows:?}");
            }
        })
        .unwrap();
    }
}

/// A file from a newer Freshkube is never written, so each section that says
/// where it saves says that its changes aren't, and why, as soon as the
/// notice is told.
#[gpui_kit::test]
fn a_newer_settings_file_makes_its_sections_say_changes_are_not_saved(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (_runtime, handle) = open(cx, session(dir.path(), None), None);
    let line = |cx: &mut TestAppContext, file: &'static str| {
        cx.update(|cx| {
            freshkube_ui::form::saved_in_text(&freshkube_ui::form::SavedIn::file(file), cx)
                .to_string()
        })
    };
    assert_eq!(line(cx, "preferences.json"), "Saved in preferences.json");
    let newer = |file: &str| freshkube_prefs::Notice {
        file: dir.path().join(file),
        why: freshkube_prefs::Why::Newer { version: 2 },
    };
    cx.update_window(handle, |_, window, cx| {
        crash::tell_notices(
            vec![newer("preferences.json"), newer("follows.json")],
            window,
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    for file in ["preferences.json", "follows.json"] {
        assert_eq!(
            line(cx, file),
            format!("Changes aren't saved: {file} was written by a newer Freshkube (version 2)")
        );
    }
    assert_eq!(line(cx, "monitoring.json"), "Saved in monitoring.json");
}
