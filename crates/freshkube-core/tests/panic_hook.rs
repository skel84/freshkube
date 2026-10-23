//! The panic hook is process-wide, so this is its own test binary: a panic on
//! a thread and a panic in a Tokio task both reach the log and the marker, and
//! a seeded token reaches neither.

use freshkube_core::crash_log::{self, marker};
use tracing_subscriber::prelude::*;

// One test, in order: the hook is process-wide and each start chains another
// over it, so scenarios run in parallel would write each other's markers.
#[test]
fn the_hook_and_the_marker() {
    let (_kept, _also) = (the_next_launch_finds_what_a_crash_left(), ());
    panics_are_recorded_from_a_thread_and_from_a_tokio_task();
}

const SEED: &str = "s3cr3tTokenValue9f8e7d6c5b4a";

fn panics_are_recorded_from_a_thread_and_from_a_tokio_task() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("freshkube.log");
    let session = crash_log::start(&log, Some(dir.path()), "9.9.9").unwrap();
    let writer = session.writer();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_writer(move || writer.clone())
            .with_ansi(false),
    );
    // Global: the panicking thread is not this one.
    tracing::subscriber::set_global_default(subscriber).unwrap();
    // The running session's marker exists and names no panic.
    let running = marker::read(&marker::path_in(dir.path())).unwrap();
    assert_eq!((running.version.as_str(), running.panic), ("9.9.9", None));

    // The panic still ends the thread: the hook only records.
    let thread = std::thread::Builder::new()
        .name("worker".into())
        .spawn(|| panic!("thread went wrong, password={SEED}"))
        .unwrap();
    assert!(thread.join().is_err());

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let task = runtime.spawn(async { panic!("task went wrong") });
    assert!(runtime.block_on(task).unwrap_err().is_panic());

    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.contains("thread went wrong"), "{text}");
    assert!(text.contains("task went wrong"), "{text}");
    assert!(text.contains("thread 'worker'") && text.contains("panic_hook.rs"));
    assert!(!text.contains(SEED), "{text}");

    let marked = marker::read(&marker::path_in(dir.path())).unwrap();
    let message = marked.panic.unwrap();
    assert!(
        message.contains("task went wrong"),
        "the last panic is kept: {message}"
    );
    assert!(
        !std::fs::read_to_string(marker::path_in(dir.path()))
            .unwrap()
            .contains(SEED)
    );

    // A clean quit takes the marker away.
    session.finish();
    assert!(marker::read(&marker::path_in(dir.path())).is_none());
}

fn the_next_launch_finds_what_a_crash_left() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("freshkube.log");
    std::fs::write(&log, "INFO the crashed run\n").unwrap();
    let report = marker::CrashReport::started("1.0.0", &log).with_panic(
        &format!("boom token={SEED}"),
        Some("src/x.rs:1:1".into()),
        "main",
    );
    marker::write(&marker::path_in(dir.path()), &report).unwrap();

    let session = crash_log::start(&log, Some(dir.path()), "1.0.1").unwrap();
    let info = session.info();
    let found = info
        .last_crash
        .clone()
        .expect("the leftover marker is found");
    assert_eq!(found.version, "1.0.0");
    // The crashed run's log was kept, and the copy holds the panic, not the token.
    assert_eq!(
        std::fs::read_to_string(crash_log::rotate::generation(&log, 1)).unwrap(),
        "INFO the crashed run\n"
    );
    let text = info.diagnostics();
    assert!(text.contains("boom") && text.contains("the crashed run"));
    assert!(!text.contains(SEED), "{text}");
    // Starting wrote this session's own marker over it.
    assert_eq!(
        marker::read(&marker::path_in(dir.path())).unwrap().version,
        "1.0.1"
    );
    dir
}
