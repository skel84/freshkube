//! A view as identities only (#873): a place inside one cluster, and the
//! `freshkube://open?…` link that carries it.
//!
//! The type is closed. It has no place for credentials, labels, YAML, log
//! text, filter text, a route or an action, so a link can neither open a
//! dialog nor start a shell, a forward, Operations or maintenance. Every
//! field is bounded and checked when it is built, and parsing is total: it
//! returns a `LinkError` and never panics.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{NodeTab, Tab};

/// The link's whole length, in bytes.
pub const LINK_MAX_BYTES: usize = 2048;
/// How many `key=value` pairs a link may hold, known or not.
pub const LINK_MAX_PARAMETERS: usize = 12;

const SCHEME: &str = "freshkube";
const HOST: &str = "open";
/// The newest version this build reads and writes.
const VERSION: u8 = 1;
/// A fingerprint token: `base32(sha256(…)[..16])`, lower case, unpadded.
const TOKEN_LEN: usize = 26;

/// Why a link was not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// Not a `freshkube://open` URL.
    NotALink,
    TooLong,
    TooManyParameters,
    /// A bad `%` escape, bytes that are not UTF-8, or a control character.
    BadEncoding,
    /// A known key given twice.
    Duplicate(&'static str),
    /// A key the page needs and the link lacks.
    Missing(&'static str),
    /// A value outside its bounds or alphabet.
    Invalid(&'static str),
    /// A key the page cannot carry.
    NotCarried(&'static str),
    /// Made by a newer Freshkube.
    NewerVersion(u8),
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotALink => f.write_str("not a Freshkube link"),
            Self::TooLong => f.write_str("the link is too long"),
            Self::TooManyParameters => f.write_str("the link has too many parameters"),
            Self::BadEncoding => f.write_str("the link is not valid text"),
            Self::Duplicate(key) => write!(f, "the link repeats `{key}`"),
            Self::Missing(key) => write!(f, "the link has no `{key}`"),
            Self::Invalid(key) => write!(f, "the link's `{key}` is not valid"),
            Self::NotCarried(key) => write!(f, "this page has no `{key}`"),
            Self::NewerVersion(_) => f.write_str("the link was made by a newer Freshkube"),
        }
    }
}

impl std::error::Error for LinkError {}

/// A bounded string over an alphabet, built only through `parse`.
macro_rules! token {
    ($(#[$meta:meta])* $name:ident, $field:literal, $max:expr, $check:expr) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: &str) -> Result<Self, LinkError> {
                let check: fn(&str) -> bool = $check;
                if value.is_empty() || value.len() > $max || !check(value) {
                    return Err(LinkError::Invalid($field));
                }
                Ok(Self(value.to_owned()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

fn is_dns_label(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
}

fn is_dns_subdomain(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-' || *b == b'.')
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
}

fn is_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        })
}

token!(
    /// A kubectl resource key (`pods`, `deployments.apps`). It is checked
    /// against the cluster's discovery when the link opens and grants nothing.
    KindKey, "kind", 253,
    |v| v.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
);
token!(
    /// A namespace: a DNS label.
    Namespace, "ns", 63, is_dns_label
);
token!(
    /// An object or node name: a DNS subdomain.
    Name, "name", 253, is_dns_subdomain
);
token!(
    /// A canonical lower-case UUID.
    ObjectUid, "uid", 36, is_uuid
);
token!(
    /// A cluster fingerprint: 26 characters of lower-case base32.
    FingerprintToken, "cluster", TOKEN_LEN,
    |v| v.len() == TOKEN_LEN && v.bytes().all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
);

/// The pages a link may open: the plain places. Not Settings, Operations or
/// maintenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkPage {
    Overview,
    Nodes,
    Health,
    Databases,
    Access,
    Resources,
    Etcd,
    SystemServices,
    Security,
    Lifecycle,
    Monitoring,
    Observability,
    Applications,
}

impl LinkPage {
    pub const ALL: [Self; 13] = [
        Self::Overview,
        Self::Nodes,
        Self::Health,
        Self::Databases,
        Self::Access,
        Self::Resources,
        Self::Etcd,
        Self::SystemServices,
        Self::Security,
        Self::Lifecycle,
        Self::Monitoring,
        Self::Observability,
        Self::Applications,
    ];

    /// The page's slug, as in `FRESHKUBE_PAGE`.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Nodes => "nodes",
            Self::Health => "health",
            Self::Databases => "databases",
            Self::Access => "access",
            Self::Resources => "resources",
            Self::Etcd => "etcd",
            Self::SystemServices => "system-services",
            Self::Security => "security",
            Self::Lifecycle => "lifecycle",
            Self::Monitoring => "monitoring",
            Self::Observability => "observability",
            Self::Applications => "applications",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|page| page.slug() == slug)
    }
}

/// How the object's pane opens. Details is the default and is left out of
/// the link; `Logs` opens the dock's logs and the pane stays on Details.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ViewTab {
    #[default]
    Details,
    Yaml,
    Events,
    Ports,
    Logs,
}

impl ViewTab {
    fn slug(self) -> &'static str {
        match self {
            Self::Details => "details",
            Self::Yaml => "yaml",
            Self::Events => "events",
            Self::Ports => "ports",
            Self::Logs => "logs",
        }
    }

    fn from_slug(slug: &str) -> Option<Self> {
        [
            Self::Details,
            Self::Yaml,
            Self::Events,
            Self::Ports,
            Self::Logs,
        ]
        .into_iter()
        .find(|tab| tab.slug() == slug)
    }

    /// The pane tab this opens.
    pub fn tab(self) -> Tab {
        match self {
            Self::Details => Tab::Overview,
            Self::Yaml => Tab::Yaml,
            Self::Events => Tab::Events,
            Self::Ports => Tab::Ports,
            Self::Logs => Tab::Logs,
        }
    }

    pub fn from_tab(tab: Tab) -> Self {
        match tab {
            Tab::Overview => Self::Details,
            Tab::Yaml => Self::Yaml,
            Tab::Events => Self::Events,
            Tab::Ports => Self::Ports,
            Tab::Logs => Self::Logs,
        }
    }
}

/// The object a view names: its name, and its UID when known.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ViewObject {
    pub name: Name,
    pub uid: Option<ObjectUid>,
}

/// A place inside one cluster. The cluster is not part of it, so a tab or an
/// agent's read can pair a view with its own entry.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ViewState {
    pub page: LinkPage,
    /// Resources only.
    pub kind: Option<KindKey>,
    /// Resources only, with a kind.
    pub ns: Option<Namespace>,
    /// Resources only, with a kind.
    pub object: Option<ViewObject>,
    /// Resources only, with an object.
    pub tab: ViewTab,
    /// Nodes only.
    pub node: Option<Name>,
    /// Nodes only, with a node.
    pub node_tab: NodeTab,
}

impl ViewState {
    pub fn page(page: LinkPage) -> Self {
        Self {
            page,
            kind: None,
            ns: None,
            object: None,
            tab: ViewTab::Details,
            node: None,
            node_tab: NodeTab::Overview,
        }
    }

    /// The fields the page can carry, checked as one: a page with a field it
    /// cannot carry is an error, not a silent drop.
    pub fn check(&self) -> Result<(), LinkError> {
        let resources = self.page == LinkPage::Resources;
        let nodes = self.page == LinkPage::Nodes;
        if !resources {
            if self.kind.is_some() {
                return Err(LinkError::NotCarried("kind"));
            }
            if self.ns.is_some() {
                return Err(LinkError::NotCarried("ns"));
            }
            if self.object.is_some() {
                return Err(LinkError::NotCarried("name"));
            }
            if self.tab != ViewTab::Details {
                return Err(LinkError::NotCarried("tab"));
            }
        }
        if !nodes {
            if self.node.is_some() {
                return Err(LinkError::NotCarried("node"));
            }
            if self.node_tab != NodeTab::Overview {
                return Err(LinkError::NotCarried("ntab"));
            }
        }
        if self.kind.is_none() {
            if self.ns.is_some() {
                return Err(LinkError::Missing("kind"));
            }
            if self.object.is_some() {
                return Err(LinkError::Missing("kind"));
            }
        }
        if self.object.is_none() && self.tab != ViewTab::Details {
            return Err(LinkError::Missing("name"));
        }
        if self.node.is_none() && self.node_tab != NodeTab::Overview {
            return Err(LinkError::Missing("node"));
        }
        Ok(())
    }

    /// The canonical query: fixed key order, defaults left out.
    pub fn to_query(&self) -> String {
        let mut out = Vec::new();
        self.push_pairs(&mut out);
        out.join("&")
    }

    fn push_pairs(&self, out: &mut Vec<String>) {
        out.push(format!("page={}", self.page.slug()));
        if let Some(kind) = &self.kind {
            out.push(format!("kind={}", kind.as_str()));
        }
        if let Some(ns) = &self.ns {
            out.push(format!("ns={}", ns.as_str()));
        }
        if let Some(object) = &self.object {
            out.push(format!("name={}", object.name.as_str()));
            if let Some(uid) = &object.uid {
                out.push(format!("uid={}", uid.as_str()));
            }
        }
        if self.tab != ViewTab::Details {
            out.push(format!("tab={}", self.tab.slug()));
        }
        if let Some(node) = &self.node {
            out.push(format!("node={}", node.as_str()));
        }
        if self.node_tab != NodeTab::Overview {
            out.push(format!("ntab={}", self.node_tab.slug()));
        }
    }

    /// Reads a query (`page=…&kind=…`). Unknown keys are dropped; a known key
    /// given twice refuses the link.
    pub fn parse_query(query: &str) -> Result<Self, LinkError> {
        Ok(Link::parse_pairs(query)?.view)
    }
}

/// Which cluster a link was made in: fingerprints, never a context name.
/// Either may be absent; a link with neither opens on the open cluster.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ClusterRef {
    /// A salted hash of the `kube-system` namespace UID, or of a setup-file
    /// override. Carried as `cluster=u-<token>`.
    pub strong: Option<FingerprintToken>,
    /// A salted hash of the cluster's CA certificates. Carried as `ca=`.
    pub ca: Option<FingerprintToken>,
}

/// The `freshkube://open?…` envelope: a cluster and a view in it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Link {
    pub cluster: ClusterRef,
    pub view: ViewState,
}

impl Link {
    /// A link on the open cluster: no fingerprint.
    pub fn here(view: ViewState) -> Self {
        Self {
            cluster: ClusterRef::default(),
            view,
        }
    }

    /// Reads a link. Total: every input is a `Link` or a `LinkError`.
    pub fn parse(url: &str) -> Result<Self, LinkError> {
        let url = url.trim();
        if url.len() > LINK_MAX_BYTES {
            return Err(LinkError::TooLong);
        }
        let rest = url
            .strip_prefix(SCHEME)
            .and_then(|rest| rest.strip_prefix("://"))
            .and_then(|rest| rest.strip_prefix(HOST))
            .ok_or(LinkError::NotALink)?;
        let query = rest
            .strip_prefix("/?")
            .or_else(|| rest.strip_prefix('?'))
            .ok_or(LinkError::NotALink)?;
        if query.contains('#') {
            return Err(LinkError::NotALink);
        }
        Self::parse_pairs(query)
    }

    fn parse_pairs(query: &str) -> Result<Self, LinkError> {
        let mut fields = Fields::default();
        let mut count = 0;
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            count += 1;
            if count > LINK_MAX_PARAMETERS {
                return Err(LinkError::TooManyParameters);
            }
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let key = decode(key)?;
            let value = decode(value)?;
            fields.take(&key, value)?;
        }
        fields.finish()
    }

    /// The canonical URL.
    pub fn to_url(&self) -> String {
        let mut out = Vec::new();
        if let Some(strong) = &self.cluster.strong {
            out.push(format!("cluster=u-{}", strong.as_str()));
        }
        if let Some(ca) = &self.cluster.ca {
            out.push(format!("ca={}", ca.as_str()));
        }
        self.view.push_pairs(&mut out);
        format!("{SCHEME}://{HOST}?{}", out.join("&"))
    }
}

/// The values a query held, before the page decides what it may carry.
#[derive(Default)]
struct Fields {
    version: Option<String>,
    cluster: Option<String>,
    ca: Option<String>,
    page: Option<String>,
    kind: Option<String>,
    ns: Option<String>,
    name: Option<String>,
    uid: Option<String>,
    tab: Option<String>,
    node: Option<String>,
    ntab: Option<String>,
}

impl Fields {
    fn take(&mut self, key: &str, value: String) -> Result<(), LinkError> {
        let (slot, name) = match key {
            "v" => (&mut self.version, "v"),
            "cluster" => (&mut self.cluster, "cluster"),
            "ca" => (&mut self.ca, "ca"),
            "page" => (&mut self.page, "page"),
            "kind" => (&mut self.kind, "kind"),
            "ns" => (&mut self.ns, "ns"),
            "name" => (&mut self.name, "name"),
            "uid" => (&mut self.uid, "uid"),
            "tab" => (&mut self.tab, "tab"),
            "node" => (&mut self.node, "node"),
            "ntab" => (&mut self.ntab, "ntab"),
            // Unknown keys are dropped: no route, action or extra survives.
            _ => return Ok(()),
        };
        if slot.is_some() {
            return Err(LinkError::Duplicate(name));
        }
        *slot = Some(value);
        Ok(())
    }

    fn finish(self) -> Result<Link, LinkError> {
        if let Some(version) = &self.version {
            match version.parse::<u8>() {
                Ok(v) if v == VERSION => {}
                Ok(v) if v > VERSION => return Err(LinkError::NewerVersion(v)),
                _ => return Err(LinkError::Invalid("v")),
            }
        }
        let page = self.page.ok_or(LinkError::Missing("page"))?;
        let page = LinkPage::from_slug(&page).ok_or(LinkError::Invalid("page"))?;
        let strong = match self.cluster {
            None => None,
            Some(value) => {
                let token = value
                    .strip_prefix("u-")
                    .ok_or(LinkError::Invalid("cluster"))?;
                Some(FingerprintToken::parse(token).map_err(|_| LinkError::Invalid("cluster"))?)
            }
        };
        let ca = match self.ca {
            None => None,
            Some(value) => {
                Some(FingerprintToken::parse(&value).map_err(|_| LinkError::Invalid("ca"))?)
            }
        };
        let object = match self.name {
            None => {
                if self.uid.is_some() {
                    return Err(LinkError::Missing("name"));
                }
                None
            }
            Some(name) => Some(ViewObject {
                name: Name::parse(&name)?,
                uid: self.uid.as_deref().map(ObjectUid::parse).transpose()?,
            }),
        };
        let tab = match self.tab {
            None => ViewTab::Details,
            Some(tab) => ViewTab::from_slug(&tab).ok_or(LinkError::Invalid("tab"))?,
        };
        let node_tab = match self.ntab {
            None => NodeTab::Overview,
            Some(tab) => NodeTab::from_slug(&tab).ok_or(LinkError::Invalid("ntab"))?,
        };
        let view = ViewState {
            page,
            kind: self.kind.as_deref().map(KindKey::parse).transpose()?,
            ns: self.ns.as_deref().map(Namespace::parse).transpose()?,
            object,
            tab,
            node: self.node.as_deref().map(Name::parse).transpose()?,
            node_tab,
        };
        view.check()?;
        Ok(Link {
            cluster: ClusterRef { strong, ca },
            view,
        })
    }
}

/// Percent-decoding that takes only UTF-8 and refuses control characters.
fn decode(text: &str) -> Result<String, LinkError> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3).ok_or(LinkError::BadEncoding)?;
            let hex = std::str::from_utf8(hex).map_err(|_| LinkError::BadEncoding)?;
            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(LinkError::BadEncoding);
            }
            out.push(u8::from_str_radix(hex, 16).map_err(|_| LinkError::BadEncoding)?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let decoded = String::from_utf8(out).map_err(|_| LinkError::BadEncoding)?;
    if decoded.chars().any(char::is_control) {
        return Err(LinkError::BadEncoding);
    }
    Ok(decoded)
}

impl Serialize for ViewState {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_query())
    }
}

impl<'de> Deserialize<'de> for ViewState {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let query = String::deserialize(deserializer)?;
        Self::parse_query(&query).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests;
