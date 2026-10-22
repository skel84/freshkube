//! The causes a derived change carries on its hops (#823).

use serde_json::json;

use super::super::{Cause, Change, Hop as Row, Next, Role, SettingsField};
use super::read;
use super::tests::{one_cluster, place, stage_of};
use crate::fixtures::*;
use crate::tests::observed_at;
use freshkube_core::indicators::HealthIndicator;

fn hop<'a>(change: &'a Change, key: &str) -> &'a Row {
    change
        .hops
        .iter()
        .find(|hop| hop.key == key)
        .unwrap_or_else(|| {
            let keys: Vec<&str> = change.hops.iter().map(|hop| hop.key.as_str()).collect();
            panic!("{key} in {keys:?}")
        })
}

/// The Freight, verified nowhere yet.
fn unverified() -> serde_json::Value {
    let mut freight = freight("f-new", NEW, SHA);
    freight["status"] = json!({});
    freight
}

async fn change_of(reader: &FixtureReader) -> Change {
    read(reader, &place("f-new"), observed_at())
        .await
        .expect("read")
}

#[tokio::test]
async fn a_stage_the_freight_has_not_reached_waits_on_every_gate() {
    // `prod` takes Freight verified in `dev`, which doesn't hold it yet.
    let reader = one_cluster(|world| {
        world.kargo = std::mem::take(&mut world.kargo)
            .with("freights", vec![unverified()])
            .with(
                "stages",
                vec![
                    stage_of("dev", &[], &["f-old"]),
                    stage_of("prod", &["dev"], &["f-old"]),
                ],
            );
    });
    let change = change_of(&reader).await;
    for key in ["prod-eligible", "prod-promotion", "prod-verification"] {
        let gate = hop(&change, key);
        assert_eq!(gate.state, HealthIndicator::Pending, "{key}");
        let cause = gate.cause.as_ref().unwrap_or_else(|| panic!("{key}"));
        assert!(matches!(cause, Cause::Waiting { .. }), "{key}: {cause:?}");
        assert!(!cause.is_problem(), "{key}: waiting is no problem");
        assert_eq!(cause.next(), [Next::OpenInKargo]);
    }
}

#[tokio::test]
async fn a_verification_waits_only_while_the_promotion_is_under_way() {
    let with_phase = |phase: &'static str| {
        one_cluster(move |world| {
            let mut running = promotion("f-new");
            running["status"]["phase"] = json!(phase);
            world.kargo = std::mem::take(&mut world.kargo)
                .with("freights", vec![unverified()])
                .with("promotions", vec![running]);
        })
    };
    let waiting = change_of(&with_phase("Running")).await;
    assert!(matches!(
        hop(&waiting, "dev-verification").cause,
        Some(Cause::Waiting { .. })
    ));
    // The Promotion ended and Kargo recorded no verification: not found.
    let ended = change_of(&with_phase("Succeeded")).await;
    let gate = hop(&ended, "dev-verification");
    assert_eq!(gate.state, HealthIndicator::Pending);
    assert!(matches!(gate.cause, Some(Cause::NotFound { .. })));
    assert_eq!(gate.cause.as_ref().unwrap().next(), [Next::CheckSources]);
}

#[tokio::test]
async fn a_capped_promotions_list_names_no_cause_for_what_it_did_not_find() {
    // The Promotion may be past the cap.
    let reader = one_cluster(|world| {
        world.kargo = std::mem::take(&mut world.kargo)
            .with("freights", vec![unverified()])
            .with(
                "stages",
                vec![
                    stage_of("dev", &[], &["f-old"]),
                    stage_of("prod", &["dev"], &["f-old"]),
                ],
            )
            .with("promotions", vec![])
            .capped("promotions");
    });
    let change = change_of(&reader).await;
    assert_eq!(hop(&change, "prod-promotion").cause, None);
    let held = one_cluster(|world| {
        world.kargo = std::mem::take(&mut world.kargo)
            .with("promotions", vec![])
            .capped("promotions");
    });
    assert_eq!(hop(&change_of(&held).await, "dev-promotion").cause, None);
}

#[tokio::test]
async fn a_gate_that_is_met_has_no_cause() {
    let change = change_of(&one_cluster(|_| {})).await;
    assert_eq!(hop(&change, "dev-eligible").cause, None);
    assert_eq!(hop(&change, "dev-promotion").cause, None);
}

#[tokio::test]
async fn refused_promotions_name_the_role_on_the_promotion_gate() {
    let reader = one_cluster(|world| {
        world.kargo = std::mem::take(&mut world.kargo).refusing("promotions");
    });
    let change = change_of(&reader).await;
    let gate = hop(&change, "dev-promotion");
    assert_eq!(gate.state, HealthIndicator::Unknown);
    assert_eq!(
        gate.cause.as_ref().map(Cause::next),
        Some(vec![Next::CopyAccessRequest(Role::new(
            "kargo.akuity.io",
            "promotions",
            Some("storefront")
        ))])
    );
}

#[tokio::test]
async fn refused_stages_name_the_role_on_the_stages_row() {
    let reader = one_cluster(|world| {
        world.kargo = std::mem::take(&mut world.kargo).refusing("stages");
    });
    let change = change_of(&reader).await;
    let stages = hop(&change, "stages");
    assert_eq!(
        stages.cause.as_ref().map(Cause::next),
        Some(vec![Next::CopyAccessRequest(Role::new(
            "kargo.akuity.io",
            "stages",
            Some("storefront")
        ))])
    );
}

#[tokio::test]
async fn a_promotion_kargo_no_longer_has_is_not_found_where_the_stage_runs_the_freight() {
    let reader = one_cluster(|world| {
        world.kargo = std::mem::take(&mut world.kargo).with("promotions", vec![]);
    });
    let change = change_of(&reader).await;
    let gate = hop(&change, "dev-promotion");
    assert_eq!(gate.state, HealthIndicator::Unknown);
    assert!(matches!(gate.cause, Some(Cause::NotFound { .. })));
}

#[tokio::test]
async fn a_freight_with_no_commit_and_no_image_has_no_build_to_find() {
    let reader = one_cluster(|world| {
        let mut bare = freight("f-new", NEW, SHA);
        bare["images"] = json!([]);
        bare["commits"] = json!([]);
        world.kargo = std::mem::take(&mut world.kargo).with("freights", vec![bare]);
    });
    let change = change_of(&reader).await;
    let build = hop(&change, "build");
    assert!(matches!(build.cause, Some(Cause::NotFound { .. })));
}

#[tokio::test]
async fn the_application_a_stage_has_no_name_for_points_at_the_stage_naming_setting() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).with("applications", vec![]);
    });
    let change = change_of(&reader).await;
    let not_set_up: Vec<&Row> = change
        .hops
        .iter()
        .filter(|hop| matches!(hop.cause, Some(Cause::NotSetUp { .. })))
        .collect();
    assert_eq!(not_set_up.len(), 1, "{not_set_up:#?}");
    assert_eq!(
        not_set_up[0].cause.as_ref().map(Cause::next),
        Some(vec![Next::OpenSettings(SettingsField::STAGE_NAMING)])
    );
    // The Inspector's link carries the same cause as its row.
    let super::super::Shows::Hop(detail) = &not_set_up[0].shows else {
        panic!("a hop");
    };
    assert_eq!(
        detail.link.as_ref().and_then(|link| link.cause.clone()),
        not_set_up[0].cause
    );
}

#[test]
fn pods_of_an_unread_entry_can_be_read_again() {
    let workload = Row {
        cause: None,
        key: "w".into(),
        group: 2,
        state: HealthIndicator::Unknown,
        name: "Deployment api".into(),
        detail: String::new(),
        from: "argocd".into(),
        at: None,
        link: None,
        after: None,
        shows: super::super::Shows::Stage(0),
    };
    let pods = super::super::unread::pods(&workload, "prod-lon", "no answer", false);
    assert_eq!(
        pods.cause.as_ref().map(Cause::next),
        Some(vec![Next::Retry])
    );
    // A refusing entry is not mended by reading again.
    let refused = super::super::unread::pods(&workload, "prod-lon", "forbidden", true);
    assert_eq!(
        refused.cause.as_ref().map(Cause::next),
        Some(Vec::<Next>::new())
    );
    assert!(matches!(pods.cause, Some(Cause::CantRead { .. })));
}

/// A template that doesn't resolve for the project: a setup problem, not a
/// refusal of the cluster.
fn unresolved_build_namespace() -> super::Place {
    let mut place = place("f-new");
    place.build_namespace = Some(format!("{{project}}-{}", "c".repeat(60)));
    place
}

#[tokio::test]
async fn an_unresolved_build_namespace_is_not_set_up_not_a_refusal() {
    let reader = one_cluster(|_| {});
    let change = read(&reader, &unresolved_build_namespace(), observed_at())
        .await
        .expect("read");
    let build = change
        .hops
        .iter()
        .find(|hop| hop.key.starts_with("build-run"))
        .expect("the build hop");
    let cause = build.cause.as_ref().expect("a cause");
    assert!(matches!(cause, Cause::NotSetUp { .. }), "{cause:?}");
    assert_eq!(
        cause.next(),
        [Next::OpenSettings(SettingsField::BUILD_NAMESPACE)]
    );
    // No Role is asked for: nothing was refused.
    let super::super::Shows::Hop(detail) = &build.shows else {
        panic!("a hop");
    };
    assert_eq!(
        detail.link.as_ref().and_then(|link| link.cause.clone()),
        build.cause
    );
}

#[tokio::test]
async fn a_refused_pipelineruns_list_still_asks_for_its_role() {
    let reader = one_cluster(|world| {
        world.tekton = std::mem::take(&mut world.tekton).refusing("pipelineruns");
    });
    let change = change_of(&reader).await;
    let build = hop(&change, "build-run-0");
    assert!(matches!(build.cause, Some(Cause::CantRead { .. })));
}
