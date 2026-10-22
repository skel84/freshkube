//! The Stage actions on core's recording fake API: what each writes, the
//! confirmation each level asks, and that a blocked action asks the cluster
//! nothing. Kargo's objects are invented and the fake is in memory.

use std::time::Duration;

use freshkube_core::write::testing::Cluster;
use freshkube_core::write::{
    ANNOTATION_NOTICE, ActionDescription, AuditStore, Block, Confirmation, Destination, Dialog,
    Headline, NotSent, Phase, Protection, Runner, Status, Tier, Verdict, Who,
};
use serde_json::{Value, json};

use super::*;
use crate::change::example::{FREIGHT, PROJECT, example_acts};

const NS: &str = "/apis/kargo.akuity.io/v1alpha1/namespaces/checkout";

fn kargo() -> Cluster {
    let object = |kind: &str, name: &str| {
        json!({"apiVersion": "kargo.akuity.io/v1alpha1", "kind": kind,
            "metadata": {"name": name, "namespace": PROJECT,
                "uid": format!("uid-{}-{name}", kind.to_lowercase()), "resourceVersion": "100"},
            "status": {}})
    };
    Cluster::with(vec![
        (format!("{NS}/stages/prod-ams"), object("Stage", "prod-ams")),
        (
            format!("{NS}/warehouses/checkout-images"),
            object("Warehouse", "checkout-images"),
        ),
        (
            format!("{NS}/promotions/prod-ams.01k8d2qx5f8c3h6j9m2n4p7r0t.wonky-o"),
            object("Promotion", "prod-ams.01k8d2qx5f8c3h6j9m2n4p7r0t.wonky-o"),
        ),
    ])
}

fn kargo_runner(cluster: &Cluster, dir: &std::path::Path, level: Protection) -> Runner {
    Runner::new(
        cluster.client.clone(),
        AuditStore::new(dir.join("audit")),
        Who {
            entry: "cicd-fra".into(),
            context: "cicd-fra".into(),
            protection: level,
            actor: "tester".into(),
        },
    )
}

/// A destination cluster at `level`, as a caller describes one.
fn dest(entry: &str, level: Protection) -> Destination {
    Destination {
        entry: entry.into(),
        protection: level,
        cell: None,
        covers: vec![],
    }
}

fn acts() -> StageActs {
    example_acts("prod-ams")
}

fn change() -> change::Change {
    crate::change::example::for_stage(PROJECT, "prod-ams", chrono::Utc::now()).unwrap()
}

fn running() -> Promoting {
    Promoting {
        promotion: Handle {
            name: "prod-ams.01k8d2qx5f8c3h6j9m2n4p7r0t.wonky-o".into(),
            uid: "uid-promotion-prod-ams.01k8d2qx5f8c3h6j9m2n4p7r0t.wonky-o".into(),
            resource_version: "100".into(),
        },
        freight: FREIGHT.into(),
        phase: "Running".into(),
    }
}

fn promote_action(level: Protection) -> ActionDescription {
    let acts = acts();
    promote(
        &change(),
        &[Pick {
            acts: &acts,
            destination: Some(dest("prod-ams", level)),
        }],
        "01k8d2qy4v9z3m6n7p8r5t0w1x",
    )
}

fn phases(dir: &std::path::Path) -> Vec<Phase> {
    AuditStore::new(dir.join("audit"))
        .read()
        .unwrap()
        .records
        .iter()
        .map(|r| r.phase)
        .collect()
}

fn writes(cluster: &Cluster) -> Vec<freshkube_core::fake_api::Recorded> {
    cluster.writes()
}

#[tokio::test(start_paused = true)]
async fn refresh_sets_the_annotation_asks_nothing_at_any_writable_level_and_is_audited() {
    for level in [Protection::Protected, Protection::Guarded, Protection::Open] {
        let (cluster, dir) = (kargo(), tempfile::tempdir().unwrap());
        let runner = kargo_runner(&cluster, dir.path(), level);
        let verdict = runner
            .preflight(refresh_stage(&acts(), "2026-10-22T10:00:00Z"))
            .await;
        assert_eq!(
            Dialog::for_verdict(&verdict, "cicd-fra"),
            Dialog::SendWithoutAsking {
                notice: ANNOTATION_NOTICE
            },
            "{level:?}"
        );
        let Verdict::Ready(prepared) = verdict else {
            panic!("{level:?}")
        };
        runner
            .execute(*prepared, Confirmation::NotAsked, || false, |_| {})
            .await;
        let sent = writes(&cluster);
        assert_eq!(sent.len(), 1, "{level:?}: {sent:?}");
        assert_eq!(sent[0].method, "PATCH");
        assert!(sent[0].path.ends_with("/stages/prod-ams"));
        let body: Value = serde_json::from_str(&sent[0].body).unwrap();
        assert_eq!(
            body["metadata"]["annotations"][REFRESH],
            "2026-10-22T10:00:00Z"
        );
        assert_eq!(
            phases(dir.path())[..2],
            [Phase::Sending, Phase::Accepted],
            "{level:?}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn refresh_of_the_warehouse_and_reverify_go_on_their_own_objects() {
    let (cluster, dir) = (kargo(), tempfile::tempdir().unwrap());
    let runner = kargo_runner(&cluster, dir.path(), Protection::Guarded);
    let mut stage = acts();
    stage.verification = Some("ver-prod-ams-1".into());
    for (action, path, key, value) in [
        (
            refresh_warehouse(&stage, "t1").unwrap(),
            "/warehouses/checkout-images",
            REFRESH,
            "t1",
        ),
        (
            reverify(&stage).unwrap(),
            "/stages/prod-ams",
            REVERIFY,
            "ver-prod-ams-1",
        ),
    ] {
        let Verdict::Ready(prepared) = runner.preflight(action).await else {
            panic!("{key}")
        };
        runner
            .execute(*prepared, Confirmation::NotAsked, || false, |_| {})
            .await;
        let sent = writes(&cluster);
        let last = sent.last().unwrap();
        assert!(last.path.ends_with(path), "{}", last.path);
        let body: Value = serde_json::from_str(&last.body).unwrap();
        assert_eq!(body["metadata"]["annotations"][key], value);
    }
    // Without the verification id there is nothing to repeat.
    stage.verification = None;
    assert!(reverify(&stage).is_none());
    stage.warehouse = None;
    assert!(refresh_warehouse(&stage, "t").is_none());
}

#[tokio::test(start_paused = true)]
async fn read_only_blocks_every_stage_action_and_asks_the_cluster_nothing() {
    let stage = acts();
    let all = [
        refresh_stage(&stage, "t"),
        refresh_warehouse(&stage, "t").unwrap(),
        reverify(&example_acts("dev")).unwrap(),
        promote_action(Protection::Guarded),
        abort(&stage, &running(), &change()),
    ];
    for action in all {
        let (cluster, dir) = (kargo(), tempfile::tempdir().unwrap());
        let runner = kargo_runner(&cluster, dir.path(), Protection::ReadOnly);
        let kind = action.kind.clone();
        assert!(
            matches!(
                runner.preflight(action).await,
                Verdict::Blocked(Block::ReadOnly(_))
            ),
            "{kind}"
        );
        assert!(cluster.recorder.requests().is_empty(), "{kind}");
        assert!(phases(dir.path()).is_empty(), "{kind}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_read_only_destination_blocks_a_promotion_the_kargo_cluster_would_allow() {
    let (cluster, dir) = (kargo(), tempfile::tempdir().unwrap());
    let runner = kargo_runner(&cluster, dir.path(), Protection::Open);
    assert!(matches!(
        runner.preflight(promote_action(Protection::ReadOnly)).await,
        Verdict::Blocked(Block::ReadOnly(_))
    ));
    assert!(cluster.recorder.requests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn promote_creates_a_promotion_and_the_dialog_names_the_destination_and_the_preview() {
    let (cluster, dir) = (kargo(), tempfile::tempdir().unwrap());
    let runner = kargo_runner(&cluster, dir.path(), Protection::Guarded);
    let verdict = runner.preflight(promote_action(Protection::Guarded)).await;
    // No GET of the Promotion that doesn't exist yet; nothing written.
    cluster.recorder.assert_lines(&[
        "POST /apis/authorization.k8s.io/v1/selfsubjectaccessreviews",
        &format!("POST {NS}/promotions?&dryRun=All&fieldManager=freshkube"),
    ]);
    let Dialog::Ask(ask) = Dialog::for_verdict(&verdict, "cicd-fra") else {
        panic!("asks");
    };
    assert_eq!(ask.type_name, None, "guarded shows the preview, no typing");
    assert_eq!(ask.destinations, [("prod-ams".into(), Protection::Guarded)]);
    let titles: Vec<&str> = ask.preview.iter().map(|p| p.title.as_str()).collect();
    assert!(titles.contains(&"Stage prod-ams"), "{titles:?}");
    assert!(titles.contains(&"This change") && titles.contains(&"Going back"));
    let lines = ask.preview[0].lines.join("\n");
    assert!(lines.contains("Runs now: Freight brave-l"), "{lines}");
    assert!(lines.contains("Would run: Freight wonky-o"), "{lines}");
    assert!(lines.contains("Image registry.example.test"), "{lines}");
    let Verdict::Ready(prepared) = verdict else {
        panic!()
    };
    let finished = runner
        .execute(*prepared, Confirmation::Confirmed, || false, |_| {})
        .await;
    assert_ne!(finished.headline(), Headline::Refused);
    let sent = writes(&cluster);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].method, "POST");
    let body: Value = serde_json::from_str(&sent[0].body).unwrap();
    assert_eq!(body["kind"], "Promotion");
    assert_eq!(
        body["spec"],
        json!({"stage": "prod-ams", "freight": FREIGHT})
    );
    assert_eq!(
        body["metadata"]["name"],
        "prod-ams.01k8d2qy4v9z3m6n7p8r5t0w1x.wonky-o"
    );
    assert!(!sent[0].body.contains("resourceVersion"));
    // The promotion is audited as a spec change and offers no Undo.
    assert!(finished.undo.is_none());
    let records = AuditStore::new(dir.path().join("audit"))
        .read()
        .unwrap()
        .records;
    assert_eq!(records[0].tier, "spec-change");
    assert_eq!(records[0].action, "promote");
    assert_eq!(promote_action(Protection::Guarded).tier, Tier::SpecChange);
}

#[tokio::test(start_paused = true)]
async fn a_protected_destination_asks_for_the_stage_s_name_though_kargo_s_cluster_is_guarded() {
    let (cluster, dir) = (kargo(), tempfile::tempdir().unwrap());
    let runner = kargo_runner(&cluster, dir.path(), Protection::Guarded);
    let verdict = runner
        .preflight(promote_action(Protection::Protected))
        .await;
    let Dialog::Ask(ask) = Dialog::for_verdict(&verdict, "cicd-fra") else {
        panic!("asks");
    };
    assert_eq!(ask.type_name.as_deref(), Some("prod-ams"));
    let Verdict::Ready(prepared) = verdict else {
        panic!()
    };
    // The Promotion's generated name, or the cluster's id, unlocks nothing.
    let finished = runner
        .execute(
            *prepared,
            Confirmation::Typed("prod-ams.01k8d2qy4v9z3m6n7p8r5t0w1x.wonky-o".into()),
            || false,
            |_| {},
        )
        .await;
    assert_eq!(finished.status, Status::NotSent(NotSent::Unconfirmed));
    assert!(writes(&cluster).is_empty());
    // And a protected Kargo cluster asks for it with an open destination.
    let (cluster, dir) = (kargo(), tempfile::tempdir().unwrap());
    let protected = kargo_runner(&cluster, dir.path(), Protection::Protected);
    let verdict = protected.preflight(promote_action(Protection::Open)).await;
    let Dialog::Ask(ask) = Dialog::for_verdict(&verdict, "cicd-fra") else {
        panic!("asks");
    };
    assert_eq!(ask.type_name.as_deref(), Some("prod-ams"));
}

#[tokio::test(start_paused = true)]
async fn several_stages_type_once_and_the_dialog_names_the_protected_ones() {
    // A create reads nothing, so the cluster needs no objects.
    let (cluster, dir) = (Cluster::with(vec![]), tempfile::tempdir().unwrap());
    let runner = kargo_runner(&cluster, dir.path(), Protection::Guarded);
    let (dev, prod) = (example_acts("dev"), example_acts("prod-ams"));
    let action = promote(
        &change(),
        &[
            Pick {
                acts: &dev,
                destination: Some(dest("dev-fra", Protection::Open)),
            },
            Pick {
                acts: &prod,
                destination: Some(dest("prod-ams", Protection::Protected)),
            },
        ],
        "01k8d2qy4v9z3m6n7p8r5t0w1x",
    );
    assert_eq!(action.subjects, ["dev", "prod-ams"]);
    let verdict = runner.preflight(action).await;
    let Dialog::Ask(ask) = Dialog::for_verdict(&verdict, "cicd-fra") else {
        panic!("asks");
    };
    // Once, for the cluster's id; the protected Stage is named.
    assert_eq!(ask.type_name.as_deref(), Some("cicd-fra"));
    assert_eq!(ask.protected, ["prod-ams"]);
    assert_eq!(ask.subjects, ["dev", "prod-ams"]);
    assert_eq!(ask.destinations.len(), 2);
}

#[test]
fn promote_becomes_abort_while_any_promotion_runs_whoever_made_it() {
    let mut stage = acts();
    assert_eq!(stage.slot(), Slot::Promote);
    stage.holds = true;
    assert_eq!(
        stage.slot(),
        Slot::Not("The Stage already runs this Freight.")
    );
    stage.promoting = vec![running()];
    assert!(matches!(stage.slot(), Slot::Abort(r) if r.phase == "Running"));
    stage.holds = false;
    assert!(matches!(stage.slot(), Slot::Abort(_)));
}

#[tokio::test(start_paused = true)]
async fn abort_marks_the_promotion_counts_down_and_says_what_already_ran() {
    let stage = acts();
    let action = abort(&stage, &running(), &change());
    assert_eq!(action.tier, Tier::Destructive);
    let (cluster, dir) = (kargo(), tempfile::tempdir().unwrap());
    let runner = kargo_runner(&cluster, dir.path(), Protection::Protected);
    let verdict = runner.preflight(action.clone()).await;
    let Dialog::Ask(ask) = Dialog::for_verdict(&verdict, "cicd-fra") else {
        panic!("asks");
    };
    assert_eq!(ask.type_name.as_deref(), Some("prod-ams"));
    assert_eq!(ask.cancel_window, Some(Duration::from_secs(5)));
    assert_eq!(
        ask.impact,
        ["Promotion/prod-ams.01k8d2qx5f8c3h6j9m2n4p7r0t.wonky-o"]
    );
    let text: String = ask.preview.iter().flat_map(|p| p.lines.clone()).collect();
    assert!(
        text.contains("already ran") || text.contains("stays"),
        "{text}"
    );
    assert!(text.contains("the next sync deploys it"));
    let Verdict::Ready(prepared) = verdict else {
        panic!()
    };
    let finished = runner
        .execute(
            *prepared,
            Confirmation::Typed("prod-ams".into()),
            || false,
            |_| {},
        )
        .await;
    assert!(matches!(finished.status, Status::Sent));
    let sent = writes(&cluster);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(
        sent[0]
            .path
            .ends_with("/promotions/prod-ams.01k8d2qx5f8c3h6j9m2n4p7r0t.wonky-o")
    );
    let body: Value = serde_json::from_str(&sent[0].body).unwrap();
    assert_eq!(
        body["metadata"]["annotations"][ABORT],
        r#"{"action":"terminate"}"#
    );
}

#[test]
fn a_promotion_name_has_a_lowercase_ulid_that_sorts_by_time_as_kargo_s_do() {
    let later = new_ulid(chrono::DateTime::from_timestamp_millis(1_792_000_000_000).unwrap());
    let earlier = new_ulid(chrono::DateTime::from_timestamp_millis(1_791_999_999_000).unwrap());
    for id in [&later, &earlier] {
        assert_eq!(id.len(), 26);
        assert!(
            id.bytes()
                .all(|b| b"0123456789abcdefghjkmnpqrstvwxyz".contains(&b)),
            "{id}"
        );
    }
    assert!(later > earlier, "{later} should sort after {earlier}");
    // The first digits are the time alone, whatever the random part is.
    assert_eq!(ulid(1, 0)[..10], ulid(1, u128::MAX)[..10]);
    assert_eq!(ulid(0, 0), "0".repeat(26));
    // Two made in one instant differ, so two presses never collide.
    let now = chrono::Utc::now();
    assert_ne!(new_ulid(now), new_ulid(now));
    // And a Promotion made now sorts after one Kargo made a minute ago.
    let kargo = format!(
        "prod-ams.{}.wonky-o",
        new_ulid(now - chrono::Duration::minutes(1))
    );
    let ours = promote_action(Protection::Guarded);
    let name = &ours.changes[0].target.name;
    assert!(
        name.split('.').nth(1).is_some_and(|u| u.len() == 26),
        "{name}"
    );
    let ours_now = format!("prod-ams.{}.wonky-o", new_ulid(now));
    assert!(ours_now > kargo);
}
