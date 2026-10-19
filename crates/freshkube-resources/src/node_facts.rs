//! What Resources shows of the cluster's nodes, handed over by the shell.

use freshkube_ui::ui::Tone;
use gpui_kit::SharedString;

/// What Resources shows of one Kubernetes node: the pods list's not-ready
/// group and a pod's "Runs on" card. The Nodes workspace builds it from its
/// joined node row (`NodeRow::resource_facts`); Resources only reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeFacts {
    /// The Kubernetes node's name, as pods name it.
    pub name: String,
    /// Kubernetes reports the node and its Ready condition isn't True.
    pub not_ready: bool,
    /// When that condition last changed, as a Unix time, when known.
    pub not_ready_since: Option<i64>,
    /// "Ready", "NotReady", "Last known", … as the Nodes table words it.
    pub ready_label: SharedString,
    pub tone: Tone,
    /// Used and total bytes, when Talos reported them.
    pub memory: Option<NodeMemory>,
    pub memory_label: SharedString,
    /// Talos doesn't answer for the node right now, so memory is last known.
    pub memory_stale: bool,
    pub problems: Vec<SharedString>,
    /// The node has system services to link to.
    pub services: bool,
    /// Talos's kubelet service, worded, when Talos reports the node.
    pub kubelet: Option<(Tone, SharedString)>,
}

/// Used and total bytes of a node's memory.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeMemory {
    pub used: u64,
    pub total: u64,
}
