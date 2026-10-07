//! Drawing a graph from its source: the toolbar, then one card holding
//! the shared inspector split, with the graph, its summary and legend on
//! the leading side and the inspector beside it, or under it once the
//! whole graph can't sit beside it.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Div, PathBuilder, SharedString, TestSupportExt, Window, canvas, div,
    point, px, uniform_list,
};

use freshkube_graph::layout::{NODE_H, NODE_W};

use super::{GraphEdge, GraphSource, STACKED};
use crate::inspector::{self, Inspector};
use crate::page::PANE_PADDING;
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, Tone, dp};

/// The summary's line and the legend's, each padded 14 above and below.
const LINE_H: f32 = 48.;
/// The room under a graph that scrolls sideways, for its scrollbar.
const SCROLLBAR_H: f32 = 10.;
/// The least height of the card with the inspector beside the graph, so
/// the inspector shows its head and a few calls.
const BESIDE_HEIGHT: f32 = 460.;

/// Draws a [`GraphSource`]'s graph. Like the data table, it is a plain
/// struct: the source's entity owns the state and the view only reads it.
pub struct GraphView {
    /// The width the graph's card may take, in dp.
    available: f32,
    /// The page's width, in dp, which the inspector's split compares with
    /// [`inspector::SPLIT_WIDTH`].
    page_width: f32,
}

impl GraphView {
    /// A view `available` dp wide, such as the page's content width.
    pub fn new(available: f32) -> Self {
        Self {
            available,
            page_width: available,
        }
    }

    /// The page's width in dp, when it is wider than what the card may take,
    /// such as `screens::page_width` on a padded page.
    pub fn page_width(mut self, width: f32) -> Self {
        self.page_width = width;
        self
    }

    pub fn render<S: GraphSource>(
        self,
        source: &S,
        window: &Window,
        cx: &mut Context<S>,
    ) -> AnyElement {
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
        let split = source.graph_split();
        let display = &state.display;
        // The inspector sits beside the graph only on a wide page, and only
        // while the whole graph fits next to its least width in the split's
        // room, inside the card's borders; otherwise it opens under the
        // graph. Beside, it takes at most the rest, however wide it was
        // dragged before, and a drag stops at the graph: a saved width that
        // outgrew the room would otherwise stack it for good, with no handle
        // to narrow it (#425).
        let room = self.available - 2.;
        let beside = self.page_width >= inspector::SPLIT_WIDTH
            && room >= display.width + inspector::MIN_WIDTH;
        if beside {
            split.keep_lead(display.width, room);
        }
        // Stacked, the graph scrolls when it is wider than the card.
        let scrolls = !beside && self.available < display.width + 2.;
        let lead = LINE_H * 2. + display.height + if scrolls { SCROLLBAR_H } else { 0. };
        let height = if beside {
            lead.max(BESIDE_HEIGHT)
        } else {
            let lead = lead.clamp(STACKED.lead.least, STACKED.lead.most.unwrap_or(lead));
            split.lead_start(lead, cx);
            lead + STACKED.trail.start
        };
        let graph = render_lead(source, scrolls, cx).into_any_element();
        let inspector = render_inspector(source, cx);
        let p = palette(cx);
        v_flex()
            .gap(dp(14.))
            .child(toolbar)
            .child(
                v_flex()
                    .id(state.ids.panel.clone())
                    .test_support()
                    .w_full()
                    // The panes' heights, inside the card's borders.
                    .h(ui::dp_px(height, window) + px(2.))
                    .min_w_0()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .bg(p.surface)
                    .border_1()
                    .border_color(p.line)
                    .child(inspector::split(
                        state.ids.split.clone(),
                        split,
                        beside,
                        graph,
                        Some(inspector),
                        window,
                    )),
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

/// The split's leading side: the summary, the graph scrolling sideways
/// inside it, and the legend. It scrolls down on its own when the graph
/// and legend are taller than the most the card gives it; only a vertical
/// wheel scrolls it, so a sideways one always reaches the graph.
fn render_lead<S: GraphSource>(
    source: &S,
    scrolls: bool,
    cx: &mut Context<S>,
) -> impl IntoElement + use<S> {
    let state = source.graph();
    let graph = render_graph(source, cx);
    v_flex()
        .id(state.ids.lead.clone())
        .test_support()
        .size_full()
        .min_w_0()
        .min_h_0()
        .overflow_y_scroll()
        .restrict_scroll_to_axis()
        .child(
            line()
                .flex_none()
                .justify_between()
                .p(dp(14.))
                .child(muted(state.display.summary.clone(), cx)),
        )
        // A graph wider than its side is inset like the summary, so it is
        // clipped inside the card's border, with a scrollbar under it that
        // says there is more. One that fits is drawn as it always was.
        .child(
            div()
                .relative()
                .flex_none()
                .when(scrolls, |this| this.mx(dp(14.)).pb(dp(SCROLLBAR_H)))
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
                .id(state.ids.legend.clone())
                .test_support()
                .flex_none()
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

/// The shared inspector: the graph's title as its heading; the source's
/// head for the selected call, how many calls there are and their list,
/// which takes the height left; and the source's footer.
fn render_inspector<S: GraphSource>(source: &S, cx: &mut Context<S>) -> AnyElement {
    let state = source.graph();
    let selected = state.selected_edge();
    let head = source.inspector_head(selected, cx);
    let footer = source.inspector_footer(selected, cx);
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
    .flex_1()
    .min_h_0()
    .w_full();
    let content = v_flex()
        .size_full()
        .min_w_0()
        .min_h_0()
        .gap(dp(12.))
        .p(dp(PANE_PADDING))
        .child(div().flex_none().min_w_0().child(head))
        .child(muted(state.display.count.clone(), cx).flex_none())
        .child(list);
    Inspector::new(state.ids.inspector.clone())
        .heading(
            div()
                .min_w_0()
                .child(state.text.title.clone())
                .font_weight(ui::HEADING_WEIGHT)
                .text_size(dp(14.)),
        )
        .content(content)
        .footer(footer)
        .render(cx)
        .into_any_element()
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
            .when(!edge.tooltip.is_empty(), |this| {
                this.tooltip(edge.tooltip.clone())
            })
            .on_click(cx.listener(move |this: &mut S, _, _, cx| {
                this.graph_mut().select(&from, &to);
                cx.notify();
            })),
    )
}
