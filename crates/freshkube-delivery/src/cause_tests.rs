//! Why a link is Unknown, and what makes Kargo's record a Confirmed link,
//! from each real source in the join (#823).

use serde_json::{Value, json};

use super::change::{Cause, Next, Role, SettingsField};
use super::fixtures::*;
use super::join::{Confidence, Hop, Key};
use super::tests::{ENV, naming, one, run, run_configured};
use freshkube_core::resources::{Failure, FailureKind};

/// A Stage whose newest Freight history entry holds `names` and whose older
/// entries hold `older`, with no image on any of them.
fn history(names: &[&str], older: &[&str]) -> Value {
    let entry = |names: &[&str]| {
        let items: serde_json::Map<String, Value> = names
            .iter()
            .map(|name| {
                (
                    format!("Warehouse/storefront/{name}"),
                    json!({"name": name}),
                )
            })
            .collect();
        json!({"id": "x", "items": items})
    };
    let mut stage = stage(&[]);
    stage["status"]["freightHistory"] = json!([entry(names), entry(older)]);
    stage
}

fn with_stage(stage: Value) -> World {
    let mut world = healthy();
    world.kargo = world.kargo.with("stages", vec![stage]);
    world
}

#[tokio::test]
async fn kargos_newest_history_entry_confirms_a_freight_that_has_no_digest_to_join_on() {
    // A Freight of a commit alone: nothing but the name joins it to the
    // Stage's record.
    let mut world = with_stage(history(&["f-new"], &[]));
    let mut bare = freight("f-new", NEW, SHA);
    bare["images"] = json!([]);
    world.kargo = world.kargo.with("freights", vec![bare]);
    let trail = run(&world, &ENV).await;
    let link = one(&trail, Hop::Freight, Hop::Stage);
    assert_eq!(link.confidence, Confidence::Confirmed);
    assert_eq!(link.key, Key::Name("f-new".into()));
    assert!(link.reason.contains("newest entry"), "{}", link.reason);
    assert_eq!(link.cause, None);
    // Both sides are reported: the Freight's own name, the Stage's record.
    assert!(
        link.evidence
            .iter()
            .any(|seen| seen.object.kind == "Freight")
    );
    assert!(link.evidence.iter().any(|seen| seen.object.kind == "Stage"));
}

#[tokio::test]
async fn the_older_fields_of_a_stage_stay_a_claim() {
    // Older Kargo records `currentFreight` and no history: the Stage's own
    // say-so, not Kargo's record of what it runs.
    let mut old = stage(&[]);
    old["status"] = json!({
        "currentFreight": {"name": "f-new"},
        "health": {"status": "Healthy"}
    });
    let trail = run(&with_stage(old), &ENV).await;
    let link = one(&trail, Hop::Freight, Hop::Stage);
    assert_eq!(link.confidence, Confidence::Claimed);
    assert!(link.reason.contains("names this Freight only"));

    let mut last = stage(&[]);
    last["status"] = json!({
        "lastPromotion": {"name": "p", "freight": {"name": "f-new"}},
        "health": {"status": "Healthy"}
    });
    let trail = run(&with_stage(last), &ENV).await;
    assert_eq!(
        one(&trail, Hop::Freight, Hop::Stage).confidence,
        Confidence::Claimed
    );
}

#[tokio::test]
async fn a_history_that_does_not_name_the_freight_confirms_nothing() {
    // The Stage runs another Freight, and this one was never promoted to it:
    // not reached yet, and nothing is wrong.
    let trail = run(&with_stage(history(&["f-old"], &[])), &ENV).await;
    let link = one(&trail, Hop::Freight, Hop::Stage);
    assert_eq!(link.confidence, Confidence::Unknown);
    assert!(link.reason.contains("no Stage reports it as current"));
    assert!(matches!(link.cause, Some(Cause::Waiting { .. })));
    assert!(!link.cause.as_ref().unwrap().is_problem());
}

#[tokio::test]
async fn a_freight_the_stage_has_moved_past_is_not_waiting() {
    // An older history entry names it: it was promoted and the Stage runs
    // another Freight now, so nothing waits on it.
    let trail = run(&with_stage(history(&["f-old"], &["f-new"])), &ENV).await;
    let link = one(&trail, Hop::Freight, Hop::Stage);
    assert_eq!(link.confidence, Confidence::Unknown);
    assert!(matches!(link.cause, Some(Cause::NotFound { .. })));
}

#[tokio::test]
async fn a_record_with_another_digest_keeps_the_stage_a_claim() {
    // The name is in the newest entry, but the Stage holds another image.
    let world = with_stage(stage_holding("f-new", OLD));
    let trail = run(&world, &ENV).await;
    assert_eq!(
        one(&trail, Hop::Freight, Hop::Stage).confidence,
        Confidence::Claimed
    );
}

#[tokio::test]
async fn a_refused_list_names_the_role_that_would_read_it_and_offers_no_retry() {
    let mut world = healthy();
    world.kargo = world.kargo.refusing("promotions");
    let trail = run(&world, &ENV).await;
    let link = one(&trail, Hop::Freight, Hop::Promotion);
    assert_eq!(link.confidence, Confidence::Unknown);
    let Some(Cause::CantRead { role, retry, .. }) = &link.cause else {
        panic!("{:?}", link.cause);
    };
    assert_eq!(
        role.as_ref(),
        Some(&Role::new(
            "kargo.akuity.io",
            "promotions",
            Some("storefront")
        ))
    );
    assert!(!retry, "a refusal doesn't pass by reading again");
    assert_eq!(
        link.cause.as_ref().unwrap().next(),
        [Next::CopyAccessRequest(role.clone().unwrap())]
    );
}

#[tokio::test]
async fn an_unreachable_list_offers_a_retry_and_no_role() {
    let mut world = healthy();
    world.kargo = world.kargo.failing(
        "stages",
        Failure::new(FailureKind::Unreachable, "connection refused"),
    );
    let trail = run(&world, &ENV).await;
    let link = one(&trail, Hop::Freight, Hop::Stage);
    assert_eq!(
        link.cause,
        Some(Cause::cant_read(
            "the Stages of the project, to see which one runs this Freight",
            None,
            false
        ))
    );
    assert_eq!(link.cause.as_ref().unwrap().next(), [Next::Retry]);
}

#[tokio::test]
async fn refused_pods_name_the_pods_role_of_their_namespace() {
    let mut world = healthy();
    world.environment = world.environment.refusing("pods");
    let trail = run(&world, &ENV).await;
    let link = one(&trail, Hop::Rollout, Hop::Pod);
    let Some(Cause::CantRead { role, .. }) = &link.cause else {
        panic!("{:?}", link.cause);
    };
    let role = role.as_ref().expect("a refusal names its kind");
    assert_eq!((role.group.as_str(), role.resource.as_str()), ("", "pods"));
    assert!(role.namespace.is_some());
}

#[tokio::test]
async fn no_application_naming_a_stage_is_not_set_up_without_a_naming_and_not_found_with_one() {
    let mut world = healthy();
    world.argocd = world.argocd.with("applications", vec![]);
    let trail = run(&world, &ENV).await;
    let link = one(&trail, Hop::Stage, Hop::Application);
    assert_eq!(link.confidence, Confidence::Unknown);
    let Some(Cause::NotSetUp { field, .. }) = &link.cause else {
        panic!("{:?}", link.cause);
    };
    assert_eq!(*field, SettingsField::STAGE_NAMING);
    assert_eq!(field.section, "settings-sources");

    let trail = run_configured(&world, &ENV, &Default::default(), None, |plan| {
        plan.stage_naming = Some(naming());
    })
    .await;
    let link = one(&trail, Hop::Stage, Hop::Application);
    assert!(
        matches!(link.cause, Some(Cause::NotFound { .. })),
        "{:?}",
        link.cause
    );
    assert_eq!(link.cause.as_ref().unwrap().next(), [Next::CheckSources]);
}

#[tokio::test]
async fn a_build_that_is_not_there_is_not_found() {
    let mut world = healthy();
    world.tekton = world.tekton.with("pipelineruns", vec![]);
    let trail = run(&world, &ENV).await;
    let link = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(link.confidence, Confidence::Unknown);
    assert!(matches!(link.cause, Some(Cause::NotFound { .. })));
}

#[tokio::test]
async fn a_capped_listing_names_no_cause_since_the_piece_may_be_past_the_cap() {
    let mut world = healthy();
    world.tekton = world
        .tekton
        .with("pipelineruns", vec![])
        .capped("pipelineruns");
    let trail = run(&world, &ENV).await;
    let link = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(link.cause, None);
}

#[tokio::test]
async fn a_warehouse_the_freight_names_but_is_missing_is_not_found() {
    let mut world = healthy();
    world.kargo = world.kargo.with("warehouses", vec![]);
    let trail = run(&world, &ENV).await;
    let link = one(&trail, Hop::Freight, Hop::Warehouse);
    assert!(matches!(link.cause, Some(Cause::NotFound { .. })));
}

#[test]
fn a_digest_search_without_a_build_namespace_is_not_set_up() {
    use crate::join::{DigestOutcome, DigestSearch};
    let search = |outcome| DigestSearch {
        digests: Vec::new(),
        outcome,
        archive: None,
    };
    assert_eq!(
        search(DigestOutcome::NoNamespace).cause().map(|c| c.next()),
        Some(vec![Next::OpenSettings(SettingsField::BUILD_NAMESPACE)])
    );
    // A failed search says only its words.
    assert_eq!(search(DigestOutcome::Failed("x".into())).cause(), None);
}

#[tokio::test]
async fn only_an_unknown_link_has_a_cause() {
    let trail = run(&healthy(), &ENV).await;
    for link in &trail.links {
        if link.confidence != Confidence::Unknown {
            assert_eq!(link.cause, None, "{:?}->{:?}", link.from, link.to);
        }
    }
}

/// The Role a refused kind's link names.
fn role_of(link: &crate::join::Link) -> Role {
    match &link.cause {
        Some(Cause::CantRead {
            role: Some(role), ..
        }) => role.clone(),
        other => panic!("a refusal names its Role, got {other:?}"),
    }
}

#[tokio::test]
async fn refused_applications_ask_for_a_role_in_the_argo_cd_namespace() {
    let mut world = healthy();
    world.argocd = world.argocd.refusing("applications");
    let trail = run(&world, &ENV).await;
    assert_eq!(
        role_of(one(&trail, Hop::Stage, Hop::Application)),
        Role::new("argoproj.io", "applications", Some("argocd"))
    );
}

#[tokio::test]
async fn refused_pipeline_runs_ask_in_the_build_namespace_and_cluster_wide_without_one() {
    let mut world = healthy();
    world.tekton = world.tekton.refusing("pipelineruns");
    let trail = run(&world, &ENV).await;
    assert_eq!(
        role_of(one(&trail, Hop::Commit, Hop::PipelineRun)),
        Role::new("tekton.dev", "pipelineruns", Some("acme-builds"))
    );
    // With no build namespace the builds are read across all of them, so
    // only a ClusterRole reads them.
    let trail = run_configured(&world, &ENV, &Default::default(), None, |plan| {
        plan.build_namespace = Ok(None);
    })
    .await;
    let role = role_of(one(&trail, Hop::Commit, Hop::PipelineRun));
    assert_eq!(role.namespace, None);
    assert!(role.manifest().contains("kind: ClusterRole"));
}

#[tokio::test]
async fn refused_rollouts_ask_in_the_applications_namespace() {
    let mut world = healthy();
    world.environment = world.environment.refusing("rollouts");
    let trail = run(&world, &ENV).await;
    let role = role_of(one(&trail, Hop::Application, Hop::Rollout));
    assert_eq!(
        (role.group.as_str(), role.resource.as_str()),
        ("argoproj.io", "rollouts")
    );
    assert!(role.namespace.is_some());
}

#[tokio::test]
async fn a_sha_256_commit_can_not_be_a_label_so_reading_again_cannot_help() {
    let world = healthy();
    let trail = run_configured(&world, &ENV, &Default::default(), None, |plan| {
        plan.sha = "a".repeat(64);
    })
    .await;
    let link = one(&trail, Hop::Commit, Hop::PipelineRun);
    assert_eq!(link.confidence, Confidence::Unknown);
    let cause = link.cause.as_ref().expect("a cause");
    assert!(
        matches!(cause, Cause::CantRead { retry: false, .. }),
        "{cause:?}"
    );
    assert_eq!(cause.next(), Vec::<Next>::new());
}
