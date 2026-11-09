//! Which policies apply to a pod, or to the pods a workload's template makes,
//! and what each direction comes to with them together. Policies only add:
//! a pod is governed in a direction when any policy that selects it governs
//! it, and what is allowed is the union of their rules.
use super::facts::{Direction, DirectionVerdict, PolicyFacts};
use super::select::{Match, selector_matches, selector_matches_template};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub name: String,
    pub ingress: DirectionVerdict,
    pub egress: DirectionVerdict,
    /// The selector could not be judged, so it may or may not apply.
    pub uncertain: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applying {
    /// The policies that select the pod, or may (`uncertain`), by name.
    pub policies: Vec<Applied>,
    pub ingress: DirectionVerdict,
    pub egress: DirectionVerdict,
    /// The pods share the node's network, so both verdicts are Unknown: the
    /// network plugin decides whether NetworkPolicy applies to them.
    pub host_network: bool,
}

impl Applying {
    pub fn verdict(&self, direction: Direction) -> DirectionVerdict {
        match direction {
            Direction::Ingress => self.ingress,
            Direction::Egress => self.egress,
        }
    }
}

/// A policy that governs `direction` and selects (`Yes`) or may select
/// (`Unknown`) the pod.
fn combined(applying: &[(&PolicyFacts, Match)], direction: Direction) -> DirectionVerdict {
    let governing = applying
        .iter()
        .filter(|(policy, _)| policy.governs(direction))
        .collect::<Vec<_>>();
    let certain = governing.iter().filter(|(_, m)| *m == Match::Yes);
    let mut rules = 0;
    for (policy, _) in certain {
        let verdict = policy.verdict(direction);
        if verdict == DirectionVerdict::OpensToAll {
            return DirectionVerdict::OpensToAll;
        }
        rules += policy.rules(direction).len();
    }
    // Something may govern this pod and can't be judged: neither "not
    // governed" nor "denies" is known.
    if governing.iter().any(|(_, m)| *m == Match::Unknown) {
        return DirectionVerdict::Unknown;
    }
    if governing.is_empty() {
        DirectionVerdict::NotGoverned
    } else if rules == 0 {
        DirectionVerdict::DeniesAll
    } else {
        DirectionVerdict::Restricts
    }
}

/// The pods a view asks about: a Pod, or the pods a workload's template makes.
#[derive(Clone, Copy, Debug)]
pub struct PodsOf<'a> {
    pub namespace: &'a str,
    pub labels: &'a BTreeMap<String, String>,
    /// `labels` are a workload template's, which lack the labels its
    /// controller adds to each pod.
    pub template: bool,
    pub host_network: bool,
}

/// The policies of the namespace against the pods. Host-network pods get an
/// Unknown verdict both ways, whatever the policies say.
pub fn applying(policies: &[PolicyFacts], pods: &PodsOf) -> Applying {
    let selected = policies
        .iter()
        .filter(|policy| policy.namespace == pods.namespace)
        .map(|policy| {
            let m = if pods.template {
                selector_matches_template(&policy.pod_selector, pods.labels)
            } else {
                selector_matches(&policy.pod_selector, pods.labels)
            };
            (policy, m)
        })
        .filter(|(_, m)| *m != Match::No)
        .collect::<Vec<_>>();
    let (ingress, egress) = if pods.host_network {
        (DirectionVerdict::Unknown, DirectionVerdict::Unknown)
    } else {
        (
            combined(&selected, Direction::Ingress),
            combined(&selected, Direction::Egress),
        )
    };
    Applying {
        policies: selected
            .iter()
            .map(|(policy, m)| Applied {
                name: policy.name.clone(),
                ingress: policy.verdict(Direction::Ingress),
                egress: policy.verdict(Direction::Egress),
                uncertain: *m == Match::Unknown,
            })
            .collect(),
        ingress,
        egress,
        host_network: pods.host_network,
    }
}
