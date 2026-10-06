//! Offline example data for `GpuiOptions::fixture()`. No credentials are read
//! and no cluster is contacted; every value here is synthetic.
use freshkube_core::cluster_overview::{ClusterConnectionStatus, ClusterOverview, EtcdSummary};
use freshkube_core::logs::LogEvent;
use talos_rs::{
    DiscoveryMember, EtcdMemberInfo, MemInfo, NodeCpuInfo, NodeLoadAvg, NodeMemory, NodeServices,
    ServiceHealth, ServiceInfo, VersionInfo,
};

pub(crate) const CONTEXTS: [&str; 4] = [
    "prod-fra",
    "staging-eu",
    "homelab",
    "talos-production-frankfurt-equinix-fr5-baremetal-b7",
];

const GIB: f64 = 1024. * 1024. * 1024.;
const CONTROL_PLANE_SERVICES: [&str; 11] = [
    "apid",
    "auditd",
    "containerd",
    "cri",
    "dashboard",
    "etcd",
    "kubelet",
    "machined",
    "syslogd",
    "trustd",
    "udevd",
];
const WORKER_SERVICES: [&str; 9] = [
    "apid",
    "auditd",
    "containerd",
    "cri",
    "dashboard",
    "kubelet",
    "machined",
    "syslogd",
    "udevd",
];

struct NodeSpec {
    name: &'static str,
    address: &'static str,
    controlplane: bool,
    cores: usize,
    memory_gib: f64,
    used_gib: f64,
    load1: f64,
    version: &'static str,
    arch: &'static str,
    responding: bool,
    unhealthy: &'static [&'static str],
}

const fn spec(
    name: &'static str,
    address: &'static str,
    controlplane: bool,
    cores: usize,
    memory_gib: f64,
    used_gib: f64,
    load1: f64,
) -> NodeSpec {
    NodeSpec {
        name,
        address,
        controlplane,
        cores,
        memory_gib,
        used_gib,
        load1,
        version: "v1.13.2",
        arch: "amd64",
        responding: true,
        unhealthy: &[],
    }
}

fn specs(context: &str) -> Vec<NodeSpec> {
    match context {
        "staging-eu" => vec![
            spec("stg-cp-01", "198.51.100.10", true, 4, 8., 4.6, 0.9),
            spec("stg-wk-01", "198.51.100.20", false, 4, 16., 6.2, 1.4),
            NodeSpec {
                version: "v1.13.1",
                ..spec("stg-wk-02", "198.51.100.21", false, 4, 16., 5.1, 0.7)
            },
        ],
        "homelab" | "talos-production-frankfurt-equinix-fr5-baremetal-b7" => vec![NodeSpec {
            arch: "arm64",
            ..spec("talos-home", "203.0.113.5", true, 4, 8., 3.2, 0.35)
        }],
        _ => vec![
            spec("talos-cp-fra1-01", "192.0.2.10", true, 8, 16., 9.1, 0.82),
            spec("talos-cp-fra1-02", "192.0.2.11", true, 8, 16., 8.4, 0.64),
            spec(
                "talos-cp-fra1-03-baremetal-rack-b7",
                "192.0.2.12",
                true,
                8,
                16.,
                7.9,
                0.71,
            ),
            spec("talos-wk-fra1-01", "192.0.2.20", false, 16, 64., 58.3, 11.2),
            NodeSpec {
                unhealthy: &["kubelet"],
                ..spec("talos-wk-fra1-02", "192.0.2.21", false, 16, 64., 31.0, 3.1)
            },
            NodeSpec {
                responding: false,
                ..spec("talos-wk-fra1-03", "192.0.2.22", false, 16, 64., 0., 0.)
            },
        ],
    }
}

fn service(id: &str, unhealthy: bool) -> ServiceInfo {
    let (healthy, unknown, message) = if unhealthy {
        (
            false,
            false,
            "Health check failed: Get \"http://127.0.0.1:10248/healthz\": dial tcp 127.0.0.1:10248: connect: connection refused".to_owned(),
        )
    } else {
        match id {
            "auditd" => (
                false,
                true,
                "Started task auditd (PID 1187) for container auditd".into(),
            ),
            "dashboard" => (
                false,
                true,
                "Process Process([\"/sbin/dashboard\"]) started with PID 2114".into(),
            ),
            "syslogd" => (
                false,
                true,
                "Started task syslogd (PID 1191) for container syslogd".into(),
            ),
            _ => (true, false, "Health check successful".into()),
        }
    };
    ServiceInfo {
        id: id.into(),
        state: "Running".into(),
        health: Some(ServiceHealth {
            unknown,
            healthy,
            last_message: message,
        }),
    }
}

fn node_services(node: &NodeSpec) -> Vec<ServiceInfo> {
    let ids: &[&str] = if node.controlplane {
        &CONTROL_PLANE_SERVICES
    } else {
        &WORKER_SERVICES
    };
    ids.iter()
        .map(|id| service(id, node.unhealthy.contains(id)))
        .collect()
}

/// The example cluster's nodes as Kubernetes lists them.
pub(crate) struct KubernetesNode {
    pub(crate) name: &'static str,
    pub(crate) address: &'static str,
    pub(crate) control_plane: bool,
    /// A node whose kubelet is down or unhealthy isn't ready.
    pub(crate) ready: bool,
}

pub(crate) fn kubernetes_nodes(context: &str) -> Vec<KubernetesNode> {
    specs(context)
        .into_iter()
        .map(|spec| KubernetesNode {
            name: spec.name,
            address: spec.address,
            control_plane: spec.controlplane,
            ready: spec.responding && !spec.unhealthy.contains(&"kubelet"),
        })
        .collect()
}

/// A deterministic wobble so repeated refreshes visibly move the load
/// sparklines without any randomness in tests.
fn wobble(tick: u64, phase: f64) -> f64 {
    1. + 0.22 * ((tick as f64) * 0.9 + phase).sin()
}

pub(crate) fn cluster(context: &str, tick: u64) -> ClusterOverview {
    let mut cluster = ClusterOverview {
        name: context.into(),
        connection: ClusterConnectionStatus::Connected,
        kubeconfig_source: Some("Example data; no Kubernetes source used".into()),
        ..Default::default()
    };
    if context == "staging-eu" {
        cluster.discovery_warning = Some(
            "Talos discovery is unreachable, so the node list comes from the Kubernetes API. Workers that never joined Kubernetes will not appear.".into(),
        );
    }
    let nodes = specs(context);
    let control_planes = nodes.iter().filter(|node| node.controlplane).count();
    cluster.etcd_summary = Some(EtcdSummary {
        healthy: control_planes,
        total: control_planes,
        has_quorum: true,
    });
    for (ix, node) in nodes.iter().enumerate() {
        let name = node.name.to_owned();
        cluster.node_ips.insert(name.clone(), node.address.into());
        cluster.discovery_members.push(DiscoveryMember {
            id: name.clone(),
            hostname: name.clone(),
            addresses: vec![node.address.into()],
            machine_type: if node.controlplane {
                "controlplane"
            } else {
                "worker"
            }
            .into(),
            operating_system: "Talos (example)".into(),
        });
        if node.controlplane {
            cluster.etcd_members.push(EtcdMemberInfo {
                id: ix as u64 + 1,
                hostname: name.clone(),
                peer_urls: vec![format!("https://{}:2380", node.address)],
                client_urls: vec![format!("https://{}:2379", node.address)],
                is_learner: false,
            });
        }
        if !node.responding {
            continue;
        }
        cluster.versions.push(VersionInfo {
            node: name.clone(),
            version: node.version.into(),
            sha: "example".into(),
            built: String::new(),
            go_version: String::new(),
            os: "linux".into(),
            arch: node.arch.into(),
            platform: "metal".into(),
        });
        cluster.services.push(NodeServices {
            node: name.clone(),
            services: node_services(node),
        });
        cluster.cpu_info.push(NodeCpuInfo {
            node: name.clone(),
            cpu_count: node.cores,
            model_name: "Example CPU".into(),
            mhz: 2400.,
        });
        let load1 = node.load1 * wobble(tick, ix as f64);
        cluster.load_avg.push(NodeLoadAvg {
            node: name.clone(),
            load1,
            load5: node.load1 * 0.82,
            load15: node.load1 * 0.64,
        });
        let total = node.memory_gib * GIB;
        let used = (node.used_gib * (1. + 0.01 * wobble(tick, ix as f64 * 2.)))
            .min(node.memory_gib * 0.97)
            * GIB;
        cluster.memory.push(NodeMemory {
            node: name,
            meminfo: Some(MemInfo {
                mem_total: total as u64,
                mem_free: 0,
                mem_available: (total - used) as u64,
                buffers: 0,
                cached: 0,
            }),
        });
    }
    cluster
}

/// The service catalog a node would report, or nothing if it doesn't answer.
pub(crate) fn services(context: &str, node: &str) -> Vec<ServiceInfo> {
    specs(context)
        .iter()
        .find(|spec| spec.name == node && spec.responding)
        .map(node_services)
        .unwrap_or_default()
}

const TEMPLATES: &[(&str, &str)] = &[
    (
        "apid",
        "level=info msg=\"rpc request\" method=/machine.MachineService/ServiceList duration=1.4ms peer=192.0.2.1:52114",
    ),
    (
        "apid",
        "level=info msg=\"rpc request\" method=/machine.MachineService/Memory duration=0.9ms peer=192.0.2.1:52114",
    ),
    (
        "apid",
        "level=warn msg=\"rpc request\" method=/machine.MachineService/EtcdStatus duration=10.0s code=DeadlineExceeded",
    ),
    (
        "kubelet",
        "level=info msg=\"SyncLoop (PLEG): event for pod\" pod=\"kube-system/kube-proxy-8wq2d\" event=ContainerStarted",
    ),
    (
        "kubelet",
        "level=info msg=\"Successfully pulled image\" image=\"registry.k8s.io/pause:3.10\" duration=\"312ms\"",
    ),
    (
        "kubelet",
        "level=warn msg=\"Probe failed\" probeType=Readiness pod=\"kube-system/coredns-7c65d6cfc9-2xk4n\" output=\"HTTP probe failed with statuscode: 503\"",
    ),
    (
        "kubelet",
        "level=error msg=\"Error syncing pod, skipping\" err=\"failed to StartContainer for cilium-agent with CrashLoopBackOff: back-off 5m0s restarting failed container=cilium-agent pod=cilium-6vtzq_kube-system(3f1b9c2e-8d4a-4c7e-9a51-2b6f0d7e1a94)\" pod=\"kube-system/cilium-6vtzq\"",
    ),
    (
        "etcd",
        "level=info msg=\"finished scheduled compaction\" compact-revision=4815162 took=18.3ms",
    ),
    (
        "etcd",
        "level=warn msg=\"apply request took too long\" took=112.4ms expected-duration=100ms prefix=\"read-only range \"",
    ),
    (
        "containerd",
        "level=info msg=\"StartContainer for 3f9c2a41b7e0 returns successfully\"",
    ),
    (
        "containerd",
        "level=info msg=\"shim disconnected\" id=b71e0c55d2aa namespace=k8s.io",
    ),
    (
        "machined",
        "level=info msg=\"[talos] service[kubelet](Running): Health check successful\"",
    ),
    (
        "machined",
        "level=debug msg=\"[talos] controller runtime: task network.AddressSpecController reconciled\"",
    ),
    ("udevd", "udevd[812]: rules file changed, reloading"),
];

/// A realistic backlog for a node, oldest first, with timestamps ending now.
pub(crate) fn logs(node: &str, services: &[ServiceInfo]) -> Vec<LogEvent> {
    let now = chrono::Local::now();
    let catalog: Vec<&str> = services.iter().map(|service| service.id.as_str()).collect();
    let kubelet_unhealthy = services.iter().any(|service| {
        service.id == "kubelet"
            && service
                .health
                .as_ref()
                .is_some_and(|health| !health.unknown && !health.healthy)
    });
    let templates: Vec<_> = TEMPLATES
        .iter()
        .filter(|(service, _)| catalog.contains(service))
        .collect();
    if templates.is_empty() {
        return Vec::new();
    }
    (0..170u32)
        .map(|ix| {
            let at = now - chrono::Duration::milliseconds(i64::from(170 - ix) * 1400 + i64::from(ix % 7) * 97);
            let stamp = at.format("%Y-%m-%dT%H:%M:%S%.3f%:z");
            if kubelet_unhealthy && ix % 5 == 0 {
                return LogEvent::new(
                    "kubelet",
                    format!("{stamp} level=error msg=\"Failed to start healthz server\" err=\"listen tcp 127.0.0.1:10248: bind: address already in use\" node={node}"),
                )
                .talos();
            }
            let (service, line) = templates[(ix as usize * 7 + ix as usize / 3) % templates.len()];
            LogEvent::new(*service, format!("{stamp} {line}")).talos()
        })
        .collect()
}

/// One line for the live example stream; `sequence` keeps lines distinct.
pub(crate) fn stream_line(service: &str, sequence: u64) -> String {
    let stamp = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z");
    let matching: Vec<_> = TEMPLATES
        .iter()
        .filter(|(template_service, _)| *template_service == service)
        .map(|(_, line)| *line)
        .collect();
    let line = if matching.is_empty() {
        "level=info msg=\"heartbeat\""
    } else {
        matching[sequence as usize % matching.len()]
    };
    format!("{stamp} {line} seq={sequence}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::{Health, cluster_summary, node_summaries, service_health};

    #[test]
    fn example_contexts_cover_healthy_degraded_and_unreachable_states() {
        let prod = cluster("prod-fra", 0);
        let nodes = node_summaries(&prod);
        assert_eq!(nodes.len(), 6);
        assert_eq!(nodes.iter().filter(|node| !node.responding).count(), 1);
        let summary = cluster_summary(&prod, &nodes);
        assert_eq!(summary.services.unhealthy, 1);
        assert_eq!(
            summary.first_unhealthy,
            Some(("talos-wk-fra1-02".into(), "kubelet".into()))
        );
        assert!(summary.peak_memory.unwrap().1 > 85.);
        let staging = cluster("staging-eu", 0);
        assert!(staging.discovery_warning.is_some());
        let staging_nodes = node_summaries(&staging);
        assert_eq!(cluster_summary(&staging, &staging_nodes).versions.len(), 2);
        assert!(services("prod-fra", "talos-wk-fra1-03").is_empty());
        assert!(
            services("prod-fra", "talos-cp-fra1-01")
                .iter()
                .all(|service| service_health(service) != Health::Unhealthy)
        );
    }

    #[test]
    fn example_logs_parse_with_timestamps_and_levels() {
        let catalog = services("prod-fra", "talos-wk-fra1-02");
        let events = logs("talos-wk-fra1-02", &catalog);
        assert_eq!(events.len(), 170);
        let entry =
            freshkube_core::logs::parse_log_line(events[0].service.clone(), &events[0].line);
        assert!(entry.timestamp.is_some());
        assert!(stream_line("kubelet", 3).contains("seq=3"));
    }
}
