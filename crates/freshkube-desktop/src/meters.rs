//! Fog bullet meters. The request is a band, use a thin bar, and the
//! right edge the limit. Unknown limits never pretend to be known.
use crate::{
    palette::Palette,
    ui::{dp, dp_px},
};
use gpui_kit::{prelude::*, *};

#[derive(Clone, Copy)]
pub(crate) enum Resource {
    Cpu,
    Memory,
}

pub(crate) fn bullet(
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
    let color = if !stale && limit.is_some_and(|v| v > 0. && used / v >= 0.85) {
        p.warn
    } else if !stale && request.is_some_and(|v| used > v) {
        resource_color.blend(p.ink.opacity(0.22))
    } else {
        resource_color
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
