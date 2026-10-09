use serde_json::{Value, json};

use super::super::{
    Change, Destination, Eligible, Hop as Row, Phase, Promotion, Shows, Value as Shown,
};
use super::{Place, read};
use crate::delivery::argocd::IN_CLUSTER_SERVER;
use crate::delivery::fixtures::*;
use crate::delivery::join::{Confidence, Hop, join};
use crate::delivery::tests::{observed_at, plan};
use crate::indicators::HealthIndicator;

fn place(freight: &str) -> Place {
    Place {
        cluster: "cluster-key".into(),
        label: "core".into(),
        project: "storefront".into(),
        freight: freight.into(),
        argocd_namespace: "argocd".into(),
    }
}

/// A Stage of storefront taking Freight from the Warehouse (`upstream`
/// empty) or from the Stages named, holding `current`.
fn stage_of(name: &str, upstream: &[&str], current: &[&str]) -> Value {
    let mut value = stage(current);
    value["metadata"]["name"] = json!(name);
    value["spec"]["requestedFreight"][0]["sources"] = if upstream.is_empty() {
        json!({"direct": true})
    } else {
        json!({"stages": upstream})
    };
    value
}

/// [`healthy`], with its Application deploying to the cluster Argo CD runs
/// in, the Stage taking Freight from the Warehouse, all in one cluster.
fn one_cluster(edit: impl FnOnce(&mut World)) -> FixtureReader {
    let mut world = healthy();
    world.argocd = world
        .argocd
        .with("applications", vec![application(Some(IN_CLUSTER_SERVER))]);
    world.kargo = world
        .kargo
        .with("stages", vec![stage_of("dev", &[], &["f-new"])]);
    edit(&mut world);
    world.one_cluster()
}

#[tokio::test]
async fn a_change_reads_from_its_freight_to_the_pods_in_one_cluster() {
    let reader = one_cluster(|_| {});
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("the Freight is read");

    assert_eq!(change.freight, "wonky-otter");
    assert_eq!(change.kargo_cluster, "core");
    let phases: Vec<Phase> = change.groups.iter().map(|g| g.phase.clone()).collect();
    assert_eq!(phases, [Phase::Build, Phase::Freight, Phase::Stage(0)]);

    let dev = &change.stages[0];
    assert_eq!(dev.name, "dev");
    assert_eq!(
        dev.cluster,
        Destination::Cluster("core".into()),
        "Argo CD deploys to its own cluster"
    );
    assert_eq!(change.groups[2].detail, "deploys to core");
    assert_eq!(
        (dev.state, dev.words.as_str()),
        (HealthIndicator::Healthy, "Verified")
    );
    assert_eq!(dev.eligible, Eligible::Warehouse { at: None });
    assert!(
        matches!(&dev.promotion, Promotion::Other { how, .. } if how.starts_with("Likely automatic")),
        "{:?}",
        dev.promotion
    );
    assert_eq!(dev.running, None);
    assert_eq!(dev.object.cluster, "cluster-key", "objects open by the key");
    assert_eq!(dev.object.kind, "Stage");

    let names: Vec<&str> = change.hops.iter().map(|hop| hop.name.as_str()).collect();
    assert!(names.contains(&"Run storefront-push-x"), "{names:?}");
    assert!(names.contains(&"Freight wonky-otter"), "{names:?}");
    assert!(names.contains(&"Warehouse storefront"), "{names:?}");
    for gate in ["Eligible", "Promotion", "Verification"] {
        let hop = change
            .hops
            .iter()
            .find(|hop| hop.key == format!("dev-{}", gate.to_lowercase()))
            .unwrap_or_else(|| panic!("{gate} in {names:?}"));
        assert_eq!(hop.group, 2);
        assert!(matches!(hop.shows, Shows::Stage(0)));
    }
    let pods = change
        .hops
        .iter()
        .find(|hop| hop.name == "Pods")
        .expect("the pods are reached");
    assert_eq!(pods.group, 2);
    assert_eq!(pods.state, HealthIndicator::Healthy, "the pod is ready");
    assert!(change.hops.iter().all(|hop| hop.from == "core"));
}

#[tokio::test]
async fn every_links_confidence_is_the_joins() {
    let reader = one_cluster(|_| {});
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let mut plan = plan(&[]);
    plan.followed = Some("f-new".into());
    plan.build_namespace = None;
    plan.argocd = "cluster-key".into();
    plan.environment = "cluster-key".into();
    let clusters = crate::delivery::collect::Clusters {
        kargo: &reader,
        argocd: &reader,
        tekton: &reader,
        environment: &reader,
        github: &FixtureGitHub::default(),
    };
    let trail = join(&crate::delivery::collect::collect(&clusters, &plan, observed_at()).await);
    for (key, hop) in [
        ("build-run-0", Hop::PipelineRun),
        ("dev-argocd-0", Hop::Application),
        ("dev-rollout-1", Hop::Rollout),
        ("dev-pods-2", Hop::Pod),
    ] {
        let shown = change
            .hops
            .iter()
            .find(|row| row.key == key)
            .unwrap_or_else(|| panic!("{key}"));
        let joined = trail
            .links
            .iter()
            .find(|link| link.to == hop)
            .unwrap_or_else(|| panic!("{hop:?}"));
        assert_eq!(shown.link, Some(joined.confidence), "{key}");
    }
}

#[tokio::test]
async fn a_freight_without_a_commit_still_reaches_its_pods() {
    let reader = one_cluster(|world| {
        let mut bare = freight("f-new", NEW, SHA);
        bare["commits"] = json!([]);
        world.kargo = std::mem::take(&mut world.kargo).with("freights", vec![bare]);
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let build = &change.hops[0];
    assert_eq!(build.group, 0);
    assert_eq!(build.state, HealthIndicator::Unknown);
    assert_eq!(build.link, Some(Confidence::Unknown));
    let freight = change
        .hops
        .iter()
        .find(|hop| hop.key == "freight")
        .expect("freight");
    assert_eq!(freight.state, HealthIndicator::Unknown);
    assert!(
        change.hops.iter().any(|hop| hop.name == "Pods"),
        "the followed Freight's Stage is read without a commit"
    );
    assert!(
        !reader
            .requests
            .borrow()
            .iter()
            .any(|r| r.contains("pipelineruns")),
        "no build is looked for without a commit"
    );
}

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
async fn an_application_in_a_cluster_that_isnt_read_leaves_its_pods_unknown() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).with(
            "applications",
            vec![application(Some("https://env-a.example:6443"))],
        );
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    assert_eq!(
        change.stages[0].cluster,
        Destination::Unknown(
            "server https://env-a.example:6443, not known to be this cluster".into()
        )
    );
    assert!(
        change
            .hops
            .iter()
            .filter(|hop| hop.group == 2 && hop.link.is_some())
            .skip(1)
            .all(|hop| hop.state == HealthIndicator::Unknown),
        "{:#?}",
        change.hops
    );
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
async fn a_freight_that_cant_be_read_says_why() {
    let missing = read(&one_cluster(|_| {}), &place("f-gone"), observed_at())
        .await
        .expect_err("no such Freight");
    assert!(
        missing.contains("f-gone") && missing.contains("wasn't found"),
        "{missing}"
    );

    let reader = one_cluster(|world| {
        world.kargo = std::mem::take(&mut world.kargo).refusing("freights");
    });
    let refused = read(&reader, &place("f-new"), observed_at())
        .await
        .expect_err("refused");
    assert!(refused.contains("refused"), "{refused}");
}

fn row<'a>(change: &'a Change, key: &str) -> &'a Row {
    change
        .hops
        .iter()
        .find(|hop| hop.key == key)
        .unwrap_or_else(|| {
            let keys: Vec<&str> = change.hops.iter().map(|hop| hop.key.as_str()).collect();
            panic!("{key} in {keys:?}")
        })
}

#[tokio::test]
async fn refused_applications_leave_the_destination_unknown_and_say_where() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).refusing("applications");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let dev = &change.stages[0];
    let Destination::Unknown(why) = &dev.cluster else {
        panic!("{:?}", dev.cluster);
    };
    assert!(
        why.starts_with("Argo CD Applications in argocd weren't read") && why.contains("forbidden"),
        "{why}"
    );
    assert!(change.groups[2].detail.starts_with("destination unknown: "));

    // The join's Unknown link names the Stage: the row says what wasn't found.
    let app = row(&change, "dev-argocd-0");
    assert_eq!(app.name, "Application not found");
    assert_eq!(app.state, HealthIndicator::Unknown);
    let Shows::Hop(detail) = &app.shows else {
        panic!("a hop");
    };
    assert_eq!(detail.title, "Application not found");
    assert!(
        detail
            .fields
            .iter()
            .any(|f| f.label == "Kargo Stage" && f.value == Shown::Mono("storefront/dev".into())),
        "{:?}",
        detail.fields
    );
    assert!(
        detail.fields.iter().all(|f| f.label != "Name"),
        "the Stage isn't shown as the Application: {:?}",
        detail.fields
    );
}

#[tokio::test]
async fn no_application_for_the_stage_is_unknown_not_another_cluster() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).with("applications", Vec::new());
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    assert_eq!(
        change.stages[0].cluster,
        Destination::Unknown("no Argo CD Application in argocd was found for it".into())
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

#[tokio::test]
async fn a_degraded_rollout_and_unready_pods_are_shown_as_they_are() {
    let image_id = format!("docker-pullable://{REPO}@{NEW}");
    let reader = one_cluster(|world| {
        let mut degraded = rollout(&format!("{REPO}:v1.4.0"));
        degraded["status"]["phase"] = json!("Degraded");
        world.environment = std::mem::take(&mut world.environment)
            .with("rollouts", vec![degraded])
            .with(
                "pods",
                vec![pod_of(
                    "storefront-5d9c-a",
                    "5d9c",
                    &format!("{REPO}:v1.4.0"),
                    &image_id,
                    false,
                )],
            );
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let rollout = row(&change, "dev-rollout-1");
    assert_eq!(
        rollout.state,
        HealthIndicator::Error,
        "joined, and degraded all the same"
    );
    assert_ne!(rollout.link, Some(Confidence::Unknown));
    let pods = row(&change, "dev-pods-2");
    let Shows::Hop(detail) = &pods.shows else {
        panic!("a hop");
    };
    assert_eq!(
        (pods.state, detail.state.as_str()),
        (HealthIndicator::Warning, "None ready")
    );
}

#[tokio::test]
async fn a_commit_from_an_image_revision_says_why_no_build_joined() {
    let reader = one_cluster(|world| {
        let mut bare = freight("f-new", NEW, SHA);
        bare["commits"] = json!([]);
        bare["images"][0]["annotations"] = json!({"org.opencontainers.image.revision": SHA});
        world.kargo = std::mem::take(&mut world.kargo).with("freights", vec![bare]);
        world.tekton = std::mem::take(&mut world.tekton).with("pipelineruns", Vec::new());
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let freight = row(&change, "freight");
    assert!(
        freight
            .detail
            .contains("from an image's revision annotation")
            && freight.detail.contains("no PipelineRun"),
        "{}",
        freight.detail
    );
    assert!(
        !freight.detail.contains("names no commit"),
        "{}",
        freight.detail
    );
}
