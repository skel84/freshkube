use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};

use grafaui_model::{Dashboard, data::QueryContext, time::TimeWindow};
use grafaui_prometheus::api::ApiRequest;
use http::{Request, Response};
use k8s_openapi::api::core::v1::Service;
use kube::client::Body;
use serde_json::{Value, json};

use super::transport::scrape_interval_of;
use super::*;

/// Every request the fake API saw, as `METHOD path?query`.
type Seen = Arc<Mutex<Vec<String>>>;

/// A fake API server. `route` gets the method, path and query and returns
/// a status and body; status 0 never answers.
fn fake(
    route: impl Fn(&http::Method, &str, &str) -> (u16, String) + Send + Sync + 'static,
) -> (kube::Client, Seen) {
    let seen: Seen = Default::default();
    let log = seen.clone();
    let route = Arc::new(route);
    let service = tower::service_fn(move |request: Request<Body>| {
        let uri = request.uri().clone();
        log.lock()
            .unwrap()
            .push(format!("{} {uri}", request.method()));
        let (status, body) = route(request.method(), uri.path(), uri.query().unwrap_or(""));
        async move {
            if status == 0 {
                futures::future::pending::<()>().await;
            }
            Ok::<_, Infallible>(
                Response::builder()
                    .status(status)
                    .header("content-type", "application/json")
                    .body(Body::from(body.into_bytes()))
                    .unwrap(),
            )
        }
    });
    (kube::Client::new(service, "default"), seen)
}

fn service(namespace: &str, name: &str, labels: Value, ports: Value) -> Value {
    json!({
        "metadata": {"name": name, "namespace": namespace, "labels": labels},
        "spec": {"ports": ports},
    })
}

fn list(items: Vec<Value>) -> String {
    json!({"kind": "ServiceList", "apiVersion": "v1", "metadata": {}, "items": items}).to_string()
}

fn success(data: Value) -> String {
    json!({"status": "success", "data": data}).to_string()
}

fn status(code: u16, message: &str) -> String {
    json!({"kind": "Status", "apiVersion": "v1", "status": "Failure", "message": message, "code": code})
        .to_string()
}

fn query_one() -> String {
    success(json!({"resultType": "scalar", "result": [1700000000, "1"]}))
}

fn build_info() -> String {
    success(json!({"version": "2.53.0", "revision": "abc"}))
}

fn targets() -> String {
    success(json!({"activeTargets": [
        {"scrapeInterval": "15s"},
        {"scrapeInterval": "15s"},
        {"scrapeInterval": "1m"},
    ]}))
}

fn operated() -> Value {
    service(
        "monitoring",
        "prometheus-operated",
        json!({"operated-prometheus": "true"}),
        json!([{"name": "web", "port": 9090}]),
    )
}

fn parse(value: Value) -> Service {
    serde_json::from_value(value).unwrap()
}

#[test]
fn proxy_paths_name_scheme_service_and_port() {
    let mut service = PrometheusService::new("monitoring", "prometheus-k8s", 9090);
    assert_eq!(
        service.proxy_base(),
        "/api/v1/namespaces/monitoring/services/prometheus-k8s:9090/proxy/"
    );
    service.https = true;
    assert_eq!(
        service.proxy_base(),
        "/api/v1/namespaces/monitoring/services/https:prometheus-k8s:9090/proxy/"
    );
    assert_eq!(service.label(), "monitoring/prometheus-k8s:9090");
}

#[test]
fn ranking_prefers_the_operator_then_labels_then_names_then_ports() {
    let rank_of = |value| rank(&parse(value)).map(|candidate| candidate.rank);
    assert_eq!(rank_of(operated()), Some(Rank::Operated));
    assert_eq!(
        rank_of(service(
            "obs",
            "metrics",
            json!({"app.kubernetes.io/name": "prometheus"}),
            json!([{"name": "http", "port": 80}]),
        )),
        Some(Rank::Labelled)
    );
    assert_eq!(
        rank_of(service(
            "monitoring",
            "kube-prometheus-stack-prometheus",
            json!({}),
            json!([{"name": "http-web", "port": 9090}, {"name": "reloader-web", "port": 8080}]),
        )),
        Some(Rank::Named)
    );
    // Prometheus's companions only rank by a port, and the operator's own
    // https port isn't one.
    assert_eq!(
        rank_of(service(
            "monitoring",
            "alertmanager-operated",
            json!({}),
            json!([{"name": "web", "port": 9093}]),
        )),
        Some(Rank::Port)
    );
    assert_eq!(
        rank_of(service(
            "monitoring",
            "prometheus-operator",
            json!({}),
            json!([{"name": "https", "port": 8443}]),
        )),
        None
    );
    assert_eq!(
        rank_of(service(
            "default",
            "kubernetes",
            json!({}),
            json!([{"name": "https", "port": 443}]),
        )),
        None
    );
}

#[test]
fn the_chosen_port_and_scheme_follow_its_name() {
    let candidate = rank(&parse(service(
        "monitoring",
        "prometheus",
        json!({"app": "prometheus"}),
        json!([{"name": "grpc", "port": 10901}, {"name": "https-web", "port": 9091}, {"name": "web", "port": 9090}]),
    )))
    .unwrap();
    assert_eq!(candidate.service.port, 9090);
    assert!(!candidate.service.https);
    let candidate = rank(&parse(service(
        "monitoring",
        "prometheus",
        json!({"app": "prometheus"}),
        json!([{"name": "https", "port": 443}]),
    )))
    .unwrap();
    assert_eq!(candidate.service.port, 443);
    assert!(candidate.service.https);
}

#[tokio::test]
async fn discovery_confirms_the_operator_service_and_reads_its_scrape_interval() {
    let (client, seen) = fake(|_, path, _| match path {
        "/api/v1/services" => (
            200,
            list(vec![
                service(
                    "default",
                    "web",
                    json!({}),
                    json!([{"name": "http", "port": 80}]),
                ),
                operated(),
            ]),
        ),
        "/api/v1/namespaces/monitoring/services/prometheus-operated:9090/proxy/api/v1/query" => {
            (200, query_one())
        }
        "/api/v1/namespaces/monitoring/services/prometheus-operated:9090/proxy/api/v1/status/buildinfo" => {
            (200, build_info())
        }
        "/api/v1/namespaces/monitoring/services/prometheus-operated:9090/proxy/api/v1/targets" => {
            (200, targets())
        }
        _ => (404, status(404, "not found")),
    });
    let Discovery::Found {
        prometheus,
        build,
        tried,
    } = discover(&client, None).await.unwrap()
    else {
        panic!("Prometheus should be found");
    };
    assert_eq!(
        prometheus.service().unwrap().label(),
        "monitoring/prometheus-operated:9090"
    );
    assert_eq!(build.version.as_deref(), Some("2.53.0"));
    assert_eq!(prometheus.scrape_interval(), 15.);
    assert!(tried.is_empty());
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter().all(|request| request.starts_with("GET ")),
        "{seen:?}"
    );
    assert_eq!(seen.len(), 4, "{seen:?}");
    assert!(seen[1].ends_with("/api/v1/query?query=1"), "{seen:?}");
    assert!(
        seen[3].ends_with("/api/v1/targets?state=active"),
        "{seen:?}"
    );
}

#[tokio::test]
async fn discovery_moves_past_candidates_that_are_not_prometheus() {
    let (client, _) = fake(|_, path, _| match path {
        "/api/v1/services" => (
            200,
            list(vec![
                service(
                    "monitoring",
                    "alertmanager-operated",
                    json!({}),
                    json!([{"name": "web", "port": 9093}]),
                ),
                operated(),
                service(
                    "monitoring",
                    "prometheus-k8s",
                    json!({"app.kubernetes.io/name": "prometheus"}),
                    json!([{"name": "web", "port": 9090}]),
                ),
            ]),
        ),
        // The operator's Service has no endpoints yet.
        p if p.contains("prometheus-operated") => (
            503,
            status(
                503,
                "no endpoints available for service \"prometheus-operated\"",
            ),
        ),
        p if p.contains("prometheus-k8s:9090/proxy/api/v1/query") => (200, query_one()),
        p if p.contains("prometheus-k8s:9090/proxy/api/v1/status/buildinfo") => (200, build_info()),
        p if p.contains("prometheus-k8s") => (500, "oops".into()),
        _ => (404, "404 page not found".into()),
    });
    let Discovery::Found {
        prometheus, tried, ..
    } = discover(&client, None).await.unwrap()
    else {
        panic!("Prometheus should be found");
    };
    assert_eq!(prometheus.service().unwrap().name, "prometheus-k8s");
    // A failed targets read keeps the default interval.
    assert_eq!(prometheus.scrape_interval(), DEFAULT_SCRAPE_INTERVAL);
    assert_eq!(tried.len(), 1);
    assert_eq!(tried[0].error.kind, ErrorKind::Unavailable);
    assert!(
        tried[0].error.message.contains("no endpoints"),
        "{}",
        tried[0].error.message
    );
}

#[tokio::test]
async fn discovery_without_prometheus_lists_what_it_tried() {
    let (client, _) = fake(|_, path, _| match path {
        "/api/v1/services" => (
            200,
            list(vec![
                service(
                    "monitoring",
                    "alertmanager-operated",
                    json!({}),
                    json!([{"name": "web", "port": 9093}]),
                ),
                service(
                    "default",
                    "kubernetes",
                    json!({}),
                    json!([{"name": "https", "port": 443}]),
                ),
            ]),
        ),
        _ => (404, "404 page not found".into()),
    });
    let Discovery::Missing { candidates, tried } = discover(&client, None).await.unwrap() else {
        panic!("nothing should be found");
    };
    assert_eq!(candidates.len(), 1);
    assert_eq!(tried.len(), 1);
    assert_eq!(tried[0].error.kind, ErrorKind::NotFound);
    assert_eq!(tried[0].error.message, "Not found: 404 page not found");

    let (client, _) = fake(|_, _, _| (200, list(Vec::new())));
    assert!(matches!(
        discover(&client, None).await.unwrap(),
        Discovery::Missing { candidates, tried } if candidates.is_empty() && tried.is_empty()
    ));
}

#[tokio::test]
async fn a_refused_proxy_is_refused_not_missing() {
    let (client, seen) = fake(|_, path, _| match path {
        "/api/v1/services" => (200, list(vec![operated()])),
        _ => (
            403,
            status(
                403,
                "services \"prometheus-operated:9090\" is forbidden: User \"alice\" cannot get resource \"services/proxy\"",
            ),
        ),
    });
    let error = discover(&client, None).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::Refused);
    assert_eq!(
        error.message,
        "Not allowed to query Prometheus (services/proxy)"
    );
    assert!(error.is_permanent());
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_refused_service_list_falls_back_to_the_monitoring_namespaces() {
    let (client, seen) = fake(|_, path, _| match path {
        "/api/v1/services" => (403, status(403, "forbidden")),
        "/api/v1/namespaces/monitoring/services" => (200, list(vec![operated()])),
        p if p.ends_with("/services") => (403, status(403, "forbidden")),
        p if p.ends_with("/api/v1/query") => (200, query_one()),
        p if p.ends_with("/status/buildinfo") => (200, build_info()),
        _ => (404, status(404, "not found")),
    });
    assert!(matches!(
        discover(&client, None).await.unwrap(),
        Discovery::Found { .. }
    ));
    let listed = seen.lock().unwrap().clone();
    assert!(
        listed
            .iter()
            .any(|r| r == "GET /api/v1/namespaces/kube-system/services?"
                || r == "GET /api/v1/namespaces/kube-system/services")
    );

    let (client, _) = fake(|_, _, _| (403, status(403, "forbidden")));
    let error = discover(&client, None).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::Refused);
    assert_eq!(
        error.message,
        "Not allowed to list Services to find Prometheus"
    );
}

#[tokio::test]
async fn the_remembered_service_is_tried_before_listing() {
    let (client, seen) = fake(|_, path, _| match path {
        p if p.contains("/services/prom:9090/proxy/api/v1/query") => (200, query_one()),
        p if p.contains("/services/prom:9090/proxy/api/v1/status/buildinfo") => (200, build_info()),
        _ => (404, status(404, "not found")),
    });
    let remembered = PrometheusService::new("obs", "prom", 9090);
    let Discovery::Found { prometheus, .. } =
        discover(&client, Some(remembered.clone())).await.unwrap()
    else {
        panic!("the remembered Service should answer");
    };
    assert_eq!(prometheus.service(), Some(&remembered));
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.contains("/api/v1/services"))
    );
}

#[tokio::test(start_paused = true)]
async fn a_silent_prometheus_times_out() {
    let (client, _) = fake(|_, _, _| (0, String::new()));
    let prometheus = Prometheus::new(client, PrometheusService::new("m", "p", 9090))
        .with_timeout(Duration::from_secs(2));
    let error = prometheus.build_info().await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::TimedOut);
    assert_eq!(error.message, "Prometheus didn't answer within 2 s");
}

#[tokio::test]
async fn an_oversized_answer_is_refused_whole() {
    let (client, _) = fake(|_, _, _| (200, success(json!({"version": "x".repeat(4096)}))));
    let prometheus =
        Prometheus::new(client, PrometheusService::new("m", "p", 9090)).with_limit(1024);
    let error = prometheus.build_info().await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::BadAnswer);
    assert!(
        error.message.contains("larger than 1 MiB"),
        "{}",
        error.message
    );
}

#[tokio::test]
async fn prometheus_errors_and_bad_answers_are_told_apart() {
    let (client, _) = fake(|_, path, _| match path {
        p if p.ends_with("/query") => (
            400,
            json!({"status": "error", "errorType": "bad_data", "error": "parse error at char 4"})
                .to_string(),
        ),
        p if p.ends_with("/labels") => (200, "<html>login</html>".into()),
        _ => (
            502,
            status(
                502,
                "error trying to reach service: dial tcp 10.0.0.1:9090: connect: connection refused",
            ),
        ),
    });
    let prometheus = Prometheus::new(client, PrometheusService::new("m", "p", 9090));
    let error = prometheus
        .get(&ApiRequest::new(
            "query",
            vec![("query".into(), "up{".into())],
        ))
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Rejected);
    assert!(error.message.contains("parse error"), "{}", error.message);
    let error = prometheus
        .get(&ApiRequest::new("labels", Vec::new()))
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::BadAnswer);
    let error = prometheus
        .get(&ApiRequest::new("targets", Vec::new()))
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    // No message names the proxy path.
    assert!(!error.message.contains("/proxy/"), "{}", error.message);
}

#[tokio::test]
async fn a_query_too_long_for_a_get_is_never_sent() {
    let (client, seen) = fake(|_, _, _| (200, success(json!([]))));
    let prometheus = Prometheus::new(client, PrometheusService::new("m", "p", 9090));
    let error = prometheus
        .get(&ApiRequest::new(
            "query",
            vec![("query".into(), "a".repeat(9000))],
        ))
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unsupported);
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn the_scrape_interval_is_the_most_common_one() {
    assert_eq!(
        scrape_interval_of(&json!({"activeTargets": [
            {"scrapeInterval": "30s"}, {"scrapeInterval": "1m"}, {"scrapeInterval": "1m"}, {}
        ]})),
        Some(60.)
    );
    assert_eq!(
        scrape_interval_of(&json!({"activeTargets": [
            {"scrapeInterval": "30s"}, {"scrapeInterval": "15s"}
        ]})),
        Some(15.)
    );
    assert_eq!(scrape_interval_of(&json!({"activeTargets": []})), None);
}

const DASHBOARD: &str = r#"{
    "uid": "nodes", "title": "Nodes",
    "time": {"from": "now-1h", "to": "now"},
    "templating": {"list": [{
        "name": "node", "type": "query", "query": "label_values(up{job=\"node\"}, node)",
        "datasource": {"type": "prometheus", "uid": "prom"},
        "current": {"text": "All", "value": "$__all"}, "includeAll": true
    }]},
    "panels": [{
        "id": 1, "type": "timeseries", "title": "CPU",
        "gridPos": {"x": 0, "y": 0, "w": 12, "h": 8},
        "datasource": {"type": "prometheus", "uid": "prom"},
        "targets": [{"refId": "A", "expr": "sum by (node) (rate(cpu{node=~\"$node\"}[5m]))", "legendFormat": "{{node}}"}]
    }]
}"#;

fn window() -> TimeWindow {
    TimeWindow::new(1_700_003_600, 3600, 90)
}

#[tokio::test]
async fn a_dashboard_resolves_and_queries_over_get() {
    let (client, seen) = fake(|_, path, _| match path {
        p if p.ends_with("/label/node/values") => (200, success(json!(["cp-1", "worker-1"]))),
        p if p.ends_with("/query_range") => (
            200,
            success(json!({"resultType": "matrix", "result": [
                {"metric": {"node": "cp-1"}, "values": [[1_700_000_000, "0.5"], [1_700_000_060, "0.7"]]},
                {"metric": {"node": "worker-1"}, "values": [[1_700_000_000, "0.2"]]},
            ]})),
        ),
        _ => (404, status(404, "not found")),
    });
    let dashboard = Dashboard::parse(DASHBOARD).unwrap();
    let source = Source::Prometheus(Prometheus::new(
        client,
        PrometheusService::new("monitoring", "prometheus-operated", 9090),
    ));
    let (variables, warnings) = source
        .resolve_variables(&dashboard.variables, &[], window())
        .await
        .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(variables.options(0), ["All", "cp-1", "worker-1"]);
    let context = QueryContext {
        window: window(),
        variables: variables.context_values(),
    };
    let result = source
        .query_panel(&dashboard.panels[0], &context, &variables)
        .await
        .unwrap();
    assert_eq!(result.frame.series.len(), 2);
    assert_eq!(result.frame.series[0].name, "cp-1");
    assert_eq!(
        result.expressions,
        [(
            "A".to_owned(),
            r#"sum by (node) (rate(cpu{node=~"(cp-1|worker-1)"}[5m]))"#.to_owned()
        )]
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(
        seen.iter().all(|request| request.starts_with(
            "GET /api/v1/namespaces/monitoring/services/prometheus-operated:9090/proxy/api/v1/"
        )),
        "{seen:?}"
    );
    assert!(
        seen[0].contains("match%5B%5D=up%7Bjob%3D%22node%22%7D"),
        "{seen:?}"
    );
}

#[tokio::test]
async fn a_failed_variable_names_itself() {
    let (client, _) = fake(|_, _, _| (403, status(403, "forbidden")));
    let dashboard = Dashboard::parse(DASHBOARD).unwrap();
    let prometheus = Prometheus::new(client, PrometheusService::new("m", "p", 9090));
    let error = prometheus
        .resolve_variables(&dashboard.variables, &[], window())
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Refused);
    assert_eq!(
        error.message,
        "variable node: Not allowed to query Prometheus (services/proxy)"
    );
}

#[tokio::test]
async fn the_example_source_answers_through_the_same_plan() {
    let dashboard = Dashboard::parse(DASHBOARD).unwrap();
    let source = Source::Example(
        ExampleSource::new("nodes").with_values("node", vec!["alpha".into(), "beta".into()]),
    );
    let (variables, _) = source
        .resolve_variables(&dashboard.variables, &[], window())
        .await
        .unwrap();
    assert_eq!(variables.options(0), ["All", "alpha", "beta"]);
    let context = QueryContext {
        window: window(),
        variables: variables.context_values(),
    };
    let first = source
        .query_panel(&dashboard.panels[0], &context, &variables)
        .await
        .unwrap();
    assert!(!first.frame.series.is_empty());
    assert_eq!(first.frame.times.len(), 90);
    assert!(first.expressions[0].1.contains("(alpha|beta)"));
    // Deterministic: the same panel and window give the same numbers.
    let second = source
        .query_panel(&dashboard.panels[0], &context, &variables)
        .await
        .unwrap();
    assert_eq!(first.frame.series[0].values, second.frame.series[0].values);
}

#[tokio::test]
async fn markers_read_what_they_may_and_name_what_was_refused() {
    let at = "2023-11-14T22:00:00Z";
    let (client, seen) = fake(move |_, path, _| match path {
        "/apis/apps/v1/replicasets" => (403, status(403, "replicasets is forbidden")),
        "/api/v1/nodes" => (
            200,
            json!({"kind": "NodeList", "apiVersion": "v1", "metadata": {}, "items": [{
                "metadata": {"name": "worker-2"},
                "status": {"conditions": [
                    {"type": "Ready", "status": "False", "lastTransitionTime": at}
                ]},
            }]})
            .to_string(),
        ),
        "/api/v1/events" => (
            200,
            json!({"kind": "EventList", "apiVersion": "v1", "metadata": {}, "items": [{
                "metadata": {"name": "cp-1.1", "namespace": "default"},
                "involvedObject": {"kind": "Node", "name": "cp-1"},
                "reason": "Rebooted",
                "lastTimestamp": at,
            }]})
            .to_string(),
        ),
        _ => (404, status(404, "not found")),
    });
    let read = markers::read_markers(client, 0).await;
    let labels: Vec<_> = read.markers.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(labels, ["worker-2 NotReady", "cp-1 rebooted"]);
    assert_eq!(read.unavailable, ["Can't list replicasets: forbidden"]);
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter().all(|request| request.starts_with("GET ")),
        "{seen:?}"
    );
    assert!(
        seen.iter()
            .any(|request| request.contains("/api/v1/events?")
                && request.contains("fieldSelector=involvedObject.kind%3DNode")),
        "{seen:?}"
    );
}

#[test]
fn compatible_servers_rank_with_their_port_and_prefix() {
    let candidate = |name: &str, labels: Value, ports: Value| {
        rank(&parse(service("metrics", name, labels, ports)))
    };
    let single = candidate(
        "vmsingle-victoria",
        json!({"app.kubernetes.io/name": "vmsingle"}),
        json!([{"name": "http", "port": 8429}]),
    )
    .unwrap();
    assert_eq!(single.rank, Rank::Compatible);
    assert_eq!(single.service.label(), "metrics/vmsingle-victoria:8429");
    assert_eq!(single.service.backend(), Backend::VictoriaMetrics);

    let select = candidate(
        "vmselect-cluster",
        json!({}),
        json!([{"name": "http", "port": 8481}]),
    )
    .unwrap();
    assert_eq!(
        select.service.proxy_base(),
        "/api/v1/namespaces/metrics/services/vmselect-cluster:8481/proxy/select/0/prometheus/"
    );

    let thanos = candidate(
        "thanos-query",
        json!({}),
        json!([{"name": "grpc", "port": 10901}, {"name": "http", "port": 10902}]),
    )
    .unwrap();
    assert_eq!(thanos.service.port, 10902);
    assert_eq!(thanos.service.backend(), Backend::Thanos);

    let mimir = candidate(
        "mimir-query-frontend",
        json!({}),
        json!([{"name": "http-metrics", "port": 8080}]),
    )
    .unwrap();
    assert_eq!(
        mimir.service.label(),
        "metrics/mimir-query-frontend:8080/prometheus"
    );
    assert_eq!(mimir.service.backend(), Backend::Mimir);

    // The writers have no query API.
    assert!(
        candidate(
            "vminsert-cluster",
            json!({}),
            json!([{"name": "http", "port": 8480}])
        )
        .is_none()
    );
    // Prometheus itself still outranks them.
    assert!(Rank::Operated < Rank::Compatible && Rank::Labelled < Rank::Compatible);
}

#[test]
fn paths_and_urls_are_normalised() {
    assert_eq!(
        normalise_path("/select//0/prometheus/"),
        "select/0/prometheus"
    );
    assert_eq!(normalise_path("../a/./b"), "a/b");
    assert_eq!(
        normalise_url(" https://vm.example.com:8481/select/0/prometheus ").unwrap(),
        "https://vm.example.com:8481/select/0/prometheus/"
    );
    assert_eq!(
        normalise_url("http://prom.lan").unwrap(),
        "http://prom.lan/"
    );
    assert!(normalise_url("ftp://prom.lan").is_err());
    assert!(normalise_url("prom.lan:9090").is_err());
    assert!(normalise_url("https://user:secret@prom.lan").is_err());
    assert!(normalise_url("https://prom.lan/?token=x").is_err());
}

/// Each request's target and Authorization header.
type Heard = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// A local HTTP server for one direct-URL test: it records each request's
/// path and Authorization header and answers by path.
async fn http_server(
    route: impl Fn(&str) -> (u16, String) + Send + Sync + 'static,
) -> (String, Heard) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen: Heard = Default::default();
    let log = seen.clone();
    let route = Arc::new(route);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let log = log.clone();
            let route = route.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0; 16 * 1024];
                let mut read = 0;
                while !buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buffer[read..]).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => read += n,
                    }
                }
                let head = String::from_utf8_lossy(&buffer[..read]).into_owned();
                let target = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                let auth = head
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("authorization: ")
                            .or_else(|| line.strip_prefix("Authorization: "))
                    })
                    .map(str::to_owned);
                let (status, body) = route(&target);
                log.lock().unwrap().push((target, auth));
                let answer = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(answer.as_bytes()).await;
            });
        }
    });
    (format!("http://{address}"), seen)
}

#[tokio::test]
async fn a_url_is_confirmed_with_its_token_and_prefix() {
    let (base, seen) = http_server(|target| match target {
        "/select/0/prometheus/api/v1/query?query=1" => (200, query_one()),
        // VictoriaMetrics answers buildinfo with the version it imitates.
        "/select/0/prometheus/api/v1/status/buildinfo" => (200, build_info()),
        _ => (404, "not found".into()),
    })
    .await;
    let url = normalise_url(&format!("{base}/select/0/prometheus")).unwrap();
    let (prometheus, build) = confirm_url(url.clone(), Some(" s3cret ".into()))
        .await
        .unwrap();
    assert_eq!(prometheus.endpoint(), &Endpoint::Url(url));
    assert!(prometheus.service().is_none());
    assert_eq!(build.version.as_deref(), Some("2.53.0"));
    // No targets endpoint: the default interval stays.
    assert_eq!(prometheus.scrape_interval(), DEFAULT_SCRAPE_INTERVAL);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].0, "/select/0/prometheus/api/v1/query?query=1");
    assert!(
        seen.iter()
            .all(|(_, auth)| auth.as_deref() == Some("Bearer s3cret")),
        "{seen:?}"
    );
}

#[tokio::test]
async fn a_url_without_buildinfo_is_still_confirmed_and_errors_hide_the_token() {
    let (base, _) = http_server(|target| match target {
        "/api/v1/query?query=1" => (200, query_one()),
        _ => (404, "404 page not found".into()),
    })
    .await;
    let (_, build) = confirm_url(normalise_url(&base).unwrap(), None)
        .await
        .unwrap();
    assert_eq!(build.version, None);

    let (base, _) = http_server(|_| (401, "unauthorized".into())).await;
    let error = confirm_url(normalise_url(&base).unwrap(), Some("s3cret".into()))
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::Refused);
    assert!(!error.message.contains("s3cret"), "{}", error.message);
    assert!(!error.message.contains("127.0.0.1"), "{}", error.message);

    // Something that isn't a Prometheus API.
    let (base, _) = http_server(|_| (200, "<html></html>".into())).await;
    let error = confirm_url(normalise_url(&base).unwrap(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::BadAnswer);

    // Nothing listening.
    let error = confirm_url("http://127.0.0.1:1/".into(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    assert_eq!(error.message, "Couldn't connect to the server");
}
