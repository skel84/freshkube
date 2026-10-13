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
use std::sync::Mutex;
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

    /// The file called `name` in this directory, holding a `T`. `mode` is the
    /// Unix permission bits a save asks for when it creates the file (`0o644`
    /// for `navigation.json`, `0o666` for `preferences.json`); the process's
    /// umask clears bits from it, as for any file it creates. A file that
    /// exists keeps its own. On Windows there are no such bits: the mode is
    /// ignored and a new file takes what its directory gives it.
    pub fn document<T>(&self, name: &str, layout: Layout, mode: u32) -> Document<T> {
        Document {
            path: self.0.join(name),
            layout,
            mode,
            value: PhantomData,
        }
    }
}

/// One JSON file holding a `T`.
#[derive(Debug)]
pub struct Document<T> {
    path: PathBuf,
    layout: Layout,
    mode: u32,
    value: PhantomData<fn() -> T>,
}

impl<T> Document<T> {
    /// The file at `path`, holding a `T`, for a caller that has the whole
    /// path and not a directory and a name ([`Dir::document`]).
    pub fn at(path: &Path, layout: Layout, mode: u32) -> Self {
        Self {
            path: path.to_path_buf(),
            layout,
            mode,
            value: PhantomData,
        }
    }
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
        match self.read() {
            Ok(loaded) => loaded,
            Err(error) => self.unusable(format!("could not be read: {error}")),
        }
    }

    /// As [`load`](Self::load), but a read error other than "not found" is
    /// returned and not turned into the default. A file that parses wrongly
    /// is still the default with a problem: it holds nothing worth keeping.
    fn read(&self) -> std::io::Result<Loaded<T>> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Loaded {
                    value: T::default(),
                    problem: None,
                });
            }
            Err(error) => return Err(error),
        };
        Ok(match serde_json::from_slice(&bytes) {
            Ok(value) => Loaded {
                value,
                problem: None,
            },
            Err(error) => self.unusable(format!("could not be parsed: {}", describe(&error))),
        })
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
    ///
    /// The file keeps its permissions when it exists, and takes the
    /// document's mode, less the umask, when it is new. A path that is a
    /// symlink stays one: the temporary file goes beside its target and is
    /// renamed over the target.
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
        let permissions = std::fs::metadata(&target).ok().map(|m| m.permissions());
        let temporary = temporary_beside(&target);
        let result = self
            .write_synced(&temporary, text.as_bytes(), permissions.is_some())
            .and_then(|()| give_permissions(&temporary, permissions))
            .and_then(|()| std::fs::rename(&temporary, &target));
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    /// The file a save replaces: the path itself, or what a symlink there
    /// points to.
    fn target(&self) -> PathBuf {
        let is_link = std::fs::symlink_metadata(&self.path)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false);
        if !is_link {
            return self.path.clone();
        }
        std::fs::canonicalize(&self.path).unwrap_or_else(|_| {
            // A link to a file that doesn't exist yet: write where it points.
            match std::fs::read_link(&self.path) {
                Ok(to) => self.path.with_file_name("").join(to),
                Err(_) => self.path.clone(),
            }
        })
    }

    /// Creates the temporary file and writes it to disk. A new file is
    /// created with the document's mode, so the umask applies; one that
    /// replaces a file is private until it gets that file's permissions.
    fn write_synced(&self, path: &Path, bytes: &[u8], replaces: bool) -> std::io::Result<()> {
        use std::io::Write as _;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(
            &mut options,
            if replaces { 0o600 } else { self.mode & 0o7777 },
        );
        #[cfg(not(unix))]
        let _ = (replaces, self.mode);
        let mut file = options.open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }

    /// Loads the file, lets `change` edit its value and saves it back. A
    /// writer that owns some keys of a shared document sets those and leaves
    /// the rest as it found it, so a `Document<Map<String, Value>>` keeps
    /// the keys this build does not know. A file that is missing starts from
    /// the default, and so does one that parses wrongly (it holds nothing
    /// this build can keep). A file that exists but can't be read (no
    /// permission, an I/O error, a directory) returns that error and saves
    /// nothing, so a transient failure never wipes the other keys.
    ///
    /// Updates in this process run one at a time, so two of them never lose
    /// each other's change. Another process is not excluded. `change` must
    /// not call `update`, on this document or any other: the lock is not
    /// re-entrant.
    pub fn update(&self, change: impl FnOnce(&mut T)) -> std::io::Result<()> {
        static UPDATING: Mutex<()> = Mutex::new(());
        let _one_at_a_time = UPDATING
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut value = self.read()?.value;
        change(&mut value);
        self.save(&value)
    }
}

#[cfg(unix)]
fn give_permissions(
    temporary: &Path,
    existing: Option<std::fs::Permissions>,
) -> std::io::Result<()> {
    match existing {
        Some(permissions) => std::fs::set_permissions(temporary, permissions),
        None => Ok(()),
    }
}

#[cfg(not(unix))]
fn give_permissions(
    _temporary: &Path,
    _existing: Option<std::fs::Permissions>,
) -> std::io::Result<()> {
    Ok(())
}

/// A name no other save of this process, or another process, uses.
fn temporary_beside(target: &Path) -> PathBuf {
    static SAVES: AtomicU64 = AtomicU64::new(0);
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        SAVES.fetch_add(1, Ordering::Relaxed)
    ))
}

/// A parse error in words that never carry the file's contents: serde_json's
/// own message echoes the value it did not expect, which in a typed document
/// can be a path, a context name or an endpoint. Only the kind of error and
/// where it is.
fn describe(error: &serde_json::Error) -> String {
    use serde_json::error::Category;
    let kind = match error.classify() {
        Category::Io => "a read error",
        Category::Syntax => "invalid JSON",
        Category::Data => "a value of the wrong type or shape",
        Category::Eof => "the file ends too early",
    };
    format!("{kind} at line {}, column {}", error.line(), error.column())
}

#[cfg(test)]
mod tests;
