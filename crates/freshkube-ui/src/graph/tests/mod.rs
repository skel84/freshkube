use freshkube_graph::layout::{NODE_H, NODE_W, point_at};
use gpui_kit::component::Root;
use gpui_kit::component::button::Button;
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyElement, AnyWindowHandle, Context, Entity, ScrollDelta, ScrollStrategy, TestAppContext,
    TestSupportExt, Window, div, point, px, size,
};

use super::*;
use crate::split_size::{MemorySizes, SizeStore as _, set_store};
use crate::ui::{Tone, dp};

mod chain;
mod inspector_layout;
mod keyboard;
mod paging;
mod rendering;
mod selection;
mod tooltips;
mod unpaged;

/// The page the tests' split saves under.
const MAP: &str = "map";

/// Invented services and the calls between them: `nodes` boxes, `edges`
/// calls spread over them by a fixed sequence, every seventh a warning
/// and every eleventh failing.
fn invented(nodes: usize, edges: usize) -> (Vec<GraphNode<usize>>, Vec<GraphEdge<usize>>) {
    let boxes = (0..nodes)
        .map(|ix| GraphNode {
            key: ix,
            id: format!("map-node-{ix}").into(),
            label: format!("service-{ix:03}").into(),
            detail: "app-a".into(),
            tone: Some(Tone::Good),
            tooltip: format!("service-{ix:03}").into(),
            ghost: false,
        })
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    let mut seed = 7u64;
    let mut calls = vec![];
    while calls.len() < edges {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let from = (seed >> 33) as usize % nodes;
        let to = (seed >> 17) as usize % nodes;
        if from == to || !seen.insert((from, to)) {
            continue;
        }
        let n = calls.len();
        let tone = if n % 11 == 0 {
            Tone::Crit
        } else if n % 7 == 0 {
            Tone::Warn
        } else {
            Tone::Good
        };
        calls.push(GraphEdge {
            from,
            to,
            marker_id: format!("map-edge-{from}--{to}").into(),
            row_id: format!("map-link-{from}--{to}").into(),
            tone: Some(tone),
            width: 1.1,
            label: format!("service-{from:03} → service-{to:03}").into(),
            tooltip: SharedString::default(),
            style: EdgeStyle::Auto,
            pill: None,
            pill_tone: None,
        });
    }
    (boxes, calls)
}

fn text() -> GraphText {
    GraphText {
        nouns: ("services", "calls"),
        title: "Calls".into(),
        problems: "Problem calls".into(),
        caption: "Callers on the left".into(),
    }
}

fn state(nodes: usize, edges: usize) -> GraphState<usize> {
    let mut state = GraphState::new("map", text());
    let (nodes, edges) = invented(nodes, edges);
    state.set(nodes, edges);
    state
}

/// The boxes on the current page, by key and top-left corner.
fn boxes(state: &GraphState<usize>) -> Vec<(usize, f32, f32)> {
    state
        .placed()
        .map(|(node, x, y)| (node.key, x, y))
        .collect()
}

/// A fingerprint of every page of a graph: its boxes, size and the routes
/// of the calls it draws.
fn fingerprint(state: &mut GraphState<usize>) -> u64 {
    let mut text = String::new();
    for page in 0..state.pages() {
        state.set_page(page);
        text += &format!("{:?}{:?}", boxes(state), state.size());
        for (edge, route) in state.shown() {
            text += &format!("{}{:?}", edge.marker_id, route);
        }
    }
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

const PINNED: [u64; 3] = [0x4cb1ffc4ce2485e6, 0x5a36a5ac4317f9a6, 0x0b3bd8c5e6daeac5];

/// A page showing the graph: it records the boxes it was asked to open.
struct Map {
    graph: GraphState<usize>,
    split: InspectorSplit,
    opened: Vec<usize>,
    /// Where the keyboard is, for a graph with keys.
    cursor: Option<Cursor<usize>>,
    highlighted: Option<usize>,
    /// Where the split saves its width, as the app's file would.
    sizes: Rc<MemorySizes>,
}

impl GraphSource for Map {
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

    fn highlighted(&self) -> Option<&usize> {
        self.highlighted.as_ref()
    }

    fn open(&mut self, node: &usize, _: &mut Window, cx: &mut Context<Self>) {
        self.opened.push(*node);
        cx.notify();
    }

    fn cursor(&self) -> Option<Cursor<usize>> {
        self.cursor.clone()
    }

    fn step_to(&mut self, cursor: Cursor<usize>, _: &mut Window, _: &mut Context<Self>) {
        if let Cursor::Link(from, to) = &cursor {
            self.graph.select(from, to);
        }
        self.cursor = Some(cursor);
    }

    fn inspector_head(
        &self,
        selected: Option<&GraphEdge<usize>>,
        _: &mut Context<Self>,
    ) -> AnyElement {
        match selected {
            Some(edge) => div()
                .id("map-selected")
                .test_support()
                .child(edge.label.clone())
                .into_any_element(),
            None => div().child("Choose a call").into_any_element(),
        }
    }

    fn inspector_footer(
        &self,
        selected: Option<&GraphEdge<usize>>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let from = selected?.from;
        Some(
            Button::new("map-open")
                .label("Open caller")
                .on_click(cx.listener(move |this: &mut Self, _, _, cx| {
                    this.opened.push(from);
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    fn empty(&self, _: &mut Context<Self>) -> AnyElement {
        div().child("No services").into_any_element()
    }
}

impl Render for Map {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The content width in dp, inside the page's padding, as a page
        // hands it over.
        let available = window.viewport_size().width / crate::ui::dp_px(1., window) - 32.;
        div()
            .size_full()
            .p(dp(16.))
            .child(GraphView::new(available).render(self, window, cx))
    }
}

fn open(
    cx: &mut TestAppContext,
    graph: GraphState<usize>,
    width: f32,
    text: Option<f32>,
) -> (AnyWindowHandle, Entity<Map>) {
    open_saved(cx, graph, width, text, None)
}

/// [`open`] with the inspector's width a page saved before.
fn open_saved(
    cx: &mut TestAppContext,
    graph: GraphState<usize>,
    width: f32,
    text: Option<f32>,
    saved: Option<f32>,
) -> (AnyWindowHandle, Entity<Map>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        cx.set_reduce_motion(true);
        if let Some(text) = text {
            crate::text_size::set(text, cx);
        }
    });
    let sizes = Rc::new(match saved {
        Some(width) => MemorySizes::with(inspector::width_key(MAP), width),
        None => MemorySizes::default(),
    });
    cx.update(|cx| set_store(sizes.clone(), cx));
    let mut view = None;
    // Tall enough to show the inspector stacked under the graph.
    let handle = cx.open_window(size(px(width), px(2000.)), |window, cx| {
        let map = cx.new(|cx| Map {
            graph,
            split: inspector_split(MAP, cx),
            opened: vec![],
            cursor: None,
            highlighted: None,
            sizes,
        });
        view = Some(map.clone());
        Root::new(map, window, cx)
    });
    cx.run_until_parked();
    (handle.into(), view.unwrap())
}

/// An element's bounds.
fn bounds(window: &mut Window, id: &'static str) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    window.find(id).bounds()
}

/// A chain with labels and the three line styles, and a side branch:
///
/// ```text
/// 0 → 1 → 2 → 3 → 4      (1 → 5 is the branch, a ghost)
/// ```
fn chain() -> GraphState<usize> {
    let nodes = (0..6)
        .map(|ix| GraphNode {
            key: ix,
            id: format!("map-node-{ix}").into(),
            label: format!("hop-{ix}").into(),
            detail: "Detail".into(),
            tone: Some(if ix == 5 { Tone::Unknown } else { Tone::Good }),
            tooltip: SharedString::default(),
            ghost: ix == 5,
        })
        .collect();
    let call = |from: usize, to: usize, style, pill: Option<&str>| GraphEdge {
        from,
        to,
        marker_id: format!("map-edge-{from}--{to}").into(),
        row_id: format!("map-link-{from}--{to}").into(),
        tone: None,
        width: 1.1,
        label: format!("hop-{from} → hop-{to}").into(),
        tooltip: SharedString::default(),
        style,
        pill: pill.map(Into::into),
        pill_tone: None,
    };
    let edges = vec![
        call(0, 1, EdgeStyle::Solid, Some("3f9a1c2")),
        call(1, 2, EdgeStyle::Dashed, Some("sha256:9")),
        call(2, 3, EdgeStyle::Dotted, Some("no key")),
        call(3, 4, EdgeStyle::Solid, Some("a1b2c3d")),
        call(1, 5, EdgeStyle::Dotted, Some("no key")),
    ];
    let mut state = GraphState::new("map", text()).unpaged();
    state.set(nodes, edges);
    state
}

/// A graph with keys, in a window wide enough to show the whole chain.
fn open_chain(
    cx: &mut TestAppContext,
    width: f32,
    text: Option<f32>,
) -> (AnyWindowHandle, Entity<Map>) {
    cx.update(bind_keys);
    let graph = cx.update(|cx| chain().with_keys(cx));
    open(cx, graph, width, text)
}

/// Moves the pointer to `position` with no button down.
fn hover(window: &mut Window, position: gpui_kit::Point<gpui_kit::Pixels>, cx: &mut gpui_kit::App) {
    use gpui_kit::{InputEvent, MouseMoveEvent};
    window.dispatch_event(
        MouseMoveEvent {
            position,
            ..Default::default()
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}
