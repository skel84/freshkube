//! What a team file may set. The file is read as plain YAML, not as a
//! [`crate::setup::Setup`], because it never names a context. Every key the
//! team may not set is named in [`Version::ignored`] and left out of
//! [`Version::values`]; none is an error.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::workspace::MAX_BYTES;

/// The team's values, flat: `coroot.server`, `clusters[cluster-a].protection`.
/// A list that isn't `clusters` is one value.
pub type Values = BTreeMap<String, Value>;

/// Why a team file is refused as a whole. The text never repeats the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionError {
    TooLarge,
    NotYaml { line: usize, column: usize },
    NotAMap,
    MissingVersion,
    UnknownVersion(u64),
    ClusterWithoutId,
    DuplicateCluster(String),
}

impl std::fmt::Display for VersionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionError::TooLarge => f.write_str("the team file is larger than 256 KiB"),
            VersionError::NotYaml { line, column } => {
                write!(f, "invalid YAML at line {line}, column {column}")
            }
            VersionError::NotAMap => f.write_str("the team file isn’t a map of settings"),
            VersionError::MissingVersion => f.write_str("the team file has no version"),
            VersionError::UnknownVersion(n) => {
                write!(f, "the team file is version {n}, and this app reads 1")
            }
            VersionError::ClusterWithoutId => f.write_str("a team cluster has no id"),
            VersionError::DuplicateCluster(id) => {
                write!(f, "the team cluster “{id}” is listed twice")
            }
        }
    }
}

impl std::error::Error for VersionError {}

/// Why a key is ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IgnoredKey {
    Context,
    LocalPath,
    KeyOrToken,
    AgentAccess,
    /// What the app may write is the person's own setting.
    Writes,
    Notifications,
    History,
}

impl IgnoredKey {
    pub fn words(self) -> &'static str {
        match self {
            IgnoredKey::Context => "contexts are always yours",
            IgnoredKey::LocalPath => "a path on your computer is yours",
            IgnoredKey::KeyOrToken => "keys and tokens are never shared",
            IgnoredKey::AgentAccess => "agent access is your own consent",
            IgnoredKey::Writes => "what the app may write is yours",
            IgnoredKey::Notifications => "notification and follow settings are yours",
            IgnoredKey::History => "history is yours",
        }
    }
}

/// A key the file set that this app ignores, with where it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ignored {
    pub at: String,
    pub why: IgnoredKey,
}

/// One version of the team file: an id (a commit, or the content's hash), the
/// values it sets and what it set that is ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub id: String,
    pub values: Values,
    pub ignored: Vec<Ignored>,
    /// Key files named by `supply_chain.public_key_file`: path → fingerprint of
    /// its contents, read at this version by the fetcher.
    pub key_fingerprints: BTreeMap<String, String>,
}

/// The one key whose value is a path inside the team repository.
pub const KEY_FILE: &str = "supply_chain.public_key_file";

impl Version {
    pub fn parse(
        id: impl Into<String>,
        bytes: &[u8],
        key_fingerprints: BTreeMap<String, String>,
    ) -> Result<Version, VersionError> {
        if bytes.len() as u64 > MAX_BYTES {
            return Err(VersionError::TooLarge);
        }
        let root: Value = serde_yaml::from_slice(bytes).map_err(|error| {
            let at = error.location();
            VersionError::NotYaml {
                line: at.as_ref().map_or(1, |l| l.line()),
                column: at.as_ref().map_or(1, |l| l.column()),
            }
        })?;
        let Value::Object(map) = root else {
            return Err(VersionError::NotAMap);
        };
        match map.get("version").and_then(Value::as_u64) {
            None => return Err(VersionError::MissingVersion),
            Some(1) => {}
            Some(other) => return Err(VersionError::UnknownVersion(other)),
        }
        let mut values = Values::new();
        let mut ignored = Vec::new();
        for (key, value) in &map {
            match key.as_str() {
                "version" => {}
                "kubeconfig" => ignore(&mut ignored, key, IgnoredKey::LocalPath),
                "notifications" | "follows" | "follow" => {
                    ignore(&mut ignored, key, IgnoredKey::Notifications)
                }
                "history" => ignore(&mut ignored, key, IgnoredKey::History),
                "agent" => ignore(&mut ignored, key, IgnoredKey::AgentAccess),
                "writes" | "write" | "write_model" => ignore(&mut ignored, key, IgnoredKey::Writes),
                "clusters" => clusters(value, &mut values, &mut ignored)?,
                _ => flatten(key, value, &mut values, &mut ignored),
            }
        }
        Ok(Version {
            id: id.into(),
            values,
            ignored,
            key_fingerprints,
        })
    }
}

fn ignore(ignored: &mut Vec<Ignored>, at: &str, why: IgnoredKey) {
    ignored.push(Ignored {
        at: at.to_string(),
        why,
    });
}

fn clusters(
    value: &Value,
    values: &mut Values,
    ignored: &mut Vec<Ignored>,
) -> Result<(), VersionError> {
    let Value::Array(list) = value else {
        return Err(VersionError::NotAMap);
    };
    let mut seen = std::collections::HashSet::new();
    for item in list {
        let Some(id) = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            return Err(VersionError::ClusterWithoutId);
        };
        if !seen.insert(id.to_string()) {
            return Err(VersionError::DuplicateCluster(id.to_string()));
        }
        let base = format!("clusters[{id}]");
        values.insert(base.clone(), Value::Bool(true));
        if let Value::Object(fields) = item {
            for (key, value) in fields {
                if key != "id" {
                    flatten(&format!("{base}.{key}"), value, values, ignored);
                }
            }
        }
    }
    Ok(())
}

fn flatten(at: &str, value: &Value, values: &mut Values, ignored: &mut Vec<Ignored>) {
    let last = at.rsplit('.').next().unwrap_or(at);
    let why = match last {
        "context" | "talos_context" => Some(IgnoredKey::Context),
        "talosconfig" | "ca_file" | "api_key_file" => Some(IgnoredKey::LocalPath),
        "token" | "key" => Some(IgnoredKey::KeyOrToken),
        "agent" => Some(IgnoredKey::AgentAccess),
        "writes" | "write" | "write_model" => Some(IgnoredKey::Writes),
        _ => None,
    };
    if let Some(why) = why {
        ignore(ignored, at, why);
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                flatten(&format!("{at}.{key}"), value, values, ignored);
            }
        }
        _ => {
            values.insert(at.to_string(), value.clone());
        }
    }
}
