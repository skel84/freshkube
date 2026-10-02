use super::*;
#[test]
fn unavailable_health_never_becomes_healthy_or_failed() {
    let mut service = ServiceInfo {
        id: "kubelet".into(),
        state: "Running".into(),
        health: None,
    };
    assert_eq!(service_health(&service), Health::Unknown);
    service.health = Some(talos_rs::ServiceHealth {
        unknown: true,
        healthy: false,
        last_message: String::new(),
    });
    assert_eq!(service_health(&service), Health::Unknown);
}
#[test]
fn service_selection_uses_exact_domain_identity_across_refresh() {
    let services = vec![
        ServiceInfo {
            id: "APId".into(),
            state: "Running".into(),
            health: None,
        },
        ServiceInfo {
            id: "kubelet".into(),
            state: "Running".into(),
            health: None,
        },
    ];
    assert_eq!(
        selected_service(&services, Some("APId")).unwrap().id,
        "APId"
    );
    assert!(selected_service(&services, Some("apid")).is_none());
    assert!(selected_service(&services[1..], Some("APId")).is_none());
}
#[test]
fn missing_metrics_and_role_are_explicit() {
    let cluster = ClusterOverview {
        endpoints: vec!["192.0.2.1".into()],
        ..Default::default()
    };
    let nodes = node_summaries(&cluster);
    assert_eq!(nodes[0].role, Role::Unknown);
    assert!(!nodes[0].responding);
    assert!(nodes[0].cores.is_none());
    assert!(nodes[0].memory.is_none());
    assert!(nodes[0].load.is_none());
    let summary = cluster_summary(&cluster, &nodes);
    assert_eq!(summary.responding, 0);
    assert!(summary.peak_memory.is_none());
    assert_eq!(summary.services, HealthCounts::default());
}
#[test]
fn memory_levels_follow_core_thresholds() {
    assert_eq!(memory_level(84.9), MemoryLevel::Normal);
    assert_eq!(memory_level(MEMORY_WARNING_PERCENT), MemoryLevel::High);
    assert_eq!(memory_level(MEMORY_CRITICAL_PERCENT), MemoryLevel::Critical);
}
#[test]
fn etcd_tolerance_counts_members_that_can_fail() {
    assert_eq!(etcd_failure_tolerance(0), 0);
    assert_eq!(etcd_failure_tolerance(1), 0);
    assert_eq!(etcd_failure_tolerance(3), 1);
    assert_eq!(etcd_failure_tolerance(5), 2);
}
#[test]
fn load_history_is_bounded_and_forgets_departed_nodes() {
    let node = |name: &str, load: Option<f64>| NodeSummary {
        name: name.into(),
        address: name.into(),
        role: Role::Worker,
        etcd_member: false,
        responding: load.is_some(),
        version: None,
        cores: Some(4),
        memory: None,
        load: load.map(|load| [load, load, load]),
        services: Vec::new(),
    };
    let mut history = LoadHistory::default();
    for ix in 0..LOAD_HISTORY_LEN + 5 {
        history.record(&[node("a", Some(ix as f64)), node("b", None)]);
    }
    let samples = history.get("a");
    assert_eq!(samples.len(), LOAD_HISTORY_LEN);
    assert_eq!(samples.last(), Some(&((LOAD_HISTORY_LEN + 4) as f64)));
    assert!(history.get("b").is_empty());
    history.record(&[node("b", Some(1.0))]);
    assert!(history.get("a").is_empty());
    assert_eq!(history.get("b"), vec![1.0]);
}
