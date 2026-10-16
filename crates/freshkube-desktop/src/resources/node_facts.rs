//! What Resources shows of the cluster's nodes, handed over by the shell.

use freshkube_ui::ui::Tone;
use gpui_kit::SharedString;

/// What Resources shows of one Kubernetes node: the pods list's not-ready
/// group and a pod's "Runs on" card. The Nodes workspace builds it from its
/// joined node row (`NodeRow::resource_facts`); Resources only reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeFacts {
    /// The Kubernetes node's name, as pods name it.
    pub(crate) name: String,
    /// Kubernetes reports the node and its Ready condition isn't True.
    pub(crate) not_ready: bool,
    /// When that condition last changed, as a Unix time, when known.
    pub(crate) not_ready_since: Option<i64>,
    /// "Ready", "NotReady", "Last known", … as the Nodes table words it.
    pub(crate) ready_label: SharedString,
    pub(crate) tone: Tone,
    /// Used and total bytes, when Talos reported them.
    pub(crate) memory: Option<NodeMemory>,
    pub(crate) memory_label: SharedString,
    /// Talos doesn't answer for the node right now, so memory is last known.
    pub(crate) memory_stale: bool,
    pub(crate) problems: Vec<SharedString>,
    /// The node has system services to link to.
    pub(crate) services: bool,
    /// Talos's kubelet service, worded, when Talos reports the node.
    pub(crate) kubelet: Option<(Tone, SharedString)>,
}

/// Used and total bytes of a node's memory.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NodeMemory {
    pub(crate) used: u64,
    pub(crate) total: u64,
}
