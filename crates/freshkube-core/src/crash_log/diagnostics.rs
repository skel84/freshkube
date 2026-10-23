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

/// The system as diagnostics and About name it: `linux x86_64`.
pub fn system() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

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

/// What names a person's cluster in the log: the context a launch connected
/// with. Copied diagnostics get a placeholder; the file on disk keeps it.
const CONTEXT_LINES: [&str; 3] = ["Using context: ", "Using kubeconfig context: ", "Context: "];

/// The copied text with the context names and the home folder taken out:
/// people paste diagnostics into public issues.
fn scrub(text: &str, home: Option<&str>) -> String {
    text.split_inclusive('\n')
        .map(|line| {
            let line = match CONTEXT_LINES
                .iter()
                .find_map(|label| line.find(label).map(|at| at + label.len()))
            {
                Some(at) => {
                    let end = if line.ends_with('\n') {
                        line.len() - 1
                    } else {
                        line.len()
                    };
                    format!("{}<context>{}", &line[..at], &line[end..])
                }
                None => line.to_owned(),
            };
            match home.filter(|home| home.len() > 1) {
                Some(home) => line.replace(home, "~"),
                None => line,
            }
        })
        .collect()
}

/// The diagnostics: version, system, the last crash if there was one, and the
/// end of `log`. Everything in it has been through [`redact`], the context
/// names are `<context>` and the home folder is `~`.
pub fn text(version: &str, log: &Path, crash: Option<&CrashReport>) -> String {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok();
    text_at(version, log, crash, home.as_deref())
}

fn text_at(version: &str, log: &Path, crash: Option<&CrashReport>, home: Option<&str>) -> String {
    scrub(&build(version, log, crash), home)
}

fn build(version: &str, log: &Path, crash: Option<&CrashReport>) -> String {
    let mut out = String::new();
    out.push_str("Freshkube diagnostics\n");
    out.push_str(&format!("Version: {version}\n"));
    out.push_str(&format!("System: {}\n", system()));
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
        let out = text_at("0.20.0", &log, Some(&crash), None);
        assert!(out.contains("Version: 0.20.0") && out.contains("Version: 0.19.0"));
        assert!(out.contains(std::env::consts::OS) && out.contains(std::env::consts::ARCH));
        assert!(out.contains("Panic: boom") && out.contains("ERROR panic: boom"));
        assert!(
            out.contains("freshkube.previous.log") && !out.contains(dir.path().to_str().unwrap())
        );
    }

    #[test]
    fn the_context_and_the_home_folder_are_not_copied() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("freshkube.log");
        std::fs::write(
            &log,
            "INFO Using context: acme-prod-ams\nINFO Using kubeconfig context: acme-prod\n\
             INFO Using config: /home/jo/.talos/config\nINFO kept line\n",
        )
        .unwrap();
        let out = text_at("1", &log, None, Some("/home/jo"));
        assert!(!out.contains("acme"), "{out}");
        assert!(out.contains("Using context: <context>\n"), "{out}");
        assert!(
            out.contains("Using kubeconfig context: <context>\n"),
            "{out}"
        );
        assert!(out.contains("Using config: ~/.talos/config"), "{out}");
        assert!(out.contains("INFO kept line"));
        // The file on disk is untouched.
        assert!(
            std::fs::read_to_string(&log)
                .unwrap()
                .contains("acme-prod-ams")
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
