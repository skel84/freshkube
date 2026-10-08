//! The graph view (`freshkube_ui::graph`) on invented services: boxes in
//! columns, callers left of what they call, a line with an arrowhead and a
//! marker for each call, and the list of calls beside them. Its shapes are
//! the cases the layout handles: a chain, a fan-out that wraps, a cycle and
//! boxes with no connection. Every call keeps clear of the boxes, and the
//! header counts where calls cross.

use freshkube_graph::layout;
use freshkube_ui::graph::{
    GraphEdge, GraphNode, GraphSource, GraphState, GraphText, GraphView, inspector_split,
};
use freshkube_ui::inspector::InspectorSplit;
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::palette::palette;
use freshkube_ui::ui::{self, MONO_FONT, Tone, dp};
use gpui_kit::component::button::Button;
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, AnyView, App, Context, Div, SharedString, Window, div, px};

/// The id prefix of everything the story draws.
const PREFIX: &str = "graph";

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(GraphStory::new).into()
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

/// The story's graph: a shape's services as boxes, drawn by the same
/// [`GraphView`] as the service map.
pub struct GraphStory {
    shape: Shape,
    graph: GraphState<usize>,
    /// The graph and its inspector; a dragged width isn't saved.
    split: InspectorSplit,
    /// Placed and counted when the shape changes; drawing only reads them.
    crossings: usize,
    meta: SharedString,
}

impl GraphStory {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut story = Self {
            shape: Shape::Shop,
            graph: GraphState::new(
                PREFIX,
                GraphText {
                    nouns: ("services", "calls"),
                    title: "Calls".into(),
                    problems: "Problem calls".into(),
                    caption: "Callers on the left · dashed lines have a problem".into(),
                },
            ),
            split: inspector_split("workbench-graph", cx),
            crossings: 0,
            meta: SharedString::default(),
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
        self.graph
            .placed()
            .map(|(node, x, y)| (node.label.clone(), x, y))
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
        let (services, calls) = self.shape.graph();
        let nodes = services
            .into_iter()
            .enumerate()
            .map(|(ix, (label, namespace, tone))| GraphNode {
                key: ix,
                id: SharedString::from(format!("{PREFIX}-node-{ix}")),
                tooltip: label.clone().into(),
                label: label.into(),
                detail: namespace.into(),
                tone: Some(tone),
            })
            .collect::<Vec<_>>();
        let edges = calls
            .iter()
            .map(|&(from, to, tone)| GraphEdge {
                from,
                to,
                marker_id: SharedString::from(format!("{PREFIX}-call-{from}--{to}")),
                row_id: SharedString::from(format!("{PREFIX}-row-{from}--{to}")),
                tone: Some(tone),
                width: 1.1,
                label: format!("{} → {}", nodes[from].label, nodes[to].label).into(),
                tooltip: SharedString::default(),
            })
            .collect();
        self.graph.set(nodes, edges);
        let routes: Vec<_> = self.graph.shown().map(|(_, route)| route.clone()).collect();
        self.crossings = layout::crossings(&routes);
        self.meta = format!(
            "{} services · {} calls · {} crossings",
            self.graph.nodes().len(),
            self.graph.edges().len(),
            self.crossings
        )
        .into();
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
            .meta([div().child(self.meta.clone()).into_any_element()])
            .render(window, cx)
    }
}

impl GraphSource for GraphStory {
    type Key = usize;

    fn graph(&self) -> &GraphState<usize> {
        &self.graph
    }

    fn graph_mut(&mut self) -> &mut GraphState<usize> {
        &mut self.graph
    }

    fn graph_split(&self) -> &InspectorSplit {
        &self.split
    }

    /// The story has nothing to open; a box shows only its hover.
    fn open(&mut self, _: &usize, _: &mut Window, _: &mut Context<Self>) {}

    fn inspector_head(
        &self,
        selected: Option<&GraphEdge<usize>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        match selected {
            Some(edge) => div()
                .font_family(MONO_FONT)
                .child(edge.label.clone())
                .into_any_element(),
            None => div()
                .text_color(p.muted)
                .child("Choose a call")
                .into_any_element(),
        }
    }

    fn empty(&self, _: &mut Context<Self>) -> AnyElement {
        div().child("No services").into_any_element()
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

impl Render for GraphStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(debug_assertions)]
        {
            static STORY: freshkube_probe::first_frame::FirstFrame =
                freshkube_probe::first_frame::FirstFrame::new("graph");
            if STORY.pending() {
                window.on_next_frame(|_, _| STORY.mark());
            }
        }
        let header = self.render_header(window, cx);
        // The story's width beside the list of stories, and inside its inset.
        let width = window.viewport_size().width / ui::dp_px(1., window) - crate::LIST_WIDTH;
        let graph = GraphView::new(width - page::PANE_PADDING * 2.)
            .page_width(width)
            .render(self, window, cx);
        page::page(SharedString::from(format!("{PREFIX}-page")))
            .child(page::toolbar(cx).child(header))
            .child(
                div()
                    .id(SharedString::from(format!("{PREFIX}-body")))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .child(page::inset().child(graph)),
            )
    }
}
