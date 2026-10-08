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
//!   manages a workload labelled `part-of=public-status`.
//! - `loyalty` is found only by its label.
//! - `checkout-worker`, labelled `part-of=checkout` in `prod-fra`, joins the
//!   Kargo application by its name.

use serde_json::{Value, json};

use super::read::parse_labelled_workload;
use super::{Inputs, KargoProjectRead, SessionInputs, SessionKey};
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
    ("prod-fra", "prod-fra"),
];
const AUTHORIZED_STAGE: &str = "kargo.akuity.io/authorized-stage";

fn server(cluster: &str) -> String {
    format!("https://{cluster}.example.test:6443")
}

fn kargo_project(name: &str) -> KargoProjectRead {
    let stages = ENVIRONMENTS
        .iter()
        .filter_map(|(stage, _)| {
            parse_stage(&json!({"metadata": {"namespace": name, "name": stage}}))
        })
        .collect();
    let warehouse = json!({"metadata": {"namespace": name, "name": format!("{name}-images")}});
    KargoProjectRead {
        name: name.into(),
        stages: Source::Read(stages),
        warehouses: Source::Read(parse_warehouse(&warehouse).into_iter().collect()),
    }
}

fn argo_app(name: &str, destination: Value, metadata: Value, status: Value) -> Value {
    let mut meta = json!({"namespace": "argocd", "name": name});
    if let (Some(into), Some(extra)) = (meta.as_object_mut(), metadata.as_object()) {
        into.extend(extra.clone());
    }
    json!({"metadata": meta, "spec": {"destination": destination}, "status": status})
}

fn pinned_to(project: &str, stage: &str, cluster: &str) -> Value {
    argo_app(
        &format!("{project}-{stage}"),
        json!({"server": server(cluster), "namespace": project}),
        json!({"annotations": {AUTHORIZED_STAGE: format!("{project}:{stage}")}}),
        json!({}),
    )
}

fn generated(set: &str, stage: &str, cluster: &str) -> Value {
    argo_app(
        &format!("{set}-{stage}"),
        json!({"server": server(cluster), "namespace": set}),
        json!({"ownerReferences": [{
            "kind": "ApplicationSet", "apiVersion": "argoproj.io/v1alpha1", "name": set,
        }]}),
        json!({}),
    )
}

fn core() -> SessionInputs {
    let mut applications = Vec::new();
    for project in ["checkout", "cart"] {
        for (stage, cluster) in ENVIRONMENTS {
            applications.push(pinned_to(project, stage, cluster));
        }
    }
    for (stage, cluster) in ENVIRONMENTS {
        applications.push(generated("catalog", stage, cluster));
    }
    applications.push(argo_app(
        "status-page",
        json!({"server": "https://kubernetes.default.svc", "namespace": "status"}),
        json!({}),
        json!({"resources": [{
            "group": "apps", "kind": "Deployment", "namespace": "status", "name": "status-page",
        }]}),
    ));
    let set = json!({"metadata": {"namespace": "argocd", "name": "catalog"}});
    SessionInputs {
        key: SessionKey::new(CORE),
        kargo: Source::Read(vec![kargo_project("checkout"), kargo_project("cart")]),
        argo_applications: Source::Read(
            applications.iter().filter_map(parse_application).collect(),
        ),
        argo_application_sets: Source::Read(parse_application_set(&set).into_iter().collect()),
        workloads: Source::Read(
            labelled(&[("status", "status-page", "public-status")])
                .into_iter()
                .collect(),
        ),
    }
}

fn labelled(items: &[(&str, &str, &str)]) -> Vec<super::LabelledWorkload> {
    items
        .iter()
        .filter_map(|(namespace, name, part_of)| {
            parse_labelled_workload(
                &json!({"metadata": {
                    "namespace": namespace, "name": name,
                    "labels": {"app.kubernetes.io/part-of": part_of},
                }}),
                WorkloadKind::Deployment,
            )
        })
        .collect()
}

fn environment(cluster: &str) -> SessionInputs {
    let items: &[(&str, &str, &str)] = match cluster {
        "dev-fra" => &[
            ("checkout", "checkout-api", "checkout"),
            ("cart", "cart-api", "cart"),
            ("loyalty", "loyalty-api", "loyalty"),
        ],
        "prod-fra" => &[
            ("checkout", "checkout-api", "checkout"),
            ("checkout", "checkout-worker", "checkout"),
            ("catalog", "catalog-api", "catalog"),
        ],
        _ => &[],
    };
    SessionInputs {
        key: SessionKey::new(cluster),
        kargo: Source::NotInstalled("API group kargo.akuity.io is not served".into()),
        argo_applications: Source::NotInstalled("API group argoproj.io is not served".into()),
        argo_application_sets: Source::NotInstalled("API group argoproj.io is not served".into()),
        workloads: Source::Read(labelled(items)),
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
        assert_eq!(deployment.basis, Basis::ManagedBy);
        assert!(
            found
                .applications
                .iter()
                .all(|app| app.name != "public-status")
        );
    }
}
