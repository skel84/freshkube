//! Diagnostics example data for a Talos node.
use super::ExampleNode;
use crate::diagnostic_runner::{
    AddonPod, AddonPodSource, AddonPresence, AddonSnapshot, AddonStatus, CniFileEvidence,
    CniSnapshot, DiagnosticContext, DiagnosticSnapshot, DiagnosticTarget, EtcdSnapshot,
    EtcdStatusSnapshot, FileProbe, KubernetesAccess, KubernetesSnapshot, ServicesSnapshot,
    SourceState, SourceUnavailable, SystemSnapshot,
};
use crate::diagnostics::{CniInfo, CniPodInfo, CniType, PodHealthInfo, UnhealthyPodInfo};

/// Invented evidence, judged by the same `evaluate()` as a live snapshot.
pub fn diagnostics(
    node: &ExampleNode,
    context: &str,
    kubernetes_ok: bool,
    control_plane_address: &str,
) -> DiagnosticSnapshot {
    let control_plane = node.control_plane;
    let unavailable = |what: &str| {
        SourceUnavailable {
        source: what.to_owned(),
        reason: "Selected Kubernetes access unavailable: no Kubernetes source is available for this example context"
            .to_owned(),
    }
    };
    let target = DiagnosticTarget::new(
        &node.name,
        &node.address,
        if control_plane {
            "controlplane"
        } else {
            "worker"
        },
        context,
    );
    // etcd, on control planes only.
    let etcd = if control_plane {
        let leader = node.name.ends_with("-01");
        let status = EtcdStatusSnapshot {
            node: node.name.clone(),
            member_id: 0x8e9e_05c5_2164_694d,
            leader_id: 0x8e9e_05c5_2164_694d,
            is_leader: leader,
            protocol_version: "3.6.0".into(),
            db_size_bytes: 41_943_040,
            db_size_in_use_bytes: 18_874_368,
            raft_index: 90_412,
            raft_term: 7,
            raft_applied_index: 90_412,
            errors: Vec::new(),
            is_learner: false,
        };
        EtcdSnapshot::Available(status)
    } else {
        EtcdSnapshot::NotApplicable
    };

    // Kubernetes.
    let access = if kubernetes_ok {
        SourceState::Available(KubernetesAccess {
            control_plane_address: control_plane_address.to_owned(),
            config_identity: context.to_owned(),
            source: None,
            warning: None,
        })
    } else {
        SourceState::Unavailable(SourceUnavailable {
            source: "Selected Kubernetes access".into(),
            reason: "no Kubernetes source is available for this example context".into(),
        })
    };
    let pod_health = if kubernetes_ok {
        SourceState::Available(PodHealthInfo {
            crashing: vec![UnhealthyPodInfo {
                name: "billing-worker-6f7c9d8b5-x2k4p".into(),
                namespace: "payments".into(),
                state: "CrashLoopBackOff".into(),
                restart_count: 14,
            }],
            image_pull_errors: Vec::new(),
            not_ready: Vec::new(),
            total_pods: 142,
        })
    } else {
        SourceState::Unavailable(unavailable("Kubernetes Pod API"))
    };
    // CNI: Flannel, detected from pods or, without Kubernetes, from files.
    let cni_pods = if kubernetes_ok {
        SourceState::Available(CniInfo {
            cni_type: CniType::Flannel,
            pods: (1..=3)
                .map(|n| CniPodInfo {
                    name: format!("kube-flannel-ds-{n}x7k"),
                    node_name: Some(format!("node-{n}")),
                    phase: "Running".into(),
                    ready: true,
                    restart_count: 0,
                })
                .collect(),
        })
    } else {
        SourceState::Unavailable(unavailable("Kubernetes CNI pod API"))
    };
    let cni = CniSnapshot {
        cni_type: SourceState::Available(CniType::Flannel),
        pods: cni_pods,
        files: CniFileEvidence {
            flannel_subnet: FileProbe::Present,
            flannel_subnet_valid: Some(true),
            br_netfilter: FileProbe::Present,
            ..Default::default()
        },
        cilium: None,
    };

    // Addons: cert-manager through CRDs and pods.
    let presence = |detected: bool| {
        if !kubernetes_ok {
            AddonPresence::Unknown
        } else if detected {
            AddonPresence::Detected
        } else {
            AddonPresence::NotDetected
        }
    };
    let addons = AddonSnapshot {
        crd_names: if kubernetes_ok {
            SourceState::Available(vec![
                "certificates.cert-manager.io".into(),
                "issuers.cert-manager.io".into(),
            ])
        } else {
            SourceState::Unavailable(unavailable("Kubernetes CRD API"))
        },
        pod_sources: vec![AddonPodSource {
            namespace: "cert-manager".into(),
            pods: if kubernetes_ok {
                SourceState::Available(vec![
                    "cert-manager-5c9d8c7b4-q8m2z".into(),
                    "cert-manager-webhook-7d6f9b8c5-k4n7p".into(),
                    "cert-manager-cainjector-6b8d7c9f4-t5w2r".into(),
                ])
            } else {
                SourceState::Unavailable(unavailable("Kubernetes cert-manager pod API"))
            },
        }],
        addons: vec![
            AddonStatus {
                id: "argocd",
                name: "Argo CD",
                presence: presence(false),
            },
            AddonStatus {
                id: "cert-manager",
                name: "cert-manager",
                presence: presence(true),
            },
            AddonStatus {
                id: "external-secrets",
                name: "External Secrets",
                presence: presence(false),
            },
            AddonStatus {
                id: "flux",
                name: "Flux",
                presence: presence(false),
            },
            AddonStatus {
                id: "kyverno",
                name: "Kyverno",
                presence: presence(false),
            },
        ],
        cert_manager: kubernetes_ok.then(|| {
            SourceState::Available(
                [
                    "cert-manager-5c9d8c7b4-q8m2z",
                    "cert-manager-webhook-7d6f9b8c5-k4n7p",
                    "cert-manager-cainjector-6b8d7c9f4-t5w2r",
                ]
                .into_iter()
                .map(|name| AddonPod {
                    name: Some(name.into()),
                    phase: Some("Running".into()),
                    ready: true,
                })
                .collect(),
            )
        }),
    };
    let mut snapshot = DiagnosticSnapshot {
        context: DiagnosticContext {
            target,
            platform: SourceState::Available("metal".into()),
            cpu_count: match node.cores {
                Some(cores) => SourceState::Available(cores),
                None => SourceState::unavailable("Talos CPUInfo API", "no CPU information"),
            },
            kubernetes_access: access,
        },
        system: SystemSnapshot {
            memory: match node.memory.clone() {
                Some(memory) => SourceState::Available(memory),
                None => SourceState::unavailable("Talos Memory API", "no memory information"),
            },
            load_average: match node.load.clone() {
                Some(load) => SourceState::Available(load),
                None => SourceState::unavailable("Talos LoadAvg API", "no load information"),
            },
        },
        services: ServicesSnapshot {
            services: SourceState::Available(node.services.clone()),
        },
        etcd,
        kubernetes: KubernetesSnapshot { pod_health },
        cni,
        addons,
        checks: Vec::new(),
    };
    snapshot.checks = snapshot.evaluate();
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic_runner::{
        DiagnosticCheck, LoadAverageSnapshot, MemorySnapshot, ServiceHealthSnapshot,
        ServiceSnapshot,
    };
    use crate::diagnostics::CheckStatus;

    fn node(name: &str, control_plane: bool) -> ExampleNode {
        ExampleNode::new(name, control_plane, Some(8)).with_diagnostics(
            "192.0.2.20",
            Some(MemorySnapshot {
                total_bytes: 8 * 1024 * 1024 * 1024,
                available_bytes: 5 * 1024 * 1024 * 1024,
                used_bytes: 3 * 1024 * 1024 * 1024,
                usage_percent: 37.5,
            }),
            Some(LoadAverageSnapshot {
                one_minute: 0.84,
                five_minutes: 0.71,
                fifteen_minutes: 0.66,
            }),
            vec![ServiceSnapshot {
                node: name.into(),
                id: "kubelet".into(),
                state: "Running".into(),
                health: SourceState::Available(ServiceHealthSnapshot {
                    healthy: false,
                    last_message: "Example health check failed".into(),
                }),
            }],
        )
    }

    fn check<'a>(snapshot: &'a DiagnosticSnapshot, id: &str) -> &'a DiagnosticCheck {
        snapshot.checks.iter().find(|check| check.id == id).unwrap()
    }

    #[test]
    fn control_plane_and_worker_keep_node_facts_and_differ_on_etcd() {
        let control_plane = diagnostics(
            &node("example-control-02", true),
            "example",
            true,
            "192.0.2.10",
        );
        let worker = diagnostics(
            &node("example-worker-02", false),
            "example",
            true,
            "192.0.2.10",
        );
        for (snapshot, name, role) in [
            (&control_plane, "example-control-02", "controlplane"),
            (&worker, "example-worker-02", "worker"),
        ] {
            assert_eq!(
                snapshot.context.target,
                DiagnosticTarget::new(name, "192.0.2.20", role, "example")
            );
            assert_eq!(snapshot.context.cpu_count.as_ref(), Some(&8));
            assert_eq!(
                snapshot.context.platform.as_ref().map(String::as_str),
                Some("metal")
            );
            assert_eq!(
                snapshot.context.kubernetes_access.as_ref().unwrap(),
                &KubernetesAccess {
                    control_plane_address: "192.0.2.10".into(),
                    config_identity: "example".into(),
                    source: None,
                    warning: None,
                }
            );
            let memory = snapshot.system.memory.as_ref().unwrap();
            assert_eq!(memory.total_bytes, 8 * 1024 * 1024 * 1024);
            assert_eq!(memory.available_bytes, 5 * 1024 * 1024 * 1024);
            assert_eq!(memory.used_bytes, 3 * 1024 * 1024 * 1024);
            assert_eq!(memory.usage_percent, 37.5);
            let load = snapshot.system.load_average.as_ref().unwrap();
            assert_eq!(
                (load.one_minute, load.five_minutes, load.fifteen_minutes),
                (0.84, 0.71, 0.66)
            );
            let services = snapshot.services.services.as_ref().unwrap();
            assert_eq!(services.len(), 1);
            assert_eq!(services[0].node, name);
            let service = check(snapshot, "service_kubelet");
            assert_eq!(service.status, CheckStatus::Fail);
            assert_eq!(
                service.details.as_deref(),
                Some("Example health check failed")
            );
            assert!(service.fix.is_some());
            assert_eq!(check(snapshot, "memory").status, CheckStatus::Pass);
            assert_eq!(check(snapshot, "cpu_load").status, CheckStatus::Pass);
            assert_eq!(
                snapshot.kubernetes.pod_health.as_ref().unwrap().total_pods,
                142
            );
        }
        assert!(matches!(control_plane.etcd, EtcdSnapshot::Available(_)));
        assert_eq!(check(&control_plane, "etcd").status, CheckStatus::Pass);
        assert!(matches!(worker.etcd, EtcdSnapshot::NotApplicable));
        assert!(!worker.checks.iter().any(|check| check.id == "etcd"));
    }

    #[test]
    fn missing_memory_and_load_remain_unknown() {
        let node = ExampleNode::new("example-worker", false, None).with_diagnostics(
            "192.0.2.20",
            None,
            None,
            Vec::new(),
        );
        let snapshot = diagnostics(&node, "example", true, "192.0.2.10");
        assert_eq!(
            snapshot.system.memory.unavailable_info(),
            Some(&SourceUnavailable {
                source: "Talos Memory API".into(),
                reason: "no memory information".into(),
            })
        );
        assert_eq!(
            snapshot.system.load_average.unavailable_info(),
            Some(&SourceUnavailable {
                source: "Talos LoadAvg API".into(),
                reason: "no load information".into(),
            })
        );
        assert_eq!(
            snapshot.context.cpu_count.unavailable_info(),
            Some(&SourceUnavailable {
                source: "Talos CPUInfo API".into(),
                reason: "no CPU information".into(),
            })
        );
        for id in ["memory", "cpu_load", "services"] {
            assert_eq!(check(&snapshot, id).status, CheckStatus::Unknown);
        }
    }

    #[test]
    fn unavailable_kubernetes_keeps_talos_evidence() {
        let snapshot = diagnostics(
            &node("example-control-02", true),
            "example-without-kubernetes",
            false,
            "",
        );
        assert_eq!(
            snapshot.context.target.config_identity,
            "example-without-kubernetes"
        );
        assert_eq!(
            snapshot.context.kubernetes_access.unavailable_info(),
            Some(&SourceUnavailable {
                source: "Selected Kubernetes access".into(),
                reason: "no Kubernetes source is available for this example context".into(),
            })
        );
        for info in [
            snapshot.kubernetes.pod_health.unavailable_info().unwrap(),
            snapshot.cni.pods.unavailable_info().unwrap(),
            snapshot.addons.crd_names.unavailable_info().unwrap(),
            snapshot.addons.pod_sources[0]
                .pods
                .unavailable_info()
                .unwrap(),
        ] {
            assert_eq!(
                info.reason,
                "Selected Kubernetes access unavailable: no Kubernetes source is available for this example context"
            );
        }
        assert!(
            snapshot
                .addons
                .addons
                .iter()
                .all(|addon| addon.presence == AddonPresence::Unknown)
        );
        assert!(snapshot.addons.cert_manager.is_none());
        for id in ["kubernetes_api", "pod_health", "flannel_pods"] {
            assert_eq!(check(&snapshot, id).status, CheckStatus::Unknown);
        }
        for id in ["memory", "cpu_load", "etcd", "br_netfilter", "cni"] {
            assert_eq!(check(&snapshot, id).status, CheckStatus::Pass);
        }
        assert_eq!(
            check(&snapshot, "service_kubelet").status,
            CheckStatus::Fail
        );
        // Core accepts the desktop decision even for the adapter's special context name.
        let available = diagnostics(
            &node("example-control-02", true),
            "homelab",
            true,
            "192.0.2.10",
        );
        assert!(available.context.kubernetes_access.is_available());
    }

    #[test]
    fn only_control_plane_names_ending_in_01_are_etcd_leaders() {
        for (name, expected_leader) in [("example-control-01", true), ("example-control-02", false)]
        {
            let snapshot = diagnostics(&node(name, true), "example", true, "192.0.2.10");
            let EtcdSnapshot::Available(status) = &snapshot.etcd else {
                panic!("control plane must have etcd evidence");
            };
            assert_eq!(status.node, name);
            assert_eq!(status.is_leader, expected_leader);
            assert_eq!(status.member_id, 0x8e9e_05c5_2164_694d);
            assert_eq!(status.leader_id, 0x8e9e_05c5_2164_694d);
            assert_eq!(status.raft_index, 90_412);
            assert_eq!(status.raft_applied_index, 90_412);
            assert_eq!(status.raft_term, 7);
            let message = &check(&snapshot, "etcd").message;
            if expected_leader {
                assert_eq!(message, "Leader, healthy");
            } else {
                assert_eq!(message, "Follower (leader: 8e9e05c52164694d)");
            }
        }
        let worker = diagnostics(
            &node("example-worker-01", false),
            "example",
            true,
            "192.0.2.10",
        );
        assert!(matches!(worker.etcd, EtcdSnapshot::NotApplicable));
    }
}
