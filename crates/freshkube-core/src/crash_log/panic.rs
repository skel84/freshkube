//! The one panic hook. It writes the panic through tracing, notes it in the
//! crash marker, flushes the log, then runs the hook that was installed
//! before it, so the panic prints and ends the thread exactly as it did. A
//! panic in a Tokio task reaches the same hook: the runtime catches the
//! unwind only after the hook ran.

use super::Recorder;
use std::backtrace::Backtrace;
use std::io::Write;
use std::panic::{self, PanicHookInfo};
use std::sync::Arc;

/// Installs the hook over whatever hook is set now.
pub(super) fn install(recorder: Arc<Recorder>) {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        record(&recorder, info);
        previous(info);
    }));
}

/// The panic's message: its payload when that is text.
pub(super) fn message(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "a panic with a payload that is not text".to_owned()
    }
}

fn record(recorder: &Recorder, info: &PanicHookInfo<'_>) {
    let message = message(info);
    let location = info
        .location()
        .map(|at| format!("{}:{}:{}", at.file(), at.line(), at.column()));
    let current = std::thread::current();
    let thread = current.name().unwrap_or("<unnamed>");
    let backtrace = Backtrace::force_capture();
    tracing::error!(
        target: "panic",
        "panic on thread '{thread}' at {}: {message}\n{backtrace}",
        location.as_deref().unwrap_or("an unknown location"),
    );
    recorder.note_panic(&message, location, thread);
    let mut sink = recorder.sink.clone();
    _ = sink.flush();
}
