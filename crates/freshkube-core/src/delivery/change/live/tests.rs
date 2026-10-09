use serde_json::{Value, json};

use super::super::{
    Change, Destination, Eligible, Hop as Row, Phase, Promotion, Shows, Value as Shown,
};
use super::{Mapping, Place, read};
use crate::delivery::argocd::IN_CLUSTER_SERVER;
use crate::delivery::fixtures::*;
use crate::delivery::join::{Confidence, Hop, join};
use crate::delivery::tests::{observed_at, plan};
use crate::indicators::HealthIndicator;
use crate::workspace::{Destination as MappedRow, Key};

fn place(freight: &str) -> Place {
    Place {
        cluster: "cluster-key".into(),
        label: "core".into(),
        project: "storefront".into(),
        freight: freight.into(),
        argocd_namespace: "argocd".into(),
        mapping: Mapping::default(),
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
            "server https://env-a.example:6443 isn't mapped to a workspace cluster".into()
        )
    );
    assert!(
        change
            .hops
            .iter()
            .any(|hop| hop.detail.contains("map it in Settings › Workspace")),
        "the app says where to map it, not the spike's flags: {:#?}",
        change.hops
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

    // The join's Unknown link names the Stage: the row says what wasn't
    // read, since the list was refused.
    let app = row(&change, "dev-argocd-0");
    assert_eq!(app.name, "Application not read");
    assert_eq!(app.state, HealthIndicator::Unknown);
    let Shows::Hop(detail) = &app.shows else {
        panic!("a hop");
    };
    assert_eq!(detail.title, "Application not read");
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
    // The list was read, so the row says what wasn't found.
    assert_eq!(row(&change, "dev-argocd-0").name, "Application not found");
}

#[tokio::test]
async fn a_capped_application_list_says_the_application_may_be_past_the_cap() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd)
            .with("applications", Vec::new())
            .capped("applications");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let Destination::Unknown(why) = &change.stages[0].cluster else {
        panic!("{:?}", change.stages[0].cluster);
    };
    assert!(
        why.starts_with(
            "no Argo CD Application in argocd was found for it; the listing stopped at the page cap"
        ),
        "{why}"
    );
    assert_eq!(row(&change, "dev-argocd-0").name, "Application not found");
}

#[tokio::test]
async fn pods_of_a_freight_without_a_digest_are_unknown_not_another_digest() {
    let reader = one_cluster(|world| {
        let mut tagged = freight("f-new", NEW, SHA);
        tagged["images"][0]
            .as_object_mut()
            .expect("an image")
            .remove("digest");
        world.kargo = std::mem::take(&mut world.kargo).with("freights", vec![tagged]);
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");

    let pods = change
        .hops
        .iter()
        .find(|hop| hop.name == "Pods")
        .expect("the pods are reached");
    let Shows::Hop(detail) = &pods.shows else {
        panic!("a hop");
    };
    assert_eq!(pods.link, Some(Confidence::Claimed));
    assert_eq!(
        (pods.state, detail.state.as_str()),
        (
            HealthIndicator::Unknown,
            "The Freight names no image digest"
        )
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

/// Every link out of the change: the hop's key, the button, where it
/// leads (an address, or the pod and container of a Logs), and why it's
/// greyed out.
fn links_out(change: &Change) -> Vec<(String, String, Option<String>, Option<String>)> {
    use super::super::Target;
    let mut out = Vec::new();
    for hop in &change.hops {
        let Shows::Hop(detail) = &hop.shows else {
            continue;
        };
        for action in &detail.actions {
            let (label, to) = match &action.target {
                Target::Browser { label, address } => {
                    (label.clone(), address.as_ref().map(|a| a.to_string()))
                }
                Target::Logs {
                    what,
                    pod,
                    container,
                } => (
                    format!("Logs of {what}"),
                    Some(format!(
                        "{}/{}/{} {container:?}",
                        pod.cluster, pod.namespace, pod.name
                    )),
                ),
                Target::Resource { .. } => continue,
            };
            out.push((hop.key.clone(), label, to, action.disabled.clone()));
        }
    }
    out
}

fn link_out<'a>(
    links: &'a [(String, String, Option<String>, Option<String>)],
    label: &str,
) -> &'a (String, String, Option<String>, Option<String>) {
    links
        .iter()
        .find(|(_, l, _, _)| l == label)
        .unwrap_or_else(|| panic!("{label} in {links:#?}"))
}

fn config_map(namespace: &str, name: &str, key: &str, value: &str) -> Value {
    json!({
        "metadata": {"name": name, "namespace": namespace},
        "data": {key: value, "other": "kept out"}
    })
}

/// The run as Pipelines as Code annotates a pull request's, and its task
/// as Tekton records the pod it ran in.
fn annotated(world: &mut World, provider: &str, sha_url: &str) {
    annotated_with(
        world,
        provider,
        sha_url,
        "https://console.example.test/runs/storefront-push-x",
    );
}

/// [`annotated`], with the run's console page at `log_url`.
fn annotated_with(world: &mut World, provider: &str, sha_url: &str, log_url: &str) {
    let mut run = pipeline_run(SHA, true, Some(NEW));
    let annotations = run["metadata"]["annotations"]
        .as_object_mut()
        .expect("annotations");
    for (key, value) in [
        ("repo-url", "https://git.example/acme/storefront"),
        ("sha-url", sha_url),
        ("git-provider", provider),
        ("pull-request", "7"),
        ("log-url", log_url),
    ] {
        annotations.insert(format!("pipelinesascode.tekton.dev/{key}"), json!(value));
    }
    let mut task = task_run();
    task["metadata"]["name"] = json!("storefront-push-x-build");
    task["metadata"]["labels"]["tekton.dev/pipelineTask"] = json!("build");
    task["status"]["podName"] = json!("storefront-push-x-build-pod");
    task["status"]["steps"] = json!([{"container": "step-build"}, {"container": "step-push"}]);
    task["status"]["conditions"] = json!([{"type": "Succeeded", "status": "True"}]);
    world.tekton = std::mem::take(&mut world.tekton)
        .with("pipelineruns", vec![run])
        .with("taskruns", vec![task]);
}

#[tokio::test]
async fn links_out_are_made_from_what_the_cluster_records() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).with(
            "configmaps",
            vec![
                // User info, a query and a fragment never leave the cluster.
                config_map(
                    "argocd",
                    "argocd-cm",
                    "url",
                    "https://admin:pw@argocd.example.test/?token=abc#x",
                ),
                config_map(
                    "kargo",
                    "kargo-api",
                    "ADMIN_ACCOUNT_TOKEN_ISSUER",
                    "https://kargo.example.test",
                ),
            ],
        );
        annotated(
            world,
            "github",
            "https://git.example/acme/storefront/commit/abc",
        );
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);

    let freight = "https://kargo.example.test/project/storefront/freight/f-new";
    assert_eq!(
        change.page.as_ref().map(|a| a.to_string()),
        Ok(freight.into())
    );
    let kargo = link_out(&links, "Open in Kargo");
    assert_eq!((kargo.2.as_deref(), &kargo.3), (Some(freight), &None));

    let argocd = link_out(&links, "Open in Argo CD");
    assert_eq!(
        argocd.2.as_deref(),
        Some("https://argocd.example.test/applications/argocd/storefront-dev")
    );
    assert_eq!(
        link_out(&links, "Open the pull request").2.as_deref(),
        Some("https://git.example/acme/storefront/pull/7")
    );
    assert_eq!(
        link_out(&links, "Open the commit").2.as_deref(),
        Some("https://git.example/acme/storefront/commit/abc")
    );
    assert_eq!(
        link_out(&links, "Open the run").2.as_deref(),
        Some("https://console.example.test/runs/storefront-push-x")
    );
    let logs = link_out(&links, "Logs of build");
    assert_eq!(
        (logs.2.as_deref(), &logs.3),
        (
            Some("cluster-key/acme-builds/storefront-push-x-build-pod Some(\"step-build\")"),
            &None
        )
    );

    // Only the two ConfigMaps were read beside the change: never a Secret.
    let requests = reader.requests.borrow();
    assert!(
        !requests.iter().any(|r| r.contains("secrets")),
        "{requests:#?}"
    );
    let maps: Vec<&String> = requests
        .iter()
        .filter(|r| r.contains("configmaps"))
        .collect();
    assert_eq!(
        maps,
        [
            "GET /configmaps Some(\"argocd\") argocd-cm",
            "GET /configmaps Some(\"kargo\") kargo-api"
        ]
    );
}

#[tokio::test]
async fn links_without_a_recorded_address_are_greyed_out_with_why() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).refusing("configmaps");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);

    let why = change.page.as_ref().expect_err("no Kargo page");
    assert!(
        why.starts_with("Kargo's address isn't known: ConfigMap kargo-api in kargo is refused"),
        "{why}"
    );
    let argocd = link_out(&links, "Open in Argo CD");
    assert_eq!(argocd.2, None);
    assert!(
        argocd.3.as_deref().is_some_and(|why| why.starts_with(
            "Argo CD's address isn't known: ConfigMap argocd-cm in argocd is refused"
        )),
        "{argocd:?}"
    );
    // The healthy run carries no Pipelines as Code addresses.
    assert_eq!(
        link_out(&links, "Open the commit").3.as_deref(),
        Some("The run has no Pipelines as Code sha-url")
    );
}

#[tokio::test]
async fn only_http_and_https_addresses_open() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).with(
            "configmaps",
            vec![
                config_map("argocd", "argocd-cm", "url", "javascript:alert(1)"),
                config_map(
                    "kargo",
                    "kargo-api",
                    "ADMIN_ACCOUNT_TOKEN_ISSUER",
                    "kargo.example.test",
                ),
            ],
        );
        annotated(world, "bitbucket-datacenter", "file:///etc/passwd");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);

    for (label, why) in [
        (
            "Open in Argo CD",
            "Argo CD's address isn't known: url of ConfigMap argocd-cm in argocd: it isn't an http or https address",
        ),
        (
            "Open in Kargo",
            "Kargo's address isn't known: ADMIN_ACCOUNT_TOKEN_ISSUER of ConfigMap kargo-api in kargo: it isn't an address",
        ),
        (
            "Open the commit",
            "The run's sha-url: it isn't an http or https address",
        ),
        (
            "Open the pull request",
            "Pipelines as Code's provider bitbucket-datacenter has no known pull request page",
        ),
    ] {
        let link = link_out(&links, label);
        assert_eq!(
            (link.2.as_deref(), link.3.as_deref()),
            (None, Some(why)),
            "{label}"
        );
    }
    assert!(change.page.is_err());
}

/// One run's links, read from a cluster whose run Pipelines as Code
/// annotated with `provider`, `sha_url` and `log_url`.
async fn run_links(
    provider: &str,
    sha_url: &str,
    log_url: &str,
) -> Vec<(String, String, Option<String>, Option<String>)> {
    let reader = one_cluster(|world| annotated_with(world, provider, sha_url, log_url));
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    links_out(&change)
}

#[tokio::test]
async fn each_provider_has_its_own_pull_request_page() {
    for (provider, page) in [
        ("github", "https://git.example/acme/storefront/pull/7"),
        ("gitea", "https://git.example/acme/storefront/pulls/7"),
        ("forgejo", "https://git.example/acme/storefront/pulls/7"),
        (
            "gitlab",
            "https://git.example/acme/storefront/-/merge_requests/7",
        ),
        (
            "bitbucket-cloud",
            "https://git.example/acme/storefront/pull-requests/7",
        ),
    ] {
        let links = run_links(
            provider,
            "https://git.example/c",
            "https://console.example.test/r",
        )
        .await;
        let pull = link_out(&links, "Open the pull request");
        assert_eq!(
            (pull.2.as_deref(), &pull.3),
            (Some(page), &None),
            "{provider}"
        );
    }
}

/// Pipelines as Code names a fallback console when none is configured;
/// it serves no page, so the run's link is greyed out.
async fn assert_no_console(log_url: &str) {
    let links = run_links("github", "https://git.example/c", log_url).await;
    let run = link_out(&links, "Open the run");
    assert_eq!(
        (run.2.as_deref(), run.3.as_deref()),
        (None, Some("Pipelines as Code has no dashboard configured")),
        "{log_url}"
    );
}

#[tokio::test]
async fn pipelines_as_codes_fallback_console_is_no_page() {
    assert_no_console(
        "https://dashboard.url.is.not.configured/#/namespaces/acme-builds/pipelineruns/x",
    )
    .await;
}

/// Some releases write the fallback without `url`.
#[tokio::test]
async fn pipelines_as_codes_shorter_fallback_console_is_no_page() {
    assert_no_console("https://dashboard.is.not.configured/acme-builds/x").await;
}

/// The Tekton Dashboard's run page is a route in the fragment: it is kept,
/// without its query, where every other link drops its fragment.
#[tokio::test]
async fn the_dashboards_run_route_is_kept() {
    let links = run_links(
        "github",
        "https://git.example/c#L1",
        "https://dashboard.example.test/#/namespaces/acme-builds/pipelineruns/x?pipelineTask=build",
    )
    .await;
    assert_eq!(
        link_out(&links, "Open the run").2.as_deref(),
        Some("https://dashboard.example.test/#/namespaces/acme-builds/pipelineruns/x")
    );
    assert_eq!(
        link_out(&links, "Open the commit").2.as_deref(),
        Some("https://git.example/c")
    );
}

/// A commit page on another host than the repository isn't taken for it.
#[tokio::test]
async fn a_commit_on_another_host_is_greyed_out() {
    let links = run_links(
        "github",
        "https://elsewhere.example.test/acme/storefront/commit/abc",
        "https://console.example.test/r",
    )
    .await;
    let commit = link_out(&links, "Open the commit");
    assert_eq!(
        (commit.2.as_deref(), commit.3.as_deref()),
        (
            None,
            Some("The run's sha-url names another host than its repo-url")
        )
    );
}

/// A ConfigMap GET that hangs greys its links out after its own deadline;
/// the change is still read.
#[tokio::test(start_paused = true)]
async fn a_hanging_address_read_greys_its_links_out_not_the_change() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).hanging("configmaps");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("the change is read");
    let why = change.page.as_ref().expect_err("no Kargo page");
    assert_eq!(
        why,
        "Kargo's address isn't known: ConfigMap kargo-api in kargo didn't answer within 5 s"
    );
    let argocd = link_out(&links_out(&change), "Open in Argo CD").clone();
    assert_eq!(
        argocd.3.as_deref(),
        Some(
            "Argo CD's address isn't known: ConfigMap argocd-cm in argocd didn't answer within 5 s"
        )
    );
    assert!(!change.groups.is_empty());
}

/// Without a failed task, the logs are the task's that ended last, by its
/// completion time rather than the order the TaskRuns were listed in.
#[tokio::test]
async fn the_logs_are_the_task_that_ended_last() {
    let reader = one_cluster(|world| {
        annotated(world, "github", "https://git.example/c");
        let mut tasks = Vec::new();
        for (task, ended) in [
            ("push", "2026-01-01T10:05:00Z"),
            ("build", "2026-01-01T10:09:00Z"),
            ("lint", "2026-01-01T10:01:00Z"),
        ] {
            let mut run = task_run();
            run["metadata"]["name"] = json!(format!("storefront-push-x-{task}"));
            run["metadata"]["labels"]["tekton.dev/pipelineTask"] = json!(task);
            run["status"]["podName"] = json!(format!("storefront-push-x-{task}-pod"));
            run["status"]["steps"] = json!([{"container": format!("step-{task}")}]);
            run["status"]["conditions"] = json!([{"type": "Succeeded", "status": "True"}]);
            run["status"]["completionTime"] = json!(ended);
            tasks.push(run);
        }
        world.tekton = std::mem::take(&mut world.tekton).with("taskruns", tasks);
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);
    assert_eq!(
        link_out(&links, "Logs of build").2.as_deref(),
        Some("cluster-key/acme-builds/storefront-push-x-build-pod Some(\"step-build\")")
    );
}

/// The workspace `core` (open), `prod` and `stage`, with these rows.
fn mapped(rows: &[(Key, &str)]) -> Mapping {
    Mapping {
        open: Some("core".into()),
        entries: vec!["core".into(), "prod".into(), "stage".into()],
        destinations: rows
            .iter()
            .map(|(key, entry)| MappedRow::new(key.clone(), *entry))
            .collect(),
    }
}

/// The change, its Application deploying to `server`, or to Argo CD's
/// cluster `env-dev` without one, read with `mapping`.
async fn read_mapped(server: Option<&str>, mapping: Mapping) -> Change {
    let reader = one_cluster(|world| {
        world.argocd =
            std::mem::take(&mut world.argocd).with("applications", vec![application(server)]);
    });
    let mut place = place("f-new");
    place.mapping = mapping;
    read(&reader, &place, observed_at()).await.expect("read")
}

const ENV_A: &str = "https://env-a.example:6443";

fn by_server() -> Key {
    // Written as the person might: case, a trailing slash.
    Key::Server("https://ENV-A.example:6443/".into())
}

fn by_name() -> Key {
    Key::Name("env-dev".into())
}

#[tokio::test]
async fn a_destination_mapped_by_server_names_its_workspace_cluster() {
    let change = read_mapped(Some(ENV_A), mapped(&[(by_server(), "prod")])).await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Entry {
            entry: "prod".into(),
            via: format!("server {ENV_A}"),
        }
    );
    assert_eq!(change.groups[2].detail, "deploys to prod (mapped)");

    // What Argo CD reports there, its claim, opening on prod; the Service it
    // also lists is no workload.
    let reported: Vec<&Row> = change
        .hops
        .iter()
        .filter(|hop| hop.key.starts_with("dev-reported-"))
        .collect();
    assert_eq!(reported.len(), 1, "{:#?}", change.hops);
    let rollout = reported[0];
    assert_eq!(rollout.name, "Rollout storefront");
    assert_eq!(rollout.group, 2);
    assert_eq!(rollout.link, Some(Confidence::Claimed));
    assert_eq!(rollout.from, "core", "Argo CD was read on the open cluster");
    let Shows::Hop(detail) = &rollout.shows else {
        panic!("a hop")
    };
    assert_eq!(
        detail.link.as_ref().map(|link| link.confidence),
        Some(Confidence::Claimed)
    );
    let object = detail
        .actions
        .iter()
        .find_map(|action| match &action.target {
            super::super::Target::Resource { object, .. } => Some(object),
            _ => None,
        })
        .expect("it opens");
    assert_eq!(
        (
            object.cluster.as_str(),
            object.group.as_str(),
            object.version.as_str(),
            object.plural.as_str(),
            object.namespace.as_str(),
            object.name.as_str(),
        ),
        (
            "prod",
            "argoproj.io",
            "v1alpha1",
            "rollouts",
            "shop",
            "storefront"
        )
    );
    // Nothing on prod was read, so no row says it was not the environment.
    assert!(
        !change
            .hops
            .iter()
            .any(|hop| hop.group == 2 && hop.detail.contains("nothing was read there")),
        "{:#?}",
        change.hops
    );
}

#[tokio::test]
async fn a_destination_mapped_by_name_names_its_workspace_cluster() {
    let change = read_mapped(None, mapped(&[(by_name(), "prod")])).await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Entry {
            entry: "prod".into(),
            via: "name env-dev".into(),
        }
    );
    // A server row doesn't match a destination given by name.
    let change = read_mapped(None, mapped(&[(Key::Server(ENV_A.into()), "prod")])).await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Unknown(
            "Argo CD's cluster env-dev isn't mapped to a workspace cluster".into()
        )
    );
}

#[tokio::test]
async fn an_unmapped_destination_stays_unknown_with_why() {
    let change = read_mapped(Some(ENV_A), mapped(&[(by_name(), "prod")])).await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Unknown(format!(
            "server {ENV_A} isn't mapped to a workspace cluster"
        ))
    );
    assert!(!change.hops.iter().any(|hop| hop.key.contains("-reported-")));
    let change = read_mapped(None, Mapping::default()).await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Unknown(
            "Argo CD's cluster env-dev isn't mapped to a workspace cluster".into()
        )
    );
}

#[tokio::test]
async fn a_destination_mapped_to_a_cluster_no_longer_listed_is_unknown() {
    let change = read_mapped(Some(ENV_A), mapped(&[(by_server(), "gone")])).await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Unknown("mapped to gone, which the workspace no longer lists".into())
    );
    assert!(!change.hops.iter().any(|hop| hop.key.contains("-reported-")));
}

#[tokio::test]
async fn two_destinations_mapped_to_one_cluster_both_name_it() {
    let rows = [(by_server(), "prod"), (by_name(), "prod")];
    for (server, via) in [
        (Some(ENV_A), format!("server {ENV_A}")),
        (None, "name env-dev".to_owned()),
    ] {
        let change = read_mapped(server, mapped(&rows)).await;
        assert_eq!(
            change.stages[0].cluster,
            Destination::Entry {
                entry: "prod".into(),
                via,
            }
        );
    }
}

#[tokio::test]
async fn one_destination_mapped_twice_is_never_guessed() {
    let change = read_mapped(
        Some(ENV_A),
        mapped(&[(by_server(), "stage"), (Key::Server(ENV_A.into()), "prod")]),
    )
    .await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Unknown(
            "its destination is mapped to more than one workspace cluster (prod, stage)".into()
        )
    );
}

#[tokio::test]
async fn a_destination_mapped_to_the_open_cluster_is_read_there() {
    let change = read_mapped(Some(ENV_A), mapped(&[(by_server(), "core")])).await;
    assert_eq!(
        change.stages[0].cluster,
        Destination::Cluster("core".into())
    );
    let pods = change
        .hops
        .iter()
        .find(|hop| hop.name == "Pods")
        .expect("the pods are reached");
    assert_eq!(pods.state, HealthIndicator::Healthy, "its pods were read");
    assert!(!change.hops.iter().any(|hop| hop.key.contains("-reported-")));
}
