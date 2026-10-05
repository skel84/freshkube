//! Fog bullet meters. The request is a band, use a thin bar, and the
//! right edge the limit. Unknown limits never pretend to be known.
//! `tip` words what the bullet draws, from the same emphasis.
use crate::{
    palette::Palette,
    ui::{dp, dp_px},
};
use gpui_kit::{prelude::*, *};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Resource {
    Cpu,
    Memory,
}

/// What the meter's right edge is: a pod's limit or a node's allocatable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum End {
    Limit,
    Allocatable,
}

/// One meter's figures, for its tooltip. `used` is `None` when no reading
/// arrived; the bullet then isn't drawn.
#[derive(Clone, Copy, Debug)]
pub struct Reading {
    pub resource: Resource,
    pub used: Option<f64>,
    pub request: Option<f64>,
    pub end: Option<f64>,
    pub kind: End,
    pub stale: bool,
}

/// How the bullet stands out, and the tooltip's closing words for it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Emphasis {
    Normal,
    AboveRequest,
    NearEnd,
}

/// The share of the end at which the bar turns amber.
const NEAR_END: f64 = 0.85;

fn emphasis(used: f64, request: Option<f64>, end: Option<f64>, stale: bool) -> Emphasis {
    if stale {
        Emphasis::Normal
    } else if end.is_some_and(|v| v > 0. && used / v >= NEAR_END) {
        Emphasis::NearEnd
    } else if request.is_some_and(|v| used > v) {
        Emphasis::AboveRequest
    } else {
        Emphasis::Normal
    }
}

/// Rounded down, so a value never reads as the next threshold.
fn whole_percent(part: f64, whole: f64) -> u64 {
    (part / whole * 100.).floor().max(0.) as u64
}

/// The meter's tooltip: use, its share of the end, the request, the end, and
/// why the bar stands out. `format` writes an amount in the page's units.
/// A pod without a limit really has none; a node's missing figures are
/// unknown.
pub fn tip(reading: &Reading, format: impl Fn(f64) -> String) -> String {
    let (end_name, missing) = match reading.kind {
        End::Limit => ("limit", "none"),
        End::Allocatable => ("allocatable", "unknown"),
    };
    let text = |amount: Option<f64>| amount.map_or(missing.to_owned(), &format);
    let end = reading.end.filter(|v| *v > 0.);
    let mut tip = match reading.resource {
        Resource::Cpu => "CPU".to_owned(),
        Resource::Memory => "Memory".to_owned(),
    };
    match reading.used {
        Some(used) => {
            tip.push_str(&format!(" {} used", format(used)));
            if reading.stale {
                tip.push_str(" (last known)");
            }
        }
        None => tip.push_str(" used unknown"),
    }
    if let (Some(used), Some(end)) = (reading.used, end) {
        tip.push_str(&format!(" · {}% of {end_name}", whole_percent(used, end)));
    }
    tip.push_str(" · requested ");
    tip.push_str(&text(reading.request));
    if let (Some(request), Some(end)) = (reading.request, end) {
        tip.push_str(&format!(" ({}%)", whole_percent(request, end)));
    }
    tip.push_str(&format!(" · {end_name} {}", text(reading.end)));
    match reading
        .used
        .map(|used| emphasis(used, reading.request, reading.end, reading.stale))
    {
        Some(Emphasis::AboveRequest) => tip.push_str(" · above request"),
        Some(Emphasis::NearEnd) => tip.push_str(&format!(" · at least 85% of {end_name}")),
        _ => {}
    }
    tip
}

pub fn bullet(
    resource: Resource,
    used: f64,
    request: Option<f64>,
    limit: Option<f64>,
    stale: bool,
    p: &Palette,
) -> impl IntoElement {
    let span = limit
        .filter(|v| *v > 0.)
        .or(request.filter(|v| *v > 0.).map(|v| v * 2.))
        .unwrap_or(used.max(1.));
    let use_fraction = (used / span).clamp(0., 1.) as f32;
    let request_fraction = (request.unwrap_or(0.) / span).clamp(0., 1.) as f32;
    let resource_color = match resource {
        Resource::Cpu => p.accent,
        Resource::Memory => p.memory,
    };
    let color = match emphasis(used, request, limit, stale) {
        Emphasis::NearEnd => p.warn,
        Emphasis::AboveRequest => resource_color.blend(p.ink.opacity(0.22)),
        Emphasis::Normal => resource_color,
    };
    let track = p.surface_2;
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let band = Bounds::new(
                bounds.origin,
                size(bounds.size.width * request_fraction, bounds.size.height),
            );
            window.paint_quad(fill(bounds, track).corner_radii(px(3.)));
            window.paint_quad(fill(band, resource_color.opacity(0.28)).corner_radii(px(3.)));
            let height = dp_px(4., window);
            let bar = Bounds::new(
                point(bounds.left(), bounds.center().y - height / 2.),
                size(bounds.size.width * use_fraction, height),
            );
            window.paint_quad(
                fill(bar, color.opacity(if stale { 0.4 } else { 1. })).corner_radii(px(2.)),
            );
            if stale {
                let step = dp_px(5., window);
                let mut x = bar.left();
                let mut stripes = PathBuilder::stroke(dp_px(1.5, window));
                while x < bar.right() {
                    stripes.move_to(point(x, bar.bottom()));
                    stripes.line_to(point((x + height).min(bar.right()), bar.top()));
                    x += step;
                }
                if let Ok(path) = stripes.build() {
                    window.paint_path(path, resource_color);
                }
            }
        },
    )
    .w(dp(44.))
    .h(dp(10.))
    .flex_none()
}

#[cfg(test)]
mod tests {
    use super::{Emphasis, End, Reading, Resource, emphasis, tip};

    fn cpu(used: f64, request: Option<f64>, end: Option<f64>, kind: End, stale: bool) -> String {
        tip(
            &Reading {
                resource: Resource::Cpu,
                used: Some(used),
                request,
                end,
                kind,
                stale,
            },
            |v| format!("{v:.0}m"),
        )
    }

    #[test]
    fn a_pod_reads_use_request_and_limit() {
        assert_eq!(
            cpu(115., Some(100.), Some(200.), End::Limit, false),
            "CPU 115m used · 57% of limit · requested 100m (50%) · limit 200m · above request"
        );
        assert_eq!(
            cpu(50., Some(100.), Some(200.), End::Limit, false),
            "CPU 50m used · 25% of limit · requested 100m (50%) · limit 200m"
        );
    }

    #[test]
    fn a_node_reads_allocatable() {
        assert_eq!(
            cpu(3400., Some(1000.), Some(4000.), End::Allocatable, false),
            "CPU 3400m used · 85% of allocatable · requested 1000m (25%) · allocatable 4000m · at least 85% of allocatable"
        );
    }

    #[test]
    fn exactly_85_percent_is_near_the_end_and_just_under_is_not() {
        assert!(
            cpu(170., None, Some(200.), End::Limit, false).ends_with("· at least 85% of limit")
        );
        let under = cpu(169.9, None, Some(200.), End::Limit, false);
        assert!(under.contains("84% of limit"), "{under}");
        assert!(!under.contains("at least"), "{under}");
    }

    #[test]
    fn stale_use_names_no_emphasis() {
        assert_eq!(
            cpu(190., Some(100.), Some(200.), End::Limit, true),
            "CPU 190m used (last known) · 95% of limit · requested 100m (50%) · limit 200m"
        );
    }

    #[test]
    fn missing_figures_are_none_on_a_pod_and_unknown_on_a_node() {
        assert_eq!(
            cpu(115., None, None, End::Limit, false),
            "CPU 115m used · requested none · limit none"
        );
        assert_eq!(
            cpu(115., Some(100.), None, End::Limit, false),
            "CPU 115m used · requested 100m · limit none · above request"
        );
        assert_eq!(
            cpu(115., None, Some(200.), End::Allocatable, false),
            "CPU 115m used · 57% of allocatable · requested unknown · allocatable 200m"
        );
        let unread = tip(
            &Reading {
                resource: Resource::Memory,
                used: None,
                request: Some(1.),
                end: None,
                kind: End::Allocatable,
                stale: false,
            },
            |v| format!("{v:.1}Gi"),
        );
        assert_eq!(
            unread,
            "Memory used unknown · requested 1.0Gi · allocatable unknown"
        );
    }

    #[test]
    fn emphasis_is_what_the_bullet_colours_by() {
        assert_eq!(
            emphasis(90., Some(50.), Some(100.), false),
            Emphasis::NearEnd
        );
        assert_eq!(emphasis(90., Some(50.), Some(100.), true), Emphasis::Normal);
        assert_eq!(
            emphasis(60., Some(50.), Some(100.), false),
            Emphasis::AboveRequest
        );
        assert_eq!(
            emphasis(60., Some(50.), None, false),
            Emphasis::AboveRequest
        );
        assert_eq!(
            emphasis(50., Some(50.), Some(100.), false),
            Emphasis::Normal
        );
        assert_eq!(emphasis(90., None, Some(0.), false), Emphasis::Normal);
        // The tooltip's tail names the same state.
        for (used, request, end, stale) in [
            (90., Some(50.), Some(100.), false),
            (60., Some(50.), Some(100.), false),
            (40., Some(50.), Some(100.), false),
            (90., Some(50.), Some(100.), true),
        ] {
            let text = cpu(used, request, end, End::Limit, stale);
            assert_eq!(
                text.ends_with("at least 85% of limit"),
                emphasis(used, request, end, stale) == Emphasis::NearEnd
            );
            assert_eq!(
                text.ends_with("above request"),
                emphasis(used, request, end, stale) == Emphasis::AboveRequest
            );
        }
    }
}
