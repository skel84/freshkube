//! Freight -> Warehouse, and a Stage's verification as a note on
//! Freight -> Stage. Invented names throughout.

use serde_json::{Value, json};

use super::fixtures::*;
use super::join::{Confidence, Hop, Key, Link};
use super::kargo::{parse_stage, parse_warehouse};
use super::tests::{ENV, link, one, run};

async fn warehouse_link(world: &World) -> Link {
    let trail = run(world, &ENV).await;
    link(&trail, Hop::Freight, Hop::Warehouse)
        .into_iter()
        .find(|l| l.subject == "storefront/f-new")
        .expect("the new Freight's Warehouse link")
        .clone()
}

fn with_warehouse(value: Value) -> World {
    let mut world = healthy();
    world.kargo = world.kargo.with("warehouses", vec![value]);
    world
}

// ---- Freight -> Warehouse ----------------------------------------------

#[tokio::test]
async fn a_warehouse_that_lists_the_digest_confirms_the_freights_origin() {
    let link = warehouse_link(&healthy().with_meta()).await;
    assert_eq!(link.confidence, Confidence::Confirmed, "{link:#?}");
    assert!(matches!(&link.key, Key::Digest(d) if d.as_str() == NEW));
    let kinds: Vec<&str> = link
        .evidence
        .iter()
        .map(|s| s.object.kind.as_str())
        .collect();
    assert!(
        kinds.contains(&"Freight") && kinds.contains(&"Warehouse"),
        "{kinds:?}"
    );
}

#[tokio::test]
async fn a_digest_gone_from_the_discoveries_is_claimed_and_says_so_plainly() {
    let link = warehouse_link(&with_warehouse(warehouse(&[OLD]))).await;
    assert_eq!(link.confidence, Confidence::Claimed);
    assert!(
        link.reason
            .contains("the digest is no longer in the Warehouse's recent discoveries"),
        "{}",
        link.reason
    );
    assert!(link.reason.contains("(declared)"), "{}", link.reason);
}

#[tokio::test]
async fn a_warehouse_with_no_discovered_artifacts_only_claims() {
    let mut value = warehouse(&[]);
    value["status"] = json!({});
    let link = warehouse_link(&with_warehouse(value)).await;
    assert_eq!(link.confidence, Confidence::Claimed);
    assert!(
        link.reason.contains("reports no discovered artifacts"),
        "{}",
        link.reason
    );
}

#[tokio::test]
async fn a_subscription_to_another_repository_is_said() {
    let mut value = warehouse(&[OLD]);
    value["spec"]["subscriptions"] = json!([{"image": {"repoURL": "registry.example/acme/other"}}]);
    let link = warehouse_link(&with_warehouse(value)).await;
    assert_eq!(link.confidence, Confidence::Claimed);
    assert!(
        link.reason
            .contains("no subscription of the Warehouse names"),
        "{}",
        link.reason
    );
}

#[tokio::test]
async fn a_digest_listed_under_another_repository_is_not_a_match() {
    let mut value = warehouse(&[NEW]);
    value["status"]["discoveredArtifacts"]["images"][0]["repoURL"] =
        json!("registry.example/acme/other");
    let link = warehouse_link(&with_warehouse(value)).await;
    assert_eq!(link.confidence, Confidence::Claimed);
}

#[tokio::test]
async fn warehouses_that_cannot_be_read_or_found_are_unknown() {
    let mut world = healthy();
    world.kargo = world.kargo.refusing("warehouses");
    let link = warehouse_link(&world).await;
    assert_eq!(link.confidence, Confidence::Unknown);
    assert!(link.reason.contains("forbidden by RBAC"), "{}", link.reason);
    assert!(link.evidence.is_empty());

    let mut world = healthy();
    world.kargo = world.kargo.with("warehouses", vec![]).capped("warehouses");
    let link = warehouse_link(&world).await;
    assert_eq!(link.confidence, Confidence::Unknown);
    assert!(link.reason.contains("was not found"), "{}", link.reason);
    assert!(link.reason.contains("page cap"), "{}", link.reason);
}

#[tokio::test]
async fn a_warehouse_never_stops_the_summary() {
    let mut world = healthy();
    world.kargo = world.kargo.with("warehouses", vec![]);
    let trail = run(&world, &ENV).await;
    assert!(
        !trail.summary().contains("Warehouse"),
        "{}",
        trail.summary()
    );
}

#[test]
fn a_warehouse_without_the_newer_fields_is_still_read() {
    let old = parse_warehouse(&json!({
        "metadata": {"name": "w", "namespace": "storefront"},
        "spec": {"subscriptions": [{"image": {"repoURL": REPO}}]}
    }))
    .unwrap();
    assert_eq!(old.image_repos, [REPO]);
    assert!(old.discovered.is_none());
    let new = parse_warehouse(&warehouse(&[NEW])).unwrap();
    let images = new.discovered.unwrap();
    assert_eq!(images[0].repo_url, REPO);
    assert_eq!(images[0].digests[0].as_str(), NEW);
}

// ---- verification ------------------------------------------------------

fn verified_stage(phase: Option<&str>) -> Value {
    let mut stage = stage_holding("f-new", NEW);
    if let Some(phase) = phase {
        stage["status"]["freightHistory"][0]["verificationHistory"] =
            json!([{"id": "v1", "phase": phase}]);
    }
    stage
}

async fn stage_link(stage: Value, freight_verified: bool) -> Link {
    let mut world = healthy();
    let mut freight = freight("f-new", NEW, SHA);
    if !freight_verified {
        freight["status"] = json!({});
    }
    world.kargo = world.kargo.with("stages", vec![stage]).with(
        "freights",
        vec![freight, self::freight("f-old", OLD, OTHER_SHA)],
    );
    let trail = run(&world, &ENV).await;
    one(&trail, Hop::Freight, Hop::Stage).clone()
}

#[tokio::test]
async fn both_sides_reporting_a_verification_are_listed_and_never_change_confidence() {
    let link = stage_link(verified_stage(Some("Successful")), true).await;
    assert_eq!(link.confidence, Confidence::Confirmed);
    assert!(
        link.reason.contains("verified here: the Freight lists this Stage and the Stage's latest verification is Successful"),
        "{}",
        link.reason
    );
    let fields: Vec<&str> = link.evidence.iter().map(|s| s.field.as_str()).collect();
    assert!(fields.contains(&"/status/verifiedIn/dev"), "{fields:?}");
    assert!(
        fields.contains(&"/status/freightHistory/0/verificationHistory/0/phase"),
        "{fields:?}"
    );
}

#[tokio::test]
async fn kargos_own_words_for_a_phase_are_kept() {
    for phase in ["Failed", "Error", "Aborted", "Inconclusive"] {
        let link = stage_link(verified_stage(Some(phase)), true).await;
        assert_eq!(link.confidence, Confidence::Confirmed);
        assert!(
            link.reason
                .contains(&format!("the Stage's latest verification of it is {phase}")),
            "{}",
            link.reason
        );
    }
}

#[tokio::test]
async fn a_freight_that_says_verified_when_the_stage_records_none_is_a_note() {
    let link = stage_link(verified_stage(None), true).await;
    assert_eq!(link.confidence, Confidence::Confirmed);
    assert!(
        link.reason
            .contains("the Stage records no verification of it"),
        "{}",
        link.reason
    );
}

#[tokio::test]
async fn a_stage_verification_the_freight_does_not_list_is_a_note() {
    let link = stage_link(verified_stage(Some("Successful")), false).await;
    assert!(
        link.reason
            .contains("the Freight does not list this Stage as verified"),
        "{}",
        link.reason
    );
}

#[tokio::test]
async fn no_verification_on_either_side_says_so() {
    let link = stage_link(verified_stage(None), false).await;
    assert!(
        link.reason.contains("no verification reported"),
        "{}",
        link.reason
    );
}

#[test]
fn a_stage_without_a_verification_history_is_still_read() {
    let stage = parse_stage(&stage(&["f-new"])).unwrap();
    assert!(stage.verifications.is_empty());
    let stage = parse_stage(&verified_stage(Some("Successful"))).unwrap();
    assert_eq!(stage.verifications[0].freight, ["f-new"]);
    assert_eq!(stage.verifications[0].phase.as_deref(), Some("Successful"));
}
