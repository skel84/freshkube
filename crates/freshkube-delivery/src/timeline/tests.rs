use super::example::{self, Shape};
use super::*;

const NOW: i64 = 1_800_000_000;

fn quiet_rows(timeline: &Timeline) -> Vec<(i64, i64, Vec<Source>, Vec<Source>)> {
    timeline
        .rows
        .iter()
        .filter_map(|row| match row {
            Row::Quiet {
                from,
                until,
                read,
                not_read,
                ..
            } => Some((*from, *until, read.clone(), not_read.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn entries_merge_newest_first_and_outside_the_window_are_dropped() {
    let timeline = derive(&example::reads(NOW, Shape::Busy), NOW, 3);
    let times: Vec<i64> = timeline
        .rows
        .iter()
        .filter(|row| matches!(row, Row::Entry(_)))
        .map(Row::at)
        .collect();
    assert!(times.windows(2).all(|pair| pair[0] >= pair[1]), "{times:?}");
    assert!(times.iter().all(|at| *at >= timeline.since));
    // The node went NotReady 4 h ago: outside 3 h, inside 6 h.
    assert_eq!(timeline.entries(), 8);
    let longer = derive(&example::reads(NOW, Shape::Busy), NOW, 6);
    assert_eq!(longer.entries(), 9);
}

#[test]
fn events_stop_where_the_cluster_stops_keeping_them() {
    let timeline = derive(&example::reads(NOW, Shape::Busy), NOW, 3);
    let marker = timeline
        .rows
        .iter()
        .find_map(|row| match row {
            Row::NotKept {
                source,
                from,
                until,
            } => Some((*source, *from, *until)),
            _ => None,
        })
        .expect("a row where events stop");
    assert_eq!(
        marker,
        (Source::Events, NOW - 3 * 3600, NOW - EVENT_RETENTION)
    );
    let chip = timeline
        .chips
        .iter()
        .find(|chip| chip.source == Source::Events)
        .unwrap();
    assert_eq!(
        chip.state,
        ChipState::Read {
            count: 2,
            kept_from: Some(NOW - EVENT_RETENTION),
            cut: None,
            state_only: false,
        }
    );
    // A window the retention covers needs no marker.
    let reads =
        vec![SourceRead::read(Source::Events, "c", NOW, Vec::new()).kept_from(NOW - 4 * 3600)];
    let covered = derive(&reads, NOW, 3);
    assert!(
        !covered
            .rows
            .iter()
            .any(|row| matches!(row, Row::NotKept { .. }))
    );
}

#[test]
fn time_before_the_retention_is_not_quiet_for_events() {
    let reads = vec![
        SourceRead::read(Source::Deploys, "c", NOW, Vec::new()),
        SourceRead::read(Source::Events, "c", NOW, Vec::new()).kept_from(NOW - 3600),
    ];
    let timeline = derive(&reads, NOW, 3);
    let quiet = quiet_rows(&timeline);
    // The newest hour is covered by both; the two before it by Deploys alone.
    assert_eq!(quiet.len(), 2);
    assert_eq!(quiet[0].2, vec![Source::Deploys, Source::Events]);
    assert_eq!(quiet[1].2, vec![Source::Deploys]);
    assert!(quiet[1].3.contains(&Source::Events));
    assert_eq!((quiet[1].0, quiet[1].1), (NOW - 3 * 3600, NOW - 3600));
}

#[test]
fn a_source_cut_by_a_cap_says_so_and_speaks_for_no_quiet_hour() {
    let reads = vec![
        SourceRead::read(Source::Deploys, "c", NOW, Vec::new()),
        SourceRead::read(Source::Events, "c", NOW, Vec::new()).cut("cut after 1000 events in shop"),
    ];
    let timeline = derive(&reads, NOW, 3);
    let chip = timeline
        .chips
        .iter()
        .find(|chip| chip.source == Source::Events)
        .unwrap();
    assert_eq!(
        chip.state,
        ChipState::Read {
            count: 0,
            kept_from: None,
            cut: Some("cut after 1000 events in shop".into()),
            state_only: false,
        }
    );
    // Every hour is quiet for Deploys alone; Events, which may hold what
    // the cap left out, is named as not read in them.
    let quiet = quiet_rows(&timeline);
    assert_eq!(quiet.len(), 1);
    assert_eq!(quiet[0].2, vec![Source::Deploys]);
    assert!(quiet[0].3.contains(&Source::Events));
}

#[test]
fn a_source_that_shows_only_the_present_state_speaks_for_no_quiet_hour() {
    // Only state-only sources read: nothing can say the hours were quiet.
    let reads = vec![
        SourceRead::read(Source::Restarts, "c", NOW, Vec::new()).state_only(),
        SourceRead::read(Source::Nodes, "c", NOW, Vec::new()).state_only(),
    ];
    let timeline = derive(&reads, NOW, 3);
    assert!(quiet_rows(&timeline).is_empty(), "{:?}", timeline.rows);
    // With a source that can, the row lists them apart.
    let reads = vec![
        SourceRead::read(Source::Events, "c", NOW, Vec::new()),
        SourceRead::read(Source::Restarts, "c", NOW, Vec::new()).state_only(),
        SourceRead::read(Source::Nodes, "c", NOW, Vec::new()).state_only(),
    ];
    let timeline = derive(&reads, NOW, 3);
    let rows: Vec<_> = timeline
        .rows
        .iter()
        .filter_map(|row| match row {
            Row::Quiet {
                read,
                state_only,
                not_read,
                ..
            } => Some((read.clone(), state_only.clone(), not_read.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, vec![Source::Events]);
    assert_eq!(rows[0].1, vec![Source::Restarts, Source::Nodes]);
    assert!(!rows[0].2.contains(&Source::Restarts));
}

#[test]
fn a_source_that_could_not_be_read_says_why_and_is_not_quiet() {
    let timeline = derive(&example::reads(NOW, Shape::Partial), NOW, 3);
    let state = |source| {
        timeline
            .chips
            .iter()
            .find(|chip| chip.source == source)
            .unwrap()
            .state
            .clone()
    };
    assert_eq!(
        state(Source::Incidents),
        ChipState::NotRead("refused (403)".into())
    );
    assert_eq!(
        state(Source::Syncs),
        ChipState::NotRead("Argo CD isn't installed".into())
    );
    assert!(timeline.rows.iter().all(|row| !matches!(
        row,
        Row::Entry(entry) if entry.source == Source::Incidents
    )));
    // A source never handed over is not asked, not quiet.
    let alone = derive(&[], NOW, 3);
    assert!(
        alone
            .chips
            .iter()
            .all(|chip| chip.state == ChipState::NotRead("not asked".into()))
    );
    assert!(alone.rows.is_empty(), "nothing read, so no quiet hour");
    let none = derive(&example::reads(NOW, Shape::NoneRead), NOW, 24);
    assert!(none.rows.is_empty());
}

#[test]
fn a_quiet_hour_names_every_source_it_read() {
    let timeline = derive(&example::reads(NOW, Shape::Quiet), NOW, 3);
    let quiet = quiet_rows(&timeline);
    // Events are kept for the last hour only, so the older two hours are one
    // row and the last hour another. Deploys, Restarts and Nodes show only
    // the present state and vouch for neither.
    assert_eq!(quiet.len(), 2);
    let sources = [Source::Promotions, Source::Syncs, Source::Incidents];
    assert_eq!(quiet[0].2, [&sources[..], &[Source::Events]].concat());
    assert!(quiet[0].3.is_empty());
    assert_eq!(quiet[1].0, NOW - 3 * 3600);
    assert_eq!(quiet[1].2, sources.to_vec());
    assert_eq!(quiet[1].3, vec![Source::Events]);
    assert_eq!(timeline.entries(), 0);
}

#[test]
fn hours_with_entries_are_not_quiet_and_neighbours_are_split_by_them() {
    let timeline = derive(&example::reads(NOW, Shape::Busy), NOW, 6);
    for (from, until, ..) in quiet_rows(&timeline) {
        let inside = timeline
            .rows
            .iter()
            .any(|row| matches!(row, Row::Entry(entry) if entry.at >= from && entry.at < until));
        assert!(!inside, "{from}..{until} holds an entry");
    }
}

#[test]
fn every_entry_says_where_and_when_it_was_read() {
    let timeline = derive(&example::reads(NOW, Shape::Busy), NOW, 24);
    for row in &timeline.rows {
        if let Row::Entry(entry) = row {
            assert!(!entry.read_from.is_empty());
            assert_eq!(entry.read_at, NOW);
        }
    }
    let ids: std::collections::HashSet<_> = timeline
        .rows
        .iter()
        .filter_map(|row| match row {
            Row::Entry(entry) => Some(entry.id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), timeline.entries());
}

mod live {
    use serde_json::{Value, json};

    use super::super::live::{self, Place, Workload};
    use super::super::*;
    use crate::fixtures::FixtureReader;
    use freshkube_core::resources::{Failure, FailureKind};
    use freshkube_core::workloads::WorkloadKind;

    const NOW: i64 = 1_800_000_000;

    fn stamp(ago: i64) -> String {
        chrono::DateTime::from_timestamp(NOW - ago, 0)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    fn place() -> Place {
        Place {
            cluster: "core-fra".into(),
            workloads: vec![(
                "shop".into(),
                vec![Workload::new("api", WorkloadKind::Deployment)],
            )],
        }
    }

    fn cluster() -> FixtureReader {
        let event = |name: &str, object: &str, kind: &str, ago: i64| -> Value {
            json!({"metadata": {"namespace": "shop", "name": format!("{name}.1")},
                "type": kind, "reason": "BackOff", "message": "Back-off restarting",
                "involvedObject": {"kind": object, "name": name},
                "lastTimestamp": stamp(ago), "count": 3})
        };
        let owned_by =
            |kind: &str, name: &str| json!([{"kind": kind, "name": name, "controller": true}]);
        FixtureReader::default()
            .with(
                "events",
                vec![
                    event("api-7d9f-x2k4", "Pod", "Warning", 300),
                    event("api-7d9f-x2k4", "Pod", "Normal", 200),
                    // A pod that is gone is still the ReplicaSet's.
                    event("api-7d9f-gone1", "Pod", "Warning", 700),
                    event("api", "Deployment", "Warning", 800),
                    event("api-gateway-5c4d-abcde", "Pod", "Warning", 100),
                    event("api-gateway", "Deployment", "Warning", 150),
                    event("api-7d9f-x2k4", "Pod", "Warning", 5 * 3600),
                ],
            )
            .with(
                "pods",
                vec![
                    json!({"metadata": {"namespace": "shop", "name": "api-7d9f-x2k4",
                        "ownerReferences": owned_by("ReplicaSet", "api-7d9f")},
                        "spec": {"nodeName": "worker-2"},
                        "status": {"containerStatuses": [{"name": "api", "restartCount": 5,
                            "lastState": {"terminated": {"reason": "OOMKilled", "finishedAt": stamp(400)}}}]}}),
                    json!({"metadata": {"namespace": "shop", "name": "api-gateway-5c4d-abcde",
                        "ownerReferences": owned_by("ReplicaSet", "api-gateway-5c4d")},
                        "spec": {"nodeName": "worker-3"},
                        "status": {"containerStatuses": [{"name": "gw", "restartCount": 2,
                            "lastState": {"terminated": {"reason": "Error", "finishedAt": stamp(300)}}}]}}),
                ],
            )
            .with(
                "replicasets",
                vec![
                    json!({"metadata": {"namespace": "shop", "name": "api-7d9f",
                        "creationTimestamp": stamp(1000),
                        "annotations": {"deployment.kubernetes.io/revision": "14"},
                        "ownerReferences": owned_by("Deployment", "api")}}),
                    json!({"metadata": {"namespace": "shop", "name": "api-gateway-5c4d",
                        "creationTimestamp": stamp(900),
                        "ownerReferences": owned_by("Deployment", "api-gateway")}}),
                ],
            )
            .with(
                "nodes",
                vec![
                    json!({"metadata": {"name": "worker-2"}, "status": {"conditions":
                        [{"type": "Ready", "status": "False", "lastTransitionTime": stamp(600)}]}}),
                    json!({"metadata": {"name": "worker-3"}, "status": {"conditions":
                        [{"type": "Ready", "status": "False", "lastTransitionTime": stamp(600)}]}}),
                ],
            )
    }

    fn titles(reads: &[SourceRead], source: Source) -> Vec<String> {
        reads
            .iter()
            .find(|read| read.source == source)
            .unwrap()
            .outcome
            .as_ref()
            .unwrap()
            .iter()
            .map(|item| item.title.clone())
            .collect()
    }

    #[tokio::test]
    async fn reads_only_the_applications_objects_in_the_window() {
        let reads = live::read(&cluster(), &place(), NOW, 3).await;
        // The Deployment's own and its pods', the gone one's too; not
        // api-gateway's, whose name starts the same.
        assert_eq!(
            titles(&reads, Source::Events),
            [
                "BackOff: api-7d9f-x2k4",
                "BackOff: api-7d9f-gone1",
                "BackOff: api"
            ]
        );
        assert_eq!(
            titles(&reads, Source::Restarts),
            ["api-7d9f-x2k4 restarted"]
        );
        assert_eq!(
            titles(&reads, Source::Deploys),
            ["deploy/api → revision 14"]
        );
        // Only the node the application's pod runs on.
        assert_eq!(titles(&reads, Source::Nodes), ["worker-2 NotReady"]);
        let events = reads.iter().find(|r| r.source == Source::Events).unwrap();
        // The oldest event read is older than the default retention.
        assert_eq!(events.kept_from, Some(NOW - 5 * 3600));
        assert!(!events.state_only);
        for source in [Source::Restarts, Source::Nodes, Source::Deploys] {
            assert!(
                reads
                    .iter()
                    .find(|r| r.source == source)
                    .unwrap()
                    .state_only
            );
        }
        assert_eq!(events.from, "core-fra");
    }

    #[tokio::test]
    async fn sources_not_read_yet_say_so_and_a_refused_list_leaves_the_others() {
        let refused = cluster().failing("events", Failure::new(FailureKind::Forbidden, "no"));
        let reads = live::read(&refused, &place(), NOW, 3).await;
        let outcome = |source| {
            reads
                .iter()
                .find(|r| r.source == source)
                .unwrap()
                .outcome
                .clone()
        };
        assert_eq!(
            outcome(Source::Events),
            Err("Forbidden: can't list events".into())
        );
        assert!(outcome(Source::Restarts).is_ok());
        for source in [Source::Promotions, Source::Syncs, Source::Incidents] {
            assert_eq!(outcome(source), Err(live::NOT_YET.into()));
        }
        let timeline = derive(&reads, NOW, 3);
        assert!(timeline.chips.iter().any(
            |chip| chip.source == Source::Events && matches!(chip.state, ChipState::NotRead(_))
        ));
    }

    #[tokio::test]
    async fn a_workload_does_not_claim_a_neighbour_whose_name_starts_the_same() {
        let reads = live::read(&cluster(), &place(), NOW, 3).await;
        for source in [
            Source::Events,
            Source::Restarts,
            Source::Deploys,
            Source::Nodes,
        ] {
            let titles = titles(&reads, source);
            assert!(
                titles
                    .iter()
                    .all(|title| !title.contains("gateway") && !title.contains("worker-3")),
                "{source:?}: {titles:?}"
            );
        }
    }

    #[tokio::test]
    async fn retention_is_the_default_unless_an_older_event_shows_more() {
        let fresh = FixtureReader::default()
            .with("events", Vec::new())
            .with("pods", Vec::new())
            .with("replicasets", Vec::new())
            .with("nodes", Vec::new());
        let reads = live::read(&fresh, &place(), NOW, 3).await;
        let events = reads.iter().find(|r| r.source == Source::Events).unwrap();
        assert_eq!(events.kept_from, Some(NOW - EVENT_RETENTION));
    }

    #[tokio::test]
    async fn pods_that_could_not_be_listed_leave_the_nodes_cut() {
        let refused = cluster().failing("pods", Failure::new(FailureKind::Forbidden, "no"));
        let reads = live::read(&refused, &place(), NOW, 3).await;
        let nodes = reads.iter().find(|r| r.source == Source::Nodes).unwrap();
        assert!(nodes.outcome.is_ok());
        assert_eq!(
            nodes.cut.as_deref(),
            Some("hosts unknown: pods not listed in shop")
        );
        let restarts = reads.iter().find(|r| r.source == Source::Restarts).unwrap();
        assert_eq!(restarts.outcome, Err("Forbidden: can't list pods".into()));
    }

    #[tokio::test]
    async fn a_listing_cut_at_the_cap_is_read_in_part() {
        let capped = cluster().capped("events");
        let reads = live::read(&capped, &place(), NOW, 3).await;
        let events = reads.iter().find(|r| r.source == Source::Events).unwrap();
        assert!(events.outcome.is_ok());
        assert!(
            events
                .cut
                .as_deref()
                .is_some_and(|cut| cut.contains("events in shop")),
            "{:?}",
            events.cut
        );
        let others = reads.iter().find(|r| r.source == Source::Restarts).unwrap();
        assert_eq!(others.cut, None);
        // No quiet hour is drawn for the part that was cut.
        let timeline = derive(&reads, NOW, 3);
        for row in &timeline.rows {
            if let Row::Quiet { read, .. } = row {
                assert!(!read.contains(&Source::Events), "{row:?}");
            }
        }
    }

    #[tokio::test]
    async fn namespaces_past_the_cap_cut_every_source_that_reads_by_namespace() {
        let many = Place {
            cluster: "core-fra".into(),
            workloads: (0..live::MAX_NAMESPACES + 2)
                .map(|ix| {
                    (
                        format!("shop-{ix}"),
                        vec![Workload::new("api", WorkloadKind::Deployment)],
                    )
                })
                .collect(),
        };
        let reads = live::read(&cluster(), &many, NOW, 3).await;
        for source in [
            Source::Deploys,
            Source::Events,
            Source::Restarts,
            Source::Nodes,
        ] {
            let read = reads.iter().find(|r| r.source == source).unwrap();
            assert_eq!(
                read.cut.as_deref(),
                Some("only the first 5 of 7 namespaces were read"),
                "{source:?}"
            );
        }
    }

    #[tokio::test]
    async fn an_application_with_no_workload_here_reads_nothing_and_asks_nothing() {
        let reader = cluster();
        let nowhere = Place {
            cluster: "core-fra".into(),
            workloads: Vec::new(),
        };
        let reads = live::read(&reader, &nowhere, NOW, 3).await;
        assert!(reads.iter().all(|read| read.outcome.is_err()));
        assert!(
            !reader
                .requests
                .borrow()
                .iter()
                .any(|r| r.contains("events") || r.contains("pods")),
            "{:?}",
            reader.requests.borrow()
        );
    }
}

#[test]
fn times_and_spans_are_printed_in_words() {
    assert_eq!(clock(0), "00:00");
    assert_eq!(clock(14 * 3600 + 2 * 60 + 59), "14:02");
    assert_eq!(span_words(900), "15 min");
    assert_eq!(span_words(3600), "1 h");
    assert_eq!(span_words(5400), "1 h 30 min");
    assert_eq!(span_words(-5), "0 min");
}
