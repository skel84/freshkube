//! The allow decision for core NetworkPolicy only. Three answers, and never
//! "reachable": allowed by policy, denied by policy, or can't tell.
//!
//! A rule is judged against the remote side's namespace, labels and IP, and a
//! port against the destination pod's container ports. What the caller could
//! not read is `None`, and `None` is can't tell, never a no.
use super::facts::{Direction, NamespaceScope, Peer, PolicyFacts, PortRef, PortSpec, Rule};
use super::read::NamespaceLabels;
use super::select::{Match, selector_matches};
use std::collections::BTreeMap;
use std::net::IpAddr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Allowed,
    Denied,
    CantTell,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub answer: Answer,
    /// Why, in words: the policy that allowed it, or what could not be judged.
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedPort {
    pub name: String,
    pub number: i32,
    pub protocol: String,
}

/// The pod whose policies are consulted.
#[derive(Clone, Debug)]
pub struct Subject {
    pub namespace: String,
    pub labels: BTreeMap<String, String>,
    /// Container ports with names; `None` when not read.
    pub named_ports: Option<Vec<NamedPort>>,
}

/// The other side.
#[derive(Clone, Debug)]
pub enum Remote {
    Pod {
        namespace: String,
        labels: BTreeMap<String, String>,
        ip: Option<IpAddr>,
        named_ports: Option<Vec<NamedPort>>,
    },
    External {
        ip: Option<IpAddr>,
    },
}

fn cidr_contains(cidr: &str, ip: IpAddr) -> Option<bool> {
    let (net, bits) = cidr.split_once('/')?;
    let net: IpAddr = net.parse().ok()?;
    let bits: u32 = bits.parse().ok()?;
    match (net, ip) {
        (IpAddr::V4(net), IpAddr::V4(ip)) if bits <= 32 => {
            let mask = if bits == 0 {
                0
            } else {
                u32::MAX << (32 - bits)
            };
            Some(u32::from(net) & mask == u32::from(ip) & mask)
        }
        (IpAddr::V6(net), IpAddr::V6(ip)) if bits <= 128 => {
            let mask = if bits == 0 {
                0
            } else {
                u128::MAX << (128 - bits)
            };
            Some(u128::from(net) & mask == u128::from(ip) & mask)
        }
        // A family mismatch is a plain no; a bad prefix is not judged.
        (IpAddr::V4(_), IpAddr::V6(_)) | (IpAddr::V6(_), IpAddr::V4(_)) => {
            let max = if net.is_ipv4() { 32 } else { 128 };
            (bits <= max).then_some(false)
        }
        _ => None,
    }
}

fn ip_block(cidr: &str, except: &[String], ip: Option<IpAddr>) -> Match {
    let Some(ip) = ip else {
        return Match::Unknown;
    };
    match cidr_contains(cidr, ip) {
        None => Match::Unknown,
        Some(false) => Match::No,
        Some(true) => {
            let mut result = Match::Yes;
            for except in except {
                result = result.and(match cidr_contains(except, ip) {
                    None => Match::Unknown,
                    Some(inside) => (!inside).into(),
                });
            }
            result
        }
    }
}

fn peer_matches(
    peer: &Peer,
    policy: &PolicyFacts,
    remote: &Remote,
    namespaces: Option<&[NamespaceLabels]>,
) -> Match {
    match (peer, remote) {
        (Peer::Invalid, _) => Match::Unknown,
        (Peer::IpBlock { cidr, except }, Remote::Pod { ip, .. } | Remote::External { ip }) => {
            ip_block(cidr, except, *ip)
        }
        // Pod and namespace selectors never match a cluster-external address.
        (Peer::Pods { .. }, Remote::External { .. }) => Match::No,
        (
            Peer::Pods {
                namespaces: scope,
                pods,
            },
            Remote::Pod {
                namespace, labels, ..
            },
        ) => {
            let ns = match scope {
                NamespaceScope::Own => (*namespace == policy.namespace).into(),
                NamespaceScope::Selector(selector) => match namespaces {
                    None => Match::Unknown,
                    Some(list) => match list.iter().find(|n| n.name == *namespace) {
                        None => Match::Unknown,
                        Some(found) => selector_matches(selector, &found.labels),
                    },
                },
            };
            let pod = pods
                .as_ref()
                .map_or(Match::Yes, |selector| selector_matches(selector, labels));
            ns.and(pod)
        }
    }
}

fn port_matches(
    spec: &PortSpec,
    number: i32,
    protocol: &str,
    destination: Option<&[NamedPort]>,
    destination_is_pod: bool,
) -> Match {
    if !spec.protocol.eq_ignore_ascii_case(protocol) {
        return Match::No;
    }
    match &spec.port {
        None => Match::Yes,
        Some(PortRef::Number(first)) => {
            let last = spec.end_port.unwrap_or(*first);
            (*first..=last).contains(&number).into()
        }
        Some(PortRef::Name(name)) => {
            if !destination_is_pod {
                return Match::Unknown;
            }
            match destination {
                None => Match::Unknown,
                Some(ports) => ports
                    .iter()
                    .any(|p| {
                        p.name == *name
                            && p.number == number
                            && p.protocol.eq_ignore_ascii_case(protocol)
                    })
                    .into(),
            }
        }
    }
}

struct Query<'a> {
    policy: &'a PolicyFacts,
    remote: &'a Remote,
    namespaces: Option<&'a [NamespaceLabels]>,
    number: i32,
    protocol: &'a str,
    destination: Option<&'a [NamedPort]>,
    destination_is_pod: bool,
}

fn rule_matches(rule: &Rule, q: &Query) -> Match {
    let peers = match &rule.peers {
        None => Match::Yes,
        Some(peers) => peers.iter().fold(Match::No, |acc, peer| {
            acc.or(peer_matches(peer, q.policy, q.remote, q.namespaces))
        }),
    };
    let ports = match &rule.ports {
        None => Match::Yes,
        Some(ports) => ports.iter().fold(Match::No, |acc, spec| {
            acc.or(port_matches(
                spec,
                q.number,
                q.protocol,
                q.destination,
                q.destination_is_pod,
            ))
        }),
    };
    peers.and(ports)
}

/// One direction on one pod: does `subject`'s policies let `remote` through on
/// `number`/`protocol`? `namespaces` is `None` when the namespace list was
/// refused or cut. A named port resolves on the destination pod: the subject
/// for ingress, the remote pod for egress.
pub fn decide(
    direction: Direction,
    policies: &[PolicyFacts],
    subject: &Subject,
    remote: &Remote,
    number: i32,
    protocol: &str,
    namespaces: Option<&[NamespaceLabels]>,
) -> Decision {
    let (destination, destination_is_pod) = match (direction, remote) {
        (Direction::Ingress, _) => (subject.named_ports.as_deref(), true),
        (Direction::Egress, Remote::Pod { named_ports, .. }) => (named_ports.as_deref(), true),
        (Direction::Egress, Remote::External { .. }) => (None, false),
    };
    let mut governed = false;
    let mut unknown = Vec::new();
    for policy in policies.iter().filter(|p| p.namespace == subject.namespace) {
        if !policy.governs(direction) {
            continue;
        }
        match selector_matches(&policy.pod_selector, &subject.labels) {
            Match::No => continue,
            Match::Unknown => {
                unknown.push(format!("{}'s pod selector can't be evaluated", policy.name));
                continue;
            }
            Match::Yes => governed = true,
        }
        let q = Query {
            policy,
            remote,
            namespaces,
            number,
            protocol,
            destination,
            destination_is_pod,
        };
        for rule in policy.rules(direction) {
            match rule_matches(rule, &q) {
                Match::Yes => {
                    return Decision {
                        answer: Answer::Allowed,
                        reason: format!("policy {} has a rule that allows it", policy.name),
                    };
                }
                Match::Unknown => unknown.push(format!(
                    "{}: a rule can't be judged with what was read",
                    policy.name
                )),
                Match::No => {}
            }
        }
    }
    if !unknown.is_empty() {
        return Decision {
            answer: Answer::CantTell,
            reason: unknown.join("; "),
        };
    }
    if governed {
        Decision {
            answer: Answer::Denied,
            reason: "a policy governs this pod and no rule allows it".into(),
        }
    } else {
        Decision {
            answer: Answer::Allowed,
            reason: "no policy governs this direction of the pod".into(),
        }
    }
}

/// Both ends: `from`'s egress and `to`'s ingress. Allowed only when both say
/// so; denied when either does; otherwise can't tell. `policies_from` and
/// `policies_to` are each namespace's policies, and either is `None` when its
/// read was refused or cut.
#[allow(clippy::too_many_arguments)]
pub fn can_reach(
    policies_from: Option<&[PolicyFacts]>,
    policies_to: Option<&[PolicyFacts]>,
    from: &Subject,
    from_ip: Option<IpAddr>,
    to: &Subject,
    to_ip: Option<IpAddr>,
    number: i32,
    protocol: &str,
    namespaces: Option<&[NamespaceLabels]>,
) -> Decision {
    let egress = match policies_from {
        None => unread("the source namespace's policies"),
        Some(policies) => decide(
            Direction::Egress,
            policies,
            from,
            &Remote::Pod {
                namespace: to.namespace.clone(),
                labels: to.labels.clone(),
                ip: to_ip,
                named_ports: to.named_ports.clone(),
            },
            number,
            protocol,
            namespaces,
        ),
    };
    let ingress = match policies_to {
        None => unread("the destination namespace's policies"),
        Some(policies) => decide(
            Direction::Ingress,
            policies,
            to,
            &Remote::Pod {
                namespace: from.namespace.clone(),
                labels: from.labels.clone(),
                ip: from_ip,
                named_ports: from.named_ports.clone(),
            },
            number,
            protocol,
            namespaces,
        ),
    };
    let reason = |side: &str, d: &Decision| format!("{side}: {}", d.reason);
    match (egress.answer, ingress.answer) {
        (Answer::Denied, _) => Decision {
            answer: Answer::Denied,
            reason: reason("egress", &egress),
        },
        (_, Answer::Denied) => Decision {
            answer: Answer::Denied,
            reason: reason("ingress", &ingress),
        },
        (Answer::Allowed, Answer::Allowed) => Decision {
            answer: Answer::Allowed,
            reason: format!(
                "{}; {}",
                reason("egress", &egress),
                reason("ingress", &ingress)
            ),
        },
        (Answer::CantTell, _) => Decision {
            answer: Answer::CantTell,
            reason: reason("egress", &egress),
        },
        (_, Answer::CantTell) => Decision {
            answer: Answer::CantTell,
            reason: reason("ingress", &ingress),
        },
    }
}

fn unread(what: &str) -> Decision {
    Decision {
        answer: Answer::CantTell,
        reason: format!("{what} could not be read in full"),
    }
}
