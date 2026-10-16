use super::*;

#[test]
fn joined_row_preserves_display_contract_and_problem_order() {
    let (mut talos, kube) = projection_sources();
    talos.responding = false;
    let row = join::join(&[talos], &[kube], true, true).remove(0);
    assert_eq!(row.key.talos.as_deref(), Some("joined"));
    assert_eq!(row.key.kubernetes.as_deref(), Some("joined"));
    assert_eq!(row.id, "node-joined");
    assert_eq!(row.open_id, "node-joined-open");
    assert_eq!(row.name, "joined");
    assert_eq!(row.role, presentation::Role::ControlPlane);
    assert_eq!(row.address, "10.0.0.1");
    assert_eq!(row.tone, Tone::Crit);
    assert_eq!(row.note, "NotReady");
    assert_eq!(row.ready, "NotReady");
    assert_eq!(row.talos_state, "No response");
    assert_eq!(row.load, "1.00 · 2.00 · 3.00");
    assert_eq!(row.memory, "90% · 10.0 KB");
    assert_eq!(row.services, "—");
    assert!(row.service_problem);
    assert_eq!(row.pods, "7");
    assert_eq!(row.pod_label, "Pods 7");
    assert_eq!(row.kubelet_pods, "Pods on this node (7)");
    assert_eq!(
        row.problems,
        [
            "Kubernetes NotReady",
            "Talos API not answering",
            "kubelet unhealthy",
            "Memory 90%"
        ]
    );
    assert_eq!(
        row.chips,
        [
            "Control plane",
            "10.0.0.1",
            "Talos v1.12.0",
            "8 cores",
            "10.0 KB",
            "kubelet v1.35.0",
            "Cordoned"
        ]
    );
    let facts: Vec<_> = row
        .facts
        .iter()
        .map(|(key, value)| (key.as_ref(), value.as_ref()))
        .collect();
    assert_eq!(
        facts,
        [
            ("Kubernetes", "NotReady"),
            ("Talos", "No response"),
            ("Load", "1.00 · 2.00 · 3.00"),
            ("Memory", "90% · 10.0 KB"),
            ("System services", "—"),
            ("etcd member", "Yes"),
            ("Ready", "False · KubeletNotReady · runtime unavailable"),
            ("Hostname", "joined.example"),
            ("InternalIP", "10.0.0.2"),
            ("Capacity cpu", "16"),
            ("Capacity memory", "16Gi"),
            ("Taint", "dedicated=infra:NoSchedule"),
        ]
    );
}

#[test]
fn partial_rows_keep_source_labels_and_display_fallbacks() {
    let (mut talos, mut kube) = projection_sources();
    talos.version = None;
    talos.load = None;
    talos.memory = None;
    talos.services.clear();
    talos.role = presentation::Role::Unknown;
    for (available, ready) in [
        (false, "Kubernetes unavailable"),
        (true, "Not in Kubernetes"),
    ] {
        let row = join::join(&[talos.clone()], &[], true, available).remove(0);
        assert_eq!(row.ready, ready);
        assert_eq!(row.talos_state, "Version not reported");
        assert_eq!(row.role, presentation::Role::Unknown);
        assert_eq!(row.note, "worker");
        assert_eq!(row.tone, Tone::Good);
        assert_eq!(row.services, "0 healthy · 0 unhealthy");
        assert_eq!(row.load, "—");
        assert_eq!(row.memory, "—");
        assert_eq!(row.pods, "—");
        assert_eq!(row.pod_label, "Pods");
        assert_eq!(row.kubelet_pods, "Pods on this node");
        assert!(row.problems.is_empty());
        assert!(row.key.kubernetes.is_none());
    }
    kube.roles.clear();
    kube.conditions[0].status = "True".into();
    for (available, state) in [(false, "Talos unavailable"), (true, "No Talos data")] {
        let row = join::join(&[], &[kube.clone()], available, true).remove(0);
        assert_eq!(row.talos_state, state);
        assert_eq!(row.ready, "Ready");
        assert_eq!(row.role, presentation::Role::Worker);
        assert_eq!(row.address, "10.0.0.2");
        assert_eq!(row.tone, Tone::Good);
        assert_eq!(row.services, "—");
        assert_eq!(
            row.chips,
            [
                "Worker",
                "10.0.0.2",
                "16 cores",
                "16Gi memory",
                "kubelet v1.35.0",
                "Cordoned"
            ]
        );
        assert!(!row.service_problem);
        assert!(row.key.talos.is_none());
    }
    kube.addresses.pop();
    assert_eq!(
        join::join(&[], &[kube.clone()], false, true)[0].address,
        "joined.example"
    );
    kube.addresses.clear();
    assert_eq!(join::join(&[], &[kube.clone()], false, true)[0].address, "");
    talos.address.clear();
    kube.addresses
        .push(("InternalIP".into(), "10.0.0.2".into()));
    assert_eq!(join::join(&[talos], &[kube], true, true)[0].address, "");
}

#[test]
fn compact_note_tracks_health_priority_then_role() {
    let (mut talos, mut kube) = projection_sources();
    kube.conditions[0].status = "True".into();
    talos.responding = false;
    let project = |talos: &presentation::NodeSummary| {
        join::join(
            std::slice::from_ref(talos),
            std::slice::from_ref(&kube),
            true,
            true,
        )
        .remove(0)
    };
    assert_eq!(project(&talos).note, "No response");
    talos.responding = true;
    assert_eq!(project(&talos).note, "1 svc");
    assert_eq!(project(&talos).tone, Tone::Warn);
    assert_eq!(project(&talos).services, "0 healthy · 1 unhealthy");
    talos.services[0].health.as_mut().unwrap().unknown = true;
    assert_eq!(project(&talos).note, "90% · 10.0 KB");
    assert!(!project(&talos).service_problem);
    // Just under the memory level's High.
    let memory = talos.memory.as_mut().unwrap();
    memory.used = memory.total * 85 / 100 - 1;
    assert_eq!(project(&talos).note, "cp");
    assert_eq!(project(&talos).tone, Tone::Good);
}

#[test]
fn names_are_reserved_before_address_fallback_and_each_side_joins_once() {
    let mut talos = presentation::node_summaries(&examples::cluster("prod-fra", 0));
    let summary = example::summary("prod-fra", chrono::Utc::now().timestamp());
    let kube = summary.nodes.loaded().unwrap();
    talos[0].name = "different-hostname".into();
    talos[0].address = talos[1].address.clone();
    let rows = join::join(&talos, kube, true, true);
    assert_eq!(rows.len(), 7);
    let reserved = rows
        .iter()
        .find(|row| row.key.talos.as_deref() == Some(&talos[1].name))
        .unwrap();
    assert_eq!(
        reserved.key.kubernetes.as_deref(),
        Some(talos[1].name.as_str())
    );
    let unmatched = rows
        .iter()
        .find(|row| row.key.talos.as_deref() == Some("different-hostname"))
        .unwrap();
    assert_eq!(unmatched.ready, "Not in Kubernetes");
    assert_eq!(
        rows.iter()
            .filter(|row| row.key.kubernetes.is_some())
            .count(),
        6
    );
    let rows = join::join(&[], kube, false, true);
    assert!(
        rows.iter()
            .all(|row| row.talos_state == "Talos unavailable")
    );
    let rows = join::join(&talos, &[], true, false);
    assert!(rows.iter().all(|row| row.ready == "Kubernetes unavailable"));
}

#[test]
fn an_unmatched_hostname_uses_its_address_once() {
    let mut talos = presentation::node_summaries(&examples::cluster("prod-fra", 0));
    let summary = example::summary("prod-fra", chrono::Utc::now().timestamp());
    let kube = summary.nodes.loaded().unwrap();
    let name = talos[0].name.clone();
    talos[0].name = "changed-hostname".into();
    let rows = join::join(&talos, kube, true, true);
    assert_eq!(rows.len(), 6);
    assert_eq!(
        rows.iter()
            .find(|row| row.key.talos.as_deref() == Some("changed-hostname"))
            .unwrap()
            .key
            .kubernetes
            .as_deref(),
        Some(name.as_str())
    );
}

#[test]
fn health_tally_treats_stale_and_absent_sources_as_unknown() {
    use super::super::projection::Status;
    let (talos, kube) = projection_sources();
    let mut stale = join::join(
        std::slice::from_ref(&talos),
        std::slice::from_ref(&kube),
        true,
        false,
    );
    stale[0].tone = super::super::projection::assessed_tone(&stale[0], true);
    assert_eq!(Status::of(&stale[0]), Status::Unknown);
    let mut absent = join::join(std::slice::from_ref(&talos), &[], true, true);
    absent[0].tone = super::super::projection::assessed_tone(&absent[0], true);
    assert_eq!(Status::of(&absent[0]), Status::Unknown);
    let current = join::join(&[talos], &[kube], true, true);
    assert_eq!(Status::of(&current[0]), Status::Failing);
}

#[test]
fn incomplete_assessments_are_unknown_and_ready_service_or_memory_problems_warn() {
    let (mut talos, mut kube) = projection_sources();
    kube.conditions[0].status = "True".into();
    let warning = join::join(
        std::slice::from_ref(&talos),
        std::slice::from_ref(&kube),
        true,
        true,
    )
    .remove(0);
    assert_eq!(warning.ready, "Ready");
    assert_eq!(
        super::super::projection::assessed_tone(&warning, true),
        Tone::Warn
    );
    assert_eq!(
        super::super::projection::assessed_tone(&warning, false),
        Tone::Unknown
    );
    let kube_only = join::join(&[], std::slice::from_ref(&kube), false, true).remove(0);
    assert_eq!(
        super::super::projection::assessed_tone(&kube_only, false),
        Tone::Unknown
    );
    talos.services.clear();
    talos.memory.as_mut().unwrap().used = 100;
    let healthy = join::join(&[talos], std::slice::from_ref(&kube), true, true).remove(0);
    assert_eq!(
        super::super::projection::assessed_tone(&healthy, true),
        Tone::Good
    );
    kube.conditions[0].status = "False".into();
    let failing = join::join(&[], &[kube], false, true).remove(0);
    assert_eq!(
        super::super::projection::assessed_tone(&failing, false),
        Tone::Crit
    );
}

#[test]
fn service_status_uses_health_facts_and_keeps_unknown_and_stale_honest() {
    let (mut talos, kube) = projection_sources();
    let row = join::join(&[talos.clone()], std::slice::from_ref(&kube), true, true).remove(0);
    // Kubernetes NotReady makes the node failing; a service problem is a warning.
    assert_eq!(row.tone, Tone::Crit);
    assert_eq!(row.service_status.tone, Tone::Warn);
    assert_eq!(row.service_status.count, "1");
    assert_eq!(
        row.service_status.label,
        "System services: 0 healthy, 1 unhealthy, 0 unknown"
    );
    talos.services[0].health.as_mut().unwrap().healthy = true;
    let mut row = join::join(&[talos.clone()], &[kube], true, true).remove(0);
    // Display text is not a health source.
    row.services = "99 unhealthy".into();
    assert_eq!(row.service_status.tone, Tone::Good);
    assert_eq!(row.service_status.count, "1");
    assert_eq!(
        row.service_status.label,
        "System services: 1 healthy, 0 unhealthy, 0 unknown"
    );
    let stale = join::ServiceStatus::new(Some(&talos), false);
    assert_eq!(stale.tone, Tone::Unknown);
    assert_eq!(stale.count, "—");
    assert_eq!(
        stale.label,
        "System services: 1 healthy, 0 unhealthy, 0 unknown · last known"
    );
    talos.services[0].health = None;
    let unknown = join::ServiceStatus::new(Some(&talos), true);
    assert_eq!(unknown.tone, Tone::Unknown);
    assert_eq!(unknown.count, "1");
    assert_eq!(
        unknown.label,
        "System services: 0 healthy, 0 unhealthy, 1 unknown"
    );
    talos.services.clear();
    let empty = join::ServiceStatus::new(Some(&talos), true);
    assert_eq!(empty.tone, Tone::Unknown);
    assert_eq!(empty.count, "—");
    talos.responding = false;
    for source in [Some(&talos), None] {
        let absent = join::ServiceStatus::new(source, true);
        assert_eq!(absent.tone, Tone::Unknown);
        assert_eq!(absent.count, "—");
        assert_eq!(absent.label, "System services unavailable");
    }
}

/// Resources gets the same node, worded as Nodes words it: readiness and
/// when it changed, memory and whether it is last known, and the kubelet.
#[test]
fn resource_facts_carry_what_resources_shows() {
    use crate::resources::NodeMemory;
    let (mut talos, mut kube) = projection_sources();
    let since = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    kube.conditions[0].since = Some(since);
    let row = join::join(&[talos.clone()], &[kube.clone()], true, true).remove(0);
    let facts = row.resource_facts().unwrap();
    assert_eq!(facts.name, "joined");
    assert!(facts.not_ready);
    assert_eq!(facts.not_ready_since, Some(1_700_000_000));
    assert_eq!(facts.ready_label, "NotReady");
    assert_eq!(facts.tone, row.tone);
    assert_eq!(
        facts.memory,
        Some(NodeMemory {
            used: 9216,
            total: 10240
        })
    );
    assert_eq!(facts.memory_label, row.memory);
    assert!(!facts.memory_stale);
    assert_eq!(facts.problems, row.problems);
    assert!(facts.services);
    assert_eq!(
        facts.kubelet,
        Some((Tone::Crit, "kubelet running · unhealthy".into()))
    );

    talos.responding = false;
    talos.services.clear();
    let silent = join::join(&[talos.clone()], &[kube.clone()], true, true).remove(0);
    let facts = silent.resource_facts().unwrap();
    assert!(facts.memory_stale);
    assert_eq!(facts.memory_label, silent.memory);
    assert_eq!(
        facts.kubelet,
        Some((Tone::Unknown, "kubelet not reported".into()))
    );

    kube.conditions[0].status = "True".into();
    let kubernetes_only = join::join(&[], &[kube], false, true).remove(0);
    let facts = kubernetes_only.resource_facts().unwrap();
    assert!(!facts.not_ready);
    assert_eq!(facts.memory, None);
    assert!(facts.memory_stale);
    assert!(!facts.services);
    assert_eq!(facts.kubelet, None);

    let talos_only = join::join(&[talos], &[], true, true).remove(0);
    assert_eq!(talos_only.resource_facts(), None);
}

/// Pods name their node by the Kubernetes node's name, which the joined
/// row's key carries, so one name serves the not-ready group and the card.
#[test]
fn resource_facts_name_each_kubernetes_node_once() {
    let talos = presentation::node_summaries(&examples::cluster("prod-fra", 0));
    let summary = example::summary("prod-fra", 0);
    let kubernetes = summary.nodes.loaded().unwrap();
    let rows = join::join(&talos, kubernetes, true, true);
    let facts: Vec<_> = rows.iter().filter_map(|row| row.resource_facts()).collect();
    assert_eq!(facts.len(), kubernetes.len());
    for node in kubernetes {
        let facts = facts.iter().find(|facts| facts.name == node.name).unwrap();
        assert_eq!(facts.not_ready, !node.is_ready());
    }
    assert!(facts.iter().any(|facts| facts.not_ready));
}
