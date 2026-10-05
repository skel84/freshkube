//! Panels that reduce each series to one value: stats (a Grafana gauge is a
//! stat with a bar), bar lists (bar gauge, pie and bar chart), and tables.
use std::collections::HashMap;
use std::rc::Rc;

use chrono::{Local, TimeZone};
use freshkube_core::monitoring::model::{
    PanelSpec, Viz,
    data::Frame,
    spec::{Calc, FieldSpec, StatColor},
    transform::{self, Cell},
};
use gpui_kit::SharedString;

use super::{Body, Shown, display, format};
use crate::monitoring::colors::{self, Tier};

/// Stats drawn at most in one panel.
pub(crate) const MAX_STATS: usize = 24;
/// Bar rows drawn at most.
pub(crate) const MAX_BARS: usize = 50;
/// Table rows kept at most, a hard cap; the rest are counted.
pub(crate) const MAX_ROWS: usize = 1_000;
/// Table rows shown until Show all.
pub(crate) const FOLDED_ROWS: usize = 100;

#[derive(Clone, Debug)]
pub(crate) struct Stat {
    /// The series' name, when the panel shows several.
    pub name: Option<SharedString>,
    pub value: SharedString,
    pub tier: Option<Tier>,
    /// Why the value carries a status, such as "above 90%".
    pub note: Option<SharedString>,
    pub spark: Option<Spark>,
    /// A gauge's fill, 0 to 1.
    pub gauge: Option<f32>,
}

/// A stat's sparkline: grey history and a last stretch in the stat's tone.
#[derive(Clone, Debug)]
pub(crate) struct Spark {
    /// Per sample, 0 to 1 across the series' own range; NaN at a gap.
    pub ys: Vec<f32>,
    /// The first sample of the last stretch.
    pub tail: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct BarRow {
    /// A leading `namespace/`, drawn muted.
    pub prefix: Option<SharedString>,
    pub name: SharedString,
    pub value: SharedString,
    /// The bar's fill, 0 to 1.
    pub fraction: f32,
    pub tier: Option<Tier>,
    /// The series had no value to show.
    pub missing: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TableData {
    pub columns: Vec<Column>,
    /// At most [`MAX_ROWS`], in the answer's order.
    pub rows: Vec<TableRow>,
    /// Rows in the answer, kept or not.
    pub total: usize,
    /// Whether a column names each row's severity, so the table leads with
    /// its glyph.
    pub severity: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Column {
    pub name: SharedString,
    /// Right-aligned in IBM Plex Mono.
    pub numeric: bool,
    /// The most characters a cell or the name has.
    pub chars: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct TableRow {
    pub key: RowKey,
    pub cells: Vec<SharedString>,
    /// From the severity column: only critical or error and warning or warn
    /// carry one.
    pub tier: Option<Tier>,
}

/// A table row's identity: its label cells, and which of the rows with
/// the same labels it is, in the answer's order. Numbers and times change
/// with every answer, so they don't count.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RowKey {
    pub labels: Rc<[SharedString]>,
    pub occurrence: usize,
}

pub(super) fn summary(viz: &Viz, shown: &[Shown]) -> Body {
    match viz {
        Viz::Stat(options) => {
            let status = options.color != StatColor::None;
            Body::Stats(stats(shown, options.calc, status, options.sparkline, false))
        }
        Viz::Gauge(options) => Body::Stats(stats(shown, options.calc, true, false, true)),
        Viz::BarGauge(options) => Body::Bars(bars(shown, options.calc, Share::Range, true)),
        Viz::Pie(options) => Body::Bars(bars(shown, options.calc, Share::Total, false)),
        _ => Body::Bars(bars(shown, Calc::LastNotNull, Share::Peak, false)),
    }
}

fn stats(shown: &[Shown], calc: Calc, status: bool, spark: bool, gauge: bool) -> Rc<[Stat]> {
    let named = shown.len() > 1;
    shown
        .iter()
        .take(MAX_STATS)
        .map(|series| {
            let value = calc.reduce(&series.values);
            let range = range(&series.field, &series.values);
            let (tier, note) = if status {
                status_of(&series.field, value, range)
            } else {
                (None, None)
            };
            Stat {
                name: named.then(|| series.name.clone().into()),
                value: display(&series.field, value).into(),
                tier,
                note: note.map(Into::into),
                spark: spark.then(|| sparkline(&series.values)).flatten(),
                gauge: gauge.then(|| fraction(value, range)),
            }
        })
        .collect()
}

/// A stat's samples on their own range, with the last sixth as the tail.
fn sparkline(values: &[f64]) -> Option<Spark> {
    let finite = values.iter().copied().filter(|v| v.is_finite());
    let low = finite.clone().reduce(f64::min)?;
    let high = finite.reduce(f64::max)?;
    if values.len() < 2 {
        return None;
    }
    let span = high - low;
    let ys = values
        .iter()
        .map(|value| {
            if !value.is_finite() {
                f32::NAN
            } else if span > 0. {
                ((value - low) / span) as f32
            } else {
                0.5
            }
        })
        .collect();
    Some(Spark {
        ys,
        tail: values.len() - values.len().div_ceil(6).max(2).min(values.len()),
    })
}

/// The range a gauge or bar fills: the dashboard's, else the unit's, else
/// zero to the largest value.
fn range(field: &FieldSpec, values: &[f64]) -> (f64, f64) {
    let (unit_min, unit_max) = match field.unit.as_deref() {
        Some("percent") => (0., 100.),
        Some("percentunit") => (0., 1.),
        _ => (
            0.,
            values
                .iter()
                .copied()
                .filter(|v| v.is_finite())
                .fold(0., f64::max)
                .max(f64::MIN_POSITIVE),
        ),
    };
    let min = field.min.unwrap_or(unit_min);
    let max = field.max.unwrap_or(unit_max);
    (min, if max > min { max } else { min + 1. })
}

fn fraction(value: f64, (min, max): (f64, f64)) -> f32 {
    if value.is_finite() {
        ((value - min) / (max - min)).clamp(0., 1.) as f32
    } else {
        0.
    }
}

/// The status a value's threshold step stands for, and the step it passed.
pub(super) fn status_of(
    field: &FieldSpec,
    value: f64,
    (min, max): (f64, f64),
) -> (Option<Tier>, Option<String>) {
    if !value.is_finite() || field.steps.is_empty() {
        return (None, None);
    }
    let convert = |step: f64| {
        if field.percent_steps && step.is_finite() {
            min + (max - min) * step / 100.
        } else {
            step
        }
    };
    let index = field
        .steps
        .iter()
        .rposition(|step| value >= convert(step.value))
        .unwrap_or(0);
    let tiers = colors::step_tiers(&field.steps);
    let Some(tier) = tiers[index] else {
        return (None, None);
    };
    let note = if index > 0 {
        format!("above {}", format(field, convert(field.steps[index].value)))
    } else if let Some(next) = field.steps.get(1) {
        format!("below {}", format(field, convert(next.value)))
    } else {
        return (Some(tier), None);
    };
    (Some(tier), Some(note))
}

/// What a bar's length measures.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Share {
    /// The gauge range.
    Range,
    /// Its part of the total, as a pie.
    Total,
    /// Against the largest value.
    Peak,
}

fn bars(shown: &[Shown], calc: Calc, share: Share, status: bool) -> Rc<[BarRow]> {
    let values: Vec<f64> = shown.iter().map(|s| calc.reduce(&s.values)).collect();
    let finite = values.iter().copied().filter(|v| v.is_finite());
    let total: f64 = finite.clone().map(f64::abs).sum();
    let peak = finite.fold(0., |peak: f64, v| peak.max(v.abs()));
    shown
        .iter()
        .zip(values)
        .take(MAX_BARS)
        .map(|(series, value)| {
            let range = range(&series.field, &series.values);
            let fraction = match share {
                Share::Range => fraction(value, range),
                Share::Total if total > 0. => fraction(value.abs(), (0., total)),
                Share::Peak if peak > 0. => fraction(value.abs(), (0., peak)),
                _ => 0.,
            };
            let (prefix, name) = split_prefix(&series.name);
            BarRow {
                prefix: prefix.map(|p| p.to_owned().into()),
                name: name.to_owned().into(),
                value: display(&series.field, value).into(),
                fraction,
                tier: if status {
                    status_of(&series.field, value, range).0
                } else {
                    None
                },
                missing: !value.is_finite(),
            }
        })
        .collect()
}

/// A leading `namespace/` of a name such as `payments/api-6c4f8`.
pub(super) fn split_prefix(name: &str) -> (Option<&str>, &str) {
    match name.split_once('/') {
        Some((prefix, rest))
            if !prefix.is_empty() && !rest.is_empty() && !name.contains(char::is_whitespace) =>
        {
            (Some(&name[..=prefix.len()]), rest)
        }
        _ => (None, name),
    }
}

pub(super) fn table(spec: &PanelSpec, frame: &Frame) -> Body {
    let queries: Vec<&str> = spec.queries.iter().map(|q| q.ref_id.as_str()).collect();
    let table = transform::table(&spec.transforms, &frame.series, &queries);
    if table.rows.is_empty() {
        return Body::NoData;
    }
    let columns: Vec<(usize, FieldSpec, bool)> = (0..table.columns.len())
        .filter_map(|index| {
            let values: Vec<f64> = table
                .rows
                .iter()
                .filter_map(|row| match row.get(index) {
                    Some(Cell::Number(value)) => Some(*value),
                    _ => None,
                })
                .collect();
            let numeric = !values.is_empty();
            let mut context = table.field_context(index);
            if numeric {
                context = context.values(&values);
            }
            if spec.field.style_for_field(&context).hidden {
                return None;
            }
            let time = table.sources[index] == "Time";
            Some((index, spec.field.for_field(&context).into_owned(), time))
        })
        .collect();
    let mut seen: HashMap<Rc<[SharedString]>, usize> = HashMap::new();
    let mut rows: Vec<TableRow> = table
        .rows
        .iter()
        .take(MAX_ROWS)
        .map(|row| {
            let cells: Vec<SharedString> = columns
                .iter()
                .map(|(index, field, time)| match row.get(*index) {
                    Some(Cell::Number(value)) if *time => date_time(*value).into(),
                    Some(Cell::Number(value)) => display(field, *value).into(),
                    Some(Cell::Text(text)) => text.clone().into(),
                    None => SharedString::default(),
                })
                .collect();
            let labels: Rc<[SharedString]> = columns
                .iter()
                .zip(&cells)
                .filter(|((index, _, _), _)| matches!(row.get(*index), Some(Cell::Text(_))))
                .map(|(_, cell)| cell.clone())
                .collect();
            let count = seen.entry(labels.clone()).or_default();
            let occurrence = *count;
            *count += 1;
            TableRow {
                key: RowKey { labels, occurrence },
                cells,
                tier: None,
            }
        })
        .collect();
    let columns: Vec<Column> = columns
        .iter()
        .enumerate()
        .map(|(position, (index, _, time))| {
            let name: SharedString = table.columns[*index].clone().into();
            let widest = rows
                .iter()
                .map(|row| row.cells[position].chars().count())
                .max()
                .unwrap_or(0);
            Column {
                chars: widest.max(name.chars().count()),
                name,
                numeric: !time
                    && table
                        .rows
                        .iter()
                        .any(|row| matches!(row.get(*index), Some(Cell::Number(_)))),
            }
        })
        .collect();
    let severity = columns
        .iter()
        .position(|column| !column.numeric && column.name.eq_ignore_ascii_case("severity"));
    if let Some(at) = severity {
        for row in &mut rows {
            row.tier = severity_tier(&row.cells[at]);
        }
    }
    Body::Table(Rc::new(TableData {
        columns,
        rows,
        total: table.rows.len(),
        severity: severity.is_some(),
    }))
}

/// A severity's status: critical and error are critical, warning and warn
/// a warning, in any case. Anything else (info, none, debug, a name we
/// don't know) carries none, and never reads as good.
pub(crate) fn severity_tier(severity: &str) -> Option<Tier> {
    let severity = severity.trim();
    if ["critical", "error"]
        .iter()
        .any(|name| severity.eq_ignore_ascii_case(name))
    {
        Some(Tier::Crit)
    } else if ["warning", "warn"]
        .iter()
        .any(|name| severity.eq_ignore_ascii_case(name))
    {
        Some(Tier::Warn)
    } else {
        None
    }
}

/// A time cell on the local clock. Table times arrive in milliseconds.
fn date_time(value: f64) -> String {
    let seconds = if value.abs() > 1e11 {
        value / 1000.
    } else {
        value
    };
    Local
        .timestamp_opt(seconds as i64, 0)
        .single()
        .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}
