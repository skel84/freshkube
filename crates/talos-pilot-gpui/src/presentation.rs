use std::collections::{BTreeMap, BTreeSet, VecDeque};
use talos_pilot_core::cluster_overview::{ClusterOverview, EtcdSummary};
use talos_pilot_core::constants::{MEMORY_CRITICAL_PERCENT, MEMORY_WARNING_PERCENT};
use talos_rs::ServiceInfo;

/// Samples kept per node for the load sparkline (5 minutes at 15 s).
pub(crate) const LOAD_HISTORY_LEN: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Health {
    Healthy,
    Unhealthy,
    Unknown,
}

pub(crate) fn service_health(service: &ServiceInfo) -> Health {
    match &service.health {
        Some(health) if !health.unknown && health.healthy => Health::Healthy,
        Some(health) if !health.unknown => Health::Unhealthy,
        _ => Health::Unknown,
    }
}

pub(crate) fn health_text(health: &Health) -> &'static str {
    match health {
        Health::Healthy => "Healthy",
        Health::Unhealthy => "Unhealthy",
        Health::Unknown => "Not reported",
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HealthCounts {
    pub(crate) healthy: usize,
    pub(crate) unhealthy: usize,
    pub(crate) unknown: usize,
}

impl HealthCounts {
    fn add(&mut self, health: Health) {
        match health {
            Health::Healthy => self.healthy += 1,
            Health::Unhealthy => self.unhealthy += 1,
            Health::Unknown => self.unknown += 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Role {
    ControlPlane,
    Worker,
    Unknown,
}

impl Role {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Role::ControlPlane => "Control plane",
            Role::Worker => "Worker",
            Role::Unknown => "Unknown role",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemoryLevel {
    Normal,
    High,
    Critical,
}

pub(crate) fn memory_level(percent: f64) -> MemoryLevel {
    if percent >= MEMORY_CRITICAL_PERCENT {
        MemoryLevel::Critical
    } else if percent >= MEMORY_WARNING_PERCENT {
        MemoryLevel::High
    } else {
        MemoryLevel::Normal
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Memory {
    pub(crate) used: u64,
    pub(crate) total: u64,
}

impl Memory {
    pub(crate) fn percent(&self) -> f64 {
        100.0 * self.used as f64 / self.total as f64
    }

    pub(crate) fn level(&self) -> MemoryLevel {
        memory_level(self.percent())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct NodeSummary {
    pub(crate) name: String,
    pub(crate) address: String,
    pub(crate) role: Role,
    pub(crate) etcd_member: bool,
    pub(crate) responding: bool,
    pub(crate) version: Option<String>,
    pub(crate) cores: Option<usize>,
    pub(crate) memory: Option<Memory>,
    pub(crate) load: Option<[f64; 3]>,
    /// Sorted by service ID so positions stay stable across refreshes.
    pub(crate) services: Vec<ServiceInfo>,
}

impl NodeSummary {
    pub(crate) fn health_counts(&self) -> HealthCounts {
        let mut counts = HealthCounts::default();
        for service in &self.services {
            counts.add(service_health(service));
        }
        counts
    }

    pub(crate) fn unhealthy_services(&self) -> impl Iterator<Item = &ServiceInfo> {
        self.services
            .iter()
            .filter(|service| service_health(service) == Health::Unhealthy)
    }
}

pub(crate) fn node_summaries(cluster: &ClusterOverview) -> Vec<NodeSummary> {
    let mut names: BTreeSet<String> = cluster.node_ips.keys().cloned().collect();
    names.extend(cluster.versions.iter().map(|v| v.node.clone()));
    names.extend(cluster.services.iter().map(|v| v.node.clone()));
    if names.is_empty() {
        names.extend(cluster.endpoints.iter().cloned());
    }
    let mut nodes: Vec<_> = names
        .into_iter()
        .map(|name| {
            let address = cluster
                .node_ips
                .get(&name)
                .cloned()
                .unwrap_or_else(|| name.clone());
            let etcd_member = cluster.etcd_members.iter().any(|member| {
                member.ip_address().as_deref() == Some(address.as_str()) || member.hostname == name
            });
            let discovered = cluster
                .discovery_members
                .iter()
                .find(|member| member.hostname == name || member.addresses.contains(&address));
            let role = match discovered.map(|member| member.machine_type.to_lowercase()) {
                Some(kind) if kind == "controlplane" => Role::ControlPlane,
                Some(kind) if kind == "worker" => Role::Worker,
                Some(_) => Role::Unknown,
                None if etcd_member || cluster.node_is_controlplane(&name) => Role::ControlPlane,
                None => Role::Unknown,
            };
            let version = cluster.versions.iter().find(|v| v.node == name);
            let services = cluster.services.iter().find(|v| v.node == name);
            let mut service_list = services
                .map(|node| node.services.clone())
                .unwrap_or_default();
            service_list.sort_by(|a, b| a.id.cmp(&b.id));
            NodeSummary {
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
                    .map(|m| Memory {
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
        })
        .collect();
    nodes.sort_by(|a, b| a.role.cmp(&b.role).then_with(|| a.name.cmp(&b.name)));
    nodes
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Roster {
    Fixture,
    Discovery,
    FallbackOrEndpoints,
}

impl Roster {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Roster::Fixture => "Example data",
            Roster::Discovery => "Talos discovery",
            Roster::FallbackOrEndpoints => "Kubernetes or endpoints",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ClusterSummary {
    pub(crate) total: usize,
    pub(crate) responding: usize,
    pub(crate) control_planes: usize,
    pub(crate) workers: usize,
    pub(crate) versions: Vec<String>,
    pub(crate) platforms: Vec<String>,
    pub(crate) arches: Vec<String>,
    pub(crate) services: HealthCounts,
    /// The first unhealthy service as (node, service), in node order.
    pub(crate) first_unhealthy: Option<(String, String)>,
    /// The responding node with the highest memory use, with its percentage.
    pub(crate) peak_memory: Option<(String, f64)>,
    pub(crate) etcd: Option<EtcdSummary>,
}

pub(crate) fn cluster_summary(cluster: &ClusterOverview, nodes: &[NodeSummary]) -> ClusterSummary {
    let mut services = HealthCounts::default();
    let mut first_unhealthy = None;
    let mut peak_memory: Option<(String, f64)> = None;
    for node in nodes.iter().filter(|node| node.responding) {
        for service in &node.services {
            let health = service_health(service);
            services.add(health);
            if health == Health::Unhealthy && first_unhealthy.is_none() {
                first_unhealthy = Some((node.name.clone(), service.id.clone()));
            }
        }
        if let Some(memory) = node.memory {
            let percent = memory.percent();
            if peak_memory.as_ref().is_none_or(|(_, peak)| percent > *peak) {
                peak_memory = Some((node.name.clone(), percent));
            }
        }
    }
    let unique = |values: Vec<String>| -> Vec<String> {
        values
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    };
    ClusterSummary {
        total: nodes.len(),
        responding: nodes.iter().filter(|node| node.responding).count(),
        control_planes: nodes
            .iter()
            .filter(|node| node.role == Role::ControlPlane)
            .count(),
        workers: nodes
            .iter()
            .filter(|node| node.role == Role::Worker)
            .count(),
        versions: unique(cluster.versions.iter().map(|v| v.version.clone()).collect()),
        platforms: unique(
            cluster
                .versions
                .iter()
                .map(|v| v.platform.clone())
                .collect(),
        ),
        arches: unique(cluster.versions.iter().map(|v| v.arch.clone()).collect()),
        services,
        first_unhealthy,
        peak_memory,
        etcd: cluster.etcd_summary.clone(),
    }
}

/// Members that can fail while etcd keeps quorum.
pub(crate) fn etcd_failure_tolerance(total: usize) -> usize {
    total.saturating_sub(1) / 2
}

/// Recent load1 samples per node, owned by the UI because the backend only
/// reports the current value.
#[derive(Clone, Debug, Default)]
pub(crate) struct LoadHistory {
    samples: BTreeMap<String, VecDeque<f64>>,
}

impl LoadHistory {
    pub(crate) fn record(&mut self, nodes: &[NodeSummary]) {
        let present: BTreeSet<&str> = nodes.iter().map(|node| node.name.as_str()).collect();
        self.samples
            .retain(|name, _| present.contains(name.as_str()));
        for node in nodes {
            let Some([load1, _, _]) = node.load else {
                continue;
            };
            let samples = self.samples.entry(node.name.clone()).or_default();
            if samples.len() == LOAD_HISTORY_LEN {
                samples.pop_front();
            }
            samples.push_back(load1);
        }
    }

    pub(crate) fn get(&self, node: &str) -> Vec<f64> {
        self.samples
            .get(node)
            .map(|samples| samples.iter().copied().collect())
            .unwrap_or_default()
    }

    pub(crate) fn clear(&mut self) {
        self.samples.clear();
    }
}

pub(crate) fn selected_service<'a>(
    services: &'a [ServiceInfo],
    selected: Option<&str>,
) -> Option<&'a ServiceInfo> {
    selected.and_then(|id| services.iter().find(|service| service.id == id))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_health_never_becomes_healthy_or_failed() {
        let mut service = ServiceInfo {
            id: "kubelet".into(),
            state: "Running".into(),
            health: None,
        };
        assert_eq!(service_health(&service), Health::Unknown);
        service.health = Some(talos_rs::ServiceHealth {
            unknown: true,
            healthy: false,
            last_message: String::new(),
        });
        assert_eq!(service_health(&service), Health::Unknown);
    }
    #[test]
    fn service_selection_uses_exact_domain_identity_across_refresh() {
        let services = vec![
            ServiceInfo {
                id: "APId".into(),
                state: "Running".into(),
                health: None,
            },
            ServiceInfo {
                id: "kubelet".into(),
                state: "Running".into(),
                health: None,
            },
        ];
        assert_eq!(
            selected_service(&services, Some("APId")).unwrap().id,
            "APId"
        );
        assert!(selected_service(&services, Some("apid")).is_none());
        assert!(selected_service(&services[1..], Some("APId")).is_none());
    }
    #[test]
    fn missing_metrics_and_role_are_explicit() {
        let cluster = ClusterOverview {
            endpoints: vec!["192.0.2.1".into()],
            ..Default::default()
        };
        let nodes = node_summaries(&cluster);
        assert_eq!(nodes[0].role, Role::Unknown);
        assert!(!nodes[0].responding);
        assert!(nodes[0].cores.is_none());
        assert!(nodes[0].memory.is_none());
        assert!(nodes[0].load.is_none());
        let summary = cluster_summary(&cluster, &nodes);
        assert_eq!(summary.responding, 0);
        assert!(summary.peak_memory.is_none());
        assert_eq!(summary.services, HealthCounts::default());
    }
    #[test]
    fn memory_levels_follow_core_thresholds() {
        assert_eq!(memory_level(84.9), MemoryLevel::Normal);
        assert_eq!(memory_level(MEMORY_WARNING_PERCENT), MemoryLevel::High);
        assert_eq!(memory_level(MEMORY_CRITICAL_PERCENT), MemoryLevel::Critical);
    }
    #[test]
    fn etcd_tolerance_counts_members_that_can_fail() {
        assert_eq!(etcd_failure_tolerance(0), 0);
        assert_eq!(etcd_failure_tolerance(1), 0);
        assert_eq!(etcd_failure_tolerance(3), 1);
        assert_eq!(etcd_failure_tolerance(5), 2);
    }
    #[test]
    fn load_history_is_bounded_and_forgets_departed_nodes() {
        let node = |name: &str, load: Option<f64>| NodeSummary {
            name: name.into(),
            address: name.into(),
            role: Role::Worker,
            etcd_member: false,
            responding: load.is_some(),
            version: None,
            cores: Some(4),
            memory: None,
            load: load.map(|load| [load, load, load]),
            services: Vec::new(),
        };
        let mut history = LoadHistory::default();
        for ix in 0..LOAD_HISTORY_LEN + 5 {
            history.record(&[node("a", Some(ix as f64)), node("b", None)]);
        }
        let samples = history.get("a");
        assert_eq!(samples.len(), LOAD_HISTORY_LEN);
        assert_eq!(samples.last(), Some(&((LOAD_HISTORY_LEN + 4) as f64)));
        assert!(history.get("b").is_empty());
        history.record(&[node("b", Some(1.0))]);
        assert!(history.get("a").is_empty());
        assert_eq!(history.get("b"), vec![1.0]);
    }
}
