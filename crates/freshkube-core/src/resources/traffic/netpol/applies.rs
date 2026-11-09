//! Which policies apply to a pod, or to the pods a workload's template makes,
//! and what each direction comes to with them together. Policies only add:
//! a pod is governed in a direction when any policy that selects it governs
//! it, and what is allowed is the union of their rules.
use super::facts::{Direction, DirectionVerdict, PolicyFacts};
use super::select::{Match, selector_matches};
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

/// The policies of `namespace` against a pod with `labels` in it.
pub fn applying(
    policies: &[PolicyFacts],
    namespace: &str,
    labels: &BTreeMap<String, String>,
) -> Applying {
    let selected = policies
        .iter()
        .filter(|policy| policy.namespace == namespace)
        .map(|policy| (policy, selector_matches(&policy.pod_selector, labels)))
        .filter(|(_, m)| *m != Match::No)
        .collect::<Vec<_>>();
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
        ingress: combined(&selected, Direction::Ingress),
        egress: combined(&selected, Direction::Egress),
    }
}
