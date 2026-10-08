//! A timeseries with many series draws only the ones with the highest
//! peaks, with the rest a click away (docs/MONITORING.md, The series cap).
//! Every path costs the renderer vertex copies each frame (K24 in
//! docs/GPUI_FRICTION.md), and past a few dozen grey lines a chart shows
//! little more.

use freshkube_core::monitoring::model::{chart::StackMode, spec::TimeSeriesOptions};

use super::Shown;

/// The most series a capped chart draws.
pub(crate) const MOST_SERIES: usize = 30;

/// Whether a timeseries with more than [`MOST_SERIES`] series draws only
/// those with the highest peaks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SeriesCap {
    /// The highest peaks only, with a way to show all.
    #[default]
    Top,
    /// Every series, with a way back to the top ones.
    All,
    /// Every series, with no cap to offer: the query already picks them.
    Off,
}

/// A chart that has more series than the cap, and whether it draws them all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Capped {
    /// The series it could draw.
    pub of: usize,
    pub all: bool,
}

/// The series to draw, in their frame order, and whether the cap applies.
///
/// Only series the legend lists count and can be left out; one the
/// dashboard hides from the legend, usually a reference line, always draws.
/// A stacked chart is never capped, since a series left out would lower the
/// stack's total.
pub(super) fn cap(
    shown: Vec<Shown>,
    series: SeriesCap,
    options: &TimeSeriesOptions,
) -> (Vec<Shown>, Option<Capped>) {
    let listed = shown.iter().filter(|s| !s.style.hidden_in_legend).count();
    let stacked = shown
        .iter()
        .any(|s| s.style.stacking.as_ref().unwrap_or(&options.stacking).mode != StackMode::None);
    if series == SeriesCap::Off || listed <= MOST_SERIES || stacked {
        return (shown, None);
    }
    let capped = Capped {
        of: listed,
        all: series == SeriesCap::All,
    };
    if capped.all {
        return (shown, Some(capped));
    }
    let mut ranked: Vec<(usize, f64)> = shown
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.style.hidden_in_legend)
        .map(|(index, s)| (index, peak(&s.values)))
        .collect();
    // Stable, so a tie keeps frame order.
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut kept = vec![false; shown.len()];
    for (index, _) in ranked.into_iter().take(MOST_SERIES) {
        kept[index] = true;
    }
    let shown = shown
        .into_iter()
        .zip(kept)
        .filter(|(s, kept)| *kept || s.style.hidden_in_legend)
        .map(|(s, _)| s)
        .collect();
    (shown, Some(capped))
}

/// The value furthest from zero, so a series drawn below zero ranks by its
/// size; a series with no value ranks last.
fn peak(values: &[f64]) -> f64 {
    values
        .iter()
        .filter(|value| value.is_finite())
        .map(|value| value.abs())
        .fold(f64::NEG_INFINITY, f64::max)
}

/// Whether a query sent for the panel already picks its series with
/// `topk` or `bottomk`, so the chart draws what it was asked for.
pub(crate) fn picks_series<'a>(expressions: impl IntoIterator<Item = &'a str>) -> bool {
    expressions.into_iter().any(|expression| {
        let lower = expression.to_ascii_lowercase();
        ["topk", "bottomk"]
            .iter()
            .any(|word| whole_word(&lower, word))
    })
}

fn whole_word(text: &str, word: &str) -> bool {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == ':';
    text.match_indices(word)
        .any(|(at, _)| !text[..at].ends_with(ident) && !text[at + word.len()..].starts_with(ident))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_that_picks_its_series_is_named_by_whole_words() {
        assert!(picks_series(["topk(5, rate(x[5m]))"]));
        assert!(picks_series(["sum(x)", "BOTTOMK (3, y)"]));
        assert!(picks_series(["topk by (job) (5, x)"]));
        assert!(!picks_series([
            "sum(my_topk_total)",
            "x:topk:rate5m",
            "stopk(x)"
        ]));
        assert!(!picks_series([]));
    }

    #[test]
    fn a_peak_is_the_value_furthest_from_zero() {
        assert_eq!(peak(&[1., -7., f64::NAN, 3.]), 7.);
        assert_eq!(peak(&[f64::NAN]), f64::NEG_INFINITY);
    }
}
