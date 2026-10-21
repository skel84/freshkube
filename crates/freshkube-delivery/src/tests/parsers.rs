use super::*;

#[test]
fn freight_and_stage_are_read_in_both_kargo_shapes() {
    let new = parse_freight(&freight("f-new", NEW, SHA)).unwrap();
    assert_eq!(new.alias.as_deref(), Some("wonky-otter"));
    assert_eq!(new.images[0].tag.as_deref(), Some("v1.4.0"));
    assert_eq!(new.images[0].digest.as_ref().map(|d| d.as_str()), Some(NEW));
    assert_eq!(new.verified_in, ["dev"]);
    // Older Kargo: contents under `spec`, one currentFreight.
    let old = parse_freight(&serde_json::json!({
        "metadata": {"name": "f", "namespace": "storefront"},
        "spec": {"origin": {"name": "storefront"}, "images": [{"repoURL": REPO, "digest": OLD}]}
    }))
    .unwrap();
    assert_eq!(old.warehouse.as_deref(), Some("storefront"));
    assert_eq!(old.images.len(), 1);
    let stage = parse_stage(&serde_json::json!({
        "metadata": {"name": "s", "namespace": "storefront"},
        "status": {"currentFreight": {"name": "f"}}
    }))
    .unwrap();
    assert_eq!(stage.current_freight, ["f"]);
    // Nothing but a name is tolerated.
    assert!(
        parse_stage(&serde_json::json!({"metadata": {"name": "s", "namespace": "n"}})).is_some()
    );
    assert!(parse_freight(&serde_json::json!({})).is_none());
}

#[test]
fn a_stage_says_where_it_takes_freight_from() {
    let stage = |sources| {
        parse_stage(&serde_json::json!({
            "metadata": {"name": "s", "namespace": "storefront"},
            "spec": {"requestedFreight": [
                {"origin": {"kind": "Warehouse", "name": "images"}, "sources": sources}
            ]}
        }))
        .unwrap()
    };
    let first = stage(serde_json::json!({"direct": true}));
    assert!(first.direct);
    assert!(first.upstream.is_empty());
    let later = stage(serde_json::json!({"stages": ["test", "uat"]}));
    assert!(!later.direct);
    assert_eq!(later.upstream, ["test", "uat"]);
    assert_eq!(later.warehouses, ["images"]);
}

#[test]
fn build_results_pair_urls_with_digests_and_conforma_is_parsed() {
    let results = [
        ("IMAGE_URL", format!("{REPO}:v1")),
        ("IMAGE_DIGEST", NEW.to_owned()),
        (
            "sidecar_IMAGE_URL",
            "registry.example/acme/sidecar:v1".to_owned(),
        ),
        ("sidecar_IMAGE_DIGEST", "not-a-digest".to_owned()),
        (
            "TEST_OUTPUT",
            "{\"result\":\"FAILURE\",\"failures\":2}".to_owned(),
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    let images = built_images(&results);
    assert_eq!(
        images.len(),
        1,
        "a result without a real digest is not an image"
    );
    assert_eq!(images[0].digest.as_str(), NEW);
    let verdict = conforma(&results).unwrap();
    assert_eq!(
        (verdict.outcome.as_str(), verdict.failures, verdict.warnings),
        ("FAILURE", 2, 0)
    );
}

#[test]
fn a_promotions_steps_give_pushed_and_checked_out_commits_apart() {
    let promotion =
        super::kargo::parse_promotion(&promotion_pushing("f-new", NEW, "Succeeded", PUSHED))
            .unwrap();
    assert_eq!(promotion.pushed_commits, [PUSHED]);
    assert_eq!(promotion.source_commits, [SHA]);
    assert_eq!(promotion.freight_digests.len(), 1);
    // A value that is not a commit id is ignored, never matched.
    let mut value = promotion_pushing("f-new", NEW, "Succeeded", "not-a-commit");
    value["status"]["state"]["step-4"] = serde_json::json!({"commit": "zz"});
    let parsed = super::kargo::parse_promotion(&value).unwrap();
    assert!(parsed.pushed_commits.is_empty());
}

#[test]
fn a_commit_that_two_steps_record_is_pushed_once() {
    // A commit step and the push step after it both record the commit; the
    // push step's may differ in case.
    let mut value = promotion_pushing("f-new", NEW, "Succeeded", PUSHED);
    value["status"]["state"]["step-4"] =
        serde_json::json!({"commit": PUSHED.to_ascii_uppercase(), "branch": "main"});
    let promotion = super::kargo::parse_promotion(&value).unwrap();
    assert_eq!(promotion.pushed_commits, [PUSHED]);
}

#[test]
fn two_different_pushed_commits_are_both_kept_in_order() {
    let mut value = promotion_pushing("f-new", NEW, "Succeeded", PUSHED);
    value["status"]["state"]["step-4"] = serde_json::json!({"commit": OTHER_SHA});
    let promotion = super::kargo::parse_promotion(&value).unwrap();
    assert_eq!(promotion.pushed_commits, [PUSHED, OTHER_SHA]);
}

#[test]
fn every_parser_keeps_uid_and_resource_version() {
    use super::kargo::{parse_promotion, parse_warehouse};
    use super::observation::Meta;
    use super::pods::parse_pod;
    use super::rollouts::{parse_analysis_run, parse_replica_set, parse_rollout};
    use super::tekton::{parse_pipeline_run, parse_task_run};
    let object = serde_json::json!({
        "metadata": {
            "namespace": "acme",
            "name": "example",
            "uid": "u-1",
            "resourceVersion": "42",
        },
        "status": {"containerStatuses": [{"name": "app"}]},
    });
    let want = Meta {
        uid: Some("u-1".into()),
        resource_version: Some("42".into()),
    };
    let found = [
        parse_application(&object).unwrap().meta,
        parse_freight(&object).unwrap().meta,
        parse_stage(&object).unwrap().meta,
        parse_promotion(&object).unwrap().meta,
        parse_warehouse(&object).unwrap().meta,
        parse_rollout(&object).unwrap().meta,
        parse_replica_set(&object).unwrap().meta,
        parse_analysis_run(&object).unwrap().meta,
        parse_pipeline_run(&object).unwrap().meta,
        parse_task_run(&object).unwrap().meta,
        parse_pod(&object).remove(0).meta,
    ];
    for meta in found {
        assert_eq!(meta, want);
    }
    // Missing from what was read is missing, not invented.
    let bare = serde_json::json!({"metadata": {"namespace": "acme", "name": "example"}});
    assert_eq!(parse_application(&bare).unwrap().meta, Meta::default());
}

/// Pipelines as Code's `sha-title` holds the whole commit message; the
/// title is its first line that isn't blank, so a body never reaches a
/// one-line cell.
#[test]
fn a_run_titles_its_commit_by_the_first_line_of_sha_title() {
    let run = |title: &str| {
        super::tekton::parse_pipeline_run(&serde_json::json!({
            "metadata": {
                "namespace": "acme-ci",
                "name": "storefront-push-x",
                "annotations": {"pipelinesascode.tekton.dev/sha-title": title},
            },
        }))
        .unwrap()
        .title
    };
    assert_eq!(
        run("\n  Keep the basket when signing in  \r\n\nA guest's basket was dropped.\n")
            .as_deref(),
        Some("Keep the basket when signing in")
    );
    assert_eq!(run("One line").as_deref(), Some("One line"));
    assert_eq!(run(" \n\t\n"), None, "nothing but blank lines is no title");
}
