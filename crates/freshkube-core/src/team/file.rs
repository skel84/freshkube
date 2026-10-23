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
    NotYaml {
        line: usize,
        column: usize,
    },
    NotAMap,
    MissingVersion,
    UnknownVersion(u64),
    ClusterWithoutId,
    /// A cluster id that is empty-looking, long, or not letters, digits, `-`
    /// and `_`. The id is never repeated.
    BadClusterId,
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
            VersionError::BadClusterId => {
                f.write_str("a team cluster id may only use letters, digits, - and _ (64 at most)")
            }
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
    /// The person's own mapping of Argo CD destinations.
    Destinations,
    /// A name that could address a path of its own (`.`, `[`, `]` or `:`).
    BadKey,
    /// Not a setting a team may set, or not of the shape it takes.
    Unknown,
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
            IgnoredKey::Destinations => "your Argo CD destinations are yours",
            IgnoredKey::BadKey => "that name isn’t allowed in a key",
            IgnoredKey::Unknown => "a team may not set this",
        }
    }
}

/// A key the file set that this app ignores, with where it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ignored {
    pub at: String,
    pub why: IgnoredKey,
}

/// What a team may set, as a tree. Anything not on it is ignored.
enum Allow {
    /// A string, a number or a boolean.
    Leaf,
    Map(&'static [(&'static str, Allow)]),
}

use Allow::{Leaf, Map};

const SERVICE: Allow = Map(&[
    ("namespace", Leaf),
    ("name", Leaf),
    ("port", Leaf),
    ("https", Leaf),
    ("path", Leaf),
]);

const CLUSTER: &[(&str, Allow)] = &[
    ("role", Leaf),
    ("protection", Leaf),
    ("fingerprint", Leaf),
    ("coroot", Map(&[("project", Leaf), ("cluster", Leaf)])),
    (
        "metrics",
        Map(&[
            ("value", Leaf),
            ("source", Map(&[("service", SERVICE), ("url", Leaf)])),
        ]),
    ),
    (
        "results",
        Map(&[
            ("address", Leaf),
            (
                "forward",
                Map(&[("namespace", Leaf), ("service", Leaf), ("port", Leaf)]),
            ),
        ]),
    ),
];

const TOP: &[(&str, Allow)] = &[
    (
        "coroot",
        Map(&[("server", Leaf), ("sign_in", Leaf), ("project", Leaf)]),
    ),
    (
        "supply_chain",
        Map(&[
            ("public_key_file", Leaf),
            ("transparency_log", Leaf),
            ("transparency_url", Leaf),
            (
                "evidence",
                Map(&[("result", Leaf), ("commit", Leaf), ("digest", Leaf)]),
            ),
        ]),
    ),
    (
        "metrics",
        Map(&[
            ("mode", Leaf),
            ("label", Leaf),
            (
                "store",
                Map(&[("entry", Leaf), ("service", SERVICE), ("url", Leaf)]),
            ),
        ]),
    ),
    (
        "delivery",
        Map(&[
            ("build_namespace", Leaf),
            (
                "stage_naming",
                Map(&[("name_template", Leaf), ("project_key", Leaf)]),
            ),
        ]),
    ),
    (
        "sources",
        Map(&[("kargo", Leaf), ("builds", Leaf), ("argocd", Leaf)]),
    ),
];

/// A name with a meaning of its own, whatever level it is found at.
fn named(key: &str) -> Option<IgnoredKey> {
    Some(match key {
        "context" | "talos_context" => IgnoredKey::Context,
        "kubeconfig" | "talosconfig" | "ca_file" | "api_key_file" => IgnoredKey::LocalPath,
        "token" | "key" => IgnoredKey::KeyOrToken,
        "agent" => IgnoredKey::AgentAccess,
        "writes" | "write" | "write_model" => IgnoredKey::Writes,
        "notifications" | "follows" | "follow" => IgnoredKey::Notifications,
        "history" => IgnoredKey::History,
        "destinations" => IgnoredKey::Destinations,
        _ => return None,
    })
}

fn key_ok(key: &str) -> bool {
    !key.is_empty() && !key.contains(['.', '[', ']', ':'])
}

/// A cluster id: short, and nothing that could address another path.
fn cluster_id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// One version of the team file: an id (a commit, or the content's hash), the
/// values it sets and what it set that is ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    id: String,
    values: Values,
    ignored: Vec<Ignored>,
    /// Key files named by `supply_chain.public_key_file`: path → fingerprint of
    /// its contents, read at this version by the fetcher.
    key_fingerprints: BTreeMap<String, String>,
}

/// The one key whose value is a path inside the team repository.
pub const KEY_FILE: &str = "supply_chain.public_key_file";

impl Version {
    /// The commit or content hash this version is.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// What the team sets, flat.
    pub fn values(&self) -> &Values {
        &self.values
    }

    /// What the file set that is ignored.
    pub fn ignored(&self) -> &[Ignored] {
        &self.ignored
    }

    pub fn key_fingerprints(&self) -> &BTreeMap<String, String> {
        &self.key_fingerprints
    }

    /// The only way to make a version: the allowlist and the id check run here.
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
            if key == "version" {
                continue;
            }
            if key == "clusters" {
                clusters(value, &mut values, &mut ignored)?;
            } else {
                walk(key, key, value, &Map(TOP), &mut values, &mut ignored);
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
        if !cluster_id_ok(id) {
            return Err(VersionError::BadClusterId);
        }
        if !seen.insert(id.to_string()) {
            return Err(VersionError::DuplicateCluster(id.to_string()));
        }
        let base = format!("clusters[{id}]");
        values.insert(base.clone(), Value::Bool(true));
        if let Value::Object(fields) = item {
            for (key, value) in fields {
                if key != "id" {
                    walk(
                        &format!("{base}.{key}"),
                        key,
                        value,
                        &Map(CLUSTER),
                        values,
                        ignored,
                    );
                }
            }
        }
    }
    Ok(())
}

/// Takes `value` at `at` (named `key`) if `allow` lets a team set it; names it
/// as ignored otherwise.
fn walk(
    at: &str,
    key: &str,
    value: &Value,
    parent: &Allow,
    values: &mut Values,
    ignored: &mut Vec<Ignored>,
) {
    if !key_ok(key) {
        return ignore(ignored, at, IgnoredKey::BadKey);
    }
    if let Some(why) = named(key) {
        return ignore(ignored, at, why);
    }
    let Map(children) = parent else {
        return ignore(ignored, at, IgnoredKey::Unknown);
    };
    let Some((_, allow)) = children.iter().find(|(name, _)| *name == key) else {
        return ignore(ignored, at, IgnoredKey::Unknown);
    };
    match (allow, value) {
        (Map(_), Value::Object(map)) => {
            for (child, value) in map {
                walk(
                    &format!("{at}.{child}"),
                    child,
                    value,
                    allow,
                    values,
                    ignored,
                );
            }
        }
        (Leaf, Value::String(text)) if text.len() <= MAX_VALUE => {
            values.insert(at.to_string(), value.clone());
        }
        (Leaf, Value::Number(_) | Value::Bool(_)) => {
            values.insert(at.to_string(), value.clone());
        }
        _ => ignore(ignored, at, IgnoredKey::Unknown),
    }
}

/// The longest string a team value may be.
const MAX_VALUE: usize = 2048;
