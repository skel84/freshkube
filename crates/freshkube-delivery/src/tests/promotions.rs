use super::*;

#[tokio::test]
async fn a_promotion_message_is_printed_without_addresses_or_identities() {
    let mut world = healthy();
    let mut promotion = promotion("f-new");
    promotion["status"]["message"] = serde_json::json!(
        r#"push to https://git.example.test/acme/config.git failed: User "someone" denied; see /var/run/x, 192.0.2.7:443"#
    );
    world.kargo = world.kargo.with("promotions", vec![promotion]);
    let trail = run(&world, &ENV).await;
    let reason = &one(&trail, Hop::Freight, Hop::Promotion).reason;
    for word in ["git.example.test", "someone", "/var/run", "192.0.2.7"] {
        assert!(!reason.contains(word), "{reason}");
    }
    assert!(reason.contains("push to <url> failed"), "{reason}");
}

#[tokio::test]
async fn the_pushed_commit_the_application_synced_confirms_promotion_stage_and_application() {
    let trail = run(&promoted(NEW, "Succeeded", PUSHED), &ENV).await;
    let promotion = one(&trail, Hop::Freight, Hop::Promotion);
    assert_eq!(promotion.confidence, Confidence::Confirmed);
    assert!(matches!(&promotion.key, Key::Digest(d) if d.as_str() == NEW));
    let stage = one(&trail, Hop::Freight, Hop::Stage);
    assert_eq!(stage.confidence, Confidence::Confirmed);
    assert!(matches!(&stage.key, Key::Digest(d) if d.as_str() == NEW));
    let app = one(&trail, Hop::Stage, Hop::Application);
    assert_eq!(app.confidence, Confidence::Confirmed);
    assert!(matches!(&app.key, Key::Sha(sha) if sha == PUSHED));
    assert!(
        app.reason
            .contains("synced the commit the Promotion pushed")
    );
}

#[tokio::test]
async fn a_revision_that_is_not_the_pushed_commit_stays_claimed() {
    // The Application is at another commit: it moved on, or never took it.
    let trail = run(&promoted(NEW, "Succeeded", OTHER_SHA), &ENV).await;
    let app = one(&trail, Hop::Stage, Hop::Application);
    assert_eq!(app.confidence, Confidence::Claimed);
    assert!(matches!(&app.key, Key::Name(_)));
    assert!(
        app.reason
            .contains("but the Application's synced revision is")
    );
    // The digest evidence still stands on its own.
    assert_eq!(
        one(&trail, Hop::Freight, Hop::Promotion).confidence,
        Confidence::Confirmed
    );
}

#[tokio::test]
async fn a_promotion_that_did_not_succeed_confirms_nothing() {
    let trail = run(&promoted(NEW, "Failed", PUSHED), &ENV).await;
    let app = one(&trail, Hop::Stage, Hop::Application);
    assert_eq!(app.confidence, Confidence::Claimed);
    assert!(
        app.reason
            .contains("no succeeded Promotion recorded a pushed commit")
    );
}

#[tokio::test]
async fn records_that_carry_another_digest_are_not_a_match() {
    // The Promotion and the Stage hold a different image than the Freight.
    let trail = run(&promoted(OLD, "Succeeded", PUSHED), &ENV).await;
    assert_eq!(
        one(&trail, Hop::Freight, Hop::Promotion).confidence,
        Confidence::Claimed
    );
    assert_eq!(
        one(&trail, Hop::Freight, Hop::Stage).confidence,
        Confidence::Claimed
    );
}

#[tokio::test]
async fn another_stages_application_at_the_same_revision_keeps_it_claimed() {
    let mut world = promoted(NEW, "Succeeded", PUSHED);
    // A later stage's Application of the same project, on the same branch, is at
    // the same commit: the commit cannot tell the two stages apart.
    let mut prod = application(Some("https://env-a.example:6443"));
    prod["metadata"]["name"] = serde_json::json!("storefront-prod");
    prod["metadata"]["annotations"]["kargo.akuity.io/authorized-stage"] =
        serde_json::json!("storefront:prod");
    prod["status"]["sync"]["revision"] = serde_json::json!(PUSHED);
    let mut dev = application(Some("https://env-a.example:6443"));
    dev["status"]["sync"]["revision"] = serde_json::json!(PUSHED);
    world.argocd = world.argocd.with("applications", vec![dev, prod]);
    let trail = run(&world, &ENV).await;
    let app = link(&trail, Hop::Stage, Hop::Application)
        .into_iter()
        .find(|l| l.subject == "argocd/storefront-dev")
        .expect("the dev Application is linked");
    assert_eq!(app.confidence, Confidence::Claimed);
    assert!(
        app.reason
            .contains("another stage's Application is at it too")
    );
}

fn promotion_by(actor: Option<&str>) -> World {
    let mut world = promoted(NEW, "Succeeded", PUSHED);
    let mut promotion = promotion_pushing("f-new", NEW, "Succeeded", PUSHED);
    if let Some(actor) = actor {
        promotion["metadata"]["annotations"] =
            serde_json::json!({"kargo.akuity.io/create-actor": actor});
    }
    world.kargo = world.kargo.with("promotions", vec![promotion]);
    world
}

#[tokio::test]
async fn a_promotion_says_auto_or_manual_and_the_commit_it_pushed() {
    for (actor, words) in [
        (None, "likely auto-promoted"),
        (
            Some("controller:kargo-controller"),
            "created by a Kargo controller",
        ),
        (Some("email:someone@example.test"), "promoted by hand"),
        (Some("kubernetes:someone"), "promoted by hand"),
        (Some("admin"), "promoted by hand"),
        (
            Some("kubernetes:system:serviceaccount:delivery:someone"),
            "created by a service account, an API client",
        ),
        (Some("unknown actor"), "its creator is not known"),
    ] {
        let trail = run(&promotion_by(actor), &ENV).await;
        let reason = &one(&trail, Hop::Freight, Hop::Promotion).reason;
        assert!(reason.contains(words), "{actor:?}: {reason}");
        assert!(reason.contains("it pushed dddddddddddd"), "{reason}");
        // The actor is an identity and is never printed.
        assert!(!reason.contains("someone"), "{reason}");
    }
    let trail = run(&healthy(), &ENV).await;
    assert!(
        one(&trail, Hop::Freight, Hop::Promotion)
            .reason
            .contains("it recorded no pushed commit")
    );
}

#[tokio::test]
async fn an_unhealthy_stage_says_why() {
    let mut world = healthy();
    let mut stage = stage(&["f-new"]);
    stage["status"]["health"] = serde_json::json!({
        "status": "Unhealthy",
        "issues": [
            "Argo CD Application \"argocd/storefront-dev\" is synced to eeeeeeeeeeee, not the desired revision dddddddddddd",
            "fetching https://git.example.test/acme/config.git failed"
        ]
    });
    world.kargo = world.kargo.with("stages", vec![stage]);
    let trail = run(&world, &ENV).await;
    let reason = &one(&trail, Hop::Freight, Hop::Stage).reason;
    assert!(
        reason.contains("health Unhealthy (Argo CD Application \"argocd/storefront-dev\" is synced to eeeeeeeeeeee, not the desired revision dddddddddddd; fetching <url> failed)"),
        "{reason}"
    );
    assert!(!reason.contains("git.example.test"), "{reason}");
}

#[tokio::test]
async fn the_freight_says_whether_its_image_revision_is_the_commit() {
    let sha_256 = "a".repeat(64);
    for (revision, words) in [
        (SHA, "names aaaaaaaaaaaa, this change's commit"),
        (OTHER_SHA, "names bbbbbbbbbbbb, not this change's commit"),
        // An abbreviation is compared as the start of the full id, case aside.
        ("AAAAAAA", "names aaaaaaa, this change's commit"),
        ("aaaaaaab", "names aaaaaaab, not this change's commit"),
        // A longer id than the commit's is another hash algorithm's.
        (
            sha_256.as_str(),
            "names aaaaaaaaaaaa, which can't be compared with this change's commit",
        ),
    ] {
        let mut world = healthy();
        let mut item = freight("f-new", NEW, SHA);
        item["images"][0]["annotations"] =
            serde_json::json!({"org.opencontainers.image.revision": revision});
        world.kargo = world.kargo.with("freights", vec![item]);
        let trail = run(&world, &ENV).await;
        let reason = &one(&trail, Hop::PipelineRun, Hop::Freight).reason;
        assert!(reason.contains(words), "{reason}");
    }
    let trail = run(&healthy(), &ENV).await;
    assert!(
        !one(&trail, Hop::PipelineRun, Hop::Freight)
            .reason
            .contains("OCI revision")
    );
}
