//! Deploys and node events across a timeseries: a thin line with a small
//! triangle at the foot, in accent blue for a deploy and critical red for a
//! node, fainter when the node is Ready again. They are placed on the
//! chart's window when the markers or the answer change; the cursor's
//! readout names the one under the pointer.
use std::rc::Rc;

use freshkube_core::monitoring::markers::{Marker, MarkerKind};
use gpui_kit::component::h_flex;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Bounds, Hsla, PathBuilder, Pixels, Point, SharedString, Window, canvas, div, fill,
    point, px, size,
};

use super::cursor::when;
use crate::monitoring::derive::Chart;
use crate::palette::Palette;
use crate::ui::{self, dp, dp_px};

/// How near the pointer must come to a marker's line to name it, in dp.
pub(super) const REACH: f32 = 6.;

/// A marker on one chart's window.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Placed {
    /// Across the window, 0 to 1.
    pub at: f32,
    pub kind: MarkerKind,
    pub label: SharedString,
    /// "Today 13:12".
    pub time: SharedString,
}

/// The markers that fall within `chart`'s window.
pub(super) fn place(markers: &[Marker], chart: &Chart) -> Rc<[Placed]> {
    let span = (chart.end - chart.start).max(1.);
    markers
        .iter()
        .filter_map(|marker| {
            let at = ((marker.at as f64 - chart.start) / span) as f32;
            (0.0..=1.0).contains(&at).then(|| Placed {
                at,
                kind: marker.kind,
                label: marker.label.clone().into(),
                time: when(marker.at as f64).into(),
            })
        })
        .collect()
}

/// A marker's colour, and the opacity of its line and of its triangle.
pub(super) fn ink(kind: MarkerKind, p: &Palette) -> (Hsla, f32, f32) {
    match kind {
        MarkerKind::Deploy => (p.accent, 0.55, 1.),
        MarkerKind::NodeNotReady | MarkerKind::NodeReboot => (p.crit, 0.55, 1.),
        MarkerKind::NodeReady => (p.crit, 0.25, 0.45),
    }
}

/// Paints each marker's line over the plot's inner rectangle and its
/// triangle on the baseline. `left`, `top`, `width` and `height` are the
/// inner rectangle from `origin`.
pub(super) fn paint(
    markers: &[Placed],
    origin: Point<Pixels>,
    (left, top, width, height): (Pixels, Pixels, Pixels, Pixels),
    p: &Palette,
    window: &mut Window,
) {
    let (half, tall) = (dp_px(4.5, window), dp_px(7., window));
    for marker in markers {
        let (color, line, glyph) = ink(marker.kind, p);
        let x = (left + width * marker.at).floor();
        window.paint_quad(fill(
            Bounds::new(origin + point(x, top), size(px(1.), height)),
            color.opacity(line),
        ));
        let foot = origin + point(x + px(0.5), top + height);
        let mut triangle = PathBuilder::fill();
        triangle.move_to(foot - point(px(0.), tall));
        triangle.line_to(foot + point(half, px(0.)));
        triangle.line_to(foot - point(half, px(0.)));
        triangle.close();
        if let Ok(path) = triangle.build() {
            window.paint_path(path, color.opacity(glyph));
        }
    }
}

/// The marker nearest `fraction` across the window, when within `reach`.
pub(super) fn nearest(markers: &[Placed], fraction: f32, reach: f32) -> Option<usize> {
    markers
        .iter()
        .enumerate()
        .map(|(index, marker)| (index, (marker.at - fraction).abs()))
        .filter(|(_, distance)| *distance <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

/// A small triangle glyph in the marker's colour, for the readout and the
/// header's toggles.
pub(crate) fn glyph(color: Hsla, size_dp: f32) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            let (w, h) = (bounds.size.width, bounds.size.height);
            let mut path = PathBuilder::fill();
            path.move_to(bounds.origin + point(w / 2., h * 0.15));
            path.line_to(bounds.origin + point(w * 0.9, h * 0.85));
            path.line_to(bounds.origin + point(w * 0.1, h * 0.85));
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .flex_none()
    .size(dp(size_dp))
    .into_any_element()
}

/// The readout's row for the marker under the pointer.
pub(super) fn readout_row(marker: &Placed, p: &Palette) -> AnyElement {
    let (color, _, opacity) = ink(marker.kind, p);
    h_flex()
        .gap(dp(8.))
        .pb(dp(2.))
        .child(glyph(color.opacity(opacity), 12.))
        .child(
            div()
                .max_w(dp(260.))
                .truncate()
                .text_size(dp(12.))
                .text_color(p.ink)
                .child(marker.label.clone()),
        )
        .child(
            div()
                .flex_none()
                .font_family(ui::MONO_FONT)
                .text_size(dp(11.))
                .text_color(p.muted)
                .child(marker.time.clone()),
        )
        .into_any_element()
}
