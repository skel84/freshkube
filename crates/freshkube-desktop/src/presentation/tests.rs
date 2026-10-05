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
fn whole_percent_rounds_down_so_a_value_never_reads_as_the_next_threshold() {
    assert_eq!(whole_percent(84.9), 84);
    assert_eq!(whole_percent(85.0), 85);
    assert_eq!(whole_percent(94.5), 94);
    assert_eq!(whole_percent(94.99), 94);
    assert_eq!(whole_percent(95.0), 95);
    assert_eq!(whole_percent(100.0), 100);
    assert_eq!(whole_percent(0.4), 0);
    assert_eq!(whole_percent(-1.0), 0);
}

#[test]
fn etcd_card_shows_remaining_tolerance_and_warns_when_none_remains() {
    for (healthy, total, remaining) in [(2, 3, 0), (3, 5, 0), (3, 3, 1), (5, 5, 2)] {
        let mut cluster = crate::fixture::cluster("prod-fra", 1);
        cluster.etcd_summary = Some(freshkube_core::cluster_overview::EtcdSummary {
            healthy,
            total,
            has_quorum: true,
        });
        let overview = overview::Overview::build(&[], &[], None, Some(&cluster), true, false);
        let card = overview
            .cards
            .iter()
            .find(|card| card.id == "tile-etcd")
            .unwrap();
        assert!(
            card.detail
                .contains(&format!("tolerates {remaining} additional member"))
        );
        assert_eq!(
            card.tone,
            if remaining > 0 {
                crate::ui::Tone::Good
            } else {
                crate::ui::Tone::Warn
            }
        );
    }
}
#[test]
fn peak_memory_card_takes_the_memory_levels_tone_and_word() {
    use crate::ui::Tone;
    let cluster = crate::fixture::cluster("prod-fra", 1);
    for (percent, tone, word) in [
        (84., Tone::Good, None),
        (85., Tone::Warn, Some("High")),
        (90., Tone::Warn, Some("High")),
        (94.9, Tone::Warn, Some("High")),
        (95., Tone::Crit, Some("Critical")),
    ] {
        let node = NodeSummary {
            name: "talos-cp-1".into(),
            address: "192.0.2.1".into(),
            role: Role::ControlPlane,
            etcd_member: true,
            responding: true,
            version: None,
            cores: Some(4),
            memory: Some(Memory {
                used: (percent * 10.) as u64,
                total: 1000,
            }),
            load: None,
            services: Vec::new(),
        };
        let overview = overview::Overview::build(&[], &[node], None, Some(&cluster), true, false);
        let card = overview
            .cards
            .iter()
            .find(|card| card.id == "tile-memory")
            .unwrap();
        assert_eq!(card.tone, tone, "{percent}");
        assert_eq!(
            card.meter.map(|(_, level)| level),
            Some(memory_level(percent))
        );
        assert_eq!(
            card.detail.as_ref(),
            match word {
                Some(word) => format!("talos-cp-1 · {word}"),
                None => "talos-cp-1".into(),
            },
            "{percent}"
        );
    }
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
