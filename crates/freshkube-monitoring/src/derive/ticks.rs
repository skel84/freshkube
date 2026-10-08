//! Axis ticks: round values on the value axis and local clock times on the
//! time axis, derived per answer. The time axis keeps one tick set per
//! interval; paint picks the finest whose labels fit the plot's width.
use chrono::{Local, Offset, TimeZone};
use gpui_kit::SharedString;

/// A labelled position along an axis, 0 at the start (bottom or left) and
/// 1 at the end.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Tick {
    pub at: f32,
    pub label: SharedString,
}

/// Time ticks at one interval.
#[derive(Clone, Debug)]
pub(crate) struct TickSet {
    /// The distance between ticks, as a fraction of the axis.
    pub spacing: f32,
    pub ticks: Vec<Tick>,
}

/// Seconds between time ticks, finest first.
const INTERVALS: [i64; 16] = [
    10, 30, 60, 300, 600, 900, 1800, 3600, 7200, 10_800, 21_600, 43_200, 86_400, 172_800, 604_800,
    2_592_000,
];

/// The first tick set whose ticks stand at least `gap` apart on an axis
/// `width` long, else the coarsest.
pub(crate) fn fitting(sets: &[TickSet], width: f32, gap: f32) -> Option<&TickSet> {
    sets.iter()
        .find(|set| set.spacing * width >= gap)
        .or_else(|| sets.last())
}

/// Tick sets for the window from `start` to `end`, in Unix seconds, on the
/// local clock.
pub(crate) fn time_ticks(start: f64, end: f64) -> Vec<TickSet> {
    let span = end - start;
    if span.is_nan() || span <= 0. {
        return Vec::new();
    }
    let offset = Local
        .timestamp_opt(end as i64, 0)
        .single()
        .map_or(0, |time| time.offset().fix().local_minus_utc() as i64);
    INTERVALS
        .iter()
        .filter(|interval| {
            let count = span / **interval as f64;
            (1.5..=40.).contains(&count)
        })
        .map(|&interval| {
            let first = ((start as i64 + offset) as f64 / interval as f64).ceil() as i64 * interval
                - offset;
            let ticks = (0..)
                .map(|n| first + n * interval)
                .take_while(|&time| time as f64 <= end)
                .map(|time| Tick {
                    at: ((time as f64 - start) / span) as f32,
                    label: clock_label(time, interval, span).into(),
                })
                .collect();
            TickSet {
                spacing: (interval as f64 / span) as f32,
                ticks,
            }
        })
        .collect()
}

fn clock_label(time: i64, interval: i64, span: f64) -> String {
    let Some(local) = Local.timestamp_opt(time, 0).single() else {
        return String::new();
    };
    let format = if interval >= 86_400 {
        "%m/%d"
    } else if span > 2. * 86_400. {
        "%m/%d %H:%M"
    } else if interval < 60 {
        "%H:%M:%S"
    } else {
        "%H:%M"
    };
    local.format(format).to_string()
}

/// A round step near `range / intervals`: 1, 2, 2.5 or 5 times a power of
/// ten.
pub(crate) fn nice_step(range: f64, intervals: f64) -> f64 {
    let raw = range / intervals;
    if !raw.is_finite() || raw <= 0. {
        return 1.;
    }
    let magnitude = 10f64.powf(raw.log10().floor());
    let normal = raw / magnitude;
    let nice = if normal <= 1. {
        1.
    } else if normal <= 2. {
        2.
    } else if normal <= 2.5 {
        2.5
    } else if normal <= 5. {
        5.
    } else {
        10.
    };
    nice * magnitude
}

/// A linear axis around `low..high`: rounded out to whole steps unless the
/// dashboard fixes an end, and the values to label. An axis of `whole`
/// numbers, such as counts, labels only whole numbers.
pub(crate) fn linear(
    low: f64,
    high: f64,
    fixed_min: Option<f64>,
    fixed_max: Option<f64>,
    whole: bool,
) -> (f64, f64, Vec<f64>) {
    let (mut low, mut high) = (fixed_min.unwrap_or(low), fixed_max.unwrap_or(high));
    if high <= low {
        high = low + low.abs().max(1.);
    }
    let step = nice_step(high - low, 4.);
    // 1 for a step under 1, and 2 for 2.5; larger round steps are whole.
    let step = if whole { step.max(1.).floor() } else { step };
    if fixed_min.is_none() {
        low = (low / step).floor() * step;
    }
    if fixed_max.is_none() {
        high = (high / step).ceil() * step;
    }
    let first = (low / step).ceil();
    let values = (0..)
        .map(|n| (first + n as f64) * step)
        .take_while(|value| *value <= high + step * 1e-9)
        .take(12)
        // `-0` labels as "-0".
        .map(|value| if value == 0. { 0. } else { value })
        .collect();
    (low, high, values)
}

/// A log axis around `low..high` (both positive): whole powers of `base`.
pub(crate) fn log(base: f64, low: f64, high: f64) -> (f64, f64, Vec<f64>) {
    let first = low.log(base).floor();
    let last = high.log(base).ceil().max(first + 1.);
    let stride = ((last - first) / 5.).ceil().max(1.);
    let values = (0..)
        .map(|n| first + n as f64 * stride)
        .take_while(|exponent| *exponent <= last)
        .map(|exponent| base.powf(exponent))
        .collect();
    (base.powf(first), base.powf(last), values)
}
