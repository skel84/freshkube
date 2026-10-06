//! `freshkube-graph`'s layout on invented services: boxes in columns,
//! callers left of what they call, and a line with an arrowhead for each
//! call. Its shapes are the cases the layout handles: a chain, a fan-out
//! that wraps, a cycle and boxes with no connection. Every call keeps clear
//! of the boxes, and the header counts where calls cross.

use std::rc::Rc;

use freshkube_graph::layout::{self, NODE_H, NODE_W, Node, Route};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use freshkube_ui::ui::{self, MONO_FONT, Tone, dp};
use gpui_kit::component::button::Button;
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyView, App, Context, Div, PathBuilder, Role, SharedString, TestSupportExt, Window, canvas,
    div, point, px,
};

/// The id prefix of everything the story draws.
const PREFIX: &str = "graph";

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|_| GraphStory::new()).into()
}

/// A service by name, namespace and tone.
type Service = (String, &'static str, Tone);
/// A call from one service to another, by index, with its tone.
type Link = (usize, usize, Tone);

/// The graphs the story offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A shop's calls, three columns deep.
    Shop,
    /// One gateway calling twelve services: its column wraps.
    FanOut,
    /// Three services calling each other in a ring.
    Cycle,
    /// A few calls and services with none.
    Loose,
}

impl Shape {
    const ALL: [(Shape, &'static str, &'static str); 4] = [
        (Shape::Shop, "shop", "Shop"),
        (Shape::FanOut, "fan-out", "Fan-out"),
        (Shape::Cycle, "cycle", "Cycle"),
        (Shape::Loose, "loose", "Unconnected"),
    ];

    /// Its services, by name, namespace and tone, and its calls between them.
    fn graph(self) -> (Vec<Service>, Vec<Link>) {
        let named = |names: &[&str], namespace| {
            names
                .iter()
                .map(|name| (name.to_string(), namespace, Tone::Good))
                .collect::<Vec<_>>()
        };
        match self {
            Shape::Shop => {
                let mut nodes = named(
                    &[
                        "shop-web",
                        "shop-api",
                        "checkout",
                        "search",
                        "shop-redis",
                        "ledger-sync",
                        "payments-db",
                        "mailer",
                    ],
                    "shop",
                );
                nodes[2].2 = Tone::Warn;
                nodes[6].2 = Tone::Crit;
                let calls = vec![
                    (0, 1, Tone::Good),
                    (1, 2, Tone::Good),
                    (1, 3, Tone::Good),
                    (1, 4, Tone::Good),
                    (2, 5, Tone::Warn),
                    (2, 6, Tone::Crit),
                    (5, 6, Tone::Good),
                    (2, 7, Tone::Good),
                ];
                (nodes, calls)
            }
            Shape::FanOut => {
                let mut nodes = vec![("gateway".to_string(), "app-a", Tone::Good)];
                nodes.extend((1..=12).map(|ix| (format!("service-{ix:02}"), "app-a", Tone::Good)));
                nodes[5].2 = Tone::Warn;
                let calls = (1..=12).map(|to| (0, to, Tone::Good)).collect();
                (nodes, calls)
            }
            Shape::Cycle => {
                let nodes = named(&["shop-api", "checkout", "ledger-sync"], "shop");
                let calls = vec![(0, 1, Tone::Good), (1, 2, Tone::Good), (2, 0, Tone::Warn)];
                (nodes, calls)
            }
            Shape::Loose => {
                let mut nodes = named(&["shop-web", "shop-api", "shop-redis"], "shop");
                nodes.extend(named(&["batch-report", "cron-cleanup"], "app-b"));
                nodes[4].2 = Tone::Unknown;
                let calls = vec![(0, 1, Tone::Good), (1, 2, Tone::Good)];
                (nodes, calls)
            }
        }
    }
}

/// A placed box.
#[derive(Clone)]
pub struct GraphNode {
    id: SharedString,
    label: SharedString,
    namespace: SharedString,
    tone: Tone,
    x: f32,
    y: f32,
}

impl Node for GraphNode {
    fn position(&self) -> (f32, f32) {
        (self.x, self.y)
    }

    fn set_position(&mut self, x: f32, y: f32) {
        (self.x, self.y) = (x, y);
    }
}

/// A call's line and tone.
struct Call {
    route: Route,
    tone: Tone,
}

pub struct GraphStory {
    shape: Shape,
    /// Placed and routed when the shape changes; drawing only reads them.
    nodes: Rc<Vec<GraphNode>>,
    calls: Rc<Vec<Call>>,
    width: f32,
    height: f32,
    crossings: usize,
}

impl GraphStory {
    pub fn new() -> Self {
        let mut story = Self {
            shape: Shape::Shop,
            nodes: Rc::default(),
            calls: Rc::default(),
            width: 0.,
            height: 0.,
            crossings: 0,
        };
        story.place();
        story
    }

    pub fn shape(&self) -> Shape {
        self.shape
    }

    /// How many times the drawn calls cross one another.
    pub fn crossings(&self) -> usize {
        self.crossings
    }

    /// Each box's label and top-left corner, in dp.
    pub fn positions(&self) -> Vec<(SharedString, f32, f32)> {
        self.nodes
            .iter()
            .map(|node| (node.label.clone(), node.x, node.y))
            .collect()
    }

    fn set_shape(&mut self, shape: Shape, cx: &mut Context<Self>) {
        if shape != self.shape {
            self.shape = shape;
            self.place();
            cx.notify();
        }
    }

    fn place(&mut self) {
        let (nodes, calls) = self.shape.graph();
        let mut nodes: Vec<GraphNode> = nodes
            .into_iter()
            .enumerate()
            .map(|(ix, (label, namespace, tone))| GraphNode {
                id: SharedString::from(format!("{PREFIX}-node-{ix}")),
                label: label.into(),
                namespace: namespace.into(),
                tone,
                x: 0.,
                y: 0.,
            })
            .collect();
        let links: Vec<_> = calls.iter().map(|&(from, to, _)| (from, to)).collect();
        let placed = layout::route(&mut nodes, &links);
        (self.width, self.height) = (placed.width, placed.height);
        let routes = placed.routes;
        self.crossings = layout::crossings(&routes);
        self.nodes = Rc::new(nodes);
        self.calls = Rc::new(
            routes
                .into_iter()
                .zip(calls)
                .map(|(route, (_, _, tone))| Call { route, tone })
                .collect(),
        );
    }

    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let shapes = Shape::ALL.map(|(shape, id, label)| {
            ui::segment(
                Button::new(SharedString::from(format!("{PREFIX}-shape-{id}"))),
                self.shape == shape,
                cx,
            )
            .small()
            .label(label)
            .on_click(cx.listener(move |this, _, _, cx| this.set_shape(shape, cx)))
        });
        PageHeader::new(PREFIX, "Service map")
            .secondary(track(shapes, cx))
            .meta([div()
                .child(format!(
                    "{} services · {} calls · {} crossings",
                    self.nodes.len(),
                    self.calls.len(),
                    self.crossings
                ))
                .into_any_element()])
            .render(window, cx)
    }

    /// The calls, under the boxes: a line and an arrowhead each, dashed
    /// when the call has a problem.
    fn render_calls(&self, cx: &App) -> impl IntoElement + use<> {
        let p = palette(cx);
        let calls = self.calls.clone();
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let unit = ui::dp_px(1., window);
                let at =
                    |(x, y): (f32, f32)| point(bounds.left() + unit * x, bounds.top() + unit * y);
                for call in calls.iter() {
                    let color = match call.tone {
                        Tone::Crit => p.crit,
                        Tone::Warn => p.warn,
                        _ => p.line_strong,
                    };
                    let mut path = PathBuilder::stroke(unit * 1.1);
                    if call.tone != Tone::Good {
                        let dash = unit * 5.;
                        path = path.dash_array(&[dash, dash * 0.6]);
                    }
                    path.move_to(at(call.route.segments[0][0]));
                    for &[_, b, c, d] in &call.route.segments {
                        path.cubic_bezier_to(at(d), at(b), at(c));
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                    let [tip, left, right] = call.route.head();
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
        .size_full()
    }

    fn render_graph(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        #[cfg(debug_assertions)]
        {
            static STORY: freshkube_probe::first_frame::FirstFrame =
                freshkube_probe::first_frame::FirstFrame::new("graph");
            if STORY.pending() {
                window.on_next_frame(|_, _| STORY.mark());
            }
        }
        #[cfg(not(debug_assertions))]
        let _ = window;
        let p = palette(cx);
        let graph = div()
            .id(SharedString::from(format!("{PREFIX}-canvas")))
            .test_support()
            .role(Role::Group)
            .relative()
            .flex_none()
            .w(dp(self.width))
            .h(dp(self.height))
            .child(self.render_calls(cx))
            .children(self.nodes.iter().map(|node| {
                h_flex()
                    .id(node.id.clone())
                    .test_support()
                    .absolute()
                    .left(dp(node.x))
                    .top(dp(node.y))
                    .w(dp(NODE_W))
                    .h(dp(NODE_H))
                    .px(dp(10.))
                    .gap(dp(8.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(p.line_strong)
                    .bg(p.surface_2)
                    .children(ui::status_glyph(node.tone, cx))
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .font_family(MONO_FONT)
                                    .font_weight(ui::HEADING_WEIGHT)
                                    .child(node.label.clone()),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .text_size(dp(12.))
                                    .text_color(p.muted)
                                    .child(node.namespace.clone()),
                            ),
                    )
            }));
        // The map scrolls both ways inside its card when it is larger.
        page::inset().flex().flex_col().flex_1().min_h_0().child(
            page::card(cx).flex_1().min_h_0().child(
                div()
                    .id(SharedString::from(format!("{PREFIX}-scroll")))
                    .size_full()
                    .overflow_scroll()
                    .child(graph),
            ),
        )
    }
}

/// Segmented options on a track.
fn track<const N: usize>(options: [Button; N], cx: &App) -> Div {
    h_flex()
        .gap(dp(2.))
        .p(dp(3.))
        .rounded(px(8.))
        .bg(palette(cx).surface_2)
        .children(options)
}

impl Default for GraphStory {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for GraphStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(window, cx);
        let graph = self.render_graph(window, cx);
        page::page(SharedString::from(format!("{PREFIX}-page")))
            .child(page::toolbar(cx).child(header))
            .child(graph)
    }
}
