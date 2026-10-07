//! The chart inks (docs/MONITORING.md, Colours), with each theme's values in
//! the palette's `ChartInks`. Every colour a dashboard asks for is ignored:
//! palette modes, fixed and named colours, overrides, continuous schemes
//! and threshold colours. A series' colour
//! follows only from its position, or from its level when the series read
//! as quantiles or histogram buckets. A threshold step's colour gives only
//! its meaning: of several steps the highest is critical and the rest are
//! warnings, and a single step is a warning. The one exception is a Coroot
//! chart of log severities, whose series draw in their severity's colour.
use freshkube_core::coroot::SeriesColor;
use freshkube_core::monitoring::model::{color::Rgba, spec::Step};
use gpui_kit::{Hsla, rgb};

use crate::palette::{ChartInks, Palette};

/// Opacity of an area under its line.
pub(crate) const AREA_OPACITY: f32 = 0.14;
/// Opacity of the other series while one is hovered or picked.
pub(crate) const FADED_OPACITY: f32 = 0.15;
/// Opacity of a threshold's dashed line.
pub(crate) const THRESHOLD_OPACITY: f32 = 0.8;

/// How a series is coloured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Ink {
    /// One of the two slots.
    Slot(usize),
    /// Past the second series: grey, its own slot only while focused.
    Overflow(usize),
    /// A level on the blue ramp, 0 (lowest, darkest) to 1 (highest).
    Level(f32),
    /// A Coroot log severity's colour, as the nearest Console colour.
    Named(SeriesColor),
}

/// The chart ink a series draws in, the same whatever the theme: two
/// series with the same swatch draw alike in both.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Swatch {
    Slot(usize),
    Overflow,
    Level(f32),
    Critical,
    Warning,
    Ok,
    Purple,
}

impl Ink {
    /// The swatch drawn, given whether the series is hovered or picked.
    pub(crate) fn swatch(self, focused: bool) -> Swatch {
        match self {
            Ink::Slot(slot) => Swatch::Slot(slot % 2),
            Ink::Overflow(slot) if focused => Swatch::Slot(slot % 2),
            Ink::Overflow(_) => Swatch::Overflow,
            Ink::Level(level) => Swatch::Level(level),
            Ink::Named(color) => named(color),
        }
    }

    /// The colour drawn in the palette's theme.
    pub(crate) fn color(self, palette: &Palette, focused: bool) -> Hsla {
        self.swatch(focused).color(&palette.chart)
    }
}

impl Swatch {
    pub(crate) fn color(self, inks: &ChartInks) -> Hsla {
        match self {
            Swatch::Slot(slot) => hex(inks.slots[slot % inks.slots.len()]),
            Swatch::Overflow => hex(inks.overflow),
            Swatch::Level(level) => ramp(inks, level),
            Swatch::Critical => hex(inks.critical),
            Swatch::Warning => hex(inks.warning),
            Swatch::Ok => hex(inks.ok),
            Swatch::Purple => hex(inks.purple),
        }
    }
}

/// The Console colour drawn for a severity (docs/DESIGN.md, Charts).
fn named(color: SeriesColor) -> Swatch {
    match color {
        SeriesColor::Critical => Swatch::Critical,
        SeriesColor::Warning => Swatch::Warning,
        SeriesColor::Ok => Swatch::Ok,
        SeriesColor::Blue => Swatch::Slot(0),
        SeriesColor::Orange => Swatch::Slot(1),
        SeriesColor::Purple => Swatch::Purple,
        SeriesColor::Grey => Swatch::Overflow,
    }
}

fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

/// A colour along the ramp; its three stops at 0, ½ and 1.
fn ramp(inks: &ChartInks, level: f32) -> Hsla {
    let stops = &inks.ramp;
    let level = level.clamp(0., 1.) * (stops.len() - 1) as f32;
    let low = (level.floor() as usize).min(stops.len() - 2);
    let t = level - low as f32;
    let channel = |value: u32, shift: u32| ((value >> shift) & 0xff) as f32;
    let mix = |shift: u32| {
        let (a, b) = (channel(stops[low], shift), channel(stops[low + 1], shift));
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
                if index < 2 {
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

impl Tier {
    /// The ink a threshold's line and band draw in.
    pub(crate) fn color(self, palette: &Palette) -> Hsla {
        match self {
            Tier::Warn => Swatch::Warning,
            Tier::Crit => Swatch::Critical,
        }
        .color(&palette.chart)
    }
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
    use crate::palette::{dark, light};

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
        assert_eq!(inks[0].color(&dark(), false), hex(0x5E93E6));
        assert_eq!(inks[5].color(&dark(), false), hex(0x737A85));
        // Grey until focused, then its own slot (the first again).
        assert_eq!(inks[6].color(&dark(), false), hex(0x737A85));
        assert_eq!(inks[6].color(&dark(), true), hex(0x5E93E6));
        assert_eq!(inks[7].color(&dark(), true), hex(0xCC7C4A));
    }

    #[test]
    fn quantiles_take_the_ramp_lowest_darkest() {
        let inks = named(&["p99", "p50", "p95"]);
        assert_eq!(inks, [Ink::Level(1.), Ink::Level(0.), Ink::Level(0.5)]);
        assert_eq!(inks[1].color(&dark(), false), hex(0x5379BB));
        assert_eq!(inks[2].color(&dark(), false), hex(0x7AA0E6));
        assert_eq!(inks[0].color(&dark(), false), hex(0xB3CEFA));
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
        assert_eq!(inks[0].color(&dark(), false), hex(0xF28B82));
        assert_eq!(inks[0].color(&dark(), true), hex(0xF28B82));
        assert_eq!(
            Ink::Named(SeriesColor::Warning).color(&dark(), false),
            hex(0xF2C46D)
        );
        assert_eq!(inks[2].color(&dark(), false), hex(0x82D4AB));
    }

    #[test]
    fn the_ramp_interpolates_between_its_stops() {
        assert_eq!(ramp(&dark().chart, 0.), hex(0x5379BB));
        assert_eq!(ramp(&dark().chart, 1.), hex(0xB3CEFA));
        assert_eq!(ramp(&dark().chart, 0.25), hex(0x678DD1));
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

    /// WCAG contrast of `ink` over `ground`, both `0xRRGGBB`, with `ink`
    /// drawn at `opacity`.
    fn contrast(ink: u32, ground: u32, opacity: f32) -> f32 {
        let channel = |value: u32, shift: u32| ((value >> shift) & 0xff) as f32 / 255.;
        let luminance = |rgb: [f32; 3]| {
            let lin = |c: f32| {
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2])
        };
        let over = [16, 8, 0]
            .map(|shift| channel(ink, shift) * opacity + channel(ground, shift) * (1. - opacity));
        let (a, b) = (
            luminance(over),
            luminance([16, 8, 0].map(|s| channel(ground, s))),
        );
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn rgb(color: Hsla) -> u32 {
        let c = color.to_rgb();
        [c.r, c.g, c.b]
            .map(|v| (v * 255.).round() as u32)
            .into_iter()
            .fold(0, |acc, v| acc << 8 | v)
    }

    /// Every series ink reaches 3:1 on the card in both themes, as bars and
    /// lines draw it, and each threshold at the opacity its line draws at.
    #[test]
    fn every_ink_reaches_three_to_one_on_the_card() {
        for (theme, palette) in [("light", light()), ("dark", dark())] {
            let card = rgb(palette.surface);
            let inks = palette.chart;
            let mut series: Vec<(String, u32)> = vec![
                ("slot 1".into(), inks.slots[0]),
                ("slot 2".into(), inks.slots[1]),
                ("overflow".into(), inks.overflow),
                ("critical".into(), inks.critical),
                ("warning".into(), inks.warning),
                ("ok".into(), inks.ok),
                ("purple".into(), inks.purple),
            ];
            for level in [0., 0.25, 0.5, 0.75, 1.] {
                series.push((format!("ramp {level}"), rgb(ramp(&inks, level))));
            }
            for (name, ink) in series {
                let ratio = contrast(ink, card, 1.);
                assert!(ratio >= 3., "{theme} {name} {ink:06X}: {ratio:.2}");
            }
            for tier in [Tier::Warn, Tier::Crit] {
                let ink = rgb(tier.color(&palette));
                let ratio = contrast(ink, card, THRESHOLD_OPACITY);
                assert!(ratio >= 3., "{theme} {tier:?} {ink:06X}: {ratio:.2}");
            }
        }
    }

    /// Light draws its own, darker inks; the same swatch draws alike in
    /// both themes, so lines group the same way in either.
    #[test]
    fn light_draws_darker_inks_by_the_same_swatch() {
        let ink = Ink::Named(SeriesColor::Blue);
        assert_eq!(ink.swatch(false), Ink::Slot(0).swatch(false));
        assert_eq!(ink.color(&light(), false), hex(0x2F6BD6));
        assert_eq!(ink.color(&dark(), false), hex(0x5E93E6));
        assert_eq!(
            Ink::Named(SeriesColor::Warning).color(&light(), false),
            hex(0x917300)
        );
        assert_eq!(Tier::Warn.color(&dark()), dark().warn);
        assert_eq!(Tier::Crit.color(&dark()), dark().crit);
    }
}
