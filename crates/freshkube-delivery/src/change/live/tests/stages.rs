use super::*;

#[tokio::test]
async fn stages_follow_promotion_order_and_say_what_they_wait_for() {
    let reader = one_cluster(|world| {
        world.kargo = std::mem::take(&mut world.kargo).with(
            "stages",
            vec![
                stage_of("prod", &["staging"], &["f-old"]),
                stage_of("staging", &["dev"], &["f-old"]),
                stage_of("dev", &[], &["f-new"]),
            ],
        );
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let order: Vec<&str> = change.stages.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(order, ["dev", "staging", "prod"]);

    // Verified in dev, so staging may take it, but it runs f-old.
    let staging = &change.stages[1];
    assert_eq!(staging.words, "Waiting for promotion");
    assert_eq!(staging.running.as_deref(), Some("f-old"));
    assert!(matches!(&staging.eligible, Eligible::Verified { upstream, .. } if upstream == "dev"));
    assert_eq!(staging.promotion, Promotion::NotYet);
    assert_eq!(
        staging.cluster,
        Destination::Unknown(
            "it doesn't run this Freight, so its Application wasn't looked for".into()
        ),
        "nothing was looked for, so nothing is said to be missing"
    );

    let prod = &change.stages[2];
    assert_eq!(prod.words, "Not eligible yet");
    assert!(matches!(&prod.eligible, Eligible::NotYet { upstream } if upstream == &["staging"]));
}

#[tokio::test]
async fn a_promotion_that_failed_fails_the_stage() {
    let reader = one_cluster(|world| {
        let mut failed = promotion("f-new");
        failed["metadata"]["name"] = json!("dev.02.def");
        failed["status"] = json!({"phase": "Failed", "message": "step 2 failed"});
        world.kargo =
            std::mem::take(&mut world.kargo).with("promotions", vec![promotion("f-new"), failed]);
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let dev = &change.stages[0];
    assert_eq!(
        (dev.state, dev.words.as_str()),
        (HealthIndicator::Error, "Promotion failed")
    );
    assert_eq!(
        dev.promotion,
        Promotion::Failed {
            phase: "Failed".into(),
            message: Some("step 2 failed".into())
        }
    );
}

#[tokio::test]
async fn refused_stages_say_so_rather_than_showing_none() {
    let reader = one_cluster(|world| {
        world.kargo = std::mem::take(&mut world.kargo).refusing("stages");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    assert!(change.stages.is_empty());
    let stages = row(&change, "stages");
    assert_eq!(stages.state, HealthIndicator::Unknown);
    assert_eq!(stages.group, 1);
    assert!(stages.detail.contains("forbidden"), "{}", stages.detail);
}

#[tokio::test]
async fn refused_promotions_leave_the_promotion_gate_unknown() {
    let reader = one_cluster(|world| {
        world.kargo = std::mem::take(&mut world.kargo).refusing("promotions");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let dev = &change.stages[0];
    assert!(
        matches!(&dev.promotion, Promotion::Unknown { why } if why.contains("forbidden")),
        "{:?}",
        dev.promotion
    );
    let gate = row(&change, "dev-promotion");
    assert_eq!(
        (gate.state, gate.detail.as_str()),
        (HealthIndicator::Unknown, "Promotions not readable")
    );
}
