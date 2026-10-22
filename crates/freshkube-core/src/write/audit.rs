//! The audit log of the writes the app sends: `<preferences>/audit/writes.jsonl`,
//! one JSON object per line. It holds field paths, never values, and no
//! credentials or Secret values (`docs/WRITE_MODEL.md`, "The audit log").

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Protection, Tier};

/// The record layout's version.
pub const SCHEMA: u32 = 1;

/// A result message is cut to this many characters.
pub const RESULT_LIMIT: usize = 512;

/// The log rotates at this size.
pub const ROTATE_AT: u64 = 5 * 1024 * 1024;

/// Older files kept beside the current one.
pub const KEEP: usize = 3;

/// Where in a write's life a line was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Written before the request. If it can't be, nothing is sent.
    Sending,
    /// The API said yes. Not the same as the cluster getting there.
    Accepted,
    /// The API said no.
    Refused,
    /// A server-side apply conflict or a changed object.
    Conflict,
    /// The server-side dry run failed; nothing was written.
    DryRunFailed,
    /// The cluster reached the new state.
    Observed,
    /// The wait ended without the cluster reaching it.
    NotObserved,
    /// The request got no answer: not known whether the cluster took it.
    Unknown,
    /// Undo was offered.
    UndoOffered,
    /// Undo was sent.
    UndoSent,
    /// The person cancelled before the request.
    Cancelled,
}

/// An object a write reached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditObject {
    pub group: String,
    pub resource: String,
    pub namespace: Option<String>,
    pub name: String,
    pub uid: String,
    /// The field paths the request changes, never their values. For a
    /// Secret, the names of the keys.
    pub fields: Vec<String>,
}

/// The request's summary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRequest {
    pub sentence: String,
}

/// One line of the log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub schema: u32,
    pub at: DateTime<Utc>,
    /// Links the lines of one write.
    pub run: String,
    /// The cluster entry's id.
    pub entry: String,
    pub context: String,
    /// The OS user.
    pub actor: String,
    pub action: String,
    pub tier: String,
    /// The level at that moment.
    pub protection: String,
    pub objects: Vec<AuditObject>,
    pub request: AuditRequest,
    pub phase: Phase,
    pub result: String,
}

impl Record {
    /// The tier as a [`Tier`], if the line names a known one.
    pub fn tier_kind(&self) -> Option<Tier> {
        Tier::ALL.into_iter().find(|tier| tier.key() == self.tier)
    }

    pub fn level(&self) -> Option<Protection> {
        Protection::ALL
            .into_iter()
            .find(|level| level.key() == self.protection)
    }
}

/// A write's identity in the log. Consumed when the write is sent, so a run
/// is never sent twice.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RunId(String);

static COUNTER: AtomicU64 = AtomicU64::new(0);

impl RunId {
    pub fn new() -> Self {
        let millis = Utc::now().timestamp_millis();
        Self(format!(
            "{millis:x}-{:x}-{:x}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

/// Cuts an API error to what the log keeps: one line of at most
/// [`RESULT_LIMIT`] characters. Quoted values (`Invalid value: "…"`, which
/// echoes what was sent, such as an environment variable), bearer tokens,
/// `key=value` credentials and long opaque strings are replaced: an error
/// names the field and the reason, never the value.
pub fn scrub(message: &str) -> String {
    const NAMES: [&str; 5] = ["token", "password", "secret", "key", "authorization"];
    let message = strip_values(message);
    let mut words: Vec<String> = vec![];
    let mut redact_next = false;
    for word in message.split_whitespace() {
        let lower = word.to_ascii_lowercase();
        let bare = lower.trim_end_matches([':', '=']);
        let names_one = NAMES.contains(&bare) && bare.len() < lower.len();
        let pair = NAMES.iter().any(|name| {
            lower.starts_with(&format!("{name}=")) && lower.len() > name.len() + 1
                || lower.starts_with(&format!("{name}:")) && lower.len() > name.len() + 1
        });
        let opaque = word.len() >= 32
            && word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+/=_-.".contains(c))
            && word.chars().any(|c| c.is_ascii_digit());
        words.push(if redact_next || pair || opaque {
            "[redacted]".to_owned()
        } else {
            word.to_owned()
        });
        redact_next = names_one || lower == "bearer";
    }
    let line = words.join(" ");
    if line.chars().count() > RESULT_LIMIT {
        let mut cut: String = line.chars().take(RESULT_LIMIT - 1).collect();
        cut.push('…');
        cut
    } else {
        line
    }
}

/// Replaces what a validation message echoes: anything in double or single
/// quotes or backticks, and what follows `value:` (`Invalid value: 10:`,
/// `Unsupported value: map[a:b]`) up to the next `: ` or the end.
fn strip_values(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut chars = message.chars().peekable();
    while let Some(c) = chars.next() {
        if matches!(c, '"' | '\'' | '`') {
            // An apostrophe inside a word is not a quote.
            let word_apostrophe = c == '\''
                && out.chars().last().is_some_and(char::is_alphanumeric)
                && chars.peek().is_some_and(|n| n.is_alphabetic());
            if word_apostrophe {
                out.push(c);
                continue;
            }
            let mut closed = false;
            for inner in chars.by_ref() {
                if inner == c {
                    closed = true;
                    break;
                }
            }
            out.push_str(if closed { "[value]" } else { "[value" });
            continue;
        }
        out.push(c);
    }
    // `value: X` up to the next `: ` or the end.
    let mut result = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(at) = rest.find("value: ") {
        let (head, tail) = rest.split_at(at + "value: ".len());
        result.push_str(head);
        let end = tail.find(": ").unwrap_or(tail.len());
        let echoed = &tail[..end];
        if echoed.starts_with("[value]") || echoed.is_empty() {
            result.push_str(echoed);
        } else {
            result.push_str("[value]");
        }
        rest = &tail[end..];
    }
    result.push_str(rest);
    result
}

/// The store: a directory holding `writes.jsonl` and its rotated files.
#[derive(Clone, Debug)]
pub struct AuditStore {
    dir: PathBuf,
    rotate_at: u64,
}

static LOCK: Mutex<()> = Mutex::new(());

/// What a read of the log found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Read {
    /// Oldest first.
    pub records: Vec<Record>,
    /// Lines that were not a record this version reads.
    pub skipped: usize,
}

impl AuditStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            rotate_at: ROTATE_AT,
        }
    }

    /// A store that rotates at `bytes`, for a test.
    pub fn rotating_at(mut self, bytes: u64) -> Self {
        self.rotate_at = bytes;
        self
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join("writes.jsonl")
    }

    fn rotated(&self, n: usize) -> PathBuf {
        self.dir.join(format!("writes.jsonl.{n}"))
    }

    /// Appends one line, rotating first when the file would pass its size.
    /// An error means the line was not written.
    pub fn append(&self, record: &Record) -> io::Result<()> {
        let mut line = serde_json::to_string(record).map_err(io::Error::other)?;
        line.push('\n');
        let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        fs::create_dir_all(&self.dir)?;
        let path = self.path();
        let size = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        if size > 0 && size + line.len() as u64 > self.rotate_at {
            self.rotate()?;
        }
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&path)?;
        file.write_all(line.as_bytes())?;
        file.sync_data()
    }

    fn rotate(&self) -> io::Result<()> {
        let _ = fs::remove_file(self.rotated(KEEP));
        for n in (1..KEEP).rev() {
            if self.rotated(n).exists() {
                fs::rename(self.rotated(n), self.rotated(n + 1))?;
            }
        }
        fs::rename(self.path(), self.rotated(1))
    }

    /// Every record kept, oldest first. A missing log is an empty one.
    pub fn read(&self) -> io::Result<Read> {
        let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut read = Read::default();
        let files: Vec<PathBuf> = (1..=KEEP)
            .rev()
            .map(|n| self.rotated(n))
            .chain([self.path()])
            .collect();
        for file in files {
            let text = match fs::read_to_string(&file) {
                Ok(text) => text,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            for line in text.lines().filter(|line| !line.trim().is_empty()) {
                match serde_json::from_str::<Record>(line) {
                    Ok(record) if record.schema == SCHEMA => read.records.push(record),
                    _ => read.skipped += 1,
                }
            }
        }
        Ok(read)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(run: &str, phase: Phase, result: &str) -> Record {
        Record {
            schema: SCHEMA,
            at: Utc::now(),
            run: run.into(),
            entry: "prod-fra".into(),
            context: "ctx".into(),
            actor: "ops".into(),
            action: "scale".into(),
            tier: "reversible".into(),
            protection: "guarded".into(),
            objects: vec![AuditObject {
                group: "apps".into(),
                resource: "deployments".into(),
                namespace: Some("shop".into()),
                name: "web".into(),
                uid: "u-1".into(),
                fields: vec!["spec.replicas".into()],
            }],
            request: AuditRequest {
                sentence: "Scale web to 3".into(),
            },
            phase,
            result: result.into(),
        }
    }

    #[test]
    fn lines_round_trip_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let store = AuditStore::new(dir.path().join("audit"));
        assert_eq!(store.read().unwrap(), Read::default());
        for (n, phase) in [Phase::Sending, Phase::Accepted, Phase::Observed]
            .into_iter()
            .enumerate()
        {
            store
                .append(&record(&format!("r{n}"), phase, "ok"))
                .unwrap();
        }
        let read = store.read().unwrap();
        let runs: Vec<_> = read.records.iter().map(|r| r.run.as_str()).collect();
        assert_eq!(runs, ["r0", "r1", "r2"]);
        assert_eq!(read.records[1].phase, Phase::Accepted);
        let text = fs::read_to_string(store.path()).unwrap();
        assert_eq!(text.lines().count(), 3);
        assert!(text.contains("\"phase\":\"accepted\""));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = AuditStore::new(dir.path());
        store.append(&record("r", Phase::Sending, "")).unwrap();
        let mode = fs::metadata(store.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0);
    }

    #[test]
    fn rotation_keeps_three_older_files_and_drops_the_fourth() {
        let dir = tempfile::tempdir().unwrap();
        let one = serde_json::to_string(&record("r0", Phase::Sending, ""))
            .unwrap()
            .len() as u64
            + 1;
        let store = AuditStore::new(dir.path()).rotating_at(one * 2);
        for n in 0..9 {
            store
                .append(&record(&format!("r{n}"), Phase::Sending, ""))
                .unwrap();
        }
        let names: std::collections::BTreeSet<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            [
                "writes.jsonl",
                "writes.jsonl.1",
                "writes.jsonl.2",
                "writes.jsonl.3"
            ]
        );
        let runs: Vec<_> = store
            .read()
            .unwrap()
            .records
            .into_iter()
            .map(|r| r.run)
            .collect();
        // Oldest lines were dropped, order is kept, the newest is last.
        assert_eq!(runs.last().map(String::as_str), Some("r8"));
        assert!(!runs.contains(&"r0".to_owned()));
        assert!(runs.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn a_line_this_version_cannot_read_is_counted_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let store = AuditStore::new(dir.path());
        store.append(&record("r", Phase::Sending, "")).unwrap();
        let mut file = OpenOptions::new().append(true).open(store.path()).unwrap();
        writeln!(file, "not json").unwrap();
        writeln!(file, "{{\"schema\":99}}").unwrap();
        let read = store.read().unwrap();
        assert_eq!((read.records.len(), read.skipped), (1, 2));
    }

    #[test]
    fn an_unwritable_directory_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a-file");
        fs::write(&file, "x").unwrap();
        let store = AuditStore::new(file.join("inside"));
        assert!(store.append(&record("r", Phase::Sending, "")).is_err());
    }

    #[test]
    fn scrub_removes_credentials_and_cuts_long_text() {
        let scrubbed = scrub(
            "forbidden: Bearer abcdef.ghi token=xyz password: hunter2 \
             eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.sig1234567890abcdef used",
        );
        for leak in ["abcdef", "xyz", "hunter2", "eyJhbGci"] {
            assert!(!scrubbed.contains(leak), "{leak}: {scrubbed}");
        }
        assert!(scrubbed.starts_with("forbidden:"));
        assert!(scrubbed.ends_with("used"));
        let long = scrub(&"word ".repeat(400));
        assert_eq!(long.chars().count(), RESULT_LIMIT);
        assert!(long.ends_with('…'));
        assert!(!scrub("a\nb\tc").contains('\n'));
    }

    #[test]
    fn a_validation_error_names_the_field_and_never_the_value() {
        let message = "Deployment.apps \"web\" is invalid: spec.template.spec.containers[0].env[0].value: \
            Invalid value: \"s3cr3t-pass\": a valid value; spec.replicas: Invalid value: -1: must be non-negative; \
            spec.x: Unsupported value: map[token:abc]: supported values: \"a\", \"b\"";
        let scrubbed = scrub(message);
        for leak in ["s3cr3t-pass", "token:abc", "-1:"] {
            assert!(!scrubbed.contains(leak), "{leak}: {scrubbed}");
        }
        assert!(scrubbed.contains("env[0].value"), "{scrubbed}");
        assert!(scrubbed.contains("spec.replicas"), "{scrubbed}");
        assert!(scrubbed.contains("[value]"));
        // An apostrophe in a word is left alone.
        assert!(scrub("can't be empty").contains("can't"));
    }

    #[test]
    fn run_ids_are_unique() {
        assert_ne!(RunId::new(), RunId::new());
    }
}
