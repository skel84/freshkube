//! The preferences files beside `preferences.json`: where they live and how
//! one is read and written. No GPUI and nothing of the app's own types, so
//! any crate may use it.
//!
//! A [`Dir`] is resolved once from the preferences path the app already uses
//! (`GpuiOptions.preferences`). It names each file in that directory, and a
//! [`Document`] loads and saves one of them as a typed value. A missing file
//! is the default. A file that can't be read or parsed is also the default,
//! and the load says why once, in [`Loaded::problem`], for the caller to log.
//! A save writes a temporary file in the same directory and renames it into
//! place, so a reader never sees half a file and a failed save leaves the old
//! one. A file that exists keeps its permissions; a new one takes the mode
//! its `Document` names (`0o644` unless [`Document::new_file_mode`] says
//! otherwise; Unix only). A `preferences.json` that is a symlink is written
//! through: the temporary file goes beside the link's target and replaces the
//! target, so the link stays.
//!
//! Which keys a file holds, and who may write them, stays with the file's
//! owner; this module owns only the reading and the writing.
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// How a document's JSON is laid out on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// One line, as `navigation.json` is written.
    Compact,
    /// Indented, as `preferences.json` is written.
    Pretty,
}

/// The directory the preferences files live in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dir(PathBuf);

impl Dir {
    /// The directory of `preferences`, the path of `preferences.json`.
    pub fn beside(preferences: &Path) -> Self {
        Self(
            preferences
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
        )
    }

    /// The file called `name` in this directory, holding a `T`.
    pub fn document<T>(&self, name: &str, layout: Layout) -> Document<T> {
        Document {
            path: self.0.join(name),
            layout,
            mode: 0o644,
            value: PhantomData,
        }
    }
}

/// One JSON file holding a `T`.
#[derive(Debug)]
pub struct Document<T> {
    path: PathBuf,
    layout: Layout,
    /// The mode a new file is created with, before the umask.
    mode: u32,
    value: PhantomData<fn() -> T>,
}

/// What a load found.
#[derive(Debug)]
pub struct Loaded<T> {
    /// The file's value, or the default when it is missing or unusable.
    pub value: T,
    /// Why the file was not used, when it exists but could not be read or
    /// parsed. A missing file is not a problem.
    pub problem: Option<String>,
}

impl<T: Serialize + DeserializeOwned + Default> Document<T> {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The mode a file that doesn't exist yet is created with, before the
    /// umask: `0o600` for a file that names a credential's place. Ignored
    /// off Unix. A file that exists keeps the mode it has.
    pub fn new_file_mode(mut self, mode: u32) -> Self {
        self.mode = mode;
        self
    }

    /// The file's value, read once.
    pub fn load(&self) -> Loaded<T> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Loaded {
                    value: T::default(),
                    problem: None,
                };
            }
            Err(error) => return self.unusable(format!("could not be read ({})", error.kind())),
        };
        match serde_json::from_slice(&bytes) {
            Ok(value) => Loaded {
                value,
                problem: None,
            },
            Err(error) => self.unusable(describe(&error)),
        }
    }

    fn unusable(&self, why: String) -> Loaded<T> {
        let problem = format!("{} {why}", self.path.display());
        tracing::warn!("{problem}; using the defaults");
        Loaded {
            value: T::default(),
            problem: Some(problem),
        }
    }

    /// Writes `value` whole, through a temporary file in the same directory
    /// renamed into place. On failure the old file is untouched and the
    /// temporary file is removed.
    pub fn save(&self, value: &T) -> std::io::Result<()> {
        let mut text = match self.layout {
            Layout::Compact => serde_json::to_string(value),
            Layout::Pretty => serde_json::to_string_pretty(value),
        }
        .map_err(std::io::Error::other)?;
        text.push('\n');
        let target = self.target();
        let directory = target.parent().unwrap_or(Path::new(""));
        if !directory.as_os_str().is_empty() {
            std::fs::create_dir_all(directory)?;
        }
        let existing = std::fs::metadata(&target).ok().map(|m| m.permissions());
        let temporary = Self::temporary(&target);
        let result = self
            .write_synced(&temporary, text.as_bytes())
            .and_then(|()| match existing {
                Some(permissions) => std::fs::set_permissions(&temporary, permissions),
                None => Ok(()),
            })
            .and_then(|()| std::fs::rename(&temporary, &target));
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    /// The file a save replaces: the path, or what it links to when it is
    /// a symlink, so that the link stays a link.
    fn target(&self) -> PathBuf {
        let linked = std::fs::symlink_metadata(&self.path).is_ok_and(|m| m.is_symlink());
        linked
            .then(|| std::fs::canonicalize(&self.path).ok())
            .flatten()
            .unwrap_or_else(|| self.path.clone())
    }

    /// A name no other save of this process, or another process, uses,
    /// beside the file and visible there if a crash leaves it.
    fn temporary(target: &Path) -> PathBuf {
        static SAVES: AtomicU64 = AtomicU64::new(0);
        let name = target.file_name().unwrap_or_default().to_string_lossy();
        target.with_file_name(format!(
            "{name}.{}.{}.tmp",
            std::process::id(),
            SAVES.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn write_synced(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::Write as _;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(self.mode);
        }
        let mut file = options.open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }
}

/// What a parse error says about the file, without its contents: serde_json's
/// own message can quote a value, and a value may be a path or an endpoint.
fn describe(error: &serde_json::Error) -> String {
    use serde_json::error::Category;
    let place = format!("line {}, column {}", error.line(), error.column());
    match error.classify() {
        Category::Io => "could not be read".into(),
        Category::Syntax => format!("is not valid JSON ({place})"),
        Category::Eof => format!("ends early ({place})"),
        Category::Data => format!("does not have the expected shape ({place})"),
    }
}

#[cfg(test)]
mod tests;
