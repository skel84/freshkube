//! Keeping a file that could not be parsed, and telling the person once.
//!
//! A copy is made byte for byte beside the file, as
//! `<name>.corrupt-<UTC time>.json`, before anything overwrites it. A copy is
//! never overwritten, the newest [`KEEP`] are kept, and a file whose bytes
//! equal the newest copy's is not copied again.
use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// How many copies of one file are kept.
pub const KEEP: usize = 3;

/// Something to tell the person about a settings file, once per start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub file: PathBuf,
    /// The copy of a file that could not be parsed. `None`: the file was
    /// written by a newer Freshkube and is left as it is.
    pub kept_as: Option<PathBuf>,
}

impl fmt::Display for Notice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = |path: &Path| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        };
        match &self.kept_as {
            Some(copy) => write!(
                f,
                "{} could not be read; the old file is kept as {}",
                name(&self.file),
                name(copy)
            ),
            None => write!(
                f,
                "{} was written by a newer Freshkube; it is not used or changed",
                name(&self.file)
            ),
        }
    }
}

static NOTICES: Mutex<(Vec<Notice>, Vec<Notice>)> = Mutex::new((Vec::new(), Vec::new()));

/// Queues `notice` unless this run has told it already.
pub(crate) fn tell(notice: Notice) {
    let mut notices = NOTICES.lock().unwrap_or_else(|p| p.into_inner());
    if !notices.1.contains(&notice) {
        notices.1.push(notice.clone());
        notices.0.push(notice);
    }
}

/// The notices queued since the last call. A file read again later gives
/// the same notice no second time.
pub fn take_notices() -> Vec<Notice> {
    std::mem::take(&mut NOTICES.lock().unwrap_or_else(|p| p.into_inner()).0)
}

/// Copies `bytes`, the contents of `file`, aside and queues the notice.
/// Returns the copy's path.
pub(crate) fn copy(file: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    copy_at(file, bytes, SystemTime::now())
}

pub(crate) fn copy_at(file: &Path, bytes: &[u8], now: SystemTime) -> std::io::Result<PathBuf> {
    let stem = file.file_stem().unwrap_or_default().to_string_lossy();
    let prefix = format!("{stem}.corrupt-");
    let mut copies = copies_of(file, &prefix);
    let kept_as = match copies.last() {
        Some(newest) if std::fs::read(newest).is_ok_and(|old| old == bytes) => newest.clone(),
        _ => {
            let stamp = chrono::DateTime::<chrono::Utc>::from(now).format("%Y%m%dT%H%M%SZ");
            let mut made = None;
            for n in 0.. {
                let suffix = if n == 0 {
                    String::new()
                } else {
                    format!(".{n}")
                };
                let path = file.with_file_name(format!("{prefix}{stamp}{suffix}.json"));
                match create(&path, bytes) {
                    Ok(()) => {
                        made = Some(path);
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
            }
            let made = made.expect("the loop returns or breaks");
            copies.push(made.clone());
            // Names sort by time, so the oldest come first.
            for old in copies.drain(..copies.len().saturating_sub(KEEP)) {
                let _ = std::fs::remove_file(old);
            }
            made
        }
    };
    tell(Notice {
        file: file.to_path_buf(),
        kept_as: Some(kept_as.clone()),
    });
    Ok(kept_as)
}

/// The copies of `file` beside it, oldest first. A name without its
/// `.json` sorts by time: `…Z` before `…Z.1` before the next second.
fn copies_of(file: &Path, prefix: &str) -> Vec<PathBuf> {
    let directory = file.parent().filter(|p| !p.as_os_str().is_empty());
    let Ok(entries) = std::fs::read_dir(directory.unwrap_or(Path::new("."))) else {
        return Vec::new();
    };
    let mut found: Vec<(String, PathBuf)> = entries
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().to_string_lossy().into_owned();
            let key = name.strip_prefix(prefix)?.strip_suffix(".json")?.to_owned();
            Some((key, file.with_file_name(name)))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, path)| path).collect()
}

/// Creates `path`, which must not exist, private to the owner.
fn create(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}
