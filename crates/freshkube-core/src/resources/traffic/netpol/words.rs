//! Rules and peers in words, for the pane. AND within one entry, OR across
//! entries.
use super::facts::{NamespaceScope, Peer, PolicyFacts, PortRef, PortSpec, Rule};
use super::select::selector_text;

impl Peer {
    pub fn describe(&self) -> String {
        match self {
            Peer::Invalid => "an entry with no selector or ipBlock (invalid, not judged)".into(),
            Peer::IpBlock { cidr, except } if except.is_empty() => format!("IP range {cidr}"),
            Peer::IpBlock { cidr, except } => {
                format!("IP range {cidr} except {}", except.join(", "))
            }
            Peer::Pods { namespaces, pods } => {
                let pods = match pods.as_ref().map(selector_text) {
                    None | Some(None) => "any pod".to_owned(),
                    Some(Some(text)) => format!("pods {text}"),
                };
                let ns = match namespaces {
                    NamespaceScope::Own => "in this namespace".to_owned(),
                    NamespaceScope::Selector(sel) => match selector_text(sel) {
                        None => "in any namespace".to_owned(),
                        Some(text) => format!("in namespaces {text}"),
                    },
                };
                format!("{pods} {ns}")
            }
        }
    }
}

impl PortSpec {
    pub fn describe(&self) -> String {
        match (&self.port, self.end_port) {
            (None, _) => format!("any {} port", self.protocol),
            (Some(PortRef::Number(n)), Some(end)) => format!("{} {n}-{end}", self.protocol),
            (Some(PortRef::Number(n)), None) => format!("{} {n}", self.protocol),
            (Some(PortRef::Name(name)), _) => format!("{} port {name}", self.protocol),
        }
    }
}

impl Rule {
    /// `peers` and `ports` as two lines: "from/to" and "on".
    pub fn describe(&self, peers_word: &str) -> String {
        let peers = match &self.peers {
            None => format!("{peers_word} anything"),
            Some(list) => format!(
                "{peers_word} {}",
                list.iter()
                    .map(Peer::describe)
                    .collect::<Vec<_>>()
                    .join("; or ")
            ),
        };
        let ports = match &self.ports {
            None => "on any port".to_owned(),
            Some(list) => format!(
                "on {}",
                list.iter()
                    .map(PortSpec::describe)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        format!("{peers} {ports}")
    }
}

impl PolicyFacts {
    pub fn selector_words(&self) -> String {
        selector_text(&self.pod_selector).unwrap_or_else(|| "every pod in the namespace".into())
    }
}
