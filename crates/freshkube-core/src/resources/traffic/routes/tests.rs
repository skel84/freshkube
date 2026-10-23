use super::*;
use crate::failure::{Failure, FailureKind};
use crate::resources::traffic::{Published, ServiceFacts, ServiceReading};
use k8s_openapi::api::core::v1::Service;
use k8s_openapi::api::discovery::v1::EndpointSlice;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn cond(kind: &str, status: &str, reason: &str, generation: i64) -> Value {
    json!({"type": kind, "status": status, "reason": reason, "message": "", "observedGeneration": generation})
}

fn route_json(spec: Value, status: Value) -> Value {
    let mut route = json!({
        "metadata": {"name": "web", "namespace": "shop", "uid": "r1", "generation": 3},
        "spec": {
            "parentRefs": [{"name": "edge", "namespace": "infra", "sectionName": "https"}],
            "hostnames": ["shop.example.com"],
            "rules": [{"matches": [{"path": {"type": "PathPrefix", "value": "/api"}}],
                "backendRefs": [{"name": "cart", "port": 8080}]}],
        },
        "status": {"parents": [{
            "parentRef": {"name": "edge", "namespace": "infra", "sectionName": "https"},
            "controllerName": "example.io/gateway",
            "conditions": [cond("Accepted", "True", "Accepted", 3), cond("ResolvedRefs", "True", "ResolvedRefs", 3)],
        }]},
    });
    merge(&mut route["spec"], spec);
    merge(&mut route["status"], status);
    route
}

fn merge(into: &mut Value, with: Value) {
    match (into, with) {
        (Value::Object(into), Value::Object(with)) => {
            for (key, value) in with {
                merge(into.entry(key).or_insert(Value::Null), value);
            }
        }
        (into, with) => *into = with,
    }
}

fn gateway_json(edit: Value) -> Value {
    let mut gateway = json!({
        "metadata": {"name": "edge", "namespace": "infra", "generation": 2},
        "spec": {"gatewayClassName": "example",
            "listeners": [{"name": "https", "port": 443, "protocol": "HTTPS", "hostname": "*.example.com",
                "allowedRoutes": {"namespaces": {"from": "All"}}}]},
        "status": {
            "conditions": [cond("Accepted", "True", "Accepted", 2), cond("Programmed", "True", "Programmed", 2)],
            "addresses": [{"type": "IPAddress", "value": "203.0.113.10"}],
            "listeners": [{"name": "https", "attachedRoutes": 1,
                "conditions": [cond("Accepted", "True", "Accepted", 2), cond("Programmed", "True", "Programmed", 2)]}],
        },
    });
    merge(&mut gateway, edit);
    gateway
}

fn class_json(status: Value) -> Value {
    json!({"metadata": {"name": "example"}, "spec": {"controllerName": "example.io/gateway"},
        "status": status})
}

fn accepted_class() -> Value {
    class_json(json!({"conditions": [cond("Accepted", "True", "Accepted", 1)]}))
}

fn service_reading(ready: usize, not_ready: usize, ports: &[i32]) -> ServiceReading {
    let service: Service = serde_json::from_value(json!({
        "metadata": {"name": "cart", "namespace": "shop", "uid": "s1"},
        "spec": {"selector": {"app": "cart"},
            "ports": ports.iter().map(|p| json!({"port": p, "targetPort": 8080})).collect::<Vec<_>>()},
    }))
    .unwrap();
    let facts = ServiceFacts::from_service(&service);
    let endpoints = (0..ready + not_ready)
        .map(|n| json!({"addresses": [format!("10.0.0.{n}")], "conditions": {"ready": n < ready}}))
        .collect::<Vec<_>>();
    let slices: Vec<EndpointSlice> = if endpoints.is_empty() {
        Vec::new()
    } else {
        vec![serde_json::from_value(json!({
            "metadata": {"name": "cart-a", "namespace": "shop", "labels": {"kubernetes.io/service-name": "cart"}},
            "addressType": "IPv4", "ports": [{"port": 8080}], "endpoints": endpoints,
        }))
        .unwrap()]
    };
    let published = Published::from_slices(&facts, slices, false);
    ServiceReading::new(facts, Ok(published), None)
}

/// Everything a trace reads, with the knobs the tests turn.
struct World {
    gateways: Result<Listing<GatewayFacts>, Failure>,
    classes: Result<Listing<ClassFacts>, Failure>,
    grants: Result<Listing<GrantFacts>, Failure>,
    namespaces: Result<NamespaceLabels, Failure>,
    backends: BTreeMap<(String, String), BackendRead>,
}

fn refused() -> Failure {
    Failure::new(FailureKind::Forbidden, "forbidden for the invented viewer")
}

impl World {
    fn healthy() -> Self {
        Self::with(gateway_json(json!({})), accepted_class())
    }

    fn with(gateway: Value, class: Value) -> Self {
        Self {
            gateways: Ok(Listing::new(
                vec![GatewayFacts::parse(&gateway).unwrap()],
                false,
            )),
            classes: Ok(Listing::new(
                vec![ClassFacts::parse(&class).unwrap()],
                false,
            )),
            grants: Ok(Listing::new(Vec::new(), false)),
            namespaces: Ok(Listing::new(
                vec![(
                    "shop".into(),
                    [("team".to_owned(), "web".to_owned())].into(),
                )],
                false,
            )),
            backends: [(
                ("shop".to_owned(), "cart".to_owned()),
                Ok(Some(service_reading(2, 0, &[8080]))),
            )]
            .into(),
        }
    }

    fn trace(&self, route: Value) -> RouteTrace {
        let route = RouteFacts::parse(&route).unwrap();
        trace_route(
            &route,
            &Inputs {
                gateways: &self.gateways,
                classes: &self.classes,
                grants: &self.grants,
                namespaces: &self.namespaces,
                backends: &self.backends,
                grant_version: "v1beta1",
            },
        )
    }
}

fn healthy_route() -> Value {
    route_json(json!({}), json!({}))
}

fn step(trace: &RouteTrace, id: StepId) -> &TraceStep {
    let parent = &trace.parents[0];
    parent
        .steps
        .iter()
        .chain(parent.branches.iter().flat_map(|b| &b.steps))
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("no {id:?} step"))
}

fn stops(trace: &RouteTrace, id: StepId) {
    let broke = trace.parents[0]
        .steps
        .iter()
        .chain(trace.parents[0].branches.iter().flat_map(|b| &b.steps))
        .position(|s| matches!(s.verdict, StepVerdict::Fail | StepVerdict::Unknown))
        .map(|ix| {
            trace.parents[0]
                .steps
                .iter()
                .chain(trace.parents[0].branches.iter().flat_map(|b| &b.steps))
                .nth(ix)
                .unwrap()
                .id
        });
    assert_eq!(broke, Some(id), "{trace:#?}");
    // Everything shared after the break reads not reached.
    let steps = &trace.parents[0].steps;
    if let Some(at) = steps.iter().position(|s| s.id == id) {
        for later in &steps[at + 1..] {
            assert_eq!(later.verdict, StepVerdict::NotReached, "{later:?}");
        }
        if !trace.parents[0].branches.is_empty() {
            panic!("branches are traced only past the shared steps");
        }
    }
}

#[test]
fn a_healthy_route_serves_and_every_step_passes() {
    let trace = World::healthy().trace(healthy_route());
    assert_eq!(trace.verdict, RouteVerdict::Serving);
    assert!(!trace.stale);
    let parent = &trace.parents[0];
    assert_eq!(parent.controller.as_deref(), Some("example.io/gateway"));
    assert!(
        parent.steps.iter().all(|s| s.verdict == StepVerdict::Pass),
        "{parent:#?}"
    );
    let branch = &parent.branches[0];
    assert_eq!(branch.label, "shop/cart:8080");
    assert!(
        branch.steps.iter().all(|s| s.verdict == StepVerdict::Pass),
        "{branch:#?}"
    );
    assert_eq!(trace.summary.stops_at, None);
    assert!(trace.summary.reason.contains("infra/edge"));
    // The step quotes what the route asks and what the gateway serves.
    let listener = step(&trace, StepId::Listener);
    assert!(listener.asks.contains("shop.example.com"));
    assert!(listener.serves.contains("*.example.com"));
}

#[test]
fn the_label_is_the_first_host_and_path() {
    let route = RouteFacts::parse(&healthy_route()).unwrap();
    assert_eq!(route.label(), "shop.example.com/api");
    let route = RouteFacts::parse(&route_json(json!({"hostnames": []}), json!({}))).unwrap();
    assert_eq!(route.label(), "*/api");
}

#[test]
fn a_missing_parent_gateway_is_the_first_break() {
    let mut world = World::healthy();
    world.gateways = Ok(Listing::new(Vec::new(), false));
    let trace = world.trace(healthy_route());
    assert_eq!(trace.verdict, RouteVerdict::NotServing);
    assert_eq!(trace.parents[0].steps[0].owner, Some(Owner::Yours));
    assert!(trace.summary.reason.contains("no longer exists"));
    assert!(
        trace.parents[0].steps[1..]
            .iter()
            .all(|s| s.verdict == StepVerdict::NotReached)
    );
}

#[test]
fn a_refused_or_cut_gateway_list_is_unknown_never_not_serving() {
    let mut world = World::healthy();
    world.gateways = Err(refused());
    let trace = world.trace(healthy_route());
    assert_eq!(trace.verdict, RouteVerdict::Unknown);
    assert!(trace.summary.reason.contains("Forbidden"));
    world.gateways = Ok(Listing::new(Vec::new(), true));
    assert_eq!(world.trace(healthy_route()).verdict, RouteVerdict::Unknown);
}

#[test]
fn the_class_must_exist_and_be_claimed() {
    let mut world = World::healthy();
    world.classes = Ok(Listing::new(Vec::new(), false));
    let trace = world.trace(healthy_route());
    stops(&trace, StepId::Class);
    assert_eq!(step(&trace, StepId::Class).owner, Some(Owner::Yours));
    assert_eq!(trace.verdict, RouteVerdict::NotServing);

    let unclaimed = World::with(
        gateway_json(json!({})),
        class_json(json!({"conditions": [cond("Accepted", "False", "InvalidParameters", 1)]})),
    )
    .trace(healthy_route());
    stops(&unclaimed, StepId::Class);
    assert_eq!(step(&unclaimed, StepId::Class).verdict, StepVerdict::Fail);
    assert!(
        step(&unclaimed, StepId::Class)
            .controller
            .as_deref()
            .unwrap()
            .contains("InvalidParameters")
    );

    let blind = World::with(gateway_json(json!({})), class_json(json!({}))).trace(healthy_route());
    stops(&blind, StepId::Class);
    assert_eq!(step(&blind, StepId::Class).verdict, StepVerdict::Unknown);
    assert_eq!(blind.verdict, RouteVerdict::Unknown);
    assert!(step(&blind, StepId::Class).reason.contains("blind"));

    world.classes = Err(refused());
    assert_eq!(world.trace(healthy_route()).verdict, RouteVerdict::Unknown);
}

#[test]
fn pending_blind_and_failed_gateways_are_three_different_states() {
    let gateway = |status: Value| {
        let mut gateway = gateway_json(json!({}));
        gateway["status"] = status;
        gateway
    };
    // No status at all: blind.
    let none = World::with(gateway(json!({})), accepted_class()).trace(healthy_route());
    stops(&none, StepId::Gateway);
    assert!(step(&none, StepId::Gateway).reason.starts_with("blind"));
    assert_eq!(none.verdict, RouteVerdict::Unknown);
    // Programmed, no address: blind too, and the platform's to fix.
    let no_address = World::with(
        gateway(json!({"conditions": [cond("Accepted", "True", "Accepted", 2), cond("Programmed", "True", "Programmed", 2)]})),
        accepted_class(),
    )
    .trace(healthy_route());
    stops(&no_address, StepId::Gateway);
    assert_eq!(
        step(&no_address, StepId::Gateway).owner,
        Some(Owner::Platform)
    );
    assert!(
        step(&no_address, StepId::Gateway)
            .reason
            .contains("no address")
    );
    assert_eq!(no_address.verdict, RouteVerdict::Unknown);
    // Pending.
    let pending = World::with(
        gateway(json!({"conditions": [cond("Accepted", "True", "Accepted", 2), cond("Programmed", "Unknown", "Pending", 2)]})),
        accepted_class(),
    )
    .trace(healthy_route());
    assert!(
        step(&pending, StepId::Gateway)
            .reason
            .starts_with("pending")
    );
    assert_eq!(pending.verdict, RouteVerdict::Unknown);
    // Failed.
    let failed = World::with(
        gateway(json!({"conditions": [cond("Accepted", "True", "Accepted", 2), cond("Programmed", "False", "AddressNotAssigned", 2)]})),
        accepted_class(),
    )
    .trace(healthy_route());
    stops(&failed, StepId::Gateway);
    assert_eq!(step(&failed, StepId::Gateway).verdict, StepVerdict::Fail);
    assert_eq!(step(&failed, StepId::Gateway).owner, Some(Owner::Platform));
    assert_eq!(failed.verdict, RouteVerdict::NotServing);
}

#[test]
fn a_stale_gateway_verdict_is_a_warning_and_the_trace_goes_on() {
    let mut gateway = gateway_json(json!({}));
    gateway["metadata"]["generation"] = json!(5);
    let trace = World::with(gateway, accepted_class()).trace(healthy_route());
    assert!(trace.stale);
    let step = step(&trace, StepId::Gateway);
    assert_eq!(step.verdict, StepVerdict::Warn);
    assert!(step.stale && step.reason.contains("generation 2, now 5"));
    assert_eq!(trace.verdict, RouteVerdict::Serving);
}

#[test]
fn a_stale_route_verdict_is_a_warning_too() {
    let mut route = healthy_route();
    route["metadata"]["generation"] = json!(9);
    let trace = World::healthy().trace(route);
    assert!(trace.stale);
    assert_eq!(step(&trace, StepId::Namespace).verdict, StepVerdict::Warn);
    assert_eq!(trace.verdict, RouteVerdict::Serving);
}

#[test]
fn a_listener_that_does_not_serve_the_host_quotes_both_sides() {
    let route = route_json(json!({"hostnames": ["shop.example.org"]}), json!({}));
    let trace = World::healthy().trace(route);
    stops(&trace, StepId::Listener);
    let listener = step(&trace, StepId::Listener);
    assert_eq!(listener.owner, Some(Owner::Yours));
    assert!(listener.asks.contains("shop.example.org"));
    assert!(listener.serves.contains("*.example.com"));
    assert!(listener.reason.contains("match no listener hostname"));
    assert_eq!(trace.verdict, RouteVerdict::NotServing);
    assert!(
        trace
            .summary
            .line()
            .starts_with("stops at Listener accepts — ")
    );
}

#[test]
fn listener_names_ports_protocols_and_kinds_are_checked() {
    let world = World::healthy();
    let named = route_json(
        json!({"parentRefs": [{"name": "edge", "namespace": "infra", "sectionName": "web"}]}),
        json!({}),
    );
    assert!(
        step(&world.trace(named), StepId::Listener)
            .reason
            .contains("no listener named web")
    );
    let ported = route_json(
        json!({"parentRefs": [{"name": "edge", "namespace": "infra", "port": 8443}]}),
        json!({}),
    );
    assert!(
        step(&world.trace(ported), StepId::Listener)
            .reason
            .contains("port 8443")
    );

    let tcp = World::with(
        gateway_json(
            json!({"spec": {"listeners": [{"name": "https", "port": 443, "protocol": "TCP"}]}}),
        ),
        accepted_class(),
    );
    assert!(
        step(&tcp.trace(healthy_route()), StepId::Listener)
            .reason
            .contains("no HTTP or HTTPS listener")
    );

    let kinds = World::with(
        gateway_json(
            json!({"spec": {"listeners": [{"name": "https", "port": 443, "protocol": "HTTPS",
            "allowedRoutes": {"kinds": [{"kind": "GRPCRoute"}]}}]}}),
        ),
        accepted_class(),
    );
    assert!(
        step(&kinds.trace(healthy_route()), StepId::Listener)
            .reason
            .contains("does not allow HTTPRoute")
    );
}

#[test]
fn an_unaccepted_listener_breaks_the_listener_step_with_the_controllers_words() {
    let gateway = gateway_json(
        json!({"status": {"listeners": [{"name": "https", "attachedRoutes": 0,
        "conditions": [cond("Accepted", "False", "PortUnavailable", 2)]}]}}),
    );
    let trace = World::with(gateway, accepted_class()).trace(healthy_route());
    stops(&trace, StepId::Listener);
    let listener = step(&trace, StepId::Listener);
    assert_eq!(listener.owner, Some(Owner::Platform));
    assert!(
        listener
            .controller
            .as_deref()
            .unwrap()
            .contains("PortUnavailable")
    );
    assert_eq!(trace.verdict, RouteVerdict::NotServing);
}

#[test]
fn a_listener_with_a_bad_certificate_ref_is_yours() {
    let gateway = gateway_json(
        json!({"status": {"listeners": [{"name": "https", "attachedRoutes": 0,
        "conditions": [cond("Accepted", "True", "Accepted", 2), cond("ResolvedRefs", "False", "InvalidCertificateRef", 2)]}]}}),
    );
    let trace = World::with(gateway, accepted_class()).trace(healthy_route());
    assert_eq!(step(&trace, StepId::Listener).owner, Some(Owner::Yours));
}

#[test]
fn an_accepted_listener_is_preferred_to_one_that_is_not() {
    let gateway = gateway_json(json!({
        "spec": {"listeners": [
            {"name": "a", "port": 443, "protocol": "HTTPS"},
            {"name": "b", "port": 8443, "protocol": "HTTPS"}]},
        "status": {"listeners": [
            {"name": "a", "conditions": [cond("Accepted", "False", "PortUnavailable", 2)]},
            {"name": "b", "conditions": [cond("Accepted", "True", "Accepted", 2)]}]},
    }));
    let route = route_json(
        json!({"parentRefs": [{"name": "edge", "namespace": "infra"}]}),
        json!({
            "parents": [{"parentRef": {"name": "edge", "namespace": "infra"}, "controllerName": "c",
                "conditions": [cond("Accepted", "True", "Accepted", 3)]}]
        }),
    );
    let trace = World::with(gateway, accepted_class()).trace(route);
    assert!(step(&trace, StepId::Listener).serves.contains("listener b"));
}

#[test]
fn the_namespace_must_be_allowed() {
    let same = |from: &str| {
        gateway_json(
            json!({"spec": {"listeners": [{"name": "https", "port": 443, "protocol": "HTTPS",
            "hostname": "*.example.com", "allowedRoutes": {"namespaces": {"from": from}}}]}}),
        )
    };
    let trace = World::with(same("Same"), accepted_class()).trace(healthy_route());
    stops(&trace, StepId::Namespace);
    let namespace = step(&trace, StepId::Namespace);
    assert_eq!(namespace.owner, Some(Owner::Platform));
    assert!(namespace.asks.contains("shop") && namespace.serves.contains("infra"));
    assert_eq!(trace.verdict, RouteVerdict::NotServing);
}

#[test]
fn a_label_selector_is_checked_against_the_namespaces_labels() {
    let gateway = |value: &str| {
        gateway_json(
            json!({"spec": {"listeners": [{"name": "https", "port": 443, "protocol": "HTTPS",
            "hostname": "*.example.com", "allowedRoutes": {"namespaces": {"from": "Selector",
                "selector": {"matchLabels": {"team": value}}}}}]}}),
        )
    };
    let ok = World::with(gateway("web"), accepted_class()).trace(healthy_route());
    assert_eq!(ok.verdict, RouteVerdict::Serving);
    assert!(step(&ok, StepId::Namespace).serves.contains("team=web"));
    let no = World::with(gateway("data"), accepted_class()).trace(healthy_route());
    stops(&no, StepId::Namespace);
    assert_eq!(step(&no, StepId::Namespace).verdict, StepVerdict::Fail);
    let mut refused_world = World::with(gateway("web"), accepted_class());
    refused_world.namespaces = Err(refused());
    let unknown = refused_world.trace(healthy_route());
    assert_eq!(
        step(&unknown, StepId::Namespace).verdict,
        StepVerdict::Unknown
    );
    assert_eq!(unknown.verdict, RouteVerdict::Unknown);
}

#[test]
fn a_cut_namespace_list_is_unknown_even_when_the_namespace_is_in_it() {
    let gateway = gateway_json(
        json!({"spec": {"listeners": [{"name": "https", "port": 443, "protocol": "HTTPS",
        "hostname": "*.example.com", "allowedRoutes": {"namespaces": {"from": "Selector",
            "selector": {"matchLabels": {"team": "data"}}}}}]}}),
    );
    let mut world = World::with(gateway, accepted_class());
    // The labels in the partial list say "broken", but the list is cut.
    world.namespaces = Ok(Listing::new(
        vec![(
            "shop".into(),
            [("team".to_owned(), "web".to_owned())].into(),
        )],
        true,
    ));
    let trace = world.trace(healthy_route());
    assert_eq!(
        step(&trace, StepId::Namespace).verdict,
        StepVerdict::Unknown
    );
    assert!(step(&trace, StepId::Namespace).reason.contains("cut"));
    assert_eq!(trace.verdict, RouteVerdict::Unknown);
}

#[test]
fn a_listeners_certificate_refs_are_shown_by_name_and_nothing_reads_them() {
    let gateway = gateway_json(
        json!({"spec": {"listeners": [{"name": "https", "port": 443, "protocol": "HTTPS",
        "hostname": "*.example.com", "tls": {"certificateRefs": [{"name": "shop-tls"}, {"name": "spare-tls"}]},
        "allowedRoutes": {"namespaces": {"from": "All"}}}]}}),
    );
    let trace = World::with(gateway, accepted_class()).trace(healthy_route());
    assert!(
        step(&trace, StepId::Listener)
            .serves
            .contains("certificate shop-tls, spare-tls")
    );
}

#[test]
fn selector_expressions_match_like_kubernetes() {
    let labels: BTreeMap<String, String> = [("env".to_owned(), "prod".to_owned())].into();
    let selector = |op: &str, key: &str, values: &[&str]| Selector {
        labels: BTreeMap::new(),
        expressions: vec![(
            key.into(),
            op.into(),
            values.iter().map(|v| (*v).into()).collect(),
        )],
    };
    assert!(selector("In", "env", &["prod", "stage"]).matches(&labels));
    assert!(!selector("In", "env", &["stage"]).matches(&labels));
    assert!(selector("NotIn", "env", &["stage"]).matches(&labels));
    assert!(selector("NotIn", "other", &["x"]).matches(&labels));
    assert!(selector("Exists", "env", &[]).matches(&labels));
    assert!(selector("DoesNotExist", "other", &[]).matches(&labels));
    assert!(!selector("Bogus", "env", &[]).matches(&labels));
    assert!(Selector::default().matches(&labels));
}

#[test]
fn the_controller_saying_no_is_its_own_line_and_breaks_the_trace() {
    let status = json!({"parents": [{
        "parentRef": {"name": "edge", "namespace": "infra", "sectionName": "https"},
        "controllerName": "example.io/gateway",
        "conditions": [cond("Accepted", "False", "NotAllowedByListeners", 3)],
    }]});
    let trace = World::healthy().trace(route_json(json!({}), status));
    stops(&trace, StepId::Namespace);
    let namespace = step(&trace, StepId::Namespace);
    assert_eq!(namespace.owner, Some(Owner::Platform));
    assert!(
        namespace
            .controller
            .as_deref()
            .unwrap()
            .contains("NotAllowedByListeners")
    );
    // Configuration passed; only the controller's status disagrees.
    assert!(namespace.serves.contains("from All"));
}

#[test]
fn a_controller_that_writes_no_status_for_the_route_is_blind() {
    let trace = World::healthy().trace(route_json(json!({}), json!({"parents": []})));
    stops(&trace, StepId::Namespace);
    assert!(step(&trace, StepId::Namespace).reason.starts_with("blind"));
    assert_eq!(trace.verdict, RouteVerdict::Unknown);
}

fn cross_namespace_route() -> Value {
    route_json(
        json!({"rules": [{"backendRefs": [{"name": "cart", "namespace": "payments", "port": 8080}]}]}),
        json!({}),
    )
}

fn payments_cart() -> BTreeMap<(String, String), BackendRead> {
    [(
        ("payments".to_owned(), "cart".to_owned()),
        Ok(Some(service_reading(1, 0, &[8080]))),
    )]
    .into()
}

#[test]
fn a_cross_namespace_ref_without_a_grant_is_refused_and_proposes_one_to_copy() {
    let mut world = World::healthy();
    world.backends = payments_cart();
    let trace = world.trace(cross_namespace_route());
    let refs = step(&trace, StepId::Refs);
    assert_eq!(refs.verdict, StepVerdict::Fail);
    assert_eq!(refs.owner, Some(Owner::Platform));
    assert_eq!(trace.verdict, RouteVerdict::NotServing);
    assert_eq!(trace.summary.stops_at, Some("References resolve"));
    let proposals = trace.proposals();
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0].namespace, "payments");
    assert_eq!(proposals[0].from_namespace, "shop");
    assert_eq!(
        proposals[0].yaml,
        "apiVersion: gateway.networking.k8s.io/v1beta1\n\
         kind: ReferenceGrant\n\
         metadata:\n  name: allow-shop-to-cart\n  namespace: payments\n\
         spec:\n  from:\n    - group: gateway.networking.k8s.io\n      kind: HTTPRoute\n      namespace: shop\n  \
         to:\n    - group: \"\"\n      kind: Service\n      name: cart\n"
    );
    // The branch stops there: the Service and endpoints were not reached.
    let branch = &trace.parents[0].branches[0];
    assert_eq!(branch.steps[1].verdict, StepVerdict::NotReached);
    assert_eq!(branch.steps[2].verdict, StepVerdict::NotReached);
}

#[test]
fn a_grant_for_exactly_that_pair_resolves_the_ref() {
    let grant = |from: &str, name: Option<&str>| {
        let mut to = json!({"group": "", "kind": "Service"});
        if let Some(name) = name {
            to["name"] = json!(name);
        }
        GrantFacts::parse(&json!({
            "metadata": {"name": "allow", "namespace": "payments"},
            "spec": {"from": [{"group": "gateway.networking.k8s.io", "kind": "HTTPRoute", "namespace": from}], "to": [to]},
        }))
        .unwrap()
    };
    for (grant, resolves) in [
        (grant("shop", Some("cart")), true),
        (grant("shop", None), true),
        (grant("shop", Some("other")), false),
        (grant("elsewhere", Some("cart")), false),
    ] {
        let mut world = World::healthy();
        world.backends = payments_cart();
        world.grants = Ok(Listing::new(vec![grant], false));
        let trace = world.trace(cross_namespace_route());
        let refs = step(&trace, StepId::Refs);
        assert_eq!(refs.verdict == StepVerdict::Pass, resolves, "{refs:?}");
        assert_eq!(trace.verdict == RouteVerdict::Serving, resolves);
        assert_eq!(trace.proposals().is_empty(), resolves);
    }
}

#[test]
fn a_grant_in_the_wrong_namespace_does_not_count() {
    let grant = GrantFacts::parse(&json!({
        "metadata": {"name": "allow", "namespace": "other"},
        "spec": {"from": [{"group": "gateway.networking.k8s.io", "kind": "HTTPRoute", "namespace": "shop"}],
            "to": [{"group": "", "kind": "Service"}]},
    }))
    .unwrap();
    let mut world = World::healthy();
    world.backends = payments_cart();
    world.grants = Ok(Listing::new(vec![grant], false));
    assert_eq!(
        step(&world.trace(cross_namespace_route()), StepId::Refs).verdict,
        StepVerdict::Fail
    );
}

#[test]
fn unreadable_or_cut_grants_are_unknown_and_propose_nothing() {
    let mut world = World::healthy();
    world.backends = payments_cart();
    world.grants = Err(refused());
    let trace = world.trace(cross_namespace_route());
    assert_eq!(step(&trace, StepId::Refs).verdict, StepVerdict::Unknown);
    assert_eq!(trace.verdict, RouteVerdict::Unknown);
    assert!(trace.proposals().is_empty());
    world.grants = Ok(Listing::new(Vec::new(), true));
    let cut = world.trace(cross_namespace_route());
    assert_eq!(step(&cut, StepId::Refs).verdict, StepVerdict::Unknown);
    assert!(cut.proposals().is_empty());
}

#[test]
fn a_controller_that_still_refuses_a_granted_ref_is_the_controllers() {
    let grant = GrantFacts::parse(&json!({
        "metadata": {"name": "allow", "namespace": "payments"},
        "spec": {"from": [{"group": "gateway.networking.k8s.io", "kind": "HTTPRoute", "namespace": "shop"}],
            "to": [{"group": "", "kind": "Service"}]},
    }))
    .unwrap();
    let mut world = World::healthy();
    world.backends = payments_cart();
    world.grants = Ok(Listing::new(vec![grant], false));
    let status = json!({"parents": [{
        "parentRef": {"name": "edge", "namespace": "infra", "sectionName": "https"},
        "controllerName": "example.io/gateway",
        "conditions": [cond("Accepted", "True", "Accepted", 3), cond("ResolvedRefs", "False", "RefNotPermitted", 3)],
    }]});
    let trace = world.trace(route_json(
        json!({"rules": [{"backendRefs": [{"name": "cart", "namespace": "payments", "port": 8080}]}]}),
        status,
    ));
    let refs = step(&trace, StepId::Refs);
    assert_eq!(refs.verdict, StepVerdict::Fail);
    assert_eq!(refs.owner, Some(Owner::Controller));
    assert!(
        refs.controller
            .as_deref()
            .unwrap()
            .contains("RefNotPermitted")
    );
}

#[test]
fn names_that_are_not_dns_names_get_no_proposal() {
    let route = route_json(
        json!({"rules": [{"backendRefs": [{"name": "Cart\nkind: Secret", "namespace": "payments", "port": 8080}]}]}),
        json!({}),
    );
    let mut world = World::healthy();
    world.backends = BTreeMap::new();
    let trace = world.trace(route);
    assert_eq!(step(&trace, StepId::Refs).verdict, StepVerdict::Fail);
    assert!(trace.proposals().is_empty());
}

#[test]
fn the_service_must_exist_and_have_the_port() {
    let mut world = World::healthy();
    world
        .backends
        .insert(("shop".into(), "cart".into()), Ok(None));
    let missing = world.trace(healthy_route());
    assert_eq!(step(&missing, StepId::Service).verdict, StepVerdict::Fail);
    assert_eq!(step(&missing, StepId::Service).owner, Some(Owner::Yours));
    assert_eq!(
        step(&missing, StepId::Endpoints).verdict,
        StepVerdict::NotReached
    );
    assert_eq!(missing.verdict, RouteVerdict::NotServing);

    world.backends.insert(
        ("shop".into(), "cart".into()),
        Ok(Some(service_reading(2, 0, &[80, 443]))),
    );
    let wrong = world.trace(healthy_route());
    let service = step(&wrong, StepId::Service);
    assert_eq!(service.verdict, StepVerdict::Fail);
    assert!(service.asks.contains("8080") && service.serves.contains("80, 443"));

    let no_port = world.trace(route_json(
        json!({"rules": [{"backendRefs": [{"name": "cart"}]}]}),
        json!({}),
    ));
    assert!(
        step(&no_port, StepId::Service)
            .reason
            .contains("must name a port")
    );
}

#[test]
fn a_refused_or_unread_service_is_unknown() {
    let mut world = World::healthy();
    world
        .backends
        .insert(("shop".into(), "cart".into()), Err(refused()));
    let trace = world.trace(healthy_route());
    assert_eq!(step(&trace, StepId::Service).verdict, StepVerdict::Unknown);
    assert_eq!(trace.verdict, RouteVerdict::Unknown);
    world.backends.clear();
    let unread = world.trace(healthy_route());
    assert!(step(&unread, StepId::Service).reason.contains("not read"));
    assert_eq!(unread.verdict, RouteVerdict::Unknown);
}

#[test]
fn endpoints_pass_warn_or_fail_by_what_is_ready() {
    let mut world = World::healthy();
    let with = |world: &mut World, ready, not_ready| {
        world.backends.insert(
            ("shop".into(), "cart".into()),
            Ok(Some(service_reading(ready, not_ready, &[8080]))),
        );
    };
    with(&mut world, 1, 2);
    let warn = world.trace(healthy_route());
    assert_eq!(step(&warn, StepId::Endpoints).verdict, StepVerdict::Warn);
    assert_eq!(warn.verdict, RouteVerdict::Serving);
    with(&mut world, 0, 2);
    let none_ready = world.trace(healthy_route());
    assert_eq!(
        step(&none_ready, StepId::Endpoints).verdict,
        StepVerdict::Fail
    );
    assert_eq!(none_ready.verdict, RouteVerdict::NotServing);
    with(&mut world, 0, 0);
    let none = world.trace(healthy_route());
    assert!(
        step(&none, StepId::Endpoints)
            .reason
            .contains("publishes no endpoints")
    );
    let mut broken = service_reading(1, 0, &[8080]);
    broken.published = Err(refused());
    world
        .backends
        .insert(("shop".into(), "cart".into()), Ok(Some(broken)));
    let unknown = world.trace(healthy_route());
    assert_eq!(
        step(&unknown, StepId::Endpoints).verdict,
        StepVerdict::Unknown
    );
    assert_eq!(unknown.verdict, RouteVerdict::Unknown);
}

fn split_route(weights: [(&str, i64); 2]) -> Value {
    route_json(
        json!({"rules": [{"backendRefs": weights.iter()
            .map(|(name, weight)| json!({"name": name, "port": 8080, "weight": weight}))
            .collect::<Vec<_>>()}]}),
        json!({}),
    )
}

fn split_world() -> World {
    let mut world = World::healthy();
    world.backends.insert(
        ("shop".into(), "canary".into()),
        Ok(Some(service_reading(1, 0, &[8080]))),
    );
    world
}

#[test]
fn weighted_backends_are_a_split_and_a_broken_branch_is_partly_serving() {
    let mut world = split_world();
    let healthy = world.trace(split_route([("cart", 80), ("canary", 20)]));
    let shares = healthy.parents[0]
        .branches
        .iter()
        .map(|b| b.share)
        .collect::<Vec<_>>();
    assert_eq!(shares, [Some(80), Some(20)]);
    assert_eq!(healthy.verdict, RouteVerdict::Serving);

    world
        .backends
        .insert(("shop".into(), "canary".into()), Ok(None));
    let partly = world.trace(split_route([("cart", 80), ("canary", 20)]));
    assert_eq!(partly.verdict, RouteVerdict::Partly);
    assert_eq!(partly.parents[0].branches[1].verdict, BranchVerdict::Broken);
    assert!(partly.summary.stops_at.is_some());

    world
        .backends
        .insert(("shop".into(), "cart".into()), Ok(None));
    let none = world.trace(split_route([("cart", 80), ("canary", 20)]));
    assert_eq!(none.verdict, RouteVerdict::NotServing);
}

#[test]
fn a_branch_with_zero_weight_takes_no_traffic_and_is_not_judged() {
    let mut world = split_world();
    world
        .backends
        .insert(("shop".into(), "canary".into()), Ok(None));
    let trace = world.trace(split_route([("cart", 1), ("canary", 0)]));
    assert_eq!(trace.verdict, RouteVerdict::Serving);
    assert_eq!(
        trace.parents[0].branches[1].verdict,
        BranchVerdict::Excluded
    );
}

#[test]
fn an_unknown_branch_beside_a_serving_one_keeps_the_route_unknown() {
    let mut world = split_world();
    world
        .backends
        .insert(("shop".into(), "canary".into()), Err(refused()));
    let trace = world.trace(split_route([("cart", 50), ("canary", 50)]));
    assert_eq!(trace.verdict, RouteVerdict::Unknown);
    // A broken branch beside a serving one is still partly serving.
    world
        .backends
        .insert(("shop".into(), "cart".into()), Ok(None));
    let both = world.trace(split_route([("cart", 50), ("canary", 50)]));
    assert_eq!(both.verdict, RouteVerdict::Unknown);
}

#[test]
fn redirect_only_extension_and_other_backends_are_not_missing_backends() {
    let world = World::healthy();
    let redirect = world.trace(route_json(
        json!({"rules": [{"filters": [{"type": "RequestRedirect", "requestRedirect": {"scheme": "https"}}]}]}),
        json!({}),
    ));
    assert_eq!(redirect.verdict, RouteVerdict::Serving);
    assert_eq!(redirect.parents[0].branches[0].kind, BranchKind::Redirect);

    let extension = world.trace(route_json(
        json!({"rules": [{"filters": [{"type": "ExtensionRef", "extensionRef": {"group": "x", "kind": "Y", "name": "z"}}]}]}),
        json!({}),
    ));
    assert_eq!(extension.verdict, RouteVerdict::Unknown);
    assert_eq!(extension.parents[0].branches[0].kind, BranchKind::Extension);
    assert!(
        extension.parents[0].branches[0].steps[0]
            .reason
            .contains("extension")
    );

    let other = world.trace(route_json(
        json!({"rules": [{"backendRefs": [{"group": "storage.example.io", "kind": "Bucket", "name": "assets"}]}]}),
        json!({}),
    ));
    assert_eq!(other.verdict, RouteVerdict::Unknown);
    assert_eq!(other.parents[0].branches[0].kind, BranchKind::Other);
    assert_ne!(other.verdict, RouteVerdict::NotServing);

    let empty = world.trace(route_json(json!({"rules": [{}]}), json!({})));
    assert_eq!(empty.verdict, RouteVerdict::NotServing);
    assert_eq!(empty.parents[0].branches[0].kind, BranchKind::Empty);
}

#[test]
fn a_route_with_a_service_parent_is_a_mesh_route_and_is_not_judged() {
    let route = route_json(
        json!({"parentRefs": [{"group": "", "kind": "Service", "name": "cart"}]}),
        json!({"parents": []}),
    );
    let trace = World::healthy().trace(route);
    assert_eq!(trace.verdict, RouteVerdict::Mesh);
    assert!(trace.parents[0].steps.is_empty());
    assert!(trace.summary.reason.contains("mesh"));
}

#[test]
fn a_route_with_no_parent_refs_attaches_to_nothing() {
    let trace = World::healthy().trace(route_json(
        json!({"parentRefs": []}),
        json!({"parents": []}),
    ));
    assert_eq!(trace.verdict, RouteVerdict::NotServing);
    assert_eq!(trace.summary.stops_at, Some("Parent"));
    assert_eq!(trace.summary.owner, Some(Owner::Yours));
}

#[test]
fn two_parents_serve_partly_when_one_is_down() {
    let route = route_json(
        json!({"parentRefs": [
            {"name": "edge", "namespace": "infra", "sectionName": "https"},
            {"name": "gone", "namespace": "infra"}]}),
        json!({}),
    );
    let trace = World::healthy().trace(route);
    assert_eq!(trace.parents.len(), 2);
    assert_eq!(trace.parents[0].verdict, RouteVerdict::Serving);
    assert_eq!(trace.parents[1].verdict, RouteVerdict::NotServing);
    assert_eq!(trace.verdict, RouteVerdict::Partly);
}

#[test]
fn each_controller_that_wrote_a_verdict_gets_its_own_trace() {
    let entry = |controller: &str, status: &str| {
        json!({"parentRef": {"name": "edge", "namespace": "infra", "sectionName": "https"},
            "controllerName": controller,
            "conditions": [cond("Accepted", status, "Accepted", 3), cond("ResolvedRefs", "True", "ResolvedRefs", 3)]})
    };
    let trace = World::healthy().trace(route_json(
        json!({}),
        json!({"parents": [entry("a.io/gateway", "True"), entry("b.io/gateway", "False")]}),
    ));
    assert_eq!(trace.parents.len(), 2);
    assert_eq!(trace.parents[0].controller.as_deref(), Some("a.io/gateway"));
    assert_eq!(trace.parents[0].verdict, RouteVerdict::Serving);
    assert_eq!(trace.parents[1].controller.as_deref(), Some("b.io/gateway"));
    assert_eq!(trace.parents[1].verdict, RouteVerdict::NotServing);
}

#[test]
fn a_parent_ref_without_a_namespace_means_the_routes_own() {
    let route = route_json(
        json!({"parentRefs": [{"name": "edge"}]}),
        json!({"parents": [{"parentRef": {"name": "edge", "namespace": "shop"}, "controllerName": "c",
            "conditions": [cond("Accepted", "True", "Accepted", 3)]}]}),
    );
    let mut world = World::healthy();
    world.gateways = Ok(Listing::new(
        vec![
            GatewayFacts::parse(&gateway_json(json!({"metadata": {"namespace": "shop"}}))).unwrap(),
        ],
        false,
    ));
    let trace = world.trace(route);
    assert_eq!(trace.parents[0].controller.as_deref(), Some("c"));
    assert_eq!(trace.verdict, RouteVerdict::Serving);
}

#[test]
fn verdicts_sort_not_serving_first() {
    let mut all = RouteVerdict::ALL.to_vec();
    all.sort();
    assert_eq!(all, RouteVerdict::ALL);
    assert_eq!(RouteVerdict::NotServing.label(), "Not serving");
}

// -- reads -------------------------------------------------------------

const GROUPS: &str = "/apis";
const V1: &str = "/apis/gateway.networking.k8s.io/v1";
const V1BETA1: &str = "/apis/gateway.networking.k8s.io/v1beta1";

fn resources(kinds: &[(&str, &str)]) -> (u16, String) {
    (
        200,
        json!({"resources": kinds.iter().map(|(name, kind)| json!({
            "name": name, "kind": kind, "namespaced": *kind != "GatewayClass",
            "verbs": ["get", "list", "watch"]})).collect::<Vec<_>>()})
        .to_string(),
    )
}

fn groups(versions: &[&str]) -> (u16, String) {
    (
        200,
        json!({"groups": [{"name": "gateway.networking.k8s.io",
            "versions": versions.iter().map(|v| json!({"groupVersion": format!("gateway.networking.k8s.io/{v}"), "version": v})).collect::<Vec<_>>(),
            "preferredVersion": {"groupVersion": format!("gateway.networking.k8s.io/{}", versions[0]), "version": versions[0]}}]})
        .to_string(),
    )
}

fn list(items: Vec<Value>) -> (u16, String) {
    (200, json!({"metadata": {}, "items": items}).to_string())
}

fn all_kinds() -> (u16, String) {
    resources(&[
        ("httproutes", "HTTPRoute"),
        ("gateways", "Gateway"),
        ("gatewayclasses", "GatewayClass"),
        ("referencegrants", "ReferenceGrant"),
    ])
}

fn gateway_api(request: &crate::fake_api::Recorded) -> Option<(u16, String)> {
    match request.path.as_str() {
        GROUPS => Some(groups(&["v1"])),
        V1 => Some(all_kinds()),
        "/apis/gateway.networking.k8s.io/v1/httproutes" => Some(list(vec![healthy_route()])),
        "/apis/gateway.networking.k8s.io/v1/gateways" => Some(list(vec![gateway_json(json!({}))])),
        "/apis/gateway.networking.k8s.io/v1/gatewayclasses" => Some(list(vec![accepted_class()])),
        "/apis/gateway.networking.k8s.io/v1/referencegrants" => Some(list(vec![])),
        "/api/v1/namespaces/shop/services/cart" => Some((
            200,
            json!({"metadata": {"name": "cart", "namespace": "shop"},
                "spec": {"selector": {"app": "cart"}, "ports": [{"port": 8080, "targetPort": 8080}]}})
            .to_string(),
        )),
        "/apis/discovery.k8s.io/v1/namespaces/shop/endpointslices" => Some(list(vec![json!({
            "metadata": {"name": "cart-a", "namespace": "shop", "labels": {"kubernetes.io/service-name": "cart"}},
            "addressType": "IPv4", "ports": [{"port": 8080}],
            "endpoints": [{"addresses": ["10.0.0.1"], "conditions": {"ready": true}}]})])),
        _ => None,
    }
}

fn only_gets(recorder: &crate::fake_api::Recorder) {
    recorder.assert_all("the route trace only reads", |r| {
        r.method == http::Method::GET
    });
}

#[tokio::test]
async fn reading_routes_traces_them_with_gets_and_lists_only() {
    let (client, recorder) = crate::fake_api::client(gateway_api);
    let read = read_routes(&client, None).await.unwrap();
    assert_eq!(read.api, ApiState::Ready { mixed: None });
    assert_eq!(read.routes.len(), 1);
    assert_eq!(read.routes[0].trace.verdict, RouteVerdict::Serving);
    assert!(read.notes.is_empty(), "{:?}", read.notes);
    only_gets(&recorder);
    let lines = recorder.lines();
    assert!(
        lines
            .iter()
            .any(|l| l == "GET /apis/gateway.networking.k8s.io/v1/httproutes?limit=500"),
        "{lines:?}"
    );
    assert!(lines.iter().any(|l| l.contains("gateways?limit=200")));
    assert!(lines.iter().any(|l| l.contains("gatewayclasses?limit=100")));
    // Nothing reads a Secret or a namespace the listeners don't select by.
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("secrets") || l.contains("/namespaces?")),
        "{lines:?}"
    );
}

#[tokio::test]
async fn one_route_is_read_by_name_not_listed() {
    let (client, recorder) = crate::fake_api::client(|request| match request.path.as_str() {
        "/apis/gateway.networking.k8s.io/v1/namespaces/shop/httproutes/web" => {
            Some((200, healthy_route().to_string()))
        }
        _ => gateway_api(request),
    });
    let read = read_routes(&client, Some(("shop", "web"))).await.unwrap();
    assert_eq!(read.routes.len(), 1);
    only_gets(&recorder);
    assert!(
        !recorder
            .lines()
            .iter()
            .any(|l| l.contains("httproutes?limit"))
    );
}

#[tokio::test]
async fn a_missing_route_is_an_error_not_an_empty_list() {
    let (client, _) = crate::fake_api::client(|request| {
        match request.path.as_str() {
        "/apis/gateway.networking.k8s.io/v1/namespaces/shop/httproutes/web" => Some((
            404,
            json!({"kind": "Status", "apiVersion": "v1", "status": "Failure", "code": 404, "reason": "NotFound", "message": "not found"}).to_string(),
        )),
        _ => gateway_api(request),
    }
    });
    let error = read_routes(&client, Some(("shop", "web")))
        .await
        .unwrap_err();
    assert_eq!(error.kind, FailureKind::NotFound);
}

#[tokio::test]
async fn an_unserved_gateway_api_reads_plainly() {
    let (client, recorder) = crate::fake_api::client(|request| match request.path.as_str() {
        GROUPS => Some((200, json!({"groups": []}).to_string())),
        _ => None,
    });
    let read = read_routes(&client, None).await.unwrap();
    assert_eq!(read.api, ApiState::NotInstalled);
    assert!(read.routes.is_empty());
    assert!(read.api.problem().unwrap().contains("not installed"));
    recorder.assert_lines(&["GET /apis"]);
}

#[tokio::test]
async fn a_partly_installed_gateway_api_names_what_is_missing() {
    let (client, _) = crate::fake_api::client(|request| match request.path.as_str() {
        GROUPS => Some(groups(&["v1"])),
        V1 => Some(resources(&[("httproutes", "HTTPRoute")])),
        _ => None,
    });
    let read = read_routes(&client, None).await.unwrap();
    assert_eq!(
        read.api,
        ApiState::Incomplete {
            missing: vec!["Gateway", "GatewayClass"]
        }
    );
    assert!(
        read.api
            .problem()
            .unwrap()
            .contains("Gateway, GatewayClass")
    );
}

#[tokio::test]
async fn a_mixed_bundle_is_said_and_each_kind_is_read_at_its_own_version() {
    let (client, recorder) = crate::fake_api::client(|request| match request.path.as_str() {
        GROUPS => Some(groups(&["v1", "v1beta1"])),
        V1 => Some(resources(&[
            ("httproutes", "HTTPRoute"),
            ("gatewayclasses", "GatewayClass"),
        ])),
        V1BETA1 => Some(resources(&[
            ("gateways", "Gateway"),
            ("httproutes", "HTTPRoute"),
            ("referencegrants", "ReferenceGrant"),
        ])),
        "/apis/gateway.networking.k8s.io/v1/httproutes" => Some(list(vec![])),
        "/apis/gateway.networking.k8s.io/v1beta1/gateways" => Some(list(vec![])),
        "/apis/gateway.networking.k8s.io/v1/gatewayclasses" => Some(list(vec![])),
        "/apis/gateway.networking.k8s.io/v1beta1/referencegrants" => Some(list(vec![])),
        _ => None,
    });
    let read = read_routes(&client, None).await.unwrap();
    let mixed = read.api.mixed().expect("mixed");
    assert!(
        mixed.contains("Gateway v1beta1") && mixed.contains("HTTPRoute v1,"),
        "{mixed}"
    );
    only_gets(&recorder);
}

#[tokio::test]
async fn a_refused_route_list_is_an_error_and_a_refused_gateway_list_is_unknown_steps() {
    let (client, _) = crate::fake_api::client(|request| {
        match request.path.as_str() {
        "/apis/gateway.networking.k8s.io/v1/httproutes" => Some((
            403,
            json!({"kind": "Status", "apiVersion": "v1", "status": "Failure", "code": 403, "reason": "Forbidden", "message": "no"}).to_string(),
        )),
        _ => gateway_api(request),
    }
    });
    assert_eq!(
        read_routes(&client, None).await.unwrap_err().kind,
        FailureKind::Forbidden
    );

    let (client, _) = crate::fake_api::client(|request| {
        match request.path.as_str() {
        "/apis/gateway.networking.k8s.io/v1/gateways" => Some((
            403,
            json!({"kind": "Status", "apiVersion": "v1", "status": "Failure", "code": 403, "reason": "Forbidden", "message": "no"}).to_string(),
        )),
        _ => gateway_api(request),
    }
    });
    let read = read_routes(&client, None).await.unwrap();
    assert_eq!(read.routes[0].trace.verdict, RouteVerdict::Unknown);
    assert!(
        read.notes
            .iter()
            .any(|n| n.contains("Gateways could not be read"))
    );
}

#[tokio::test]
async fn cut_lists_and_backends_past_the_limit_are_said() {
    let routes = (0..BACKEND_LIMIT + 5)
        .map(|n| {
            route_json(
                json!({"rules": [{"backendRefs": [{"name": format!("svc-{n}"), "port": 80}]}]}),
                json!({}),
            )
        })
        .collect::<Vec<_>>();
    let (client, _) = crate::fake_api::client(move |request| {
        match request.path.as_str() {
        "/apis/gateway.networking.k8s.io/v1/httproutes" => Some((
            200,
            json!({"metadata": {"continue": "more"}, "items": routes}).to_string(),
        )),
        path if path.starts_with("/api/v1/namespaces/shop/services/svc-") => Some((
            200,
            json!({"metadata": {"name": "svc", "namespace": "shop"}, "spec": {"ports": [{"port": 80}]}}).to_string(),
        )),
        "/apis/discovery.k8s.io/v1/namespaces/shop/endpointslices" => Some(list(vec![])),
        _ => gateway_api(request),
    }
    });
    let read = read_routes(&client, None).await.unwrap();
    assert!(read.routes_cut);
    assert_eq!(read.backends_cut, 5);
    assert!(read.notes.iter().any(|n| n.contains("cut at 500")));
    assert!(read.notes.iter().any(|n| n.contains("past 100")));
    let unread = read
        .routes
        .iter()
        .filter(|r| r.trace.summary.reason.contains("not read"))
        .count();
    assert_eq!(unread, 5);
}

#[test]
fn the_grant_version_in_text_to_copy_is_a_plain_version() {
    let kind = |version: &str| GatewayKinds {
        route: ResourceKind::new(GATEWAY_GROUP, "v1", "HTTPRoute", "httproutes", true),
        gateway: ResourceKind::new(GATEWAY_GROUP, "v1", "Gateway", "gateways", true),
        class: ResourceKind::new(GATEWAY_GROUP, "v1", "GatewayClass", "gatewayclasses", false),
        grant: Some(ResourceKind::new(
            GATEWAY_GROUP,
            version,
            "ReferenceGrant",
            "referencegrants",
            true,
        )),
    };
    use super::read::grant_version as version;
    assert_eq!(version(&kind("v1")), "v1");
    assert_eq!(version(&kind("v1beta1")), "v1beta1");
    assert_eq!(version(&kind("v1alpha2")), "v1alpha2");
    assert_eq!(version(&kind("v1\nkind: Secret")), "v1beta1");
    assert_eq!(version(&kind("beta")), "v1beta1");
}

use crate::resources::ResourceKind;
