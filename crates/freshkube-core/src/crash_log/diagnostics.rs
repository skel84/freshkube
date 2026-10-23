//! The text Copy diagnostics puts on the clipboard: what a person can paste
//! into a report without checking it for secrets first.

use super::marker::CrashReport;
use super::redact::redact;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// How many of the log's last lines are copied.
pub const TAIL_LINES: usize = 200;

/// How much of the log's end is read to find those lines.
const TAIL_BYTES: u64 = 256 * 1024;

/// The longest a single line is copied; a longer one is cut.
const LINE_MAX: usize = 2000;

/// The last lines of a log, redacted and cut to a bounded size. A missing log
/// answers no lines.
pub fn tail(path: &Path, lines: usize) -> Vec<String> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map_or(0, |meta| meta.len());
    let from = len.saturating_sub(TAIL_BYTES);
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(from)).is_err() || file.read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut all: Vec<&str> = text.lines().collect();
    if from > 0 && !all.is_empty() {
        // The first line starts before what was read.
        all.remove(0);
    }
    let start = all.len().saturating_sub(lines);
    all[start..]
        .iter()
        .map(|line| {
            let line = redact(line);
            match line.char_indices().nth(LINE_MAX) {
                Some((at, _)) => format!("{}…", &line[..at]),
                None => line,
            }
        })
        .collect()
}

/// The diagnostics: version, system, the last crash if there was one, and the
/// end of `log`. Everything in it has been through [`redact`].
pub fn text(version: &str, log: &Path, crash: Option<&CrashReport>) -> String {
    let mut out = String::new();
    out.push_str("Freshkube diagnostics\n");
    out.push_str(&format!("Version: {version}\n"));
    out.push_str(&format!(
        "System: {} {}\n",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    out.push_str(&format!("Time: {}\n", chrono::Utc::now().to_rfc3339()));
    match crash {
        Some(crash) => {
            out.push_str("\nThe last session ended unexpectedly\n");
            out.push_str(&format!("Version: {}\n", crash.version));
            out.push_str(&format!("Started: {}\n", crash.started));
            match &crash.panic {
                Some(message) => {
                    out.push_str(&format!("Panic: {}\n", redact(message)));
                    if let Some(at) = &crash.panicked_at {
                        out.push_str(&format!("Panicked at: {at}\n"));
                    }
                    if let Some(location) = &crash.location {
                        out.push_str(&format!("Location: {}\n", redact(location)));
                    }
                    if let Some(thread) = &crash.thread {
                        out.push_str(&format!("Thread: {}\n", redact(thread)));
                    }
                }
                None => out.push_str("No panic was recorded: the app stopped without one.\n"),
            }
        }
        None => out.push_str("\nNo crash was recorded.\n"),
    }
    let name = log
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let lines = tail(log, TAIL_LINES);
    out.push_str(&format!("\nLast {} lines of {name}\n", lines.len()));
    for line in lines {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_names_the_version_the_system_and_the_panic() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("freshkube.previous.log");
        std::fs::write(&log, "INFO started\nERROR panic: boom\n").unwrap();
        let crash = CrashReport::started("0.19.0", &log).with_panic("boom", None, "main");
        let out = text("0.20.0", &log, Some(&crash));
        assert!(out.contains("Version: 0.20.0") && out.contains("Version: 0.19.0"));
        assert!(out.contains(std::env::consts::OS) && out.contains(std::env::consts::ARCH));
        assert!(out.contains("Panic: boom") && out.contains("ERROR panic: boom"));
        assert!(
            out.contains("freshkube.previous.log") && !out.contains(dir.path().to_str().unwrap())
        );
    }

    #[test]
    fn only_the_last_lines_are_copied() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("l.log");
        let body: String = (0..500).map(|n| format!("line {n}\n")).collect();
        std::fs::write(&log, body).unwrap();
        let lines = tail(&log, TAIL_LINES);
        assert_eq!(lines.len(), TAIL_LINES);
        assert_eq!(lines.last().unwrap(), "line 499");
        assert_eq!(lines[0], "line 300");
    }

    #[test]
    fn a_missing_log_is_no_lines_and_a_long_line_is_cut() {
        let dir = tempfile::tempdir().unwrap();
        assert!(tail(&dir.path().join("none.log"), 5).is_empty());
        let log = dir.path().join("l.log");
        std::fs::write(&log, "x".repeat(LINE_MAX * 3)).unwrap();
        assert!(tail(&log, 5)[0].chars().count() <= LINE_MAX + 1);
    }

    #[test]
    fn a_seeded_token_is_not_in_the_copied_text() {
        let seed = "s3cr3tTokenValue9f8e7d6c5b4a";
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("freshkube.log");
        std::fs::write(
            &log,
            format!("INFO login token={seed}\nERROR Authorization: Bearer {seed}\n"),
        )
        .unwrap();
        let mut crash = CrashReport::started("1", &log);
        // A report from an older build, or a hand-edited one, is not trusted.
        crash.panic = Some(format!("bad password={seed}"));
        crash.location = Some(format!("secret={seed}"));
        let out = text("1", &log, Some(&crash));
        assert!(!out.contains(seed), "{out}");
        assert!(out.contains("INFO login"));
    }
}
