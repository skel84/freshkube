//! Framework-native feature screens for the desktop application.
//!
//! Screens own only egui state. They receive immutable cluster/node identity
//! snapshots and execute I/O on the Tokio runtime supplied by the application.

pub(crate) mod bootstrap;
pub(crate) mod diagnostics;

pub(crate) mod etcd;
pub(crate) mod logs;
pub(crate) mod network;
pub(crate) mod operations;
pub(crate) mod processes;
pub(crate) mod security_lifecycle;
pub(crate) mod workloads;

use std::path::PathBuf;

use talos_pilot_core::cluster_overview::ClusterOverview;
use talos_rs::TalosClient;

/// The currently selected Talos context and, when applicable, node.
///
/// Identity is intentionally name-based. Screens must discard results whose
/// request target no longer matches this value rather than applying data to a
/// newly selected row.
pub(crate) struct ScreenTarget<'a> {
    pub cluster: &'a ClusterOverview,
    pub selected_node: Option<&'a str>,
    pub config_path: Option<&'a PathBuf>,
}

impl ScreenTarget<'_> {
    /// The selected Talos context name.
    pub(crate) fn context_name(&self) -> &str {
        &self.cluster.name
    }

    /// The selected node name, if this route requires a node selection.
    pub(crate) fn node_name(&self) -> Option<&str> {
        self.selected_node
    }

    /// The selected node's management address retained by the authoritative
    /// talosconfig-backed cluster snapshot.
    pub(crate) fn node_address(&self) -> Option<String> {
        let node = self.selected_node?;
        self.cluster
            .node_ips
            .get(node)
            .cloned()
            .or_else(|| Some(node.to_owned()))
    }

    /// Derive a Talos client scoped to the selected node.
    pub(crate) fn node_client(&self) -> Option<TalosClient> {
        let address = self.node_address()?;
        self.cluster
            .client
            .as_ref()
            .map(|client| client.with_node(&address))
    }

    /// The context-level Talos client.
    pub(crate) fn cluster_client(&self) -> Option<TalosClient> {
        self.cluster.client.clone()
    }

    /// Whether the selected node is a control-plane member.
    pub(crate) fn selected_node_is_controlplane(&self) -> Option<bool> {
        self.selected_node
            .map(|node| self.cluster.node_is_controlplane(node))
    }

    /// A control-plane address suitable for cluster-scoped Kubernetes work.
    pub(crate) fn control_plane_address(&self) -> Option<String> {
        self.cluster.control_plane_ip()
    }
}

/// Native desktop routes. Every variant maps to an implemented screen.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Route {
    #[default]
    Overview,
    Logs,
    Processes,
    Network,
    Etcd,
    Workloads,
    Diagnostics,
    Security,
    Lifecycle,
    Operations,
    RollingOperations,
    Audit,
    Bootstrap,
}

impl Route {
    pub(crate) const ALL: [Self; 13] = [
        Self::Overview,
        Self::Logs,
        Self::Processes,
        Self::Network,
        Self::Etcd,
        Self::Workloads,
        Self::Diagnostics,
        Self::Security,
        Self::Lifecycle,
        Self::Operations,
        Self::RollingOperations,
        Self::Audit,
        Self::Bootstrap,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Logs => "Logs",
            Self::Processes => "Processes",
            Self::Network => "Network",
            Self::Etcd => "etcd",
            Self::Workloads => "Workloads",
            Self::Diagnostics => "Diagnostics",
            Self::Security => "Security",
            Self::Lifecycle => "Lifecycle",
            Self::Operations => "Operations",
            Self::RollingOperations => "Rolling Operations",
            Self::Audit => "Audit",
            Self::Bootstrap => "Bootstrap",
        }
    }
}
