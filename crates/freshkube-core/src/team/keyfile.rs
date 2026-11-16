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
    /// A tree entry of mode 120000: a link, which is never followed.
    Symlink,
    TooLarge,
    /// Not exactly one PEM public key.
    NotAKey,
}

impl std::fmt::Display for KeyFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            KeyFileError::Absolute => "a key file in the team repository needs a relative path",
            KeyFileError::ParentSegment => "a key file path may not contain “..”",
            KeyFileError::Escapes => "that key file leads outside the team repository",
            KeyFileError::NotFound => "the key file isn’t in the team repository",
            KeyFileError::NotAFile => "the key file isn’t a regular file",
            KeyFileError::Symlink => "the key file is a link, which is refused",
            KeyFileError::TooLarge => "the key file is larger than 8 KiB",
            KeyFileError::NotAKey => "the key file isn’t one PEM public key",
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

/// The most a key file may hold.
pub const MOST_KEY_BYTES: u64 = 8 * 1024;

/// Checks a key file path as a name inside the repository, before any object
/// is looked up: relative, no `..`, no empty, `.` or backslash segment.
pub fn check_repo_path(relative: &str) -> Result<(), KeyFileError> {
    if relative.starts_with('/') || relative.starts_with('\\') || relative.contains(':') {
        return Err(KeyFileError::Absolute);
    }
    if relative.chars().any(char::is_control) || relative.contains('\\') {
        return Err(KeyFileError::Escapes);
    }
    for part in relative.split('/') {
        match part {
            ".." => return Err(KeyFileError::ParentSegment),
            "" | "." => return Err(KeyFileError::Escapes),
            _ => {}
        }
    }
    Ok(())
}

/// `SHA256:` and the base64 of the SHA-256 of the key's DER, as the
/// supply-chain screens show a key: never the key itself.
pub fn fingerprint(pem: &[u8]) -> Result<String, KeyFileError> {
    use base64::Engine;
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
    use sha2::{Digest, Sha256};
    const BEGIN: &str = "-----BEGIN PUBLIC KEY-----";
    const END: &str = "-----END PUBLIC KEY-----";
    if pem.len() as u64 > MOST_KEY_BYTES {
        return Err(KeyFileError::TooLarge);
    }
    let text = std::str::from_utf8(pem).map_err(|_| KeyFileError::NotAKey)?;
    let rest = text
        .trim()
        .strip_prefix(BEGIN)
        .ok_or(KeyFileError::NotAKey)?;
    let (body, after) = rest.split_once(END).ok_or(KeyFileError::NotAKey)?;
    if !after.trim().is_empty() {
        return Err(KeyFileError::NotAKey);
    }
    let encoded: String = body.split_whitespace().collect();
    let der = STANDARD
        .decode(encoded)
        .map_err(|_| KeyFileError::NotAKey)?;
    Ok(format!(
        "SHA256:{}",
        STANDARD_NO_PAD.encode(Sha256::digest(der))
    ))
}
