//! The diagnostic harness (#968, safety guarantee 19).
//!
//! `DiagnosticSnapshot::evaluate` is the one place every check is made, so the
//! harness runs it, never a check by name. The snapshots are not typed in
//! whole: what the collector decides without I/O is called here, so a decision
//! that turns an error into a verdict is run too. Those are `file_probe` and
//! `cni_type_from_files` (the CNI provider from Talos file reads),
//! `derive_addon_statuses` and `unavailable_addon_pod_sources` (addons) and
//! `service_health_source` (service health). What the collector reads from a
//! live API stays out of reach.
//!
//! - **source down:** every source of the snapshot unavailable, for each CNI
//!   provider and node role, must give only unknown checks and never panic;
//!   each source down alone, over a healthy snapshot, may change a check only
//!   to unknown; and Talos file reads that fail with "not found" text are run
//!   as their own scenario;
//! - **healthy:** a healthy snapshot for each provider and node role gives no
//!   failing check.
//!
//! A new check made by an existing builder is run by both cases with no edit
//! here. The snapshots are built with struct literals that name every field, so
//! a new source field stops this file compiling until it is given both a
//! healthy and a down value. The id scan below finds a check id that no
//! scenario makes; it does not prove every branch of a check is run.
// The reads return the collector's own `TalosError`, as `file_probe` takes it.
#![allow(clippy::result_large_err)]

use super::*;
use crate::constants::CERT_MANAGER_CRDS;
use crate::diagnostics::{CniInfo, CniPodInfo, CniType, PodHealthInfo};
use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use talos_rs::TalosError;
use tonic::Status;

/// A check that fails a scenario today, and the issue that tracks it. The
/// scenario's name must contain `scenario`; the check is skipped only there.
/// An entry that no longer fails is itself a failure, so a fix removes it.
struct Gap {
    id: &'static str,
    scenario: &'static str,
    issue: &'static str,
}

const KNOWN_GAPS: &[Gap] = &[Gap {
    id: "cni",
    scenario: "Talos file reads say not found",
    // `is_missing_file_error` calls any error whose text holds "not found" a
    // missing file, so four such errors name the provider `None` and `cni`
    // fails with Talos unreached.
    issue: "#1005",
}];

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

fn access() -> SourceState<KubernetesAccess> {
    SourceState::Available(KubernetesAccess {
        control_plane_address: "10.0.0.10".to_string(),
        config_identity: "lab-admin".to_string(),
        source: None,
        warning: None,
    })
}

/// What Talos says when the file really is not there.
fn no_such_file() -> TalosError {
    TalosError::Grpc(Status::not_found("file not found"))
}

/// An error that is not the file's absence, yet whose text holds "not found",
/// as `is_missing_file_error` reads it.
fn not_found_text() -> TalosError {
    TalosError::Grpc(Status::unavailable("peer unavailable, not found in cache"))
}

/// The evidence the collector builds from the five reads, made with its own
/// `file_probe` and `apply_flannel_reads`. Every read is made here; the
/// collector skips those a found provider makes needless, which never changes
/// the provider `cni_type_from_files` names.
fn evidence_from_reads(read: impl Fn(&str) -> Result<String, TalosError>) -> CniFileEvidence {
    let mut evidence = CniFileEvidence::default();
    apply_flannel_reads(
        &mut evidence,
        file_probe(FLANNEL_SUBNET_PATH, read(FLANNEL_SUBNET_PATH)),
        file_probe(BR_NETFILTER_PATH, read(BR_NETFILTER_PATH)),
    );
    evidence.cilium_config = file_probe(CILIUM_CONFIG_PATH, read(CILIUM_CONFIG_PATH)).0;
    evidence.calico_config = file_probe(CALICO_CONFIG_PATH, read(CALICO_CONFIG_PATH)).0;
    let (directory, content) = file_probe(CNI_DIRECTORY_PATH, read(CNI_DIRECTORY_PATH));
    evidence.cni_directory = directory;
    evidence.generic_config_present = content.map(|content| !content.trim().is_empty());
    evidence
}

fn healthy_read(provider: Provider, path: &str) -> Result<String, TalosError> {
    let present = |wanted: Provider| provider == wanted;
    match path {
        FLANNEL_SUBNET_PATH if present(Provider::Flannel) => {
            Ok("FLANNEL_NETWORK=10.244.0.0/16\nFLANNEL_SUBNET=10.244.1.1/24\n".to_string())
        }
        BR_NETFILTER_PATH => Ok("1\n".to_string()),
        CILIUM_CONFIG_PATH if present(Provider::Cilium) => Ok("{}".to_string()),
        CALICO_CONFIG_PATH if present(Provider::Calico) => Ok("{}".to_string()),
        CNI_DIRECTORY_PATH if present(Provider::Unrecognised) => Ok("10-custom.conf\n".to_string()),
        _ => Err(no_such_file()),
    }
}

fn service(id: &str, health: SourceState<ServiceHealthSnapshot>) -> ServiceSnapshot {
    ServiceSnapshot {
        node: "node-1".to_string(),
        id: id.to_string(),
        state: "Running".to_string(),
        health,
    }
}

fn healthy_service_health() -> SourceState<ServiceHealthSnapshot> {
    service_health_source(Some(talos_rs::ServiceHealth {
        unknown: false,
        healthy: true,
        last_message: String::new(),
    }))
}

/// A source's pods, read from the namespaces the collector asks about.
fn healthy_pod_sources(access: &SourceState<KubernetesAccess>) -> Vec<AddonPodSource> {
    unavailable_addon_pod_sources(access)
        .into_iter()
        .map(|source| {
            let pods = if source.namespace == "kube-system" {
                vec!["coredns-5d78c9869d-x7k2p".to_string()]
            } else {
                Vec::new()
            };
            AddonPodSource {
                namespace: source.namespace,
                pods: SourceState::Available(pods),
            }
        })
        .collect()
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
    let prefix = match provider {
        Provider::Flannel => "kube-flannel-ds",
        Provider::Cilium => "cilium",
        Provider::Calico => "calico-node",
        Provider::Unrecognised => "cni",
    };
    let ready_deployment = DeploymentEvidence::Found {
        replicas: 2,
        ready: 2,
        available: 2,
        unavailable: 0,
    };
    // As `collect_cni` does: the provider and the flannel files it reads, then
    // the same decision from all the files.
    let files = evidence_from_reads(|path| healthy_read(provider, path));
    let cni_type = match provider {
        Provider::Flannel => SourceState::Available(CniType::Flannel),
        Provider::Cilium => SourceState::Available(CniType::Cilium),
        Provider::Calico => SourceState::Available(CniType::Calico),
        Provider::Unrecognised => cni_type_from_files(&files),
    };
    let access = access();
    let crd_names = SourceState::Available(
        CERT_MANAGER_CRDS
            .iter()
            .map(|crd| crd.to_string())
            .collect::<Vec<_>>(),
    );
    let pod_sources = healthy_pod_sources(&access);
    let addons = derive_addon_statuses(&crd_names, &pod_sources);
    DiagnosticSnapshot {
        context: DiagnosticContext {
            target: target(control_plane),
            platform: SourceState::Available("metal".to_string()),
            cpu_count: SourceState::Available(4),
            kubernetes_access: access,
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
                    .map(|id| service(id, healthy_service_health()))
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
            cni_type,
            pods: SourceState::Available(cni_pods(prefix)),
            files,
            cilium: (provider == Provider::Cilium).then(|| CiliumEvidence {
                operator: ready_deployment.clone(),
                hubble_relay: ready_deployment,
            }),
        },
        addons: AddonSnapshot {
            crd_names,
            pod_sources,
            addons,
            cert_manager: Some(SourceState::Available(cert_manager_pods())),
        },
        checks: Vec::new(),
    }
}

/// Every source of the snapshot down: a Talos that answers nothing and no
/// Kubernetes client. Struct literals name every field, so a new one does not
/// compile until it is given its down value here. `reads` is what each Talos
/// file read says; the provider the files name is what `collect_cni` would
/// find. A pod-detected provider is kept, though its pods are down, so that
/// provider's checks run with everything they read down: `collect_cni` gets
/// it from the pods alone, and these scenarios have no pods.
fn all_down(
    provider: Provider,
    control_plane: bool,
    reads: fn() -> TalosError,
) -> DiagnosticSnapshot {
    let access = gone::<KubernetesAccess>("Kubernetes kubeconfig");
    let files = evidence_from_reads(|_| Err(reads()));
    let cni_type = match provider {
        Provider::Flannel => SourceState::Available(CniType::Flannel),
        Provider::Cilium => SourceState::Available(CniType::Cilium),
        Provider::Calico => SourceState::Available(CniType::Calico),
        Provider::Unrecognised => cni_type_from_files(&files),
    };
    let crd_names = gone("Kubernetes CRD API");
    let pod_sources = unavailable_addon_pod_sources(&access);
    let addons = derive_addon_statuses(&crd_names, &pod_sources);
    DiagnosticSnapshot {
        context: DiagnosticContext {
            target: target(control_plane),
            platform: gone("Talos Version API"),
            cpu_count: gone("Talos CPUInfo API"),
            kubernetes_access: access,
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
            cni_type,
            pods: gone("Kubernetes CNI pod API"),
            files,
            cilium: (provider == Provider::Cilium).then(|| CiliumEvidence {
                operator: DeploymentEvidence::Unavailable(down("Kubernetes Cilium operator API")),
                hubble_relay: DeploymentEvidence::Unavailable(down("Kubernetes Hubble API")),
            }),
        },
        addons: AddonSnapshot {
            crd_names,
            pod_sources,
            addons,
            cert_manager: None,
        },
        checks: Vec::new(),
    }
}

fn refused() -> TalosError {
    TalosError::Connection("connection refused".to_string())
}

/// The down scenarios: every provider and role with Talos refusing the
/// connection, and the one whose file reads fail with "not found" text.
fn down_scenarios() -> Vec<(String, DiagnosticSnapshot)> {
    let mut scenarios = Vec::new();
    for provider in PROVIDERS {
        for control_plane in [true, false] {
            scenarios.push((
                format!("{provider:?} down, control plane {control_plane}"),
                all_down(provider, control_plane, refused),
            ));
        }
    }
    scenarios.push((
        "Unrecognised down, Talos file reads say not found".to_string(),
        all_down(Provider::Unrecognised, true, not_found_text),
    ));
    scenarios
}

/// Runs `evaluate`, turning a panic into a failure that says which scenario.
fn run(scenario: &str, snapshot: &DiagnosticSnapshot) -> Vec<DiagnosticCheck> {
    catch_unwind(AssertUnwindSafe(|| snapshot.evaluate()))
        .unwrap_or_else(|_| panic!("{scenario}: evaluate panicked"))
}

fn is_known_gap(scenario: &str, id: &str) -> bool {
    KNOWN_GAPS
        .iter()
        .any(|gap| gap.id == id && scenario.contains(gap.scenario))
}

fn statuses(checks: &[DiagnosticCheck]) -> Vec<(String, CheckStatus)> {
    checks
        .iter()
        .map(|check| (check.id.clone(), check.status.clone()))
        .collect()
}

#[test]
fn safety_g19_every_check_with_its_sources_down_says_unknown_and_never_fails() {
    for (scenario, snapshot) in down_scenarios() {
        let checks = run(&scenario, &snapshot);
        assert!(!checks.is_empty(), "{scenario}: no check was made");
        for check in &checks {
            if is_known_gap(&scenario, &check.id) {
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

#[test]
fn safety_g19_every_check_on_a_healthy_fixture_reports_no_failure() {
    for provider in PROVIDERS {
        for control_plane in [true, false] {
            let scenario = format!("{provider:?} healthy, control plane {control_plane}");
            let checks = run(&scenario, &healthy(provider, control_plane));
            assert!(!checks.is_empty(), "{scenario}: no check was made");
            for check in &checks {
                if is_known_gap(&scenario, &check.id) {
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
/// to unknown, and no check may begin to fail. The entries are a second list,
/// kept by hand, of single sources; the all-down case is what finds one left
/// out.
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
                // As the collector turns an omitted health field.
                service.health = service_health_source(None);
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
    ("crds", |s| {
        s.addons.crd_names = gone("Kubernetes CRD API");
        s.addons.addons = derive_addon_statuses(&s.addons.crd_names, &s.addons.pod_sources);
    }),
    ("addon pods", |s| {
        s.addons.pod_sources = unavailable_addon_pod_sources(&s.context.kubernetes_access);
        s.addons.addons = derive_addon_statuses(&s.addons.crd_names, &s.addons.pod_sources);
    }),
    ("cert-manager pods", |s| {
        s.addons.cert_manager = Some(gone("Kubernetes cert-manager pod API"))
    }),
];

#[test]
fn safety_g19_each_source_down_alone_changes_a_check_only_to_unknown() {
    for provider in PROVIDERS {
        let before = statuses(&run("healthy base", &healthy(provider, true)));
        for (name, mutate) in ONE_DOWN {
            let scenario = format!("{provider:?} with {name} down");
            let mut snapshot = healthy(provider, true);
            mutate(&mut snapshot);
            let after = run(&scenario, &snapshot);
            for check in &after {
                if is_known_gap(&scenario, &check.id) {
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

/// The check ids written as string literals in `evaluate.rs`: a constructor's
/// first argument, or `cni_pod_check`'s. A helper that takes an id needs its
/// name here, so this is a second list kept by hand; and it shows that an id
/// is reached, not that each branch under it is.
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
fn safety_g19_every_check_id_in_the_builders_is_produced_by_the_harness() {
    let mut produced = BTreeSet::new();
    for provider in PROVIDERS {
        for control_plane in [true, false] {
            produced.extend(
                healthy(provider, control_plane)
                    .evaluate()
                    .into_iter()
                    .map(|check| check.id),
            );
        }
    }
    for (_, snapshot) in down_scenarios() {
        produced.extend(snapshot.evaluate().into_iter().map(|check| check.id));
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
         in `healthy` or `down_scenarios`, so both cases run them"
    );
}

#[test]
fn safety_g19_a_known_gap_names_an_issue_and_still_fails() {
    for gap in KNOWN_GAPS {
        assert!(
            gap.issue.starts_with('#') && gap.issue[1..].chars().all(|c| c.is_ascii_digit()),
            "known gap {} must name its issue, as `#123`: {:?}",
            gap.id,
            gap.issue
        );
        let reproduced = down_scenarios()
            .iter()
            .filter(|(scenario, _)| scenario.contains(gap.scenario))
            .any(|(scenario, snapshot)| {
                run(scenario, snapshot)
                    .iter()
                    .any(|check| check.id == gap.id && check.status != CheckStatus::Unknown)
            });
        assert!(
            reproduced,
            "known gap {} ({}) no longer fails in {:?}; remove it from KNOWN_GAPS",
            gap.id, gap.issue, gap.scenario
        );
    }
}
