//! The workspace file: the clusters a person works with, as definitions only.
//!
//! `workspace.json` sits beside the preferences. It names each cluster by a
//! stable entry id, never by a session key: `AccessIdentity::key()` is
//! process-local and must not reach a file. No file means a workspace of one,
//! which is today's behaviour. The active entry is UI state and lives in
//! `navigation.json`, not here.
//!
//! A file that can't be used is ignored, never half-used: [`load`] says why
//! and leaves it where it is. This half only reads; writing the file, setting
//! a refused one aside, and starting on an entry arrive with the callers that
//! need them.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{BoundedReadError, read_bounded_regular_file};

/// The file format this version reads.
pub const VERSION: u32 = 1;
/// The largest file read; a larger one is refused.
pub const MAX_BYTES: u64 = 256 * 1024;
/// The most clusters one workspace holds.
pub const MAX_ENTRIES: usize = 64;
const MAX_ID_BYTES: usize = 64;

/// What a cluster is for. A fixed set; the file refuses any other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Entry {
    pub id: String,
    pub role: Role,
    pub context: String,
    #[serde(default)]
    pub talosconfig: Option<PathBuf>,
}

impl Entry {
    pub fn new(id: impl Into<String>, role: Role, context: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            role,
            context: context.into(),
            talosconfig: None,
        }
    }
}

/// The clusters of a workspace, in the order the person keeps them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Workspace {
    /// The kubeconfig every entry's context is read from; `None` is the
    /// automatic one (`KUBECONFIG`, then the home default).
    #[serde(default)]
    pub kubeconfig: Option<PathBuf>,
    #[serde(default)]
    pub clusters: Vec<Entry>,
}

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
    let workspace: Workspace =
        serde_json::from_value(value).map_err(|error| Invalid::Malformed(error.to_string()))?;
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
        Err(BoundedReadError::TooLarge) => Loaded::Refused(Invalid::TooLarge),
        Err(BoundedReadError::NotFound) => Loaded::Missing,
        Err(error) => Loaded::Refused(Invalid::NotReadable(error.to_string())),
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
    }
}

#[cfg(test)]
mod tests;
