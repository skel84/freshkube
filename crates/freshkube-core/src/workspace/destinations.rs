//! The workspace's `destinations`: which workspace entry an Argo CD
//! destination is, as the person said. An Application records its
//! destination by server URL or by Argo CD's name for the cluster; a row
//! names one of the two and the entry it is. Nothing is inferred from
//! kubeconfig servers, and a destination no row names stays unknown.
//!
//! A row may name an entry the workspace no longer lists: it is kept, so it
//! can be pointed elsewhere, and resolves to nothing until it is.

use serde::{Deserialize, Serialize};

use super::Invalid;

/// The most destinations one workspace maps.
pub const MAX_DESTINATIONS: usize = 256;

/// Argo CD's own names for the cluster it runs in. It knows them already, so
/// they can't be mapped. Kept equal to `delivery::argocd`'s.
const ARGOCD_OWN_NAME: &str = "in-cluster";
const ARGOCD_OWN_SERVER: &str = "https://kubernetes.default.svc";

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
}

/// One mapping: an Argo CD destination, and the workspace entry it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Destination {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// The workspace entry's id.
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
    /// Argo CD.
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

    /// Why the row can't be used, if it can't.
    pub fn fault(&self) -> Option<Fault> {
        let server = self.server.as_deref().map(str::trim);
        let name = self.name.as_deref().map(str::trim);
        let key = match (server, name) {
            (Some(_), Some(_)) => Some(Fault::ServerAndName),
            (None, None) => Some(Fault::Neither),
            (Some(""), None) => Some(Fault::EmptyServer),
            (None, Some("")) => Some(Fault::EmptyName),
            (Some(server), None) => server_fault(server),
            (None, Some(name)) => (name == ARGOCD_OWN_NAME).then_some(Fault::ArgoCdOwn),
        };
        key.or_else(|| self.entry.trim().is_empty().then_some(Fault::NoEntry))
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

/// Why a server can't be mapped: it isn't an http or https address, or it
/// is Argo CD's own.
fn server_fault(server: &str) -> Option<Fault> {
    let lower = server.to_ascii_lowercase();
    let bare = lower.trim_end_matches('/');
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        Some(Fault::NotHttp)
    } else if bare == ARGOCD_OWN_SERVER || bare == format!("{ARGOCD_OWN_SERVER}:443") {
        Some(Fault::ArgoCdOwn)
    } else {
        None
    }
}

/// Why a destination row is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    ServerAndName,
    Neither,
    EmptyServer,
    EmptyName,
    NotHttp,
    ArgoCdOwn,
    NoEntry,
}

impl Fault {
    pub fn words(self) -> &'static str {
        match self {
            Fault::ServerAndName => "names both a server and a name",
            Fault::Neither => "names no server or name",
            Fault::EmptyServer => "names an empty server",
            Fault::EmptyName => "names an empty name",
            Fault::NotHttp => "names a server that isn’t an http or https address",
            Fault::ArgoCdOwn => "names Argo CD’s own cluster, which is known already",
            Fault::NoEntry => "names no workspace cluster",
        }
    }
}

/// Checks every row, naming the first one that can't be used.
pub(super) fn validate(rows: &[Destination]) -> Result<(), Invalid> {
    if rows.len() > MAX_DESTINATIONS {
        return Err(Invalid::TooManyDestinations);
    }
    for (index, row) in rows.iter().enumerate() {
        if let Some(fault) = row.fault() {
            return Err(Invalid::Destination {
                row: row.named(index),
                fault,
            });
        }
    }
    Ok(())
}
