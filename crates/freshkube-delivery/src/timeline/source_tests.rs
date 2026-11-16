//! Slice 3's sources: Kargo Promotions read from the cluster, and Coroot's
//! incidents from the sample the page hands over.
use std::time::Duration;

use freshkube_core::coroot::{Incident, ReadError};
use freshkube_core::resources::{Failure, FailureKind};
use freshkube_core::workloads::WorkloadKind;
use serde_json::{Value, json};

use super::incidents::{self, Absent};
use super::live::{self, Place, Workload};
use super::*;
use crate::fixtures::FixtureReader;

const NOW: i64 = 1_800_000_000;

fn stamp(ago: i64) -> String {
    chrono::DateTime::from_timestamp(NOW - ago, 0)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn promotion(name: &str, phase: &str, ago: i64) -> Value {
    json!({"metadata": {"name": name, "namespace": "shop-ci", "creationTimestamp": stamp(ago + 60)},
        "spec": {"stage": "prod", "freight": "0123456789abcdef0123456789abcdef"},
        "status": {"phase": phase, "message": "step failed, see https://git.example.com/shop/repo", "finishedAt": stamp(ago)}})
}

fn running(name: &str, started_ago: i64) -> Value {
    json!({"metadata": {"name": name, "namespace": "shop-ci", "creationTimestamp": stamp(started_ago)},
        "spec": {"stage": "prod", "freight": "feedface"}, "status": {"phase": "Running"}})
}

fn project() -> Place {
    Place {
        cluster: "core-fra".into(),
        kargo: vec!["shop-ci".into()],
        ..Default::default()
    }
}

fn kargo(items: Vec<Value>) -> FixtureReader {
    FixtureReader::default()
        .serves("kargo.akuity.io", "v1alpha1", &["promotions"])
        .with("promotions", items)
}

fn promotions(reads: &[SourceRead]) -> &SourceRead {
    reads
        .iter()
        .find(|r| r.source == Source::Promotions)
        .unwrap()
}

#[tokio::test]
async fn promotions_inside_the_window_are_listed_and_running_ones_whenever_they_began() {
    let reader = kargo(vec![
        promotion("a", "Succeeded", 600),
        promotion("b", "Failed", 1200),
        promotion("old", "Succeeded", 5 * 3600),
        running("c", 6 * 3600),
    ]);
    let reads = live::read(&reader, &project(), NOW, 3).await;
    let read = promotions(&reads);
    let items = read.outcome.as_ref().unwrap();
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Promotion Succeeded: prod",
            "Promotion Failed: prod",
            "Promotion Running: prod"
        ]
    );
    assert_eq!(items[1].severity, Severity::Bad);
    assert!(
        items[1].detail.contains("freight 0123456789ab"),
        "{}",
        items[1].detail
    );
    assert!(
        !items[1].detail.contains("example.com"),
        "{}",
        items[1].detail
    );
    // A running one that began before the window sits at its start.
    assert_eq!(items[2].at, NOW - 3 * 3600);
    assert!(items[2].detail.contains("started before this window"));
    assert!(read.cut.is_none() && !read.state_only);
}

#[tokio::test]
async fn a_fully_read_promotions_listing_names_itself_in_a_quiet_hour() {
    let reads = live::read(&kargo(Vec::new()), &project(), NOW, 3).await;
    let timeline = derive(&reads, NOW, 3);
    let quiet = timeline.rows.iter().find_map(|row| match row {
        Row::Quiet { read, .. } => Some(read.clone()),
        _ => None,
    });
    assert_eq!(quiet, Some(vec![Source::Promotions]));
}

#[tokio::test]
async fn what_the_promotions_read_did_not_see_is_said_and_speaks_for_no_hour() {
    let reader = kargo(vec![promotion("a", "Succeeded", 60)]);
    let elsewhere = Place {
        kargo_elsewhere: 2,
        ..project()
    };
    let reads = live::read(&reader, &elsewhere, NOW, 3).await;
    assert_eq!(
        promotions(&reads).cut.as_deref(),
        Some("2 Kargo Projects are read through another connection")
    );
    let one = Place {
        kargo_elsewhere: 1,
        ..project()
    };
    let reads = live::read(&reader, &one, NOW, 3).await;
    assert_eq!(
        promotions(&reads).cut.as_deref(),
        Some("1 Kargo Project is read through another connection")
    );
    let capped = live::read(&kargo(Vec::new()).capped("promotions"), &project(), NOW, 3).await;
    assert!(
        promotions(&capped)
            .cut
            .as_deref()
            .unwrap()
            .starts_with("cut after")
    );
    let many = Place {
        kargo: (0..7).map(|ix| format!("p{ix}")).collect(),
        ..project()
    };
    let reads = live::read(&reader, &many, NOW, 3).await;
    assert!(
        promotions(&reads)
            .cut
            .as_deref()
            .unwrap()
            .contains("first 5 of 7 Kargo Projects")
    );
    // A cut source vouches for no hour.
    let timeline = derive(&capped, NOW, 3);
    assert!(
        timeline
            .rows
            .iter()
            .all(|row| !matches!(row, Row::Quiet { .. }))
    );
    assert!(matches!(
        timeline.chips[0].state,
        ChipState::Read { cut: Some(_), .. }
    ));
}

#[tokio::test]
async fn promotions_not_read_say_why() {
    let why = |reads: Vec<SourceRead>| promotions(&reads).outcome.clone().unwrap_err();
    let reader = kargo(Vec::new());
    let none = Place {
        kargo: Vec::new(),
        ..project()
    };
    assert_eq!(
        why(live::read(&reader, &none, NOW, 3).await),
        "no Kargo Project belongs to it"
    );
    let away = Place {
        kargo: Vec::new(),
        kargo_elsewhere: 1,
        ..project()
    };
    assert_eq!(
        why(live::read(&reader, &away, NOW, 3).await),
        "its Kargo Project is read through another connection"
    );
    assert_eq!(
        why(live::read(
            &kargo(Vec::new()).refusing("promotions"),
            &project(),
            NOW,
            3
        )
        .await),
        "Forbidden: can't list Promotions"
    );
    assert_eq!(
        why(live::read(&FixtureReader::default(), &project(), NOW, 3).await),
        "Kargo's Promotions are not served here"
    );
    let timed_out =
        kargo(Vec::new()).failing("promotions", Failure::new(FailureKind::Timeout, "slow"));
    let text = why(live::read(&timed_out, &project(), NOW, 3).await);
    assert!(
        text.starts_with("can't list Promotions: ") && text.contains("timed out"),
        "{text}"
    );
}

#[tokio::test]
async fn one_project_failing_leaves_the_others_standing() {
    let two = Place {
        kargo: vec!["shop-ci".into(), "shop-eu".into()],
        ..project()
    };
    let reader = kargo(vec![promotion("a", "Succeeded", 60)]).failing_in(
        "promotions",
        "shop-eu",
        Failure::new(FailureKind::Forbidden, "no"),
    );
    let reads = live::read(&reader, &two, NOW, 3).await;
    let read = promotions(&reads);
    assert_eq!(read.outcome.as_ref().unwrap().len(), 1);
    assert!(
        read.cut
            .as_deref()
            .unwrap()
            .starts_with("shop-eu not read: ")
    );
    let _ = Duration::ZERO;
}

fn incident(
    app: &str,
    severity: &str,
    state: &str,
    opened_ago: i64,
    resolved_ago: Option<i64>,
) -> Incident {
    let at = |ago: i64| stamp(ago);
    serde_json::from_value(json!({
        "key": format!("inc-{opened_ago}"), "app": app, "cluster": "core-fra",
        "severity": severity, "state": state,
        "opened_at": at(opened_ago), "resolved_at": resolved_ago.map(at),
        "duration_seconds": opened_ago - resolved_ago.unwrap_or(0),
        "impact_percent": 12.5, "description": "high latency"
    }))
    .unwrap()
}

fn workloads() -> Vec<(String, Vec<Workload>)> {
    vec![(
        "shop".into(),
        vec![Workload::new("api", WorkloadKind::Deployment)],
    )]
}

fn read_incidents(sample: Result<Vec<Incident>, Absent>) -> SourceRead {
    incidents::read("core-fra", "c1", &workloads(), sample, NOW, 3)
}

#[test]
fn incidents_are_the_applications_by_their_coroot_id_and_overlap_the_window() {
    let read = read_incidents(Ok(vec![
        incident(
            "c1:shop:Deployment:api",
            "critical",
            "resolved",
            1800,
            Some(600),
        ),
        incident("c1:shop:Deployment:api", "warning", "open", 5 * 3600, None),
        incident(
            "c1:shop:Deployment:api",
            "warning",
            "resolved",
            6 * 3600,
            Some(5 * 3600),
        ),
        incident("c1:shop:Deployment:other", "critical", "open", 60, None),
        incident("c2:shop:Deployment:api", "critical", "open", 60, None),
        incident("c1:shop:StatefulSet:api", "critical", "open", 60, None),
    ]));
    let items = read.outcome.as_ref().unwrap();
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(items[0].title, "Incident Critical: api");
    assert!(
        items[0].detail.contains("resolved after 20 min"),
        "{}",
        items[0].detail
    );
    assert_eq!(items[1].at, NOW - 3 * 3600);
    assert!(items[1].detail.contains("started before this window"));
    assert!(items[1].detail.contains("open"));
    // Coroot's list is a sample, never the window's history.
    assert!(read.state_only && read.cut.is_none());
    let timeline = derive(&[read], NOW, 3);
    assert!(
        timeline
            .rows
            .iter()
            .all(|row| !matches!(row, Row::Quiet { .. }))
    );
}

#[test]
fn a_full_sample_is_cut_and_incidents_without_a_start_are_counted() {
    let mut list = vec![incident("c1:shop:Deployment:api", "warning", "open", 60, None); 100];
    list[0].opened_at = None;
    let read = read_incidents(Ok(list));
    let cut = read.cut.unwrap();
    assert!(
        cut.contains("latest 100") && cut.contains("1 without a start time"),
        "{cut}"
    );
}

#[test]
fn incidents_not_read_say_why() {
    let why = |absent| read_incidents(Err(absent)).outcome.unwrap_err();
    assert_eq!(why(Absent::NotConnected), "Coroot isn't connected");
    assert_eq!(
        why(Absent::NotLinked),
        "the cluster isn't linked to a Coroot cluster"
    );
    assert_eq!(
        why(Absent::NoProject),
        "no Coroot project is named for the cluster"
    );
    assert_eq!(
        why(Absent::Failed(ReadError::Refused)),
        "Forbidden: Coroot refused the incidents"
    );
    assert_eq!(
        why(Absent::Failed(ReadError::Unreachable)),
        "Unreachable: Coroot didn't answer"
    );
    assert!(why(Absent::Failed(ReadError::Timeout)).starts_with("Timed out"));
    assert_eq!(why(Absent::Other("stopped".into())), "stopped");
}

/// Everything slice 3 asks a cluster is a GET or a list.
#[tokio::test]
async fn promotions_ask_the_api_only_to_read() {
    use freshkube_core::fake_api::{self, json};
    use http::Method;
    let (client, recorder) = fake_api::client(|request| {
        if request.path == "/apis" {
            let version =
                json!({"groupVersion": "kargo.akuity.io/v1alpha1", "version": "v1alpha1"});
            return Some(json(
                200,
                json!({"kind": "APIGroupList", "apiVersion": "v1",
                "groups": [{"name": "kargo.akuity.io", "versions": [version], "preferredVersion": version}]}),
            ));
        }
        if request.path == "/apis/kargo.akuity.io/v1alpha1" {
            return Some(json(
                200,
                json!({"kind": "APIResourceList", "groupVersion": "kargo.akuity.io/v1alpha1",
                "resources": [{"name": "promotions", "namespaced": true, "kind": "Promotion", "verbs": ["get", "list"]}]}),
            ));
        }
        if request.path.ends_with("/promotions") {
            return Some(json(
                200,
                json!({"apiVersion": "kargo.akuity.io/v1alpha1", "kind": "PromotionList",
                "metadata": {}, "items": [promotion("a", "Succeeded", 60)]}),
            ));
        }
        None
    });
    let reader = crate::entry::reader(client);
    let reads = live::read(&reader, &project(), NOW, 3).await;
    assert_eq!(
        promotions(&reads).outcome.as_ref().map(Vec::len),
        Ok(1),
        "{:?}",
        recorder.lines()
    );
    assert!(!recorder.requests().is_empty());
    recorder.assert_all("the timeline only reads", |request| {
        request.method == Method::GET
    });
    assert!(
        recorder
            .lines()
            .iter()
            .any(|line| line.contains("/namespaces/shop-ci/promotions")),
        "{:?}",
        recorder.lines()
    );
}
