use serde_json::{Value, json};

use super::*;

use crate::argocd::{StageNaming, parse_application, parse_application_set};

use crate::kargo::{parse_stage, parse_warehouse};

fn key(name: &str) -> SessionKey {
    SessionKey::new(name)
}

fn stage(project: &str, name: &str) -> Stage {
    parse_stage(&json!({"metadata": {"namespace": project, "name": name}})).unwrap()
}

fn warehouse(project: &str, name: &str) -> Warehouse {
    parse_warehouse(&json!({"metadata": {"namespace": project, "name": name}})).unwrap()
}

fn project(name: &str, stages: &[&str]) -> KargoProjectRead {
    KargoProjectRead {
        name: name.into(),
        stages: Source::Read(stages.iter().map(|s| stage(name, s)).collect()),
        warehouses: Source::Read(vec![warehouse(name, "images")]),
    }
}

fn argo(name: &str, extra: Value) -> ArgoApplication {
    let mut value = json!({
        "metadata": {"namespace": "argocd", "name": name},
        "spec": {"destination": {"server": "https://kubernetes.default.svc", "namespace": name}},
    });
    merge(&mut value, extra);
    parse_application(&value).unwrap()
}

fn merge(into: &mut Value, extra: Value) {
    match (into, extra) {
        (Value::Object(into), Value::Object(extra)) => {
            for (k, v) in extra {
                merge(into.entry(k).or_insert(Value::Null), v);
            }
        }
        (into, extra) => *into = extra,
    }
}

fn owned_by(set: &str) -> Value {
    json!({"metadata": {"ownerReferences": [
        {"kind": "ApplicationSet", "apiVersion": "argoproj.io/v1alpha1", "name": set}
    ]}})
}

fn managing(kind: &str, namespace: &str, name: &str) -> Value {
    json!({"status": {"resources": [
        {"group": "apps", "kind": kind, "namespace": namespace, "name": name}
    ]}})
}

fn workload(namespace: &str, name: &str, part_of: &str) -> LabelledWorkload {
    LabelledWorkload {
        namespace: namespace.into(),
        name: name.into(),
        kind: WorkloadKind::Deployment,
        part_of: part_of.into(),
        tracking_id: None,
        instance: None,
    }
}

fn session(name: &str) -> SessionInputs {
    SessionInputs {
        key: key(name),
        kargo: Source::NotInstalled("API group kargo.akuity.io is not served".into()),
        argo_scope: ArgoScope::AllNamespaces,
        argo_applications: Source::Read(Vec::new()),
        argo_application_sets: Source::Read(Vec::new()),
        workloads: Source::Read(Vec::new()),
        asked: crate::applications::Asked::ALL,
        absent: None,
    }
}

fn derived(sessions: Vec<SessionInputs>) -> Derived {
    derive(
        &Inputs {
            sessions,
            stage_naming: None,
            destinations: Vec::new(),
        },
        &Override::default(),
    )
}

fn id(rule: Rule, name: &str) -> ApplicationId {
    ApplicationId::new(rule, name)
}

fn set_id(name: &str) -> ApplicationId {
    ApplicationId::argo_application_set("argocd", name)
}

fn app_id(name: &str) -> ApplicationId {
    ApplicationId::argo_application("argocd", name)
}

fn names(application: &Application) -> Vec<&str> {
    application
        .members
        .iter()
        .map(|m| m.at.name.as_str())
        .collect()
}

fn with_override(over: Override) -> Derived {
    let mut core = session("core-fra");
    core.kargo = Source::Read(vec![project("checkout", &["dev"])]);
    core.argo_applications = Source::Read(vec![argo("catalog", json!({}))]);
    core.workloads = Source::Read(vec![workload("shop", "web", "storefront")]);
    derive(
        &Inputs {
            sessions: vec![core],
            stage_naming: None,
            destinations: Vec::new(),
        },
        &over,
    )
}

fn override_claim_inputs() -> Inputs {
    let mut core = session("core-fra");
    core.kargo = Source::Read(vec![
        project("checkout", &["dev"]),
        project("cart", &["prod"]),
    ]);
    core.argo_applications = Source::Read(vec![argo(
        "checkout-dev",
        managing("Deployment", "checkout-dev", "web"),
    )]);
    let mut web = workload("checkout-dev", "web", "storefront");
    web.instance = Some("checkout-dev".into());
    core.workloads = Source::Read(vec![web]);
    Inputs {
        sessions: vec![core],
        stage_naming: None,
        destinations: Vec::new(),
    }
}

// The override applies after every rule, to a Kargo Project's application
// as to any other.

fn kargo_world(over: Override) -> Derived {
    let mut core = session("core-fra");
    core.kargo = Source::Read(vec![
        project("checkout", &["dev", "prod"]),
        project("cart", &["dev"]),
    ]);
    core.argo_applications = Source::Read(vec![argo("catalog", json!({}))]);
    derive(
        &Inputs {
            sessions: vec![core],
            stage_naming: None,
            destinations: Vec::new(),
        },
        &over,
    )
}

fn derived_named(sessions: Vec<SessionInputs>, template: &str) -> Derived {
    derive(
        &Inputs {
            sessions,
            stage_naming: Some(StageNaming {
                project_key: None,
                name_template: template.into(),
            }),
            destinations: Vec::new(),
        },
        &Override::default(),
    )
}

fn member_of<'a>(found: &'a Derived, project: &str, application: &str) -> Option<&'a Member> {
    found
        .find(&id(Rule::Kargo, project))?
        .members
        .iter()
        .find(|m| m.at.kind == MemberKind::ArgoApplication && m.at.name == application)
}

mod coverage;
mod grouping;
mod membership;
mod naming;
mod override_claims;
mod overrides;
mod reads;
