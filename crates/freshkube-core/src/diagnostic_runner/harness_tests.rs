//! The diagnostic harness (#968, safety guarantee 17).
//!
//! `DiagnosticSnapshot::evaluate` is the one place every check is made, so the
//! harness runs it, never a check by name:
//!
//! - **source down:** every source of the snapshot unavailable, for each CNI
//!   provider, must give only unknown checks and never panic; and each source
//!   down alone, over a healthy snapshot, may change a check only to unknown;
//! - **healthy:** a healthy snapshot for each provider and node role gives no
//!   failing check.
//!
//! A new check made by an existing builder is run by both cases with no edit
//! here. The snapshots are built with struct literals that name every field, so
//! a new source field stops this file compiling until it is given both a
//! healthy and a down value. And `every_check_id_in_the_builders_is_produced`
//! fails when a check id written in `evaluate.rs` is made by none of the
//! scenarios, so a check behind a branch the scenarios never reach is found.
use super::*;
use crate::diagnostics::{CniInfo, CniPodInfo, CniType, PodHealthInfo};
use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// A check that fails the harness today: its id, the issue that tracks it, and
/// the scenario it fails in. Empty while every check passes both cases.
const KNOWN_GAPS: &[(&str, &str)] = &[];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Provider {
    Flannel,
    Cilium,
    Calico,
    Unrecognised,
}

const PROVIDERS: [Provider; 4] = [
    Provider::Flannel,
    Provider::Cilium,
    Provider::Calico,
    Provider::Unrecognised,
];

fn down(source: &str) -> SourceUnavailable {
    SourceUnavailable {
        source: source.to_string(),
        reason: "harness: the source does not answer".to_string(),
    }
}

fn gone<T>(source: &str) -> SourceState<T> {
    SourceState::Unavailable(down(source))
}

fn cni_pods(prefix: &str) -> CniInfo {
    CniInfo {
        cni_type: CniType::Unknown,
        pods: (1..=3)
            .map(|n| CniPodInfo {
                name: format!("{prefix}-{n}x7k"),
                node_name: Some(format!("node-{n}")),
                phase: "Running".to_string(),
                ready: true,
                restart_count: 0,
            })
            .collect(),
    }
}

fn target(control_plane: bool) -> DiagnosticTarget {
    let role = if control_plane {
        "controlplane"
    } else {
        "worker"
    };
    DiagnosticTarget::new("node-1", "10.0.0.11", role, "lab-admin")
}

fn cert_manager_pods() -> Vec<AddonPod> {
    ["cert-manager-5c9d", "cert-manager-webhook-7d6f"]
        .into_iter()
        .map(|name| AddonPod {
            name: Some(name.to_string()),
            phase: Some("Running".to_string()),
            ready: true,
        })
        .collect()
}

fn healthy(provider: Provider, control_plane: bool) -> DiagnosticSnapshot {
    let (cni_type, prefix) = match provider {
        Provider::Flannel => (CniType::Flannel, "kube-flannel-ds"),
        Provider::Cilium => (CniType::Cilium, "cilium"),
        Provider::Calico => (CniType::Calico, "calico-node"),
        Provider::Unrecognised => (CniType::Unknown, "cni"),
    };
    let ready_deployment = DeploymentEvidence::Found {
        replicas: 2,
        ready: 2,
        available: 2,
        unavailable: 0,
    };
    DiagnosticSnapshot {
        context: DiagnosticContext {
            target: target(control_plane),
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
            memory: SourceState::Available(MemorySnapshot {
                total_bytes: 8 << 30,
                available_bytes: 6 << 30,
                used_bytes: 2 << 30,
                usage_percent: 25.0,
            }),
            load_average: SourceState::Available(LoadAverageSnapshot {
                one_minute: 0.4,
                five_minutes: 0.3,
                fifteen_minutes: 0.2,
            }),
        },
        services: ServicesSnapshot {
            services: SourceState::Available(
                ["apid", "kubelet", "etcd"]
                    .into_iter()
                    .map(|id| ServiceSnapshot {
                        node: "node-1".to_string(),
                        id: id.to_string(),
                        state: "Running".to_string(),
                        health: SourceState::Available(ServiceHealthSnapshot {
                            healthy: true,
                            last_message: String::new(),
                        }),
                    })
                    .collect(),
            ),
        },
        etcd: if control_plane {
            EtcdSnapshot::Available(EtcdStatusSnapshot {
                node: "node-1".to_string(),
                member_id: 1,
                leader_id: 1,
                is_leader: true,
                protocol_version: "3.6.0".to_string(),
                db_size_bytes: 40 << 20,
                db_size_in_use_bytes: 18 << 20,
                raft_index: 900,
                raft_term: 7,
                raft_applied_index: 900,
                errors: Vec::new(),
                is_learner: false,
            })
        } else {
            EtcdSnapshot::NotApplicable
        },
        kubernetes: KubernetesSnapshot {
            pod_health: SourceState::Available(PodHealthInfo {
                crashing: Vec::new(),
                image_pull_errors: Vec::new(),
                not_ready: Vec::new(),
                total_pods: 42,
            }),
        },
        cni: CniSnapshot {
            cni_type: SourceState::Available(cni_type),
            pods: SourceState::Available(cni_pods(prefix)),
            files: CniFileEvidence {
                flannel_subnet: FileProbe::Present,
                flannel_subnet_valid: Some(true),
                cilium_config: FileProbe::NotChecked,
                calico_config: FileProbe::NotChecked,
                cni_directory: FileProbe::NotChecked,
                generic_config_present: None,
                br_netfilter: FileProbe::Present,
            },
            cilium: (provider == Provider::Cilium).then(|| CiliumEvidence {
                operator: ready_deployment.clone(),
                hubble_relay: ready_deployment,
            }),
        },
        addons: AddonSnapshot {
            crd_names: SourceState::Available(vec!["certificates.cert-manager.io".to_string()]),
            pod_sources: vec![AddonPodSource {
                namespace: "cert-manager".to_string(),
                pods: SourceState::Available(vec!["cert-manager-5c9d".to_string()]),
            }],
            addons: vec![AddonStatus {
                id: "cert-manager",
                name: "cert-manager",
                presence: AddonPresence::Detected,
            }],
            cert_manager: Some(SourceState::Available(cert_manager_pods())),
        },
        checks: Vec::new(),
    }
}

/// Every source of the snapshot down. Struct literals name every field, so a
/// new one does not compile until it is given its down value here.
fn all_down(provider: Provider, control_plane: bool) -> DiagnosticSnapshot {
    // `Unknown` is an answer (configuration present, provider not recognised),
    // so a down snapshot has no provider at all for it.
    let cni_type = match provider {
        Provider::Flannel => SourceState::Available(CniType::Flannel),
        Provider::Cilium => SourceState::Available(CniType::Cilium),
        Provider::Calico => SourceState::Available(CniType::Calico),
        Provider::Unrecognised => gone("Kubernetes CNI API"),
    };
    DiagnosticSnapshot {
        context: DiagnosticContext {
            target: target(control_plane),
            platform: gone("Talos Version API"),
            cpu_count: gone("Talos CPUInfo API"),
            kubernetes_access: gone("Kubernetes kubeconfig"),
        },
        system: SystemSnapshot {
            memory: gone("Talos Memory API"),
            load_average: gone("Talos LoadAvg API"),
        },
        services: ServicesSnapshot {
            services: gone("Talos ServiceList API"),
        },
        etcd: if control_plane {
            EtcdSnapshot::Unavailable(down("Talos etcd API"))
        } else {
            EtcdSnapshot::NotApplicable
        },
        kubernetes: KubernetesSnapshot {
            pod_health: gone("Kubernetes Pod API"),
        },
        cni: CniSnapshot {
            // A known provider is kept, as a snapshot keeps it from an earlier
            // read; what is down is everything its checks read.
            cni_type,
            pods: gone("Kubernetes CNI pod API"),
            files: CniFileEvidence {
                flannel_subnet: FileProbe::Unavailable(down("Talos flannel state")),
                flannel_subnet_valid: None,
                cilium_config: FileProbe::Unavailable(down("Talos cilium state")),
                calico_config: FileProbe::Unavailable(down("Talos calico state")),
                cni_directory: FileProbe::Unavailable(down("Talos CNI directory")),
                generic_config_present: None,
                br_netfilter: FileProbe::Unavailable(down("Talos br_netfilter state")),
            },
            cilium: (provider == Provider::Cilium).then(|| CiliumEvidence {
                operator: DeploymentEvidence::Unavailable(down("Kubernetes Cilium operator API")),
                hubble_relay: DeploymentEvidence::Unavailable(down("Kubernetes Hubble API")),
            }),
        },
        addons: AddonSnapshot {
            crd_names: gone("Kubernetes CRD API"),
            pod_sources: vec![AddonPodSource {
                namespace: "cert-manager".to_string(),
                pods: gone("Kubernetes cert-manager pod API"),
            }],
            addons: vec![AddonStatus {
                id: "cert-manager",
                name: "cert-manager",
                presence: AddonPresence::Unknown,
            }],
            cert_manager: None,
        },
        checks: Vec::new(),
    }
}

/// Runs `evaluate`, turning a panic into a failure that says which scenario.
fn run(scenario: &str, snapshot: &DiagnosticSnapshot) -> Vec<DiagnosticCheck> {
    catch_unwind(AssertUnwindSafe(|| snapshot.evaluate()))
        .unwrap_or_else(|_| panic!("{scenario}: evaluate panicked"))
}

fn is_known_gap(id: &str) -> bool {
    KNOWN_GAPS.iter().any(|(gap, _)| *gap == id)
}

fn statuses(checks: &[DiagnosticCheck]) -> Vec<(String, CheckStatus)> {
    checks
        .iter()
        .map(|check| (check.id.clone(), check.status.clone()))
        .collect()
}

#[test]
fn safety_g17_every_check_with_its_sources_down_says_unknown_and_never_fails() {
    for provider in PROVIDERS {
        for control_plane in [true, false] {
            let scenario = format!("{provider:?} down, control plane {control_plane}");
            let checks = run(&scenario, &all_down(provider, control_plane));
            assert!(!checks.is_empty(), "{scenario}: no check was made");
            for check in &checks {
                if is_known_gap(&check.id) {
                    continue;
                }
                assert_eq!(
                    check.status,
                    CheckStatus::Unknown,
                    "{scenario}: {} said {:?} ({}) with its source down",
                    check.id,
                    check.status,
                    check.message
                );
                assert!(
                    check.fix.is_none(),
                    "{scenario}: {} offers a fix with its source down",
                    check.id
                );
            }
        }
    }
}

#[test]
fn safety_g17_every_check_on_a_healthy_fixture_reports_no_failure() {
    for provider in PROVIDERS {
        for control_plane in [true, false] {
            let scenario = format!("{provider:?} healthy, control plane {control_plane}");
            let checks = run(&scenario, &healthy(provider, control_plane));
            assert!(!checks.is_empty(), "{scenario}: no check was made");
            for check in &checks {
                if is_known_gap(&check.id) {
                    continue;
                }
                assert!(
                    !matches!(check.status, CheckStatus::Fail),
                    "{scenario}: {} failed on a healthy fixture ({})",
                    check.id,
                    check.message
                );
                assert!(
                    check.fix.is_none() || check.status != CheckStatus::Pass,
                    "{scenario}: passing {} offers a fix",
                    check.id
                );
            }
        }
    }
}

/// One source down over a healthy snapshot: a check it feeds may change only
/// to unknown, and no check may begin to fail. Each entry only names a source;
/// the all-down test above is what proves nothing was left out.
type Mutation = (&'static str, fn(&mut DiagnosticSnapshot));

const ONE_DOWN: &[Mutation] = &[
    ("platform", |s| {
        s.context.platform = gone("Talos Version API")
    }),
    ("cpu count", |s| {
        s.context.cpu_count = gone("Talos CPUInfo API")
    }),
    ("kubernetes access", |s| {
        s.context.kubernetes_access = gone("Kubernetes kubeconfig")
    }),
    ("memory", |s| s.system.memory = gone("Talos Memory API")),
    ("load", |s| {
        s.system.load_average = gone("Talos LoadAvg API")
    }),
    ("services", |s| {
        s.services.services = gone("Talos ServiceList API")
    }),
    ("service health", |s| {
        if let SourceState::Available(services) = &mut s.services.services {
            for service in services {
                service.health = gone("Talos service health");
            }
        }
    }),
    ("etcd", |s| {
        if !matches!(s.etcd, EtcdSnapshot::NotApplicable) {
            s.etcd = EtcdSnapshot::Unavailable(down("Talos etcd API"));
        }
    }),
    ("pod health", |s| {
        s.kubernetes.pod_health = gone("Kubernetes Pod API")
    }),
    ("cni type", |s| s.cni.cni_type = gone("Kubernetes CNI API")),
    ("cni pods", |s| s.cni.pods = gone("Kubernetes CNI pod API")),
    ("flannel file", |s| {
        s.cni.files.flannel_subnet = FileProbe::Unavailable(down("Talos flannel state"))
    }),
    ("br_netfilter", |s| {
        s.cni.files.br_netfilter = FileProbe::Unavailable(down("Talos br_netfilter state"))
    }),
    ("cilium operator", |s| {
        if let Some(cilium) = &mut s.cni.cilium {
            cilium.operator = DeploymentEvidence::Unavailable(down("Cilium operator"));
        }
    }),
    ("hubble relay", |s| {
        if let Some(cilium) = &mut s.cni.cilium {
            cilium.hubble_relay = DeploymentEvidence::Unavailable(down("Hubble Relay"));
        }
    }),
    ("crds", |s| s.addons.crd_names = gone("Kubernetes CRD API")),
    ("addon pods", |s| {
        for source in &mut s.addons.pod_sources {
            source.pods = gone("Kubernetes addon pod API");
        }
    }),
    ("cert-manager pods", |s| {
        s.addons.cert_manager = Some(gone("Kubernetes cert-manager pod API"))
    }),
];

#[test]
fn safety_g17_each_source_down_alone_changes_a_check_only_to_unknown() {
    for provider in PROVIDERS {
        let base = healthy(provider, true);
        let before = statuses(&run("healthy base", &base));
        for (name, mutate) in ONE_DOWN {
            let scenario = format!("{provider:?} with {name} down");
            let mut snapshot = healthy(provider, true);
            mutate(&mut snapshot);
            let after = run(&scenario, &snapshot);
            for check in &after {
                if is_known_gap(&check.id) {
                    continue;
                }
                let was = before
                    .iter()
                    .find(|(id, _)| *id == check.id)
                    .map(|(_, s)| s);
                assert!(
                    check.status != CheckStatus::Fail,
                    "{scenario}: {} fails",
                    check.id
                );
                if let Some(was) = was
                    && *was != check.status
                {
                    assert_eq!(
                        check.status,
                        CheckStatus::Unknown,
                        "{scenario}: {} moved from {was:?} to {:?}, not to unknown",
                        check.id,
                        check.status
                    );
                }
            }
        }
    }
}

/// The ids a builder writes as a string literal, found in `evaluate.rs`
/// itself: a constructor's first argument, or a helper's (`cni_pod_check`).
fn literal_check_ids() -> BTreeSet<String> {
    let source = include_str!("evaluate.rs");
    let source = source.split("#[cfg(test)]").next().unwrap_or(source);
    let mut ids = BTreeSet::new();
    for marker in [
        "DiagnosticCheck::pass(",
        "DiagnosticCheck::warn(",
        "DiagnosticCheck::fail(",
        "DiagnosticCheck::unknown(",
        "cni_pod_check(",
    ] {
        for part in source.split(marker).skip(1) {
            let rest = part.trim_start();
            if let Some(rest) = rest.strip_prefix('"')
                && let Some(end) = rest.find('"')
            {
                ids.insert(rest[..end].to_string());
            }
        }
    }
    ids
}

#[test]
fn safety_g17_every_check_id_in_the_builders_is_produced_by_the_harness() {
    let mut produced = BTreeSet::new();
    for provider in PROVIDERS {
        for control_plane in [true, false] {
            for snapshot in [
                healthy(provider, control_plane),
                all_down(provider, control_plane),
            ] {
                produced.extend(snapshot.evaluate().into_iter().map(|check| check.id));
            }
        }
    }
    let literals = literal_check_ids();
    assert!(literals.len() > 10, "the id scan found {literals:?}");
    let missing: Vec<_> = literals
        .iter()
        .filter(|id| !produced.contains(*id))
        .collect();
    assert!(
        missing.is_empty(),
        "check ids no harness scenario makes: {missing:?}. Add a scenario that reaches them \
         in `healthy` or `all_down`, so both cases run them"
    );
}

#[test]
fn safety_g17_a_known_gap_names_an_issue() {
    for (id, issue) in KNOWN_GAPS {
        assert!(
            issue.starts_with('#') && issue[1..].chars().all(|c| c.is_ascii_digit()),
            "known gap {id} must name its issue, as `#123`: {issue:?}"
        );
    }
}
