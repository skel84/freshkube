//! What Talos reports about each node in a cluster overview: which nodes
//! there are, how to reach them, their role and what they answered. Sorting
//! and display stay with the views.

use std::collections::BTreeSet;

use talos_rs::ServiceInfo;

use crate::{ClusterOverview, NodeRole};

/// Memory in use against the total, in the units Talos reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TalosNodeMemory {
    pub used: u64,
    pub total: u64,
}

/// One node as the overview saw it.
#[derive(Clone, Debug)]
pub struct TalosNodeObservation {
    pub name: String,
    pub address: String,
    pub role: NodeRole,
    pub etcd_member: bool,
    /// The node answered with its version or its services.
    pub responding: bool,
    pub version: Option<String>,
    pub cores: Option<usize>,
    pub memory: Option<TalosNodeMemory>,
    pub load: Option<[f64; 3]>,
    /// Sorted by service ID so positions stay stable across refreshes.
    pub services: Vec<ServiceInfo>,
}

/// Every node the overview knows of, in name order.
///
/// The nodes are those with an address, a version or services; only when
/// there are none do the configured endpoints stand in for them.
pub fn observe_talos_nodes(cluster: &ClusterOverview) -> Vec<TalosNodeObservation> {
    let mut names: BTreeSet<String> = cluster.node_ips.keys().cloned().collect();
    names.extend(cluster.versions.iter().map(|v| v.node.clone()));
    names.extend(cluster.services.iter().map(|v| v.node.clone()));
    if names.is_empty() {
        names.extend(cluster.endpoints.iter().cloned());
    }
    names
        .into_iter()
        .map(|name| observe(cluster, name))
        .collect()
}

fn observe(cluster: &ClusterOverview, name: String) -> TalosNodeObservation {
    let address = cluster
        .node_ips
        .get(&name)
        .cloned()
        .unwrap_or_else(|| name.clone());
    let etcd_member = cluster.etcd_members.iter().any(|member| {
        member.ip_address().as_deref() == Some(address.as_str()) || member.hostname == name
    });
    let role = role(cluster, &name, &address, etcd_member);
    let version = cluster.versions.iter().find(|v| v.node == name);
    let services = cluster.services.iter().find(|v| v.node == name);
    let mut service_list = services
        .map(|node| node.services.clone())
        .unwrap_or_default();
    service_list.sort_by(|a, b| a.id.cmp(&b.id));
    TalosNodeObservation {
        responding: version.is_some() || services.is_some(),
        version: version.map(|v| v.version.clone()),
        cores: cluster
            .cpu_info
            .iter()
            .find(|v| v.node == name)
            .map(|v| v.cpu_count),
        memory: cluster
            .memory
            .iter()
            .find(|v| v.node == name)
            .and_then(|v| v.meminfo.as_ref())
            .filter(|m| m.mem_total > 0)
            .map(|m| TalosNodeMemory {
                used: m.mem_total.saturating_sub(m.mem_available),
                total: m.mem_total,
            }),
        load: cluster
            .load_avg
            .iter()
            .find(|v| v.node == name)
            .map(|v| [v.load1, v.load5, v.load15]),
        services: service_list,
        name,
        address,
        role,
        etcd_member,
    }
}

/// Discovery decides the role when it lists the node; without it, an etcd
/// member or a node the overview takes for a control plane is one.
fn role(cluster: &ClusterOverview, name: &str, address: &str, etcd_member: bool) -> NodeRole {
    let discovered = cluster
        .discovery_members
        .iter()
        .find(|member| member.hostname == name || member.addresses.iter().any(|a| a == address));
    match discovered.map(|member| member.machine_type.to_lowercase()) {
        Some(kind) if kind == "controlplane" => NodeRole::ControlPlane,
        Some(kind) if kind == "worker" => NodeRole::Worker,
        Some(_) => NodeRole::Unknown,
        None if etcd_member || cluster.node_is_controlplane(name) => NodeRole::ControlPlane,
        None => NodeRole::Unknown,
    }
}
