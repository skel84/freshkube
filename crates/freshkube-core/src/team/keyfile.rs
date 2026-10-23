//! A key file may live in the team repository, read at the applied version.
//! Its path must resolve inside the repository's checkout.

use std::path::{Component, Path, PathBuf};

/// Why a key file path is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyFileError {
    Absolute,
    ParentSegment,
    /// The path, or a link on the way, leads outside the repository.
    Escapes,
    NotFound,
    NotAFile,
}

impl std::fmt::Display for KeyFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            KeyFileError::Absolute => "a key file in the team repository needs a relative path",
            KeyFileError::ParentSegment => "a key file path may not contain “..”",
            KeyFileError::Escapes => "that key file leads outside the team repository",
            KeyFileError::NotFound => "the key file isn’t in the team repository",
            KeyFileError::NotAFile => "the key file isn’t a regular file",
        })
    }
}

impl std::error::Error for KeyFileError {}

/// Resolves `relative` inside the checkout at `root`: no absolute path, no `..`
/// and no symlink that points outside. Returns the real path to read.
pub fn resolve_in_repo(root: &Path, relative: &str) -> Result<PathBuf, KeyFileError> {
    let path = Path::new(relative);
    if path.is_absolute() || path.has_root() {
        return Err(KeyFileError::Absolute);
    }
    for part in path.components() {
        match part {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => return Err(KeyFileError::ParentSegment),
            _ => return Err(KeyFileError::Absolute),
        }
    }
    let root = root.canonicalize().map_err(|_| KeyFileError::NotFound)?;
    let real = root
        .join(path)
        .canonicalize()
        .map_err(|_| KeyFileError::NotFound)?;
    if !real.starts_with(&root) {
        return Err(KeyFileError::Escapes);
    }
    if !real.is_file() {
        return Err(KeyFileError::NotAFile);
    }
    Ok(real)
}
