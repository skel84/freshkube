//! A chart from Coroot's application view as a Monitoring panel: a
//! one-panel Grafana timeseries, the answer it shows and its window, so the
//! desktop draws it with the same chart as Monitoring.
use super::app_view::{Chart, Series};
use crate::monitoring::PanelResult;
use crate::monitoring::markers::{Marker, MarkerKind};
use grafaui_model::{
    Dashboard, PanelSpec,
    data::{Frame, Series as FrameSeries},
    time::TimeWindow,
};
use serde_json::{Value, json};

/// The icon Coroot gives an annotation for a rollout.
const DEPLOY_ICON: &str = "mdi-swap-horizontal-circle-outline";
/// The opacity of an area Coroot fills, as Grafana's percentage.
const FILL_OPACITY: u32 = 20;

/// A Coroot chart, ready for a Monitoring panel.
#[derive(Clone, Debug)]
pub struct ChartPanel {
    pub spec: PanelSpec,
    pub result: PanelResult,
    pub window: TimeWindow,
    /// Coroot's deployment annotations. Incidents and other events have no
    /// marker kind yet, so they are left out.
    pub markers: Vec<Marker>,
}

/// The series' name as its legend shows it.
fn label(series: &Series) -> &str {
    if series.title.is_empty() {
        &series.name
    } else {
        &series.title
    }
}

/// Coroot names a chart's unit after its title's last comma, as in
/// `Network round-trip time, seconds`; the Grafana unit that formats it,
/// or none for a plain number. Seconds per second, such as CPU delay, read
/// as the seconds lost in each second.
fn unit(title: &str) -> Option<&'static str> {
    let (_, suffix) = title.rsplit_once(", ")?;
    Some(match suffix.trim() {
        "seconds" | "seconds/second" => "s",
        "bytes" => "bytes",
        "bytes/second" => "Bps",
        "percent" | "%" => "percent",
        _ => return None,
    })
}

/// The Grafana panel Coroot's chart describes.
fn panel_json(chart: &Chart) -> Value {
    let mut overrides: Vec<Value> = chart
        .series
        .iter()
        .filter(|s| s.fill && !chart.stacked)
        .map(|s| {
            json!({
                "matcher": {"id": "byName", "options": label(s)},
                "properties": [{"id": "custom.fillOpacity", "value": FILL_OPACITY}],
            })
        })
        .collect();
    // A limit is a dashed line; a total Coroot fills, such as all requests
    // behind the failed ones, is a quiet area.
    if let Some(threshold) = &chart.threshold {
        let mut properties = vec![
            json!({"id": "custom.stacking", "value": {"mode": "none"}}),
            json!({"id": "custom.fillOpacity", "value": if threshold.fill { FILL_OPACITY / 2 } else { 0 }}),
        ];
        if !threshold.fill {
            properties.push(
                json!({"id": "custom.lineStyle", "value": {"fill": "dash", "dash": [10, 10]}}),
            );
        }
        overrides.push(json!({
            "matcher": {"id": "byName", "options": label(threshold)},
            "properties": properties,
        }));
    }
    let custom = json!({
        "drawStyle": if chart.column { "bars" } else { "line" },
        "lineWidth": 2,
        "fillOpacity": if chart.stacked { FILL_OPACITY } else { 0 },
        "stacking": {"mode": if chart.stacked { "normal" } else { "none" }},
    });
    json!({
        "panels": [{
            "id": 1,
            "type": "timeseries",
            "title": chart.title,
            "gridPos": {"x": 0, "y": 0, "w": 24, "h": 8},
            "targets": [{"refId": "A"}],
            "fieldConfig": {
                "defaults": {"custom": custom, "unit": unit(&chart.title)},
                "overrides": overrides,
            },
            "options": {
                "legend": {
                    "showLegend": !chart.hide_legend,
                    "displayMode": "list",
                    "placement": "bottom",
                },
                "tooltip": {"mode": "multi"},
            },
        }],
    })
}

impl ChartPanel {
    /// None when the chart has no points to place in time.
    pub fn new(chart: &Chart) -> Option<Self> {
        let all = || chart.series.iter().chain(&chart.threshold);
        let points = all().map(|s| s.points.len()).max()?;
        if points < 2 || chart.step_ms <= 0 {
            return None;
        }
        let step = (chart.step_ms / 1000).max(1);
        let start = chart.from_ms / 1000;
        let span = step * (points as i64 - 1);
        let window = TimeWindow::new(start + span, span as u64, points);
        let dashboard = Dashboard::parse(&panel_json(chart).to_string()).ok()?;
        let spec = dashboard.panels.into_iter().next()?;
        let frame = Frame {
            times: (0..points as i64)
                .map(|i| (start + i * step) as f64)
                .collect(),
            series: all()
                .map(|s| FrameSeries {
                    name: label(s).to_owned(),
                    query: "A".into(),
                    field: None,
                    labels: vec![],
                    values: (0..points)
                        .map(|i| match s.points.get(i) {
                            Some(Some(v)) => f64::from(*v),
                            _ => f64::NAN,
                        })
                        .collect(),
                })
                .collect(),
        };
        let markers = chart
            .annotations
            .iter()
            .filter(|a| a.icon == DEPLOY_ICON)
            .map(|a| Marker {
                kind: MarkerKind::Deploy,
                at: a.from_ms / 1000,
                namespace: None,
                node: None,
                label: a.name.replace("<br>", " · "),
            })
            .collect();
        Some(Self {
            spec,
            result: PanelResult {
                frame,
                warnings: vec![],
                expressions: vec![],
            },
            window,
            markers,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::app_view::Annotation;
    use super::*;
    use grafaui_model::{
        Viz,
        chart::StackMode,
        spec::{DrawStyle, LineStyle},
    };

    fn series(name: &str, points: Vec<Option<f32>>) -> Series {
        Series {
            name: name.into(),
            points,
            ..Default::default()
        }
    }

    fn chart() -> Chart {
        Chart {
            title: "CPU usage, cores".into(),
            from_ms: 1_760_000_000_000,
            to_ms: 1_760_000_120_000,
            step_ms: 60_000,
            series: vec![
                series("worker-a", vec![Some(0.5), None, Some(0.7)]),
                series("worker-b", vec![Some(0.2), Some(0.3), Some(0.4)]),
            ],
            threshold: None,
            featured: false,
            stacked: false,
            column: false,
            shift_colors: false,
            hide_legend: false,
            annotations: vec![],
        }
    }

    fn options(panel: &ChartPanel) -> &grafaui_model::spec::TimeSeriesOptions {
        match &panel.spec.viz {
            Viz::TimeSeries(options) => options,
            other => panic!("a timeseries, not {other:?}"),
        }
    }

    #[test]
    fn a_line_chart_keeps_its_points_and_gaps() {
        let panel = ChartPanel::new(&chart()).unwrap();
        assert!(panel.spec.ignored.is_empty(), "{:?}", panel.spec.ignored);
        assert_eq!(panel.spec.title, "CPU usage, cores");
        let options = options(&panel);
        assert_eq!(options.draw, DrawStyle::Line);
        assert_eq!(options.stacking.mode, StackMode::None);
        assert!(options.legend);
        let frame = &panel.result.frame;
        assert_eq!(
            frame.times,
            [1_760_000_000., 1_760_000_060., 1_760_000_120.]
        );
        assert_eq!(
            panel.window.times(),
            [1_760_000_000, 1_760_000_060, 1_760_000_120]
        );
        assert_eq!(frame.series[0].name, "worker-a");
        // A point Coroot didn't measure is a gap, never zero.
        assert!(frame.series[0].values[1].is_nan());
        assert_eq!(frame.series[1].values, [0.2f32, 0.3, 0.4].map(f64::from));
    }

    #[test]
    fn stacked_column_and_hidden_legend_carry_over() {
        let stacked = ChartPanel::new(&Chart {
            stacked: true,
            ..chart()
        })
        .unwrap();
        assert_eq!(options(&stacked).stacking.mode, StackMode::Normal);
        let column = ChartPanel::new(&Chart {
            column: true,
            hide_legend: true,
            ..chart()
        })
        .unwrap();
        assert_eq!(options(&column).draw, DrawStyle::Bars);
        assert!(!options(&column).legend);
    }

    #[test]
    fn the_threshold_is_a_dashed_series_of_its_own() {
        let panel = ChartPanel::new(&Chart {
            threshold: Some(Series {
                name: "limit".into(),
                title: "CPU limit".into(),
                ..series("", vec![Some(1.), Some(1.), Some(1.)])
            }),
            ..chart()
        })
        .unwrap();
        let names: Vec<_> = panel.result.frame.series.iter().map(|s| &s.name).collect();
        assert_eq!(names, ["worker-a", "worker-b", "CPU limit"]);
        let style = grafaui_model::overrides::style(&panel.spec.field.overrides, "CPU limit");
        assert!(
            matches!(style.line_style, Some(LineStyle::Dashed(_))),
            "{style:?}"
        );
        assert_eq!(style.fill_opacity, Some(0.));
        assert!(
            grafaui_model::overrides::style(&panel.spec.field.overrides, "worker-a")
                .line_style
                .is_none()
        );
        // A total Coroot fills stays solid, behind the stack.
        let filled = ChartPanel::new(&Chart {
            stacked: true,
            threshold: Some(Series {
                fill: true,
                ..series("total", vec![Some(2.), Some(2.), Some(2.)])
            }),
            ..chart()
        })
        .unwrap();
        let style = grafaui_model::overrides::style(&filled.spec.field.overrides, "total");
        assert!(style.line_style.is_none(), "{style:?}");
        assert_eq!(style.fill_opacity, Some(0.1));
        assert_eq!(style.stacking.map(|s| s.mode), Some(StackMode::None));
    }

    #[test]
    fn the_title_names_the_unit() {
        let unit = |title: &str| {
            ChartPanel::new(&Chart {
                title: title.into(),
                ..chart()
            })
            .unwrap()
            .spec
            .field
            .unit
        };
        assert_eq!(
            unit("Network round-trip time, seconds").as_deref(),
            Some("s")
        );
        assert_eq!(unit("Memory usage, bytes").as_deref(), Some("bytes"));
        // 0.002 seconds of delay each second reads as 2 ms.
        assert_eq!(unit("CPU delay, seconds/second").as_deref(), Some("s"));
        // Cores and rates are plain numbers.
        assert_eq!(unit("CPU usage, cores"), None);
        assert_eq!(unit("Instances"), None);
    }

    #[test]
    fn only_deployments_become_markers() {
        let annotation = |icon: &str, name: &str| Annotation {
            name: name.into(),
            from_ms: 1_760_000_060_000,
            to_ms: 1_760_000_060_000,
            icon: icon.into(),
        };
        let panel = ChartPanel::new(&Chart {
            annotations: vec![
                annotation(DEPLOY_ICON, "deployment worker:1.8.2"),
                annotation("mdi-alert-octagon-outline", "worker-a is down"),
            ],
            ..chart()
        })
        .unwrap();
        assert_eq!(panel.markers.len(), 1);
        assert_eq!(panel.markers[0].kind, MarkerKind::Deploy);
        assert_eq!(panel.markers[0].at, 1_760_000_060);
        assert_eq!(panel.markers[0].label, "deployment worker:1.8.2");
    }

    #[test]
    fn a_chart_without_points_is_none() {
        let empty = Chart {
            series: vec![series("worker-a", vec![])],
            ..chart()
        };
        assert!(ChartPanel::new(&empty).is_none());
        assert!(
            ChartPanel::new(&Chart {
                series: vec![],
                ..chart()
            })
            .is_none()
        );
    }
}
