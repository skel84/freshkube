//! The workspace's `destinations`: which workspace entry an Argo CD
//! destination is, as the person said. An Application records its
//! destination by server URL or by Argo CD's name for the cluster; a row
//! names one of the two and the entry it is. Nothing is inferred from
//! kubeconfig servers, and a destination no row names stays unknown.
//!
//! A row may name an entry the workspace no longer lists: it is kept, so it
//! can be pointed elsewhere, and resolves to nothing until it is.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use super::{Chosen, Invalid, Part, Workspace};

/// The name Argo CD gives the cluster it runs in.
pub const IN_CLUSTER_NAME: &str = "in-cluster";
/// The address Argo CD gives the cluster it runs in, as seen from inside it.
pub const IN_CLUSTER_SERVER: &str = "https://kubernetes.default.svc";

/// `https://Host:443/` and `https://host` name the same server.
pub fn normalize_server(server: &str) -> String {
    let lower = server.trim().to_ascii_lowercase();
    let trimmed = lower.trim_end_matches('/');
    trimmed
        .strip_suffix(":443")
        .filter(|rest| rest.starts_with("https://"))
        .unwrap_or(trimmed)
        .to_owned()
}

/// The most destinations one workspace maps.
pub const MAX_DESTINATIONS: usize = 256;

/// How a row matches an Application's destination.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    /// `spec.destination.server`, compared without case, a trailing slash
    /// or an explicit `:443`.
    Server(String),
    /// `spec.destination.name`, compared exactly.
    Name(String),
}

impl Key {
    /// `server https://…` or `name prod-lon`, as a row is named in words.
    pub fn describe(&self) -> String {
        match self {
            Key::Server(server) => format!("server {server}"),
            Key::Name(name) => format!("name {name}"),
        }
    }

    /// The server or name alone.
    pub fn value(&self) -> &str {
        match self {
            Key::Server(value) | Key::Name(value) => value,
        }
    }

    /// Whether both match the same destination, as an Application is
    /// matched: servers without case, a trailing slash or an explicit
    /// `:443`, names exactly.
    pub fn same(&self, other: &Key) -> bool {
        match (self, other) {
            (Key::Server(a), Key::Server(b)) => normalize_server(a) == normalize_server(b),
            (Key::Name(a), Key::Name(b)) => a == b,
            _ => false,
        }
    }
}

/// One mapping: an Argo CD destination, and the workspace entry it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Destination {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// The workspace entry's id. A row without one is read, and refused
    /// by [`Destination::fault`] with its row named.
    #[serde(default)]
    pub entry: String,
    /// Keys this version doesn't know, written back as they were read.
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

impl Destination {
    pub fn new(key: Key, entry: impl Into<String>) -> Self {
        let (server, name) = match key {
            Key::Server(server) => (Some(server), None),
            Key::Name(name) => (None, Some(name)),
        };
        Self {
            server,
            name,
            entry: entry.into(),
            extra: Default::default(),
        }
    }

    /// What the row matches. A row that names both is refused when the file
    /// is read; were one to get here, its server would win, as it does in
    /// `argocd::match_destination` (Argo CD refuses an Application that
    /// names both).
    pub fn key(&self) -> Key {
        match (&self.server, &self.name) {
            (Some(server), _) => Key::Server(server.clone()),
            (None, Some(name)) => Key::Name(name.clone()),
            (None, None) => Key::Name(String::new()),
        }
    }

    /// Replaces what the row matches, keeping its entry and unknown keys.
    pub fn set_key(&mut self, key: Key) {
        let entry = std::mem::take(&mut self.entry);
        let extra = std::mem::take(&mut self.extra);
        *self = Self {
            extra,
            ..Self::new(key, entry)
        };
    }

    /// Why the row can't be read on its own. [`Self::fault_in`] also checks
    /// that an in-cluster alias names the workspace's Argo CD source.
    pub fn fault(&self) -> Option<Fault> {
        let server = self.server.as_deref().map(str::trim);
        let name = self.name.as_deref().map(str::trim);
        let key = match (server, name) {
            (Some(_), Some(_)) => Some(Fault::ServerAndName),
            (None, None) => Some(Fault::Neither),
            (Some(""), None) => Some(Fault::EmptyServer),
            (None, Some("")) => Some(Fault::EmptyName),
            // Matched as written, a padded value would never match.
            (Some(value), None) | (None, Some(value))
                if Some(value) != self.server.as_deref().or(self.name.as_deref()) =>
            {
                Some(Fault::Spaces)
            }
            (Some(server), None) => server_fault(server),
            (None, Some(_)) => None,
        };
        key.or_else(|| self.entry.trim().is_empty().then_some(Fault::NoEntry))
    }

    /// Checks the row against the same source selection delivery uses.
    /// Neither an alias nor its target selects where Argo CD is read.
    pub fn fault_in(&self, workspace: &Workspace) -> Option<Fault> {
        if let Some(fault) = self.fault() {
            return Some(fault);
        }
        let own = match self.key() {
            Key::Server(server) => normalize_server(&server) == IN_CLUSTER_SERVER,
            Key::Name(name) => name == IN_CLUSTER_NAME,
        };
        if !own {
            return None;
        }
        match super::sources::choose(workspace, Part::ArgoCd) {
            Chosen::Entry { entry, .. } => {
                (entry.id != self.entry).then_some(Fault::ArgoCdOther(entry.id))
            }
            Chosen::Open | Chosen::NoEntry(_) | Chosen::Gone(_) => Some(Fault::ArgoCdUnknown),
        }
    }

    /// The row as an error names it: `destinations[2] (server https://…)`,
    /// counting from 0 as the file's array does.
    fn named(&self, index: usize) -> String {
        let what = match (&self.server, &self.name) {
            (Some(server), _) if !server.trim().is_empty() => format!(" (server {server})"),
            (_, Some(name)) if !name.trim().is_empty() => format!(" (name {name})"),
            _ => String::new(),
        };
        format!("destinations[{index}]{what}")
    }

    /// The row's unknown keys, as `destinations[i].key`.
    pub(super) fn unknown_keys(&self, index: usize) -> impl Iterator<Item = String> + '_ {
        self.extra
            .keys()
            .map(move |key| format!("destinations[{index}].{key}"))
    }
}

/// Why a server can't be mapped: it isn't an http or https address.
fn server_fault(server: &str) -> Option<Fault> {
    let lower = server.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        Some(Fault::NotHttp)
    } else {
        None
    }
}

/// Why a destination row is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    ServerAndName,
    Neither,
    EmptyServer,
    EmptyName,
    /// A server or name with spaces around it.
    Spaces,
    NotHttp,
    /// An in-cluster alias maps to a different entry than this selected
    /// Argo CD source entry id.
    ArgoCdOther(String),
    /// No workspace entry is selected as Argo CD's source.
    ArgoCdUnknown,
    NoEntry,
    /// Not an object of strings: a number for a server, say.
    Unreadable,
}

impl Fault {
    pub fn words(&self) -> Cow<'static, str> {
        let words = match self {
            Fault::ServerAndName => "names both a server and a name",
            Fault::Neither => "names no server or name",
            Fault::EmptyServer => "names an empty server",
            Fault::EmptyName => "names an empty name",
            Fault::Spaces => "names a server or name with spaces around it",
            Fault::NotHttp => "names a server that isn’t an http or https address",
            Fault::ArgoCdOther(entry) => {
                return format!(
                    "names Argo CD's own cluster but maps it to another entry; use {entry}, the selected Argo CD source entry"
                ).into();
            }
            Fault::ArgoCdUnknown => {
                "names Argo CD's own cluster but its host entry cannot be determined; set sources.argocd to a usable cluster entry"
            }
            Fault::NoEntry => "names no workspace cluster",
            Fault::Unreadable => {
                "isn’t a server or name and an entry, each a string, so it can’t be read"
            }
        };
        words.into()
    }
}

/// Reads the `destinations` array one row at a time, so a row that isn't
/// the right shape is named, as a row that can't be used is.
pub(super) fn read(value: serde_json::Value) -> Result<Vec<Destination>, Invalid> {
    let serde_json::Value::Array(rows) = value else {
        return Err(Invalid::Malformed("destinations isn’t a list".into()));
    };
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| {
            let text = |key: &str| row.get(key).and_then(|v| v.as_str()).map(str::to_owned);
            let (server, name) = (text("server"), text("name"));
            serde_json::from_value(row).map_err(|_| Invalid::Destination {
                row: Destination {
                    server,
                    name,
                    entry: String::new(),
                    extra: Default::default(),
                }
                .named(index),
                fault: Fault::Unreadable,
            })
        })
        .collect()
}

/// Checks every row, naming the first one that can't be used.
pub(super) fn validate(workspace: &Workspace) -> Result<(), Invalid> {
    let rows = &workspace.destinations;
    if rows.len() > MAX_DESTINATIONS {
        return Err(Invalid::TooManyDestinations);
    }
    for (index, row) in rows.iter().enumerate() {
        if let Some(fault) = row.fault_in(workspace) {
            return Err(Invalid::Destination {
                row: row.named(index),
                fault,
            });
        }
    }
    Ok(())
}
