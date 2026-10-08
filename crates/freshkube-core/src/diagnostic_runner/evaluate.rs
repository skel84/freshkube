//! Checks derived from the collected snapshots.
use super::*;

pub(super) fn build_system_checks(
    context: &DiagnosticContext,
    system: &SystemSnapshot,
) -> Vec<DiagnosticCheck> {
    let memory = match &system.memory {
        SourceState::Available(memory) => {
            let message = format!(
                "{} / {} ({:.0}%)",
                format_bytes(memory.used_bytes),
                format_bytes(memory.total_bytes),
                memory.usage_percent
            );
            if memory.usage_percent > 90.0 {
                DiagnosticCheck::fail("memory", CheckCategory::System, "Memory", message)
            } else if memory.usage_percent > 80.0 {
                DiagnosticCheck::warn("memory", CheckCategory::System, "Memory", message)
            } else {
                DiagnosticCheck::pass("memory", CheckCategory::System, "Memory", message)
            }
        }
        SourceState::Unavailable(unavailable) => {
            DiagnosticCheck::unknown("memory", CheckCategory::System, "Memory", unavailable)
        }
    };

    let load = match (&system.load_average, &context.cpu_count) {
        (SourceState::Available(load), SourceState::Available(cpu_count)) => {
            let message = format!(
                "{:.2} / {:.2} / {:.2}",
                load.one_minute, load.five_minutes, load.fifteen_minutes
            );
            let threshold = *cpu_count as f64 * 1.5;
            if load.one_minute > threshold {
                DiagnosticCheck::warn("cpu_load", CheckCategory::System, "CPU Load", message)
                    .with_details(format!(
                        "Load exceeds threshold ({threshold:.1} for {cpu_count} CPUs)"
                    ))
            } else {
                DiagnosticCheck::pass("cpu_load", CheckCategory::System, "CPU Load", message)
            }
        }
        (SourceState::Unavailable(unavailable), _) => {
            DiagnosticCheck::unknown("cpu_load", CheckCategory::System, "CPU Load", unavailable)
        }
        (_, SourceState::Unavailable(unavailable)) => {
            DiagnosticCheck::unknown("cpu_load", CheckCategory::System, "CPU Load", unavailable)
        }
    };

    vec![memory, load]
}

pub(super) fn build_service_checks(services: &ServicesSnapshot) -> Vec<DiagnosticCheck> {
    match &services.services {
        SourceState::Unavailable(unavailable) => vec![DiagnosticCheck::unknown(
            "services",
            CheckCategory::Services,
            "Services",
            unavailable,
        )],
        SourceState::Available(services) if services.is_empty() => vec![DiagnosticCheck::unknown(
            "services",
            CheckCategory::Services,
            "Services",
            &SourceUnavailable {
                source: "Talos ServiceList API".to_string(),
                reason: "Talos returned no services".to_string(),
            },
        )],
        SourceState::Available(services) => services
            .iter()
            .map(|service| match &service.health {
                SourceState::Available(health) if health.healthy => {
                    let message = format!("{} (healthy)", service.state);
                    let check = DiagnosticCheck::pass(
                        format!("service_{}", service.id),
                        CheckCategory::Services,
                        service.id.clone(),
                        message,
                    );
                    if health.last_message.is_empty() {
                        check
                    } else {
                        check.with_details(health.last_message.clone())
                    }
                }
                SourceState::Available(health) => {
                    let message = format!("{} (unhealthy)", service.state);
                    let check = DiagnosticCheck::fail(
                        format!("service_{}", service.id),
                        CheckCategory::Services,
                        service.id.clone(),
                        message,
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
                SourceState::Unavailable(unavailable) => DiagnosticCheck::unknown(
                    format!("service_{}", service.id),
                    CheckCategory::Services,
                    service.id.clone(),
                    unavailable,
                )
                .with_details(format!(
                    "{}\nReported state: {}",
                    unavailable.details(),
                    service.state
                )),
            })
            .collect(),
    }
}

pub(super) fn build_etcd_checks(etcd: &EtcdSnapshot) -> Vec<DiagnosticCheck> {
    match etcd {
        EtcdSnapshot::NotApplicable => Vec::new(),
        EtcdSnapshot::Unavailable(unavailable) => vec![DiagnosticCheck::unknown(
            "etcd",
            CheckCategory::Kubernetes,
            "Etcd",
            unavailable,
        )],
        EtcdSnapshot::Available(status) if !status.errors.is_empty() => vec![
            DiagnosticCheck::fail(
                "etcd",
                CheckCategory::Kubernetes,
                "Etcd",
                "Member reports errors",
            )
            .with_details(status.errors.join("\n")),
        ],
        EtcdSnapshot::Available(status) => {
            let role = if status.is_leader {
                "Leader, healthy".to_string()
            } else {
                format!("Follower (leader: {:x})", status.leader_id)
            };
            let details = format!(
                "Database: {} total, {} in use\nRaft: term {}, index {}, applied {}{}",
                format_bytes_signed(status.db_size_bytes),
                format_bytes_signed(status.db_size_in_use_bytes),
                status.raft_term,
                status.raft_index,
                status.raft_applied_index,
                if status.is_learner {
                    "\nMember is a learner"
                } else {
                    ""
                },
            );
            vec![
                DiagnosticCheck::pass("etcd", CheckCategory::Kubernetes, "Etcd", role)
                    .with_details(details),
            ]
        }
    }
}

pub(super) fn build_kubernetes_checks(
    context: &DiagnosticContext,
    kubernetes: &KubernetesSnapshot,
) -> Vec<DiagnosticCheck> {
    let access = match &context.kubernetes_access {
        SourceState::Available(access) => {
            let summary = access.source.clone().unwrap_or_else(|| {
                format!("Pinned kubeconfig from {}", access.control_plane_address)
            });
            let mut details = format!("Talos config identity: {}", access.config_identity);
            if let Some(warning) = &access.warning {
                details.push('\n');
                details.push_str(warning);
            }
            DiagnosticCheck::pass(
                "kubernetes_api",
                CheckCategory::Kubernetes,
                "Kubernetes API",
                summary,
            )
            .with_details(details)
        }
        SourceState::Unavailable(unavailable) => DiagnosticCheck::unknown(
            "kubernetes_api",
            CheckCategory::Kubernetes,
            "Kubernetes API",
            unavailable,
        ),
    };

    let pod_health = match &kubernetes.pod_health {
        SourceState::Available(health) if health.has_issues() => {
            let mut details = String::new();
            if !health.crashing.is_empty() {
                details.push_str("Crashing pods:\n");
                for pod in &health.crashing {
                    details.push_str(&format!(
                        "  {}/{} ({} restarts)\n",
                        pod.namespace, pod.name, pod.restart_count
                    ));
                }
            }
            if !health.image_pull_errors.is_empty() {
                if !details.is_empty() {
                    details.push('\n');
                }
                details.push_str("Image pull errors:\n");
                for pod in &health.image_pull_errors {
                    details.push_str(&format!("  {}/{}\n", pod.namespace, pod.name));
                }
            }
            DiagnosticCheck::warn(
                "pod_health",
                CheckCategory::Kubernetes,
                "Pod Health",
                health.summary(),
            )
            .with_details(details.trim_end())
        }
        SourceState::Available(health) => DiagnosticCheck::pass(
            "pod_health",
            CheckCategory::Kubernetes,
            "Pod Health",
            format!("{} pods", health.total_pods),
        ),
        SourceState::Unavailable(unavailable) => DiagnosticCheck::unknown(
            "pod_health",
            CheckCategory::Kubernetes,
            "Pod Health",
            unavailable,
        ),
    };

    vec![access, pod_health]
}

pub(super) fn build_cni_checks(
    context: &DiagnosticContext,
    cni: &CniSnapshot,
) -> Vec<DiagnosticCheck> {
    match &cni.cni_type {
        SourceState::Unavailable(unavailable) => vec![DiagnosticCheck::unknown(
            "cni",
            CheckCategory::Cni,
            "CNI",
            unavailable,
        )],
        SourceState::Available(CniType::Flannel) => build_flannel_checks(context, cni),
        SourceState::Available(CniType::Cilium) => build_cilium_checks(cni),
        SourceState::Available(CniType::Calico) => build_calico_checks(cni),
        SourceState::Available(CniType::None) => vec![DiagnosticCheck::fail(
            "cni",
            CheckCategory::Cni,
            "CNI",
            "No CNI configuration found",
        )],
        SourceState::Available(CniType::Unknown) => vec![DiagnosticCheck::pass(
            "cni",
            CheckCategory::Cni,
            "CNI",
            "Configuration present (provider unknown)",
        )],
    }
}

fn build_flannel_checks(context: &DiagnosticContext, cni: &CniSnapshot) -> Vec<DiagnosticCheck> {
    let br_netfilter = match &cni.files.br_netfilter {
        FileProbe::Present => {
            DiagnosticCheck::pass("br_netfilter", CheckCategory::Cni, "br_netfilter", "Loaded")
        }
        FileProbe::Missing => {
            let (message, details) = if is_container_platform(&context.platform) {
                (
                    "Missing (load on host)",
                    "The br_netfilter kernel module must be loaded on the Docker host, then the Talos container restarted.",
                )
            } else {
                (
                    "Missing",
                    "The br_netfilter kernel module is required for Flannel networking.",
                )
            };
            DiagnosticCheck::fail("br_netfilter", CheckCategory::Cni, "br_netfilter", message)
                .with_details(details)
                .with_fix(br_netfilter_fix(context))
        }
        FileProbe::Unavailable(unavailable) => DiagnosticCheck::unknown(
            "br_netfilter",
            CheckCategory::Cni,
            "br_netfilter",
            unavailable,
        ),
        FileProbe::NotChecked => DiagnosticCheck::unknown(
            "br_netfilter",
            CheckCategory::Cni,
            "br_netfilter",
            &SourceUnavailable {
                source: "Talos br_netfilter state".to_string(),
                reason: "The probe was not performed".to_string(),
            },
        ),
    };

    let mut checks = vec![
        br_netfilter,
        cni_pod_check("flannel_pods", "Flannel Pods", &cni.pods),
    ];
    let cni_state = match &cni.files.flannel_subnet {
        FileProbe::Present if cni.files.flannel_subnet_valid == Some(true) => {
            DiagnosticCheck::pass("cni", CheckCategory::Cni, "CNI (Flannel)", "OK")
        }
        FileProbe::Present => {
            flannel_not_initialized_check(context, "Flannel subnet file is incomplete")
        }
        FileProbe::Missing => {
            flannel_not_initialized_check(context, "The /run/flannel/subnet.env file is missing")
        }
        FileProbe::Unavailable(unavailable) => {
            DiagnosticCheck::unknown("cni", CheckCategory::Cni, "CNI (Flannel)", unavailable)
        }
        FileProbe::NotChecked => DiagnosticCheck::unknown(
            "cni",
            CheckCategory::Cni,
            "CNI (Flannel)",
            &SourceUnavailable {
                source: "Talos Flannel state file".to_string(),
                reason: "The probe was not performed".to_string(),
            },
        ),
    };
    checks.push(cni_state);
    checks
}

fn cni_pod_check(id: &str, name: &str, pods: &SourceState<CniInfo>) -> DiagnosticCheck {
    match pods {
        SourceState::Unavailable(unavailable) => {
            DiagnosticCheck::unknown(id, CheckCategory::Cni, name, unavailable)
        }
        SourceState::Available(info) if info.pods.is_empty() => {
            DiagnosticCheck::warn(id, CheckCategory::Cni, name, "No pods found")
                .with_details("Could not find CNI pods in kube-system namespace.")
        }
        SourceState::Available(info) if info.are_pods_healthy() => {
            DiagnosticCheck::pass(id, CheckCategory::Cni, name, info.pod_health_summary())
        }
        SourceState::Available(info) => {
            let details = info
                .pods
                .iter()
                .filter(|pod| pod.phase != "Running" || !pod.ready)
                .map(|pod| format!("  {} - {} (ready: {})", pod.name, pod.phase, pod.ready))
                .collect::<Vec<_>>()
                .join("\n");
            DiagnosticCheck::fail(id, CheckCategory::Cni, name, info.pod_health_summary())
                .with_details(format!("Unhealthy pods:\n{details}"))
        }
    }
}

fn is_container_platform(platform: &SourceState<String>) -> bool {
    matches!(platform, SourceState::Available(platform) if platform == "container")
}

fn br_netfilter_fix(context: &DiagnosticContext) -> DiagnosticFix {
    if is_container_platform(&context.platform) {
        DiagnosticFix {
            description: "Load br_netfilter on Docker host".to_string(),
            action: DiagnosticFixAction::CopyGuidance {
                title: "Load br_netfilter on Docker host".to_string(),
                text: format!(
                    "sudo modprobe br_netfilter && docker restart {}",
                    context.target.node_name
                ),
            },
        }
    } else {
        DiagnosticFix::kernel_module("br_netfilter")
    }
}

fn flannel_not_initialized_check(context: &DiagnosticContext, evidence: &str) -> DiagnosticCheck {
    let mut check = DiagnosticCheck::fail(
        "cni",
        CheckCategory::Cni,
        "CNI (Flannel)",
        "Flannel not initialized",
    )
    .with_details(format!(
        "{evidence}. Check Flannel pod state from Kubernetes before changing configuration."
    ));

    if is_container_platform(&context.platform) {
        check = check.with_fix(DiagnosticFix {
            description: "Check Flannel pod status on Docker host".to_string(),
            action: DiagnosticFixAction::CopyGuidance {
                title: "Check Flannel pod status".to_string(),
                text: format!(
                    "kubectl get pods -n kube-flannel && docker restart {}",
                    context.target.node_name
                ),
            },
        });
    }

    check
}

fn build_cilium_checks(cni: &CniSnapshot) -> Vec<DiagnosticCheck> {
    let not_collected = |source: &str| {
        DeploymentEvidence::Unavailable(SourceUnavailable {
            source: source.to_string(),
            reason: "Cilium component state was not collected".to_string(),
        })
    };
    let (operator, hubble_relay) = match &cni.cilium {
        Some(cilium) => (cilium.operator.clone(), cilium.hubble_relay.clone()),
        None => (
            not_collected("Kubernetes Cilium operator API"),
            not_collected("Kubernetes Hubble Relay API"),
        ),
    };
    let mut checks = vec![cilium_agents_check(&cni.pods)];
    checks.push(cilium_operator_check(&operator));
    if let Some(hubble) = hubble_relay_check(&hubble_relay) {
        checks.push(hubble);
    }

    let status = if checks.iter().any(|check| check.status == CheckStatus::Fail) {
        DiagnosticCheck::fail(
            "cni",
            CheckCategory::Cni,
            "CNI (Cilium)",
            "Some components unhealthy",
        )
    } else if checks.iter().any(|check| check.status == CheckStatus::Warn) {
        DiagnosticCheck::warn(
            "cni",
            CheckCategory::Cni,
            "CNI (Cilium)",
            "Minor issues detected",
        )
    } else if checks
        .iter()
        .any(|check| check.status == CheckStatus::Unknown)
    {
        DiagnosticCheck::unknown(
            "cni",
            CheckCategory::Cni,
            "CNI (Cilium)",
            &SourceUnavailable {
                source: "Cilium component state".to_string(),
                reason: "One or more required component sources are unavailable".to_string(),
            },
        )
    } else {
        DiagnosticCheck::pass("cni", CheckCategory::Cni, "CNI (Cilium)", "OK")
    };
    checks.push(status);
    checks
}

fn cilium_agents_check(pods: &SourceState<CniInfo>) -> DiagnosticCheck {
    let info = match pods {
        SourceState::Available(info) => info,
        SourceState::Unavailable(unavailable) => {
            return DiagnosticCheck::unknown(
                "cilium_agents",
                CheckCategory::Cni,
                "Cilium Agents",
                unavailable,
            );
        }
    };

    let agents = info
        .pods
        .iter()
        .filter(|pod| {
            let name = pod.name.to_ascii_lowercase();
            name.starts_with("cilium-")
                && !name.contains("operator")
                && !name.contains("hubble")
                && !name.contains("envoy")
        })
        .collect::<Vec<_>>();
    if agents.is_empty() {
        return DiagnosticCheck::warn(
            "cilium_agents",
            CheckCategory::Cni,
            "Cilium Agents",
            "No agent pods found",
        )
        .with_details("Expected DaemonSet pods matching cilium-* in kube-system.");
    }

    let unhealthy = agents
        .iter()
        .filter(|pod| pod.phase != "Running" || !pod.ready)
        .collect::<Vec<_>>();
    let total_restarts: i32 = agents.iter().map(|pod| pod.restart_count).sum();
    if unhealthy.is_empty() {
        let message = if total_restarts > 0 {
            format!(
                "{}/{} agents ready ({} restarts)",
                agents.len(),
                agents.len(),
                total_restarts
            )
        } else {
            format!("{}/{} agents ready", agents.len(), agents.len())
        };
        let details = agents
            .iter()
            .map(|pod| {
                format!(
                    "  Ready on {}{}",
                    pod.node_name.as_deref().unwrap_or("unknown"),
                    if pod.restart_count > 0 {
                        format!(" ({} restarts)", pod.restart_count)
                    } else {
                        String::new()
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        DiagnosticCheck::pass(
            "cilium_agents",
            CheckCategory::Cni,
            "Cilium Agents",
            message,
        )
        .with_details(format!("Per-node status:\n{details}"))
    } else {
        let details = unhealthy
            .iter()
            .map(|pod| {
                format!(
                    "  {} on {} - {} (ready: {})",
                    pod.name,
                    pod.node_name.as_deref().unwrap_or("unknown"),
                    pod.phase,
                    pod.ready
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        DiagnosticCheck::fail(
            "cilium_agents",
            CheckCategory::Cni,
            "Cilium Agents",
            format!(
                "{}/{} agents ready",
                agents.len() - unhealthy.len(),
                agents.len()
            ),
        )
        .with_details(format!("Unhealthy agents:\n{details}"))
    }
}

fn cilium_operator_check(operator: &DeploymentEvidence) -> DiagnosticCheck {
    let (replicas, ready, available, unavailable) = match operator {
        DeploymentEvidence::Found {
            replicas,
            ready,
            available,
            unavailable,
        } => (*replicas, *ready, *available, *unavailable),
        DeploymentEvidence::Unavailable(unavailable) => {
            return DiagnosticCheck::unknown(
                "cilium_operator",
                CheckCategory::Cni,
                "Cilium Operator",
                unavailable,
            );
        }
        DeploymentEvidence::NotFound => {
            return DiagnosticCheck::unknown(
                "cilium_operator",
                CheckCategory::Cni,
                "Cilium Operator",
                &SourceUnavailable {
                    source: "Kubernetes Cilium operator deployment".to_string(),
                    reason: "No supported Cilium operator deployment was found".to_string(),
                },
            );
        }
    };

    if ready > 0 && ready == replicas {
        return DiagnosticCheck::pass(
            "cilium_operator",
            CheckCategory::Cni,
            "Cilium Operator",
            format!("{ready}/{replicas} ready"),
        );
    }
    if available >= 1 && unavailable > 0 {
        return DiagnosticCheck::pass(
            "cilium_operator",
            CheckCategory::Cni,
            "Cilium Operator",
            format!("{ready}/{replicas} ready (HA limited)"),
        )
        .with_details(format!(
            "{unavailable} replica(s) pending, likely due to pod anti-affinity; {available} replica(s) are available."
        ));
    }
    if available > 0 {
        return DiagnosticCheck::warn(
            "cilium_operator",
            CheckCategory::Cni,
            "Cilium Operator",
            format!("{ready}/{replicas} ready ({available} available)"),
        );
    }
    DiagnosticCheck::fail(
        "cilium_operator",
        CheckCategory::Cni,
        "Cilium Operator",
        format!("{ready}/{replicas} ready"),
    )
    .with_details(
        "Cilium Operator is not ready; IP allocation and CiliumNetworkPolicy may be affected.",
    )
}

fn hubble_relay_check(relay: &DeploymentEvidence) -> Option<DiagnosticCheck> {
    match relay {
        DeploymentEvidence::Found {
            replicas, ready, ..
        } => {
            let (replicas, ready) = (*replicas, *ready);
            if ready > 0 && ready == replicas {
                Some(DiagnosticCheck::pass(
                    "hubble_relay",
                    CheckCategory::Cni,
                    "Hubble Relay",
                    format!("{ready}/{replicas} ready"),
                ))
            } else {
                Some(
                    DiagnosticCheck::fail(
                        "hubble_relay",
                        CheckCategory::Cni,
                        "Hubble Relay",
                        format!("{ready}/{replicas} ready"),
                    )
                    .with_details(
                        "Hubble Relay is not ready; network flow observability is unavailable.",
                    ),
                )
            }
        }
        DeploymentEvidence::NotFound => None,
        DeploymentEvidence::Unavailable(unavailable) => Some(DiagnosticCheck::unknown(
            "hubble_relay",
            CheckCategory::Cni,
            "Hubble Relay",
            unavailable,
        )),
    }
}

fn build_calico_checks(cni: &CniSnapshot) -> Vec<DiagnosticCheck> {
    let pod_check = cni_pod_check("calico_pods", "Calico Pods", &cni.pods);
    let overall = match &pod_check.status {
        CheckStatus::Fail => DiagnosticCheck::fail(
            "cni",
            CheckCategory::Cni,
            "CNI (Calico)",
            "Some pods unhealthy",
        ),
        CheckStatus::Warn => DiagnosticCheck::warn(
            "cni",
            CheckCategory::Cni,
            "CNI (Calico)",
            "Pod state needs attention",
        ),
        CheckStatus::Unknown => DiagnosticCheck::unknown(
            "cni",
            CheckCategory::Cni,
            "CNI (Calico)",
            &SourceUnavailable {
                source: "Kubernetes Calico pod API".to_string(),
                reason: "Calico pod state is unavailable".to_string(),
            },
        ),
        CheckStatus::Pass | CheckStatus::Checking => {
            DiagnosticCheck::pass("cni", CheckCategory::Cni, "CNI (Calico)", "OK")
        }
    };
    vec![pod_check, overall]
}

pub(super) fn build_addon_checks(addons: &AddonSnapshot) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();
    if addons.has_unknown_sources() {
        checks.push(DiagnosticCheck::unknown(
            "addons",
            CheckCategory::Addons,
            "Addon Discovery",
            &SourceUnavailable {
                source: "Kubernetes addon discovery".to_string(),
                reason: "One or more CRD or namespace sources are unavailable".to_string(),
            },
        ));
    } else {
        let names = addons.detected_names();
        let message = if names.is_empty() {
            "No supported addons detected".to_string()
        } else {
            names.join(", ")
        };
        checks.push(DiagnosticCheck::pass(
            "addons",
            CheckCategory::Addons,
            "Addon Discovery",
            message,
        ));
    }

    match (addons.presence("cert-manager"), &addons.cert_manager) {
        (Some(AddonPresence::Detected), Some(pods)) => {
            checks.extend(cert_manager_checks(pods));
        }
        (Some(AddonPresence::Detected), None) => checks.push(DiagnosticCheck::unknown(
            "cert_manager",
            CheckCategory::Addons,
            "cert-manager",
            &SourceUnavailable {
                source: "Kubernetes cert-manager API".to_string(),
                reason: "Kubernetes access is unavailable".to_string(),
            },
        )),
        (Some(AddonPresence::Unknown), _) => checks.push(DiagnosticCheck::unknown(
            "cert_manager",
            CheckCategory::Addons,
            "cert-manager",
            &SourceUnavailable {
                source: "Kubernetes CRD API".to_string(),
                reason: "cert-manager presence cannot be determined".to_string(),
            },
        )),
        _ => {}
    }

    checks
}

fn cert_manager_checks(pods: &SourceState<Vec<AddonPod>>) -> Vec<DiagnosticCheck> {
    match pods {
        SourceState::Available(pods) => {
            let pod_check = cert_manager_pod_check(pods);
            let webhook_check = cert_manager_webhook_check(pods);
            vec![pod_check, webhook_check]
        }
        SourceState::Unavailable(unavailable) => {
            vec![
                DiagnosticCheck::unknown(
                    "cert_manager_pods",
                    CheckCategory::Addons,
                    "cert-manager Pods",
                    unavailable,
                ),
                DiagnosticCheck::unknown(
                    "cert_manager_webhook",
                    CheckCategory::Addons,
                    "cert-manager Webhook",
                    unavailable,
                ),
            ]
        }
    }
}

fn cert_manager_pod_check(pods: &[AddonPod]) -> DiagnosticCheck {
    if pods.is_empty() {
        return DiagnosticCheck::fail(
            "cert_manager_pods",
            CheckCategory::Addons,
            "cert-manager Pods",
            "No pods found",
        )
        .with_details("cert-manager was detected by CRDs but no pods are running.");
    }

    let healthy = pods.iter().filter(|pod| pod.ready).count();
    if healthy == pods.len() {
        DiagnosticCheck::pass(
            "cert_manager_pods",
            CheckCategory::Addons,
            "cert-manager Pods",
            format!("{healthy}/{} healthy", pods.len()),
        )
    } else {
        let details = pods
            .iter()
            .filter(|pod| !pod.ready)
            .map(|pod| {
                let phase = pod.phase.as_deref().unwrap_or("Unknown");
                format!("{}: {phase}", pod.name.as_deref().unwrap_or("unknown"))
            })
            .collect::<Vec<_>>()
            .join("\n");
        DiagnosticCheck::warn(
            "cert_manager_pods",
            CheckCategory::Addons,
            "cert-manager Pods",
            format!("{healthy}/{} healthy", pods.len()),
        )
        .with_details(format!("Unhealthy pods:\n{details}"))
    }
}

fn cert_manager_webhook_check(pods: &[AddonPod]) -> DiagnosticCheck {
    let webhook_pods = pods
        .iter()
        .filter(|pod| {
            pod.name
                .as_deref()
                .is_some_and(|name| name.contains("webhook"))
        })
        .collect::<Vec<_>>();
    if webhook_pods.is_empty() {
        return DiagnosticCheck::warn(
            "cert_manager_webhook",
            CheckCategory::Addons,
            "cert-manager Webhook",
            "Not found",
        )
        .with_details("Webhook pod not found. Certificate validation may not work.");
    }
    if webhook_pods.iter().any(|pod| pod.ready) {
        DiagnosticCheck::pass(
            "cert_manager_webhook",
            CheckCategory::Addons,
            "cert-manager Webhook",
            "Ready",
        )
    } else {
        DiagnosticCheck::warn(
            "cert_manager_webhook",
            CheckCategory::Addons,
            "cert-manager Webhook",
            "Not ready",
        )
        .with_details(
            "Webhook pod exists but is not ready. New certificates may fail to be issued.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{PodCondition, PodStatus};
    use kube::core::ErrorResponse;

    /// A healthy Cilium node with cert-manager detected, built by hand: the
    /// evaluator sees only this evidence.
    fn snapshot(
        cilium: Option<CiliumEvidence>,
        cert_manager: Option<SourceState<Vec<AddonPod>>>,
    ) -> DiagnosticSnapshot {
        DiagnosticSnapshot {
            context: DiagnosticContext {
                target: DiagnosticTarget::new("cp-1", "10.0.0.10", "controlplane", "lab-admin"),
                platform: SourceState::Available("metal".to_string()),
                cpu_count: SourceState::Available(4),
                kubernetes_access: SourceState::Available(KubernetesAccess {
                    control_plane_address: "10.0.0.10".to_string(),
                    config_identity: "lab-admin".to_string(),
                    source: None,
                    warning: None,
                }),
            },
            system: SystemSnapshot {
                memory: SourceState::unavailable("Talos Memory API", "not read"),
                load_average: SourceState::unavailable("Talos LoadAvg API", "not read"),
            },
            services: ServicesSnapshot {
                services: SourceState::Available(Vec::new()),
            },
            etcd: EtcdSnapshot::NotApplicable,
            kubernetes: KubernetesSnapshot {
                pod_health: SourceState::unavailable("Kubernetes Pod API", "not read"),
            },
            cni: CniSnapshot {
                cni_type: SourceState::Available(CniType::Cilium),
                pods: SourceState::Available(CniInfo {
                    cni_type: CniType::Cilium,
                    pods: vec![CniPodInfo {
                        name: "cilium-x7k2p".to_string(),
                        node_name: Some("cp-1".to_string()),
                        phase: "Running".to_string(),
                        ready: true,
                        restart_count: 0,
                    }],
                }),
                files: CniFileEvidence::default(),
                cilium,
            },
            addons: AddonSnapshot {
                crd_names: SourceState::Available(vec!["certificates.cert-manager.io".to_string()]),
                pod_sources: Vec::new(),
                addons: vec![AddonStatus {
                    id: "cert-manager",
                    name: "cert-manager",
                    presence: AddonPresence::Detected,
                }],
                cert_manager,
            },
            checks: Vec::new(),
        }
    }

    fn deployment(
        replicas: i32,
        ready: i32,
        available: i32,
        unavailable: i32,
    ) -> DeploymentEvidence {
        DeploymentEvidence::Found {
            replicas,
            ready,
            available,
            unavailable,
        }
    }

    fn cilium(operator: DeploymentEvidence, hubble_relay: DeploymentEvidence) -> CiliumEvidence {
        CiliumEvidence {
            operator,
            hubble_relay,
        }
    }

    fn pod(name: &str, ready: bool) -> AddonPod {
        AddonPod {
            name: Some(name.to_string()),
            phase: Some(if ready { "Running" } else { "Pending" }.to_string()),
            ready,
        }
    }

    fn healthy_cert_manager() -> SourceState<Vec<AddonPod>> {
        SourceState::Available(vec![
            pod("cert-manager-5c9d8c7b4-q8m2z", true),
            pod("cert-manager-webhook-7d6f9b8c5-k4n7p", true),
        ])
    }

    fn check<'a>(checks: &'a [DiagnosticCheck], id: &str) -> Option<&'a DiagnosticCheck> {
        checks.iter().find(|check| check.id == id)
    }

    /// The status and message of the check with this id, from the evaluator.
    fn outcome(snapshot: &DiagnosticSnapshot, id: &str) -> (CheckStatus, String) {
        let checks = snapshot.evaluate();
        let check = check(&checks, id).unwrap_or_else(|| panic!("no {id} check"));
        (check.status.clone(), check.message.clone())
    }

    fn api_error(code: u16, reason: &str) -> kube::Error {
        kube::Error::Api(ErrorResponse {
            status: "Failure".to_string(),
            message: format!("{reason} for this request"),
            reason: reason.to_string(),
            code,
        })
    }

    fn refused() -> kube::Error {
        api_error(403, "Forbidden")
    }

    fn failed() -> kube::Error {
        kube::Error::Service("connection refused".into())
    }

    #[test]
    fn the_cilium_operator_passes_warns_and_fails_on_its_replicas() {
        let operator = |evidence| {
            outcome(
                &snapshot(
                    Some(cilium(evidence, DeploymentEvidence::NotFound)),
                    Some(healthy_cert_manager()),
                ),
                "cilium_operator",
            )
        };

        assert_eq!(
            operator(deployment(2, 2, 2, 0)),
            (CheckStatus::Pass, "2/2 ready".to_string())
        );
        assert_eq!(
            operator(deployment(2, 1, 1, 1)),
            (CheckStatus::Pass, "1/2 ready (HA limited)".to_string())
        );
        assert_eq!(
            operator(deployment(2, 0, 1, 0)),
            (CheckStatus::Warn, "0/2 ready (1 available)".to_string())
        );
        assert_eq!(
            operator(deployment(2, 0, 0, 2)),
            (CheckStatus::Fail, "0/2 ready".to_string())
        );
        // No replicas at all is not ready, never a pass.
        assert_eq!(operator(deployment(0, 0, 0, 0)).0, CheckStatus::Fail);
    }

    #[test]
    fn cilium_rolls_up_its_components_worst_first() {
        let overall = |operator, relay| {
            outcome(
                &snapshot(Some(cilium(operator, relay)), Some(healthy_cert_manager())),
                "cni",
            )
            .0
        };
        let healthy = || deployment(1, 1, 1, 0);

        assert_eq!(overall(healthy(), healthy()), CheckStatus::Pass);
        assert_eq!(
            overall(healthy(), deployment(1, 0, 0, 1)),
            CheckStatus::Fail
        );
        assert_eq!(
            overall(deployment(2, 0, 1, 0), healthy()),
            CheckStatus::Warn
        );
        assert_eq!(
            overall(DeploymentEvidence::NotFound, healthy()),
            CheckStatus::Unknown
        );
        // A failing component outranks an unknown one.
        assert_eq!(
            overall(DeploymentEvidence::NotFound, deployment(1, 0, 0, 1)),
            CheckStatus::Fail
        );
    }

    #[test]
    fn hubble_relay_passes_or_fails_and_is_left_out_when_not_installed() {
        let relay = |evidence| {
            snapshot(
                Some(cilium(deployment(1, 1, 1, 0), evidence)),
                Some(healthy_cert_manager()),
            )
        };

        assert_eq!(
            outcome(&relay(deployment(1, 1, 1, 0)), "hubble_relay"),
            (CheckStatus::Pass, "1/1 ready".to_string())
        );
        assert_eq!(
            outcome(&relay(deployment(1, 0, 0, 1)), "hubble_relay"),
            (CheckStatus::Fail, "0/1 ready".to_string())
        );
        let checks = relay(DeploymentEvidence::NotFound).evaluate();
        assert!(check(&checks, "hubble_relay").is_none());
        assert_eq!(check(&checks, "cni").unwrap().status, CheckStatus::Pass);
    }

    #[test]
    fn cert_manager_pods_and_webhook_pass_warn_and_fail() {
        let checks =
            |pods: Vec<AddonPod>| snapshot(None, Some(SourceState::Available(pods))).evaluate();
        let status = |checks: &[DiagnosticCheck], id| check(checks, id).unwrap().status.clone();

        let healthy = checks(vec![
            pod("cert-manager-5c9d8c7b4-q8m2z", true),
            pod("cert-manager-webhook-7d6f9b8c5-k4n7p", true),
        ]);
        assert_eq!(status(&healthy, "cert_manager_pods"), CheckStatus::Pass);
        assert_eq!(
            check(&healthy, "cert_manager_pods").unwrap().message,
            "2/2 healthy"
        );
        assert_eq!(status(&healthy, "cert_manager_webhook"), CheckStatus::Pass);

        let webhook_down = checks(vec![
            pod("cert-manager-5c9d8c7b4-q8m2z", true),
            pod("cert-manager-webhook-7d6f9b8c5-k4n7p", false),
        ]);
        let pods = check(&webhook_down, "cert_manager_pods").unwrap();
        assert_eq!(
            (pods.status.clone(), pods.message.as_str()),
            (CheckStatus::Warn, "1/2 healthy")
        );
        assert_eq!(
            pods.details.as_deref(),
            Some("Unhealthy pods:\ncert-manager-webhook-7d6f9b8c5-k4n7p: Pending")
        );
        let webhook = check(&webhook_down, "cert_manager_webhook").unwrap();
        assert_eq!(
            (webhook.status.clone(), webhook.message.as_str()),
            (CheckStatus::Warn, "Not ready")
        );

        let no_webhook = checks(vec![pod("cert-manager-5c9d8c7b4-q8m2z", true)]);
        let webhook = check(&no_webhook, "cert_manager_webhook").unwrap();
        assert_eq!(
            (webhook.status.clone(), webhook.message.as_str()),
            (CheckStatus::Warn, "Not found")
        );

        let none = checks(Vec::new());
        assert_eq!(status(&none, "cert_manager_pods"), CheckStatus::Fail);
    }

    #[test]
    fn missing_evidence_is_unknown_never_a_failure() {
        // No Kubernetes client: collection records why, from the access state.
        let no_client = |source: &str| {
            DeploymentEvidence::Unavailable(SourceUnavailable {
                source: source.to_string(),
                reason: "Kubeconfig unavailable".to_string(),
            })
        };
        let checks = snapshot(
            Some(cilium(
                no_client("Kubernetes Cilium operator API"),
                no_client("Kubernetes Hubble Relay API"),
            )),
            None,
        )
        .evaluate();
        for id in ["cilium_operator", "hubble_relay", "cni", "cert_manager"] {
            assert_eq!(
                check(&checks, id).unwrap().status,
                CheckStatus::Unknown,
                "{id}"
            );
        }
        // Detected without a pod read is one unknown, not two.
        assert!(check(&checks, "cert_manager_pods").is_none());

        // Neither operator Deployment exists.
        assert_eq!(
            outcome(
                &snapshot(
                    Some(cilium(
                        DeploymentEvidence::NotFound,
                        DeploymentEvidence::NotFound
                    )),
                    Some(healthy_cert_manager()),
                ),
                "cilium_operator",
            )
            .0,
            CheckStatus::Unknown
        );

        // A Cilium snapshot whose component state was never collected.
        let checks = snapshot(None, Some(healthy_cert_manager())).evaluate();
        assert_eq!(
            check(&checks, "cilium_operator").unwrap().status,
            CheckStatus::Unknown
        );
        assert_eq!(
            check(&checks, "hubble_relay").unwrap().status,
            CheckStatus::Unknown
        );
        assert_eq!(check(&checks, "cni").unwrap().status, CheckStatus::Unknown);
    }

    #[test]
    fn a_refused_deployment_read_is_unknown_with_its_reason() {
        let evidence = deployment_evidence(Err(refused()), "Kubernetes Cilium operator API");
        let DeploymentEvidence::Unavailable(unavailable) = &evidence else {
            panic!("a 403 is unavailable evidence, got {evidence:?}");
        };
        assert_eq!(unavailable.source, "Kubernetes Cilium operator API");
        assert!(
            unavailable.reason.contains("Forbidden"),
            "{}",
            unavailable.reason
        );

        let checks = snapshot(
            Some(cilium(evidence.clone(), evidence)),
            Some(healthy_cert_manager()),
        )
        .evaluate();
        for id in ["cilium_operator", "hubble_relay"] {
            let check = check(&checks, id).unwrap();
            assert_eq!(check.status, CheckStatus::Unknown, "{id}");
            assert!(
                check.details.as_deref().unwrap().contains("Forbidden"),
                "{id}"
            );
        }
    }

    #[test]
    fn a_failed_deployment_read_is_unknown_with_its_reason() {
        let evidence = deployment_evidence(Err(failed()), "Kubernetes Hubble Relay API");
        let DeploymentEvidence::Unavailable(unavailable) = &evidence else {
            panic!("a failed read is unavailable evidence, got {evidence:?}");
        };
        assert!(
            unavailable.reason.contains("connection refused"),
            "{}",
            unavailable.reason
        );

        let checks = snapshot(
            Some(cilium(evidence.clone(), evidence)),
            Some(healthy_cert_manager()),
        )
        .evaluate();
        for id in ["cilium_operator", "hubble_relay"] {
            let check = check(&checks, id).unwrap();
            assert_eq!(check.status, CheckStatus::Unknown, "{id}");
            assert!(
                check
                    .details
                    .as_deref()
                    .unwrap()
                    .contains("connection refused"),
                "{id}"
            );
        }
        // Only a 404 means not installed.
        assert_eq!(
            deployment_evidence(
                Err(api_error(404, "NotFound")),
                "Kubernetes Hubble Relay API"
            ),
            DeploymentEvidence::NotFound
        );
    }

    #[test]
    fn a_refused_or_failed_cert_manager_list_is_unknown_for_both_checks() {
        for (error, reason) in [(refused(), "Forbidden"), (failed(), "connection refused")] {
            let pods = cert_manager_pods(Err(error));
            assert!(!pods.is_available());
            let checks = snapshot(None, Some(pods)).evaluate();
            for id in ["cert_manager_pods", "cert_manager_webhook"] {
                let check = check(&checks, id).unwrap();
                assert_eq!(check.status, CheckStatus::Unknown, "{id} on {reason}");
                assert!(check.details.as_deref().unwrap().contains(reason), "{id}");
            }
        }
    }

    #[test]
    fn cert_manager_pods_are_reduced_to_name_phase_and_readiness() {
        let pod = |name: &str, ready: &str| Pod {
            metadata: kube::api::ObjectMeta {
                name: Some(name.to_string()),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some("Running".to_string()),
                conditions: Some(vec![PodCondition {
                    type_: "Ready".to_string(),
                    status: ready.to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let SourceState::Available(pods) = cert_manager_pods(Ok(vec![
            pod("cert-manager-webhook-a", "True"),
            pod("cert-manager-b", "False"),
        ])) else {
            panic!("a listed namespace is available");
        };
        assert_eq!(
            pods.iter().map(|pod| pod.ready).collect::<Vec<_>>(),
            [true, false]
        );
        assert_eq!(pods[1].phase.as_deref(), Some("Running"));
    }

    #[test]
    fn the_next_snapshot_recovers_with_nothing_kept_from_a_failure() {
        let failing = snapshot(
            Some(cilium(deployment(1, 0, 0, 1), deployment(1, 0, 0, 1))),
            Some(SourceState::Available(Vec::new())),
        );
        let unreadable = snapshot(
            Some(cilium(
                deployment_evidence(Err(failed()), "Kubernetes Cilium operator API"),
                deployment_evidence(Err(refused()), "Kubernetes Hubble Relay API"),
            )),
            Some(cert_manager_pods(Err(failed()))),
        );
        let recovered = snapshot(
            Some(cilium(deployment(1, 1, 1, 0), deployment(1, 1, 1, 0))),
            Some(healthy_cert_manager()),
        );

        let ids = [
            "cilium_operator",
            "hubble_relay",
            "cni",
            "cert_manager_pods",
        ];
        let statuses = |snapshot: &DiagnosticSnapshot| ids.map(|id| outcome(snapshot, id).0);
        assert_eq!(statuses(&failing), [(); 4].map(|_| CheckStatus::Fail));
        assert_eq!(statuses(&unreadable), [(); 4].map(|_| CheckStatus::Unknown));
        assert_eq!(statuses(&recovered), [(); 4].map(|_| CheckStatus::Pass));
        // Evaluating is a function of the snapshot alone.
        assert_eq!(statuses(&failing), [(); 4].map(|_| CheckStatus::Fail));
    }
}
