//! The preferences files beside `preferences.json`: where they live and how
//! one is read and written.
//!
//! A [`Dir`] is resolved once from the preferences path the app already uses
//! (`GpuiOptions.preferences`). It names each file in that directory, and a
//! [`Document`] loads and saves one of them as a typed value. A missing file
//! is the default. A file that can't be read or parsed is also the default,
//! and the load says why once, in [`Loaded::problem`], for the caller to log.
//! A save writes a temporary file in the same directory and renames it into
//! place, so a reader never sees half a file and a failed save leaves the old
//! one.
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
        Self(preferences.with_file_name(""))
    }

    /// The file called `name` in this directory, holding a `T`.
    pub fn document<T>(&self, name: &str, layout: Layout) -> Document<T> {
        Document {
            path: self.0.join(name),
            layout,
            value: PhantomData,
        }
    }
}

/// One JSON file holding a `T`.
#[derive(Debug)]
pub struct Document<T> {
    path: PathBuf,
    layout: Layout,
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
            Err(error) => return self.unusable(format!("could not be read: {error}")),
        };
        match serde_json::from_slice(&bytes) {
            Ok(value) => Loaded {
                value,
                problem: None,
            },
            Err(error) => self.unusable(format!("could not be parsed: {error}")),
        }
    }

    fn unusable(&self, why: String) -> Loaded<T> {
        Loaded {
            value: T::default(),
            problem: Some(format!("{} {why}", self.path.display())),
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
        let directory = self.path.parent().unwrap_or(Path::new(""));
        if !directory.as_os_str().is_empty() {
            std::fs::create_dir_all(directory)?;
        }
        let temporary = self.temporary();
        let result = write_synced(&temporary, text.as_bytes())
            .and_then(|()| std::fs::rename(&temporary, &self.path));
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    /// A name no other save of this process, or another process, uses.
    fn temporary(&self) -> PathBuf {
        static SAVES: AtomicU64 = AtomicU64::new(0);
        let name = self.path.file_name().unwrap_or_default().to_string_lossy();
        self.path.with_file_name(format!(
            ".{name}.{}.{}.tmp",
            std::process::id(),
            SAVES.fetch_add(1, Ordering::Relaxed)
        ))
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests;
