use freshkube_core::resources::FailureKind;
use serde_json::{Value, json};

use super::super::{
    Change, Destination, Eligible, Hop as Row, Phase, Promotion, Reach, Shows, Value as Shown,
};
use super::derive::reach;
use super::{Mapping, Place, clusters, plan as plan_of, read};
use crate::argocd::IN_CLUSTER_SERVER;
use crate::collect::collect;
use crate::fixtures::*;
use crate::join::{Confidence, Hop, Key as JoinKey, Link, join};
use crate::kargo::read_freight;
use crate::tests::{observed_at, plan};
use freshkube_core::indicators::HealthIndicator;
use freshkube_core::workspace::{Destination as MappedRow, Key};

pub(super) fn place(freight: &str) -> Place {
    Place {
        cluster: "cluster-key".into(),
        label: "core".into(),
        project: "storefront".into(),
        freight: freight.into(),
        argocd_namespace: "argocd".into(),
        mapping: Mapping::default(),
        sources: Default::default(),
        public_key_file: None,
        transparency_log: false,
        transparency_url: None,
        evidence_result: None,
        build_namespace: None,
        stage_naming: None,
    }
}

/// A Stage of storefront taking Freight from the Warehouse (`upstream`
/// empty) or from the Stages named, holding `current`.
pub(super) fn stage_of(name: &str, upstream: &[&str], current: &[&str]) -> Value {
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
pub(super) fn one_cluster(edit: impl FnOnce(&mut World)) -> FixtureReader {
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
    let clusters = crate::collect::Clusters {
        kargo: &reader,
        argocd: &reader,
        tekton: &reader,
        environment: &reader,
        github: &FixtureGitHub::default(),
    };
    let trail = join(&crate::collect::collect(&clusters, &plan, observed_at()).await);
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
    // Until Settings can map it, the reasons keep the spike's words.
    assert!(
        change
            .hops
            .iter()
            .any(|hop| hop.detail.contains("--known-as")),
        "{:#?}",
        change.hops
    );
    let change = read_mapped(Some("https://env-a.example:6443"), mapped(&[])).await;
    assert!(
        change
            .hops
            .iter()
            .any(|hop| hop.detail.contains("map it in Settings › Workspace")),
        "with Settings, the app says where to map it, not the spike's flags: {:#?}",
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
        missing.message.contains("f-gone") && missing.message.contains("wasn't found"),
        "{missing}"
    );

    let reader = one_cluster(|world| {
        world.kargo = std::mem::take(&mut world.kargo).refusing("freights");
    });
    let refused = read(&reader, &place("f-new"), observed_at())
        .await
        .expect_err("refused");
    assert_eq!(refused.kind, FailureKind::Forbidden);
    assert!(refused.message.contains("refused"), "{refused}");
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
                Target::Revision { deployment, hash } => (
                    "Coroot revision".to_owned(),
                    Some(format!(
                        "{}/{}/{} {hash:?}",
                        deployment.cluster, deployment.namespace, deployment.name
                    )),
                ),
                Target::Resource { .. } | Target::OnEntry { .. } => continue,
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
    // The open entry's id differs from the place's label, so a test can
    // tell which one a row names.
    Mapping {
        open: Some("home".into()),
        entries: vec!["home".into(), "prod".into(), "stage".into()],
        destinations: rows
            .iter()
            .map(|(key, entry)| MappedRow::new(key.clone(), *entry))
            .collect(),
        settings: true,
    }
}

/// The change, its Application deploying to `server`, or to Argo CD's
/// cluster `env-dev` without one, read with `mapping`.
async fn read_mapped(server: Option<&str>, mapping: Mapping) -> Change {
    read_app(application(server), mapping).await
}

/// The change, its one Application `app`, read with `mapping`.
async fn read_app(app: Value, mapping: Mapping) -> Change {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).with("applications", vec![app]);
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
    let (entry, object) = detail
        .actions
        .iter()
        .find_map(|action| match &action.target {
            super::super::Target::OnEntry { entry, object, .. } => Some((entry, object)),
            _ => None,
        })
        .expect("it opens on its workspace cluster");
    assert_eq!(
        object.cluster, "",
        "a workspace entry is never a connection"
    );
    assert_eq!(
        (
            entry.as_str(),
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
    // Its rows say so as the Stage does.
    let rows: Vec<&str> = change
        .hops
        .iter()
        .filter(|hop| hop.group == 2 && hop.detail.contains("nothing was read there"))
        .map(|hop| hop.detail.as_str())
        .collect();
    assert!(!rows.is_empty(), "{:#?}", change.hops);
    assert!(
        rows.iter()
            .all(|detail| detail
                .ends_with("it is mapped to gone, which the workspace no longer lists")),
        "{rows:#?}"
    );
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
    let change = read_mapped(Some(ENV_A), mapped(&[(by_server(), "home")])).await;
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

/// What Argo CD reports of a workload nobody read is its claim: Healthy is
/// never more than Info, so the Stage doesn't fold as fine.
#[tokio::test]
async fn a_reported_workload_takes_argo_cds_health_no_higher_than_info() {
    let mut app = application(Some(ENV_A));
    app["status"]["resources"] = json!([
        {"group": "apps", "version": "v1", "kind": "Deployment", "namespace": "shop", "name": "web", "health": {"status": "Healthy"}},
        {"group": "apps", "version": "v1", "kind": "StatefulSet", "namespace": "shop", "name": "db", "health": {"status": "Degraded"}},
        {"group": "apps", "version": "v1", "kind": "DaemonSet", "namespace": "shop", "name": "agent", "health": {"status": "Missing"}},
        {"group": "argoproj.io", "kind": "Rollout", "namespace": "shop", "name": "storefront", "health": {"status": "Progressing"}},
        {"group": "", "kind": "ConfigMap", "namespace": "shop", "name": "settings", "health": {"status": "Healthy"}},
        {"group": "batch", "kind": "Job", "namespace": "shop", "name": "migrate"}
    ]);
    let change = read_app(app, mapped(&[(by_server(), "prod")])).await;
    let reported: Vec<(&str, HealthIndicator)> = change
        .hops
        .iter()
        .filter(|hop| hop.key.starts_with("dev-reported-"))
        .map(|hop| (hop.name.as_str(), hop.state))
        .collect();
    assert_eq!(
        reported,
        vec![
            ("Deployment web", HealthIndicator::Info),
            ("StatefulSet db", HealthIndicator::Warning),
            ("DaemonSet agent", HealthIndicator::Warning),
            ("Rollout storefront", HealthIndicator::Info),
        ],
        "only workloads, each worded as Argo CD's claim"
    );
    assert!(
        change
            .hops
            .iter()
            .any(|hop| hop.group == 2 && hop.state != HealthIndicator::Healthy),
        "a Stage known only from Argo CD's claim isn't all fine: {:#?}",
        change.hops
    );
}

#[test]
fn a_mapping_is_the_workspace_with_the_open_entry() {
    let workspace = freshkube_core::workspace::example();
    let mapping = Mapping::of(&workspace, Some("core-fra"));
    assert_eq!(mapping.open.as_deref(), Some("core-fra"));
    assert_eq!(
        mapping.entries,
        workspace
            .clusters
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(mapping.destinations, workspace.destinations);
    assert!(mapping.settings, "the app's Settings can map them");
    assert_eq!(Mapping::of(&workspace, None).open, None);
}

/// [`one_cluster`] with its Application deploying the storefront
/// Deployment, whose current ReplicaSet is `sets`' revision `2`.
fn deployed(sets: Vec<Value>) -> FixtureReader {
    deployed_running(sets, NEW)
}

/// [`deployed`], its pod reporting `running` as its image's digest.
fn deployed_running(sets: Vec<Value>, running: &str) -> FixtureReader {
    let image = format!("{REPO}@{NEW}");
    let id = format!("docker-pullable://{REPO}@{running}");
    one_cluster(|world| {
        let mut app = application(Some(IN_CLUSTER_SERVER));
        app["status"]["resources"] = json!([
            {"group": "apps", "kind": "Deployment", "namespace": "shop", "name": "storefront"}]);
        world.argocd = std::mem::take(&mut world.argocd).with("applications", vec![app]);
        world.environment = FixtureReader::default()
            .with("deployments", vec![deployment(&image)])
            .with("replicasets", sets)
            .with(
                "pods",
                vec![deployment_pod(
                    "storefront-6fdf-x",
                    "6fdf",
                    &image,
                    &id,
                    true,
                )],
            );
    })
}

/// The hop's field labelled `label`, as shown.
fn field<'a>(hop: &'a Row, label: &str) -> Option<&'a Shown> {
    let Shows::Hop(detail) = &hop.shows else {
        return None;
    };
    detail
        .fields
        .iter()
        .find(|field| field.label == label)
        .map(|field| &field.value)
}

#[tokio::test]
async fn a_deployment_names_coroots_revision_by_its_current_replica_sets_hash() {
    let image = format!("{REPO}@{NEW}");
    let reader = deployed(vec![
        deployment_set("4b1c", "1", &image, 0, 0),
        deployment_set("6fdf", "2", &image, 1, 1),
    ]);
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);
    let (key, _, to, disabled) = link_out(&links, "Coroot revision");
    assert_eq!(
        to.as_deref(),
        Some("cluster-key/shop/storefront Some(\"6fdf\")"),
        "the current revision's hash, not an older one's"
    );
    assert_eq!(disabled, &None);
    let hop = change.hop(key).expect("the Deployment's hop");
    assert!(hop.name.starts_with("Deployment"), "{}", hop.name);
    assert_eq!(
        field(hop, "Pod template hash"),
        Some(&Shown::Mono("6fdf".into()))
    );
}

#[tokio::test]
async fn a_deployment_whose_pods_run_another_digest_does_not_offer_its_revision() {
    let image = format!("{REPO}@{NEW}");
    let reader = deployed_running(vec![deployment_set("6fdf", "2", &image, 1, 1)], OLD);
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);
    let (key, _, to, disabled) = link_out(&links, "Coroot revision");
    assert_eq!(
        to.as_deref(),
        Some("cluster-key/shop/storefront Some(\"6fdf\")")
    );
    assert_eq!(
        disabled.as_deref(),
        Some(
            "Its current revision isn't confirmed to run this Freight: see its Pods row. \
             Coroot's revision of it may be another change."
        ),
        "the spec pins the digest, but the pods say otherwise"
    );
    let hop = change.hop(key).expect("the Deployment's hop");
    assert_eq!(
        field(hop, "Pod template hash"),
        Some(&Shown::Mono("6fdf".into()))
    );
}

#[tokio::test]
async fn a_deployment_without_a_current_replica_set_says_why_its_revision_isnt_known() {
    let image = format!("{REPO}@{NEW}");
    let reader = deployed(vec![deployment_set("4b1c", "1", &image, 0, 0)]);
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);
    let (key, _, to, disabled) = link_out(&links, "Coroot revision");
    assert_eq!(to.as_deref(), Some("cluster-key/shop/storefront None"));
    assert_eq!(
        disabled.as_deref(),
        Some(
            "No ReplicaSet the Deployment owns carries its current revision 2, so Coroot's \
             revision of it isn't known."
        )
    );
    let hop = change.hop(key).expect("the Deployment's hop");
    assert_eq!(field(hop, "Pod template hash"), None);
}

#[tokio::test]
async fn a_rollouts_revision_is_greyed_out_since_coroot_keeps_deployments_only() {
    let reader = one_cluster(|_| {});
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);
    let (key, _, _, disabled) = link_out(&links, "Coroot revision");
    assert_eq!(key, "dev-rollout-1");
    assert_eq!(
        disabled.as_deref(),
        Some("Coroot keeps revisions of Deployments only, and this is an Argo Rollout.")
    );
}

// ---- How sure each Stage is, as the Changes list shows it ---------------

/// The revision [`application`] reports synced.
const SYNCED: &str = "cccccccccccccccccccccccccccccccccccccccc";

/// The dev Stage, recording `f-new` with its digest as Kargo does.
fn dev_holding_new() -> Value {
    let mut dev = stage_of("dev", &[], &["f-new"]);
    dev["status"]["freightHistory"][0]["items"]["Warehouse/storefront/f-new"]["images"] =
        json!([{"repoURL": REPO, "digest": NEW}]);
    dev
}

/// One cluster whose dev Stage runs `f-new` through an Application that
/// deploys the storefront Deployment there, pinned to the Freight's digest,
/// with its current pod running it. `edit` changes the Deployment.
fn on_deployment(edit: impl FnOnce(&mut Value)) -> World {
    on_deployment_at(IN_CLUSTER_SERVER, edit)
}

/// [`on_deployment`], its Application deploying to `server`.
pub(super) fn on_deployment_at(server: &str, edit: impl FnOnce(&mut Value)) -> World {
    let image = format!("{REPO}@{NEW}");
    let mut world = without_rollouts();
    let mut app = summarised(application(Some(server)), &[&image]);
    app["status"]["resources"] = json!([
        {"group": "apps", "kind": "Deployment", "namespace": "shop", "name": "storefront"}
    ]);
    world.argocd = std::mem::take(&mut world.argocd).with("applications", vec![app]);
    world.kargo = std::mem::take(&mut world.kargo)
        .with("stages", vec![dev_holding_new()])
        // Its Promotion pushed the commit Argo CD synced.
        .with(
            "promotions",
            vec![promotion_pushing("f-new", NEW, "Succeeded", SYNCED)],
        );
    let mut deployment = deployment(&image);
    edit(&mut deployment);
    world.environment =
        std::mem::take(&mut world.environment).with("deployments", vec![deployment]);
    world
}

async fn reach_of(world: World) -> Vec<(String, Reach)> {
    let change = read(&world.one_cluster(), &place("f-new"), observed_at())
        .await
        .expect("read");
    change
        .stages
        .iter()
        .map(|stage| (stage.name.clone(), stage.reach.clone()))
        .collect()
}

fn dev(reach: Reach) -> Vec<(String, Reach)> {
    vec![("dev".into(), reach)]
}

#[tokio::test]
async fn a_digest_that_reaches_the_pods_is_confirmed() {
    assert_eq!(
        reach_of(on_deployment(|_| {})).await,
        dev(Reach::Linked(Confidence::Confirmed))
    );
}

#[tokio::test]
async fn a_deployment_that_names_its_image_by_tag_only_is_claimed() {
    let world = on_deployment(|deployment| {
        deployment["spec"]["template"]["spec"]["containers"][0]["image"] =
            json!(format!("{REPO}:v1.4.0"));
    });
    assert_eq!(
        reach_of(world).await,
        dev(Reach::Linked(Confidence::Claimed))
    );
}

#[tokio::test]
async fn a_deployment_that_hasnt_observed_its_spec_is_claimed() {
    let world = on_deployment(|deployment| {
        deployment["status"]["observedGeneration"] = json!(2);
    });
    assert_eq!(
        reach_of(world).await,
        dev(Reach::Linked(Confidence::Claimed))
    );
}

#[tokio::test]
async fn refused_pods_are_refused_not_unknown() {
    let mut world = on_deployment(|_| {});
    world.environment = std::mem::take(&mut world.environment).refusing("pods");
    assert_eq!(reach_of(world).await, dev(Reach::Refused("Pods".into())));
}

#[tokio::test]
async fn refused_applications_are_refused_not_unknown() {
    let mut world = on_deployment(|_| {});
    world.argocd = std::mem::take(&mut world.argocd).refusing("applications");
    assert_eq!(
        reach_of(world).await,
        dev(Reach::Refused("Argo CD Applications".into()))
    );
}

#[tokio::test]
async fn a_stage_that_runs_it_without_an_application_is_unknown() {
    let mut world = on_deployment(|_| {});
    world.argocd = std::mem::take(&mut world.argocd).with("applications", Vec::new());
    assert_eq!(
        reach_of(world).await,
        dev(Reach::Linked(Confidence::Unknown))
    );
}

#[tokio::test]
async fn a_stage_that_doesnt_run_it_is_not_reached() {
    let mut world = on_deployment(|_| {});
    world.kargo = std::mem::take(&mut world.kargo).with(
        "stages",
        vec![dev_holding_new(), stage_of("staging", &["dev"], &["f-old"])],
    );
    assert_eq!(
        reach_of(world).await,
        vec![
            ("dev".into(), Reach::Linked(Confidence::Confirmed)),
            ("staging".into(), Reach::NotReached),
        ]
    );
}

#[tokio::test]
async fn refused_names_only_the_kinds_its_own_stage_reaches() {
    // dev's Rollout couldn't be read; staging's Deployments neither. Each
    // Stage names its own.
    let mut world = on_deployment(|_| {});
    let mut staging = application(Some(IN_CLUSTER_SERVER));
    staging["metadata"]["name"] = json!("storefront-staging");
    staging["metadata"]["annotations"]["kargo.akuity.io/authorized-stage"] =
        json!("storefront:staging");
    staging["status"]["resources"] = json!([
        {"group": "apps", "kind": "Deployment", "namespace": "shop", "name": "storefront"}
    ]);
    world.argocd = std::mem::take(&mut world.argocd).with(
        "applications",
        vec![application(Some(IN_CLUSTER_SERVER)), staging],
    );
    world.kargo = std::mem::take(&mut world.kargo).with(
        "stages",
        vec![
            stage_of("dev", &[], &["f-new"]),
            stage_of("staging", &["dev"], &["f-new"]),
        ],
    );
    world.environment = std::mem::take(&mut world.environment)
        .serves("argoproj.io", "v1alpha1", &["rollouts", "analysisruns"])
        .refusing("rollouts")
        .refusing("deployments");
    assert_eq!(
        reach_of(world).await,
        vec![
            ("dev".into(), Reach::Refused("Rollouts".into())),
            ("staging".into(), Reach::Refused("Deployments".into())),
        ]
    );
}

#[tokio::test]
async fn a_stage_on_another_entry_is_claimed_as_argo_cd_reports_it() {
    let change = read_mapped(Some(ENV_A), mapped(&[(by_server(), "prod")])).await;
    assert_eq!(
        change.stages[0].reach,
        Reach::Linked(Confidence::Claimed),
        "the reported Rollout stands for the cluster nothing read"
    );
}

#[tokio::test]
async fn a_stage_on_another_entry_that_reports_no_workload_is_unknown() {
    let mut app = application(Some(ENV_A));
    app["status"]["resources"] =
        json!([{"group": "", "kind": "Service", "namespace": "shop", "name": "storefront"}]);
    let change = read_app(app, mapped(&[(by_server(), "prod")])).await;
    assert_eq!(change.stages[0].reach, Reach::Linked(Confidence::Unknown));
}

/// A run through an Application read here, which reached no pods, and one
/// on another entry that Argo CD reports, stays Unknown: the one elsewhere
/// stands only for its own links. No join leaves a local Application without
/// a Pod link, so the run is built by hand.
#[tokio::test]
async fn a_local_application_without_pods_stays_unknown_beside_one_elsewhere() {
    let mut local = application(Some(IN_CLUSTER_SERVER));
    local["metadata"]["name"] = json!("storefront-local");
    let mut remote = application(Some(ENV_A));
    remote["metadata"]["name"] = json!("storefront-prod");
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).with("applications", vec![local, remote]);
    });
    let mut place = place("f-new");
    place.mapping = mapped(&[(by_server(), "prod")]);
    let freight = read_freight(&reader, &place.project, &place.freight)
        .await
        .expect("freight");
    let evidence = collect(
        &clusters(&reader),
        &plan_of(&place, &freight),
        observed_at(),
    )
    .await;
    let link = |from, to, subject: &str, confidence| Link {
        cause: None,
        from,
        to,
        subject: subject.into(),
        key: JoinKey::None,
        confidence,
        reason: String::new(),
        evidence: Vec::new(),
        conflicts: Vec::new(),
        signature_warn: false,
    };
    let stage = link(
        Hop::Freight,
        Hop::Stage,
        "storefront/dev",
        Confidence::Confirmed,
    );
    let to_remote = link(
        Hop::Stage,
        Hop::Application,
        "argocd/storefront-prod",
        Confidence::Claimed,
    );
    let reported = link(
        Hop::Application,
        Hop::Rollout,
        "argocd/storefront-prod",
        Confidence::Unknown,
    );
    let to_local = link(
        Hop::Stage,
        Hop::Application,
        "argocd/storefront-local",
        Confidence::Claimed,
    );
    assert_eq!(
        reach(&place, &evidence, true, &[&stage, &to_remote, &reported]),
        Reach::Linked(Confidence::Claimed),
        "alone, the one Argo CD reports is Claimed"
    );
    assert_eq!(
        reach(
            &place,
            &evidence,
            true,
            &[&stage, &to_remote, &reported, &to_local]
        ),
        Reach::Linked(Confidence::Unknown)
    );
}

/// The why names this cluster as the page does, never by the connection's
/// key, which nobody reads; objects still open by the key.
#[tokio::test]
async fn a_stage_names_this_cluster_by_its_context_not_its_key() {
    const KEY: &str = "access:4f1e9c0d2b7a";
    let reader = one_cluster(|_| {});
    let mut place = place("f-new");
    place.cluster = KEY.into();
    let change = read(&reader, &place, observed_at()).await.expect("read");
    let argocd = change
        .hops
        .iter()
        .find(|hop| hop.key == "dev-argocd-0")
        .expect("the Stage's Application");
    assert!(
        argocd
            .detail
            .contains("destination is Argo CD's own cluster (in-cluster), context core"),
        "{}",
        argocd.detail
    );
    let texts = format!("{change:?}");
    let words: Vec<&str> = change
        .hops
        .iter()
        .map(|hop| hop.detail.as_str())
        .chain(change.groups.iter().map(|group| group.detail.as_str()))
        .collect();
    assert!(words.iter().all(|text| !text.contains(KEY)), "{words:#?}");
    assert!(
        texts.contains(&format!("cluster: \"{KEY}\"")),
        "objects open by the key"
    );
}

/// One change, with Argo CD installed in each namespace and its Stage's
/// Application in only `application_namespace`.
fn argo_in(namespaces: &[&str], application_namespace: &str) -> FixtureReader {
    one_cluster(|world| {
        let mut app = application(Some(IN_CLUSTER_SERVER));
        app["metadata"]["namespace"] = json!(application_namespace);
        world.argocd = std::mem::take(&mut world.argocd)
            .with("applications", vec![app])
            .with(
                "deployments",
                namespaces
                    .iter()
                    .map(|namespace| {
                        json!({
                            "metadata": {
                                "namespace": namespace, "name": "argocd-server",
                                "labels": {"app.kubernetes.io/part-of": "argocd"}
                            }
                        })
                    })
                    .collect(),
            );
        world.kargo = std::mem::take(&mut world.kargo)
            .serves(
                "kargo.akuity.io",
                "v1alpha1",
                &["projects", "warehouses", "freights", "stages", "promotions"],
            )
            .with(
                "projects",
                vec![json!({"metadata": {"name": "storefront"}})],
            );
    })
}

fn application_lists(reader: &FixtureReader) -> Vec<String> {
    reader
        .requests
        .borrow()
        .iter()
        .filter(|request| request.contains("/applications ns="))
        .cloned()
        .collect()
}

#[tokio::test]
async fn a_change_finds_its_application_in_each_discovered_namespace() {
    for namespace in ["gitops", "platform"] {
        let reader = argo_in(&["platform", "delivery", "gitops"], namespace);
        let change = read(&reader, &place("f-new"), observed_at()).await.unwrap();
        assert_eq!(
            change.stages[0].cluster,
            Destination::Cluster("core".into())
        );
        assert!(
            change.hops.iter().any(|hop| match &hop.shows {
                Shows::Hop(detail) => detail.actions.iter().any(|action| match &action.target {
                    super::super::Target::Resource { object, .. } =>
                        object.kind == "Application" && object.namespace == namespace,
                    _ => false,
                }),
                _ => false,
            }),
            "the Application opens in {namespace}"
        );
        assert_eq!(application_lists(&reader).len(), 3);
    }
}

#[tokio::test]
async fn the_changes_list_finds_an_application_in_the_second_namespace() {
    let reader = argo_in(&["delivery", "gitops"], "gitops");
    let at = crate::changes::live::Where {
        cluster: "cluster-key".into(),
        label: "core".into(),
        argocd_namespace: "delivery".into(),
        mapping: Mapping::default(),
        sources: Default::default(),
        public_key_file: None,
        transparency_log: false,
        transparency_url: None,
        evidence_result: None,
        build_namespace: None,
        stage_naming: None,
    };
    let read =
        crate::changes::live::read(&reader, &at, observed_at(), crate::changes::live::BUDGET).await;
    let changes = crate::changes::derive(&read);
    let change = changes
        .rows
        .iter()
        .find_map(|row| row.trail.as_ref().ok())
        .unwrap();
    assert_eq!(
        change.stages[0].cluster,
        Destination::Cluster("core".into())
    );
    assert_eq!(
        application_lists(&reader).len(),
        2,
        "each namespace read once"
    );
}

#[tokio::test]
async fn changes_and_applications_share_the_namespace_cap() {
    let namespaces = ["e-ops", "a-ops", "d-ops", "b-ops", "c-ops"];
    let applications = argo_in(&namespaces, "e-ops");
    let session = crate::applications::read::read_session(
        &applications,
        crate::applications::SessionKey::new("core"),
    )
    .await;
    let crate::applications::ArgoScope::Namespace {
        namespaces: read_namespaces,
        ..
    } = session.argo_scope
    else {
        panic!("a namespaced read");
    };
    assert_eq!(read_namespaces, ["a-ops", "b-ops", "c-ops"]);
    let expected = application_lists(&applications);
    let reader = argo_in(&namespaces, "e-ops");
    let change = read(&reader, &place("f-new"), observed_at()).await.unwrap();
    assert_eq!(application_lists(&reader), expected);
    assert!(matches!(change.stages[0].cluster, Destination::Unknown(_)));
    assert!(change.missing.iter().any(|line| line.text.contains("d-ops")
        && line.text.contains("e-ops")
        && line.text.contains("namespace cap")));

    let reader = argo_in(&namespaces, "e-ops");
    let at = crate::changes::live::Where {
        cluster: "cluster-key".into(),
        label: "core".into(),
        argocd_namespace: "a-ops".into(),
        mapping: Mapping::default(),
        sources: Default::default(),
        public_key_file: None,
        transparency_log: false,
        transparency_url: None,
        evidence_result: None,
        build_namespace: None,
        stage_naming: None,
    };
    let read =
        crate::changes::live::read(&reader, &at, observed_at(), crate::changes::live::BUDGET).await;
    assert_eq!(application_lists(&reader), expected);
    assert!(
        crate::changes::derive(&read)
            .notes
            .iter()
            .any(|note| note.text.contains("d-ops") && note.text.contains("e-ops"))
    );
}

#[test]
fn the_plan_keeps_the_setups_log_switch_and_optional_url() {
    let freight = crate::kargo::parse_freight(&freight("f-new", NEW, SHA)).unwrap();
    let mut at = place("f-new");
    let off = plan_of(&at, &freight);
    assert!(!off.transparency_log);
    assert_eq!(off.transparency_url, None);
    at.transparency_log = true;
    for url in [Some("https://rekor.example.test/log"), None] {
        at.transparency_url = url.map(str::to_owned);
        let plan = plan_of(&at, &freight);
        assert!(plan.transparency_log);
        assert_eq!(plan.transparency_url.as_deref(), url);
    }
}
