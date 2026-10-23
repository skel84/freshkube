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

mod chain;
mod destinations;
mod discovery;
mod failures;
mod links;
mod plan;
mod reach;
mod stages;
mod workloads;

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
        github: None,
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
