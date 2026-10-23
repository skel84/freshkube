//! The crash log: the log file kept across launches, panics written into it,
//! the marker that says the last session did not end cleanly, and the
//! redacted diagnostics a person can copy. No UI types; the desktop draws the
//! notice and the Settings › About actions from [`SessionInfo`].
//!
//! - [`rotate`]: `freshkube.log` → `freshkube.previous.log` → `freshkube.previous-2.log`.
//! - [`redact`]: credentials out of every line the log writes and every line copied.
//! - [`marker`]: written at start, removed on a clean quit, read at the next launch.
//! - [`diagnostics`]: the text Copy diagnostics holds.

pub mod diagnostics;
pub mod marker;
mod panic;
pub mod redact;
pub mod rotate;

pub use marker::CrashReport;
pub use redact::redact;

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The log file as the tracing subscriber writes it: every write goes through
/// [`redact`] first, so no credential reaches the disk.
#[derive(Clone)]
pub struct LogSink(Arc<Mutex<File>>);

impl LogSink {
    pub fn new(file: File) -> Self {
        Self(Arc::new(Mutex::new(file)))
    }
}

impl Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        let clean = redact(&text);
        // A poisoned lock still holds a usable file; the log is for crashes.
        let mut file = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // The ANSI codes redact removed are gone from the log too; a colour
        // code inside a field would hide it from the redaction.
        file.write_all(clean.as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).flush()
    }
}

/// What panics and the clean quit touch.
pub(crate) struct Recorder {
    marker: Option<PathBuf>,
    report: Mutex<CrashReport>,
    pub(crate) sink: LogSink,
}

impl Recorder {
    fn note_panic(&self, message: &str, location: Option<String>, thread: &str) {
        let Some(path) = &self.marker else { return };
        let mut report = self.report.lock().unwrap_or_else(|e| e.into_inner());
        *report = report.clone().with_panic(message, location, thread);
        _ = marker::write(path, &report);
    }
}

/// What the app needs of the session that just started.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionInfo {
    pub version: String,
    /// The log this session writes.
    pub log: PathBuf,
    /// The marker the last session left, when it did not quit cleanly.
    pub last_crash: Option<CrashReport>,
}

impl SessionInfo {
    /// The log to open for a problem: the crashed session's (now the previous
    /// log) when there was a crash and its file is still there, else this one.
    pub fn log_to_reveal(&self) -> PathBuf {
        let previous = rotate::generation(&self.log, 1);
        if self.last_crash.is_some() && previous.exists() {
            previous
        } else {
            self.log.clone()
        }
    }

    /// The text Copy diagnostics holds.
    pub fn diagnostics(&self) -> String {
        diagnostics::text(
            &self.version,
            &self.log_to_reveal(),
            self.last_crash.as_ref(),
        )
    }
}

/// A started session; [`Session::finish`] is the clean quit.
pub struct Session {
    info: SessionInfo,
    sink: LogSink,
    marker: Option<PathBuf>,
}

impl Session {
    /// The app-facing facts of the session.
    pub fn info(&self) -> SessionInfo {
        self.info.clone()
    }

    /// A writer for the tracing subscriber.
    pub fn writer(&self) -> LogSink {
        self.sink.clone()
    }

    /// The clean quit: the marker goes.
    pub fn finish(self) {
        drop(self);
    }
}

/// Any way out of `main` that is not a panic is a clean end, an early error
/// return included; a panic keeps the marker. A process that is killed (Ctrl-C
/// or SIGTERM from a terminal) never runs this and leaves it.
impl Drop for Session {
    fn drop(&mut self) {
        if let Some(path) = &self.marker
            && !std::thread::panicking()
        {
            marker::clear(path);
        }
    }
}

/// Starts a session: keeps the previous logs and opens a new one, reads the
/// marker a crashed session left, writes this session's marker and installs
/// the panic hook. `marker_folder` is `None` where nothing may be saved.
pub fn start(log: &Path, marker_folder: Option<&Path>, version: &str) -> io::Result<Session> {
    let sink = LogSink::new(rotate::open(log)?);
    let marker = marker_folder.map(marker::path_in);
    let last_crash = marker.as_deref().and_then(marker::read);
    let report = CrashReport::started(version, log);
    if let Some(path) = &marker
        && let Err(error) = marker::write(path, &report)
    {
        eprintln!("freshkube: could not write the crash marker: {error}");
    }
    panic::install(Arc::new(Recorder {
        marker: marker.clone(),
        report: Mutex::new(report),
        sink: sink.clone(),
    }));
    Ok(Session {
        info: SessionInfo {
            version: version.to_owned(),
            log: log.to_path_buf(),
            last_crash,
        },
        sink,
        marker,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_sink_writes_no_credential() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("freshkube.log");
        let mut sink = LogSink::new(File::create(&path).unwrap());
        writeln!(sink, "INFO sign-in token=s3cr3tTokenValue9f8e7d6c5b4a ok").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("s3cr3t") && text.contains("INFO sign-in"));
    }

    #[test]
    fn dropping_a_session_clears_the_marker_unless_it_is_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("freshkube.log");
        // Not `start`: it installs the process-wide hook.
        let session = |marker: PathBuf| Session {
            info: SessionInfo::default(),
            sink: LogSink::new(File::create(&log).unwrap()),
            marker: Some(marker),
        };
        let path = marker::path_in(dir.path());
        marker::write(&path, &CrashReport::default()).unwrap();
        drop(session(path.clone()));
        assert!(marker::read(&path).is_none(), "an early return is clean");

        marker::write(&path, &CrashReport::default()).unwrap();
        let held = session(path.clone());
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _held = held;
            panic!("unwinding with the session alive");
        }));
        assert!(unwound.is_err());
        assert!(marker::read(&path).is_some(), "a panic keeps the marker");
    }

    #[test]
    fn the_log_to_open_is_the_crashed_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("freshkube.log");
        std::fs::write(rotate::generation(&log, 1), "old\n").unwrap();
        let mut info = SessionInfo {
            version: "1".into(),
            log: log.clone(),
            last_crash: None,
        };
        assert_eq!(info.log_to_reveal(), log);
        info.last_crash = Some(CrashReport::default());
        assert_eq!(info.log_to_reveal(), rotate::generation(&log, 1));
    }
}
