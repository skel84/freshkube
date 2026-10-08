//! One application as Coroot's own page shows it: the map of its clients,
//! instances and dependencies, then one report per tab, each with its checks
//! and the charts, tables and other widgets Coroot drew for it.
use super::{tracing::plain, *};
use coroot_rs::{AppCharts, ChartHistory, ChartLimits, DeploymentRevision, Envelope};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct AppView {
    pub map: AppMap,
    /// In Coroot's order: SLO, Instances, CPU, … Profiling, Tracing.
    pub reports: Vec<AppReport>,
    /// Why some or all charts carry no [`Chart::history`]: coroot-rs
    /// couldn't decode the answer the layout came from, or a report's charts
    /// didn't line up with it.
    pub history_error: Option<ReadError>,
    /// The application's deployments, newest first, decoded from the same
    /// answer; a failure here leaves the rest of the page. None when they
    /// weren't read, as for example data that has none.
    pub revisions: Option<Result<Vec<DeploymentRevision>, ReadError>>,
}

#[derive(Clone, Debug, Default)]
pub struct AppMap {
    pub app: MapApp,
    /// Running instances, by name.
    pub instances: Vec<MapInstance>,
    /// Applications that call this one.
    pub clients: Vec<MapApp>,
    /// Applications this one calls.
    pub dependencies: Vec<MapApp>,
}

#[derive(Clone, Debug)]
pub struct MapApp {
    pub id: AppId,
    pub cluster: String,
    pub category: String,
    pub status: Status,
    /// Coroot's icon name for the application type, such as `postgres`.
    pub icon: String,
    pub labels: BTreeMap<String, String>,
    /// The connection to the page's application; none for the application itself.
    pub link: Option<MapLink>,
}

impl Default for MapApp {
    fn default() -> Self {
        Self {
            id: AppId::new(""),
            cluster: String::new(),
            category: String::new(),
            status: Status::Unknown,
            icon: String::new(),
            labels: BTreeMap::new(),
            link: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapLink {
    pub status: Status,
    /// Why the link is not ok, as Coroot words it.
    pub reason: String,
    /// The link carries traffic both ways.
    pub both_ways: bool,
    /// Coroot's figures for the link, such as `12 rps` or `3ms`.
    pub stats: Vec<String>,
    /// Requests per second, when Coroot measured them.
    pub weight: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapInstance {
    pub id: String,
    /// Role, version or proxy, when Coroot knows them.
    pub labels: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct AppReport {
    pub name: String,
    pub status: Status,
    pub checks: Vec<Check>,
    pub widgets: Vec<Widget>,
    /// Coroot's own custom report, drawn without a checks card.
    pub custom: bool,
    /// The database or runtime Coroot could instrument for this report, if any.
    pub instrumentation: String,
}

impl AppReport {
    /// Coroot shows a report's status only when it has something to judge by.
    pub fn judged(&self) -> bool {
        !self.checks.is_empty() || !self.instrumentation.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub id: String,
    pub title: String,
    pub status: Status,
    /// Coroot's verdict when the check fired; empty means ok.
    pub message: String,
    pub threshold: f32,
    pub unit: String,
    /// Such as `the CPU usage of a node > <threshold>`.
    pub condition: String,
}

impl Check {
    /// The condition around its threshold: what precedes it, the threshold
    /// in the check's unit, and what follows it.
    pub fn condition(&self) -> (&str, String, &str) {
        let (head, tail) = self
            .condition
            .split_once("<threshold>")
            .unwrap_or((self.condition.as_str(), ""));
        let threshold = match self.unit.as_str() {
            "percent" => format!("{}%", number(self.threshold)),
            "second" => duration(self.threshold),
            "seconds/second" => format!("{} seconds/second", number(self.threshold)),
            _ => number(self.threshold),
        };
        (head, threshold, tail)
    }
}

fn number(value: f32) -> String {
    let rounded = (f64::from(value) * 1000.).round() / 1000.;
    format!("{rounded}")
}

/// Seconds as Coroot writes a threshold: `100ms`, `5s`, `2m`.
fn duration(seconds: f32) -> String {
    let ms = (f64::from(seconds) * 1000.).round();
    if ms < 1000. {
        format!("{ms}ms")
    } else if ms < 60_000. {
        format!("{}s", number((ms / 1000.) as f32))
    } else {
        format!("{}m", number((ms / 60_000.) as f32))
    }
}

#[derive(Clone, Debug)]
pub struct Widget {
    pub kind: WidgetKind,
    /// The share of the row it takes; Coroot's default is half.
    pub width: f32,
}

#[derive(Clone, Debug)]
pub enum WidgetKind {
    Chart(Chart),
    /// Charts of one quantity, of which the reader picks one. The title
    /// marks where the picker goes with `<selector>`.
    ChartGroup {
        title: String,
        charts: Vec<Chart>,
    },
    Table(Table),
    Heatmap(Heatmap),
    /// The application's log search, with the check that judges its errors.
    Logs(Option<Check>),
    /// The application's profiles, as the Profiling page shows them.
    Profiling,
    /// The application's traces, as the Traces page shows them.
    Tracing,
    /// A heading between groups of widgets.
    Header(String),
    /// A widget this reader does not draw, by its kind.
    Other(&'static str),
}

/// One time series chart. Points are one per `step_ms` from `from_ms`.
#[derive(Clone, Debug, Default)]
pub struct Chart {
    pub title: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub step_ms: i64,
    pub series: Vec<Series>,
    /// A limit or desired line drawn over the series.
    pub threshold: Option<Series>,
    /// The chart a group shows first.
    pub featured: bool,
    pub stacked: bool,
    pub column: bool,
    /// Colours start one place later, so a second chart's series differ.
    pub shift_colors: bool,
    pub hide_legend: bool,
    pub annotations: Vec<Annotation>,
    /// The chart's history as coroot-rs decodes it: where each sample sits
    /// in time, the gaps and how much of the window each series covers.
    /// The fields above stay the layout: colours, fill, stacking, legend.
    pub history: Option<Box<ChartHistory>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Series {
    pub name: String,
    pub title: String,
    /// A colour name from Coroot's palette, such as `red` or `grey-lighten1`;
    /// empty for the next colour in turn.
    pub color: String,
    pub fill: bool,
    /// The layout's own points, for a chart drawn without a history, such
    /// as the Logs report's histogram. Emptied once the chart takes its
    /// [`Chart::history`], which alone places samples in time.
    pub points: Vec<Option<f32>>,
}

/// An incident or deployment marked on a chart, in epoch milliseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    pub name: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub icon: String,
}

#[derive(Clone, Debug, Default)]
pub struct Heatmap {
    pub title: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub step_ms: i64,
    pub rows: Vec<Series>,
}

#[derive(Clone, Debug, Default)]
pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
    pub value: String,
    pub short_value: String,
    /// A cell of several lines.
    pub values: Vec<String>,
    pub tags: Vec<String>,
    pub unit: String,
    pub status: Option<Status>,
    /// Coroot's icon name and colour.
    pub icon: Option<(String, String)>,
    /// What the cell links to in Coroot, such as `nodes` for a node.
    pub link: Option<CellLink>,
    pub progress: Option<(u8, String)>,
    /// Received and sent, as Coroot formatted them.
    pub bandwidth: Option<(String, String)>,
    pub chart: Vec<Option<f32>>,
    /// A placeholder such as `—`.
    pub stub: bool,
    pub deployments: Vec<DeploymentSummary>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CellLink {
    pub title: String,
    /// The Coroot view: `nodes`, `applications`, …, or the route name.
    pub view: String,
    /// The linked item's id within that view.
    pub id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentSummary {
    pub report: String,
    pub ok: bool,
    pub message: String,
    pub time_ms: Option<i64>,
}

#[derive(Deserialize)]
struct RawView {
    app_map: Option<RawMap>,
    #[serde(default)]
    reports: Option<Vec<RawReport>>,
}

#[derive(Deserialize)]
struct RawMap {
    application: Option<RawApp>,
    #[serde(default)]
    instances: Option<Vec<RawInstance>>,
    #[serde(default)]
    clients: Option<Vec<RawApp>>,
    #[serde(default)]
    dependencies: Option<Vec<RawApp>>,
}

#[derive(Deserialize)]
struct RawApp {
    id: AppId,
    #[serde(default)]
    cluster: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    status: Status,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    labels: Option<BTreeMap<String, String>>,
    #[serde(default)]
    link_status: Status,
    #[serde(default)]
    link_status_reason: Option<String>,
    #[serde(default)]
    link_direction: Option<String>,
    #[serde(default)]
    link_stats: Option<Vec<String>>,
    #[serde(default)]
    link_weight: Option<f32>,
}

#[derive(Deserialize)]
struct RawInstance {
    id: String,
    #[serde(default)]
    labels: Option<BTreeMap<String, String>>,
}

#[derive(Deserialize)]
struct RawReport {
    name: String,
    #[serde(default)]
    status: Status,
    #[serde(default)]
    checks: Option<Vec<RawCheck>>,
    #[serde(default)]
    widgets: Option<Vec<RawWidget>>,
    #[serde(default)]
    custom: bool,
    #[serde(default)]
    instrumentation: Option<String>,
}

#[derive(Deserialize)]
struct RawCheck {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    status: Status,
    #[serde(default)]
    message: String,
    #[serde(default)]
    threshold: Option<f32>,
    #[serde(default)]
    unit: Option<String>,
    #[serde(default)]
    condition_format_template: String,
}

#[derive(Deserialize)]
struct RawWidget {
    chart: Option<RawChart>,
    chart_group: Option<RawChartGroup>,
    table: Option<RawTable>,
    heatmap: Option<RawHeatmap>,
    logs: Option<RawLogs>,
    profiling: Option<serde_json::Value>,
    tracing: Option<serde_json::Value>,
    dependency_map: Option<serde_json::Value>,
    #[serde(default)]
    group_header: Option<String>,
    #[serde(default)]
    width: Option<String>,
}

#[derive(Deserialize)]
struct RawContext {
    from: i64,
    to: i64,
    step: i64,
}

#[derive(Deserialize)]
pub(super) struct RawChart {
    ctx: RawContext,
    #[serde(default)]
    title: String,
    #[serde(default)]
    series: Option<Vec<RawSeries>>,
    #[serde(default)]
    threshold: Option<RawSeries>,
    #[serde(default)]
    featured: bool,
    #[serde(default)]
    stacked: bool,
    #[serde(default)]
    column: bool,
    #[serde(default)]
    color_shift: i32,
    #[serde(default)]
    hide_legend: bool,
    #[serde(default)]
    annotations: Option<Vec<RawAnnotation>>,
}

#[derive(Deserialize)]
struct RawChartGroup {
    #[serde(default)]
    title: String,
    #[serde(default)]
    charts: Option<Vec<RawChart>>,
}

#[derive(Deserialize)]
struct RawSeries {
    #[serde(default)]
    name: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    fill: bool,
    #[serde(default)]
    data: Option<Vec<Option<f32>>>,
}

#[derive(Deserialize)]
struct RawAnnotation {
    #[serde(default)]
    name: String,
    #[serde(default)]
    x1: Option<i64>,
    #[serde(default)]
    x2: Option<i64>,
    #[serde(default)]
    icon: String,
}

#[derive(Deserialize)]
struct RawHeatmap {
    ctx: RawContext,
    #[serde(default)]
    title: String,
    #[serde(default)]
    series: Option<Vec<RawSeries>>,
}

#[derive(Deserialize)]
struct RawLogs {
    #[serde(default)]
    check: Option<RawCheck>,
}

#[derive(Deserialize)]
struct RawTable {
    #[serde(default)]
    header: Option<Vec<String>>,
    #[serde(default)]
    rows: Option<Vec<RawRow>>,
}

#[derive(Deserialize)]
struct RawRow {
    #[serde(default)]
    cells: Option<Vec<Option<RawCell>>>,
}

#[derive(Deserialize, Default)]
struct RawCell {
    #[serde(default)]
    icon: Option<RawIcon>,
    #[serde(default)]
    value: String,
    #[serde(default)]
    short_value: String,
    #[serde(default)]
    values: Option<Vec<String>>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    unit: String,
    #[serde(default)]
    status: Option<Status>,
    #[serde(default)]
    link: Option<RawLink>,
    #[serde(default)]
    progress: Option<RawProgress>,
    #[serde(default)]
    bandwidth: Option<RawBandwidth>,
    #[serde(default)]
    chart: Option<Vec<Option<f32>>>,
    #[serde(default)]
    is_stub: bool,
    #[serde(default)]
    deployment_summaries: Option<Vec<RawSummary>>,
}

#[derive(Deserialize)]
struct RawIcon {
    #[serde(default)]
    name: String,
    #[serde(default)]
    color: String,
}

#[derive(Deserialize)]
struct RawLink {
    #[serde(default)]
    title: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    params: Option<BTreeMap<String, serde_json::Value>>,
}

#[derive(Deserialize)]
struct RawProgress {
    #[serde(default)]
    percent: i64,
    #[serde(default)]
    color: String,
}

#[derive(Deserialize)]
struct RawBandwidth {
    #[serde(default, rename = "Rx")]
    rx: String,
    #[serde(default, rename = "Tx")]
    tx: String,
}

#[derive(Deserialize)]
struct RawSummary {
    #[serde(default)]
    report: String,
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    message: String,
    #[serde(default)]
    time: Option<i64>,
}

pub(super) fn decode(data: serde_json::Value) -> Result<AppView, ReadError> {
    if data.is_null() {
        return Err(ReadError::Missing);
    }
    let raw: RawView = serde_json::from_value(data).map_err(|_| ReadError::InvalidResponse)?;
    let map = raw.app_map.ok_or(ReadError::InvalidResponse)?;
    let app = map.application.ok_or(ReadError::InvalidResponse)?;
    let apps = |apps: Option<Vec<RawApp>>| {
        apps.unwrap_or_default()
            .into_iter()
            .map(|a| map_app(a, true))
            .collect()
    };
    Ok(AppView {
        map: AppMap {
            app: map_app(app, false),
            instances: map
                .instances
                .unwrap_or_default()
                .into_iter()
                .map(|i| MapInstance {
                    id: i.id,
                    labels: i.labels.unwrap_or_default(),
                })
                .collect(),
            clients: apps(map.clients),
            dependencies: apps(map.dependencies),
        },
        reports: raw
            .reports
            .unwrap_or_default()
            .into_iter()
            .map(report)
            .collect::<Result<_, _>>()?,
        history_error: None,
        revisions: None,
    })
}

fn map_app(raw: RawApp, linked: bool) -> MapApp {
    MapApp {
        id: raw.id,
        cluster: raw.cluster.unwrap_or_default(),
        category: raw.category.unwrap_or_default(),
        status: raw.status,
        icon: raw.icon.unwrap_or_default(),
        labels: raw.labels.unwrap_or_default(),
        link: linked.then(|| MapLink {
            status: raw.link_status,
            reason: plain(&raw.link_status_reason.unwrap_or_default()),
            both_ways: raw.link_direction.as_deref() == Some("both"),
            stats: raw.link_stats.unwrap_or_default(),
            weight: raw.link_weight.filter(|w| w.is_finite() && *w > 0.),
        }),
    }
}

fn report(raw: RawReport) -> Result<AppReport, ReadError> {
    Ok(AppReport {
        name: raw.name,
        status: raw.status,
        checks: raw
            .checks
            .unwrap_or_default()
            .into_iter()
            .map(check)
            .collect(),
        widgets: raw
            .widgets
            .unwrap_or_default()
            .into_iter()
            .map(widget)
            .collect::<Result<_, _>>()?,
        custom: raw.custom,
        instrumentation: raw.instrumentation.unwrap_or_default(),
    })
}

/// Coroot's placeholders are shaped like tags, so markup is dropped around one.
fn plain_around(text: &str, placeholder: &str) -> String {
    text.split(placeholder)
        .map(plain)
        .collect::<Vec<_>>()
        .join(placeholder)
}

fn check(raw: RawCheck) -> Check {
    Check {
        id: raw.id,
        title: plain(&raw.title),
        status: raw.status,
        message: plain(&raw.message),
        threshold: raw.threshold.filter(|t| t.is_finite()).unwrap_or(0.),
        unit: raw.unit.unwrap_or_default(),
        condition: plain_around(&raw.condition_format_template, "<threshold>"),
    }
}

fn widget(raw: RawWidget) -> Result<Widget, ReadError> {
    let width = raw
        .width
        .as_deref()
        .and_then(|w| w.strip_suffix('%'))
        .and_then(|w| w.trim().parse::<f32>().ok())
        .filter(|w| *w > 0. && *w <= 100.)
        .map_or(0.5, |w| w / 100.);
    let kind = if let Some(c) = raw.chart {
        WidgetKind::Chart(chart(c)?)
    } else if let Some(g) = raw.chart_group {
        WidgetKind::ChartGroup {
            title: plain_around(&g.title, "<selector>"),
            charts: g
                .charts
                .unwrap_or_default()
                .into_iter()
                .map(chart)
                .collect::<Result<_, _>>()?,
        }
    } else if let Some(t) = raw.table {
        WidgetKind::Table(table(t))
    } else if let Some(h) = raw.heatmap {
        let (from_ms, to_ms, step_ms) = context(&h.ctx)?;
        WidgetKind::Heatmap(Heatmap {
            title: plain(&h.title),
            from_ms,
            to_ms,
            step_ms,
            rows: h
                .series
                .unwrap_or_default()
                .into_iter()
                .map(series)
                .collect(),
        })
    } else if let Some(l) = raw.logs {
        WidgetKind::Logs(l.check.map(check))
    } else if raw.profiling.is_some() {
        WidgetKind::Profiling
    } else if raw.tracing.is_some() {
        WidgetKind::Tracing
    } else if let Some(h) = raw.group_header.filter(|h| !h.is_empty()) {
        WidgetKind::Header(plain(&h))
    } else if raw.dependency_map.is_some() {
        WidgetKind::Other("dependency map")
    } else {
        WidgetKind::Other("unknown")
    };
    Ok(Widget { kind, width })
}

fn context(ctx: &RawContext) -> Result<(i64, i64, i64), ReadError> {
    if ctx.step > 0 && ctx.from < ctx.to {
        Ok((ctx.from, ctx.to, ctx.step))
    } else {
        Err(ReadError::InvalidResponse)
    }
}

pub(super) fn chart(raw: RawChart) -> Result<Chart, ReadError> {
    let (from_ms, to_ms, step_ms) = context(&raw.ctx)?;
    Ok(Chart {
        title: plain(&raw.title),
        from_ms,
        to_ms,
        step_ms,
        series: raw
            .series
            .unwrap_or_default()
            .into_iter()
            .map(series)
            .collect(),
        threshold: raw.threshold.map(series),
        featured: raw.featured,
        stacked: raw.stacked || raw.column,
        column: raw.column,
        shift_colors: raw.color_shift != 0,
        hide_legend: raw.hide_legend,
        annotations: raw
            .annotations
            .unwrap_or_default()
            .into_iter()
            .filter_map(|a| {
                let from_ms = a.x1?;
                Some(Annotation {
                    name: plain(&a.name),
                    from_ms,
                    to_ms: a.x2.unwrap_or(from_ms),
                    icon: a.icon,
                })
            })
            .collect(),
        history: None,
    })
}

fn series(raw: RawSeries) -> Series {
    Series {
        name: plain(&raw.name),
        title: plain(&raw.title.unwrap_or_default()),
        color: raw.color.unwrap_or_default(),
        fill: raw.fill,
        points: raw
            .data
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.filter(|v| v.is_finite()))
            .collect(),
    }
}

fn table(raw: RawTable) -> Table {
    Table {
        header: raw.header.unwrap_or_default(),
        rows: raw
            .rows
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                r.cells
                    .unwrap_or_default()
                    .into_iter()
                    .map(|c| cell(c.unwrap_or_default()))
                    .collect()
            })
            .collect(),
    }
}

fn cell(raw: RawCell) -> Cell {
    Cell {
        value: plain(&raw.value),
        short_value: plain(&raw.short_value),
        values: raw
            .values
            .unwrap_or_default()
            .iter()
            .map(|v| plain(v))
            .collect(),
        tags: raw.tags.unwrap_or_default(),
        unit: raw.unit,
        status: raw.status,
        icon: raw.icon.map(|i| (i.name, i.color)),
        link: raw.link.map(|l| {
            let params = l.params.unwrap_or_default();
            let param = |key: &str| match params.get(key) {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(serde_json::Value::Null) | None => String::new(),
                Some(other) => other.to_string(),
            };
            let view = param("view");
            CellLink {
                title: l.title,
                view: if view.is_empty() { l.name } else { view },
                id: param("id"),
            }
        }),
        progress: raw
            .progress
            .map(|p| (p.percent.clamp(0, 100) as u8, p.color)),
        bandwidth: raw.bandwidth.map(|b| (b.rx, b.tx)),
        chart: raw.chart.unwrap_or_default(),
        stub: raw.is_stub,
        deployments: raw
            .deployment_summaries
            .unwrap_or_default()
            .into_iter()
            .map(|s| DeploymentSummary {
                report: s.report,
                ok: s.ok,
                message: plain(&s.message),
                time_ms: s.time,
            })
            .collect(),
    }
}

impl Provider {
    /// An application's page in `range`: its map and every report.
    pub async fn app_view(
        &self,
        source: &Source,
        range: TimeRange,
        app: &AppId,
    ) -> Result<AppView, ReadError> {
        let project = self.project(source, range)?;
        let path = format!("app/{}", coroot_rs::util::encode_segment(app.as_str()));
        let envelope = self.read(project.get(&path, &[])).await?;
        decode_all(envelope, app)
    }
}

/// Coroot's application page within core's bounds: every chart's history
/// and the revisions come from the one answer the layout came from, so the
/// three can't disagree.
pub(super) const CHART_LIMITS: ChartLimits = ChartLimits {
    max_reports: 32,
    max_charts: 512,
    max_series: 128,
    max_points: 4_096,
    max_annotations: 256,
    max_total_samples: 2_000_000,
};

pub(super) fn decode_all(envelope: Envelope, app: &AppId) -> Result<AppView, ReadError> {
    let histories = AppCharts::from_envelope(&envelope, app, &CHART_LIMITS)
        .map_err(|error| refused("chart histories", app, error));
    let revisions =
        DeploymentRevision::list_from_envelope(&envelope, app, coroot_rs::DEFAULT_MAX_REVISIONS)
            .map_err(|error| refused("deployment revisions", app, error))
            .and_then(|revisions| limits::revisions(&revisions).map(|()| revisions));
    let mut view = decode(envelope.data)?;
    if view.map.app.id != *app {
        return Err(ReadError::InvalidResponse);
    }
    limits::app_view(&view)?;
    match histories {
        Ok(histories) => view.history_error = attach(&mut view.reports, histories).err(),
        Err(error) => view.history_error = Some(error),
    }
    view.revisions = Some(revisions);
    Ok(view)
}

/// What coroot-rs refused in the answer, logged with its own words, which
/// name the field or the bound. coroot-rs gives a bound it passed no kind of
/// its own, only a decode error that says so; that reads as a limit here.
fn refused(what: &str, app: &AppId, error: coroot_rs::Error) -> ReadError {
    ::tracing::warn!(app = app.as_str(), "Coroot's {what}: {}", error.message());
    let message = error.message();
    let over = message.contains("exceeds ChartLimits::") || message.contains("over the limit of");
    match error.kind() {
        coroot_rs::ErrorKind::Decode if over => ReadError::Limit,
        _ => ReadError::from(error),
    }
}

/// Gives each chart its history. coroot-rs lists a report's charts in
/// widget order, a group's in place, as the layout does; a report that
/// doesn't line up keeps none rather than take another chart's.
fn attach(reports: &mut [AppReport], histories: AppCharts) -> Result<(), ReadError> {
    if histories.reports.len() != reports.len() {
        return Err(ReadError::InvalidResponse);
    }
    let mut result = Ok(());
    for (report, decoded) in reports.iter_mut().zip(histories.reports) {
        let mut charts: Vec<&mut Chart> = report
            .widgets
            .iter_mut()
            .flat_map(|w| match &mut w.kind {
                WidgetKind::Chart(chart) => std::slice::from_mut(chart),
                WidgetKind::ChartGroup { charts, .. } => charts.as_mut_slice(),
                _ => &mut [],
            })
            .collect();
        if decoded.name != report.name || decoded.charts.len() != charts.len() {
            result = Err(ReadError::InvalidResponse);
            continue;
        }
        for (chart, history) in charts.iter_mut().zip(decoded.charts) {
            // coroot-rs bounds each series' data, not the window `ctx`
            // implies, which sets how many points a panel lays out.
            let points = history.expected_points();
            if points > CHART_LIMITS.max_points {
                ::tracing::warn!(
                    report = report.name.as_str(),
                    chart = chart.title.as_str(),
                    "Coroot's chart window has {points} points, over the limit of {}",
                    CHART_LIMITS.max_points
                );
                chart.history = None;
                result = result.and(Err(ReadError::Limit));
                continue;
            }
            for series in chart.series.iter_mut().chain(&mut chart.threshold) {
                series.points = Vec::new();
            }
            chart.history = Some(Box::new(history));
        }
    }
    result
}
