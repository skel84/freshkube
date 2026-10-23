use super::*;
use crate::failure::{Failure, FailureKind};
use k8s_openapi::api::core::v1::{Endpoints, Pod, Service};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn service(spec: Value) -> ServiceFacts {
    let service: Service = serde_json::from_value(json!({
        "metadata": {"name": "api", "namespace": "shop", "uid": "svc-uid"},
        "spec": spec,
    }))
    .unwrap();
    ServiceFacts::from_service(&service)
}

fn web() -> ServiceFacts {
    service(json!({
        "selector": {"app": "web"},
        "ports": [{"name": "http", "port": 80, "targetPort": "http"}],
    }))
}

fn slice(name: &str, ports: Value, endpoints: Value) -> EndpointSlice {
    serde_json::from_value(json!({
        "metadata": {"name": name, "namespace": "shop", "labels": {"kubernetes.io/service-name": "api"}},
        "addressType": "IPv4",
        "ports": ports,
        "endpoints": endpoints,
    }))
    .unwrap()
}

fn endpoint(ip: &str, pod: &str, conditions: Value) -> Value {
    json!({"addresses": [ip], "conditions": conditions,
        "targetRef": {"kind": "Pod", "name": pod, "uid": format!("{pod}-uid")}})
}

fn pod(name: &str, spec: Value) -> Pod {
    let mut pod = json!({
        "metadata": {"name": name, "namespace": "shop", "uid": format!("{name}-uid"), "labels": {"app": "web"}},
        "spec": {"containers": [{"name": "app", "ports": [{"name": "http", "containerPort": 8080}]}]},
        "status": {"phase": "Running", "podIP": "10.0.0.9",
            "conditions": [{"type": "Ready", "status": "True"}]},
    });
    merge(&mut pod, spec);
    serde_json::from_value(pod).unwrap()
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

fn http() -> Value {
    json!([{"name": "http", "port": 8080, "protocol": "TCP"}])
}

#[test]
fn endpoints_group_by_their_conditions() {
    let slices = vec![slice(
        "api-a",
        http(),
        json!([
            endpoint(
                "10.0.0.1",
                "web-1",
                json!({"ready": true, "serving": true, "terminating": false})
            ),
            // Nil conditions: ready and serving read as true.
            endpoint("10.0.0.2", "web-2", json!({})),
            endpoint(
                "10.0.0.3",
                "web-3",
                json!({"ready": false, "serving": true, "terminating": true})
            ),
            endpoint(
                "10.0.0.4",
                "web-4",
                json!({"ready": false, "serving": false, "terminating": true})
            ),
            endpoint(
                "10.0.0.5",
                "web-5",
                json!({"ready": false, "serving": false, "terminating": false})
            ),
        ]),
    )];
    let published = Published::from_slices(&web(), slices, false);
    let addresses = |state| {
        published
            .group(state)
            .map(|e| e.address.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(addresses(EndpointState::Ready), ["10.0.0.1", "10.0.0.2"]);
    assert_eq!(addresses(EndpointState::Draining), ["10.0.0.3"]);
    assert_eq!(addresses(EndpointState::NotReady), ["10.0.0.4", "10.0.0.5"]);
    assert_eq!(published.source, EndpointSource::Slices);
}

#[test]
fn only_slices_labelled_for_the_service_count() {
    let mut other = slice(
        "other",
        http(),
        json!([endpoint("10.9.9.9", "x", json!({}))]),
    );
    other.metadata.labels = Some([(SERVICE_LABEL.to_owned(), "elsewhere".to_owned())].into());
    let mine = slice(
        "api-a",
        http(),
        json!([endpoint("10.0.0.1", "web-1", json!({}))]),
    );
    let published = Published::from_slices(&web(), vec![other, mine], false);
    assert_eq!(published.slices, 1);
    assert_eq!(published.endpoints.len(), 1);
}

#[test]
fn a_slice_without_a_service_port_leaves_its_endpoints_unrouted() {
    let slices = vec![slice(
        "api-a",
        json!([{"name": "metrics", "port": 9090}]),
        json!([endpoint("10.0.0.1", "web-1", json!({}))]),
    )];
    let published = Published::from_slices(&web(), slices, false);
    assert_eq!(published.count(EndpointState::Unrouted), 1);
    assert_eq!(published.count(EndpointState::Ready), 0);
    // A Service with no ports has nothing to be unrouted from.
    let headless = service(json!({"clusterIP": "None", "selector": {"app": "web"}}));
    let slices = vec![slice(
        "api-a",
        json!([]),
        json!([endpoint("10.0.0.1", "web-1", json!({}))]),
    )];
    assert_eq!(
        Published::from_slices(&headless, slices, false).count(EndpointState::Ready),
        1
    );
}

#[test]
fn legacy_endpoints_read_ready_and_not_ready_addresses() {
    let endpoints: Endpoints = serde_json::from_value(json!({
        "metadata": {"name": "api", "namespace": "shop"},
        "subsets": [{
            "addresses": [{"ip": "10.0.0.1", "targetRef": {"kind": "Pod", "name": "web-1"}}],
            "notReadyAddresses": [{"ip": "10.0.0.2"}],
            "ports": [{"name": "http", "port": 8080}],
        }],
    }))
    .unwrap();
    let published = Published::from_endpoints(&web(), &endpoints);
    assert_eq!(published.source, EndpointSource::Endpoints);
    assert_eq!(published.count(EndpointState::Ready), 1);
    assert_eq!(published.count(EndpointState::NotReady), 1);
}

#[test]
fn unpublished_pods_say_why() {
    let published = Published::from_slices(
        &web(),
        vec![slice(
            "api-a",
            http(),
            json!([endpoint("10.0.0.1", "web-ok", json!({}))]),
        )],
        false,
    );
    let pods = vec![
        pod("web-ok", json!({})),
        pod(
            "web-gone",
            json!({"metadata": {"deletionTimestamp": "2026-10-23T10:00:00Z"}}),
        ),
        pod(
            "web-cold",
            json!({"status": {"phase": "Pending", "podIP": null, "conditions": []}}),
        ),
        pod("web-lost", json!({})),
        pod(
            "web-noport",
            json!({"spec": {"containers": [{"name": "app", "ports": [{"name": "other", "containerPort": 1}]}]}}),
        ),
    ];
    let reasons = unpublished_pods(&web(), &published, &pods)
        .into_iter()
        .map(|p| (p.name, p.reason))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(reasons.len(), 4);
    assert_eq!(reasons["web-gone"], UnpublishedReason::Terminating);
    assert_eq!(reasons["web-cold"], UnpublishedReason::NotReady);
    assert_eq!(reasons["web-noport"], UnpublishedReason::NoMatchingPort);
    assert_eq!(reasons["web-lost"], UnpublishedReason::OutsideSlice);
}

#[test]
fn a_pod_published_by_ip_is_not_unpublished() {
    let published = Published::from_slices(
        &web(),
        vec![slice(
            "api-a",
            http(),
            json!([{"addresses": ["10.0.0.9"], "conditions": {}}]),
        )],
        false,
    );
    assert!(unpublished_pods(&web(), &published, &[pod("web-ip", json!({}))]).is_empty());
}

#[test]
fn a_selector_needs_every_entry_and_a_service_without_one_selects_nothing() {
    let labels = [
        ("app".to_owned(), "web".to_owned()),
        ("tier".to_owned(), "front".to_owned()),
    ]
    .into();
    assert!(web().selects(&labels));
    assert!(!service(json!({"selector": {"app": "web", "tier": "back"}})).selects(&labels));
    assert!(!service(json!({})).selects(&labels));
}

#[test]
fn shapes_read_as_what_they_are() {
    let alias = service(json!({"type": "ExternalName", "externalName": "db.example.com"}));
    assert_eq!(
        alias.shape(),
        ServiceShape::ExternalName("db.example.com".into())
    );
    assert_eq!(
        service(json!({"ports": []})).shape(),
        ServiceShape::NoSelector
    );
    assert_eq!(web().shape(), ServiceShape::Selector);
}

fn reading(facts: ServiceFacts, published: Result<Published, Failure>) -> ServiceReading {
    ServiceReading::new(facts, published, None)
}

fn published_of(endpoints: Value) -> Published {
    Published::from_slices(&web(), vec![slice("api-a", http(), endpoints)], false)
}

fn last(reading: &ServiceReading) -> TrafficStep {
    reading.steps().pop().unwrap()
}

#[test]
fn the_endpoints_step_is_good_only_when_every_endpoint_is_ready() {
    let all = published_of(json!([endpoint("10.0.0.1", "a", json!({}))]));
    assert_eq!(last(&reading(web(), Ok(all))).verdict, Verdict::Good);
    let some = published_of(json!([
        endpoint("10.0.0.1", "a", json!({})),
        endpoint("10.0.0.2", "b", json!({"ready": false, "serving": false})),
    ]));
    assert_eq!(last(&reading(web(), Ok(some))).verdict, Verdict::Warn);
    let none = published_of(json!([endpoint(
        "10.0.0.2",
        "b",
        json!({"ready": false, "serving": false})
    )]));
    assert_eq!(last(&reading(web(), Ok(none))).verdict, Verdict::Broken);
    let empty = published_of(json!([]));
    assert_eq!(last(&reading(web(), Ok(empty))).verdict, Verdict::Broken);
}

#[test]
fn a_refused_read_is_unknown_never_zero() {
    let refused = Err(Failure::new(
        FailureKind::Forbidden,
        "endpointslices is forbidden",
    ));
    let step = last(&reading(web(), refused));
    assert_eq!(step.verdict, Verdict::Unknown);
    assert!(step.summary.contains("unknown"));
    assert!(step.detail.unwrap().contains("Forbidden"));
}

#[test]
fn configuration_and_status_are_separate_steps() {
    let steps = reading(web(), Ok(published_of(json!([])))).steps();
    assert_eq!(steps[0].evidence, Evidence::Config);
    assert_eq!(steps.last().unwrap().evidence, Evidence::Status);
}

#[test]
fn a_selectorless_service_with_no_endpoints_is_not_broken() {
    let facts = service(json!({"ports": [{"name": "http", "port": 80}]}));
    let step = last(&reading(
        facts.clone(),
        Ok(Published::from_slices(&facts, vec![], false)),
    ));
    assert_eq!(step.verdict, Verdict::Neutral);
}

#[test]
fn an_external_name_service_has_no_endpoints_step() {
    let alias = service(json!({"type": "ExternalName", "externalName": "db.example.com"}));
    let steps = reading(alias, Err(Failure::new(FailureKind::Other, "unused"))).steps();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].verdict, Verdict::Neutral);
}

#[test]
fn a_load_balancer_without_an_address_is_pending() {
    let pending = service(json!({"type": "LoadBalancer", "selector": {"app": "web"}}));
    assert_eq!(pending.load_balancer, LoadBalancerState::Pending);
    let steps = reading(pending, Ok(published_of(json!([])))).steps();
    let step = steps
        .iter()
        .find(|s| s.kind == StepKind::LoadBalancer)
        .unwrap();
    assert_eq!(step.verdict, Verdict::Warn);
    let service: Service = serde_json::from_value(json!({
        "metadata": {"name": "api"},
        "spec": {"type": "LoadBalancer"},
        "status": {"loadBalancer": {"ingress": [{"ip": "203.0.113.7"}]}},
    }))
    .unwrap();
    assert_eq!(
        ServiceFacts::from_service(&service).load_balancer,
        LoadBalancerState::Addresses(vec!["203.0.113.7".into()])
    );
}

#[test]
fn a_workload_chain_follows_the_services_that_select_it() {
    let labels = [("app".to_owned(), "web".to_owned())].into();
    let readings = vec![
        reading(
            web(),
            Ok(published_of(json!([endpoint("10.0.0.1", "a", json!({}))]))),
        ),
        reading(
            service(json!({"selector": {"app": "other"}})),
            Ok(published_of(json!([]))),
        ),
    ];
    let chain = workload_chain(
        "Deployment",
        "shop",
        "web",
        &labels,
        &Ok(WorkloadServices::new(readings, 2)),
    );
    assert_eq!(chain.steps[0].kind, StepKind::Workload);
    assert_eq!(chain.steps.len(), 3);
    assert!(chain.steps[1..].iter().all(|s| s.subject.name == "api"));
    let none = workload_chain(
        "Deployment",
        "shop",
        "web",
        &BTreeMap::new(),
        &Ok(WorkloadServices::new(vec![], 0)),
    );
    assert_eq!(none.steps[0].verdict, Verdict::Neutral);
}

#[test]
fn a_refused_service_list_makes_the_workload_chain_unknown() {
    let refused = Err(Failure::new(
        FailureKind::Forbidden,
        "services is forbidden",
    ));
    let chain = workload_chain("Deployment", "shop", "web", &BTreeMap::new(), &refused);
    assert_eq!(chain.steps.len(), 1);
    assert_eq!(chain.steps[0].verdict, Verdict::Unknown);
}

#[test]
fn steps_prepend_in_front() {
    let mut chain = service_chain(&reading(web(), Ok(published_of(json!([])))));
    let first = chain.steps[0].clone();
    let mut route = first.clone();
    route.summary = "route".into();
    chain.prepend(vec![route]);
    assert_eq!(chain.steps[0].summary, "route");
    assert_eq!(chain.steps[1], first);
}

fn list(items: Vec<Value>) -> (u16, String) {
    (200, json!({"metadata": {}, "items": items}).to_string())
}

fn status(code: u16) -> (u16, String) {
    (code, json!({"kind":"Status","apiVersion":"v1","status":"Failure","code":code,"reason":"x","message":"no"}).to_string())
}

const SLICES: &str = "/apis/discovery.k8s.io/v1/namespaces/shop/endpointslices";
const SERVICES: &str = "/api/v1/namespaces/shop/services";

fn service_json() -> Value {
    json!({"metadata": {"name": "api", "namespace": "shop"}, "spec": {"selector": {"app": "web"},
        "ports": [{"name": "http", "port": 80, "targetPort": "http"}]}})
}

#[tokio::test]
async fn a_service_reads_its_slices_and_pods_and_nothing_else() {
    let (client, recorder) = crate::fake_api::client(|request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", SERVICES) => Some(list(vec![service_json()])),
            ("GET", "/api/v1/namespaces/shop/services/api") => {
                Some((200, service_json().to_string()))
            }
            ("GET", SLICES) => Some(list(vec![json!({
                "metadata": {"name": "api-a", "namespace": "shop", "labels": {"kubernetes.io/service-name": "api"}},
                "addressType": "IPv4", "ports": [{"name": "http", "port": 8080}],
                "endpoints": [{"addresses": ["10.0.0.1"], "conditions": {"ready": true}}],
            })])),
            ("GET", "/api/v1/namespaces/shop/pods") => Some(list(vec![])),
            _ => None,
        }
    });
    let reading = read_service_by_name(&client, "shop", "api").await.unwrap();
    assert_eq!(reading.published.unwrap().count(EndpointState::Ready), 1);
    assert!(reading.unpublished.unwrap().unwrap().is_empty());
    let lines = recorder.lines();
    assert!(
        lines.iter().all(|line| line.starts_with("GET ")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("labelSelector=kubernetes.io%2Fservice-name%3Dapi"))
    );
}

#[tokio::test]
async fn unserved_slices_fall_back_to_endpoints_and_say_so() {
    let (client, recorder) = crate::fake_api::client(|request| {
        match request.path.as_str() {
        SLICES => Some(status(404)),
        "/api/v1/namespaces/shop/endpoints/api" => Some((200, json!({
            "metadata": {"name": "api"},
            "subsets": [{"addresses": [{"ip": "10.0.0.1"}], "ports": [{"name": "http", "port": 8080}]}],
        }).to_string())),
        _ => None,
    }
    });
    let facts =
        service(json!({"selector": {"app": "web"}, "ports": [{"name": "http", "port": 80}]}));
    let published = read_published(&client, &facts).await.unwrap();
    assert_eq!(published.source, EndpointSource::Endpoints);
    assert_eq!(published.count(EndpointState::Ready), 1);
    only_gets(&recorder);
}

#[tokio::test]
async fn refused_slices_fall_back_to_endpoints_but_both_refused_is_an_error() {
    let (client, _) = crate::fake_api::client(|request| {
        match request.path.as_str() {
        SLICES => Some(status(403)),
        "/api/v1/namespaces/shop/endpoints/api" => Some((
            200,
            json!({"metadata": {"name": "api"},
                "subsets": [{"addresses": [{"ip": "10.0.0.1"}], "ports": [{"name": "http", "port": 8080}]}]})
            .to_string(),
        )),
        _ => None,
    }
    });
    let published = read_published(&client, &web()).await.unwrap();
    assert_eq!(published.source, EndpointSource::Endpoints);
    let (client, recorder) = crate::fake_api::client(|_| Some(status(403)));
    let error = read_published(&client, &web()).await.unwrap_err();
    // The slice refusal is what is reported, never an empty list.
    assert_eq!(error.kind, FailureKind::Forbidden);
    assert_eq!(recorder.lines().len(), 2);
}

#[tokio::test]
async fn an_empty_slice_list_is_an_answer_and_reads_no_endpoints() {
    let (client, recorder) = crate::fake_api::client(|request| match request.path.as_str() {
        SLICES => Some(list(vec![])),
        _ => None,
    });
    let published = read_published(&client, &web()).await.unwrap();
    assert_eq!(published.source, EndpointSource::Slices);
    assert!(published.endpoints.is_empty());
    assert_eq!(recorder.lines().len(), 1);
}

#[tokio::test]
async fn a_workload_reads_services_then_each_selecting_ones_slices() {
    let (client, recorder) = crate::fake_api::client(|request| match request.path.as_str() {
        SERVICES => Some(list(vec![service_json()])),
        SLICES => Some(list(vec![])),
        _ => None,
    });
    let labels = [("app".to_owned(), "web".to_owned())].into();
    let found = read_workload_services(&client, "shop", &labels)
        .await
        .unwrap();
    let readings = found.readings;
    assert_eq!(readings.len(), 1);
    assert_eq!(found.total, 1);
    // An empty slice list is an answer: Endpoints are not read.
    let lines = recorder.lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    only_gets(&recorder);
    // An empty slice list with no Endpoints object is an honest empty read.
    assert!(readings[0].published.as_ref().unwrap().endpoints.is_empty());
    let denied = crate::fake_api::client(|_| Some(status(403))).0;
    assert!(
        read_workload_services(&denied, "shop", &labels)
            .await
            .is_err()
    );
}

fn only_gets(recorder: &crate::fake_api::Recorder) {
    let lines = recorder.lines();
    assert!(
        lines.iter().all(|line| line.starts_with("GET ")),
        "{lines:?}"
    );
}

#[tokio::test]
async fn a_server_error_or_timeout_on_slices_does_not_fall_back_to_endpoints() {
    for code in [500, 504] {
        let (client, recorder) =
            crate::fake_api::client(move |request| match request.path.as_str() {
                SLICES => Some(status(code)),
                _ => None,
            });
        let error = read_published(&client, &web()).await.unwrap_err();
        assert_ne!(error.kind, FailureKind::NotFound, "{code}");
        let lines = recorder.lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("endpointslices"));
        only_gets(&recorder);
    }
}

#[tokio::test]
async fn a_cut_services_list_is_an_error_not_no_service() {
    let (client, _) = crate::fake_api::client(|request| match request.path.as_str() {
        SERVICES => Some((
            200,
            json!({"metadata": {"continue": "more"}, "items": [service_json()]}).to_string(),
        )),
        _ => None,
    });
    let labels = [("app".to_owned(), "web".to_owned())].into();
    let error = read_workload_services(&client, "shop", &labels)
        .await
        .unwrap_err();
    assert!(error.message.contains("cut at 500"));
}

#[test]
fn more_selecting_services_than_the_chain_follows_are_counted() {
    let labels = [("app".to_owned(), "web".to_owned())].into();
    let readings = (0..CHAIN_SERVICES)
        .map(|_| reading(web(), Ok(published_of(json!([])))))
        .collect();
    let chain = workload_chain(
        "Deployment",
        "shop",
        "web",
        &labels,
        &Ok(WorkloadServices::new(readings, CHAIN_SERVICES + 1)),
    );
    assert!(chain.steps[0].summary.contains("11"));
    assert_eq!(
        chain.steps[0].detail.as_deref(),
        Some("and 1 more not shown")
    );
}

#[test]
fn a_slice_port_of_another_protocol_is_unrouted() {
    let slices = vec![slice(
        "api-a",
        json!([{"name": "http", "port": 8080, "protocol": "UDP"}]),
        json!([endpoint("10.0.0.1", "web-1", json!({}))]),
    )];
    let published = Published::from_slices(&web(), slices, false);
    assert_eq!(published.count(EndpointState::Unrouted), 1);
}

#[tokio::test]
async fn a_cut_pod_list_is_marked() {
    let (client, recorder) = crate::fake_api::client(|request| match request.path.as_str() {
        SLICES => Some(list(vec![])),
        "/api/v1/namespaces/shop/pods" => Some((
            200,
            json!({"metadata": {"continue": "more"}, "items": []}).to_string(),
        )),
        _ => None,
    });
    let reading = read_service(&client, web(), true).await;
    assert!(reading.pods_cut);
    only_gets(&recorder);
}
