use super::*;

/// Example diagnostics for `--fixture`, shaped like the real runner's output.
/// A node that doesn't answer errors, so the screen shows its silent state.
/// The `homelab` context has no Kubernetes source, which exercises the path
/// where Kubernetes checks are unknown but Talos checks still show.
pub(super) fn example(source: &ScreenSource) -> Result<DiagnosticSnapshot, String> {
    use freshkube_core::diagnostic_runner::{
        AddonPodSource, AddonStatus, CniFileEvidence, CniSnapshot, DiagnosticContext,
        EtcdStatusSnapshot, KubernetesSnapshot, LoadAverageSnapshot, MemorySnapshot,
        ServiceHealthSnapshot, ServiceSnapshot, ServicesSnapshot, SystemSnapshot,
    };
    use freshkube_core::diagnostics::{CniInfo, CniPodInfo, PodHealthInfo, UnhealthyPodInfo};
    use freshkube_core::formatting::format_bytes;

    use crate::presentation::Role as NodeRole;

    let Some(node) = source.node() else {
        return Err("Example data has no such node".into());
    };
    if !node.responding {
        return Err(format!(
            "{} didn't answer the Talos API within 10 s (example)",
            node.name
        ));
    }
    let control_plane = node.role == NodeRole::ControlPlane;
    let kubernetes_ok = source.target.context != "homelab";
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
        &source.target.context,
    );
    let mut checks: Vec<DiagnosticCheck> = Vec::new();

    // System.
    let memory = node.memory.as_ref().map(|memory| MemorySnapshot {
        total_bytes: memory.total,
        available_bytes: memory.total.saturating_sub(memory.used),
        used_bytes: memory.used,
        usage_percent: memory.percent() as f32,
    });
    match &memory {
        Some(memory) => {
            let message = format!(
                "{} / {} ({:.0}%)",
                format_bytes(memory.used_bytes),
                format_bytes(memory.total_bytes),
                memory.usage_percent
            );
            checks.push(if memory.usage_percent > 90.0 {
                DiagnosticCheck::fail("memory", CheckCategory::System, "Memory", message)
            } else if memory.usage_percent > 80.0 {
                DiagnosticCheck::warn("memory", CheckCategory::System, "Memory", message)
            } else {
                DiagnosticCheck::pass("memory", CheckCategory::System, "Memory", message)
            });
        }
        None => checks.push(DiagnosticCheck::unknown(
            "memory",
            CheckCategory::System,
            "Memory",
            &SourceUnavailable {
                source: "Talos Memory API".into(),
                reason: "Talos returned no memory information".into(),
            },
        )),
    }
    let load = node.load.map(|load| LoadAverageSnapshot {
        one_minute: load[0],
        five_minutes: load[1],
        fifteen_minutes: load[2],
    });
    match (&load, node.cores) {
        (Some(load), Some(cores)) => {
            let message = format!(
                "{:.2} / {:.2} / {:.2}",
                load.one_minute, load.five_minutes, load.fifteen_minutes
            );
            let threshold = cores as f64 * 1.5;
            checks.push(if load.one_minute > threshold {
                DiagnosticCheck::warn("cpu_load", CheckCategory::System, "CPU Load", message)
                    .with_details(format!(
                        "Load exceeds threshold ({threshold:.1} for {cores} CPUs)"
                    ))
            } else {
                DiagnosticCheck::pass("cpu_load", CheckCategory::System, "CPU Load", message)
            });
        }
        _ => checks.push(DiagnosticCheck::unknown(
            "cpu_load",
            CheckCategory::System,
            "CPU Load",
            &SourceUnavailable {
                source: "Talos LoadAvg API".into(),
                reason: "Talos returned no load information".into(),
            },
        )),
    }

    // Services.
    let services: Vec<ServiceSnapshot> = node
        .services
        .iter()
        .map(|service| ServiceSnapshot {
            node: node.name.clone(),
            id: service.id.clone(),
            state: service.state.clone(),
            health: match &service.health {
                Some(health) if !health.unknown => SourceState::Available(ServiceHealthSnapshot {
                    healthy: health.healthy,
                    last_message: health.last_message.clone(),
                }),
                other => SourceState::unavailable(
                    "Talos service health",
                    match other {
                        Some(health) => format!(
                            "Talos reports service health as unknown: {}",
                            health.last_message
                        ),
                        None => "Talos did not return a health state for this service".to_owned(),
                    },
                ),
            },
        })
        .collect();
    for service in &services {
        let id = format!("service_{}", service.id);
        checks.push(match &service.health {
            SourceState::Available(health) if health.healthy => {
                let check = DiagnosticCheck::pass(
                    id,
                    CheckCategory::Services,
                    service.id.clone(),
                    format!("{} (healthy)", service.state),
                );
                if health.last_message.is_empty() {
                    check
                } else {
                    check.with_details(health.last_message.clone())
                }
            }
            SourceState::Available(health) => {
                let check = DiagnosticCheck::fail(
                    id,
                    CheckCategory::Services,
                    service.id.clone(),
                    format!("{} (unhealthy)", service.state),
                )
                .with_fix(DiagnosticFix {
                    description: format!("Restart {}", service.id),
                    action: DiagnosticFixAction::RestartService {
                        service_id: service.id.clone(),
                    },
                });
                if health.last_message.is_empty() {
                    check
                } else {
                    check.with_details(health.last_message.clone())
                }
            }
            SourceState::Unavailable(info) => {
                DiagnosticCheck::unknown(id, CheckCategory::Services, service.id.clone(), info)
                    .with_details(format!(
                        "{} unavailable: {}\nReported state: {}",
                        info.source, info.reason, service.state
                    ))
            }
        });
    }

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
        let role = if leader {
            "Leader, healthy".to_owned()
        } else {
            format!("Follower (leader: {:x})", status.leader_id)
        };
        checks.push(
            DiagnosticCheck::pass("etcd", CheckCategory::Kubernetes, "Etcd", role).with_details(
                "Database: 40.0 MB total, 18.0 MB in use\nRaft: term 7, index 90412, applied 90412",
            ),
        );
        EtcdSnapshot::Available(status)
    } else {
        EtcdSnapshot::NotApplicable
    };

    // Kubernetes.
    let access = if kubernetes_ok {
        SourceState::Available(KubernetesAccess {
            control_plane_address: source
                .nodes
                .iter()
                .find(|node| node.role == NodeRole::ControlPlane)
                .map(|node| node.address.clone())
                .unwrap_or_default(),
            config_identity: source.target.context.clone(),
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
            total_pods: 142,
        })
    } else {
        SourceState::Unavailable(unavailable("Kubernetes Pod API"))
    };
    match &access {
        SourceState::Available(access) => checks.push(
            DiagnosticCheck::pass(
                "kubernetes_api",
                CheckCategory::Kubernetes,
                "Kubernetes API",
                format!("Pinned kubeconfig from {}", access.control_plane_address),
            )
            .with_details(format!("Talos config identity: {}", access.config_identity)),
        ),
        SourceState::Unavailable(info) => checks.push(DiagnosticCheck::unknown(
            "kubernetes_api",
            CheckCategory::Kubernetes,
            "Kubernetes API",
            info,
        )),
    }
    checks.push(match &pod_health {
        SourceState::Available(health) => DiagnosticCheck::warn(
            "pod_health",
            CheckCategory::Kubernetes,
            "Pod Health",
            health.summary(),
        )
        .with_details("Crashing pods:\n  payments/billing-worker-6f7c9d8b5-x2k4p (14 restarts)"),
        SourceState::Unavailable(info) => {
            DiagnosticCheck::unknown("pod_health", CheckCategory::Kubernetes, "Pod Health", info)
        }
    });

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
    checks.push(DiagnosticCheck::pass(
        "br_netfilter",
        CheckCategory::Cni,
        "br_netfilter",
        "Loaded",
    ));
    checks.push(match &cni_pods {
        SourceState::Available(info) => DiagnosticCheck::pass(
            "flannel_pods",
            CheckCategory::Cni,
            "Flannel Pods",
            info.pod_health_summary(),
        ),
        SourceState::Unavailable(info) => {
            DiagnosticCheck::unknown("flannel_pods", CheckCategory::Cni, "Flannel Pods", info)
        }
    });
    checks.push(DiagnosticCheck::pass(
        "cni",
        CheckCategory::Cni,
        "CNI (Flannel)",
        "OK",
    ));
    let cni = CniSnapshot {
        cni_type: SourceState::Available(CniType::Flannel),
        pods: cni_pods,
        files: CniFileEvidence {
            flannel_subnet: FileProbe::Present,
            flannel_subnet_valid: Some(true),
            br_netfilter: FileProbe::Present,
            ..Default::default()
        },
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
    };
    if kubernetes_ok {
        checks.push(DiagnosticCheck::pass(
            "addons",
            CheckCategory::Addons,
            "Addon Discovery",
            addons.detected_names().join(", "),
        ));
        checks.push(DiagnosticCheck::pass(
            "cert_manager_pods",
            CheckCategory::Addons,
            "cert-manager Pods",
            "3/3 healthy",
        ));
        checks.push(DiagnosticCheck::pass(
            "cert_manager_webhook",
            CheckCategory::Addons,
            "cert-manager Webhook",
            "Ready",
        ));
    } else {
        checks.push(DiagnosticCheck::unknown(
            "addons",
            CheckCategory::Addons,
            "Addon Discovery",
            &SourceUnavailable {
                source: "Kubernetes addon discovery".into(),
                reason: "One or more CRD or namespace sources are unavailable".into(),
            },
        ));
        checks.push(DiagnosticCheck::unknown(
            "cert_manager",
            CheckCategory::Addons,
            "cert-manager",
            &SourceUnavailable {
                source: "Kubernetes CRD API".into(),
                reason: "cert-manager presence cannot be determined".into(),
            },
        ));
    }

    Ok(DiagnosticSnapshot {
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
            memory: match memory {
                Some(memory) => SourceState::Available(memory),
                None => SourceState::unavailable("Talos Memory API", "no memory information"),
            },
            load_average: match load {
                Some(load) => SourceState::Available(load),
                None => SourceState::unavailable("Talos LoadAvg API", "no load information"),
            },
        },
        services: ServicesSnapshot {
            services: SourceState::Available(services),
        },
        etcd,
        kubernetes: KubernetesSnapshot { pod_health },
        cni,
        addons,
        checks,
    })
}
