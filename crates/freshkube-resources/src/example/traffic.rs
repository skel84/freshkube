//! Invented traffic for the Service and workload panes (`FRESHKUBE_SERVICES`):
//! each shape answers every Service read the same way, so one capture shows
//! one state.
use freshkube_core::resources::traffic::{ServiceFacts, ServiceKind, ServiceReading, *};
use freshkube_core::resources::{Failure, FailureKind};
use k8s_openapi::api::core::v1::{Pod, Service};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    /// Every group at once, and a pod of each unpublished reason.
    #[default]
    Mixed,
    Healthy,
    /// A selector that matches pods and no endpoints at all.
    Nothing,
    Draining,
    /// The slice API is not served: the legacy Endpoints answered.
    Legacy,
    /// A LoadBalancer with no ingress address.
    Pending,
    NoSelector,
    ExternalName,
    /// The endpoint reads are refused.
    Refused,
    /// The Services list is refused (a workload's chain).
    ServicesRefused,
}

impl Shape {
    pub const NAMES: [&'static str; 10] = [
        "mixed",
        "healthy",
        "nothing",
        "draining",
        "legacy",
        "pending",
        "no-selector",
        "external-name",
        "refused",
        "services-refused",
    ];

    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "mixed" => Self::Mixed,
            "healthy" => Self::Healthy,
            "nothing" => Self::Nothing,
            "draining" => Self::Draining,
            "legacy" => Self::Legacy,
            "pending" => Self::Pending,
            "no-selector" => Self::NoSelector,
            "external-name" => Self::ExternalName,
            "refused" => Self::Refused,
            "services-refused" => Self::ServicesRefused,
            _ => return None,
        })
    }
}

fn refused() -> Failure {
    Failure::new(
        FailureKind::Forbidden,
        "endpointslices is forbidden for the invented viewer",
    )
}

/// A Service from the example's own YAML, shaped as `shape` asks.
pub(crate) fn facts(yaml: &str, shape: Shape) -> Option<ServiceFacts> {
    let service: Service = serde_yaml::from_str(yaml).ok()?;
    let mut facts = ServiceFacts::from_service(&service);
    match shape {
        Shape::Pending => {
            facts.kind = ServiceKind::LoadBalancer;
            facts.load_balancer = LoadBalancerState::Pending;
        }
        // A named target port lets a pod without it be left out.
        Shape::Mixed => {
            if let Some(port) = facts.ports.first_mut() {
                port.target = "http".into();
            }
        }
        Shape::NoSelector => facts.selector.clear(),
        Shape::ExternalName => {
            facts.kind = ServiceKind::ExternalName;
            facts.external_name = Some("db.example.com".into());
            facts.selector.clear();
        }
        _ => {}
    }
    Some(facts)
}

fn slice(facts: &ServiceFacts, endpoints: Vec<Value>, ports: Value) -> EndpointSlice {
    serde_json::from_value(json!({
        "metadata": {"name": format!("{}-x1", facts.name), "namespace": facts.namespace,
            "labels": {SERVICE_LABEL: facts.name}},
        "addressType": "IPv4", "ports": ports, "endpoints": endpoints,
    }))
    .expect("invented slice")
}

fn endpoint(n: u8, conditions: Value) -> Value {
    json!({"addresses": [format!("10.244.1.{n}")], "conditions": conditions, "nodeName": "node-a",
        "targetRef": {"kind": "Pod", "name": format!("{}-pod-{n}", "app"), "uid": format!("uid-{n}")}})
}

fn pod(facts: &ServiceFacts, n: u8, patch: Value) -> Pod {
    let mut value = json!({
        "metadata": {"name": format!("app-pod-{n}"), "namespace": facts.namespace, "uid": format!("uid-{n}"),
            "labels": facts.selector},
        "spec": {"containers": [{"name": "app", "ports": [{"name": "http", "containerPort": 8080}]}]},
        "status": {"phase": "Running", "podIP": format!("10.244.1.{n}"),
            "conditions": [{"type": "Ready", "status": "True"}]},
    });
    if let (Some(into), Some(with)) = (value.as_object_mut(), patch.as_object()) {
        for (key, with) in with {
            into.insert(key.clone(), with.clone());
        }
    }
    serde_json::from_value(value).expect("invented pod")
}

/// What `facts` publishes under `shape`, and the pods left out.
pub(crate) fn reading(facts: ServiceFacts, shape: Shape, with_pods: bool) -> ServiceReading {
    let named = facts
        .ports
        .first()
        .map(|p| p.name.clone())
        .unwrap_or_default();
    let ports = json!([{"name": named, "port": 8080, "protocol": "TCP"}]);
    let ready = json!({"ready": true, "serving": true, "terminating": false});
    let draining = json!({"ready": false, "serving": true, "terminating": true});
    let cold = json!({"ready": false, "serving": false, "terminating": false});
    let slices = |endpoints: Vec<Value>, ports: Value| {
        let slice = slice(&facts, endpoints, ports);
        Published::from_slices(&facts, vec![slice], false)
    };
    let published: Result<Published, Failure> = match shape {
        Shape::Refused => Err(refused()),
        Shape::Nothing | Shape::NoSelector => Ok(slices(Vec::new(), ports.clone())),
        Shape::Healthy | Shape::Pending => Ok(slices(
            vec![endpoint(11, ready.clone()), endpoint(12, ready.clone())],
            ports.clone(),
        )),
        Shape::Draining => Ok(slices(
            vec![endpoint(11, ready.clone()), endpoint(12, draining)],
            ports.clone(),
        )),
        Shape::Legacy => {
            let endpoints = serde_json::from_value(json!({
                "metadata": {"name": facts.name},
                "subsets": [{
                    "addresses": [{"ip": "10.244.1.11"}, {"ip": "10.244.1.12"}],
                    "notReadyAddresses": [{"ip": "10.244.1.13"}],
                    "ports": [{"name": named, "port": 8080}],
                }],
            }))
            .expect("invented endpoints");
            Ok(Published::from_endpoints(&facts, &endpoints))
        }
        Shape::Mixed | Shape::ServicesRefused | Shape::ExternalName => {
            let mut all = slices(
                vec![
                    endpoint(11, ready.clone()),
                    endpoint(12, ready),
                    endpoint(13, draining),
                    endpoint(14, cold),
                ],
                ports.clone(),
            );
            // One more behind a slice that carries only a metrics port.
            let stray = slice(
                &facts,
                vec![endpoint(15, json!({"ready": true}))],
                json!([{"name": "metrics", "port": 9090}]),
            );
            let more = Published::from_slices(&facts, vec![stray], false);
            all.endpoints.extend(more.endpoints);
            Ok(all)
        }
    };
    let unpublished = with_pods.then(|| match &published {
        Err(failure) => Err(failure.clone()),
        Ok(published) => {
            let pods = match shape {
                Shape::Mixed => vec![
                    pod(&facts, 11, json!({})),
                    pod(&facts, 21, json!({"metadata": {"name": "app-pod-21", "namespace": facts.namespace, "uid": "uid-21", "labels": facts.selector, "deletionTimestamp": "2026-10-23T10:00:00Z"}})),
                    pod(&facts, 22, json!({"status": {"phase": "Pending", "conditions": []}})),
                    pod(&facts, 23, json!({"spec": {"containers": [{"name": "app", "ports": [{"name": "other", "containerPort": 1}]}]}})),
                    pod(&facts, 24, json!({})),
                ],
                Shape::Nothing => vec![pod(&facts, 24, json!({}))],
                _ => Vec::new(),
            };
            Ok(unpublished_pods(&facts, published, &pods))
        }
    });
    ServiceReading::new(facts, published, unpublished)
}
