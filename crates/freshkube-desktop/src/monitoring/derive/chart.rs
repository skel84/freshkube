//! A timeseries panel's display data: normalized series, axes, thresholds
//! and the legend.
use std::rc::Rc;

use chrono::{Local, TimeZone};
use freshkube_core::monitoring::model::{
    chart::{self, StackMode, Stacking},
    spec::{
        AxisPlacement, AxisScale, Calc, Curve, DrawStyle, FieldSpec, LineStyle, ThresholdStyle,
        TimeSeriesOptions,
    },
    time::TimeWindow,
};
use gpui_kit::SharedString;

use super::ticks::{self, Tick, TickSet};
use super::{Body, PanelData, Shown, format};
use crate::monitoring::colors::{self, Ink, Tier};

/// Legend rows drawn at most; the rest are counted.
pub(crate) const LEGEND_ROWS: usize = 30;

/// Opacity of an unstacked area, and of stacked bands.
const AREA: f32 = colors::AREA_OPACITY;
const STACKED_AREA: f32 = 0.35;

pub(crate) struct Chart {
    /// Each sample's place across the window, 0 to 1.
    pub xs: Vec<f32>,
    /// Each sample's time, in Unix seconds.
    pub times: Vec<f64>,
    /// The window's first and last time, in Unix seconds.
    pub start: f64,
    pub end: f64,
    pub series: Vec<ChartSeries>,
    /// The left and right value axes.
    pub axes: [Option<Axis>; 2],
    pub time_ticks: Vec<TickSet>,
    pub thresholds: Vec<Threshold>,
    pub bands: Vec<Band>,
    pub curve: Curve,
    pub legend: Legend,
}

#[derive(Clone, Debug)]
pub(crate) struct Axis {
    pub min: f64,
    pub max: f64,
    pub scale: AxisScale,
    pub ticks: Vec<Tick>,
}

impl Axis {
    /// Where `value` sits, 0 at the bottom and 1 at the top; NaN when it
    /// can't be drawn.
    pub(crate) fn project(&self, value: f64) -> f32 {
        let (low, high) = (self.scale.project(self.min), self.scale.project(self.max));
        let value = self.scale.project(value);
        if !value.is_finite() || high <= low {
            return f32::NAN;
        }
        ((value - low) / (high - low)) as f32
    }
}

pub(crate) struct ChartSeries {
    pub name: SharedString,
    pub ink: Ink,
    pub draw: DrawStyle,
    /// The line, per sample, 0 to 1 on its axis; NaN at a gap.
    pub tops: Vec<f32>,
    /// A stacked series' lower edge.
    pub bases: Option<Vec<f32>>,
    /// Where an unstacked area closes.
    pub baseline: f32,
    /// The area's opacity, 0 for none.
    pub fill: f32,
    pub dashes: Option<Vec<f32>>,
    pub points: bool,
    /// The values as answered, for the cursor.
    pub values: Rc<[f64]>,
    pub field: Rc<FieldSpec>,
}

/// A dashed threshold line.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Threshold {
    pub at: f32,
    pub right: bool,
    pub tier: Tier,
    pub label: SharedString,
}

/// A muted band over a range a threshold marks.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Band {
    pub from: f32,
    pub to: f32,
    pub right: bool,
    pub tier: Tier,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LegendMode {
    Hidden,
    /// Swatch, name and last value in one wrapping line.
    Inline,
    /// Two columns of rows with a value per heading.
    Table,
}

#[derive(Clone, Debug)]
pub(crate) struct Legend {
    pub mode: LegendMode,
    pub headings: Vec<SharedString>,
    pub rows: Vec<LegendRow>,
    /// Series left out past [`LEGEND_ROWS`].
    pub more: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct LegendRow {
    pub series: usize,
    pub name: SharedString,
    pub values: Vec<SharedString>,
    /// When the last value is older than the window's end, its time.
    pub stale: Option<SharedString>,
}

pub(super) fn chart(
    times: &[f64],
    shown: Vec<Shown>,
    options: &TimeSeriesOptions,
    window: TimeWindow,
) -> PanelData {
    let end = window.end as f64;
    let start = end - window.span as f64;
    let span = (end - start).max(1.);
    let xs: Vec<f32> = times
        .iter()
        .map(|time| ((time - start) / span) as f32)
        .collect();

    let left_unit = shown
        .iter()
        .find(|s| s.field.axis != AxisPlacement::Right)
        .and_then(|s| s.field.unit.clone());
    let right: Vec<bool> = shown
        .iter()
        .map(|s| {
            s.field.axis == AxisPlacement::Right
                || (s.field.axis == AxisPlacement::Auto && s.field.unit != left_unit)
        })
        .collect();
    // With nothing on the left, the right axis moves there.
    let right: Vec<bool> = if right.iter().all(|r| *r) {
        vec![false; right.len()]
    } else {
        right
    };
    let stackings: Vec<Stacking> = shown
        .iter()
        .map(|s| {
            s.style
                .stacking
                .clone()
                .unwrap_or_else(|| options.stacking.clone())
        })
        .collect();
    let percent = [false, true].map(|side| {
        stackings
            .iter()
            .zip(&right)
            .any(|(stacking, r)| *r == side && stacking.mode == StackMode::Percent)
    });

    // Stack sample by sample; negative-Y flips only what is drawn.
    let mut tops = vec![Vec::with_capacity(times.len()); shown.len()];
    let mut bases = vec![Vec::with_capacity(times.len()); shown.len()];
    for index in 0..times.len() {
        let drawn: Vec<f64> = shown
            .iter()
            .map(|s| {
                let value = s.values.get(index).copied().unwrap_or(f64::NAN);
                if s.style.negative_y { -value } else { value }
            })
            .collect();
        let (top, base) = chart::stack(&drawn, &stackings, &right);
        for (i, (top, base)) in top.into_iter().zip(base).enumerate() {
            tops[i].push(top);
            bases[i].push(base);
        }
    }

    let axes = [false, true].map(|side| {
        axis(
            &shown,
            &right,
            &tops,
            &bases,
            &stackings,
            side,
            percent[usize::from(side)],
        )
    });

    let inks = colors::inks(
        &shown
            .iter()
            .map(|s| (s.name.as_str(), s.labels.as_slice()))
            .collect::<Vec<_>>(),
    );
    let series: Vec<ChartSeries> = shown
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let axis = axes[usize::from(right[i])]
                .as_ref()
                .expect("an axis for every side with series");
            let stacked = stackings[i].mode != StackMode::None;
            let draw = s.style.draw.unwrap_or(options.draw);
            let opacity = s.style.fill_opacity.unwrap_or(options.fill_opacity);
            let pattern = s.style.line_style.as_ref().unwrap_or(&options.line_style);
            ChartSeries {
                name: s.name.clone().into(),
                ink: inks[i],
                draw,
                tops: tops[i].iter().map(|v| axis.project(*v)).collect(),
                bases: stacked.then(|| bases[i].iter().map(|v| axis.project(*v)).collect()),
                baseline: axis
                    .project(if axis.scale == AxisScale::Linear {
                        0.
                    } else {
                        axis.min
                    })
                    .clamp(0., 1.),
                fill: match (opacity > 0., stacked) {
                    (false, _) => 0.,
                    (true, false) => AREA,
                    (true, true) => STACKED_AREA,
                },
                dashes: match pattern {
                    LineStyle::Solid => None,
                    LineStyle::Dashed(dashes) => Some(dashes.clone()),
                },
                points: s.style.show_points.unwrap_or(options.show_points)
                    || draw == DrawStyle::Points,
                values: s.values.clone().into(),
                field: Rc::new(s.field.clone()),
            }
        })
        .collect();

    let (thresholds, bands) = thresholds(&shown, &right, options, &axes);
    let mut legend = legend(&shown, options, times);
    let mut axes = axes;
    let unit = take_unit(&mut axes, &mut legend);
    PanelData {
        body: Body::Chart(Rc::new(Chart {
            xs,
            times: times.to_vec(),
            start,
            end,
            series,
            axes,
            time_ticks: ticks::time_ticks(start, end),
            thresholds,
            bands,
            curve: options.curve,
            legend,
        })),
        unit,
    }
}

/// The axis for one side, or `None` when no series uses it.
fn axis(
    shown: &[Shown],
    right: &[bool],
    tops: &[Vec<f64>],
    bases: &[Vec<f64>],
    stackings: &[Stacking],
    side: bool,
    percent: bool,
) -> Option<Axis> {
    let members: Vec<usize> = (0..shown.len()).filter(|i| right[*i] == side).collect();
    let field = &shown[*members.first()?].field;
    let scale = if percent {
        AxisScale::Linear
    } else {
        field.scale
    };
    let values = members.iter().flat_map(|&i| {
        let stacked = stackings[i].mode != StackMode::None;
        tops[i]
            .iter()
            .chain(stacked.then_some(&bases[i]).into_iter().flatten())
            .copied()
    });
    let values: Vec<f64> = values
        .filter(|v| v.is_finite() && (scale == AxisScale::Linear || *v > 0.))
        .collect();
    let low = values.iter().copied().reduce(f64::min);
    let high = values.iter().copied().reduce(f64::max);
    let (min, max, marks) = if percent {
        let low = if low.unwrap_or(0.) < 0. { -100. } else { 0. };
        let high = if high.unwrap_or(0.) > 0. { 100. } else { 0. };
        ticks::linear(low, high, Some(low), Some(high))
    } else if let AxisScale::Log(base) = scale {
        let low = field.min.filter(|v| *v > 0.).or(low).unwrap_or(1.);
        let high = field
            .max
            .filter(|v| *v > low)
            .or(high)
            .unwrap_or(low * base);
        ticks::log(base, low, high)
    } else {
        // Data that never goes below zero starts at zero.
        let low = low.map_or(0., |low| low.min(0.));
        let high = high.unwrap_or(1.);
        ticks::linear(low, high, field.min, field.max)
    };
    let mut axis = Axis {
        min,
        max,
        scale,
        ticks: Vec::new(),
    };
    let label_field = if percent {
        FieldSpec {
            unit: Some("percent".into()),
            ..field.clone()
        }
    } else {
        field.clone()
    };
    axis.ticks = marks
        .into_iter()
        .map(|value| Tick {
            at: axis.project(value),
            label: format(&label_field, value).into(),
        })
        .filter(|tick| tick.at.is_finite())
        .collect();
    Some(axis)
}

/// Threshold lines and bands from the fields' steps and a legacy graph's
/// limits, in the Console's two status colours.
fn thresholds(
    shown: &[Shown],
    right: &[bool],
    options: &TimeSeriesOptions,
    axes: &[Option<Axis>; 2],
) -> (Vec<Threshold>, Vec<Band>) {
    let mut lines: Vec<Threshold> = Vec::new();
    let mut bands: Vec<Band> = Vec::new();
    let mut seen: Vec<(bool, Vec<u64>)> = Vec::new();
    for (i, series) in shown.iter().enumerate() {
        let field = &series.field;
        if field.threshold_style == ThresholdStyle::Off {
            continue;
        }
        let side = right[i];
        let Some(axis) = &axes[usize::from(side)] else {
            continue;
        };
        let key = (
            side,
            field.steps.iter().map(|s| s.value.to_bits()).collect(),
        );
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let convert = |value: f64| {
            if field.percent_steps && value.is_finite() {
                let (min, max) = (field.min.unwrap_or(axis.min), field.max.unwrap_or(axis.max));
                min + (max - min) * value / 100.
            } else {
                value
            }
        };
        let tiers = colors::step_tiers(&field.steps);
        let area = matches!(
            field.threshold_style,
            ThresholdStyle::Area | ThresholdStyle::LineAndArea | ThresholdStyle::DashedAndArea
        );
        let line = field.threshold_style != ThresholdStyle::Area;
        for (index, step) in field.steps.iter().enumerate() {
            let from = convert(step.value);
            let to = field
                .steps
                .get(index + 1)
                .map_or(f64::INFINITY, |next| convert(next.value));
            if area && let Some(tier) = tiers[index] {
                push_band(&mut bands, axis, from, to, side, tier);
            }
            if line && index > 0 {
                let worse = tiers[index].max(tiers[index - 1]);
                if let Some(tier) = worse {
                    push_line(&mut lines, axis, field, from, side, tier);
                }
            }
        }
    }
    let legacy = &options.thresholds;
    let by_position = colors::tiers(legacy.len());
    for (index, limit) in legacy.iter().enumerate() {
        let Some(axis) = &axes[usize::from(limit.right_axis)] else {
            continue;
        };
        let meaning = limit.line.or(limit.fill).and_then(colors::meaning);
        let Some(tier) = meaning.unwrap_or(Some(by_position[index])) else {
            continue;
        };
        let field = &shown[0].field;
        if limit.line.is_some() {
            push_line(&mut lines, axis, field, limit.value, limit.right_axis, tier);
        }
        if limit.fill.is_some() {
            push_band(
                &mut bands,
                axis,
                limit.value,
                limit.end,
                limit.right_axis,
                tier,
            );
        }
    }
    (lines, bands)
}

fn push_line(
    lines: &mut Vec<Threshold>,
    axis: &Axis,
    field: &FieldSpec,
    value: f64,
    right: bool,
    tier: Tier,
) {
    let at = axis.project(value);
    if !(0. ..=1.).contains(&at) {
        return;
    }
    let line = Threshold {
        at,
        right,
        tier,
        label: format!("{} threshold", format(field, value)).into(),
    };
    if !lines.contains(&line) {
        lines.push(line);
    }
}

fn push_band(bands: &mut Vec<Band>, axis: &Axis, from: f64, to: f64, right: bool, tier: Tier) {
    let clamp = |value: f64| {
        if value == f64::NEG_INFINITY {
            0.
        } else if value == f64::INFINITY {
            1.
        } else {
            axis.project(value).clamp(0., 1.)
        }
    };
    let (from, to) = (clamp(from), clamp(to));
    if from.is_nan() || to.is_nan() || from == to {
        return;
    }
    let band = Band {
        from: from.min(to),
        to: from.max(to),
        right,
        tier,
    };
    if !bands.contains(&band) {
        bands.push(band);
    }
}

fn legend(shown: &[Shown], options: &TimeSeriesOptions, times: &[f64]) -> Legend {
    let listed: Vec<usize> = (0..shown.len())
        .filter(|i| !shown[*i].style.hidden_in_legend)
        .collect();
    let (mode, calcs): (LegendMode, Vec<Calc>) = if !options.legend || listed.is_empty() {
        (LegendMode::Hidden, Vec::new())
    } else if !options.legend_calcs.is_empty() {
        (LegendMode::Table, options.legend_calcs.clone())
    } else if listed.len() <= 4 {
        (LegendMode::Inline, vec![Calc::LastNotNull])
    } else {
        (LegendMode::Table, vec![Calc::LastNotNull, Calc::Max])
    };
    let headings = calcs
        .iter()
        .map(|calc| match calc {
            Calc::Last | Calc::LastNotNull => "last".into(),
            calc => calc.label().to_lowercase().into(),
        })
        .collect();
    let rows = listed
        .iter()
        .take(LEGEND_ROWS)
        .map(|&i| {
            let series = &shown[i];
            let values = calcs
                .iter()
                .map(|calc| format(&series.field, calc.reduce(&series.values)).into())
                .collect();
            LegendRow {
                series: i,
                name: series.name.clone().into(),
                values,
                stale: (calcs.first() == Some(&Calc::LastNotNull))
                    .then(|| stale(&series.values, times))
                    .flatten(),
            }
        })
        .collect();
    Legend {
        mode,
        headings,
        rows,
        more: listed.len().saturating_sub(LEGEND_ROWS),
    }
}

/// The time of the last value, when the series stopped before the end.
fn stale(values: &[f64], times: &[f64]) -> Option<SharedString> {
    if values.last().is_none_or(|value| value.is_finite()) {
        return None;
    }
    let index = values.iter().rposition(|value| value.is_finite())?;
    Some(clock(*times.get(index)?).into())
}

/// Moves a unit every left-axis label shares, such as "ms", from the labels
/// and the legend to the title, as the mock's latency panel shows it. A zero
/// takes no part: the formatter may name it in another unit ("0 s").
fn take_unit(axes: &mut [Option<Axis>; 2], legend: &mut Legend) -> Option<SharedString> {
    if axes[1].is_some() {
        return None;
    }
    let axis = axes[0].as_mut()?;
    let unit = common_unit(axis.ticks.iter().map(|tick| tick.label.as_ref()))?;
    let strip = |label: &str| -> Option<String> {
        if zero(label) {
            return Some("0".to_owned());
        }
        label
            .strip_suffix(unit.as_str())
            .and_then(|number| number.strip_suffix(' '))
            .map(str::to_owned)
    };
    for tick in &mut axis.ticks {
        if let Some(number) = strip(&tick.label) {
            tick.label = number.into();
        }
    }
    for row in &mut legend.rows {
        for value in &mut row.values {
            if let Some(number) = strip(value) {
                *value = number.into();
            }
        }
    }
    Some(unit.into())
}

/// A label's number and the word after it, as "250 ms".
fn split_unit(label: &str) -> Option<(&str, &str)> {
    let (number, unit) = label.rsplit_once(' ')?;
    let numeric = number
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | ','));
    (numeric && !number.is_empty() && !unit.is_empty()).then_some((number, unit))
}

/// Whether a label is zero, in whatever unit.
fn zero(label: &str) -> bool {
    let number = split_unit(label).map_or(label, |(number, _)| number);
    number.parse::<f64>().is_ok_and(|value| value == 0.)
}

/// The word after the number that every label but a zero ends with, if
/// they agree and at least one has it.
pub(super) fn common_unit<'a>(labels: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut unit = None;
    for label in labels.filter(|label| !zero(label)) {
        let (_, this) = split_unit(label)?;
        if unit.is_some_and(|unit| unit != this) {
            return None;
        }
        unit = Some(this);
    }
    unit.map(str::to_owned)
}

/// A sample time on the local clock, for the cursor and stale values.
pub(crate) fn clock(time: f64) -> String {
    Local
        .timestamp_opt(time as i64, 0)
        .single()
        .map(|time| time.format("%H:%M").to_string())
        .unwrap_or_default()
}
