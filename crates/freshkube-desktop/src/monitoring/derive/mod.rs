//! What a panel shows, derived once per answer and never in `render`: the
//! series' inks and normalized geometry, axis ticks, thresholds, legend
//! values, stat values, bar rows and table cells. Projection to pixels is
//! left to paint, which only scales these by the plot's size.
//!
//! The order of work follows grafaui-desktop's `panel::derive` (MIT, the
//! user's own code): transformations, then overrides that hide and rename
//! series, then stacking. Every colour the dashboard names is dropped here.
mod cap;
mod chart;
mod summary;
#[cfg(test)]
mod tests;
mod text;
mod ticks;

use std::rc::Rc;

use freshkube_core::monitoring::model::{
    PanelSpec, Viz,
    data::Frame,
    spec::{FieldContext, FieldSpec},
    time::TimeWindow,
    transform,
};
use gpui_kit::SharedString;

pub(crate) use cap::{Capped, MOST_SERIES, SeriesCap, picks_series};
pub(crate) use chart::{Axis, BarSlot, Chart, ChartSeries, LegendMode, LegendRow};
pub(crate) use summary::{BarRow, FOLDED_ROWS, RowKey, Stat, TableData, TableRow};
pub(crate) use ticks::fitting;

/// A panel's display data.
#[derive(Clone)]
pub(crate) struct PanelData {
    pub body: Body,
    /// A unit every axis label shared, shown muted after the title instead.
    pub unit: Option<SharedString>,
}

#[derive(Clone)]
pub(crate) enum Body {
    Chart(Rc<Chart>),
    Stats(Rc<[Stat]>),
    Bars(Rc<[BarRow]>),
    Table(Rc<TableData>),
    Text(SharedString),
    /// The answer held no values.
    NoData,
    /// A kind this app doesn't draw, by its dashboard name.
    NotDrawn(SharedString),
}

/// A series after transformations and overrides, with its own field.
pub(super) struct Shown {
    pub name: String,
    pub labels: Vec<(String, String)>,
    pub values: Vec<f64>,
    pub field: FieldSpec,
    pub style: freshkube_core::monitoring::model::overrides::SeriesStyle,
}

/// Derives what `spec` shows of `frame`, read over `window`, a timeseries
/// with as many series as `series` says.
pub(crate) fn derive(
    spec: &PanelSpec,
    frame: Frame,
    window: TimeWindow,
    series: SeriesCap,
) -> PanelData {
    crate::desktop::probe::hit("monitoring-derive");
    let plain = |body| PanelData { body, unit: None };
    match &spec.viz {
        Viz::Table => plain(summary::table(spec, &frame)),
        Viz::Text(options) => plain(Body::Text(
            text::readable(options.mode, &options.content).into(),
        )),
        Viz::TimeSeries(options) => {
            let shown = shown(spec, frame.clone(), |field, context| {
                field.for_time_series_field(context, options)
            });
            if no_values(&shown) {
                return plain(Body::NoData);
            }
            let (shown, capped) = cap::cap(shown, series, options);
            chart::chart(&frame.times, shown, options, window, capped)
        }
        Viz::Stat(_) | Viz::Gauge(_) | Viz::BarGauge(_) | Viz::Pie(_) | Viz::BarChart(_) => {
            let shown = shown(spec, frame, |field, context| {
                field.for_field(context).into_owned()
            });
            if no_values(&shown) {
                return plain(Body::NoData);
            }
            plain(summary::summary(&spec.viz, &shown))
        }
        Viz::Histogram | Viz::Heatmap(_) | Viz::Geomap(_) | Viz::Candlestick | Viz::Unsupported => {
            plain(Body::NotDrawn(spec.kind.clone().into()))
        }
    }
}

/// The text a panel kind we don't draw shows in its cell.
pub(crate) fn not_drawn(kind: &str) -> String {
    format!("Not drawn here: {kind}")
}

fn no_values(shown: &[Shown]) -> bool {
    shown
        .iter()
        .all(|series| series.values.iter().all(|value| !value.is_finite()))
}

/// The series a panel draws: transformed, without hidden ones, renamed by
/// their display-name override, each with its field after overrides.
fn shown(
    spec: &PanelSpec,
    frame: Frame,
    field_for: impl Fn(&FieldSpec, &FieldContext<'_>) -> FieldSpec,
) -> Vec<Shown> {
    let frame = transform::apply(&spec.transforms, frame);
    frame
        .series
        .into_iter()
        .filter_map(|series| {
            let context = FieldContext::new(&series.name)
                .query(&series.query)
                .values(&series.values);
            let style = spec.field.style_for_field(&context);
            if style.hidden {
                return None;
            }
            let field = field_for(&spec.field, &context);
            Some(Shown {
                name: style
                    .display_name
                    .clone()
                    .unwrap_or_else(|| series.name.clone()),
                labels: series.labels,
                values: series.values,
                field,
                style,
            })
        })
        .collect()
}

/// A value as a legend, stat or cell shows it; a missing one as a dash.
pub(super) fn display(field: &FieldSpec, value: f64) -> String {
    if value.is_finite() {
        field.display(value)
    } else {
        "–".into()
    }
}

/// A value as an axis or the cursor shows it.
pub(crate) fn format(field: &FieldSpec, value: f64) -> String {
    if value.is_finite() {
        field.format(value)
    } else {
        "–".into()
    }
}
