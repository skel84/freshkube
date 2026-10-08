//! The workspace file: the clusters a person works with, as definitions only.
//!
//! `workspace.json` sits beside the preferences. It names each cluster by a
//! stable entry id, never by a session key: `AccessIdentity::key()` is
//! process-local and must not reach a file. No file means a workspace of one,
//! which is today's behaviour. The active entry is UI state and lives in
//! `navigation.json`, not here.
//!
//! A file that can't be read is ignored, never half-used: [`load`] says why,
//! and [`set_aside`] moves it to a backup before the first save, never over an
//! earlier backup. Keys this version doesn't know (`destinations`, `sources`
//! and later ones) are kept when the file is written again.

use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::read_bounded_regular_file;

/// The file format this version reads and writes.
pub const VERSION: u32 = 1;
/// The largest file read; a larger one is treated as unreadable.
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
/// `context` is the kubeconfig context it opens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    pub context: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub talosconfig: Option<PathBuf>,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

impl Entry {
    pub fn new(id: impl Into<String>, role: Option<Role>, context: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            role,
            context: context.into(),
            talosconfig: None,
            extra: Default::default(),
        }
    }

    pub fn with_talosconfig(mut self, path: impl Into<PathBuf>) -> Self {
        self.talosconfig = Some(path.into());
        self
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
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

/// Why a file or a workspace is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invalid {
    NotJson,
    UnknownVersion(Option<u64>),
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
            Invalid::UnknownVersion(Some(version)) => {
                write!(f, "it is version {version}, and this app reads {VERSION}")
            }
            Invalid::UnknownVersion(None) => f.write_str("it has no version"),
            Invalid::TooLarge => f.write_str("it is larger than 256 KiB"),
            Invalid::TooManyEntries => write!(f, "it names more than {MAX_ENTRIES} clusters"),
            Invalid::EmptyId => f.write_str("a cluster has no id"),
            Invalid::LongId(id) => write!(f, "the id “{id}” is longer than {MAX_ID_BYTES} bytes"),
            Invalid::DuplicateId(id) => write!(f, "the id “{id}” is used twice"),
            Invalid::EmptyContext(id) => write!(f, "the cluster “{id}” names no context"),
            Invalid::RelativePath(id) => {
                write!(f, "a path for “{id}” isn’t absolute")
            }
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

    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.clusters.iter().find(|entry| entry.id == id)
    }

    /// Where the app starts when the remembered entry is gone: the first
    /// `core` entry. Never the kubeconfig's current context.
    pub fn first_core(&self) -> Option<&Entry> {
        self.clusters
            .iter()
            .find(|entry| entry.role == Some(Role::Core))
    }

    /// The entry to start on: the remembered one when it still exists, else
    /// the first `core` entry.
    pub fn start_entry(&self, remembered: Option<&str>) -> Option<&Entry> {
        remembered
            .and_then(|id| self.entry(id))
            .or_else(|| self.first_core())
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
        if self.entry(base).is_none() {
            return base.to_owned();
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|id| self.entry(id).is_none())
            .expect("an unused suffix exists")
    }
}

/// What reading the file found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loaded {
    /// No file: a workspace of one.
    Missing,
    Workspace(Workspace),
    /// A file the app won't use, and why. It stays on disk until the first
    /// save sets it aside.
    Refused(Invalid),
}

/// Parses file contents.
pub fn parse(bytes: &[u8]) -> Result<Workspace, Invalid> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| Invalid::NotJson)?;
    let version = value.get("version").and_then(|v| v.as_u64());
    if version != Some(u64::from(VERSION)) {
        return Err(Invalid::UnknownVersion(version));
    }
    let mut workspace: Workspace =
        serde_json::from_value(value).map_err(|error| Invalid::Malformed(error.to_string()))?;
    // The version is written fresh each time; it isn't an unknown key.
    workspace.extra.remove("version");
    workspace.validate()?;
    Ok(workspace)
}

/// Reads `file`. A missing file is not an error.
pub fn load(file: &Path) -> Loaded {
    match read_bounded_regular_file(file, MAX_BYTES) {
        Ok(bytes) => match parse(&bytes) {
            Ok(workspace) => Loaded::Workspace(workspace),
            Err(invalid) => Loaded::Refused(invalid),
        },
        Err(crate::BoundedReadError::TooLarge) => Loaded::Refused(Invalid::TooLarge),
        Err(crate::BoundedReadError::NotFound) => Loaded::Missing,
        Err(error) => Loaded::Refused(Invalid::NotReadable(error.to_string())),
    }
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
/// failed save leaves the old file whole.
pub fn save(file: &Path, workspace: &Workspace) -> std::io::Result<()> {
    workspace.validate().map_err(|invalid| {
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
/// `workspace.json.bak`, or `workspace.<UTC time>.bak` when that exists. An
/// earlier backup is never overwritten.
pub fn set_aside(file: &Path, now: chrono::DateTime<chrono::Utc>) -> std::io::Result<PathBuf> {
    let first = file.with_extension("json.bak");
    let target = if first.symlink_metadata().is_ok() {
        let stamp = now.format("%Y%m%dT%H%M%SZ");
        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("workspace");
        let mut candidate = file.with_file_name(format!("{stem}.{stamp}.bak"));
        let mut n = 2;
        while candidate.symlink_metadata().is_ok() {
            candidate = file.with_file_name(format!("{stem}.{stamp}-{n}.bak"));
            n += 1;
        }
        candidate
    } else {
        first
    };
    fs::rename(file, &target)?;
    Ok(target)
}

/// The acme workspace of `docs/platform/`'s mocks as a workspace file:
/// one core cluster, one for CI/CD and four environments. Names are
/// invented, for `--fixture` and for tests.
pub fn example() -> Workspace {
    let entry = |id: &str, role| Entry::new(id, Some(role), id);
    Workspace {
        kubeconfig: None,
        clusters: vec![
            entry("core-fra", Role::Core).with_talosconfig("/example/acme/core-fra.talosconfig"),
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
