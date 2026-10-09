//! The acme workspace of `docs/platform/`'s mocks as what was read from it,
//! for `--fixture` and for tests. Names are invented; the addresses are
//! `example.test`. It goes through the real parsers, so it holds what a
//! cluster would answer, not what the model is told.
//!
//! Six clusters: Argo CD and Kargo run in `core-fra`; `cicd-fra` serves
//! neither; the four environment clusters hold the workloads.
//!
//! - `checkout` and `cart` are Kargo Projects with four Stages each; their
//!   Argo CD Applications name them, and deploy to the environment clusters.
//! - `catalog` is an ApplicationSet, so it is found by the second rule.
//! - `status-page` is a plain Application that deploys to its own cluster and
//!   manages a workload labelled `part-of=public-status`, whose instance
//!   label names it back.
//! - `loyalty` is found only by its label.
//! - `checkout-worker`, labelled `part-of=checkout` in `prod-lon`, joins the
//!   Kargo application by its name.

use serde_json::{Value, json};

use super::read::parse_labelled_workload;
use super::{ArgoScope, Inputs, KargoProjectRead, SessionInputs, SessionKey};
use crate::delivery::argocd::{parse_application, parse_application_set};
use crate::delivery::kargo::{parse_stage, parse_warehouse};
use crate::delivery::source::Source;
use crate::workloads::WorkloadKind;

pub const CORE: &str = "core-fra";
pub const CICD: &str = "cicd-fra";
const ENVIRONMENTS: [(&str, &str); 4] = [
    ("dev", "dev-fra"),
    ("stage", "stage-fra"),
    ("prod-ams", "prod-ams"),
    ("prod-lon", "prod-lon"),
];
const AUTHORIZED_STAGE: &str = "kargo.akuity.io/authorized-stage";

fn server(cluster: &str) -> String {
    format!("https://{cluster}.example.test:6443")
}

/// A kind's key, as kubectl names it, and its `apiVersion` and `kind`.
const PROJECTS: (&str, &str, &str) = ("projects.kargo.akuity.io", KARGO, "Project");
const STAGES: (&str, &str, &str) = ("stages.kargo.akuity.io", KARGO, "Stage");
const WAREHOUSES: (&str, &str, &str) = ("warehouses.kargo.akuity.io", KARGO, "Warehouse");
const APPLICATIONS: (&str, &str, &str) = ("applications.argoproj.io", ARGO, "Application");
const APPLICATION_SETS: (&str, &str, &str) =
    ("applicationsets.argoproj.io", ARGO, "ApplicationSet");
const DEPLOYMENTS: (&str, &str, &str) = ("deployments.apps", "apps/v1", "Deployment");
const FREIGHT: (&str, &str, &str) = ("freights.kargo.akuity.io", KARGO, "Freight");
const TASK_RUNS: (&str, &str, &str) = ("taskruns.tekton.dev", "tekton.dev/v1", "TaskRun");
const KARGO: &str = "kargo.akuity.io/v1alpha1";
const ARGO: &str = "argoproj.io/v1alpha1";

fn object((key, api_version, kind): (&'static str, &str, &str), mut value: Value) -> CoreObject {
    if let Some(fields) = value.as_object_mut() {
        fields.insert("apiVersion".into(), api_version.into());
        fields.insert("kind".into(), kind.into());
    }
    CoreObject { key, value }
}

fn of<'a>(objects: &'a [CoreObject], key: &str) -> impl Iterator<Item = &'a Value> {
    objects
        .iter()
        .filter(move |object| object.key == key)
        .map(|object| &object.value)
}

fn kargo_project(objects: &[CoreObject], name: &str) -> KargoProjectRead {
    let in_project = |key| {
        of(objects, key)
            .filter(move |value| value.pointer("/metadata/namespace") == Some(&json!(name)))
    };
    KargoProjectRead {
        name: name.into(),
        stages: Source::Read(in_project(STAGES.0).filter_map(parse_stage).collect()),
        warehouses: Source::Read(
            in_project(WAREHOUSES.0)
                .filter_map(parse_warehouse)
                .collect(),
        ),
    }
}

fn argo_app(name: &str, destination: Value, metadata: Value, status: Value) -> CoreObject {
    let mut meta = json!({"namespace": "argocd", "name": name});
    if let (Some(into), Some(extra)) = (meta.as_object_mut(), metadata.as_object()) {
        into.extend(extra.clone());
    }
    object(
        APPLICATIONS,
        json!({"metadata": meta, "spec": {"destination": destination}, "status": status}),
    )
}

fn pinned_to(project: &str, stage: &str, cluster: &str) -> CoreObject {
    argo_app(
        &format!("{project}-{stage}"),
        json!({"server": server(cluster), "namespace": project}),
        json!({"annotations": {AUTHORIZED_STAGE: format!("{project}:{stage}")}}),
        json!({}),
    )
}

fn generated(set: &str, stage: &str, cluster: &str) -> CoreObject {
    argo_app(
        &format!("{set}-{stage}"),
        json!({"server": server(cluster), "namespace": set}),
        json!({"ownerReferences": [{
            "kind": "ApplicationSet", "apiVersion": "argoproj.io/v1alpha1", "name": set,
        }]}),
        json!({}),
    )
}

/// One object `core-fra` holds: its kind's key, as kubectl names it, and
/// the object as the cluster answers it.
#[derive(Clone, Debug)]
pub struct CoreObject {
    pub key: &'static str,
    pub value: Value,
}

/// Every object `core-fra` holds that the applications are read from:
/// Kargo's Projects, Stages and Warehouses, Argo CD's Applications and
/// ApplicationSet, and the labelled Deployment, in that order; then the
/// Freight the example change follows (`delivery::change::example`) and
/// its release run's policy TaskRun. The
/// example Resources page lists them, so it shows what the Applications
/// page read and where the change page leads.
pub fn core_objects() -> Vec<CoreObject> {
    let mut objects = Vec::new();
    for project in ["checkout", "cart"] {
        objects.push(object(PROJECTS, json!({"metadata": {"name": project}})));
    }
    for project in ["checkout", "cart"] {
        for (stage, _) in ENVIRONMENTS {
            objects.push(object(
                STAGES,
                json!({"metadata": {"namespace": project, "name": stage}}),
            ));
        }
    }
    for project in ["checkout", "cart"] {
        objects.push(object(
            WAREHOUSES,
            json!({"metadata": {"namespace": project, "name": format!("{project}-images")}}),
        ));
    }
    for project in ["checkout", "cart"] {
        for (stage, cluster) in ENVIRONMENTS {
            objects.push(pinned_to(project, stage, cluster));
        }
    }
    for (stage, cluster) in ENVIRONMENTS {
        objects.push(generated("catalog", stage, cluster));
    }
    objects.push(argo_app(
        "status-page",
        json!({"server": "https://kubernetes.default.svc", "namespace": "status"}),
        json!({}),
        json!({"resources": [{
            "group": "apps", "kind": "Deployment", "namespace": "status", "name": "status-page",
        }]}),
    ));
    objects.push(object(
        APPLICATION_SETS,
        json!({"metadata": {"namespace": "argocd", "name": "catalog"}}),
    ));
    // Labelled `public-status`, and with the instance label Argo CD's label
    // tracking writes, so it names the Application that manages it.
    objects.push(object(
        DEPLOYMENTS,
        json!({"metadata": {
            "namespace": "status", "name": "status-page",
            "labels": {
                "app.kubernetes.io/part-of": "public-status",
                "app.kubernetes.io/instance": "status-page",
            },
        }}),
    ));
    objects.push(object(
        FREIGHT,
        json!({"metadata": {
            "namespace": crate::delivery::change::example::PROJECT,
            "name": crate::delivery::change::example::FREIGHT,
        }}),
    ));
    objects.push(object(
        TASK_RUNS,
        json!({"metadata": {
            "namespace": crate::delivery::change::example::RELEASE_NAMESPACE,
            "name": crate::delivery::change::example::VERIFY_TASK,
        }}),
    ));
    objects
}

fn core() -> SessionInputs {
    let objects = core_objects();
    let projects = of(&objects, PROJECTS.0)
        .filter_map(|value| value.pointer("/metadata/name")?.as_str())
        .map(|name| kargo_project(&objects, name))
        .collect();
    SessionInputs {
        key: SessionKey::new(CORE),
        kargo: Source::Read(projects),
        argo_scope: ArgoScope::AllNamespaces,
        argo_applications: Source::Read(
            of(&objects, APPLICATIONS.0)
                .filter_map(parse_application)
                .collect(),
        ),
        argo_application_sets: Source::Read(
            of(&objects, APPLICATION_SETS.0)
                .filter_map(parse_application_set)
                .collect(),
        ),
        workloads: Source::Read(
            of(&objects, DEPLOYMENTS.0)
                .filter_map(|value| parse_labelled_workload(value, WorkloadKind::Deployment))
                .collect(),
        ),
    }
}

/// The Deployments an environment cluster holds with a `part-of` label.
/// The example Resources page lists them for the cluster's context, so a
/// part opened there is the one read.
pub fn environment_objects(cluster: &str) -> Vec<CoreObject> {
    let items: &[(&str, &str, &str)] = match cluster {
        "dev-fra" => &[
            ("checkout", "checkout-api", "checkout"),
            ("cart", "cart-api", "cart"),
            ("loyalty", "loyalty-api", "loyalty"),
        ],
        "prod-lon" => &[
            ("checkout", "checkout-api", "checkout"),
            ("checkout", "checkout-worker", "checkout"),
            ("catalog", "catalog-api", "catalog"),
        ],
        _ => &[],
    };
    items
        .iter()
        .map(|(namespace, name, part_of)| {
            object(
                DEPLOYMENTS,
                json!({"metadata": {
                    "namespace": namespace, "name": name,
                    "labels": {"app.kubernetes.io/part-of": part_of},
                }}),
            )
        })
        .collect()
}

fn environment(cluster: &str) -> SessionInputs {
    let objects = environment_objects(cluster);
    SessionInputs {
        key: SessionKey::new(cluster),
        kargo: Source::NotInstalled("API group kargo.akuity.io is not served".into()),
        argo_scope: ArgoScope::AllNamespaces,
        argo_applications: Source::NotInstalled("API group argoproj.io is not served".into()),
        argo_application_sets: Source::NotInstalled("API group argoproj.io is not served".into()),
        workloads: Source::Read(
            of(&objects, DEPLOYMENTS.0)
                .filter_map(|value| parse_labelled_workload(value, WorkloadKind::Deployment))
                .collect(),
        ),
    }
}

/// What reading the acme workspace's six clusters gives.
pub fn acme() -> Inputs {
    let mut sessions = vec![core(), environment(CICD)];
    sessions.extend(ENVIRONMENTS.iter().map(|(_, cluster)| environment(cluster)));
    Inputs {
        sessions,
        stage_naming: None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Basis, Destination, MemberKind, Override, Rule, derive};
    use super::*;

    #[test]
    fn the_acme_workspace_has_five_applications_found_by_three_rules() {
        let found = derive(&acme(), &Override::default());
        let rules: Vec<_> = found
            .applications
            .iter()
            .map(|app| (app.name.as_str(), app.rule))
            .collect();
        assert_eq!(
            rules,
            [
                ("cart", Rule::Kargo),
                ("catalog", Rule::ArgoCd),
                ("checkout", Rule::Kargo),
                ("loyalty", Rule::PartOf),
                ("status-page", Rule::ArgoCd),
            ]
        );
        assert!(found.unknown().is_empty());
        assert!(found.unmatched.is_empty());
    }

    #[test]
    fn checkout_holds_its_stages_applications_and_workloads_across_clusters() {
        let found = derive(&acme(), &Override::default());
        let checkout = found
            .applications
            .iter()
            .find(|app| app.name == "checkout")
            .unwrap();
        let count = |kind: fn(&MemberKind) -> bool| {
            checkout.members.iter().filter(|m| kind(&m.at.kind)).count()
        };
        assert_eq!(count(|k| *k == MemberKind::KargoStage), 4);
        assert_eq!(count(|k| *k == MemberKind::KargoWarehouse), 1);
        assert_eq!(count(|k| *k == MemberKind::ArgoApplication), 4);
        assert_eq!(count(|k| matches!(k, MemberKind::Workload(_))), 3);
        let remote = checkout
            .members
            .iter()
            .filter(|m| matches!(m.destination, Some(Destination::Other { .. })))
            .count();
        assert_eq!(remote, 4, "every environment is another cluster, unmapped");
        let worker = checkout
            .members
            .iter()
            .find(|m| m.at.name == "checkout-worker")
            .unwrap();
        assert_eq!(worker.basis, Basis::SameName);
    }

    #[test]
    fn core_fras_objects_are_what_its_session_was_read_from() {
        let objects = core_objects();
        let mut seen = std::collections::BTreeSet::new();
        for object in &objects {
            let at = |field: &str| object.value["metadata"][field].as_str().map(str::to_owned);
            assert!(seen.insert((object.key, at("namespace"), at("name"))));
            assert!(object.value["apiVersion"].is_string() && object.value["kind"].is_string());
        }
        let session = core();
        let Source::Read(projects) = &session.kargo else {
            panic!("Kargo read")
        };
        let stages: usize = projects
            .iter()
            .map(|p| match &p.stages {
                Source::Read(stages) => stages.len(),
                _ => 0,
            })
            .sum();
        let count = |key: &str| objects.iter().filter(|o| o.key == key).count();
        assert_eq!(count("projects.kargo.akuity.io"), projects.len());
        assert_eq!(count("stages.kargo.akuity.io"), stages);
        let Source::Read(applications) = &session.argo_applications else {
            panic!("Argo CD read")
        };
        assert_eq!(count("applications.argoproj.io"), applications.len());
        assert_eq!(count("deployments.apps"), 1);
    }

    #[test]
    fn a_workload_the_local_application_manages_joins_it() {
        let found = derive(&acme(), &Override::default());
        let status = found
            .applications
            .iter()
            .find(|app| app.name == "status-page")
            .unwrap();
        let deployment = status
            .members
            .iter()
            .find(|m| matches!(m.at.kind, MemberKind::Workload(_)))
            .unwrap();
        assert_eq!(deployment.basis, Basis::Tracked);
        assert!(
            found
                .applications
                .iter()
                .all(|app| app.name != "public-status")
        );
    }
}
