//! The graph's canvas: the calls drawn under the boxes, the markers on
//! them and the boxes themselves. [`GraphView`](super::GraphView) draws it
//! inside its card.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, PathBuilder, TestSupportExt, canvas, div, point};

use freshkube_graph::layout::{NODE_H, NODE_W};

use super::GraphSource;
use super::view::{glyph, mono, muted};
use crate::palette::palette;
use crate::ui::{self, Tone, dp};

/// Calls under the boxes, the selected one last and in the accent; a
/// marker on each call that has one; then the boxes.
pub(super) fn render_graph<S: GraphSource>(source: &S, cx: &mut Context<S>) -> AnyElement {
    let state = source.graph();
    let p = palette(cx);
    let display = &state.display;
    let edges = state.edges.clone();
    let shown = display.shown.clone();
    let selected = display.selected;
    let calls = canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            // One dp in this window's pixels, so the closure holds no borrow.
            let unit = ui::dp_px(1., window);
            let at = |(x, y): (f32, f32)| point(bounds.left() + unit * x, bounds.top() + unit * y);
            let order = (0..shown.len())
                .filter(|&ix| Some(ix) != selected)
                .chain(selected);
            for ix in order {
                let (edge, route) = (&edges[shown[ix].edge], &shown[ix].route);
                let color = if Some(ix) == selected {
                    p.accent
                } else {
                    match edge.tone {
                        Some(Tone::Crit | Tone::Died) => p.crit,
                        Some(Tone::Warn) => p.warn,
                        _ => p.line_strong,
                    }
                };
                let mut path = PathBuilder::stroke(unit * edge.width);
                if edge.tone != Some(Tone::Good) {
                    let dash = unit * 5.;
                    path = path.dash_array(&[dash, dash * 0.6]);
                }
                path.move_to(at(route.segments[0][0]));
                for &[_, b, c, d] in &route.segments {
                    path.cubic_bezier_to(at(d), at(b), at(c));
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
                let [tip, left, right] = route.head();
                let mut head = PathBuilder::fill();
                head.move_to(at(tip));
                head.line_to(at(left));
                head.line_to(at(right));
                head.close();
                if let Ok(head) = head.build() {
                    window.paint_path(head, color);
                }
            }
        },
    )
    .size_full();
    let highlighted = source.highlighted();
    div()
        .id(state.ids.graph.clone())
        .test_support()
        .relative()
        .flex_none()
        .w(dp(display.width))
        .h(dp(display.height))
        .child(calls)
        .children(display.markers.iter().map(|&ix| {
            let edge = &state.edges[display.shown[ix].edge];
            let (x, y) = display.shown[ix].route.mid;
            let (from, to) = (edge.from.clone(), edge.to.clone());
            Button::new(edge.marker_id.clone())
                .ghost()
                .group("fog-control")
                .absolute()
                .left(dp(x - 11.))
                .top(dp(y - 11.))
                .size(dp(22.))
                .px_0()
                .rounded_full()
                .bg(p.surface)
                .child(glyph(edge.tone, cx))
                .when(!edge.tooltip.is_empty(), |this| {
                    this.tooltip(edge.tooltip.clone())
                })
                .on_click(cx.listener(move |this: &mut S, _, _, cx| {
                    this.graph_mut().select(&from, &to);
                    cx.notify();
                }))
        }))
        .children(display.placed.iter().map(|placed| {
            let node = &state.nodes[placed.node];
            let key = node.key.clone();
            Button::new(node.id.clone())
                .outline()
                .group("fog-control")
                .absolute()
                .left(dp(placed.x))
                .top(dp(placed.y))
                .w(dp(NODE_W))
                .h(dp(NODE_H))
                .px(dp(10.))
                .gap(dp(8.))
                .bg(p.surface_2)
                .border_color(if highlighted == Some(&node.key) {
                    p.accent_line
                } else {
                    p.line_strong
                })
                .justify_start()
                .child(glyph(node.tone, cx))
                .child(
                    v_flex()
                        .min_w_0()
                        .flex_1()
                        .items_start()
                        .child(
                            mono(node.label.clone())
                                .w_full()
                                .truncate()
                                .font_weight(ui::HEADING_WEIGHT),
                        )
                        .child(muted(node.detail.clone(), cx).w_full().truncate()),
                )
                .when(!node.tooltip.is_empty(), |this| {
                    this.tooltip(node.tooltip.clone())
                })
                .on_click(
                    cx.listener(move |this: &mut S, _, window, cx| this.open(&key, window, cx)),
                )
        }))
        .into_any_element()
}
