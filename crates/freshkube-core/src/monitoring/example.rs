//! Made-up Prometheus data for `--fixture` and tests. Variables go through
//! the same [`VariablePlan`] a live Prometheus answers, so the example
//! exercises the real planning; panels use grafaui's deterministic
//! [`FakeSource`].
use std::collections::BTreeMap;

use grafaui_model::{
    PanelSpec, Variable,
    data::{DataSource, FakeSource, QueryContext},
    time::TimeWindow,
};
use grafaui_prometheus::{Variables, api::ApiRequest, plan::VariablePlan, query};
use serde_json::{Value, json};

use super::transport::{expressions, panel_warnings};
use super::{DEFAULT_SCRAPE_INTERVAL, PanelResult, QueryError};

#[derive(Clone, Debug)]
pub struct ExampleSource {
    fake: FakeSource,
    /// Values each label takes, as `label/<name>/values` answers them.
    labels: BTreeMap<String, Vec<String>>,
}

impl ExampleSource {
    /// Example data seeded by `seed`, such as the dashboard's uid, with
    /// plausible values for the usual Kubernetes labels.
    pub fn new(seed: &str) -> Self {
        let values = |values: &[&str]| values.iter().map(|value| (*value).to_owned()).collect();
        let nodes: Vec<String> = values(&["cp-1", "cp-2", "cp-3", "worker-1", "worker-2"]);
        let instances = nodes
            .iter()
            .enumerate()
            .map(|(index, _)| format!("10.5.0.{}:9100", index + 2))
            .collect();
        let labels = BTreeMap::from([
            ("node".into(), nodes.clone()),
            ("nodename".into(), nodes.clone()),
            ("kubernetes_node".into(), nodes),
            ("instance".into(), instances),
            (
                "namespace".into(),
                values(&[
                    "default",
                    "kube-system",
                    "monitoring",
                    "payments",
                    "cert-manager",
                ]),
            ),
            (
                "pod".into(),
                values(&[
                    "api-7f9c6d-2xk4p",
                    "api-7f9c6d-9qd2m",
                    "checkout-5d8b7-lwz8r",
                    "coredns-85b9-4tq7n",
                    "prometheus-k8s-0",
                ]),
            ),
            (
                "container".into(),
                values(&["api", "checkout", "coredns", "prometheus"]),
            ),
            (
                "job".into(),
                values(&[
                    "apiserver",
                    "kubelet",
                    "node-exporter",
                    "kube-state-metrics",
                ]),
            ),
            ("cluster".into(), values(&["example"])),
        ]);
        Self {
            fake: FakeSource::new(seed),
            labels,
        }
    }

    /// The values `label` takes, such as the example cluster's node names.
    pub fn with_values(mut self, label: &str, values: Vec<String>) -> Self {
        self.labels.insert(label.to_owned(), values);
        self
    }

    pub fn resolve_variables(
        &self,
        definitions: &[Variable],
        selections: &[String],
        window: TimeWindow,
    ) -> Result<(Variables, Vec<String>), QueryError> {
        let mut plan = VariablePlan::new(definitions, selections, window, DEFAULT_SCRAPE_INTERVAL);
        while let Some(request) = plan.next_request().map_err(QueryError::unsupported)? {
            plan.answer(&self.answer(&request, window))
                .map_err(QueryError::unsupported)?;
        }
        Ok(plan.finish())
    }

    pub fn query_panel(
        &self,
        panel: &PanelSpec,
        context: &QueryContext,
        variables: &Variables,
    ) -> Result<PanelResult, QueryError> {
        // The expressions are what a live Prometheus would be sent, so a
        // dashboard that can't be planned fails here too.
        let requests = query::requests(panel, context, variables, DEFAULT_SCRAPE_INTERVAL)
            .map_err(QueryError::unsupported)?;
        let context = QueryContext {
            window: context.window,
            variables: variables.context_values(),
        };
        Ok(PanelResult {
            frame: self.fake.query(panel, &context),
            warnings: panel_warnings(panel),
            expressions: expressions(&requests),
        })
    }

    /// What Prometheus would answer to a variable's request.
    fn answer(&self, request: &ApiRequest, window: TimeWindow) -> Value {
        let data = if request.path == "labels" {
            json!(self.labels.keys().collect::<Vec<_>>())
        } else if request.path == "label/__name__/values" {
            json!([
                "container_cpu_usage_seconds_total",
                "container_memory_working_set_bytes",
                "kube_pod_info",
                "node_cpu_seconds_total",
                "node_memory_MemAvailable_bytes",
                "up",
            ])
        } else if let Some(label) = request
            .path
            .strip_prefix("label/")
            .and_then(|rest| rest.strip_suffix("/values"))
        {
            json!(self.values(label))
        } else {
            // query_result: one series per node, as `up` would give.
            let nodes = self.values("node");
            let instances = self.values("instance");
            let result: Vec<Value> = nodes
                .iter()
                .enumerate()
                .map(|(index, node)| {
                    json!({
                        "metric": {
                            "__name__": "up",
                            "node": node,
                            "instance": instances.get(index).cloned().unwrap_or_default(),
                            "job": "node-exporter",
                        },
                        "value": [window.end, "1"],
                    })
                })
                .collect();
            json!({"resultType": "vector", "result": result})
        };
        json!({"status": "success", "data": data})
    }

    fn values(&self, label: &str) -> Vec<String> {
        self.labels
            .get(label)
            .cloned()
            .unwrap_or_else(|| (1..=3).map(|n| format!("{label}-{n}")).collect())
    }
}
