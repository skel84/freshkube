//! The preferences files beside `preferences.json`: where they live and how
//! one is read and written.
//!
//! A [`Dir`] is resolved once from the preferences path the app already uses
//! (`GpuiOptions.preferences`). It names each file in that directory, and a
//! [`Document`] loads and saves one of them as a typed value. A missing file
//! is the default. A file that can't be read or parsed is also the default,
//! and the load says why once, in [`Loaded::problem`], for the caller to log.
//!
//! A file that can't be parsed (a hand edit, a half-written file) is copied
//! aside as `<name>.corrupt-<UTC time>.json` before anything replaces it
//! (see [`keep`]). A save writes `"version": 1` into an object; a load takes
//! it out before the value is read, so a file without one is version 1. A
//! file with a newer version is the default for a load and is never written:
//! an older build does not destroy a newer build's file.
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
use serde_json::Value;

mod keep;
pub use keep::{KEEP, Notice, Why, take_notices, watch};

/// The version a save writes and a load understands.
pub const VERSION: u64 = 1;

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
            limit: None,
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
    limit: Option<u64>,
    value: PhantomData<fn() -> T>,
}

impl<T> Document<T> {
    /// Reads at most `bytes` of the file, and only a regular file. One that
    /// is larger, or is not a regular file (a directory, a FIFO), is
    /// unusable: the default, with a problem. Only reads are bounded; a
    /// save writes what it is given.
    pub fn limited_to(mut self, bytes: u64) -> Self {
        self.limit = Some(bytes);
        self
    }

    /// The file at `path`, holding a `T`, for a caller that has the whole
    /// path and not a directory and a name ([`Dir::document`]).
    pub fn at(path: &Path, layout: Layout, mode: u32) -> Self {
        Self {
            path: path.to_path_buf(),
            layout,
            mode,
            limit: None,
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
        match self.read(false) {
            Ok(loaded) => loaded,
            Err(error) => self.unusable(format!("could not be read: {error}")),
        }
    }

    /// As [`load`](Self::load), but a read error other than "not found" is
    /// returned and not turned into the default. A file that parses wrongly
    /// is still the default with a problem: it holds nothing worth keeping.
    /// A file a limited document refuses (past the limit, not a regular
    /// file) is the default with a problem, or with `refuse` an
    /// `InvalidData` error, for a caller that is about to write over it.
    fn read(&self, refuse: bool) -> std::io::Result<Loaded<T>> {
        let bytes = match self.read_bytes() {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(why)) if refuse => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{} {why}", self.path.display()),
                ));
            }
            Ok(Err(why)) => return Ok(self.unusable(why)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Loaded {
                    value: T::default(),
                    problem: None,
                });
            }
            Err(error) => return Err(error),
        };
        Ok(match parse(&bytes) {
            Found::Value(value) => Loaded {
                value,
                problem: None,
            },
            Found::Newer(version) => {
                self.tell_newer(version);
                self.unusable(newer(version))
            }
            Found::Corrupt(why) => {
                let kept = match keep::copy(&self.path, &bytes) {
                    Ok(copy) => format!("; kept as {}", copy.display()),
                    Err(error) => format!("; a copy could not be kept: {error}"),
                };
                self.unusable(format!("could not be parsed: {why}{kept}"))
            }
        })
    }

    /// The file's bytes, or why they are not to be used (past the limit,
    /// not a regular file). An I/O error is the outer error.
    fn read_bytes(&self) -> std::io::Result<Result<Vec<u8>, String>> {
        use std::io::Read as _;
        let Some(limit) = self.limit else {
            return std::fs::read(&self.path).map(Ok);
        };
        // Checked before opening, so a FIFO there does not block the open,
        // and again on the opened file, which catches a directory swapped
        // in between. A FIFO swapped in between the two can still block the
        // open: closing that window needs `O_NONBLOCK`, which needs `libc`,
        // and this crate is std, serde and serde_json only.
        if !std::fs::metadata(&self.path)?.is_file() {
            return Ok(Err("is not a regular file".into()));
        }
        let file = std::fs::File::open(&self.path)?;
        if !file.metadata()?.is_file() {
            return Ok(Err("is not a regular file".into()));
        }
        let mut bytes = Vec::new();
        file.take(limit + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            return Ok(Err(format!("is larger than {limit} bytes")));
        }
        Ok(Ok(bytes))
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
        let mut json = serde_json::to_value(value).map_err(std::io::Error::other)?;
        if let Value::Object(map) = &mut json {
            map.insert("version".into(), VERSION.into());
        }
        let mut text = match self.layout {
            Layout::Compact => serde_json::to_string(&json),
            Layout::Pretty => serde_json::to_string_pretty(&json),
        }
        .map_err(std::io::Error::other)?;
        text.push('\n');
        self.before_replacing()?;
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

    fn tell_newer(&self, version: u64) {
        keep::tell(Notice {
            file: self.path.clone(),
            why: Why::Newer { version },
        });
    }

    /// What a save may do to the file that is there: refuse to replace a
    /// newer version's (and tell the person, since a save that never loaded
    /// it has not), keep a copy of one that can't be parsed, and refuse one
    /// that can't be read or checked (past the limit, not a regular file,
    /// an I/O error), which could not be kept either. Only a missing file
    /// is replaced without a look.
    ///
    /// ponytail: another process can write between this check and the
    /// rename; an exclusive lock on the directory if that is ever seen.
    fn before_replacing(&self) -> std::io::Result<()> {
        let bytes = match self.read_bytes() {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(why)) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{} {why}; not overwritten", self.path.display()),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        match parse::<T>(&bytes) {
            Found::Value(_) => Ok(()),
            Found::Newer(version) => {
                self.tell_newer(version);
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!(
                        "{} {}; not overwritten",
                        self.path.display(),
                        newer(version)
                    ),
                ))
            }
            Found::Corrupt(_) => keep::copy(&self.path, &bytes).map(drop),
        }
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
    /// the default, and so does one that parses wrongly, after a copy of it is
    /// kept. A file with a newer version returns an error and saves nothing,
    /// and so does one that exists but can't be read (no permission, an I/O
    /// error, a directory),  so a transient failure never wipes the other keys. So does a
    /// file that a [`limited_to`](Self::limited_to) document refuses.
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
        let mut value = self.read(true)?.value;
        change(&mut value);
        self.save(&value)
    }
}

enum Found<T> {
    Value(T),
    /// The version a newer build wrote.
    Newer(u64),
    Corrupt(String),
}

/// Reads `bytes` as a `T`, without its `version` key.
fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Found<T> {
    let mut json: Value = match serde_json::from_slice(bytes) {
        Ok(json) => json,
        Err(error) => return Found::Corrupt(describe(&error)),
    };
    if let Value::Object(map) = &mut json
        && let Some(version) = map.remove("version")
    {
        match version.as_u64() {
            Some(version) if version > VERSION => return Found::Newer(version),
            Some(_) => {}
            None => return Found::Corrupt("has a version that is not a number".into()),
        }
    }
    match serde_json::from_value(json) {
        Ok(value) => Found::Value(value),
        Err(error) => Found::Corrupt(describe(&error)),
    }
}

fn newer(version: u64) -> String {
    format!("was written by a newer Freshkube (version {version}) and is not used")
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
    // An error from a value already parsed (a file with a `version`, which
    // is taken out before the typed read) has no position in the file.
    match error.line() {
        0 => kind.to_owned(),
        line => format!("{kind} at line {line}, column {}", error.column()),
    }
}

#[cfg(test)]
mod tests;
