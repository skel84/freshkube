//! The workspace file: the clusters a person works with, as definitions only.
//!
//! `workspace.json` sits beside the preferences. It names each cluster by a
//! stable entry id, never by a session key: `AccessIdentity::key()` is
//! process-local and must not reach a file. No file means a workspace of one,
//! which is today's behaviour. The active entry is UI state and lives in
//! `navigation.json`, not here.
//!
//! A file that can't be used is ignored, never half-used: [`load`] says why
//! and leaves it where it is. The first [`save`] after that sets it aside
//! with [`set_aside`], which never replaces a backup. Keys this version
//! doesn't know are kept when the file is written again, and
//! [`Workspace::unknown_keys`] names them, so a misspelt `talosconfg` doesn't
//! go unnoticed. Starting on an entry arrives with the callers that open one.

use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::{BoundedReadError, read_bounded_regular_file};

/// The file format this version reads.
pub const VERSION: u32 = 1;
/// The largest file read; a larger one is refused.
pub const MAX_BYTES: u64 = 256 * 1024;
/// The most clusters one workspace holds.
pub const MAX_ENTRIES: usize = 64;
const MAX_ID_BYTES: usize = 64;

/// What a cluster is for. A fixed set; the file refuses any other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// The management cluster, where the others are described.
    Core,
    /// The cluster that builds and delivers.
    Cicd,
    /// A cluster the software runs in.
    Environment,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Core, Role::Cicd, Role::Environment];

    pub fn label(self) -> &'static str {
        match self {
            Role::Core => "Core",
            Role::Cicd => "CI/CD",
            Role::Environment => "Environment",
        }
    }
}

/// One cluster of the workspace. `id` is stable across runs and edits;
/// `context` is the kubeconfig context it opens. Every entry in a file has a
/// role: having none is reserved for the implicit workspace of one, which is
/// never written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub role: Role,
    pub context: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub talosconfig: Option<PathBuf>,
    /// Keys this version doesn't know, written back as they were read.
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

impl Entry {
    pub fn new(id: impl Into<String>, role: Role, context: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            role,
            context: context.into(),
            talosconfig: None,
            extra: Default::default(),
        }
    }
}

/// The clusters of a workspace, in the order the person keeps them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    /// The kubeconfig every entry's context is read from; `None` is the
    /// automatic one (`KUBECONFIG`, then the home default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kubeconfig: Option<PathBuf>,
    #[serde(default)]
    pub clusters: Vec<Entry>,
    /// Keys this version doesn't know (`destinations`, `sources` and later
    /// ones), written back as they were read.
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

/// Top-level keys reserved for later steps: kept on save, never warned about.
const RESERVED: [&str; 2] = ["destinations", "sources"];

/// Why a file or a workspace is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invalid {
    NotJson,
    MissingVersion,
    UnreadableVersion,
    UnknownVersion(u64),
    TooLarge,
    TooManyEntries,
    EmptyId,
    LongId(String),
    DuplicateId(String),
    EmptyContext(String),
    RelativePath(String),
    NotReadable(String),
    Malformed(String),
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Invalid::NotJson => f.write_str("it isn’t valid JSON"),
            Invalid::MissingVersion => f.write_str("it has no version"),
            Invalid::UnreadableVersion => f.write_str("its version isn’t a whole number"),
            Invalid::UnknownVersion(version) => {
                write!(f, "it is version {version}, and this app reads {VERSION}")
            }
            Invalid::TooLarge => f.write_str("it is larger than 256 KiB"),
            Invalid::TooManyEntries => write!(f, "it names more than {MAX_ENTRIES} clusters"),
            Invalid::EmptyId => f.write_str("a cluster has no id"),
            Invalid::LongId(id) => write!(f, "the id “{id}” is longer than {MAX_ID_BYTES} bytes"),
            Invalid::DuplicateId(id) => write!(f, "the id “{id}” is used twice"),
            Invalid::EmptyContext(id) => write!(f, "the cluster “{id}” names no context"),
            Invalid::RelativePath(id) => write!(f, "a path for “{id}” isn’t absolute"),
            Invalid::NotReadable(reason) => write!(f, "it can’t be read: {reason}"),
            Invalid::Malformed(reason) => write!(f, "it isn’t a workspace: {reason}"),
        }
    }
}

impl Workspace {
    /// Checks what a file must hold before the app uses it: unique ids that
    /// stay short, a context for each, absolute paths, a bounded count.
    pub fn validate(&self) -> Result<(), Invalid> {
        if self.clusters.len() > MAX_ENTRIES {
            return Err(Invalid::TooManyEntries);
        }
        if let Some(path) = &self.kubeconfig
            && !path.is_absolute()
        {
            return Err(Invalid::RelativePath("the workspace".into()));
        }
        let mut seen = HashSet::new();
        for entry in &self.clusters {
            if entry.id.trim().is_empty() {
                return Err(Invalid::EmptyId);
            }
            if entry.id.len() > MAX_ID_BYTES {
                return Err(Invalid::LongId(entry.id.clone()));
            }
            if !seen.insert(entry.id.as_str()) {
                return Err(Invalid::DuplicateId(entry.id.clone()));
            }
            if entry.context.trim().is_empty() {
                return Err(Invalid::EmptyContext(entry.id.clone()));
            }
            if entry.talosconfig.as_ref().is_some_and(|p| !p.is_absolute()) {
                return Err(Invalid::RelativePath(entry.id.clone()));
            }
        }
        Ok(())
    }

    /// What [`validate`](Self::validate) checks, and that the written file
    /// stays within what [`load`] reads: a file the next launch would refuse
    /// for its size is not written.
    pub fn check_saveable(&self) -> Result<(), Invalid> {
        self.validate()?;
        if to_text(self).len() as u64 > MAX_BYTES {
            return Err(Invalid::TooLarge);
        }
        Ok(())
    }

    /// Keys this version doesn't know, as `key` or `cluster-id.key`, sorted. They are kept when the file is saved; the page names them so a
    /// misspelt key doesn't go unnoticed. The reserved `destinations` and
    /// `sources` are not listed.
    pub fn unknown_keys(&self) -> Vec<String> {
        let top = self
            .extra
            .keys()
            .filter(|key| !RESERVED.contains(&key.as_str()))
            .cloned();
        let entries = self
            .clusters
            .iter()
            .flat_map(|entry| entry.extra.keys().map(|key| format!("{}.{key}", entry.id)));
        let mut keys: Vec<String> = top.chain(entries).collect();
        keys.sort();
        keys
    }

    /// An id for a new entry that no entry uses, from the context's name.
    pub fn fresh_id(&self, context: &str) -> String {
        let mut base: String = context
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        base.truncate(MAX_ID_BYTES - 6);
        let base = base.trim_matches('-');
        let base = if base.is_empty() { "cluster" } else { base };
        if !self.clusters.iter().any(|entry| entry.id == base) {
            return base.to_owned();
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|id| !self.clusters.iter().any(|entry| &entry.id == id))
            .expect("an unused suffix exists")
    }
}

/// What reading the file found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loaded {
    /// No file: a workspace of one.
    Missing,
    Workspace(Workspace),
    /// A file the app won't use, and why. It stays on disk untouched.
    Refused(Invalid),
}

/// Parses file contents.
pub fn parse(bytes: &[u8]) -> Result<Workspace, Invalid> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| Invalid::NotJson)?;
    match value.get("version") {
        None => return Err(Invalid::MissingVersion),
        Some(version) => match version.as_u64() {
            None => return Err(Invalid::UnreadableVersion),
            Some(version) if version != u64::from(VERSION) => {
                return Err(Invalid::UnknownVersion(version));
            }
            Some(_) => {}
        },
    }
    let mut workspace: Workspace =
        serde_json::from_value(value).map_err(|error| Invalid::Malformed(error.to_string()))?;
    // The version is written fresh each time; it isn't an unknown key.
    workspace.extra.remove("version");
    workspace.validate()?;
    Ok(workspace)
}

/// What a file held when it was read, to tell whether it changed before a
/// save wrote over it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seen {
    Missing,
    Bytes(Vec<u8>),
    /// Too large, not a regular file, or not readable.
    Unreadable,
}

fn read_seen(file: &Path) -> (Seen, Result<Vec<u8>, BoundedReadError>) {
    let read = read_bounded_regular_file(file, MAX_BYTES);
    let seen = match &read {
        Ok(bytes) => Seen::Bytes(bytes.clone()),
        Err(BoundedReadError::NotFound) => Seen::Missing,
        Err(_) => Seen::Unreadable,
    };
    (seen, read)
}

/// Reads `file`. A missing file is not an error.
pub fn load(file: &Path) -> Loaded {
    load_seen(file).0
}

/// Reads `file` and what it held, so a later save can tell whether the file
/// changed in between.
pub fn load_seen(file: &Path) -> (Loaded, Seen) {
    let (seen, read) = read_seen(file);
    let loaded = match read {
        Ok(bytes) => match parse(&bytes) {
            Ok(workspace) => Loaded::Workspace(workspace),
            Err(invalid) => Loaded::Refused(invalid),
        },
        Err(BoundedReadError::TooLarge) => Loaded::Refused(Invalid::TooLarge),
        Err(BoundedReadError::NotFound) => Loaded::Missing,
        Err(error) => Loaded::Refused(Invalid::NotReadable(error.to_string())),
    };
    (loaded, seen)
}

/// The file's text, with its version first.
pub fn to_text(workspace: &Workspace) -> String {
    let mut object = serde_json::Map::new();
    object.insert("version".into(), VERSION.into());
    if let serde_json::Value::Object(rest) =
        serde_json::to_value(workspace).expect("a workspace serializes")
    {
        object.extend(rest);
    }
    format!("{:#}\n", serde_json::Value::Object(object))
}

/// Writes the workspace through a temporary file renamed into place, so a
/// failed save leaves the old file whole. An invalid workspace writes nothing.
pub fn save(file: &Path, workspace: &Workspace) -> std::io::Result<()> {
    workspace.check_saveable().map_err(|invalid| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, invalid.to_string())
    })?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);
    let temporary = file.with_extension(format!(
        "json.{}.{}.tmp",
        std::process::id(),
        NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut output = options.open(&temporary)?;
        output.write_all(to_text(workspace).as_bytes())?;
        output.sync_all()?;
        fs::rename(&temporary, file)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Moves a refused file to a backup beside it and returns the backup's path:
/// `workspace.json.bak`, or `workspace.<UTC time>.bak` (then `-2`, `-3`, …)
/// when that exists. A backup is never replaced: the file is linked to the
/// name, which fails if the name is taken, and only then unlinked, so a backup
/// that appears between the check and the move is not overwritten.
pub fn set_aside(file: &Path, now: chrono::DateTime<chrono::Utc>) -> std::io::Result<PathBuf> {
    let stem = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("workspace");
    let stamp = now.format("%Y%m%dT%H%M%SZ");
    let candidates = std::iter::once(file.with_extension("json.bak"))
        .chain(std::iter::once(
            file.with_file_name(format!("{stem}.{stamp}.bak")),
        ))
        .chain((2..100).map(|n| file.with_file_name(format!("{stem}.{stamp}-{n}.bak"))));
    for target in candidates {
        match fs::hard_link(file, &target) {
            Ok(()) => {
                fs::remove_file(file)?;
                return Ok(target);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::other(
        "too many backups of the workspace file",
    ))
}

/// Why [`commit`] wrote nothing, or what it did before failing.
#[derive(Debug)]
pub enum SaveError {
    /// The file is not what it was when it was read: someone changed,
    /// created or removed it. Nothing was written or moved.
    Changed,
    /// The workspace is invalid, or too large for the next launch to read.
    Invalid(Invalid),
    /// The unused file could not be set aside; nothing was written.
    Aside(std::io::Error),
    /// The write failed. `aside` is where the unused file went before it,
    /// if it was moved.
    Write {
        aside: Option<PathBuf>,
        error: std::io::Error,
    },
}

/// Saves `workspace` if the file is still as it was when it was read
/// (`expected`). `refused` says that file was one the app doesn't use, which
/// is set aside first. Returns where it went, if it was, and what the file now
/// holds, for the next save. Re-reading just before the write narrows the
/// window in which a hand edit can be lost to a few microseconds; it cannot
/// close it, as no portable compare-and-rename exists.
pub fn commit(
    file: &Path,
    expected: &Seen,
    refused: bool,
    workspace: &Workspace,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(Option<PathBuf>, Seen), SaveError> {
    workspace.check_saveable().map_err(SaveError::Invalid)?;
    if &read_seen(file).0 != expected {
        return Err(SaveError::Changed);
    }
    let aside = if refused {
        Some(set_aside(file, now).map_err(SaveError::Aside)?)
    } else {
        None
    };
    match save(file, workspace) {
        Ok(()) => Ok((aside, Seen::Bytes(to_text(workspace).into_bytes()))),
        Err(error) => Err(SaveError::Write { aside, error }),
    }
}

/// The acme workspace of `docs/platform/`'s mocks as a workspace: one core
/// cluster, one for CI/CD and four environments. Names are invented, for
/// `--fixture` and for tests; it holds no paths, so it is valid wherever
/// paths are absolute.
pub fn example() -> Workspace {
    let entry = |id: &str, role| Entry::new(id, role, id);
    Workspace {
        kubeconfig: None,
        clusters: vec![
            entry("core-fra", Role::Core),
            entry("cicd-fra", Role::Cicd),
            entry("dev-fra", Role::Environment),
            entry("stage-fra", Role::Environment),
            entry("prod-ams", Role::Environment),
            entry("prod-fra", Role::Environment),
        ],
        extra: Default::default(),
    }
}

#[cfg(test)]
mod tests;
