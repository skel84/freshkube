use super::*;

/// Example data for `--fixture`: six nodes on Talos v1.13.2, one worker on an
/// older Kubernetes patch, one worker that doesn't answer, a healthy etcd.
pub(super) fn example(source: &ScreenSource) -> Result<LifecycleView, String> {
    let Some(node) = source.node() else {
        return Err("Example data has no such node".into());
    };
    if !node.responding {
        return Err(format!(
            "{} didn't answer the Talos API within 10 s (example)",
            node.name
        ));
    }
    let silent_reason = "Talos read request timed out (example)";
    let nodes: Vec<NodeLifecycleSnapshot> = source
        .nodes
        .iter()
        .map(|node| {
            let unavailable = |what: &str| SourceSnapshot::Unavailable {
                reason: format!("{what} is unavailable: {silent_reason}"),
            };
            if node.responding {
                NodeLifecycleSnapshot {
                    name: node.name.clone(),
                    address: Some(node.address.clone()),
                    machine_type: Some(
                        if node.role == crate::presentation::Role::ControlPlane {
                            "controlplane"
                        } else {
                            "worker"
                        }
                        .into(),
                    ),
                    version: SourceSnapshot::Available(
                        node.version.clone().unwrap_or_else(|| "v1.13.2".into()),
                    ),
                    platform: SourceSnapshot::Available("metal".into()),
                    time_synchronization: SourceSnapshot::Available(TimeSynchronizationAudit {
                        server: "time.cloudflare.com".into(),
                        offset_seconds: 0.0021,
                        synced: true,
                    }),
                    config_hash: SourceSnapshot::Available("42".into()),
                }
            } else {
                NodeLifecycleSnapshot {
                    name: node.name.clone(),
                    address: Some(node.address.clone()),
                    machine_type: Some(
                        if node.role == crate::presentation::Role::ControlPlane {
                            "controlplane"
                        } else {
                            "worker"
                        }
                        .into(),
                    ),
                    version: unavailable("Talos version"),
                    platform: unavailable("Talos platform"),
                    time_synchronization: SourceSnapshot::Unavailable {
                        reason: silent_reason.into(),
                    },
                    config_hash: SourceSnapshot::Unavailable {
                        reason: "Machineconfig request timed out (example)".into(),
                    },
                }
            }
        })
        .collect();
    let discovery = source
        .nodes
        .iter()
        .map(|node| DiscoveryRosterEntry {
            id: node.name.clone(),
            name: node.name.clone(),
            addresses: vec![node.address.clone()],
            machine_type: if node.role == crate::presentation::Role::ControlPlane {
                "controlplane"
            } else {
                "worker"
            }
            .into(),
        })
        .collect();
    let kubernetes = source
        .nodes
        .iter()
        .map(|node| KubernetesNodeRosterEntry {
            name: node.name.clone(),
            internal_address: Some(node.address.clone()),
            is_control_plane: node.role == crate::presentation::Role::ControlPlane,
        })
        .collect();
    let kubelets = source
        .nodes
        .iter()
        .map(|node| KubeletEntry {
            name: node.name.clone(),
            address: Some(node.address.clone()),
            version: if node.name.contains("wk-fra1-02") {
                "v1.34.1".into()
            } else {
                "v1.34.3".into()
            },
        })
        .collect();
    let total = 3.min(
        source
            .nodes
            .iter()
            .filter(|node| node.role == crate::presentation::Role::ControlPlane)
            .count()
            .max(1),
    );
    let quorum = freshkube_core::indicators::quorum(total, total);
    Ok(LifecycleView {
        snapshot: LifecycleSnapshot {
            identity: SourceSnapshot::Available(ClusterIdentity {
                context_name: source.target.context.clone(),
                endpoint_addresses: vec![source.target.address.clone()],
                target_addresses: vec![source.target.address.clone()],
            }),
            talos_discovery: SourceSnapshot::Available(discovery),
            kubernetes_roster: SourceSnapshot::Available(kubernetes),
            nodes,
            etcd_pre_operation: SourceSnapshot::Available(EtcdPreOperationAudit {
                total_members: total,
                responding_members: total,
                quorum_required: quorum.required,
                can_lose: quorum.remaining_tolerance,
                quorum: quorum.state,
            }),
            alerts: Vec::new(),
        },
        kubelets: SourceSnapshot::Available(kubelets),
        node_observation: None,
        display: Default::default(),
    }
    .prepare())
}
