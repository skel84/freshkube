//! Invented Gateway API objects for the Routes page and an HTTPRoute's pane
//! (`FRESHKUBE_ROUTES`). Each shape picks routes so one capture shows one
//! state; `All` shows every verdict at once. The names are invented. The
//! traces are not canned: the objects go through the same `trace_route` as a
//! cluster's, so the fixture proves the interpretation as well as the page.
use super::facts::*;
use super::read::{ApiState, RouteEntry, RoutesRead};
use super::topology::{WorkloadRead, WorkloadRef};
use super::trace::{BackendRead, Inputs, Listing, trace_route};
use crate::failure::{Failure, FailureKind};
use crate::resources::traffic::{Published, ServiceFacts, ServiceReading};
use k8s_openapi::api::core::v1::Service;
use k8s_openapi::api::discovery::v1::EndpointSlice;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    /// Every verdict at once, grouped.
    #[default]
    All,
    /// A healthy cluster: a flat list.
    Serving,
    /// A split with one branch down.
    Partly,
    /// A cross-namespace reference no ReferenceGrant allows.
    CrossNamespace,
    /// A listener the controller did not accept, and a host no listener serves.
    Listener,
    /// A Gateway programmed with no address.
    NoAddress,
    /// A class no controller claims, and a parent that no longer exists.
    Class,
    /// A rule with no backend and a Service that does not exist.
    NoBackend,
    /// A controller verdict older than the route.
    Stale,
    /// A route whose parent is a Service.
    Mesh,
    /// No Gateway API in the cluster.
    NotInstalled,
    /// Only some of the kinds are served.
    Incomplete,
    /// Every verdict, from a bundle installed at two versions.
    MixedBundle,
    /// The route list is refused.
    Refused,
    /// The route list stopped at its limit of 500.
    Cut,
    /// The Gateway and GatewayClass pages' example (N2b): a Gateway in each
    /// state, hostnames two Gateways both name, a class nobody claims and a
    /// Gateway whose class does not exist.
    Gateways,
    /// The Gateway API is installed and holds no Gateway, class or route.
    Empty,
    /// The Gateway list is refused while the routes are read.
    GatewaysRefused,
    /// Every verdict, with the pods behind each backend refused.
    WorkloadsRefused,
}

impl Shape {
    /// Every word the knob takes: the one list `parse` looks words up in
    /// and `docs/DEBUG_KNOBS.md`'s test compares with the doc's shape lines.
    pub const SHAPES: &'static [(&'static str, Self)] = &[
        ("all", Self::All),
        ("serving", Self::Serving),
        ("partly", Self::Partly),
        ("cross-namespace", Self::CrossNamespace),
        ("listener", Self::Listener),
        ("no-address", Self::NoAddress),
        ("class", Self::Class),
        ("no-backend", Self::NoBackend),
        ("stale", Self::Stale),
        ("mesh", Self::Mesh),
        ("not-installed", Self::NotInstalled),
        ("incomplete", Self::Incomplete),
        ("mixed-bundle", Self::MixedBundle),
        ("refused", Self::Refused),
        ("cut", Self::Cut),
        ("gateways", Self::Gateways),
        ("empty", Self::Empty),
        ("gateways-refused", Self::GatewaysRefused),
        ("workloads-refused", Self::WorkloadsRefused),
    ];

    pub fn parse(name: &str) -> Option<Self> {
        Self::SHAPES
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, shape)| *shape)
    }

    /// The routes this shape shows, by name.
    fn names(self) -> &'static [&'static str] {
        match self {
            Self::All | Self::MixedBundle | Self::Gateways | Self::WorkloadsRefused => &[
                "web",
                "https-redirect",
                "pay-status",
                "cart-split",
                "checkout",
                "status",
                "docs",
                "promo",
                "beta",
                "old-api",
                "empty",
                "ghost",
                "stale",
                "mesh-cart",
                "multi-listener",
            ],
            Self::Serving | Self::Cut | Self::GatewaysRefused => {
                &["web", "https-redirect", "pay-status"]
            }
            Self::Partly => &["cart-split", "web"],
            Self::CrossNamespace => &["checkout", "pay-status"],
            Self::Listener => &["status", "docs", "multi-listener", "web"],
            Self::NoAddress => &["promo", "web"],
            Self::Class => &["beta", "old-api", "web"],
            Self::NoBackend => &["empty", "ghost", "web"],
            Self::Stale => &["stale", "web"],
            Self::Mesh => &["mesh-cart", "web"],
            Self::NotInstalled | Self::Incomplete | Self::Refused | Self::Empty => &[],
        }
    }
}

fn cond(kind: &str, status: &str, reason: &str, generation: i64) -> Value {
    json!({"type": kind, "status": status, "reason": reason, "message": "",
        "observedGeneration": generation, "lastTransitionTime": "2026-10-20T09:00:00Z"})
}

const CONTROLLER: &str = "example.io/gateway";

fn gateway(namespace: &str, name: &str, class: &str, listeners: Value, status: Value) -> Value {
    json!({"apiVersion": "gateway.networking.k8s.io/v1", "kind": "Gateway",
        "metadata": {"name": name, "namespace": namespace, "generation": 2,
            "uid": format!("gw-{namespace}-{name}"), "resourceVersion": "1"},
        "spec": {"gatewayClassName": class, "listeners": listeners},
        "status": status})
}

fn programmed(address: bool, listener: Value) -> Value {
    let mut status = json!({
        "conditions": [cond("Accepted", "True", "Accepted", 2), cond("Programmed", "True", "Programmed", 2)],
        "listeners": [listener],
    });
    if address {
        status["addresses"] = json!([{"type": "IPAddress", "value": "203.0.113.10"}]);
    }
    status
}

fn listener_ok(name: &str) -> Value {
    json!({"name": name, "attachedRoutes": 1,
        "conditions": [cond("Accepted", "True", "Accepted", 2), cond("Programmed", "True", "Programmed", 2)]})
}

fn class(name: &str, controller: &str, accepted: Option<(&str, &str)>) -> Value {
    json!({"apiVersion": "gateway.networking.k8s.io/v1", "kind": "GatewayClass",
        "metadata": {"name": name, "uid": format!("gc-{name}"), "resourceVersion": "1", "generation": 1},
        "spec": {"controllerName": controller},
        "status": {"conditions": accepted
            .map(|(status, reason)| vec![cond("Accepted", status, reason, 1)])
            .unwrap_or_default()}})
}

fn grant() -> Value {
    json!({"apiVersion": "gateway.networking.k8s.io/v1beta1", "kind": "ReferenceGrant",
        "metadata": {"name": "allow-web-to-ledger", "namespace": "payments"},
        "spec": {"from": [{"group": "gateway.networking.k8s.io", "kind": "HTTPRoute", "namespace": "web"}],
            "to": [{"group": "", "kind": "Service", "name": "ledger"}]}})
}

struct Spec<'a> {
    namespace: &'a str,
    name: &'a str,
    parent: Value,
    hosts: &'a [&'a str],
    rules: Value,
    accepted: (&'a str, &'a str),
    resolved: (&'a str, &'a str),
    /// `(generation, observed)`.
    generation: (i64, i64),
}

impl<'a> Spec<'a> {
    fn new(namespace: &'a str, name: &'a str, host: &'a [&'a str], rules: Value) -> Self {
        Self {
            namespace,
            name,
            parent: json!({"name": "edge", "namespace": "infra", "sectionName": "https"}),
            hosts: host,
            rules,
            accepted: ("True", "Accepted"),
            resolved: ("True", "ResolvedRefs"),
            generation: (3, 3),
        }
    }

    fn parent(mut self, parent: Value) -> Self {
        self.parent = parent;
        self
    }

    fn build(self) -> Value {
        let (generation, observed) = self.generation;
        let status = if self.parent["kind"] == "Service" {
            json!({"parents": []})
        } else {
            json!({"parents": [{
                "parentRef": self.parent,
                "controllerName": CONTROLLER,
                "conditions": [
                    cond("Accepted", self.accepted.0, self.accepted.1, observed),
                    cond("ResolvedRefs", self.resolved.0, self.resolved.1, observed)],
            }]})
        };
        json!({"apiVersion": "gateway.networking.k8s.io/v1", "kind": "HTTPRoute",
            "metadata": {"name": self.name, "namespace": self.namespace, "generation": generation,
                "uid": format!("route-{}-{}", self.namespace, self.name), "resourceVersion": "1"},
            "spec": {"parentRefs": [self.parent], "hostnames": self.hosts, "rules": self.rules},
            "status": status})
    }
}

fn path(value: &str) -> Value {
    json!([{"path": {"type": "PathPrefix", "value": value}}])
}

fn route(name: &str) -> Value {
    match name {
        "web" => Spec::new("shop", "web", &["shop.example.com"],
            json!([{"matches": path("/api"), "backendRefs": [{"name": "cart", "port": 8080}]}])),
        "https-redirect" => Spec::new("shop", "https-redirect", &["shop.example.com"],
            json!([{"filters": [{"type": "RequestRedirect", "requestRedirect": {"scheme": "https", "statusCode": 301}}]}])),
        "pay-status" => Spec::new("web", "pay-status", &["pay.example.com"],
            json!([{"matches": path("/status"), "backendRefs": [{"name": "ledger", "namespace": "payments", "port": 9000}]}])),
        "cart-split" => Spec::new("shop", "cart-split", &["cart.example.com"],
            json!([{"matches": path("/"), "backendRefs": [
                {"name": "cart", "port": 8080, "weight": 80}, {"name": "canary", "port": 8080, "weight": 20}]}])),
        "checkout" => {
            let mut spec = Spec::new("shop", "checkout", &["checkout.example.com"],
                json!([{"matches": path("/"), "backendRefs": [{"name": "ledger", "namespace": "payments", "port": 9000}]}]));
            spec.resolved = ("False", "RefNotPermitted");
            spec
        }
        "status" => Spec::new("web", "status", &["status.internal.example.com"],
            json!([{"matches": path("/"), "backendRefs": [{"name": "cart", "namespace": "shop", "port": 8080}]}]))
            .parent(json!({"name": "internal", "namespace": "infra", "sectionName": "web"})),
        "docs" => {
            let mut spec = Spec::new("web", "docs", &["docs.example.org"],
                json!([{"matches": path("/"), "backendRefs": [{"name": "cart", "namespace": "shop", "port": 8080}]}]));
            spec.accepted = ("False", "NoMatchingListenerHostname");
            spec
        }
        "promo" => Spec::new("shop", "promo", &["promo.example.com"],
            json!([{"matches": path("/"), "backendRefs": [{"name": "cart", "port": 8080}]}]))
            .parent(json!({"name": "edge", "namespace": "staging", "sectionName": "https"})),
        "beta" => Spec::new("web", "beta", &["beta.example.com"],
            json!([{"matches": path("/"), "backendRefs": [{"name": "cart", "namespace": "shop", "port": 8080}]}]))
            .parent(json!({"name": "legacy-edge", "namespace": "infra", "sectionName": "https"})),
        "old-api" => Spec::new("shop", "old-api", &["old.example.com"],
            json!([{"matches": path("/"), "backendRefs": [{"name": "cart", "port": 8080}]}]))
            .parent(json!({"name": "retired", "namespace": "infra"})),
        "empty" => Spec::new("shop", "empty", &["empty.example.com"], json!([{"matches": path("/")}])),
        "ghost" => {
            let mut spec = Spec::new("shop", "ghost", &["ghost.example.com"],
                json!([{"matches": path("/"), "backendRefs": [{"name": "ghost", "port": 8080}]}]));
            spec.resolved = ("False", "BackendNotFound");
            spec
        }
        "stale" => {
            let mut spec = Spec::new("web", "stale", &["stale.example.com"],
                json!([{"matches": path("/v2"), "backendRefs": [{"name": "ledger", "namespace": "payments", "port": 9000}]}]));
            spec.generation = (5, 3);
            spec
        }
        // A Gateway with two listeners, the first of which allows only its
        // own namespace; the route names none.
        "multi-listener" => Spec::new(
            "shop",
            "multi-listener",
            &["multi.example.com"],
            json!([{"matches": path("/"), "backendRefs": [{"name": "cart", "port": 8080}]}]),
        )
        .parent(json!({"name": "shared", "namespace": "infra"})),
        "mesh-cart" => Spec::new("shop", "mesh-cart", &[],
            json!([{"matches": path("/"), "backendRefs": [{"name": "cart", "port": 8080}]}]))
            .parent(json!({"group": "", "kind": "Service", "name": "cart"})),
        other => unreachable!("no invented route {other}"),
    }
    .build()
}

fn service(
    namespace: &str,
    name: &str,
    ready: usize,
    not_ready: usize,
    port: i32,
) -> ServiceReading {
    let service: Service = serde_json::from_value(json!({
        "metadata": {"name": name, "namespace": namespace, "uid": format!("svc-{name}")},
        "spec": {"selector": {"app": name}, "ports": [{"name": "http", "port": port, "targetPort": 8080}]},
    }))
    .expect("invented Service");
    let facts = ServiceFacts::from_service(&service);
    let endpoints = (0..ready + not_ready)
        .map(|n| json!({"addresses": [format!("10.244.2.{}", n + 10)],
            "conditions": {"ready": n < ready, "serving": n < ready, "terminating": false},
            "targetRef": {"kind": "Pod", "name": format!("{name}-pod-{n}"), "uid": format!("{name}-{n}")}}))
        .collect::<Vec<_>>();
    let slices: Vec<EndpointSlice> = vec![serde_json::from_value(json!({
        "metadata": {"name": format!("{name}-x1"), "namespace": namespace,
            "labels": {"kubernetes.io/service-name": name}},
        "addressType": "IPv4", "ports": [{"name": "http", "port": 8080}], "endpoints": endpoints,
    }))
    .expect("invented slice")];
    let published = Published::from_slices(&facts, slices, false);
    ServiceReading::new(facts, Ok(published), None)
}

/// The invented Gateway API objects for a shape, as the API would return them.
pub struct World {
    pub routes: Vec<Value>,
    pub gateways: Vec<Value>,
    pub classes: Vec<Value>,
    pub grants: Vec<Value>,
}

pub fn world(shape: Shape) -> World {
    let https = json!([{"name": "https", "port": 443, "protocol": "HTTPS", "hostname": "*.example.com",
        "tls": {"mode": "Terminate", "certificateRefs": [{"name": "edge-tls"}]},
        "allowedRoutes": {"namespaces": {"from": "All"}}}]);
    // Two listeners, one the controller did not accept: partly serving. (A
    // Gateway whose only listener is rejected serves nothing and is Failed.)
    let internal = json!([
        {"name": "web", "port": 8080, "protocol": "HTTP", "hostname": "*.internal.example.com",
            "allowedRoutes": {"namespaces": {"from": "Same"}}},
        {"name": "admin", "port": 8081, "protocol": "HTTP", "hostname": "admin.internal.example.com",
            "allowedRoutes": {"namespaces": {"from": "Same"}}},
    ]);
    let mut unaccepted = programmed(true, listener_ok("admin"));
    unaccepted["listeners"] = json!([
        {"name": "web", "attachedRoutes": 0,
            "conditions": [cond("Accepted", "False", "PortUnavailable", 2)]},
        listener_ok("admin"),
    ]);
    let mut shared_status = programmed(true, listener_ok("http"));
    shared_status["listeners"] = json!([listener_ok("http"), listener_ok("https")]);
    let shared = json!([
        {"name": "http", "port": 80, "protocol": "HTTP",
            "allowedRoutes": {"namespaces": {"from": "Same"}}},
        {"name": "https", "port": 443, "protocol": "HTTPS", "hostname": "*.example.com",
            "allowedRoutes": {"namespaces": {"from": "All"}}},
    ]);
    let gateways = vec![
        gateway("infra", "shared", "example", shared, shared_status),
        gateway(
            "infra",
            "edge",
            "example",
            https.clone(),
            programmed(true, listener_ok("https")),
        ),
        gateway("infra", "internal", "example", internal, unaccepted),
        gateway(
            "staging",
            "edge",
            "example",
            https.clone(),
            programmed(false, listener_ok("https")),
        ),
        gateway(
            "infra",
            "legacy-edge",
            "legacy",
            https,
            programmed(true, listener_ok("https")),
        ),
    ];
    let mut classes = vec![
        class("example", CONTROLLER, Some(("True", "Accepted"))),
        class(
            "legacy",
            "legacy.io/gateway",
            Some(("False", "InvalidParameters")),
        ),
    ];
    let mut gateways = gateways;
    if shape == Shape::Empty {
        return World {
            routes: Vec::new(),
            gateways: Vec::new(),
            classes: Vec::new(),
            grants: Vec::new(),
        };
    }
    if shape == Shape::Gateways {
        let named = |host: &str| {
            json!([{"name": "https", "port": 443, "protocol": "HTTPS", "hostname": host,
                "tls": {"mode": "Terminate", "certificateRefs": [{"name": "shop-tls"}]},
                "allowedRoutes": {"namespaces": {"from": "Same"}}}])
        };
        let mut pending = json!({"conditions": [
            cond("Accepted", "True", "Accepted", 2), cond("Programmed", "Unknown", "Pending", 2)]});
        pending["listeners"] = json!([]);
        classes.push(class("silent", "silent.io/gateway", None));
        classes.push(class(
            "waiting",
            "waiting.io/gateway",
            Some(("Unknown", "Pending")),
        ));
        gateways.extend([
            gateway(
                "infra",
                "pending",
                "example",
                named("pending.example.com"),
                pending,
            ),
            // The controller writes no status at all.
            gateway(
                "infra",
                "silent",
                "silent",
                named("silent.example.com"),
                json!({}),
            ),
            // Two Gateways name one host.
            gateway(
                "infra",
                "blue",
                "example",
                named("shop.example.com"),
                programmed(true, listener_ok("https")),
            ),
            gateway(
                "infra",
                "green",
                "example",
                named("shop.example.com"),
                programmed(true, listener_ok("https")),
            ),
            // A class that does not exist.
            gateway(
                "infra",
                "stray",
                "gone",
                named("stray.example.com"),
                json!({}),
            ),
        ]);
    }
    World {
        routes: shape.names().iter().map(|name| route(name)).collect(),
        gateways,
        classes,
        grants: vec![grant()],
    }
}

fn refused() -> Failure {
    Failure::new(
        FailureKind::Forbidden,
        "httproutes.gateway.networking.k8s.io is forbidden for the invented viewer",
    )
}

/// What a read of the invented cluster finds under `shape`.
pub fn read(shape: Shape) -> Result<RoutesRead, Failure> {
    match shape {
        Shape::Refused => return Err(refused()),
        Shape::NotInstalled => return Ok(RoutesRead::new(ApiState::NotInstalled)),
        Shape::Incomplete => {
            return Ok(RoutesRead::new(ApiState::Incomplete {
                missing: vec!["Gateway", "GatewayClass"],
            }));
        }
        _ => {}
    }
    let world = world(shape);
    let listing = |objects: &[Value], parse: fn(&Value) -> Option<_>| {
        Listing::new(objects.iter().filter_map(parse).collect(), false)
    };
    let gateways = if shape == Shape::GatewaysRefused {
        Err(Failure::new(
            FailureKind::Forbidden,
            "gateways.gateway.networking.k8s.io is forbidden for the invented viewer",
        ))
    } else {
        Ok(Listing::new(
            world
                .gateways
                .iter()
                .filter_map(GatewayFacts::parse)
                .collect(),
            false,
        ))
    };
    let classes = Ok(listing(&world.classes, ClassFacts::parse));
    let grants = Ok(Listing::new(
        world.grants.iter().filter_map(GrantFacts::parse).collect(),
        false,
    ));
    let namespaces = Ok(Listing::new(Vec::new(), false));
    let backends: BTreeMap<(String, String), BackendRead> = [
        (
            ("shop", "cart"),
            Ok(Some(service("shop", "cart", 3, 0, 8080))),
        ),
        (
            ("shop", "canary"),
            Ok(Some(service("shop", "canary", 0, 0, 8080))),
        ),
        (
            ("payments", "ledger"),
            Ok(Some(service("payments", "ledger", 2, 1, 9000))),
        ),
        (("shop", "ghost"), Ok(None)),
    ]
    .into_iter()
    .map(|((ns, name), read)| ((ns.to_owned(), name.to_owned()), read))
    .collect();
    let inputs = Inputs {
        gateways: &gateways,
        classes: &classes,
        grants: &grants,
        namespaces: &namespaces,
        backends: &backends,
        grant_version: "v1beta1",
    };
    let routes = world
        .routes
        .iter()
        .filter_map(RouteFacts::parse)
        .map(|facts| {
            let trace = trace_route(&facts, &inputs);
            RouteEntry::new(facts, trace)
        })
        .collect();
    let api = ApiState::Ready {
        mixed: (shape == Shape::MixedBundle)
            .then(|| "HTTPRoute v1, Gateway v1beta1, GatewayClass v1".to_owned()),
    };
    let mut read = RoutesRead::new(api)
        .with_route_kind(crate::resources::ResourceKind::new(
            GATEWAY_GROUP,
            "v1",
            "HTTPRoute",
            "httproutes",
            true,
        ))
        .with_routes(routes)
        .with_gateway_kinds(
            crate::resources::ResourceKind::new(GATEWAY_GROUP, "v1", "Gateway", "gateways", true),
            crate::resources::ResourceKind::new(
                GATEWAY_GROUP,
                "v1",
                "GatewayClass",
                "gatewayclasses",
                false,
            ),
        )
        .with_gateways(gateways, classes)
        .with_services(backends);
    if shape == Shape::Cut {
        read.set_cut(super::read::ROUTE_LIMIT);
    }
    Ok(read)
}

/// The workloads behind the example's backend Services, as the Topology page
/// reads them: a Deployment for `cart`, a StatefulSet for `ledger` and no pod
/// at all for `canary`. `WorkloadsRefused` refuses every one.
pub fn workloads(shape: Shape) -> WorkloadRead {
    let refused = || {
        Failure::new(
            FailureKind::Forbidden,
            "pods is forbidden for the invented viewer",
        )
    };
    let workload = |kind: &str, namespace: &str, name: &str| WorkloadRef {
        kind: kind.to_owned(),
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        uid: format!("{kind}-{namespace}-{name}"),
    };
    let answer = |namespace: &str, name: &str, list: Vec<WorkloadRef>| {
        (
            (namespace.to_owned(), name.to_owned()),
            if shape == Shape::WorkloadsRefused {
                Err(refused())
            } else {
                Ok(list)
            },
        )
    };
    [
        answer("shop", "cart", vec![workload("Deployment", "shop", "cart")]),
        answer("shop", "canary", Vec::new()),
        answer(
            "payments",
            "ledger",
            vec![workload("StatefulSet", "payments", "ledger")],
        ),
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod parse_pins {
    use super::*;

    /// Every word the parser takes, with what it means; written down before
    /// the parser looked its words up in a table.
    #[test]
    fn parse_takes_exactly_these_words() {
        let pinned = [
            ("all", Shape::All),
            ("serving", Shape::Serving),
            ("partly", Shape::Partly),
            ("cross-namespace", Shape::CrossNamespace),
            ("listener", Shape::Listener),
            ("no-address", Shape::NoAddress),
            ("class", Shape::Class),
            ("no-backend", Shape::NoBackend),
            ("stale", Shape::Stale),
            ("mesh", Shape::Mesh),
            ("not-installed", Shape::NotInstalled),
            ("incomplete", Shape::Incomplete),
            ("mixed-bundle", Shape::MixedBundle),
            ("refused", Shape::Refused),
            ("cut", Shape::Cut),
            ("gateways", Shape::Gateways),
            ("empty", Shape::Empty),
            ("gateways-refused", Shape::GatewaysRefused),
            ("workloads-refused", Shape::WorkloadsRefused),
        ];
        for (word, shape) in pinned {
            assert!(
                matches!(Shape::parse(word), Some(found) if found == shape),
                "{word}"
            );
        }
        for unknown in ["", " ", "nope", "Firing", "all ", " all", "ALL"] {
            assert!(Shape::parse(unknown).is_none(), "{unknown:?}");
        }
    }
}
