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
