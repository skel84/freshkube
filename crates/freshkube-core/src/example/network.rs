//! Network example data for a Talos node.
use std::time::Instant;

use chrono::{DateTime, Utc};
use talos_rs::{
    ConnectionInfo, ConnectionState, KubeSpanPeerStatus, NetDevRate, NetDevStats, ServiceHealth,
    ServiceInfo,
};

use super::ExampleNode;
use crate::inspection::{
    InspectionTarget, NetworkInspectionSnapshot, NetworkInterfaceSnapshot, NetworkSampleState,
    NetworkTotals, inspect_network_connections,
};

/// name, receive bytes/s, transmit bytes/s, bytes already counted at boot.
type ExampleInterface = (&'static str, u64, u64, u64);

fn example_connection(
    protocol: &str,
    local: (&str, u32),
    remote: (&str, u32),
    state: ConnectionState,
    process: Option<(&str, u32)>,
) -> ConnectionInfo {
    ConnectionInfo {
        protocol: protocol.into(),
        local_ip: local.0.into(),
        local_port: local.1,
        remote_ip: remote.0.into(),
        remote_port: remote.1,
        state,
        rx_queue: 0,
        tx_queue: 0,
        process_pid: process.map(|(_, pid)| pid),
        process_name: process.map(|(name, _)| name.into()),
        netns: None,
    }
}

/// Example network data for a plausible Talos node: rates wobble on each
/// refresh, and the degraded worker shows errors and a pile of sockets
/// waiting on the API server.
pub fn network(
    node: &ExampleNode,
    target: InspectionTarget,
    nodes: &[ExampleNode],
    tick: u64,
    sampled_at: Instant,
) -> NetworkInspectionSnapshot {
    use ConnectionState::{CloseWait, Established, Listen, SynSent, TimeWait};
    let control_plane = node.control_plane;
    let degraded = node.name.contains("wk-fra1-02");
    let ip = target
        .address
        .split(':')
        .next()
        .unwrap_or("10.20.0.11")
        .to_owned();
    let ip = ip.as_str();
    let apiserver = nodes
        .iter()
        .find(|other| other.control_plane && other.responding)
        .map_or("192.0.2.10".to_owned(), |other| {
            other
                .address
                .split(':')
                .next()
                .unwrap_or("192.0.2.10")
                .to_owned()
        });

    let mut spec: Vec<ExampleInterface> = vec![
        ("lo", 180_000, 180_000, 2_400_000_000),
        (
            "eth0",
            if control_plane { 2_400_000 } else { 5_200_000 },
            if control_plane { 1_900_000 } else { 3_100_000 },
            410_000_000_000,
        ),
        ("flannel.1", 1_100_000, 900_000, 96_000_000_000),
        ("cni0", 1_300_000, 1_250_000, 120_000_000_000),
        ("veth3f1a2b9c", 420_000, 380_000, 31_000_000_000),
        ("veth9c8d77e1", 160_000, 140_000, 11_000_000_000),
    ];
    if !control_plane {
        spec.extend([
            ("veth5be2a104", 880_000, 760_000, 44_000_000_000),
            ("veth01c47d3a", 52_000, 40_000, 3_200_000_000),
        ]);
    }
    if !degraded {
        spec.push(("kubespan", 360_000, 330_000, 18_000_000_000));
    }

    let wobble = |ix: usize| 1.0 + ((tick as usize + ix * 3) % 5) as f64 * 0.06;
    let interval = 2.0;
    let interfaces: Vec<NetworkInterfaceSnapshot> = spec
        .iter()
        .enumerate()
        .map(|(ix, (name, rx, tx, base))| {
            let (rx_now, tx_now) = (
                (*rx as f64 * wobble(ix)) as u64,
                (*tx as f64 * wobble(ix + 1)) as u64,
            );
            let elapsed = (tick as f64 * interval) as u64;
            // Transmit runs at 55-70% of receive, varied per interface.
            let tx_base = (*base as f64 * (0.55 + ((ix * 7 + 3) % 16) as f64 / 100.0)) as u64;
            let mut stats = NetDevStats {
                name: (*name).into(),
                rx_bytes: base + rx * elapsed + rx_now * 2,
                rx_packets: (base + rx * elapsed) / 1_187,
                tx_bytes: tx_base + tx * elapsed + tx_now * 2,
                tx_packets: (tx_base + tx * elapsed) / 1_043,
                rx_errors: 0,
                rx_dropped: 0,
                tx_errors: 0,
                tx_dropped: 0,
            };
            if degraded && *name == "eth0" {
                stats.rx_errors = 37;
                stats.rx_dropped = 9_312 + tick * 11;
                stats.tx_dropped = 41;
            }
            let rate = (tick > 0).then(|| NetDevRate {
                name: (*name).into(),
                rx_bytes_per_sec: rx_now,
                tx_bytes_per_sec: tx_now,
                rx_errors: stats.rx_errors,
                tx_errors: stats.tx_errors,
                rx_dropped: stats.rx_dropped,
                tx_dropped: stats.tx_dropped,
            });
            NetworkInterfaceSnapshot { stats, rate }
        })
        .collect();
    let totals = NetworkTotals {
        rx_bytes_per_sec: interfaces
            .iter()
            .filter_map(|i| i.rate.as_ref())
            .map(|r| r.rx_bytes_per_sec)
            .sum(),
        tx_bytes_per_sec: interfaces
            .iter()
            .filter_map(|i| i.rate.as_ref())
            .map(|r| r.tx_bytes_per_sec)
            .sum(),
        errors: interfaces.iter().map(|i| i.stats.total_errors()).sum(),
        dropped: interfaces.iter().map(|i| i.stats.total_dropped()).sum(),
    };

    let mut conns = vec![
        example_connection(
            "tcp",
            ("0.0.0.0", 50000),
            ("", 0),
            Listen,
            Some(("apid", 604)),
        ),
        example_connection(
            "tcp",
            ("0.0.0.0", 10250),
            ("", 0),
            Listen,
            Some(("kubelet", 781)),
        ),
        example_connection(
            "tcp",
            ("0.0.0.0", 10256),
            ("", 0),
            Listen,
            Some(("kube-proxy", 1120)),
        ),
        example_connection(
            "udp",
            ("0.0.0.0", 53),
            ("", 0),
            Listen,
            Some(("coredns", 1188)),
        ),
        example_connection(
            "tcp",
            ("127.0.0.1", 10248),
            ("", 0),
            Listen,
            Some(("kubelet", 781)),
        ),
        example_connection(
            "tcp",
            (ip, 50000),
            ("192.0.2.5", 51734),
            Established,
            Some(("apid", 604)),
        ),
        example_connection(
            "tcp",
            (ip, 10250),
            (&apiserver, 40122),
            Established,
            Some(("kubelet", 781)),
        ),
        example_connection(
            "tcp",
            ("127.0.0.1", 40412),
            ("127.0.0.1", 10248),
            Established,
            Some(("kubelet", 781)),
        ),
    ];
    if control_plane {
        conns.extend([
            example_connection(
                "tcp",
                ("0.0.0.0", 50001),
                ("", 0),
                Listen,
                Some(("trustd", 611)),
            ),
            example_connection(
                "tcp",
                ("0.0.0.0", 2379),
                ("", 0),
                Listen,
                Some(("etcd", 690)),
            ),
            example_connection(
                "tcp",
                ("0.0.0.0", 2380),
                ("", 0),
                Listen,
                Some(("etcd", 690)),
            ),
            example_connection(
                "tcp",
                ("0.0.0.0", 6443),
                ("", 0),
                Listen,
                Some(("kube-apiserver", 1322)),
            ),
            example_connection(
                "tcp",
                ("0.0.0.0", 10259),
                ("", 0),
                Listen,
                Some(("kube-scheduler", 1355)),
            ),
            example_connection(
                "tcp",
                ("0.0.0.0", 10257),
                ("", 0),
                Listen,
                Some(("kube-controller-manager", 1340)),
            ),
            example_connection(
                "tcp",
                (ip, 2380),
                ("192.0.2.11", 48512),
                Established,
                Some(("etcd", 690)),
            ),
            example_connection(
                "tcp",
                (ip, 2380),
                ("192.0.2.12", 52044),
                Established,
                Some(("etcd", 690)),
            ),
            example_connection(
                "tcp",
                ("127.0.0.1", 2379),
                ("127.0.0.1", 38874),
                Established,
                Some(("etcd", 690)),
            ),
            example_connection(
                "tcp",
                (ip, 50001),
                ("192.0.2.20", 44218),
                Established,
                Some(("trustd", 611)),
            ),
            example_connection(
                "tcp",
                (ip, 50001),
                ("192.0.2.21", 44990),
                Established,
                Some(("trustd", 611)),
            ),
        ]);
        for n in 0..9u32 {
            conns.push(example_connection(
                "tcp",
                (ip, 6443),
                (&format!("10.244.{}.{}", n % 3, 10 + n), 41000 + n * 37),
                Established,
                Some(("kube-apiserver", 1322)),
            ));
        }
        for n in 0..4u32 {
            conns.push(example_connection(
                "tcp",
                (ip, 6443),
                ("192.0.2.20", 52100 + n),
                TimeWait,
                None,
            ));
        }
    } else {
        for n in 0..4u32 {
            conns.push(example_connection(
                "tcp",
                (ip, 43000 + n * 11),
                (&apiserver, 6443),
                Established,
                Some(("kubelet", 781)),
            ));
        }
        conns.extend([
            example_connection(
                "tcp",
                ("0.0.0.0", 5432),
                ("", 0),
                Listen,
                Some(("postgres", 2140)),
            ),
            example_connection(
                "tcp",
                ("10.244.2.14", 5432),
                ("10.244.1.31", 47320),
                Established,
                Some(("postgres", 2140)),
            ),
            example_connection(
                "tcp",
                ("10.244.2.18", 8080),
                ("10.244.0.9", 33310),
                Established,
                Some(("node", 2215)),
            ),
            example_connection(
                "tcp",
                (ip, 50001),
                (&apiserver, 38800),
                Established,
                Some(("trustd", 611)),
            ),
        ]);
    }
    if degraded {
        // Socket pile-up toward the API server: connections that never
        // complete, and plenty of closed ones waiting out TIME_WAIT.
        for n in 0..7u32 {
            conns.push(example_connection(
                "tcp",
                (ip, 44000 + n * 13),
                (&apiserver, 6443),
                SynSent,
                Some(("kubelet", 781)),
            ));
        }
        for n in 0..132u32 {
            conns.push(example_connection(
                "tcp",
                (ip, 30000 + n * 7),
                (&apiserver, 6443),
                TimeWait,
                None,
            ));
        }
        conns.push(example_connection(
            "tcp",
            ("10.244.2.18", 8080),
            ("10.244.0.9", 33342),
            CloseWait,
            Some(("node", 2215)),
        ));
    }
    let connections = inspect_network_connections(conns);

    let healthy = |id: &str| ServiceInfo {
        id: id.into(),
        state: "Running".into(),
        health: Some(ServiceHealth {
            unknown: false,
            healthy: !(degraded && id == "kubelet"),
            last_message: String::new(),
        }),
    };
    let mut services: Vec<ServiceInfo> = ["apid", "containerd", "kubelet", "trustd"]
        .into_iter()
        .map(healthy)
        .collect();
    if control_plane {
        services.push(healthy("etcd"));
    }

    NetworkInspectionSnapshot {
        target,
        sampled_at,
        interfaces,
        totals,
        connections: Some(connections),
        services: Some(services),
        next_sample: NetworkSampleState::default(),
        unavailable: Vec::new(),
    }
}

/// Example KubeSpan peers: the degraded worker can't be read, the rest see
/// every other node.
pub fn kubespan(
    node: &ExampleNode,
    nodes: &[ExampleNode],
    tick: u64,
    now: DateTime<Utc>,
    port: u16,
) -> Result<Vec<KubeSpanPeerStatus>, String> {
    let degraded = node.name.contains("wk-fra1-02");
    if degraded {
        Err("Example: the kubespanpeerstatus query timed out after 12 s".into())
    } else {
        let stamp = |seconds: i64| {
            (now - chrono::Duration::seconds(seconds))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        };
        Ok(nodes
            .iter()
            .filter(|other| other.name != node.name)
            .enumerate()
            .map(|(ix, other)| {
                let ix = ix as i64;
                KubeSpanPeerStatus {
                    id: format!("{:0<43}=", other.name.replace('-', "")),
                    label: other.name.clone(),
                    endpoint: Some(format!(
                        "{}:{port}",
                        other.address.split(':').next().unwrap_or(&other.address)
                    )),
                    state: if other.responding { "up" } else { "down" }.into(),
                    rtt_ms: other.responding.then_some(0.4 + ix as f64 * 0.3),
                    last_handshake: Some(if other.responding {
                        stamp(20 + ix * 17 + tick as i64 * 2)
                    } else {
                        "0001-01-01T00:00:00Z".into()
                    }),
                    rx_bytes: if other.responding {
                        18_000_000_000 + ix as u64 * 2_000_000_000 + tick * 700_000
                    } else {
                        0
                    },
                    tx_bytes: if other.responding {
                        14_000_000_000 + ix as u64 * 1_500_000_000 + tick * 600_000
                    } else {
                        0
                    },
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const PORT: u16 = 51820;

    fn node(name: &str, control_plane: bool, address: &str, responding: bool) -> ExampleNode {
        ExampleNode::new(name, control_plane, None).with_network(address, responding)
    }

    fn nodes() -> Vec<ExampleNode> {
        vec![
            node("example-silent-control", true, "192.0.2.9:50000", false),
            node("example-control", true, "192.0.2.10:50000", true),
            node("example-worker", false, "192.0.2.20:50000", true),
            node("example-wk-fra1-02", false, "192.0.2.21:50000", true),
        ]
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 2, 12, 0, 0).unwrap()
    }

    fn snapshot(
        node: &ExampleNode,
        nodes: &[ExampleNode],
        tick: u64,
        sampled_at: Instant,
    ) -> NetworkInspectionSnapshot {
        network(
            node,
            InspectionTarget::new(&node.name, "192.0.2.30:50000"),
            nodes,
            tick,
            sampled_at,
        )
    }

    #[test]
    fn control_plane_and_worker_have_their_own_interfaces_sockets_and_services() {
        let nodes = nodes();
        let sampled_at = Instant::now();
        let control = snapshot(&nodes[1], &nodes, 0, sampled_at);
        let worker = snapshot(&nodes[2], &nodes, 0, sampled_at);
        assert_eq!(control.sampled_at, sampled_at);
        assert_eq!(
            format!("{control:?}"),
            format!("{:?}", snapshot(&nodes[1], &nodes, 0, sampled_at))
        );
        assert_eq!(worker.sampled_at, sampled_at);
        assert_eq!(
            control.target,
            InspectionTarget::new("example-control", "192.0.2.30:50000")
        );
        assert_eq!(control.interfaces.len(), 7);
        assert_eq!(worker.interfaces.len(), 9);
        let connections = |snapshot: &NetworkInspectionSnapshot| {
            snapshot.connections.as_ref().unwrap().connections.len()
        };
        assert_eq!(connections(&control), 32);
        assert_eq!(connections(&worker), 16);
        for (snapshot, has_etcd) in [(&control, true), (&worker, false)] {
            assert_eq!(
                snapshot
                    .services
                    .as_ref()
                    .unwrap()
                    .iter()
                    .any(|service| service.id == "etcd"),
                has_etcd
            );
            let connections = &snapshot.connections.as_ref().unwrap().connections;
            assert_eq!(
                connections
                    .iter()
                    .any(|entry| entry.connection.local_port == 6443),
                has_etcd
            );
            assert!(
                connections
                    .iter()
                    .any(|entry| entry.connection.local_ip == "192.0.2.30")
            );
            assert!(
                connections
                    .iter()
                    .any(|entry| entry.connection.remote_ip == "192.0.2.10")
            );
            assert!(
                !connections
                    .iter()
                    .any(|entry| entry.connection.remote_ip == "192.0.2.9")
            );
            assert!(!snapshot.is_partial());
        }
        let fallback = snapshot(&nodes[2], &[], 0, sampled_at);
        assert!(
            fallback
                .connections
                .unwrap()
                .connections
                .iter()
                .any(|entry| entry.connection.remote_ip == "192.0.2.10")
        );
    }

    #[test]
    fn degraded_worker_keeps_socket_pile_up_kubelet_health_and_kubespan_message() {
        let nodes = nodes();
        let degraded = snapshot(&nodes[3], &nodes, 3, Instant::now());
        assert_eq!(degraded.interfaces.len(), 8);
        assert!(
            !degraded
                .interfaces
                .iter()
                .any(|interface| interface.stats.name == "kubespan")
        );
        let eth0 = &degraded
            .interfaces
            .iter()
            .find(|interface| interface.stats.name == "eth0")
            .unwrap()
            .stats;
        assert_eq!(
            (eth0.rx_errors, eth0.rx_dropped, eth0.tx_dropped),
            (37, 9_345, 41)
        );
        let connections = &degraded.connections.as_ref().unwrap().connections;
        assert_eq!(connections.len(), 156);
        for (state, count) in [
            (ConnectionState::SynSent, 7),
            (ConnectionState::TimeWait, 132),
            (ConnectionState::CloseWait, 1),
        ] {
            assert_eq!(
                connections
                    .iter()
                    .filter(|entry| entry.connection.state == state)
                    .count(),
                count
            );
        }
        for service in degraded.services.as_ref().unwrap() {
            assert_eq!(
                service.health.as_ref().unwrap().healthy,
                service.id != "kubelet"
            );
        }
        assert_eq!(
            kubespan(&nodes[3], &nodes, 3, now(), PORT).unwrap_err(),
            "Example: the kubespanpeerstatus query timed out after 12 s"
        );
    }

    #[test]
    fn nonresponding_peer_keeps_zero_counters_and_unset_handshake() {
        let nodes = nodes();
        let peers = kubespan(&nodes[2], &nodes, 3, now(), PORT).unwrap();
        assert_eq!(peers.len(), 3);
        assert!(peers.iter().all(|peer| peer.label != "example-worker"));
        let silent = &peers[0];
        assert_eq!(silent.id, "examplesilentcontrol00000000000000000000000=");
        assert_eq!(silent.endpoint.as_deref(), Some("192.0.2.9:51820"));
        assert_eq!(silent.state, "down");
        assert_eq!(silent.rtt_ms, None);
        assert_eq!(
            silent.last_handshake.as_deref(),
            Some("0001-01-01T00:00:00Z")
        );
        assert_eq!((silent.rx_bytes, silent.tx_bytes), (0, 0));
        let up = &peers[1];
        assert_eq!(up.label, "example-control");
        assert_eq!(up.state, "up");
        assert_eq!(up.rtt_ms, Some(0.7));
        assert_eq!(up.last_handshake.as_deref(), Some("2026-01-02T11:59:17Z"));
        assert_eq!((up.rx_bytes, up.tx_bytes), (20_002_100_000, 15_501_800_000));
    }

    #[test]
    fn tick_moves_only_rates_counters_and_handshake_age() {
        let nodes = nodes();
        let sampled_at = Instant::now();
        for node in &nodes[1..] {
            let first = snapshot(node, &nodes, 0, sampled_at);
            let mut later = snapshot(node, &nodes, 3, sampled_at);
            assert!(
                first
                    .interfaces
                    .iter()
                    .all(|interface| interface.rate.is_none())
            );
            assert!(
                later
                    .interfaces
                    .iter()
                    .all(|interface| interface.rate.is_some())
            );
            assert!(later.totals.rx_bytes_per_sec > 0);
            assert!(later.totals.tx_bytes_per_sec > 0);
            assert_eq!(later.totals.errors, first.totals.errors);
            assert_eq!(
                later.totals.dropped - first.totals.dropped,
                if node.name.contains("wk-fra1-02") {
                    33
                } else {
                    0
                }
            );
            for (later, first) in later.interfaces.iter_mut().zip(&first.interfaces) {
                assert!(later.stats.rx_bytes > first.stats.rx_bytes);
                assert!(later.stats.tx_bytes > first.stats.tx_bytes);
                assert!(later.stats.rx_packets > first.stats.rx_packets);
                assert!(later.stats.tx_packets > first.stats.tx_packets);
                later.stats.rx_bytes = first.stats.rx_bytes;
                later.stats.tx_bytes = first.stats.tx_bytes;
                later.stats.rx_packets = first.stats.rx_packets;
                later.stats.tx_packets = first.stats.tx_packets;
                later.stats.rx_dropped = first.stats.rx_dropped;
                later.rate = first.rate.clone();
            }
            later.totals = first.totals;
            assert_eq!(format!("{later:?}"), format!("{first:?}"));
            let first = kubespan(node, &nodes, 0, now(), PORT);
            let later = kubespan(node, &nodes, 3, now(), PORT);
            match (first, later) {
                (Ok(first), Ok(mut later)) => {
                    for (ix, (later, first)) in later.iter_mut().zip(&first).enumerate() {
                        if later.state == "up" {
                            assert_eq!(later.rx_bytes - first.rx_bytes, 2_100_000);
                            assert_eq!(later.tx_bytes - first.tx_bytes, 1_800_000);
                            assert_eq!(
                                later.last_handshake.as_ref().unwrap(),
                                &(now() - chrono::Duration::seconds(20 + ix as i64 * 17 + 6))
                                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                            );
                            later.rx_bytes = first.rx_bytes;
                            later.tx_bytes = first.tx_bytes;
                            later.last_handshake = first.last_handshake.clone();
                        }
                    }
                    assert_eq!(format!("{later:?}"), format!("{first:?}"));
                }
                (Err(first), Err(later)) => assert_eq!(first, later),
                _ => panic!("tick changed KubeSpan availability"),
            }
        }
    }
}
