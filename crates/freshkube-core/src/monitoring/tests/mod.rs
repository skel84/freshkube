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

use super::transport::scrape_interval as scrape_interval_of;
use super::*;
use crate::failure::{Failure, FailureKind};

mod access;
mod dashboard;
mod direct_url;
mod discovery;
mod marker_reads;
mod ranking;
mod transport;

/// Every request the fake API saw, as `METHOD path?query`.
pub(super) type Seen = Arc<Mutex<Vec<String>>>;

/// A fake API server. `route` gets the method, path and query and returns
/// a status and body; status 0 never answers.
pub(super) fn fake(
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

pub(super) fn success(data: Value) -> String {
    json!({"status": "success", "data": data}).to_string()
}

pub(super) fn status(code: u16, message: &str) -> String {
    json!({"kind": "Status", "apiVersion": "v1", "status": "Failure", "message": message, "code": code})
        .to_string()
}

pub(super) fn query_one() -> String {
    success(json!({"resultType": "scalar", "result": [1700000000, "1"]}))
}

pub(super) fn build_info() -> String {
    success(json!({"version": "2.53.0", "revision": "abc"}))
}

pub(super) fn targets() -> String {
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
