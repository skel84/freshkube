//! The dashboards that ship with the app, written for the Console look:
//! panels sized for its grid, the queries kube-prometheus-stack answers, and
//! thresholds by meaning rather than colour.

/// A built-in dashboard as Grafana JSON.
#[derive(Clone, Copy, Debug)]
pub struct Builtin {
    pub uid: &'static str,
    pub title: &'static str,
    pub json: &'static str,
}

pub const BUILTINS: &[Builtin] = &[
    Builtin {
        uid: "freshkube-cluster",
        title: "Cluster",
        json: include_str!("cluster.json"),
    },
    Builtin {
        uid: "freshkube-workloads",
        title: "Workloads",
        json: include_str!("workloads.json"),
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitoring::model::{Dashboard, time::TimeWindow};
    use crate::monitoring::{ExampleSource, model::data::QueryContext};

    #[test]
    fn builtins_parse_and_plan_without_warnings() {
        let window = TimeWindow::new(1_760_000_000, 6 * 3600, 240);
        for builtin in BUILTINS {
            let dashboard = Dashboard::parse(builtin.json).expect(builtin.uid);
            assert_eq!(dashboard.uid.as_deref(), Some(builtin.uid));
            assert_eq!(dashboard.title, builtin.title);
            let source = ExampleSource::new(builtin.uid);
            let (variables, _) = source
                .resolve_variables(&dashboard.variables, &[], window)
                .unwrap();
            for panel in &dashboard.panels {
                let context = QueryContext {
                    window,
                    variables: variables.context_values(),
                };
                let result = source.query_panel(panel, &context, &variables).unwrap();
                assert!(
                    result.warnings.is_empty(),
                    "{}: {:?}",
                    panel.title,
                    result.warnings
                );
                assert!(!result.expressions.is_empty(), "{}", panel.title);
                assert!(
                    panel.ignored.is_empty(),
                    "{}: {:?}",
                    panel.title,
                    panel.ignored
                );
            }
        }
    }

    fn expressions() -> Vec<String> {
        BUILTINS
            .iter()
            .flat_map(|builtin| Dashboard::parse(builtin.json).unwrap().panels)
            .flat_map(|panel| panel.queries)
            .filter_map(|query| query.request.expr)
            .collect()
    }

    /// The example data draws a series for any label a query groups by, so
    /// it can't catch a label the exporter doesn't have. node-exporter's
    /// series carry `instance`, not the node's name.
    #[test]
    fn node_exporter_series_are_named_through_their_uname() {
        let node_exporter: Vec<_> = expressions()
            .into_iter()
            .filter(|expr| expr.contains("node_cpu_") || expr.contains("node_memory_"))
            .collect();
        assert!(!node_exporter.is_empty());
        for expr in node_exporter {
            assert!(expr.contains("node_uname_info"), "{expr}");
            assert!(
                !expr.contains("by (node)") && !expr.contains("node=~"),
                "{expr}"
            );
        }
    }

    /// Another exporter, or two kube-state-metrics pods during a rollout,
    /// can report the same object twice; each is counted once.
    #[test]
    fn kube_state_metrics_count_each_object_once() {
        for expr in expressions() {
            if expr.contains("kube_") {
                assert!(expr.contains("max by ("), "{expr}");
            }
        }
    }
}
