//! Drawing a graph from its source: the toolbar, the graph's card with
//! its calls, markers and boxes, the legend, and the inspector beside the
//! card, or under it once the graph can't sit beside it.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Div, PathBuilder, SharedString, TestSupportExt, canvas, div, point,
    px, uniform_list,
};

use freshkube_graph::layout::{NODE_H, NODE_W};

use super::{GraphEdge, GraphSource};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, Tone, dp};

/// The inspector's width beside the graph.
const INSPECTOR_W: f32 = 264.;

/// Draws a [`GraphSource`]'s graph. Like the data table, it is a plain
/// struct: the source's entity owns the state and the view only reads it.
pub struct GraphView {
    /// The width the graph and its inspector may take, in dp.
    available: f32,
}

impl GraphView {
    /// A view `available` dp wide, such as the page's content width.
    pub fn new(available: f32) -> Self {
        Self { available }
    }

    pub fn render<S: GraphSource>(self, source: &S, cx: &mut Context<S>) -> AnyElement {
        let state = source.graph();
        let toolbar = render_toolbar(source, cx);
        if state.nodes.is_empty() {
            return v_flex()
                .id(state.ids.empty.clone())
                .test_support()
                .gap(dp(12.))
                .child(toolbar)
                .child(source.empty(cx))
                .into_any_element();
        }
        // The inspector moves below once the whole graph can't sit beside it.
        let stacked = self.available < state.display.width.max(560.) + 320.;
        // Beside the inspector the graph always fits; stacked, it scrolls
        // when it is wider than the card (less its 1 px borders).
        let scrolls = stacked && self.available < state.display.width + 2.;
        let panel = render_panel(source, stacked, scrolls, cx);
        let inspector = render_inspector(source, stacked, cx);
        v_flex()
            .gap(dp(14.))
            .child(toolbar)
            .child(
                div()
                    .flex()
                    .when_else(stacked, |this| this.flex_col(), |this| this.flex_row())
                    .items_start()
                    .gap(dp(12.))
                    .child(panel)
                    .child(inspector),
            )
            .into_any_element()
    }
}

fn line() -> Div {
    h_flex().gap(dp(8.)).min_w_0()
}

fn mono(value: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .child(value.into())
        .font_family(MONO_FONT)
        .text_size(dp(12.5))
}

fn muted(value: impl Into<SharedString>, cx: &App) -> Div {
    let p = palette(cx);
    div()
        .min_w_0()
        .child(value.into())
        .text_color(p.muted)
        .text_size(dp(12.))
        .group_hover("fog-control", |style| style.text_color(p.ink_2))
}

fn action(id: impl Into<gpui_kit::ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id)
        .outline()
        .group("fog-control")
        .small()
        .label(label)
}

/// A status glyph, or a dash where there is no status.
fn glyph(tone: Option<Tone>, cx: &App) -> AnyElement {
    match tone {
        None => div()
            .min_w_0()
            .child("—")
            .text_color(palette(cx).muted)
            .into_any_element(),
        Some(tone) => div()
            .children(ui::status_glyph(tone, cx))
            .into_any_element(),
    }
}

fn render_toolbar<S: GraphSource>(source: &S, cx: &mut Context<S>) -> Div {
    let state = source.graph();
    let (page, pages) = (state.page, state.display.pages);
    line()
        .flex_wrap()
        .child(div().flex_1())
        .when(pages > 1, |this| {
            this.child(
                action(state.ids.previous.clone(), "Previous")
                    .disabled(page == 0)
                    .on_click(cx.listener(move |this: &mut S, _, _, cx| {
                        this.graph_mut().set_page(page.saturating_sub(1));
                        cx.notify();
                    })),
            )
            .child(muted(state.display.page_label.clone(), cx))
            .child(
                action(state.ids.next.clone(), "Next")
                    .disabled(page + 1 >= pages)
                    .on_click(cx.listener(move |this: &mut S, _, _, cx| {
                        this.graph_mut().set_page(page + 1);
                        cx.notify();
                    })),
            )
        })
        .child(
            ui::choice(
                action(state.ids.problems.clone(), state.text.problems.clone()),
                state.problems,
            )
            .on_click(cx.listener(|this: &mut S, _, _, cx| {
                let graph = this.graph_mut();
                graph.set_problems(!graph.problems);
                cx.notify();
            })),
        )
}

/// The graph's card: the summary, the graph scrolling sideways inside it,
/// and the legend.
fn render_panel<S: GraphSource>(
    source: &S,
    stacked: bool,
    scrolls: bool,
    cx: &mut Context<S>,
) -> impl IntoElement + use<S> {
    let state = source.graph();
    let p = palette(cx);
    let graph = render_graph(source, cx);
    v_flex()
        .id(state.ids.panel.clone())
        .test_support()
        .when(stacked, |this| this.w_full())
        .flex_1()
        .min_w_0()
        .rounded(px(12.))
        .bg(p.surface)
        .border_1()
        .border_color(p.line)
        .child(
            line()
                .justify_between()
                .p(dp(14.))
                .child(muted(state.display.summary.clone(), cx)),
        )
        // A graph wider than its card is inset like the summary, so it is
        // clipped inside the card's border, with a scrollbar under it that
        // says there is more. One that fits is drawn as it always was.
        .child(
            div()
                .relative()
                .when(scrolls, |this| this.mx(dp(14.)).pb(dp(10.)))
                .min_w_0()
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
                .when(scrolls, |this| {
                    this.child(
                        Scrollbar::horizontal(&state.pan)
                            .id(state.ids.scrollbar.clone())
                            .mode(ScrollbarMode::Always),
                    )
                }),
        )
        .child(
            line()
                .p(dp(14.))
                .flex_wrap()
                .children(
                    [
                        ("healthy", Tone::Good),
                        ("unknown", Tone::Unknown),
                        ("degraded", Tone::Warn),
                        ("failing", Tone::Crit),
                    ]
                    .map(|(label, tone)| {
                        line().child(glyph(Some(tone), cx)).child(muted(label, cx))
                    }),
                )
                .child(muted(state.text.caption.clone(), cx)),
        )
}

/// Calls under the boxes, the selected one last and in the accent; a
/// marker on each call that has one; then the boxes.
fn render_graph<S: GraphSource>(source: &S, cx: &mut Context<S>) -> AnyElement {
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
                .tooltip(edge.tooltip.clone())
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
                .tooltip(node.tooltip.clone())
                .on_click(
                    cx.listener(move |this: &mut S, _, window, cx| this.open(&key, window, cx)),
                )
        }))
        .into_any_element()
}

/// The inspector: its card, with the source's head and the list of calls
/// as its content.
fn render_inspector<S: GraphSource>(
    source: &S,
    stacked: bool,
    cx: &mut Context<S>,
) -> impl IntoElement + use<S> {
    let state = source.graph();
    let content = inspector_content(source, cx);
    let p = palette(cx);
    v_flex()
        .id(state.ids.inspector.clone())
        .test_support()
        .min_w_0()
        .rounded(px(12.))
        .bg(p.surface)
        .border_1()
        .border_color(p.line)
        .when_else(
            stacked,
            |this| this.w_full(),
            |this| this.w(dp(INSPECTOR_W)).flex_none(),
        )
        .child(
            h_flex().h(dp(42.)).px(dp(14.)).flex_none().child(
                div()
                    .min_w_0()
                    .child(state.text.title.clone())
                    .font_weight(ui::HEADING_WEIGHT)
                    .text_size(dp(14.)),
            ),
        )
        .children(content)
}

/// What the inspector shows, apart from its frame: the source's head for
/// the selected call, then how many calls there are and their list.
fn inspector_content<S: GraphSource>(source: &S, cx: &mut Context<S>) -> [Div; 2] {
    let state = source.graph();
    let head = source.inspector_head(state.selected_edge(), cx);
    let list = uniform_list(
        state.ids.connections.clone(),
        state.visible.len(),
        cx.processor(|this: &mut S, range: std::ops::Range<usize>, _, cx| {
            range
                .map(|ix| {
                    let graph = this.graph();
                    render_row(&graph.edges[graph.visible[ix]], graph, cx)
                })
                .collect()
        }),
    )
    .track_scroll(&state.scroll)
    .h(dp(300.))
    .w_full();
    [
        body().child(head),
        body()
            .child(muted(state.display.count.clone(), cx))
            .child(list),
    ]
}

fn body() -> Div {
    v_flex().p(dp(14.)).gap(dp(12.)).min_w_0()
}

fn render_row<S: GraphSource>(
    edge: &GraphEdge<S::Key>,
    state: &super::GraphState<S::Key>,
    cx: &Context<S>,
) -> Div {
    let (from, to) = (edge.from.clone(), edge.to.clone());
    let selected = state.selected.as_ref().is_some_and(|key| edge.is(key));
    div().h(dp(30.)).w_full().child(
        ui::segment(Button::new(edge.row_id.clone()), selected, cx)
            .small()
            .w_full()
            .child(
                line()
                    .w_full()
                    .child(glyph(edge.tone, cx))
                    .child(mono(edge.label.clone()).flex_1().truncate()),
            )
            .tooltip(edge.tooltip.clone())
            .on_click(cx.listener(move |this: &mut S, _, _, cx| {
                this.graph_mut().select(&from, &to);
                cx.notify();
            })),
    )
}
