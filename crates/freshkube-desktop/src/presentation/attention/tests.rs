use super::*;
use crate::resources::example;
use freshkube_core::{
    cluster_overview::EtcdSummary,
    kubernetes_summary::Part,
    workloads::{WorkloadCollectionOutcome, WorkloadSource, WorkloadSourceError},
};

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000, 0).unwrap()
}

fn node() -> NodeRow {
    let summary = example::summary("prod-fra", now().timestamp());
    let kubernetes = summary.nodes.loaded().unwrap()[0].clone();
    let talos =
        crate::presentation::node_summaries(&crate::fixture::cluster("prod-fra", 0))[0].clone();
    NodeRow {
        kubernetes_current: true,
        key: NodeKey {
            kubernetes: Some(kubernetes.name.clone()),
            talos: Some(talos.name.clone()),
        },
        id: "node-a".into(),
        open_id: "node-a-open".into(),
        pod_label: "Pods".into(),
        kubelet_pods: "Pods".into(),
        service_problem: false,
        name: "node-a".into(),
        role: talos.role,
        tone: Tone::Good,
        kubernetes: Some(kubernetes),
        talos: Some(talos),
        ready: "Ready",
        talos_state: "".into(),
        address: "".into(),
        load: "".into(),
        table_load: "".into(),
        memory: "".into(),
        pods: "".into(),
        services: "".into(),
        service_status: Default::default(),
        note: "".into(),
        chips: Vec::new(),
        facts: Vec::new(),
        problems: Vec::new(),
    }
}

#[test]
fn no_observations_produce_no_attention() {
    let attention = build(&[], None, None, now());
    assert!(attention.rows.is_empty());
    assert!(attention.by_node.is_empty());
    assert_eq!(attention.total, 0);
    assert_eq!(attention.more, "Show all 0");
}

#[test]
fn failed_parts_leave_independent_categories_unchanged() {
    let summary = example::summary("prod-fra", now().timestamp());
    let expected = build(&[], Some(&summary), None, now()).rows;
    assert!(expected.iter().any(|row| row.kind == "Pod"));
    assert!(expected.iter().any(|row| row.kind == "Deployment"));
    assert!(expected.iter().any(|row| row.kind == "Claim"));

    let mut without_pods = summary.clone();
    without_pods.pods = Part::Refused("Can't list pods: forbidden".into());
    let mut without_workloads = summary.clone();
    without_workloads.workloads = WorkloadCollectionOutcome::Unavailable {
        target: String::new(),
        errors: vec![WorkloadSourceError {
            source: WorkloadSource::Deployments,
            message: "Timed out".into(),
        }],
    };
    let mut without_claims = summary;
    without_claims.claims = Part::Failed("Timed out".into());

    for (summary, removed) in [
        (&without_pods, vec!["Pod"]),
        (
            &without_workloads,
            vec!["Deployment", "StatefulSet", "DaemonSet"],
        ),
        (&without_claims, vec!["Claim"]),
    ] {
        let kept: Vec<_> = expected
            .iter()
            .filter(|row| !removed.contains(&row.kind))
            .collect();
        assert_eq!(
            format!("{:?}", build(&[], Some(summary), None, now()).rows),
            format!("{kept:?}"),
        );
    }
}

#[test]
fn object_destinations_preserve_uids_and_pod_log_actions() {
    let summary = example::summary("prod-fra", now().timestamp());
    let attention = build(&[], Some(&summary), None, now());
    for row in &attention.rows {
        let Destination::Object(kind, object, Tab::Overview) = &row.open else {
            panic!("Unexpected destination: {:?}", row.open);
        };
        let key = (
            kind.to_string(),
            object.namespace.clone(),
            object.name.clone(),
        );
        assert_eq!(Some(&object.uid), summary.references.get(&key));
        assert_eq!(row.name, format!("{}/{}", object.namespace, object.name));
        if row.kind == "Pod" {
            let Some(Destination::Object("pods", logs, Tab::Logs)) = &row.logs else {
                panic!("Pod has no Logs destination");
            };
            assert_eq!(logs.namespace, object.namespace);
            assert_eq!(logs.name, object.name);
            assert_eq!(logs.uid, object.uid);
        } else {
            assert!(row.logs.is_none());
        }
    }
}

#[test]
fn node_problems_merge_and_unknown_service_health_stays_unknown() {
    let mut row = node();
    row.name = "node-a".into();
    row.talos.as_mut().unwrap().responding = false;
    row.talos.as_mut().unwrap().memory = Some(crate::presentation::Memory {
        used: 95,
        total: 100,
    });
    row.talos.as_mut().unwrap().services = vec![
        talos_rs::ServiceInfo {
            id: "kubelet".into(),
            state: "Running".into(),
            health: Some(talos_rs::ServiceHealth {
                unknown: false,
                healthy: false,
                last_message: String::new(),
            }),
        },
        talos_rs::ServiceInfo {
            id: "apid".into(),
            state: "Running".into(),
            health: None,
        },
        talos_rs::ServiceInfo {
            id: "trustd".into(),
            state: "Running".into(),
            health: Some(talos_rs::ServiceHealth {
                unknown: true,
                healthy: false,
                last_message: "Not reported".into(),
            }),
        },
    ];
    let ready = row
        .kubernetes
        .as_mut()
        .unwrap()
        .conditions
        .iter_mut()
        .find(|condition| condition.kind == "Ready")
        .unwrap();
    ready.status = "False".into();
    ready.since = Some(now() - chrono::Duration::minutes(10));
    let since = ready.since;
    let attention = build(&[row.clone()], None, None, now());
    assert_eq!(attention.total, 2);
    let node = &attention.rows[0];
    assert_eq!(node.id, "attention-node-node-a-node-a");
    assert_eq!(node.kind, "Node");
    assert_eq!(node.tone, Tone::Crit);
    assert_eq!(node.since, since);
    assert_eq!(
        node.reason,
        "Talos API not answering · Kubernetes NotReady for 10 min · 1 unhealthy system services · Memory at 95 %"
    );
    assert!(matches!(&node.open, Destination::Node(key, NodeTab::Overview) if key == &row.key));
    let service = &attention.rows[1];
    assert_eq!(service.reason, "Service is unhealthy");
    assert_eq!(service.tone, Tone::Warn);
    assert!(
        matches!(&service.logs, Some(Destination::Service { service, logs: true, .. }) if service == "kubelet")
    );
    assert!(
        matches!(&service.open_node, Some(Destination::Node(key, NodeTab::Services)) if key == &row.key)
    );
}

#[test]
fn a_responding_node_only_warns_once_memory_reaches_the_attention_threshold() {
    let mut node = node();
    node.kubernetes = None;
    let talos = node.talos.as_mut().unwrap();
    talos.responding = true;
    talos.services.clear();
    talos.memory = Some(crate::presentation::Memory {
        used: 89,
        total: 100,
    });
    assert_eq!(build(&[node.clone()], None, None, now()).total, 0);

    node.talos.as_mut().unwrap().memory.as_mut().unwrap().used = 90;
    let attention = build(&[node], None, None, now());
    assert_eq!(attention.total, 1);
    assert_eq!(attention.rows[0].tone, Tone::Warn);
    assert_eq!(attention.rows[0].reason, "Memory at 90 %");
}

#[test]
fn unresolved_object_uids_keep_the_same_overview_and_log_destinations() {
    let mut summary = example::summary("prod-fra", now().timestamp());
    summary.references.clear();
    let attention = build(&[], Some(&summary), None, now());
    let pod = attention.rows.iter().find(|row| row.kind == "Pod").unwrap();
    assert!(
        matches!(&pod.open, Destination::Object("pods", object, Tab::Overview) if object.uid.is_empty())
    );
    assert!(
        matches!(&pod.logs, Some(Destination::Object("pods", object, Tab::Logs)) if object.uid.is_empty())
    );
}

#[test]
fn etcd_quorum_and_deduplicated_alarms_form_one_cluster_destination() {
    let alarm = talos_rs::EtcdAlarm {
        node: "control-a".into(),
        member_id: 42,
        alarm_type: talos_rs::EtcdAlarmType::NoSpace,
    };
    let talos = ClusterOverview {
        etcd_summary: Some(EtcdSummary {
            healthy: 1,
            total: 3,
            has_quorum: false,
        }),
        etcd_alarms: Some(vec![
            alarm.clone(),
            alarm,
            talos_rs::EtcdAlarm {
                node: "control-b".into(),
                member_id: 43,
                alarm_type: talos_rs::EtcdAlarmType::None,
            },
        ]),
        ..Default::default()
    };
    let attention = build(&[], None, Some(&talos), now());
    assert_eq!(attention.total, 1);
    let row = &attention.rows[0];
    assert_eq!(row.id, "attention-etcd-cluster-etcd");
    assert_eq!(
        row.reason,
        "1 of 3 members answered; quorum needs 2 · Member 42: NOSPACE"
    );
    assert_eq!(row.tone, Tone::Crit);
    assert!(matches!(row.open, Destination::Page(Page::Etcd)));
    assert!(row.node.is_none());
    assert!(row.logs.is_none());
}

fn row(id: &str, name: &str, tone: Tone, since: Option<DateTime<Utc>>, node: &str) -> AttentionRow {
    AttentionRow {
        id: id.to_owned().into(),
        name: name.to_owned().into(),
        kind: "Pod",
        reason: "Pending".into(),
        tone,
        group: AttentionGroup::Warning,
        open: Destination::Page(Page::Resources),
        logs: None,
        node: Some(node.into()),
        open_node: None,
        since,
    }
}

#[test]
fn ordering_uses_severity_then_oldest_time_then_name_and_id() {
    let attention = finish(vec![
        row(
            "warn",
            "a",
            Tone::Warn,
            Some(now() - chrono::Duration::days(1)),
            "n",
        ),
        row("z", "same", Tone::Crit, None, "n"),
        row("recent", "z", Tone::Crit, Some(now()), "n"),
        row("a", "same", Tone::Crit, None, "n"),
        row(
            "old",
            "z",
            Tone::Crit,
            Some(now() - chrono::Duration::seconds(1)),
            "n",
        ),
        row("name", "alpha", Tone::Crit, None, "n"),
    ]);
    let ids: Vec<_> = attention.rows.iter().map(|row| row.id.as_ref()).collect();
    assert_eq!(ids, ["old", "recent", "name", "a", "z", "warn"]);
}

#[test]
fn node_groups_keep_their_own_limit_before_the_global_display_limit() {
    let mut rows = Vec::new();
    for ix in (0..60).rev() {
        rows.push(row(&format!("a-{ix:02}"), "a", Tone::Crit, None, "node-a"));
        rows.push(row(&format!("b-{ix:02}"), "b", Tone::Warn, None, "node-b"));
    }
    let attention = finish(rows);
    assert_eq!(attention.total, 120);
    assert_eq!(attention.more, "Show all 120");
    assert_eq!(attention.rows.len(), 50);
    assert!(
        attention
            .rows
            .iter()
            .all(|row| row.node.as_deref() == Some("node-a"))
    );
    let node_a = &attention.by_node["node-a"];
    let node_b = &attention.by_node["node-b"];
    assert_eq!(node_a.len(), 50);
    assert_eq!(node_b.len(), 50);
    assert_eq!(node_a[0].id, "a-00");
    assert_eq!(node_a[49].id, "a-49");
    assert_eq!(node_b[0].id, "b-00");
    assert_eq!(node_b[49].id, "b-49");
}

#[test]
fn groups_put_failing_then_warnings_then_last_known_evidence() {
    let attention = finish(vec![
        row(
            "stale",
            "a",
            Tone::Unknown,
            Some(now() - chrono::Duration::days(2)),
            "n",
        ),
        row("warn", "b", Tone::Warn, None, "n"),
        row("crit", "c", Tone::Crit, None, "m"),
        row("recent", "d", Tone::Warn, Some(now()), "n"),
    ]);
    let ids: Vec<_> = attention.rows.iter().map(|row| row.id.as_ref()).collect();
    // Older last-known evidence no longer outranks a current warning.
    assert_eq!(ids, ["crit", "recent", "warn", "stale"]);
    let groups: Vec<_> = attention.rows.iter().map(|row| row.group).collect();
    assert_eq!(
        groups,
        [
            AttentionGroup::Failing,
            AttentionGroup::Warning,
            AttentionGroup::Warning,
            AttentionGroup::Unknown
        ]
    );
    assert_eq!(attention.details, ["1 problem", "2 problems", "1 problem"]);
    assert_eq!(
        attention.node_details["n"],
        ["0 problems", "2 problems", "1 problem"]
    );
    assert_eq!(attention.node_details["m"][0], "1 problem");
}

#[test]
fn pod_tones_agree_with_the_pods_page() {
    use freshkube_core::workloads::PodIssue;
    for issue in [
        PodIssue::CrashLoopBackOff,
        PodIssue::ImagePullBackOff,
        PodIssue::ErrImagePull,
        PodIssue::Pending,
        PodIssue::OOMKilled,
        PodIssue::Error,
        PodIssue::HighRestarts(6),
        PodIssue::Unknown("ContainerCreating".into()),
    ] {
        let tone = pod_issue_tone(&issue);
        // The Pods page draws the skull from the printed status, which
        // carries the same reason.
        assert_eq!(
            tone == Tone::Died,
            crate::resources::rows::died(issue.label()),
            "{issue:?}"
        );
        let expected = match issue {
            PodIssue::CrashLoopBackOff | PodIssue::OOMKilled | PodIssue::Error => Tone::Died,
            PodIssue::ImagePullBackOff | PodIssue::ErrImagePull => Tone::Crit,
            _ => Tone::Warn,
        };
        assert_eq!(tone, expected, "{issue:?}");
    }
}

#[test]
fn a_crash_looping_pod_draws_the_skull_among_the_failing() {
    let summary = example::summary("prod-fra", now().timestamp());
    let attention = build(&[], Some(&summary), None, now());
    let crashing: Vec<_> = attention
        .rows
        .iter()
        .filter(|row| row.kind == "Pod" && row.reason.as_ref() == "CrashLoopBackOff")
        .collect();
    assert!(!crashing.is_empty(), "the example has a crash-looping pod");
    for row in crashing {
        assert_eq!(row.tone, Tone::Died, "{}", row.name);
        assert_eq!(row.group, AttentionGroup::Failing, "{}", row.name);
    }
}

#[test]
fn died_counts_as_failing() {
    let attention = finish(vec![
        row("warn", "c", Tone::Warn, None, "n"),
        row("died", "a", Tone::Died, None, "n"),
        row("crit", "b", Tone::Crit, None, "n"),
    ]);
    let ids: Vec<_> = attention.rows.iter().map(|row| row.id.as_ref()).collect();
    assert_eq!(ids, ["died", "crit", "warn"]);
    assert_eq!(attention.details, ["2 problems", "1 problem", "0 problems"]);
}

#[test]
fn group_counts_include_rows_past_the_cap() {
    let mut rows = Vec::new();
    for ix in 0..60 {
        rows.push(row(&format!("crit-{ix:02}"), "a", Tone::Crit, None, "n"));
    }
    rows.push(row("stale", "b", Tone::Unknown, None, "n"));
    let attention = finish(rows);
    assert_eq!(attention.rows.len(), 50);
    assert!(
        attention
            .rows
            .iter()
            .all(|row| row.group == AttentionGroup::Failing)
    );
    assert_eq!(
        attention.details,
        ["60 problems", "0 problems", "1 problem"]
    );
}
