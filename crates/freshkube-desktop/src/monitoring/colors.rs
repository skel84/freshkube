//! The Fog chart palette (docs/MONITORING.md, Colours). Every colour a
//! dashboard asks for is ignored: palette modes, fixed and named colours,
//! overrides, continuous schemes and threshold colours. A series' colour
//! follows only from its position, or from its level when the series read
//! as quantiles or histogram buckets. A threshold step's colour gives only
//! its meaning: of several steps the highest is critical and the rest are
//! warnings, and a single step is a warning. The one exception is a Coroot
//! chart, whose series name their colour: it draws in the nearest Console
//! colour.
use freshkube_core::coroot::SeriesColor;
use freshkube_core::monitoring::model::{color::Rgba, spec::Step};
use gpui_kit::{Hsla, rgb};

/// The two series slots, in their fixed order.
pub(crate) const SLOTS: [u32; 2] = [0x5E93E6, 0xCC7C4A];
/// Ordered levels (p50, p95, p99; le buckets), lowest level darkest.
pub(crate) const RAMP: [u32; 3] = [0x5379BB, 0x7AA0E6, 0xB3CEFA];
/// Series past the second, until hovered or picked.
pub(crate) const OVERFLOW: u32 = 0x737A85;
/// Opacity of an area under its line.
pub(crate) const AREA_OPACITY: f32 = 0.14;
/// Opacity of the other series while one is hovered or picked.
pub(crate) const FADED_OPACITY: f32 = 0.15;

/// How a series is coloured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Ink {
    /// One of the two slots.
    Slot(usize),
    /// Past the second series: grey, its own slot only while focused.
    Overflow(usize),
    /// A level on the blue ramp, 0 (lowest, darkest) to 1 (highest).
    Level(f32),
    /// A Coroot series' own colour, as the nearest Console colour.
    Named(SeriesColor),
}

impl Ink {
    /// The colour drawn, given whether the series is hovered or picked.
    pub(crate) fn color(self, focused: bool) -> Hsla {
        match self {
            Ink::Slot(slot) => hex(SLOTS[slot % SLOTS.len()]),
            Ink::Overflow(slot) if focused => hex(SLOTS[slot % SLOTS.len()]),
            Ink::Overflow(_) => hex(OVERFLOW),
            Ink::Level(level) => ramp(level),
            Ink::Named(color) => hex(named(color)),
        }
    }
}

/// The Console colour drawn for a Coroot colour (docs/DESIGN.md, Tokens).
fn named(color: SeriesColor) -> u32 {
    match color {
        SeriesColor::Critical => 0xF28B82,
        SeriesColor::Warning => 0xF2C46D,
        SeriesColor::Ok => 0x82D4AB,
        SeriesColor::Blue => SLOTS[0],
        SeriesColor::Orange => SLOTS[1],
        SeriesColor::Purple => 0xB7AAF7,
        SeriesColor::Grey => OVERFLOW,
    }
}

fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

/// A colour along the ramp; its three stops at 0, ½ and 1.
pub(crate) fn ramp(level: f32) -> Hsla {
    let level = level.clamp(0., 1.) * (RAMP.len() - 1) as f32;
    let low = (level.floor() as usize).min(RAMP.len() - 2);
    let t = level - low as f32;
    let channel = |value: u32, shift: u32| ((value >> shift) & 0xff) as f32;
    let mix = |shift: u32| {
        let (a, b) = (channel(RAMP[low], shift), channel(RAMP[low + 1], shift));
        ((a + (b - a) * t).round() as u32) << shift
    };
    hex(mix(16) | mix(8) | mix(0))
}

/// Inks for series in frame order: a colour `named` gives the series by
/// its name; otherwise the ramp when every series reads as a level, else
/// the slots with grey past the second.
pub(crate) fn inks(
    series: &[(&str, &[(String, String)])],
    named: &[(String, SeriesColor)],
) -> Vec<Ink> {
    let ordered = levels(series).unwrap_or_else(|| {
        (0..series.len())
            .map(|index| {
                if index < SLOTS.len() {
                    Ink::Slot(index)
                } else {
                    Ink::Overflow(index)
                }
            })
            .collect()
    });
    series
        .iter()
        .zip(ordered)
        .map(|((name, _), ink)| {
            named
                .iter()
                .find(|(n, _)| n == name)
                .map_or(ink, |(_, color)| Ink::Named(*color))
        })
        .collect()
}

/// Each series' place among the levels, when all of them carry one.
fn levels(series: &[(&str, &[(String, String)])]) -> Option<Vec<Ink>> {
    if series.len() < 2 {
        return None;
    }
    let values: Vec<f64> = series
        .iter()
        .map(|(name, labels)| level(name, labels))
        .collect::<Option<_>>()?;
    let mut distinct = values.clone();
    distinct.sort_by(f64::total_cmp);
    distinct.dedup();
    if distinct.len() < 2 {
        return None;
    }
    let top = (distinct.len() - 1) as f32;
    Some(
        values
            .iter()
            .map(|value| {
                let rank = distinct.iter().position(|v| v == value).unwrap_or(0);
                Ink::Level(rank as f32 / top)
            })
            .collect(),
    )
}

/// The quantile or bucket a series stands for: a `quantile` or `le` label,
/// or a name such as `p99`, `P95 latency`, `0.99` or `99th percentile`.
pub(crate) fn level(name: &str, labels: &[(String, String)]) -> Option<f64> {
    for key in ["quantile", "le"] {
        if let Some((_, value)) = labels.iter().find(|(label, _)| label == key) {
            return match value.as_str() {
                "+Inf" | "Inf" => Some(f64::INFINITY),
                value => value.parse().ok(),
            };
        }
    }
    let name = name.trim();
    if let Ok(value) = name.parse::<f64>()
        && (0. ..=1.).contains(&value)
    {
        return Some(value);
    }
    name.split(|c: char| c.is_whitespace() || matches!(c, '-' | '_' | '(' | ')' | ','))
        .find_map(|word| {
            let lower = word.to_ascii_lowercase();
            let digits = lower
                .strip_prefix('p')
                .or_else(|| lower.strip_suffix("th"))
                .or_else(|| lower.strip_suffix('%'))?;
            let value: f64 = digits.parse().ok()?;
            (value > 0. && value <= 100.).then_some(value / 100.)
        })
}

/// The status colours a threshold may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Tier {
    Warn,
    Crit,
}

/// Tiers for `count` ascending thresholds, whatever colours they name: the
/// highest is critical when there are several, the rest are warnings.
pub(crate) fn tiers(count: usize) -> Vec<Tier> {
    (0..count)
        .map(|index| {
            if count > 1 && index == count - 1 {
                Tier::Crit
            } else {
                Tier::Warn
            }
        })
        .collect()
}

/// What a dashboard colour means, never how it looks: green, blue and grey
/// are fine, yellow and orange a warning, red critical. `None` for a colour
/// with no status meaning, such as purple.
pub(crate) fn meaning(color: Rgba) -> Option<Option<Tier>> {
    let [r, g, b] = [color.red(), color.green(), color.blue()].map(|c| c as f32 / 255.);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    if color.alpha() == 0 || max - min < 0.12 {
        return Some(None);
    }
    let delta = max - min;
    let hue = if max == r {
        60. * ((g - b) / delta).rem_euclid(6.)
    } else if max == g {
        60. * ((b - r) / delta + 2.)
    } else {
        60. * ((r - g) / delta + 4.)
    };
    match hue {
        h if !(15. ..345.).contains(&h) => Some(Some(Tier::Crit)),
        h if h < 70. => Some(Some(Tier::Warn)),
        h if h < 255. => Some(None),
        _ => None,
    }
}

/// The status each threshold step stands for, the base step included. A
/// step's colour gives only its meaning; one without a status meaning
/// counts by position, as [`tiers`] says.
pub(crate) fn step_tiers(steps: &[Step]) -> Vec<Option<Tier>> {
    let raised = steps.iter().filter(|step| step.value.is_finite()).count();
    let by_position = tiers(raised);
    let mut index = 0;
    steps
        .iter()
        .map(|step| {
            if !step.value.is_finite() {
                return meaning(step.color).flatten();
            }
            let tier = meaning(step.color).unwrap_or(Some(by_position[index]));
            index += 1;
            tier
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(names: &[&str]) -> Vec<Ink> {
        let series: Vec<(&str, &[(String, String)])> =
            names.iter().map(|name| (*name, &[][..])).collect();
        inks(&series, &[])
    }

    #[test]
    fn series_take_the_slots_in_order_then_grey() {
        let names: Vec<String> = (0..8).map(|n| format!("node-{n}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let inks = named(&names);
        assert_eq!(&inks[..2], &(0..2).map(Ink::Slot).collect::<Vec<_>>()[..]);
        assert_eq!(inks[6], Ink::Overflow(6));
        assert_eq!(inks[0].color(false), hex(0x5E93E6));
        assert_eq!(inks[5].color(false), hex(OVERFLOW));
        // Grey until focused, then its own slot (the first again).
        assert_eq!(inks[6].color(false), hex(OVERFLOW));
        assert_eq!(inks[6].color(true), hex(0x5E93E6));
        assert_eq!(inks[7].color(true), hex(0xCC7C4A));
    }

    #[test]
    fn quantiles_take_the_ramp_lowest_darkest() {
        let inks = named(&["p99", "p50", "p95"]);
        assert_eq!(inks, [Ink::Level(1.), Ink::Level(0.), Ink::Level(0.5)]);
        assert_eq!(inks[1].color(false), hex(0x5379BB));
        assert_eq!(inks[2].color(false), hex(0x7AA0E6));
        assert_eq!(inks[0].color(false), hex(0xB3CEFA));
        assert_eq!(
            named(&["P50 latency", "99th percentile"]),
            [Ink::Level(0.), Ink::Level(1.)]
        );
        assert_eq!(named(&["0.5", "0.9", "0.99"])[1], Ink::Level(0.5));
        // One unreadable name and the series are ordinary.
        assert_eq!(named(&["p50", "errors"]), [Ink::Slot(0), Ink::Slot(1)]);
        // A single quantile is just a series.
        assert_eq!(named(&["p99"]), [Ink::Slot(0)]);
    }

    #[test]
    fn buckets_and_quantile_labels_are_levels() {
        let label = |key: &str, value: &str| vec![(key.to_owned(), value.to_owned())];
        let (a, b, c) = (label("le", "0.1"), label("le", "+Inf"), label("le", "1"));
        let inks = inks(&[("x", &a[..]), ("y", &b[..]), ("z", &c[..])], &[]);
        assert_eq!(inks, [Ink::Level(0.), Ink::Level(1.), Ink::Level(0.5)]);
        let (a, b) = (label("quantile", "0.5"), label("quantile", "0.99"));
        assert_eq!(
            super::inks(&[("a", &a[..]), ("b", &b[..])], &[]),
            [Ink::Level(0.), Ink::Level(1.)]
        );
    }

    #[test]
    fn a_coroot_colour_wins_and_the_rest_keep_their_turn() {
        let series: Vec<(&str, &[(String, String)])> = ["error", "info", "debug", "other"]
            .iter()
            .map(|name| (*name, &[][..]))
            .collect();
        let named = [
            ("error".to_owned(), SeriesColor::Critical),
            ("info".to_owned(), SeriesColor::Blue),
            ("debug".to_owned(), SeriesColor::Ok),
        ];
        let inks = inks(&series, &named);
        assert_eq!(
            inks,
            [
                Ink::Named(SeriesColor::Critical),
                Ink::Named(SeriesColor::Blue),
                Ink::Named(SeriesColor::Ok),
                Ink::Overflow(3),
            ]
        );
        // Drawn in its colour whether focused or not, unlike grey.
        assert_eq!(inks[0].color(false), hex(0xF28B82));
        assert_eq!(inks[0].color(true), hex(0xF28B82));
        assert_eq!(Ink::Named(SeriesColor::Warning).color(false), hex(0xF2C46D));
        assert_eq!(inks[2].color(false), hex(0x82D4AB));
    }

    #[test]
    fn the_ramp_interpolates_between_its_stops() {
        assert_eq!(ramp(0.), hex(0x5379BB));
        assert_eq!(ramp(1.), hex(0xB3CEFA));
        assert_eq!(ramp(0.25), hex(0x678DD1));
    }

    #[test]
    fn thresholds_are_amber_then_red_whatever_they_say() {
        assert_eq!(tiers(1), [Tier::Warn]);
        assert_eq!(tiers(2), [Tier::Warn, Tier::Crit]);
        assert_eq!(tiers(3), [Tier::Warn, Tier::Warn, Tier::Crit]);
    }

    #[test]
    fn steps_keep_their_meaning_not_their_colour() {
        let step = |value: f64, color: u32| Step {
            value,
            color: Rgba::rgb(color),
        };
        // Grafana's defaults: green, then red at 80.
        let steps = [step(f64::NEG_INFINITY, 0x73BF69), step(80., 0xF2495C)];
        assert_eq!(step_tiers(&steps), [None, Some(Tier::Crit)]);
        // Low is bad: red base, green from 1.
        let steps = [step(f64::NEG_INFINITY, 0xC4162A), step(1., 0x56A64B)];
        assert_eq!(step_tiers(&steps), [Some(Tier::Crit), None]);
        // Orange and yellow warn; blue and grey are fine.
        assert_eq!(meaning(Rgba::rgb(0xFF9830)), Some(Some(Tier::Warn)));
        assert_eq!(meaning(Rgba::rgb(0xFADE2A)), Some(Some(Tier::Warn)));
        assert_eq!(meaning(Rgba::rgb(0x5794F2)), Some(None));
        assert_eq!(meaning(Rgba::rgb(0x808080)), Some(None));
        // Purple says nothing, so the steps count by position.
        let steps = [
            step(f64::NEG_INFINITY, 0x73BF69),
            step(10., 0xA352CC),
            step(20., 0x8F3BB8),
        ];
        assert_eq!(
            step_tiers(&steps),
            [None, Some(Tier::Warn), Some(Tier::Crit)]
        );
    }
}
