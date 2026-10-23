use super::*;

/// A refused Node list reads as it did before the read carried a kind:
/// the sentence plus the kube error's text.
#[tokio::test]
async fn a_refused_node_list_keeps_its_sentence_and_gains_a_kind() {
    use kube::client::Body;
    let service = tower::service_fn(|_: http::Request<Body>| async {
        Ok::<_, std::convert::Infallible>(
            http::Response::builder()
                .status(403)
                .header("content-type", "application/json")
                .body(Body::from(
                    br#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"nodes is forbidden","reason":"Forbidden","code":403}"#.to_vec(),
                ))
                .unwrap(),
        )
    });
    let client = Client::new(service, "default");
    let expected = Api::<Node>::all(client.clone())
        .list(&ListParams::default())
        .await
        .unwrap_err()
        .to_string();
    let failure = collect_kubernetes_node_evidence(client).await.unwrap_err();
    assert_eq!(failure.kind, FailureKind::Forbidden);
    assert_eq!(
        failure.message,
        format!("Kubernetes Node API request failed: {expected}")
    );
}

fn disk(path: &str, readonly: bool, cdrom: bool) -> DiskInfo {
    DiskInfo {
        id: path.trim_start_matches("/dev/").to_string(),
        dev_path: path.to_string(),
        size: 1024,
        size_pretty: "1 KB".to_string(),
        model: None,
        serial: None,
        transport: None,
        rotational: false,
        readonly,
        cdrom,
        wwid: None,
        bus_path: None,
    }
}

fn endpoint() -> MaintenanceEndpoint {
    MaintenanceEndpoint::parse("https://192.0.2.10:50000").unwrap()
}

fn plan() -> BootstrapPlan {
    let endpoint = endpoint();
    BootstrapPlan::new(
        endpoint.clone(),
        "test-cluster",
        endpoint.kubernetes_api_url(),
        PathBuf::from("generated"),
        MachineRole::ControlPlane,
        BootstrapPaths {
            talosconfig_path: PathBuf::from("generated/talosconfig"),
            kubeconfig_path: None,
        },
        AuthenticatedTarget::new("test-cluster", endpoint, Some("cp-1".to_string())).unwrap(),
    )
    .unwrap()
}

fn snapshot() -> InsecureMaintenanceSnapshot {
    InsecureMaintenanceSnapshot {
        endpoint: endpoint(),
        version: SourceAvailability::Unavailable {
            reason: "maintenance version API unavailable".to_string(),
        },
        disks: vec![disk("/dev/sda", false, false)],
        volumes: SourceAvailability::Unavailable {
            reason: "volume status unavailable".to_string(),
        },
    }
}

fn talos_ready() -> TalosReadinessEvidence {
    TalosReadinessEvidence::Available {
        versions: vec![TalosVersionEvidence {
            node: "cp-1".to_string(),
            version: "v1.10.0".to_string(),
        }],
        etcd: EtcdReadinessEvidence::Ready {
            responding_members: 1,
        },
    }
}

fn cluster_ready_evidence() -> BootstrapReadinessEvidence {
    BootstrapReadinessEvidence {
        target: plan().authenticated_target,
        talos: talos_ready(),
        kubernetes: KubernetesReadinessEvidence::Ready {
            credentials_source: KubernetesCredentialsSource::TalosApi,
            nodes: vec![KubernetesNodeEvidence {
                name: "cp-1".to_string(),
                ready: true,
            }],
        },
    }
}

#[test]
fn normalizes_maintenance_endpoint_without_retaining_port() {
    assert_eq!(endpoint().address(), "192.0.2.10");
    assert_eq!(
        MaintenanceEndpoint::parse("[2001:db8::10]:50000")
            .unwrap()
            .address(),
        "2001:db8::10"
    );
    assert_eq!(
        MaintenanceEndpoint::parse("2001:db8::10")
            .unwrap()
            .kubernetes_api_url(),
        "https://[2001:db8::10]:6443"
    );
}

#[test]
fn rejects_endpoint_paths_and_invalid_ports() {
    assert!(MaintenanceEndpoint::parse("https://node.example/path").is_err());
    assert!(MaintenanceEndpoint::parse("node.example:0").is_err());
    assert!(MaintenanceEndpoint::parse("2001:db8::10:50000").is_err());
}

#[test]
fn install_target_must_be_a_writable_non_optical_talos_disk() {
    let disks = vec![disk("/dev/sda", true, false), disk("/dev/sr0", false, true)];
    assert!(matches!(
        InstallTarget::select(&disks, "/dev/sda"),
        Err(MaintenanceError::InstallTargetNotInstallable { .. })
    ));
    assert!(matches!(
        InstallTarget::select(&disks, "/dev/sr0"),
        Err(MaintenanceError::InstallTargetNotInstallable { .. })
    ));
    assert!(matches!(
        InstallTarget::select(&disks, "/dev/nvme0n1"),
        Err(MaintenanceError::InstallTargetNotFound { .. })
    ));
}

#[test]
fn state_requires_talos_then_kubernetes_api_evidence() {
    let selected = InstallTarget::select(&snapshot().disks, "/dev/sda").unwrap();
    let mut session = BootstrapSession::new(plan());
    session = session
        .reduce(MaintenanceEvent::InsecureDataCollected(snapshot()))
        .unwrap();
    session = session
        .reduce(MaintenanceEvent::InstallTargetSelected(selected.clone()))
        .unwrap();
    assert_eq!(session.phase, BootstrapPhase::Configuring);

    session = session
        .reduce(MaintenanceEvent::ConfigurationGenerationStarted)
        .unwrap();
    let generated = GeneratedConfiguration {
        machine_config_path: PathBuf::from("generated/controlplane.yaml"),
        talosconfig_path: PathBuf::from("generated/talosconfig"),
        role: MachineRole::ControlPlane,
        install_target: selected,
    };
    session = session
        .reduce(MaintenanceEvent::ConfigurationGenerationFinished(
            generated.clone(),
        ))
        .unwrap();
    session = session
        .reduce(MaintenanceEvent::ConfigurationApplicationRequested(
            ConfigurationApplicationRequest::new(
                endpoint(),
                generated.machine_config_path,
                generated.install_target,
            )
            .unwrap(),
        ))
        .unwrap();
    let confirmation = session.pending_confirmation.clone().unwrap();
    session = session
        .reduce(MaintenanceEvent::ConfigurationApplicationConfirmed(
            confirmation,
        ))
        .unwrap();
    session = session
        .reduce(MaintenanceEvent::ConfigurationApplicationFinished(
            ConfigurationApplicationOutcome {
                applied: true,
                message: "accepted".to_string(),
            },
        ))
        .unwrap();

    let unavailable = BootstrapReadinessEvidence {
        target: plan().authenticated_target,
        talos: TalosReadinessEvidence::Unavailable {
            reason: "node rebooting".to_string(),
        },
        kubernetes: KubernetesReadinessEvidence::NotChecked,
    };
    session = session
        .reduce(MaintenanceEvent::ReadinessPolled(unavailable))
        .unwrap();
    assert_eq!(session.phase, BootstrapPhase::WaitingForTalos);

    let talos_only = BootstrapReadinessEvidence {
        target: plan().authenticated_target,
        talos: talos_ready(),
        kubernetes: KubernetesReadinessEvidence::NotChecked,
    };
    session = session
        .reduce(MaintenanceEvent::ReadinessPolled(talos_only))
        .unwrap();
    assert_eq!(session.phase, BootstrapPhase::ReadyToBootstrap);

    session = session.reduce(MaintenanceEvent::BootstrapStarted).unwrap();
    session = session
        .reduce(MaintenanceEvent::BootstrapFinished(
            BootstrapCommandOutcome {
                started: true,
                message: "accepted".to_string(),
            },
        ))
        .unwrap();

    let no_kubernetes_evidence = BootstrapReadinessEvidence {
        target: plan().authenticated_target,
        talos: talos_ready(),
        kubernetes: KubernetesReadinessEvidence::NotChecked,
    };
    session = session
        .reduce(MaintenanceEvent::ReadinessPolled(no_kubernetes_evidence))
        .unwrap();
    assert_eq!(session.phase, BootstrapPhase::WaitingForKubernetes);

    session = session
        .reduce(MaintenanceEvent::ReadinessPolled(cluster_ready_evidence()))
        .unwrap();
    assert_eq!(session.phase, BootstrapPhase::Complete);
}

#[test]
fn generated_configuration_keeps_selected_disk_and_role() {
    let request = ConfigurationGenerationRequest {
        plan: plan(),
        install_target: InstallTarget::select(&snapshot().disks, "/dev/sda").unwrap(),
    };
    let generated = generated_configuration_from_result(
        &request,
        GenConfigResult {
            controlplane_path: "generated/controlplane.yaml".to_string(),
            worker_path: "generated/worker.yaml".to_string(),
            talosconfig_path: "generated/talosconfig".to_string(),
            output_dir: "generated".to_string(),
        },
    );

    assert_eq!(
        generated.machine_config_path,
        PathBuf::from("generated/controlplane.yaml")
    );
    assert_eq!(
        generated.talosconfig_path,
        request.plan.paths.talosconfig_path
    );
    assert_eq!(generated.install_target, request.install_target);
}

fn configuration_ready_session() -> BootstrapSession {
    let session = BootstrapSession::new(plan())
        .reduce(MaintenanceEvent::InsecureDataCollected(snapshot()))
        .unwrap();
    let selected = session.select_install_target("/dev/sda").unwrap();
    let session = session.reduce(selected).unwrap();
    let (session, action) = session.begin_configuration_generation().unwrap();
    let MaintenanceAction::GenerateConfiguration(request) = action else {
        panic!("wrong action")
    };
    assert_eq!(request.install_target.device_path(), "/dev/sda");
    session
        .reduce(MaintenanceEvent::ConfigurationGenerationFinished(
            GeneratedConfiguration {
                machine_config_path: PathBuf::from("generated/controlplane.yaml"),
                talosconfig_path: PathBuf::from("generated/talosconfig"),
                role: MachineRole::ControlPlane,
                install_target: request.install_target,
            },
        ))
        .unwrap()
}

#[rustfmt::skip]
#[test]
fn review_binds_exact_yaml_and_rejects_changed_install_disk_or_role() {
    let session = configuration_ready_session();
    let reviewed = "machine:\n  type: controlplane\n  install:\n    disk: /dev/sda\n";
    assert!(
        session
            .request_reviewed_configuration_application(
                reviewed.replace("/dev/sda", "/dev/sdb"),
            )
            .is_err()
    );
    assert!(
        session
            .request_reviewed_configuration_application(
                reviewed.replace("controlplane", "worker"),
            )
            .is_err()
    );
    let (pending, confirmation) = session
        .request_reviewed_configuration_application(reviewed.into())
        .unwrap();
    assert_eq!(confirmation.reviewed_content.as_deref(), Some(reviewed));
    // An unreviewed or different confirmation cannot satisfy the pending
    // frozen-content confirmation even if endpoint/path/disk match.
    let (_, unreviewed) = session.request_configuration_application().unwrap();
    assert!(
        pending
            .confirm_configuration_application(unreviewed)
            .is_err()
    );
    let (applying, action) = pending
        .confirm_configuration_application(confirmation)
        .unwrap();
    assert_eq!(applying.phase, BootstrapPhase::ApplyingConfiguration);
    assert!(matches!(action, MaintenanceAction::ApplyConfiguration(_)));
}

#[test]
fn reducer_rejects_stale_snapshot_and_authenticated_readiness_identity() {
    let session = BootstrapSession::new(plan());
    let mut stale = snapshot();
    stale.endpoint = MaintenanceEndpoint::parse("192.0.2.11").unwrap();
    assert!(
        session
            .reduce(MaintenanceEvent::InsecureDataCollected(stale))
            .is_err()
    );
    let mut waiting = configuration_ready_session();
    waiting.phase = BootstrapPhase::WaitingForTalos;
    let mut evidence = cluster_ready_evidence();
    evidence.target.node = MaintenanceEndpoint::parse("192.0.2.11").unwrap();
    assert!(
        waiting
            .reduce(MaintenanceEvent::ReadinessPolled(evidence))
            .is_err()
    );
    let mut evidence = cluster_ready_evidence();
    evidence.target.talos_context = "other-context".into();
    assert!(
        waiting
            .reduce(MaintenanceEvent::ReadinessPolled(evidence))
            .is_err()
    );
}

#[test]
fn unknown_health_never_completes_bootstrap_and_cancel_retains_results() {
    let mut waiting = configuration_ready_session();
    waiting.phase = BootstrapPhase::WaitingForKubernetes;
    let evidence = BootstrapReadinessEvidence {
        target: plan().authenticated_target,
        talos: talos_ready(),
        kubernetes: KubernetesReadinessEvidence::Unavailable {
            credentials_source: KubernetesCredentialsSource::SelectedPath("rejected".into()),
            reason: "Selected CA does not match pinned Talos identity".into(),
        },
    };
    let waiting = waiting
        .reduce(MaintenanceEvent::ReadinessPolled(evidence.clone()))
        .unwrap();
    assert_eq!(waiting.phase, BootstrapPhase::WaitingForKubernetes);
    let cancelled = waiting.reduce(MaintenanceEvent::Cancelled).unwrap();
    assert_eq!(cancelled.phase, BootstrapPhase::Cancelled);
    assert_eq!(cancelled.latest_readiness, Some(evidence));
    assert!(cancelled.generated_configuration.is_some());
}

#[tokio::test]
async fn cancellation_prevents_every_maintenance_action_before_io() {
    let session = configuration_ready_session();
    let (_, confirmation) = session
        .request_reviewed_configuration_application(
            "machine:\n  type: controlplane\n  install:\n    disk: /dev/sda\n".into(),
        )
        .unwrap();
    let actions = [
        MaintenanceAction::CollectInsecure {
            endpoint: endpoint(),
        },
        MaintenanceAction::GenerateConfiguration(ConfigurationGenerationRequest {
            plan: session.plan.clone(),
            install_target: session.install_target.clone().unwrap(),
        }),
        MaintenanceAction::ApplyConfiguration(confirmation),
        MaintenanceAction::Bootstrap(BootstrapCommand {
            paths: plan().paths,
            target: plan().authenticated_target,
        }),
        MaintenanceAction::PollReadiness(BootstrapReadinessRequest {
            paths: plan().paths,
            target: plan().authenticated_target,
        }),
    ];
    for action in actions {
        assert!(matches!(
            execute_maintenance_action(action, || true).await,
            MaintenanceEvent::Cancelled
        ));
    }
}

// Public certificate fixture; no private credentials and no live request.
const TEST_CA: &str = "-----BEGIN CERTIFICATE-----\n\
MIIBUTCB2KADAgECAgkAtXGSKEzrTR4wCgYIKoZIzj0EAwIwEDEOMAwGA1UEAwwF\n\
YmVubm8wHhcNMTgxMTEzMDI1NDQwWhcNMTkxMTEzMDI1NDQwWjAQMQ4wDAYDVQQD\n\
DAViZW5ubzB2MBAGByqGSM49AgEGBSuBBAAiA2IABDhrHLVTMHC7GTyB/MNztToW\n\
ss2zlmvR62X1pQaBN6fYhBJE1XYa0V2C1fGGXj92MencOtXyfYVxn+DY07gyT/71\n\
HQ12TJOe90wwjy2/6N1W1jOv5HjphVT8JQlVNqAC+DAKBggqhkjOPQQDAgNoADBl\n\
AjAfzS1tmZ+GSAaXrsPcfAd1A9yfWVtB8tWxFNNo2j7/cL3puf2vQnlwV/0BoZZ1\n\
K4ICMQDaE0HemgYp6RPQzeb96a2gjlaEbwNy1B8O74fE24WRXiOUm4eln9wGQ3I1\n\
iV6NtZU=\n-----END CERTIFICATE-----\n";

fn kubeconfig_fixture(certificate: &str, server: &str, insecure: bool) -> String {
    use base64::Engine;
    let ca = base64::engine::general_purpose::STANDARD.encode(certificate);
    format!(
        r#"apiVersion: v1
kind: Config
current-context: fixture
clusters:
- name: fixture
  cluster:
    server: {server}
    certificate-authority-data: {ca}
    insecure-skip-tls-verify: {insecure}
contexts:
- name: fixture
  context:
    cluster: fixture
    user: never-run
users:
- name: never-run
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: should-never-be-executed-during-identity-validation
"#
    )
}

struct FixtureFile(PathBuf);
impl FixtureFile {
    fn new(content: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "talos-maintenance-source-{}-{}.yaml",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        std::fs::write(&path, content).unwrap();
        Self(path)
    }
}
impl Drop for FixtureFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The readiness client's setup, on its real path: an unusable pinned kubeconfig is
/// Config, and says what it said before.
#[tokio::test]
async fn probe_client_refuses_an_unusable_kubeconfig_as_config() {
    let failure = probe_client(None, "not yaml".into(), "10.0.0.1".into())
        .await
        .err()
        .expect("an unparseable kubeconfig makes no client");
    assert_eq!(failure.kind, FailureKind::Config);
    assert_eq!(
        failure.message,
        "Failed to create K8s client: Authenticated Talos kubeconfig is unavailable or invalid"
    );
}

/// A setup worker that panics is Other, with the worker's sentence.
#[tokio::test]
async fn a_setup_worker_that_fails_is_other() {
    let error = run_roster_setup::<(), _>(|| panic!("worker down"))
        .await
        .expect_err("a panicking worker doesn't join");
    let failure = worker_failure(&error);
    assert_eq!(failure.kind, FailureKind::Other);
    assert_eq!(
        failure.message,
        "Failed to create K8s client: Kubernetes credential setup worker failed"
    );
}

#[rustfmt::skip]
#[test]
fn selected_readiness_requires_pinned_ca_and_tls_before_plugins() {
    let pinned = kubeconfig_fixture(TEST_CA, "https://pinned.invalid:6443", false);
    let selected = FixtureFile::new(&kubeconfig_fixture(
        TEST_CA,
        "https://selected.invalid:6443",
        false,
    ));
    let (config, options) =
        prepare_probe_kubeconfig(Some(selected.0.clone()), &pinned, "192.0.2.10").unwrap();
    assert_eq!(options.context.as_deref(), Some("fixture"));
    assert_eq!(
        config.clusters[0]
            .cluster
            .as_ref()
            .unwrap()
            .server
            .as_deref(),
        Some("https://selected.invalid:6443")
    );
    assert!(
        prepare_probe_kubeconfig(
            Some(selected.0.clone()),
            "invalid pinned config",
            "192.0.2.10"
        )
        .is_err()
    );
    let insecure = FixtureFile::new(&kubeconfig_fixture(
        TEST_CA,
        "https://selected.invalid:6443",
        true,
    ));
    assert!(prepare_probe_kubeconfig(Some(insecure.0.clone()), &pinned, "192.0.2.10").is_err());
    let plain = FixtureFile::new(&kubeconfig_fixture(
        TEST_CA,
        "http://selected.invalid:6443",
        false,
    ));
    assert!(prepare_probe_kubeconfig(Some(plain.0.clone()), &pinned, "192.0.2.10").is_err());
    // Change a certificate signature byte, preserving parseable ASN.1.
    let changed = TEST_CA.replace("iV6NtZU=", "iV6NtZQ=");
    let mismatched = FixtureFile::new(&kubeconfig_fixture(
        &changed,
        "https://other.invalid:6443",
        false,
    ));
    assert!(
        prepare_probe_kubeconfig(Some(mismatched.0.clone()), &pinned, "192.0.2.10").is_err()
    );
}

#[test]
fn authenticated_clients_pin_transport_and_node_not_context_endpoints() {
    let context = talos_rs::Context {
        endpoints: vec!["unrelated.invalid".into()],
        nodes: vec!["another-node.invalid".into()],
        ca: "preserved-ca".into(),
        crt: "preserved-client".into(),
        key: "preserved-key".into(),
    };
    let context = pinned_talos_context(context, &endpoint());
    assert_eq!(context.endpoints, vec!["192.0.2.10"]);
    assert_eq!(context.nodes, vec!["192.0.2.10"]);
    assert_eq!(context.ca, "preserved-ca");
}

fn draft() -> MaintenanceDraft {
    MaintenanceDraft::new("192.0.2.10")
}

#[test]
fn draft_generation_uses_owned_credentials() {
    let plan = draft().plan().unwrap();
    assert_eq!(
        plan.paths.talosconfig_path,
        PathBuf::from("freshkube-generated/talosconfig")
    );
    assert_eq!(plan.authenticated_target.talos_context, "talos-cluster");
    assert_eq!(plan.endpoint, plan.authenticated_target.node);
    assert!(plan.paths.kubeconfig_path.is_none());
}

#[test]
fn draft_rejects_a_different_bootstrap_node_before_io() {
    let mut draft = draft();
    draft.bootstrap_node = "192.0.2.11".into();
    assert!(matches!(
        draft.plan(),
        Err(MaintenanceError::TargetEndpointMismatch { .. })
    ));
}

#[test]
fn draft_context_follows_cluster_name_until_edited() {
    let mut draft = draft();
    draft.set_cluster_name("lab".into());
    assert_eq!(draft.context, "lab");
    draft.context = "custom".into();
    draft.set_cluster_name("other".into());
    assert_eq!(draft.context, "custom");
}

#[test]
fn draft_requires_an_https_kubernetes_api() {
    let mut draft = draft();
    draft.kubernetes_api = "http://192.0.2.10:6443".into();
    assert_eq!(
        draft.plan().unwrap_err(),
        MaintenanceError::InvalidKubernetesEndpoint
    );
}

#[test]
fn endpoint_validation_is_shared() {
    assert!(validate_maintenance_endpoint("192.0.2.10").is_ok());
    assert!(validate_maintenance_endpoint("[2001:db8::10]:50000").is_ok());
    for bad in ["", " ", "https://node.example/path", "node.example:0"] {
        assert!(validate_maintenance_endpoint(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn confirmation_requires_exact_identity_phrase() {
    assert!(confirmation_matches(
        Some("APPLY 192.0.2.10 /dev/sda".into()),
        "APPLY 192.0.2.10 /dev/sda"
    ));
    assert!(!confirmation_matches(
        Some("APPLY 192.0.2.10 /dev/sda".into()),
        "APPLY 192.0.2.11 /dev/sda"
    ));
    assert!(!confirmation_matches(None, ""));
}

#[test]
fn workers_cannot_obtain_a_bootstrap_phrase() {
    let mut draft = draft();
    draft.role = MachineRole::Worker;
    let mut session = BootstrapSession::new(draft.plan().unwrap());
    session.phase = BootstrapPhase::ReadyToBootstrap;
    assert!(session.bootstrap_confirmation_phrase().is_none());
    let mut control = BootstrapSession::new(self::draft().plan().unwrap());
    assert!(control.bootstrap_confirmation_phrase().is_none());
    control.phase = BootstrapPhase::ReadyToBootstrap;
    assert_eq!(
        control.bootstrap_confirmation_phrase().as_deref(),
        Some("BOOTSTRAP talos-cluster 192.0.2.10")
    );
}

#[test]
fn progress_history_is_bounded() {
    let mut log = ProgressLog::default();
    for index in 0..1000 {
        log.record(index.to_string());
    }
    assert_eq!(log.len(), PROGRESS_HISTORY_LIMIT);
    assert_eq!(log.last().unwrap(), "999");
}

#[test]
fn review_is_bounded_and_rejects_non_regular_files() {
    let directory = std::env::temp_dir().join(format!(
        "talos-maintenance-review-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir(&directory).unwrap();
    let file = directory.join("controlplane.yaml");
    std::fs::write(&file, "reviewed bytes").unwrap();
    assert_eq!(frozen_review(&file).unwrap(), "reviewed bytes");
    std::fs::write(&file, vec![b'x'; REVIEW_LIMIT + 1]).unwrap();
    assert!(frozen_review(&file).is_err());
    assert!(frozen_review(&directory).is_err());
    std::fs::remove_file(file).unwrap();
    std::fs::remove_dir(directory).unwrap();
}
