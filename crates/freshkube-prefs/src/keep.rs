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
    pub why: Why,
}

/// What happened to the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    /// It could not be parsed; this is the copy of it.
    Corrupt { kept_as: PathBuf },
    /// A newer Freshkube wrote it, at this version. It is not used, and a
    /// save over it is refused.
    Newer { version: u64 },
}

impl fmt::Display for Notice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = |path: &Path| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        };
        match &self.why {
            Why::Corrupt { kept_as } => write!(
                f,
                "{} could not be read; the old file is kept as {}",
                name(&self.file),
                name(kept_as)
            ),
            Why::Newer { version } => write!(
                f,
                "{} was written by a newer Freshkube (version {version}); it is not used, \
                 and changes to it are not saved",
                name(&self.file)
            ),
        }
    }
}

type Wake = Box<dyn Fn() + Send + Sync>;

/// The notices not yet taken, those told this run, and who to wake.
static NOTICES: Mutex<(Vec<Notice>, Vec<Notice>)> = Mutex::new((Vec::new(), Vec::new()));
static WAKE: Mutex<Option<Wake>> = Mutex::new(None);

/// Calls `wake` whenever a notice is queued, from the thread that queued it,
/// so the app can take it (and tell it) at once and not only at launch. One
/// watcher: a later call replaces the earlier. `wake` must not call
/// [`take_notices`] itself.
pub fn watch(wake: impl Fn() + Send + Sync + 'static) {
    *WAKE.lock().unwrap_or_else(|p| p.into_inner()) = Some(Box::new(wake));
}

/// Queues `notice` unless this run has told it already.
pub(crate) fn tell(notice: Notice) {
    {
        let mut notices = NOTICES.lock().unwrap_or_else(|p| p.into_inner());
        if notices.1.contains(&notice) {
            return;
        }
        notices.1.push(notice.clone());
        notices.0.push(notice);
    }
    if let Some(wake) = &*WAKE.lock().unwrap_or_else(|p| p.into_inner()) {
        wake();
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
            let stamp = chrono::DateTime::<chrono::Utc>::from(now)
                .format("%Y%m%dT%H%M%SZ")
                .to_string();
            // A name sorts after every copy there is, even when the one that
            // would be next in this second was rotated away, so the newest
            // never reads as the oldest.
            let (mut stamp, mut n) = (stamp, 0u32);
            if let Some((newest_stamp, newest_n)) = copies.last().and_then(|p| order_of(p, &prefix))
                && (newest_stamp.as_str(), newest_n) >= (stamp.as_str(), 0)
            {
                (stamp, n) = (newest_stamp, newest_n.saturating_add(1));
            }
            let made = loop {
                let suffix = if n == 0 {
                    String::new()
                } else {
                    format!(".{n}")
                };
                let path = file.with_file_name(format!("{prefix}{stamp}{suffix}.json"));
                match create(&path, bytes) {
                    Ok(()) => break path,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
                    Err(error) => return Err(error),
                }
            };
            copies.push(made.clone());
            // Copies are in time order, so the oldest come first.
            for old in copies.drain(..copies.len().saturating_sub(KEEP)) {
                let _ = std::fs::remove_file(old);
            }
            made
        }
    };
    tell(Notice {
        file: file.to_path_buf(),
        why: Why::Corrupt {
            kept_as: kept_as.clone(),
        },
    });
    Ok(kept_as)
}

/// A copy's place in time, from its name alone: the UTC stamp, then the
/// number that told two apart within a second (none is 0). Not the file's
/// modification time, which is coarse on some file systems.
fn order_of(path: &Path, prefix: &str) -> Option<(String, u32)> {
    let name = path.file_name()?.to_string_lossy();
    let key = name.strip_prefix(prefix)?.strip_suffix(".json")?;
    Some(match key.split_once('.') {
        Some((stamp, n)) => (stamp.to_owned(), n.parse().ok()?),
        None => (key.to_owned(), 0),
    })
}

/// The copies of `file` beside it, oldest first: by the time in the name,
/// `…Z` before `…Z.1` before `…Z.10` before the next second.
pub(crate) fn copies_of(file: &Path, prefix: &str) -> Vec<PathBuf> {
    let directory = file.parent().filter(|p| !p.as_os_str().is_empty());
    let Ok(entries) = std::fs::read_dir(directory.unwrap_or(Path::new("."))) else {
        return Vec::new();
    };
    let mut found: Vec<((String, u32), PathBuf)> = entries
        .filter_map(|entry| {
            let path = file.with_file_name(entry.ok()?.file_name());
            Some((order_of(&path, prefix)?, path))
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
    // A copy that is only partly there would pass for a good one, and
    // rotation would keep it over an older whole one.
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    if written.is_err() {
        drop(file);
        let _ = std::fs::remove_file(path);
    }
    written
}
