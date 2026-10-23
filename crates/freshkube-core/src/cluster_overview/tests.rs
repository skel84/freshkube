use super::*;
use talos_rs::ServiceInfo;

#[tokio::test]
async fn parsed_connection_does_not_reopen_the_configured_path() {
    let parsed = TalosConfig::parse("context: unused\ncontexts: {}").unwrap();
    let collector = ClusterOverviewCollector::new(
        Some(PathBuf::from("/nonexistent/never-reopen-talosconfig")),
        None,
    );
    assert!(
        collector
            .connect_from_config(&parsed)
            .await
            .unwrap()
            .is_empty()
    );

    let filtered = ClusterOverviewCollector::new(
        Some(PathBuf::from("/nonexistent/never-reopen-talosconfig")),
        Some("missing".into()),
    );
    assert!(matches!(
        filtered.connect_from_config(&parsed).await,
        Err(ClusterOverviewError::ContextNotFound { name, .. }) if name == "missing"
    ));
}

#[tokio::test]
async fn identity_client_requires_connected_selected_talos_cluster() {
    let mut collector = ClusterOverviewCollector::default();
    collector.set_kubeconfig_selection(KubeconfigSelection::File {
        path: PathBuf::from("/nonexistent/explicit-kubeconfig"),
        context: Some("selected".into()),
    });
    let cluster = ClusterOverview {
        name: "selected-talos".into(),
        ..Default::default()
    };
    let error = match collector.kubernetes_client(&cluster).await {
        Ok(_) => panic!("Disconnected identity must not create an ambient client"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("selected Talos context is disconnected")
    );
}

#[test]
fn explicit_file_rejection_retains_only_pinned_source_with_warning() {
    let pinned = kube::config::Kubeconfig::from_yaml(
        "apiVersion: v1\nkind: Config\ncurrent-context: pinned\ncontexts:\n- name: pinned\n  context:\n    cluster: selected\nclusters:\n- name: selected\n  cluster:\n    server: https://10.0.0.1:6443\n",
    ).unwrap();
    let prepared = prepare_kubeconfig(
        &KubeconfigSelection::File {
            path: PathBuf::from("/nonexistent/explicit-kubeconfig"),
            context: Some("requested".into()),
        },
        Some(pinned),
        Some("10.0.0.1"),
        || panic!("Explicit source must not consult ambient credentials"),
    );
    assert!(prepared.source.contains("selected file rejected"));
    assert!(
        prepared
            .warning
            .as_deref()
            .unwrap()
            .contains("ambient configuration is not used")
    );
    assert_eq!(
        prepared.config.unwrap().current_context.as_deref(),
        Some("pinned")
    );
}

fn version(node: &str) -> VersionInfo {
    VersionInfo {
        node: node.to_string(),
        version: String::new(),
        sha: String::new(),
        built: String::new(),
        go_version: String::new(),
        os: String::new(),
        arch: String::new(),
        platform: String::new(),
    }
}

fn member(hostname: &str, ip: &str, machine_type: &str) -> DiscoveryMember {
    DiscoveryMember {
        id: hostname.to_string(),
        addresses: vec![ip.to_string()],
        hostname: hostname.to_string(),
        machine_type: machine_type.to_string(),
        operating_system: String::new(),
    }
}

fn services(node: &str, ids: &[&str]) -> NodeServices {
    NodeServices {
        node: node.to_string(),
        services: ids
            .iter()
            .map(|id| ServiceInfo {
                id: id.to_string(),
                state: "Running".to_string(),
                health: None,
            })
            .collect(),
    }
}

#[test]
fn discovery_machine_type_is_authoritative_with_etcd_fallback() {
    let cluster = ClusterOverview {
        versions: vec![version("cp1"), version("worker1"), version("legacy-cp")],
        discovery_members: vec![
            member("cp1", "10.0.0.1", "controlplane"),
            member("worker1", "10.0.0.2", "worker"),
        ],
        services: vec![services("legacy-cp", &["etcd", "kubelet"])],
        ..Default::default()
    };

    assert!(cluster.node_is_controlplane("cp1"));
    assert!(!cluster.node_is_controlplane("worker1"));
    assert!(cluster.node_is_controlplane("legacy-cp"));
    assert!(!cluster.node_is_controlplane("unknown"));
}

#[test]
fn ipv6_node_addresses_match_whole() {
    let cluster = ClusterOverview {
        discovery_members: vec![member("cp6", "2001:db8::5", "controlplane")],
        ..Default::default()
    };
    assert!(cluster.node_is_controlplane("2001:db8::5"));
    assert!(cluster.node_is_controlplane("[2001:db8::5]:50000"));
    assert!(!cluster.node_is_controlplane("2001:db8::6"));

    assert_eq!(
        ClusterOverview {
            endpoints: vec!["[2001:db8::5]:50000".into()],
            ..Default::default()
        }
        .control_plane_ip()
        .as_deref(),
        Some("2001:db8::5")
    );

    let cluster = ClusterOverview {
        versions: vec![version("2001:db8::5"), version("10.0.0.5:50000")],
        ..Default::default()
    };
    assert_eq!(
        nodes_to_query(&cluster),
        vec![
            ("2001:db8::5".to_string(), "2001:db8::5".to_string()),
            ("10.0.0.5:50000".to_string(), "10.0.0.5".to_string()),
        ]
    );
}

#[test]
fn k8s_roster_maps_roles_and_drops_ipless_nodes() {
    let members = k8s_nodes_to_discovery_members(vec![
        K8sNodeInfo {
            name: "cp1".into(),
            internal_ip: Some("10.0.0.1".into()),
            is_control_plane: true,
        },
        K8sNodeInfo {
            name: "w1".into(),
            internal_ip: Some("10.0.0.2".into()),
            is_control_plane: false,
        },
        K8sNodeInfo {
            name: "ghost".into(),
            internal_ip: None,
            is_control_plane: false,
        },
    ]);

    assert_eq!(members.len(), 2);
    assert_eq!(members[0].machine_type, "controlplane");
    assert_eq!(members[1].machine_type, "worker");
    assert_eq!(members[1].addresses, vec!["10.0.0.2"]);
}

#[test]
fn control_plane_ip_prefers_etcd_then_roster_then_endpoint() {
    let etcd_member = EtcdMemberInfo {
        id: 1,
        hostname: "cp1".into(),
        peer_urls: vec!["https://10.0.0.1:2380".into()],
        client_urls: vec!["https://10.0.0.1:2379".into()],
        is_learner: false,
    };
    let roster_member = member("cp2", "10.0.0.9", "controlplane");

    assert_eq!(
        ClusterOverview {
            etcd_members: vec![etcd_member],
            discovery_members: vec![roster_member.clone()],
            endpoints: vec!["10.0.0.5:50000".into()],
            ..Default::default()
        }
        .control_plane_ip()
        .as_deref(),
        Some("10.0.0.1")
    );
    assert_eq!(
        ClusterOverview {
            discovery_members: vec![roster_member],
            endpoints: vec!["10.0.0.5:50000".into()],
            ..Default::default()
        }
        .control_plane_ip()
        .as_deref(),
        Some("10.0.0.9")
    );
    assert_eq!(
        ClusterOverview {
            endpoints: vec!["10.0.0.5:50000".into()],
            ..Default::default()
        }
        .control_plane_ip()
        .as_deref(),
        Some("10.0.0.5")
    );
}

#[test]
fn collector_uses_replaced_config_path() {
    let mut collector =
        ClusterOverviewCollector::new(Some(PathBuf::from("/initial/talosconfig")), None);

    collector.set_config_path(Some(PathBuf::from("/selected/talosconfig")));

    assert_eq!(
        collector.config_path.as_ref(),
        Some(&PathBuf::from("/selected/talosconfig"))
    );
}

#[cfg(unix)]
#[test]
fn collector_accepts_non_utf8_config_path() {
    use std::os::unix::ffi::OsStringExt;

    let path = PathBuf::from(std::ffi::OsString::from_vec(vec![
        b'/', b't', b'm', b'p', b'/', 0xFF,
    ]));
    let collector = ClusterOverviewCollector::new(Some(path.clone()), None);

    assert_eq!(collector.config_path, Some(path));
}

#[test]
fn kubeconfig_selection_is_per_collector_and_defaults_to_automatic() {
    let mut selected = ClusterOverviewCollector::new(None, None);
    let unchanged = selected.clone();
    let choice = KubeconfigSelection::File {
        path: PathBuf::from("/session/kubeconfig"),
        context: Some("chosen".into()),
    };
    selected.set_kubeconfig_selection(choice.clone());
    assert_eq!(selected.kubeconfig_selection, choice);
    assert_eq!(
        unchanged.kubeconfig_selection,
        KubeconfigSelection::Automatic
    );
    assert_eq!(
        ClusterOverviewCollector::default().kubeconfig_selection,
        KubeconfigSelection::Automatic
    );
}

fn services_for(node: &str) -> NodeServices {
    NodeServices {
        node: node.into(),
        services: vec![],
    }
}

fn nodes(names: &[&str]) -> Vec<(String, String)> {
    names
        .iter()
        .enumerate()
        .map(|(index, name)| ((*name).to_string(), format!("10.0.0.{index}")))
        .collect()
}

#[tokio::test]
async fn nodes_are_queried_concurrently() {
    // Every node waits for all the others; sequential collection would
    // never fill the barrier and each wait would time out.
    let barrier = Arc::new(tokio::sync::Barrier::new(4));
    let results = collect_bounded(nodes(&["a", "b", "c", "d"]), 8, move |name, _| {
        let barrier = Arc::clone(&barrier);
        async move {
            let met = tokio::time::timeout(Duration::from_secs(5), barrier.wait())
                .await
                .is_ok();
            NodeMetrics {
                services: vec![services_for(&name)],
                version_transport_failure: !met,
                ..Default::default()
            }
        }
    })
    .await;
    assert_eq!(results.len(), 4);
    assert!(results.iter().all(|node| !node.version_transport_failure));
}

#[tokio::test]
async fn concurrency_is_bounded() {
    let running = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (running_in, peak_in) = (Arc::clone(&running), Arc::clone(&peak));
    let results = collect_bounded(nodes(&["a", "b", "c", "d", "e", "f"]), 2, move |_, _| {
        let (running, peak) = (Arc::clone(&running_in), Arc::clone(&peak_in));
        async move {
            use std::sync::atomic::Ordering::SeqCst;
            let now = running.fetch_add(1, SeqCst) + 1;
            peak.fetch_max(now, SeqCst);
            tokio::time::sleep(Duration::from_millis(30)).await;
            running.fetch_sub(1, SeqCst);
            NodeMetrics::default()
        }
    })
    .await;
    assert_eq!(results.len(), 6);
    assert_eq!(peak.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[tokio::test]
async fn slow_node_times_out_without_delaying_the_others() {
    let started = std::time::Instant::now();
    let results = collect_bounded(nodes(&["down", "up-1", "up-2"]), 8, |name, _| async move {
        let limit = Duration::from_millis(150);
        let call = async {
            if name == "down" {
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            Ok::<_, TalosError>(vec![services_for(&name)])
        };
        let outcome = bounded_call(call, limit).await;
        NodeMetrics {
            version_transport_failure: outcome.transport_failure(),
            services: outcome.into_data(),
            ..Default::default()
        }
    })
    .await;
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(results[0].version_transport_failure);
    assert!(results[0].services.is_empty());
    assert_eq!(results[1].services[0].node, "up-1");
    assert_eq!(results[2].services[0].node, "up-2");
}

#[tokio::test]
async fn output_order_follows_input_not_completion() {
    let results = collect_bounded(nodes(&["slow", "mid", "fast"]), 8, |name, _| async move {
        let delay = match name.as_str() {
            "slow" => 90,
            "mid" => 45,
            _ => 1,
        };
        tokio::time::sleep(Duration::from_millis(delay)).await;
        NodeMetrics {
            services: vec![services_for(&name)],
            ..Default::default()
        }
    })
    .await;
    let order: Vec<_> = results
        .iter()
        .map(|node| node.services[0].node.as_str())
        .collect();
    assert_eq!(order, ["slow", "mid", "fast"]);
}

#[tokio::test]
async fn a_panicking_node_does_not_hide_the_others() {
    let results = collect_bounded(nodes(&["bad", "good"]), 8, |name, _| async move {
        if name == "bad" {
            panic!("collector bug");
        }
        NodeMetrics {
            services: vec![services_for(&name)],
            ..Default::default()
        }
    })
    .await;
    assert!(results[0].services.is_empty());
    assert_eq!(results[1].services[0].node, "good");
}

#[test]
fn rejected_file_selection_keeps_fallback_and_warning_when_cached() {
    use crate::client_cache::TtlCache;
    let pinned = kube::config::Kubeconfig::from_yaml(
        "apiVersion: v1\nkind: Config\ncurrent-context: pinned\ncontexts:\n- name: pinned\n  context:\n    cluster: selected\nclusters:\n- name: selected\n  cluster:\n    server: https://10.0.0.1:6443\n",
    ).unwrap();
    let prepared = prepare_kubeconfig(
        &KubeconfigSelection::File {
            path: PathBuf::from("/nonexistent/explicit-kubeconfig"),
            context: Some("requested".into()),
        },
        Some(pinned),
        Some("10.0.0.1"),
        || panic!("Explicit source must not consult ambient credentials"),
    );
    let now = std::time::Instant::now();
    let mut cache = TtlCache::new(Some(Duration::from_secs(120)), 4);
    cache.insert("key", prepared, now);
    let served = cache.get(&"key", now + Duration::from_secs(60)).unwrap();
    assert!(served.source.contains("selected file rejected"));
    assert!(
        served
            .warning
            .as_deref()
            .unwrap()
            .contains("ambient configuration is not used")
    );
    assert_eq!(
        served.config.unwrap().current_context.as_deref(),
        Some("pinned")
    );
    assert!(cache.get(&"key", now + Duration::from_secs(121)).is_none());
}

#[test]
fn unavailable_prepared_kubeconfig_is_not_cacheable() {
    assert!(
        PreparedKubeconfig::unavailable("transient")
            .config
            .is_none()
    );
}

#[test]
fn etcd_summary_counts_each_known_voter_once_and_excludes_learners() {
    let members: Vec<_> = (1..=3)
        .map(|id| EtcdMemberInfo {
            id,
            hostname: format!("cp-{id}"),
            peer_urls: Vec::new(),
            client_urls: Vec::new(),
            is_learner: id == 3,
        })
        .collect();
    let status = |id| EtcdMemberStatus {
        node: format!("cp-{id}"),
        member_id: id,
        protocol_version: String::new(),
        db_size: 0,
        db_size_in_use: 0,
        leader_id: 1,
        raft_index: 0,
        raft_term: 1,
        raft_applied_index: 0,
        errors: Vec::new(),
        is_learner: id == 3,
    };
    let summary = EtcdSummary::from_statuses(
        &members,
        &[status(1), status(1), status(2), status(3), status(99)],
    );
    assert_eq!(
        (summary.healthy, summary.total, summary.has_quorum),
        (2, 2, true)
    );
    assert_eq!(
        crate::indicators::quorum(summary.healthy, summary.total).remaining_tolerance,
        0
    );
    let summary =
        EtcdSummary::from_statuses(&members, &[status(1), status(1), status(3), status(99)]);
    assert_eq!(
        (summary.healthy, summary.total, summary.has_quorum),
        (1, 2, false)
    );
}

fn known_roster() -> ClusterOverview {
    let roster = vec![
        member("cp1", "10.0.0.1", "controlplane"),
        member("worker1", "10.0.0.2", "worker"),
    ];
    let mut cluster = ClusterOverview {
        name: "example".into(),
        ..Default::default()
    };
    replace_node_ips_from_discovery(&mut cluster, &roster);
    cluster.discovery_members = roster;
    cluster
}

#[test]
fn a_failed_discovery_keeps_a_known_roster_unless_a_kubeconfig_was_chosen() {
    let mut collector = ClusterOverviewCollector::default();
    assert!(!collector.roster_needs_kubernetes(&known_roster()));
    assert!(collector.roster_needs_kubernetes(&ClusterOverview::default()));

    collector.set_kubeconfig_selection(KubeconfigSelection::File {
        path: PathBuf::from("/nonexistent/explicit-kubeconfig"),
        context: None,
    });
    assert!(collector.roster_needs_kubernetes(&known_roster()));
}

#[test]
fn a_failed_shared_roster_keeps_the_known_roster_as_stale_evidence() {
    for failed in [
        crate::kubernetes_summary::Part::Failed("connection reset".into()),
        crate::kubernetes_summary::Part::Refused("nodes is forbidden".into()),
    ] {
        let reason = failed.error().unwrap().to_owned();
        let mut collector = ClusterOverviewCollector::default();
        collector.set_observed_nodes(Some(failed));
        let mut cluster = known_roster();

        assert!(collector.roster_from_observed(&mut cluster));

        let names = |cluster: &ClusterOverview| {
            cluster
                .discovery_members
                .iter()
                .map(|member| (member.hostname.clone(), member.addresses.clone()))
                .collect::<Vec<_>>()
        };
        let known = known_roster();
        assert_eq!(names(&cluster), names(&known));
        assert_eq!(cluster.node_ips, known.node_ips);
        assert_eq!(
            cluster.discovery_warning.as_deref(),
            Some(format!("Shared Kubernetes roster: {reason}").as_str())
        );
    }
}

#[test]
fn a_kept_roster_without_a_talos_answer_is_not_connected() {
    let cluster = known_roster();
    let status = connection_status(&cluster, false);
    assert!(!status.is_connected());
    assert!(status.error().unwrap().contains("last one known"));
}

#[test]
fn a_kubernetes_roster_without_a_talos_answer_is_not_connected() {
    let cluster = ClusterOverview {
        discovery_members: k8s_nodes_to_discovery_members(vec![K8sNodeInfo {
            name: "cp1".into(),
            internal_ip: Some("10.0.0.1".into()),
            is_control_plane: true,
        }]),
        ..Default::default()
    };
    assert!(!connection_status(&cluster, false).is_connected());
}

#[test]
fn a_talos_answer_is_connected_and_nothing_is_unreachable_without_a_roster() {
    assert!(connection_status(&known_roster(), true).is_connected());
    assert!(connection_status(&ClusterOverview::default(), true).is_connected());
    let status = connection_status(&ClusterOverview::default(), false);
    assert!(!status.error().unwrap().contains("last one known"));
}
