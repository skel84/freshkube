//! A NetworkPolicy as its spec states it. AND within a peer entry
//! (namespaceSelector with podSelector), OR across entries; an absent or
//! empty `from`/`to` or `ports` list matches everything.
use k8s_openapi::api::networking::v1::{
    NetworkPolicy, NetworkPolicyEgressRule, NetworkPolicyIngressRule, NetworkPolicyPeer,
    NetworkPolicyPort,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Ingress,
    Egress,
}

impl Direction {
    pub const ALL: [Direction; 2] = [Direction::Ingress, Direction::Egress];

    pub fn label(self) -> &'static str {
        match self {
            Direction::Ingress => "Ingress",
            Direction::Egress => "Egress",
        }
    }
}

/// What a direction of a policy, or of all the policies on a pod together,
/// comes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectionVerdict {
    /// No policy governs this direction: it is open, by Kubernetes' default.
    NotGoverned,
    /// Governed, with no rule that allows anything: default deny.
    DeniesAll,
    /// Governed, with a rule that has no peers and no ports.
    OpensToAll,
    /// Governed, and only what its rules list is allowed.
    Restricts,
    /// A selector or a read could not be judged.
    Unknown,
}

impl DirectionVerdict {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotGoverned => "Not governed",
            Self::DeniesAll => "Denies everything",
            Self::OpensToAll => "Opens to everything",
            Self::Restricts => "Restricts",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NamespaceScope {
    /// No namespaceSelector: the policy's own namespace.
    Own,
    /// A namespaceSelector; an empty one is every namespace.
    Selector(LabelSelector),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Peer {
    /// namespaceSelector and/or podSelector, ANDed.
    Pods {
        namespaces: NamespaceScope,
        pods: Option<LabelSelector>,
    },
    IpBlock {
        cidr: String,
        except: Vec<String>,
    },
    /// An entry with none of the three: invalid, so never judged.
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortRef {
    Number(i32),
    /// A container port's name, resolved on the destination pod.
    Name(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortSpec {
    /// Upper-case; TCP when the policy leaves it out.
    pub protocol: String,
    /// `None`: every port of that protocol.
    pub port: Option<PortRef>,
    pub end_port: Option<i32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    /// `None`: every peer.
    pub peers: Option<Vec<Peer>>,
    /// `None`: every port.
    pub ports: Option<Vec<PortSpec>>,
}

impl Rule {
    pub fn is_open(&self) -> bool {
        self.peers.is_none() && self.ports.is_none()
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PolicyFacts {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub pod_selector: LabelSelector,
    pub governs_ingress: bool,
    pub governs_egress: bool,
    pub ingress: Vec<Rule>,
    pub egress: Vec<Rule>,
}

fn peer(peer: &NetworkPolicyPeer) -> Peer {
    if let Some(block) = &peer.ip_block {
        return Peer::IpBlock {
            cidr: block.cidr.clone(),
            except: block.except.clone().unwrap_or_default(),
        };
    }
    if peer.namespace_selector.is_none() && peer.pod_selector.is_none() {
        return Peer::Invalid;
    }
    Peer::Pods {
        namespaces: peer
            .namespace_selector
            .clone()
            .map_or(NamespaceScope::Own, NamespaceScope::Selector),
        pods: peer.pod_selector.clone(),
    }
}

fn port(port: &NetworkPolicyPort) -> PortSpec {
    PortSpec {
        protocol: port
            .protocol
            .as_deref()
            .unwrap_or("TCP")
            .to_ascii_uppercase(),
        port: port.port.as_ref().map(|p| match p {
            IntOrString::Int(number) => PortRef::Number(*number),
            IntOrString::String(name) => PortRef::Name(name.clone()),
        }),
        end_port: port.end_port,
    }
}

fn rule(peers: Option<&Vec<NetworkPolicyPeer>>, ports: Option<&Vec<NetworkPolicyPort>>) -> Rule {
    Rule {
        peers: peers
            .filter(|p| !p.is_empty())
            .map(|p| p.iter().map(peer).collect()),
        ports: ports
            .filter(|p| !p.is_empty())
            .map(|p| p.iter().map(port).collect()),
    }
}

fn ingress_rule(r: &NetworkPolicyIngressRule) -> Rule {
    rule(r.from.as_ref(), r.ports.as_ref())
}

fn egress_rule(r: &NetworkPolicyEgressRule) -> Rule {
    rule(r.to.as_ref(), r.ports.as_ref())
}

impl PolicyFacts {
    /// A policy from its YAML, as a pane's document holds it.
    pub fn from_yaml(yaml: &str) -> Result<Self, crate::failure::Failure> {
        serde_yaml::from_str::<NetworkPolicy>(yaml)
            .map(|policy| Self::from_policy(&policy))
            .map_err(|error| {
                crate::failure::Failure::new(
                    crate::failure::FailureKind::Other,
                    format!("the policy could not be read: {error}"),
                )
            })
    }

    pub fn from_policy(policy: &NetworkPolicy) -> Self {
        let spec = policy.spec.clone().unwrap_or_default();
        let types = spec.policy_types.clone().unwrap_or_default();
        // The API server's default: Ingress always, Egress when egress rules exist.
        let (governs_ingress, governs_egress) = if types.is_empty() {
            (true, spec.egress.as_ref().is_some_and(|e| !e.is_empty()))
        } else {
            (
                types.iter().any(|t| t == "Ingress"),
                types.iter().any(|t| t == "Egress"),
            )
        };
        Self {
            namespace: policy.metadata.namespace.clone().unwrap_or_default(),
            name: policy.metadata.name.clone().unwrap_or_default(),
            uid: policy.metadata.uid.clone().unwrap_or_default(),
            pod_selector: spec.pod_selector,
            governs_ingress,
            governs_egress,
            ingress: spec.ingress.iter().flatten().map(ingress_rule).collect(),
            egress: spec.egress.iter().flatten().map(egress_rule).collect(),
        }
    }

    pub fn governs(&self, direction: Direction) -> bool {
        match direction {
            Direction::Ingress => self.governs_ingress,
            Direction::Egress => self.governs_egress,
        }
    }

    pub fn rules(&self, direction: Direction) -> &[Rule] {
        match direction {
            Direction::Ingress => &self.ingress,
            Direction::Egress => &self.egress,
        }
    }

    /// This policy's own verdict for one direction.
    pub fn verdict(&self, direction: Direction) -> DirectionVerdict {
        if !self.governs(direction) {
            return DirectionVerdict::NotGoverned;
        }
        let rules = self.rules(direction);
        if rules.is_empty() {
            DirectionVerdict::DeniesAll
        } else if rules.iter().any(Rule::is_open) {
            DirectionVerdict::OpensToAll
        } else {
            DirectionVerdict::Restricts
        }
    }
}
