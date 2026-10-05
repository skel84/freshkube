//! Typed, read-only node usage from metrics.k8s.io. The UI owns its cadence.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use http::{Request, header};
use kube::Client;
use serde::Deserialize;

use super::{Amounts, Failure, FailureKind, cpu_millis, quantity, table::nullable};

#[derive(Clone, Debug, PartialEq)]
pub struct NodeUsage {
    pub name: String,
    pub usage: Amounts,
    /// The metrics server's sample time, rather than the time we fetched it.
    pub sampled_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct MetricsList {
    #[serde(default, deserialize_with = "nullable")]
    items: Vec<NodeMetrics>,
}

#[derive(Deserialize)]
struct NodeMetrics {
    metadata: Metadata,
    #[serde(default)]
    timestamp: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "nullable")]
    usage: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Metadata {
    name: String,
}

/// A cluster without metrics-server returns NotFound; refused and failed reads
/// remain failures, so a caller can retain and label its last known answer.
pub async fn list_node_usage(client: &Client) -> Result<Vec<NodeUsage>, Failure> {
    let request = Request::get("/apis/metrics.k8s.io/v1beta1/nodes")
        .header(header::ACCEPT, "application/json")
        .body(Vec::new())
        .map_err(|error| Failure::new(FailureKind::Other, error.to_string()))?;
    let list: MetricsList = client.request(request).await.map_err(Failure::from_kube)?;
    Ok(list.items.into_iter().map(usage).collect())
}

fn usage(metrics: NodeMetrics) -> NodeUsage {
    let amount = |key: &str, parse: fn(&str) -> Option<f64>| {
        metrics
            .usage
            .get(key)
            .and_then(|q| parse(q))
            .filter(|v| v.is_finite() && *v >= 0.)
    };
    let usage = Amounts {
        cpu_millis: amount("cpu", cpu_millis),
        memory_bytes: amount("memory", quantity),
    };
    NodeUsage {
        name: metrics.metadata.name,
        sampled_at: metrics.timestamp,
        usage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn collection_reads_node_metrics_and_preserves_not_found() {
        use http::Response;
        use kube::client::Body;
        use std::convert::Infallible;
        for available in [true, false] {
            let service = tower::service_fn(move |request: Request<Body>| async move {
                assert_eq!(request.method(), http::Method::GET);
                assert_eq!(request.uri().path(), "/apis/metrics.k8s.io/v1beta1/nodes");
                let (status, body) = if available {
                    (
                        200,
                        json!({"items":[{"metadata":{"name":"node"},"usage":{"cpu":"250m","memory":"1Gi"}}]}),
                    )
                } else {
                    (
                        404,
                        json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"NotFound","message":"not installed","code":404}),
                    )
                };
                Ok::<_, Infallible>(
                    Response::builder()
                        .status(status)
                        .header("content-type", "application/json")
                        .body(Body::from(serde_json::to_vec(&body).unwrap()))
                        .unwrap(),
                )
            });
            let result = list_node_usage(&Client::new(service, "default")).await;
            if available {
                let rows = result.unwrap();
                assert_eq!(rows[0].usage.cpu_millis, Some(250.));
                assert_eq!(rows[0].usage.memory_bytes, Some(1024. * 1024. * 1024.));
            } else {
                assert_eq!(result.unwrap_err().kind, FailureKind::NotFound);
            }
        }
    }

    #[test]
    fn node_metrics_keep_units_sample_time_and_missing_amounts() {
        let list: MetricsList = serde_json::from_value(json!({"items":[
            {"metadata":{"name":"worker"},"timestamp":"2026-10-05T09:00:00Z","usage":{"cpu":"123456789n","memory":"512Mi"}},
            {"metadata":{"name":"missing"},"usage":{"cpu":"-1","memory":"bad"}}
        ]})).unwrap();
        let rows: Vec<_> = list.items.into_iter().map(usage).collect();
        assert_eq!(rows[0].name, "worker");
        assert!((rows[0].usage.cpu_millis.unwrap() - 123.456789).abs() < 0.000001);
        assert_eq!(rows[0].usage.memory_bytes, Some(512. * 1024. * 1024.));
        assert_eq!(
            rows[0].sampled_at.unwrap().to_rfc3339(),
            "2026-10-05T09:00:00+00:00"
        );
        assert_eq!(rows[1].usage, Amounts::default());
        assert_eq!(rows[1].sampled_at, None);
    }
}
