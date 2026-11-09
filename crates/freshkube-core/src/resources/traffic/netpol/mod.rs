//! NetworkPolicy, read per policy (#949, slice N5). Read-only and no UI types.
//!
//! Only Kubernetes' own NetworkPolicy is read. A CNI's policy objects (Cilium,
//! Calico, …) are not, and a cluster may not enforce NetworkPolicy at all, so
//! nothing here says traffic "is reachable": [`decide`] answers allowed by
//! policy, denied by policy, or can't tell, and anything it can't judge (a
//! refused or cut read, a selector it can't evaluate, a peer or port it lacks
//! the facts for) is can't tell, never a yes.
mod applies;
mod decide;
mod facts;
mod read;
mod select;
mod words;

pub use applies::{Applied, Applying, PodsOf, applying};
pub use decide::{
    Answer, Decision, HOST_NETWORK_NOTE, NamedPort, Remote, Subject, can_reach, decide,
};
pub use facts::{
    Direction, DirectionVerdict, NamespaceScope, Peer, PolicyFacts, PortRef, PortSpec, Rule,
};
pub use read::{
    NAMESPACE_LIMIT, NamespaceLabels, NamespacesRead, POD_LIMIT, POLICY_LIMIT, PoliciesRead,
    PolicyReading, SelectedPods, count_selected, read_applying, read_namespaces, read_policies,
    read_selected_pods,
};
pub use select::{
    CONTROLLER_LABELS, Match, selector_matches, selector_matches_template, selector_text,
};

/// Said wherever a verdict could be read as "traffic passes".
pub const SCOPE_NOTE: &str = "Only Kubernetes NetworkPolicy is read. The cluster's network plugin may enforce more, or may not enforce NetworkPolicy at all.";

#[cfg(test)]
mod tests;
