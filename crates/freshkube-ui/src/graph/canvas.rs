//! The graph's canvas: the calls drawn under the boxes, the markers on
//! them and the boxes themselves. [`GraphView`](super::GraphView) draws it
//! inside its card.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, PathBuilder, TestSupportExt, Window, canvas, div, point};

use freshkube_graph::layout::{Metrics, NODE_H, NODE_W};

use super::view::{glyph, mono, muted};
use super::{
    CONTEXT, Cursor, EdgeStyle, GraphSource, Step, WalkBack, WalkForward, WalkNext, WalkPrevious,
};
use crate::palette::palette;
use crate::ui::{self, Tone, dp};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_calls_list_marks_dashed_and_dotted_lines_only() {
        assert_eq!(style_mark(EdgeStyle::Dashed), Some("╌"));
        assert_eq!(style_mark(EdgeStyle::Dotted), Some("┄"));
        assert_eq!(style_mark(EdgeStyle::Solid), None);
        assert_eq!(style_mark(EdgeStyle::Auto), None);
    }

    #[test]
    fn only_a_box_cursor_on_that_key_rings_a_box() {
        assert!(at_cursor(&Some(Cursor::Node(1)), &1));
        assert!(!at_cursor(&Some(Cursor::Node(1)), &2));
        assert!(!at_cursor(&Some(Cursor::Link(1, 2)), &1));
        assert!(!at_cursor(&None, &1));
    }

    #[test]
    fn claimed_and_unknown_lines_are_stroked_differently_and_auto_is_the_maps_rule() {
        assert_eq!(dash(EdgeStyle::Solid, None), Dash::Solid);
        assert_eq!(dash(EdgeStyle::Dashed, None), Dash::Dashed);
        assert_eq!(dash(EdgeStyle::Dotted, None), Dash::Dotted);
        // Whatever the tone, an explicit style wins.
        assert_eq!(dash(EdgeStyle::Solid, Some(Tone::Crit)), Dash::Solid);
        assert_eq!(dash(EdgeStyle::Dotted, Some(Tone::Good)), Dash::Dotted);
        // Auto is solid only for a healthy call, as the service map draws.
        assert_eq!(dash(EdgeStyle::Auto, Some(Tone::Good)), Dash::Solid);
        for tone in [
            None,
            Some(Tone::Warn),
            Some(Tone::Crit),
            Some(Tone::Unknown),
        ] {
            assert_eq!(dash(EdgeStyle::Auto, tone), Dash::Dashed);
        }
    }
}

/// Calls under the boxes, the selected one last and in the accent; a
/// marker on each call that has one; then the boxes.
pub(super) fn render_graph<S: GraphSource>(
    source: &S,
    window: &mut Window,
    cx: &mut Context<S>,
) -> AnyElement {
    let state = source.graph();
    if let Some(focus) = state.focus.as_ref() {
        let mut watch = state.focus_watch.borrow_mut();
        if watch.is_empty() {
            watch.push(cx.on_focus(focus, window, |_, _, cx| cx.notify()));
            watch.push(cx.on_blur(focus, window, |_, _, cx| cx.notify()));
        }
    }
    let focused = state
        .focus
        .as_ref()
        .is_some_and(|focus| focus.is_focused(window));
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
                match dash(edge.style, edge.tone) {
                    Dash::Solid => {}
                    Dash::Dotted => path = path.dash_array(&[unit * 1.5, unit * 3.5]),
                    Dash::Dashed => path = path.dash_array(&[unit * 5., unit * 3.]),
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
    let cursor = if focused { source.cursor() } else { None };
    // With keys the graph is one Tab stop; its boxes and calls are not.
    let keyed = state.focus.is_some();
    // A labelled graph leaves room under its last row for the scrollbar.
    let room = if state.metrics == Metrics::MAP {
        0.
    } else {
        12.
    };
    div()
        .id(state.ids.graph.clone())
        .test_support()
        .when_some(state.focus.as_ref(), |this, focus| {
            this.track_focus(focus)
                .key_context(CONTEXT)
                .on_action(walk::<S, WalkBack>(Step::Back, cx))
                .on_action(walk::<S, WalkForward>(Step::Forward, cx))
                .on_action(walk::<S, WalkPrevious>(Step::Previous, cx))
                .on_action(walk::<S, WalkNext>(Step::Next, cx))
        })
        .when(keyed, |this| {
            this.focus(|style| style.bg(p.accent_soft))
                .on_click(cx.listener(|this: &mut S, _, window, cx| {
                    if let Some(focus) = this.graph().focus_handle() {
                        window.focus(focus, cx);
                    }
                    if this.cursor().is_none()
                        && let Some(cursor) = this.graph().step(None, Step::Next)
                    {
                        this.step_to(cursor, window, cx);
                    }
                    cx.notify();
                }))
        })
        .relative()
        .flex_none()
        .w(dp(display.width))
        .h(dp(display.height + room))
        .child(calls)
        .children(display.markers.iter().map(|&ix| {
            let edge = &state.edges[display.shown[ix].edge];
            let (x, y) = display.shown[ix].route.mid;
            let (from, to) = (edge.from.clone(), edge.to.clone());
            Button::new(edge.marker_id.clone())
                .tab_stop(!keyed)
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
                .on_click(cx.listener(move |this: &mut S, _, window, cx| {
                    this.select_link(&from, &to, window, cx);
                    cx.notify();
                }))
        }))
        .children(display.pills.iter().map(|&ix| {
            let edge = &state.edges[display.shown[ix].edge];
            let (x, y) = display.shown[ix].route.mid;
            let (from, to) = (edge.from.clone(), edge.to.clone());
            // A fixed-width box centred on the line's middle, so the pill
            // sits on the line whatever its label.
            div()
                .absolute()
                .left(dp(x - PILL_W / 2.))
                .top(dp(y - PILL_H / 2.))
                .w(dp(PILL_W))
                .h(dp(PILL_H))
                .flex()
                .justify_center()
                .child(
                    div().rounded_full().bg(p.surface).child(
                        Button::new(edge.marker_id.clone())
                            .outline()
                            .group("fog-control")
                            .h(dp(PILL_H))
                            .px(dp(6.))
                            .gap(dp(4.))
                            .rounded_full()
                            .bg(if display.selected == Some(ix) {
                                p.accent_soft
                            } else {
                                p.surface
                            })
                            .border_color(if display.selected == Some(ix) {
                                p.accent_line
                            } else {
                                p.line_strong
                            })
                            .max_w(dp(PILL_W))
                            .tab_stop(!keyed)
                            .children(edge.pill_tone.and_then(|tone| ui::status_glyph(tone, cx)))
                            .child(
                                mono(edge.pill.clone().unwrap_or_default())
                                    .text_size(dp(11.))
                                    .truncate(),
                            )
                            .when(edge.tooltip.is_empty(), |this| {
                                this.tooltip(edge.pill.clone().unwrap_or_default())
                            })
                            .when(!edge.tooltip.is_empty(), |this| {
                                this.tooltip(edge.tooltip.clone())
                            })
                            .on_click(cx.listener(move |this: &mut S, _, window, cx| {
                                this.select_link(&from, &to, window, cx);
                                cx.notify();
                            })),
                    ),
                )
        }))
        .children(display.placed.iter().map(|placed| {
            let node = &state.nodes[placed.node];
            let key = node.key.clone();
            Button::new(node.id.clone())
                .when(state.dimmed.contains(&node.key), |this| this.opacity(0.35))
                .tab_stop(!keyed)
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
                .border_color(
                    if highlighted == Some(&node.key) || at_cursor(&cursor, &node.key) {
                        p.accent_line
                    } else {
                        p.line_strong
                    },
                )
                // The keyboard's box carries a thicker ring.
                .when(at_cursor(&cursor, &node.key), |this| this.border_2())
                .when(node.ghost, |this| this.border_dashed())
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
                        .child(if source.word_details() {
                            div()
                                .w_full()
                                .text_size(dp(12.))
                                .text_color(p.muted)
                                .child(crate::table::word_cut(
                                    gpui_kit::SharedString::from(format!("{}-detail", node.id)),
                                    node.detail.clone(),
                                ))
                                .into_any_element()
                        } else {
                            muted(node.detail.clone(), cx)
                                .w_full()
                                .truncate()
                                .into_any_element()
                        }),
                )
                .when(!node.tooltip.is_empty(), |this| {
                    this.tooltip(node.tooltip.clone())
                })
                .on_click(cx.listener(move |this: &mut S, _, window, cx| {
                    if this.graph().focus_handle().is_some() {
                        this.step_to(Cursor::Node(key.clone()), window, cx);
                    }
                    this.open(&key, window, cx);
                }))
        }))
        .into_any_element()
}

/// How a line is stroked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Dash {
    Solid,
    Dashed,
    Dotted,
}

/// The stroke for a call's style: `Auto` is solid only when the call is
/// `Good`, and dashed otherwise.
pub(super) fn dash(style: EdgeStyle, tone: Option<Tone>) -> Dash {
    match (style, tone) {
        (EdgeStyle::Solid, _) | (EdgeStyle::Auto, Some(Tone::Good)) => Dash::Solid,
        (EdgeStyle::Dotted, _) => Dash::Dotted,
        (EdgeStyle::Dashed | EdgeStyle::Auto, _) => Dash::Dashed,
    }
}

/// The mark the Calls list shows for a call's line, so its style reads
/// there too; the service map's `Auto` and a solid line show none.
pub(super) fn style_mark(style: EdgeStyle) -> Option<&'static str> {
    match style {
        EdgeStyle::Dashed => Some("╌"),
        EdgeStyle::Dotted => Some("┄"),
        EdgeStyle::Solid | EdgeStyle::Auto => None,
    }
}

/// Whether the keyboard's cursor is on the box `key`.
fn at_cursor<K: PartialEq>(cursor: &Option<Cursor<K>>, key: &K) -> bool {
    matches!(cursor, Some(Cursor::Node(at)) if at == key)
}

/// The size of a pill on a line, in dp: wide enough for a short key in
/// monospace, and the box that centres it.
const PILL_W: f32 = 100.;
const PILL_H: f32 = 22.;

/// The listener for one arrow key: ask the graph where it goes from the
/// source's cursor, hand the answer to the source and scroll to it.
fn walk<S: GraphSource, A: 'static>(
    step: Step,
    cx: &mut Context<S>,
) -> impl Fn(&A, &mut Window, &mut gpui_kit::App) + 'static {
    let this = cx.entity().downgrade();
    move |_, window, cx| {
        let _ = this.update(cx, |this: &mut S, cx| {
            let Some(cursor) = this.graph().step(this.cursor().as_ref(), step) else {
                return;
            };
            let end = match &cursor {
                Cursor::Node(key) | Cursor::Link(_, key) => key.clone(),
            };
            this.step_to(cursor, window, cx);
            this.graph().reveal_node(&end, window);
            cx.notify();
        });
    }
}

/// The shared canvas inside a page's own frame and Inspector. It scrolls
/// sideways with an always visible scrollbar and keeps its graph's focus.
pub struct GraphCanvas;

impl GraphCanvas {
    /// A canvas legend with the table footer's geometry and colours.
    pub fn legend(
        id: impl Into<gpui_kit::ElementId>,
        legend: impl IntoElement,
        cx: &gpui_kit::App,
    ) -> AnyElement {
        crate::table::footer(
            id,
            vec![],
            Some(legend.into_any_element()),
            palette(cx).surface,
            cx,
        )
        .into_any_element()
    }

    pub fn render<S: GraphSource>(
        source: &S,
        window: &mut Window,
        cx: &mut Context<S>,
    ) -> AnyElement {
        use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
        let state = source.graph();
        let graph = render_graph(source, window, cx);
        div()
            .id(state.id("canvas"))
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(
                div()
                    .relative()
                    .min_w_0()
                    .pb(dp(12.))
                    .child(
                        div()
                            .id(state.ids.scroll.clone())
                            .test_support()
                            .track_scroll(&state.pan)
                            .w_full()
                            .min_w_0()
                            .overflow_x_scroll()
                            .child(graph),
                    )
                    .child(
                        Scrollbar::horizontal(&state.pan)
                            .id(state.ids.scrollbar.clone())
                            .mode(ScrollbarMode::Always),
                    ),
            )
            .into_any_element()
    }
}
