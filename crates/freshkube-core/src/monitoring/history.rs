//! CPU and memory history for one pod or one node, as a two-panel
//! dashboard the Monitoring panels draw. The queries are the ones
//! kube-prometheus-stack answers: cAdvisor's container series for a pod,
//! and node-exporter's for a node, joined on `node_uname_info` so they
//! match whatever the scrape relabels the node to.
use grafaui_model::Dashboard;
use serde_json::{Value, json};

/// Whose history a dashboard shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Subject {
    Pod { namespace: String, name: String },
    Node { name: String },
}

impl Subject {
    /// A seed for example data, so each subject draws its own.
    pub fn seed(&self) -> String {
        match self {
            Self::Pod { namespace, name } => format!("history/pod/{namespace}/{name}"),
            Self::Node { name } => format!("history/node/{name}"),
        }
    }
}

/// The subject's CPU and memory panels, side by side.
pub fn dashboard(subject: &Subject) -> Dashboard {
    let panels = match subject {
        Subject::Pod { namespace, name } => {
            let selector = format!(
                "namespace=\"{}\",pod=\"{}\",container!=\"\",container!=\"POD\"",
                literal(namespace),
                literal(name)
            );
            [
                panel(
                    0,
                    "CPU",
                    &format!(
                        "sum by (container) (rate(container_cpu_usage_seconds_total{{{selector}}}[$__rate_interval]))"
                    ),
                    "{{container}}",
                    json!({"unit": "short", "min": 0}),
                ),
                panel(
                    12,
                    "Memory",
                    &format!(
                        "sum by (container) (container_memory_working_set_bytes{{{selector}}})"
                    ),
                    "{{container}}",
                    json!({"unit": "bytes", "min": 0}),
                ),
            ]
        }
        Subject::Node { name } => {
            let node = format!(
                "on (instance) group_left (nodename) node_uname_info{{nodename=\"{}\"}}",
                literal(name)
            );
            let percent = json!({
                "unit": "percentunit",
                "min": 0,
                "max": 1,
                "custom": {"thresholdsStyle": {"mode": "line"}},
                "thresholds": {"mode": "absolute", "steps": [
                    {"color": "green", "value": null},
                    {"color": "red", "value": 0.9},
                ]},
            });
            [
                panel(
                    0,
                    "CPU",
                    &format!(
                        "1 - avg by (nodename) (rate(node_cpu_seconds_total{{mode=\"idle\"}}[$__rate_interval]) * {node})"
                    ),
                    "used",
                    percent.clone(),
                ),
                panel(
                    12,
                    "Memory",
                    &format!(
                        "max by (nodename) ((1 - node_memory_MemAvailable_bytes / node_memory_MemTotal_bytes) * {node})"
                    ),
                    "used",
                    percent,
                ),
            ]
        }
    };
    let json = json!({
        "uid": subject.seed(),
        "title": "History",
        "time": {"from": "now-1h", "to": "now"},
        "panels": panels,
    });
    Dashboard::parse(&json.to_string()).expect("the history dashboard parses")
}

fn panel(x: u32, title: &str, expr: &str, legend: &str, defaults: Value) -> Value {
    let mut defaults = defaults;
    defaults["custom"]["lineWidth"] = json!(2);
    defaults["custom"]["fillOpacity"] = json!(0);
    json!({
        "id": x + 1,
        "type": "timeseries",
        "title": title,
        "gridPos": {"x": x, "y": 0, "w": 12, "h": 6},
        "targets": [{"refId": "A", "expr": expr, "legendFormat": legend}],
        "fieldConfig": {"defaults": defaults, "overrides": []},
        "options": {
            "legend": {"displayMode": "list", "placement": "bottom", "calcs": ["lastNotNull"]},
            "tooltip": {"mode": "multi"},
        },
    })
}

/// `text` as the inside of a PromQL double-quoted string.
fn literal(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitoring::model::{data::QueryContext, time::TimeWindow};
    use crate::monitoring::{ExampleSource, prometheus::Variables};

    fn expressions(subject: &Subject) -> Vec<String> {
        let dashboard = dashboard(subject);
        let window = TimeWindow::new(1_760_000_000, 3600, 120);
        let source = ExampleSource::new(&subject.seed());
        dashboard
            .panels
            .iter()
            .map(|panel| {
                assert!(panel.ignored.is_empty(), "{:?}", panel.ignored);
                let context = QueryContext {
                    window,
                    variables: Default::default(),
                };
                let result = source
                    .query_panel(panel, &context, &Variables::default())
                    .unwrap();
                assert!(result.warnings.is_empty(), "{:?}", result.warnings);
                assert!(!result.frame.series.is_empty());
                result.expressions[0].1.clone()
            })
            .collect()
    }

    #[test]
    fn a_pods_history_reads_its_containers() {
        let pod = Subject::Pod {
            namespace: "payments".into(),
            name: "api-7f9c6d-2xk4p".into(),
        };
        let [cpu, memory] = expressions(&pod).try_into().unwrap();
        assert!(cpu.starts_with("sum by (container) (rate(container_cpu_usage_seconds_total{namespace=\"payments\",pod=\"api-7f9c6d-2xk4p\",container!=\"\",container!=\"POD\"}["), "{cpu}");
        assert!(!cpu.contains("$__rate_interval"), "{cpu}");
        assert_eq!(
            memory,
            "sum by (container) (container_memory_working_set_bytes{namespace=\"payments\",pod=\"api-7f9c6d-2xk4p\",container!=\"\",container!=\"POD\"})"
        );
    }

    #[test]
    fn a_nodes_history_joins_on_its_uname() {
        let node = Subject::Node {
            name: "worker-1".into(),
        };
        let [cpu, memory] = expressions(&node).try_into().unwrap();
        assert!(
            cpu.contains("node_uname_info{nodename=\"worker-1\"}"),
            "{cpu}"
        );
        assert!(
            memory.contains("node_memory_MemAvailable_bytes"),
            "{memory}"
        );
    }

    #[test]
    fn names_are_quoted_as_literals() {
        let pod = Subject::Pod {
            namespace: "a\"b".into(),
            name: "c\\d".into(),
        };
        let [cpu, _] = expressions(&pod).try_into().unwrap();
        assert!(cpu.contains(r#"namespace="a\"b",pod="c\\d""#), "{cpu}");
    }
}
