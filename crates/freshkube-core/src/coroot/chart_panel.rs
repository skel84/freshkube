//! A chart from Coroot's application view as a Monitoring panel: a
//! one-panel Grafana timeseries, the answer it shows and its window, so the
//! desktop draws it with the same chart as Monitoring.
use super::app_view::{CHART_LIMITS, Chart, Series};
use crate::monitoring::PanelResult;
use crate::monitoring::markers::{Marker, MarkerKind};
use coroot_rs::{DeploymentRevision, SeriesCoverage, SeriesHistory};
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
    /// How much of the window Coroot covered, when it isn't all of it;
    /// always empty for [`Self::new`], which knows nothing of coverage.
    pub coverage: Vec<Coverage>,
    /// The Console colour of each severity series, by the series' name;
    /// empty unless the chart counts log severities ([`Self::severities`]).
    /// Elsewhere Coroot's colours only tell series apart, so the series
    /// take the next colour in turn.
    pub colors: Vec<(String, SeriesColor)>,
}

/// Where a chart's history falls short of its window. Nothing is filled
/// in for it: a short series stops, an empty one isn't drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Coverage {
    /// Coroot shortened the window it was asked for.
    Truncated,
    /// The series has the first `samples` of the window's `expected`
    /// points; Coroot doesn't say where it really starts, so the rest of the
    /// window is unknown.
    Partial {
        series: String,
        samples: usize,
        expected: usize,
    },
    /// Coroot sent no data for the series.
    Empty { series: String },
}

/// The Console colour for a log severity's Coroot colour. Coroot names
/// Material colours: `red-darken1` for errors, `orange-lighten1` for
/// warnings, `black` for fatal; outside the severities its colours are
/// categorical, so only a severity chart uses these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeriesColor {
    Critical,
    Warning,
    Ok,
    Blue,
    Orange,
    Purple,
    Grey,
}

impl SeriesColor {
    /// The colour drawn for a severity Coroot colours `name`: fatal's black
    /// is critical, as error is; trace's faint green is grey, apart from
    /// debug's green; any other colour takes the nearest Console colour,
    /// whatever its shade. None for an empty or unknown name, which takes
    /// the next colour in turn.
    pub fn severity(name: &str) -> Option<Self> {
        let name = name.trim();
        let (hue, shade) = ["-lighten", "-darken", "-accent"]
            .iter()
            .find_map(|kind| {
                let (hue, level) = name.split_once(kind)?;
                Some((hue, Some((*kind, level))))
            })
            .unwrap_or((name, None));
        Some(match (hue, shade) {
            ("black", _) => Self::Critical,
            ("green", Some(("-lighten", "3" | "4" | "5"))) => Self::Grey,
            ("red" | "pink", _) => Self::Critical,
            ("orange" | "amber" | "yellow" | "lime", _) => Self::Warning,
            ("green" | "light-green" | "teal", _) => Self::Ok,
            ("blue" | "light-blue" | "cyan" | "indigo", _) => Self::Blue,
            ("deep-orange" | "brown", _) => Self::Orange,
            ("purple" | "deep-purple", _) => Self::Purple,
            ("grey" | "blue-grey" | "white", _) => Self::Grey,
            _ => return None,
        })
    }
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
            coverage: vec![],
            colors: vec![],
        })
    }

    /// The chart drawn from its [`Chart::history`]: sample *i* at
    /// [`coroot_rs::ChartHistory::point_time`], across the whole window whatever a
    /// series covers, a missing sample a gap, and what the history lacks in
    /// [`Self::coverage`]. The layout gives the legend names, colours and
    /// fill. Deploy markers come from the revisions
    /// ([`Self::with_revisions`]), not annotations, which can't tell a
    /// deploy from an incident. None without a history, with fewer than
    /// two points in its window, or with more than core's bound
    /// ([`CHART_LIMITS`]): Coroot's `ctx` sets the window, not the data, so a
    /// tiny answer can ask for billions of points.
    pub fn from_history(chart: &Chart) -> Option<Self> {
        let history = chart.history.as_ref()?;
        let points = history.expected_points();
        let step = i64::try_from(history.step.as_secs()).ok()?.max(1);
        if !(2..=CHART_LIMITS.max_points).contains(&points) {
            return None;
        }
        let start = history.anchor().timestamp();
        let span = step * (points as i64 - 1);
        let window = TimeWindow::new(start + span, span as u64, points);
        let dashboard = Dashboard::parse(&panel_json(chart).to_string()).ok()?;
        let spec = dashboard.panels.into_iter().next()?;
        let named = |ix: Option<usize>, s: &SeriesHistory| {
            let layout = match ix {
                Some(ix) => chart.series.get(ix),
                None => chart.threshold.as_ref(),
            };
            match layout {
                Some(layout) => label(layout).to_owned(),
                None if s.title.is_empty() => s.name.clone(),
                None => s.title.clone(),
            }
        };
        let mut coverage = vec![];
        if history.truncated {
            coverage.push(Coverage::Truncated);
        }
        let mut series = vec![];
        let all = history
            .series
            .iter()
            .enumerate()
            .map(|(ix, s)| (Some(ix), s))
            .chain(history.threshold.iter().map(|s| (None, s)));
        for (ix, s) in all {
            let name = named(ix, s);
            match s.coverage {
                SeriesCoverage::Empty => {
                    coverage.push(Coverage::Empty { series: name });
                    continue;
                }
                SeriesCoverage::Partial => coverage.push(Coverage::Partial {
                    series: name.clone(),
                    samples: s.samples.len(),
                    expected: points,
                }),
                SeriesCoverage::Full => {}
            }
            series.push(FrameSeries {
                name,
                query: "A".into(),
                field: None,
                labels: vec![],
                values: (0..points)
                    .map(|i| s.samples.get(i).copied().flatten().unwrap_or(f64::NAN))
                    .collect(),
            });
        }
        Some(Self {
            spec,
            result: PanelResult {
                frame: Frame {
                    times: (0..points as i64)
                        .map(|i| (start + i * step) as f64)
                        .collect(),
                    series,
                },
                warnings: vec![],
                expressions: vec![],
            },
            window,
            markers: vec![],
            coverage,
            colors: vec![],
        })
    }

    /// Marks each revision that started within the window, as Monitoring
    /// marks a Deployment's new ReplicaSet.
    pub fn with_revisions(
        mut self,
        revisions: &[DeploymentRevision],
        namespace: Option<&str>,
    ) -> Self {
        let last = self.window.end;
        let first = last.saturating_sub(
            self.window
                .step()
                .saturating_mul(self.window.points as i64 - 1),
        );
        self.markers.extend(
            revisions
                .iter()
                .map(|r| revision_marker(r, namespace))
                .filter(|m| (first..=last).contains(&m.at)),
        );
        self
    }
}

/// A revision as a deploy marker, at the second its rollout started,
/// labelled as Coroot labels the revision: its hash and images.
pub fn revision_marker(revision: &DeploymentRevision, namespace: Option<&str>) -> Marker {
    Marker {
        kind: MarkerKind::Deploy,
        at: revision.started_at.timestamp(),
        namespace: namespace.map(str::to_owned),
        node: None,
        label: revision.version.clone(),
    }
}

impl ChartPanel {
    /// A chart of log messages by severity, as the Logs report's histogram
    /// and a pattern's chart are: [`Self::new`], with each series drawn in
    /// its severity's colour.
    pub fn severities(chart: &Chart) -> Option<Self> {
        let mut panel = Self::new(chart)?;
        panel.colors = chart
            .series
            .iter()
            .filter_map(|s| Some((label(s).to_owned(), SeriesColor::severity(&s.color)?)))
            .collect();
        Some(panel)
    }
}

#[cfg(test)]
mod tests {
    use super::super::app_view::Annotation;
    use super::*;
    use coroot_rs::ChartHistory;
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
            history: None,
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
    fn severity_colours_take_the_nearest_console_colour() {
        let color = SeriesColor::severity;
        // Coroot's severities, most severe first.
        assert_eq!(color("black"), Some(SeriesColor::Critical));
        assert_eq!(color("red-darken1"), Some(SeriesColor::Critical));
        assert_eq!(color("orange-lighten1"), Some(SeriesColor::Warning));
        assert_eq!(color("blue-lighten2"), Some(SeriesColor::Blue));
        assert_eq!(color("green-lighten2"), Some(SeriesColor::Ok));
        assert_eq!(color("green-lighten4"), Some(SeriesColor::Grey));
        assert_eq!(color("grey-lighten1"), Some(SeriesColor::Grey));
        // Any other shade takes its hue's colour.
        assert_eq!(color("red"), Some(SeriesColor::Critical));
        assert_eq!(color("amber"), Some(SeriesColor::Warning));
        assert_eq!(color("green-darken1"), Some(SeriesColor::Ok));
        assert_eq!(color("deep-orange-accent2"), Some(SeriesColor::Orange));
        assert_eq!(color("deep-purple"), Some(SeriesColor::Purple));
        assert_eq!(color("blue-grey-darken3"), Some(SeriesColor::Grey));
        // Empty or unknown: the next colour in turn.
        assert_eq!(color(""), None);
        assert_eq!(color("chartreuse"), None);
    }

    #[test]
    fn only_a_severity_chart_carries_its_colours() {
        let coloured = |name: &str, color: &str| Series {
            color: color.into(),
            ..series(name, vec![Some(1.), Some(2.)])
        };
        let chart = Chart {
            series: vec![
                coloured("fatal", "black"),
                coloured("error", "red-darken1"),
                coloured("warning", "orange-lighten1"),
                coloured("info", "blue-lighten2"),
                coloured("trace", "green-lighten4"),
                coloured("unknown", ""),
                coloured("odd", "chartreuse"),
            ],
            stacked: true,
            column: true,
            ..chart()
        };
        assert_eq!(
            ChartPanel::severities(&chart).unwrap().colors,
            [
                ("fatal".to_owned(), SeriesColor::Critical),
                ("error".to_owned(), SeriesColor::Critical),
                ("warning".to_owned(), SeriesColor::Warning),
                ("info".to_owned(), SeriesColor::Blue),
                ("trace".to_owned(), SeriesColor::Grey),
            ]
        );
        // Elsewhere Coroot's colours tell series apart: memory used is red
        // on a healthy node, so the series take their turn.
        assert!(ChartPanel::new(&chart).unwrap().colors.is_empty());
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

    fn sampled(name: &str, samples: Vec<Option<f64>>, coverage: SeriesCoverage) -> SeriesHistory {
        SeriesHistory {
            name: name.into(),
            title: String::new(),
            samples,
            coverage,
        }
    }

    /// Five minutes at one-minute steps, asked from 20 s past a minute:
    /// sample 0 sits on the minute, where Coroot read it from.
    fn history() -> ChartHistory {
        let at = |s: i64| chrono::DateTime::from_timestamp(s, 0).unwrap();
        ChartHistory {
            group: None,
            title: "CPU usage, cores".into(),
            from: at(1_759_999_980 + 20),
            to: at(1_759_999_980 + 260),
            step: std::time::Duration::from_secs(60),
            truncated: false,
            stacked: false,
            series: vec![
                sampled(
                    "worker-a",
                    vec![Some(0.5), None, Some(0.7), Some(0.6), Some(0.4)],
                    SeriesCoverage::Full,
                ),
                sampled(
                    "worker-b",
                    vec![Some(0.2), Some(0.3)],
                    SeriesCoverage::Partial,
                ),
                sampled("worker-c", vec![], SeriesCoverage::Empty),
            ],
            threshold: Some(sampled("limit", vec![Some(1.); 5], SeriesCoverage::Full)),
            annotations: vec![],
        }
    }

    fn charted(history: ChartHistory) -> Chart {
        Chart {
            series: vec![series("worker-a", vec![]), series("worker-b", vec![])],
            threshold: Some(Series {
                name: "limit".into(),
                title: "CPU limit".into(),
                ..Default::default()
            }),
            history: Some(Box::new(history)),
            ..chart()
        }
    }

    #[test]
    fn a_history_fills_its_window_and_keeps_its_gaps() {
        let panel = ChartPanel::from_history(&charted(history())).unwrap();
        let frame = &panel.result.frame;
        // From the minute the data starts at, every point of the window.
        assert_eq!(frame.times.len(), 5);
        assert_eq!(frame.times[0], 1_759_999_980.);
        assert_eq!(panel.window.times()[4], 1_759_999_980 + 240);
        let names: Vec<_> = frame.series.iter().map(|s| s.name.as_str()).collect();
        // An empty series isn't drawn; the threshold keeps its layout name.
        assert_eq!(names, ["worker-a", "worker-b", "CPU limit"]);
        assert!(frame.series[0].values[1].is_nan());
        assert_eq!(frame.series[0].values[4], 0.4);
        // A short series stops; the rest of the window stays unknown.
        assert_eq!(frame.series[1].values[1], 0.3);
        assert!(frame.series[1].values[2..].iter().all(|v| v.is_nan()));
        assert_eq!(
            panel.coverage,
            [
                Coverage::Partial {
                    series: "worker-b".into(),
                    samples: 2,
                    expected: 5,
                },
                Coverage::Empty {
                    series: "worker-c".into()
                },
            ]
        );
        assert!(panel.markers.is_empty());
    }

    #[test]
    fn a_shortened_window_says_so_and_no_history_draws_nothing() {
        let mut truncated = history();
        truncated.truncated = true;
        truncated.series.truncate(1);
        let panel = ChartPanel::from_history(&charted(truncated)).unwrap();
        assert_eq!(panel.coverage, [Coverage::Truncated]);
        assert!(ChartPanel::from_history(&chart()).is_none());
        // The layout's own points don't stand in for a missing history.
        assert!(ChartPanel::new(&chart()).is_some());
    }

    #[test]
    fn a_window_over_the_point_bound_draws_nothing() {
        let mut wide = history();
        wide.from = chrono::DateTime::from_timestamp(1, 0).unwrap();
        wide.step = std::time::Duration::from_secs(1);
        wide.series.truncate(1);
        wide.series[0].samples = vec![Some(1.)];
        wide.series[0].coverage = SeriesCoverage::Partial;
        wide.threshold = None;
        assert!(wide.expected_points() > CHART_LIMITS.max_points);
        assert!(ChartPanel::from_history(&charted(wide)).is_none());
    }

    #[test]
    fn revisions_mark_the_window_they_started_in() {
        let revision = |at: i64, version: &str| DeploymentRevision {
            id: format!("{version}:{at}"),
            hash: version.into(),
            started_at: chrono::DateTime::from_timestamp(at, 0).unwrap(),
            version: format!("{version}: example.test/shop/worker:1.8.2"),
            status: coroot_rs::Status::Ok,
            findings: vec![],
            note: Some("No notable changes".into()),
        };
        let panel = ChartPanel::from_history(&charted(history()))
            .unwrap()
            .with_revisions(
                &[
                    revision(1_759_999_980 + 300, "after"),
                    revision(1_759_999_980 + 130, "within"),
                    revision(1_759_990_000, "before"),
                ],
                Some("shop"),
            );
        assert_eq!(
            panel.markers,
            [Marker {
                kind: MarkerKind::Deploy,
                at: 1_759_999_980 + 130,
                namespace: Some("shop".into()),
                node: None,
                label: "within: example.test/shop/worker:1.8.2".into(),
            }]
        );
    }
}
