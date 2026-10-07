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

#[cfg(test)]
mod tests {
    use talos_rs::{
        DiscoveryMember, EtcdMemberInfo, MemInfo, NodeMemory, NodeServices, ServiceInfo,
        VersionInfo,
    };

    use super::*;

    fn version(node: &str) -> VersionInfo {
        VersionInfo {
            node: node.into(),
            version: "v1.9.0".into(),
            sha: String::new(),
            built: String::new(),
            go_version: String::new(),
            os: String::new(),
            arch: String::new(),
            platform: String::new(),
        }
    }

    fn services(node: &str, ids: &[&str]) -> NodeServices {
        NodeServices {
            node: node.into(),
            services: ids
                .iter()
                .map(|id| ServiceInfo {
                    id: (*id).into(),
                    state: "Running".into(),
                    health: None,
                })
                .collect(),
        }
    }

    fn discovered(hostname: &str, address: &str, machine_type: &str) -> DiscoveryMember {
        DiscoveryMember {
            id: hostname.into(),
            addresses: vec![address.into()],
            hostname: hostname.into(),
            machine_type: machine_type.into(),
            operating_system: String::new(),
        }
    }

    fn etcd(hostname: &str, peer_url: &str) -> EtcdMemberInfo {
        EtcdMemberInfo {
            id: 1,
            hostname: hostname.into(),
            peer_urls: vec![peer_url.into()],
            client_urls: Vec::new(),
            is_learner: false,
        }
    }

    fn memory(node: &str, total: u64, available: u64) -> NodeMemory {
        NodeMemory {
            node: node.into(),
            meminfo: Some(MemInfo {
                mem_total: total,
                mem_free: 0,
                mem_available: available,
                buffers: 0,
                cached: 0,
            }),
        }
    }

    fn names(nodes: &[TalosNodeObservation]) -> Vec<&str> {
        nodes.iter().map(|node| node.name.as_str()).collect()
    }

    #[test]
    fn endpoints_stand_in_only_when_no_node_is_known() {
        let cluster = ClusterOverview {
            endpoints: vec!["cp-1.example.test".into(), "cp-2.example.test".into()],
            ..Default::default()
        };
        let nodes = observe_talos_nodes(&cluster);
        assert_eq!(names(&nodes), ["cp-1.example.test", "cp-2.example.test"]);
        let node = &nodes[0];
        assert_eq!(node.address, "cp-1.example.test");
        assert_eq!(node.role, NodeRole::Unknown);
        assert!(!node.responding && !node.etcd_member);
        assert!(node.version.is_none() && node.cores.is_none());
        assert!(node.memory.is_none() && node.load.is_none());
        assert!(node.services.is_empty());

        let cluster = ClusterOverview {
            versions: vec![version("worker-1")],
            ..cluster
        };
        assert_eq!(names(&observe_talos_nodes(&cluster)), ["worker-1"]);
    }

    #[test]
    fn nodes_join_addresses_versions_and_services_in_name_order() {
        let cluster = ClusterOverview {
            node_ips: [("cp-2".to_string(), "10.0.0.2".to_string())].into(),
            versions: vec![version("worker-1"), version("cp-2")],
            services: vec![services("cp-1", &["etcd"])],
            ..Default::default()
        };
        let nodes = observe_talos_nodes(&cluster);
        assert_eq!(names(&nodes), ["cp-1", "cp-2", "worker-1"]);
        assert_eq!(nodes[0].address, "cp-1");
        assert_eq!(nodes[1].address, "10.0.0.2");
        assert_eq!(nodes[2].address, "worker-1");
    }

    #[test]
    fn without_discovery_etcd_service_makes_a_control_plane_and_the_rest_unknown() {
        let cluster = ClusterOverview {
            services: vec![
                services("cp-1", &["kubelet", "etcd"]),
                services("worker-1", &["kubelet"]),
            ],
            ..Default::default()
        };
        let nodes = observe_talos_nodes(&cluster);
        assert_eq!(nodes[0].role, NodeRole::ControlPlane);
        assert!(!nodes[0].etcd_member);
        assert_eq!(nodes[1].role, NodeRole::Unknown);
    }

    #[test]
    fn an_etcd_member_alone_is_a_control_plane() {
        let cluster = ClusterOverview {
            node_ips: [
                ("cp-1".to_string(), "10.0.0.1".to_string()),
                ("cp-2".to_string(), "10.0.0.2".to_string()),
                ("worker-1".to_string(), "10.0.0.3".to_string()),
            ]
            .into(),
            etcd_members: vec![
                etcd("member-a", "https://10.0.0.1:2380"),
                etcd("cp-2", "https://10.0.9.9:2380"),
            ],
            ..Default::default()
        };
        let nodes = observe_talos_nodes(&cluster);
        let roles: Vec<_> = nodes
            .iter()
            .map(|node| (node.etcd_member, node.role.clone()))
            .collect();
        assert_eq!(
            roles,
            [
                (true, NodeRole::ControlPlane),
                (true, NodeRole::ControlPlane),
                (false, NodeRole::Unknown),
            ]
        );
        assert!(nodes.iter().all(|node| !node.responding));
    }

    #[test]
    fn discovery_decides_the_role_by_hostname_or_address_ignoring_case() {
        let cluster = ClusterOverview {
            node_ips: [
                ("cp-1".to_string(), "10.0.0.1".to_string()),
                ("worker-1".to_string(), "10.0.0.3".to_string()),
                ("odd-1".to_string(), "10.0.0.4".to_string()),
                ("unlisted".to_string(), "10.0.0.5".to_string()),
            ]
            .into(),
            discovery_members: vec![
                discovered("cp-1", "10.0.1.1", "ControlPlane"),
                discovered("renamed", "10.0.0.3", "WORKER"),
                discovered("odd-1", "10.0.0.4", "init"),
            ],
            // Discovery outranks etcd membership for the role.
            etcd_members: vec![etcd("worker-1", "https://10.0.0.3:2380")],
            ..Default::default()
        };
        let nodes = observe_talos_nodes(&cluster);
        let roles: Vec<_> = nodes
            .iter()
            .map(|node| (node.name.as_str(), node.role.clone()))
            .collect();
        assert_eq!(
            roles,
            [
                ("cp-1", NodeRole::ControlPlane),
                ("odd-1", NodeRole::Unknown),
                ("unlisted", NodeRole::Unknown),
                ("worker-1", NodeRole::Worker),
            ]
        );
        assert!(nodes[3].etcd_member);
    }

    #[test]
    fn memory_needs_a_total_and_never_underflows() {
        let cluster = ClusterOverview {
            versions: vec![version("a"), version("b"), version("c"), version("d")],
            memory: vec![
                memory("a", 0, 0),
                memory("b", 8_000, 2_000),
                memory("c", 4_000, 6_000),
                NodeMemory {
                    node: "d".into(),
                    meminfo: None,
                },
            ],
            ..Default::default()
        };
        let memory: Vec<_> = observe_talos_nodes(&cluster)
            .into_iter()
            .map(|node| node.memory)
            .collect();
        assert_eq!(
            memory,
            [
                None,
                Some(TalosNodeMemory {
                    used: 6_000,
                    total: 8_000
                }),
                Some(TalosNodeMemory {
                    used: 0,
                    total: 4_000
                }),
                None,
            ]
        );
    }

    #[test]
    fn services_without_a_version_respond_with_services_sorted() {
        let cluster = ClusterOverview {
            services: vec![services("cp-1", &["kubelet", "apid", "etcd"])],
            versions: vec![version("worker-1")],
            ..Default::default()
        };
        let nodes = observe_talos_nodes(&cluster);
        let node = &nodes[0];
        assert!(node.responding);
        assert!(node.version.is_none());
        let ids: Vec<_> = node.services.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["apid", "etcd", "kubelet"]);

        let worker = &nodes[1];
        assert!(worker.responding);
        assert_eq!(worker.version.as_deref(), Some("v1.9.0"));
        assert!(worker.services.is_empty());
    }
}
