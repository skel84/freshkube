//! The Gateway and GatewayClass pages' interpretation and the topology, on
//! the invented cluster (`example`), so the same objects the pages draw are
//! the ones judged here.
use super::example::{self, Shape};
use super::*;
use crate::failure::{Failure, FailureKind};
use serde_json::{Value, json};

fn read(shape: Shape) -> RoutesRead {
    example::read(shape).expect("an invented read")
}

fn states(view: &GatewaysView) -> Vec<(String, GatewayState)> {
    view.gateways
        .as_ref()
        .unwrap()
        .iter()
        .map(|g| (g.key(), g.state))
        .collect()
}

fn state_of(view: &GatewaysView, key: &str) -> GatewayState {
    states(view)
        .into_iter()
        .find(|(k, _)| k == key)
        .unwrap_or_else(|| panic!("no Gateway {key}: {:?}", states(view)))
        .1
}

fn entry<'a>(view: &'a GatewaysView, key: &str) -> &'a GatewayEntry {
    view.gateways
        .as_ref()
        .unwrap()
        .iter()
        .find(|g| g.key() == key)
        .unwrap_or_else(|| panic!("no Gateway {key}"))
}

fn forbidden() -> Failure {
    Failure::new(FailureKind::Forbidden, "refused")
}

#[test]
fn every_gateway_state_reads_as_the_controller_says() {
    let view = GatewaysView::new(&read(Shape::Gateways));
    assert_eq!(state_of(&view, "infra/edge"), GatewayState::Serving);
    assert_eq!(state_of(&view, "infra/shared"), GatewayState::Serving);
    // Programmed, but a listener the controller did not accept.
    assert_eq!(state_of(&view, "infra/internal"), GatewayState::Partly);
    // Programmed with no address is blind, not failed and not serving.
    assert_eq!(state_of(&view, "staging/edge"), GatewayState::Blind);
    // The class was refused.
    assert_eq!(state_of(&view, "infra/legacy-edge"), GatewayState::Failed);
    assert_eq!(state_of(&view, "infra/pending"), GatewayState::Pending);
    // No status on the Gateway or its class: blind.
    assert_eq!(state_of(&view, "infra/silent"), GatewayState::Blind);
    // A class that does not exist is the platform's break.
    let stray = entry(&view, "infra/stray");
    assert_eq!(stray.state, GatewayState::Failed);
    assert_eq!(stray.owner, Some(Owner::Platform));
    assert!(stray.reason.contains("gone"), "{}", stray.reason);
    // Routes name a parent that no Gateway answers to.
    let retired = entry(&view, "infra/retired");
    assert_eq!(retired.state, GatewayState::Missing);
    assert_eq!(retired.routes.len(), 1);
    assert_eq!(retired.owner, Some(Owner::Yours));
}

#[test]
fn unknown_is_never_serving() {
    let view = GatewaysView::new(&read(Shape::Gateways));
    for gateway in view.gateways.as_ref().unwrap() {
        if gateway.state == GatewayState::Serving {
            assert!(gateway.address().is_some(), "{}", gateway.key());
            assert!(!gateway.facts.conditions.is_empty(), "{}", gateway.key());
        }
    }
    // A refused class list leaves every Gateway Unknown, with the reason.
    let mut refused = read(Shape::Gateways);
    refused.classes = Err(forbidden());
    let view = GatewaysView::new(&refused);
    for gateway in view.gateways.as_ref().unwrap() {
        if gateway.state != GatewayState::Missing {
            assert_eq!(gateway.state, GatewayState::Unknown, "{}", gateway.key());
            assert!(gateway.reason.contains("unreadable"), "{}", gateway.reason);
        }
    }
    assert!(view.classes.is_err());
}

#[test]
fn a_refused_gateway_list_is_an_error_and_not_an_empty_page() {
    let view = GatewaysView::new(&read(Shape::GatewaysRefused));
    assert_eq!(
        view.gateways.as_ref().unwrap_err().kind,
        FailureKind::Forbidden
    );
    // The classes were read, so their page still answers.
    assert!(view.classes.is_ok());
    // The routes were read against nothing, so they are Unknown, never broken.
    let read = read(Shape::GatewaysRefused);
    assert!(
        read.routes
            .iter()
            .all(|r| r.trace.verdict != RouteVerdict::NotServing),
        "{:?}",
        read.routes
            .iter()
            .map(|r| r.trace.verdict)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_cut_gateway_list_cannot_say_a_parent_is_missing() {
    let mut cut = read(Shape::Gateways);
    cut.gateways = cut.gateways.map(|mut list| {
        list.cut = true;
        list
    });
    let view = GatewaysView::new(&cut);
    assert!(view.gateways_cut);
    assert!(
        !states(&view)
            .iter()
            .any(|(_, s)| *s == GatewayState::Missing),
        "{:?}",
        states(&view)
    );
}

#[test]
fn a_stale_verdict_is_a_warning_on_a_serving_gateway() {
    let mut read = read(Shape::Serving);
    let stale = GatewayFacts::parse(&json!({
        "metadata": {"name": "old", "namespace": "infra", "generation": 9},
        "spec": {"gatewayClassName": "example", "listeners": []},
        "status": {"conditions": [
            {"type": "Accepted", "status": "True", "observedGeneration": 9},
            {"type": "Programmed", "status": "True", "observedGeneration": 4}],
            "addresses": [{"value": "203.0.113.20"}]},
    }))
    .unwrap();
    read.gateways.as_mut().unwrap().items.push(stale);
    let view = GatewaysView::new(&read);
    let old = entry(&view, "infra/old");
    assert_eq!(old.state, GatewayState::Serving);
    assert!(old.stale);
    assert!(old.reason.contains("stale"), "{}", old.reason);
}

#[test]
fn listeners_keep_configuration_and_the_controllers_report_apart() {
    let view = GatewaysView::new(&read(Shape::Gateways));
    let internal = entry(&view, "infra/internal");
    let web = &internal.listeners[0];
    assert_eq!((web.port, web.protocol.as_str()), (8080, "HTTP"));
    assert_eq!(web.state, ListenerState::Failed);
    assert!(web.says.as_ref().unwrap().contains("PortUnavailable"));
    assert!(web.allows.contains("own namespace"));
    assert_eq!(internal.failing_listeners(), 1);
    let edge = entry(&view, "infra/edge");
    let https = &edge.listeners[0];
    assert_eq!(https.state, ListenerState::Ready);
    // Certificates are named and no Secret is read.
    assert_eq!(https.certificates, vec!["edge-tls".to_owned()]);
    assert_eq!(https.attached, Some(1));
    // A Gateway the controller writes nothing for has listeners with no status.
    let silent = entry(&view, "infra/silent");
    assert_eq!(silent.listeners[0].state, ListenerState::Unknown);
    assert_eq!(silent.listeners[0].attached, None);
}

#[test]
fn routes_attach_with_the_trace_verdict_and_the_worst_one_wins() {
    let view = GatewaysView::new(&read(Shape::All));
    let edge = entry(&view, "infra/edge");
    assert!(edge.routes.len() >= 4, "{:?}", edge.routes);
    // Worst first.
    let verdicts = edge.routes.iter().map(|r| r.verdict).collect::<Vec<_>>();
    assert!(verdicts.is_sorted(), "{verdicts:?}");
    assert!(edge.not_serving() > 0);
    assert!(
        edge.routes_line().contains("not serving"),
        "{}",
        edge.routes_line()
    );
    // A route that names no listener attaches to a Gateway with several.
    let shared = entry(&view, "infra/shared");
    assert!(shared.routes.iter().any(|r| r.name == "multi-listener"));
    // Mesh routes have no Gateway.
    assert!(
        view.gateways
            .as_ref()
            .unwrap()
            .iter()
            .all(|g| g.routes.iter().all(|r| r.name != "mesh-cart"))
    );
}

#[test]
fn a_host_two_gateways_name_is_contested_and_a_wildcard_is_not() {
    let view = GatewaysView::new(&read(Shape::Gateways));
    let blue = entry(&view, "infra/blue");
    assert_eq!(
        blue.contested,
        vec![Contested {
            host: "shop.example.com".into(),
            also: vec!["infra/green".into()],
        }]
    );
    assert_eq!(
        entry(&view, "infra/green").contested[0].also,
        vec!["infra/blue".to_owned()]
    );
    // `*.example.com` is on several Gateways and is not called contested.
    assert!(entry(&view, "infra/edge").contested.is_empty());
}

#[test]
fn classes_say_who_claimed_them_and_which_gateways_use_them() {
    let view = GatewaysView::new(&read(Shape::Gateways));
    let classes = view.classes.as_ref().unwrap();
    let class = |name: &str| {
        classes
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no class {name}"))
    };
    assert_eq!(class("example").state, ClassState::Claimed);
    assert!(
        class("example")
            .gateways
            .iter()
            .any(|(k, _)| k == "infra/edge")
    );
    assert_eq!(class("legacy").state, ClassState::Refused);
    assert_eq!(
        class("legacy").gateways,
        vec![("infra/legacy-edge".to_owned(), GatewayState::Failed)]
    );
    assert_eq!(class("silent").state, ClassState::Blind);
    assert_eq!(class("waiting").state, ClassState::Pending);
    assert!(class("waiting").gateways.is_empty());
    // Gateways name a class the cluster lacks.
    let gone = class("gone");
    assert_eq!(gone.state, ClassState::Missing);
    assert_eq!(gone.controller, None);
    assert_eq!(gone.gateways[0].0, "infra/stray");
    // Refused first, claimed last.
    let order = classes.iter().map(|c| c.state).collect::<Vec<_>>();
    assert!(order.is_sorted(), "{order:?}");
}

#[test]
fn a_cut_class_list_cannot_say_a_class_is_missing() {
    let mut cut = read(Shape::Gateways);
    cut.classes = cut.classes.map(|mut list| {
        list.cut = true;
        list
    });
    let view = GatewaysView::new(&cut);
    assert!(view.classes_cut);
    assert!(
        !view
            .classes
            .unwrap()
            .iter()
            .any(|c| c.state == ClassState::Missing)
    );
}

#[test]
fn an_empty_cluster_has_nothing_and_says_so_without_failing() {
    let view = GatewaysView::new(&read(Shape::Empty));
    assert!(view.gateways.unwrap().is_empty());
    assert!(view.classes.unwrap().is_empty());
}

fn topology(shape: Shape) -> Topology {
    let read = read(shape);
    Topology::new(&read, &example::workloads(shape)).unwrap()
}

fn kinds(topology: &Topology, kind: NodeKind) -> Vec<&Node> {
    topology.nodes.iter().filter(|n| n.kind == kind).collect()
}

#[test]
fn the_topology_runs_from_address_to_workload() {
    let topology = topology(Shape::All);
    let edge = topology.node("gateway:infra/edge").unwrap();
    assert_eq!(edge.health, Health::Good);
    assert!(topology.node("address:203.0.113.10").is_some());
    let has = |from: &str, to: &str| topology.edges.iter().any(|e| e.from == from && e.to == to);
    assert!(has("address:203.0.113.10", "gateway:infra/edge"));
    assert!(has("gateway:infra/edge", "route:shop/web"));
    assert!(has("route:shop/web", "service:shop/cart"));
    assert!(has("service:shop/cart", "workload:Deployment/shop/cart"));
    assert!(has(
        "service:payments/ledger",
        "workload:StatefulSet/payments/ledger"
    ));
    // Boxes appear once however many routes reach them.
    let keys = topology
        .nodes
        .iter()
        .map(|n| &n.key)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(keys.len(), topology.nodes.len());
    // Every call joins two boxes.
    for call in &topology.edges {
        assert!(topology.node(&call.from).is_some(), "{}", call.from);
        assert!(topology.node(&call.to).is_some(), "{}", call.to);
    }
}

#[test]
fn a_call_reads_as_the_trace_does() {
    let topology = topology(Shape::All);
    let health = |from: &str, to: &str| {
        topology
            .edges
            .iter()
            .find(|e| e.from == from && e.to == to)
            .unwrap_or_else(|| panic!("no call {from} -> {to}"))
            .health
    };
    // checkout is refused across namespaces; cart-split is partly serving.
    assert_eq!(
        health("gateway:infra/edge", "route:shop/checkout"),
        Health::Bad
    );
    assert_eq!(health("gateway:infra/edge", "route:shop/web"), Health::Good);
    // The split: its shares are on the calls and the broken branch is drawn.
    let split = topology
        .edges
        .iter()
        .filter(|e| e.from == "route:shop/cart-split")
        .map(|e| (e.to.as_str(), e.label.as_str(), e.health))
        .collect::<Vec<_>>();
    assert!(
        split.contains(&("service:shop/cart", "80%", Health::Good)),
        "{split:?}"
    );
    assert!(
        split.contains(&("service:shop/canary", "20%", Health::Bad)),
        "{split:?}"
    );
    // A Service that does not exist is a ghost with no workloads.
    let ghost = topology.node("service:shop/ghost").unwrap();
    assert!(ghost.ghost);
    assert!(
        !topology
            .edges
            .iter()
            .any(|e| e.from == "service:shop/ghost")
    );
}

#[test]
fn a_gateway_with_no_address_draws_a_ghost_address() {
    let topology = topology(Shape::Gateways);
    let none = topology.node("address:none:staging/edge").unwrap();
    assert!(none.ghost);
    assert_eq!(none.health, Health::Unknown);
    // A parent no Gateway answers to is a ghost Gateway with no address.
    let retired = topology.node("gateway:infra/retired").unwrap();
    assert!(retired.ghost);
    assert!(retired.target.is_none());
    assert!(
        !topology
            .edges
            .iter()
            .any(|e| e.to == "gateway:infra/retired" && e.from.starts_with("address"))
    );
}

#[test]
fn workloads_that_could_not_be_read_are_a_dashed_unknown_and_never_an_empty_column() {
    let topology = topology(Shape::WorkloadsRefused);
    let ghost = topology.node("workloads:shop/cart").unwrap();
    assert!(ghost.ghost);
    assert_eq!(ghost.health, Health::Unknown);
    assert!(ghost.words.contains("forbidden") || ghost.words.contains("could not be read"));
    assert!(kinds(&topology, NodeKind::Workload).iter().all(|n| n.ghost));
    // A Service that publishes nothing is its own ghost, not "unknown".
    let normal = super::example::workloads(Shape::All);
    let mut read = read(Shape::All);
    let topology = Topology::new(&read, &normal).unwrap();
    let none = topology.node("workloads:shop/canary").unwrap();
    assert!(none.label.contains("no pods"));
    // A Service left out of the read says it was not read.
    read.routes_cut = false;
    let topology = Topology::new(&read, &WorkloadRead::new()).unwrap();
    assert!(
        topology
            .node("workloads:shop/cart")
            .unwrap()
            .label
            .contains("not read")
    );
}

#[test]
fn the_topology_notes_what_it_leaves_out() {
    let topology = topology(Shape::All);
    assert!(
        topology.notes.iter().any(|n| n.contains("mesh route")),
        "{:?}",
        topology.notes
    );
    let mut cut = read(Shape::All);
    cut.routes_cut = true;
    cut.backends_cut = 3;
    let topology = Topology::new(&cut, &WorkloadRead::new()).unwrap();
    assert!(topology.notes.iter().any(|n| n.contains("Routes were cut")));
    assert!(
        topology
            .notes
            .iter()
            .any(|n| n.contains("3 backend Services"))
    );
}

#[test]
fn a_refused_gateway_list_is_no_topology() {
    let read = read(Shape::GatewaysRefused);
    let error = Topology::new(&read, &WorkloadRead::new()).unwrap_err();
    assert_eq!(error.kind, FailureKind::Forbidden);
}

fn pod(name: &str, owner: Option<(&str, &str)>) -> Value {
    let mut pod = json!({"metadata": {"name": name, "namespace": "shop"}});
    if let Some((kind, owner)) = owner {
        pod["metadata"]["ownerReferences"] =
            json!([{"kind": kind, "name": owner, "controller": true}]);
    }
    pod
}

#[test]
fn pods_resolve_to_the_workload_that_controls_them() {
    let pods = [
        pod("cart-7d9-a", Some(("ReplicaSet", "cart-7d9"))),
        pod("cart-7d9-b", Some(("ReplicaSet", "cart-7d9"))),
        pod("ledger-0", Some(("StatefulSet", "ledger"))),
        pod("loose", None),
        pod("orphan-rs-x", Some(("ReplicaSet", "orphan-rs"))),
    ];
    let sets = [
        pod("cart-7d9", Some(("Deployment", "cart"))),
        pod("orphan-rs", None),
    ];
    let names = [
        "cart-7d9-a",
        "cart-7d9-b",
        "ledger-0",
        "loose",
        "orphan-rs-x",
        "unlisted",
    ]
    .map(String::from);
    let found = resolve_workloads("shop", &names, &pods, &sets);
    let words = found
        .iter()
        .map(|w| format!("{}/{}", w.kind, w.name))
        .collect::<Vec<_>>();
    assert_eq!(
        words,
        [
            "Deployment/cart",
            "Pod/loose",
            // A ReplicaSet nothing owns stays a ReplicaSet.
            "Pod/unlisted",
            "ReplicaSet/orphan-rs",
            "StatefulSet/ledger",
        ]
    );
}

fn list(items: Vec<Value>) -> (u16, String) {
    (200, json!({"metadata": {}, "items": items}).to_string())
}

#[tokio::test]
async fn reading_workloads_lists_pods_and_replicasets_per_namespace_and_never_writes() {
    let read = read(Shape::All);
    let (client, recorder) = crate::fake_api::client(|request| match request.path.as_str() {
        "/api/v1/namespaces/shop/pods" => Some(list(
            (0..3)
                .map(|n| pod(&format!("cart-pod-{n}"), Some(("ReplicaSet", "cart-abc"))))
                .collect(),
        )),
        "/apis/apps/v1/namespaces/shop/replicasets" => {
            Some(list(vec![pod("cart-abc", Some(("Deployment", "cart")))]))
        }
        "/api/v1/namespaces/payments/pods" => Some((
            403,
            json!({
            "kind": "Status", "status": "Failure", "reason": "Forbidden", "code": 403,
            "message": "pods is forbidden"})
            .to_string(),
        )),
        "/apis/apps/v1/namespaces/payments/replicasets" => Some(list(vec![])),
        _ => None,
    });
    let workloads = read_workloads(&client, &read).await;
    let cart = workloads[&("shop".to_owned(), "cart".to_owned())]
        .as_ref()
        .unwrap();
    assert_eq!(cart.len(), 1);
    assert_eq!(
        (cart[0].kind.as_str(), cart[0].name.as_str()),
        ("Deployment", "cart")
    );
    // A refused namespace is an error for its Services, never an empty list.
    let ledger = &workloads[&("payments".to_owned(), "ledger".to_owned())];
    assert_eq!(ledger.as_ref().unwrap_err().kind, FailureKind::Forbidden);
    recorder.assert_all("the topology only reads", |r| r.method == http::Method::GET);
    let lines = recorder.lines();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("/namespaces/shop/pods?limit=500")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("/namespaces/shop/replicasets?limit=500"))
    );
    // One pod list per namespace, not one per Service.
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.contains("/namespaces/shop/pods"))
            .count(),
        1
    );
    assert!(!lines.iter().any(|l| l.contains("secrets")), "{lines:?}");
}

#[tokio::test]
async fn namespaces_past_the_cap_read_unknown_and_are_not_listed() {
    let mut read = RoutesRead::new(ApiState::Ready { mixed: None });
    let services = (0..WORKLOAD_NAMESPACES + 2)
        .map(|n| {
            let namespace = format!("ns{n:02}");
            let reading = example::read(Shape::All)
                .unwrap()
                .services
                .remove(&("shop".to_owned(), "cart".to_owned()))
                .unwrap();
            ((namespace, "cart".to_owned()), reading)
        })
        .collect();
    read = read.with_services(services);
    let (client, recorder) = crate::fake_api::client(|_| Some(list(vec![])));
    let workloads = read_workloads(&client, &read).await;
    assert_eq!(workloads.len(), WORKLOAD_NAMESPACES + 2);
    let refused = workloads.values().filter(|w| w.is_err()).count();
    assert_eq!(refused, 2);
    let pod_lists = recorder
        .lines()
        .iter()
        .filter(|l| l.contains("/pods?"))
        .count();
    assert_eq!(pod_lists, WORKLOAD_NAMESPACES);
}
