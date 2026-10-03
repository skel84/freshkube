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

pub const BUILTINS: &[Builtin] = &[Builtin {
    uid: "freshkube-cluster",
    title: "Cluster",
    json: include_str!("cluster.json"),
}];

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
}
