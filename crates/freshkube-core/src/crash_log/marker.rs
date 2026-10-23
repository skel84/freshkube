//! The crash marker: a small file written when a session starts and removed
//! when it quits cleanly. Finding one at the next launch means the last
//! session ended without a clean quit. A panic adds its message to it.

use super::redact::redact;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The marker's file name, in the app's preferences folder.
pub const FILE_NAME: &str = "crash-marker.json";

/// The longest panic message the marker keeps.
const MESSAGE_MAX: usize = 4000;

/// What is known about a session that did not end cleanly.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashReport {
    pub version: String,
    /// When the session started, RFC 3339.
    pub started: String,
    /// The log that session wrote (it is `previous` by now).
    pub log: PathBuf,
    /// The panic's message, already redacted. `None` when the session just
    /// stopped: killed, or out of memory.
    #[serde(default)]
    pub panic: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub thread: Option<String>,
    /// When the panic happened, RFC 3339.
    #[serde(default)]
    pub panicked_at: Option<String>,
}

impl CrashReport {
    /// A running session's marker, before any panic.
    pub fn started(version: &str, log: &Path) -> Self {
        Self {
            version: version.to_owned(),
            started: chrono::Utc::now().to_rfc3339(),
            log: log.to_path_buf(),
            ..Self::default()
        }
    }

    /// Notes a panic, redacting its words and cutting them short.
    pub fn with_panic(mut self, message: &str, location: Option<String>, thread: &str) -> Self {
        let mut message = redact(message);
        if message.len() > MESSAGE_MAX {
            let mut end = MESSAGE_MAX;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
            message.push('…');
        }
        self.panic = Some(message);
        self.location = location;
        self.thread = Some(thread.to_owned());
        self.panicked_at = Some(chrono::Utc::now().to_rfc3339());
        self
    }
}

/// Where the marker lives.
pub fn path_in(folder: &Path) -> PathBuf {
    folder.join(FILE_NAME)
}

/// Reads the marker a session left. A missing, unreadable or unparsable file
/// answers `None`: a broken marker must never stop a launch, or be taken for
/// a crash it can't describe.
pub fn read(path: &Path) -> Option<CrashReport> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Writes the marker through a temporary file, so a crash while writing
/// leaves the old one or the new one.
pub fn write(path: &Path, report: &CrashReport) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(report)?)?;
    fs::rename(&temporary, path)
}

/// Removes the marker: the session quit cleanly.
pub fn clear(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("could not clear the crash marker: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_round_trips_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        assert_eq!(read(&path), None);
        let report = CrashReport::started("1.2.3", Path::new("/t/freshkube.log")).with_panic(
            "boom",
            Some("src/x.rs:1:2".into()),
            "main",
        );
        write(&path, &report).unwrap();
        assert_eq!(read(&path), Some(report));
        clear(&path);
        assert_eq!(read(&path), None);
        clear(&path);
    }

    #[test]
    fn a_broken_marker_is_no_crash() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(read(&path), None);
    }

    #[test]
    fn a_marker_from_a_session_that_only_stopped_has_no_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        write(&path, &CrashReport::started("1", Path::new("l"))).unwrap();
        assert_eq!(read(&path).unwrap().panic, None);
    }

    #[test]
    fn the_panic_is_redacted_and_cut_short() {
        let report = CrashReport::default().with_panic("bad token=abc123secret", None, "t");
        assert!(!report.panic.unwrap().contains("abc123secret"));
        let long = "é".repeat(MESSAGE_MAX);
        let report = CrashReport::default().with_panic(&long, None, "t");
        assert!(report.panic.unwrap().len() <= MESSAGE_MAX + 4);
    }
}
