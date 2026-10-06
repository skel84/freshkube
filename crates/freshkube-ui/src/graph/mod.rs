//! A graph of boxes and the calls between them, as the service map draws
//! it (docs/DESIGN.md#components): a page of boxes laid out by
//! `freshkube_graph::layout::route`, each call along its route with an
//! arrowhead and a marker, and an inspector listing every call.
//!
//! It is shaped like the data table: the page's entity implements
//! [`GraphSource`] and owns a [`GraphState`]; [`GraphView`] draws from
//! them. The state derives everything a frame needs when its data, page,
//! filter or selection changes, so drawing only reads.

use std::collections::BTreeMap;
use std::rc::Rc;

use freshkube_graph::layout::{Node, Route, route};
use gpui_kit::{
    AnyElement, Context, ScrollStrategy, SharedString, UniformListScrollHandle, Window,
};

use crate::ui::Tone;

mod view;

pub use view::GraphView;

/// Boxes on one page of the graph.
pub const PER_PAGE: usize = 24;
/// Calls on a page that carry a marker; the selected call always does.
pub const MARKERS: usize = 24;

/// A box: what it is, its id and how it reads.
#[derive(Clone, Debug)]
pub struct GraphNode<K> {
    pub key: K,
    /// The box's element id, from the source's domain.
    pub id: SharedString,
    pub label: SharedString,
    /// The muted line under the label.
    pub detail: SharedString,
    /// `None` draws a dash where the glyph would be.
    pub tone: Option<Tone>,
    pub tooltip: SharedString,
}

/// A call from one box to another.
#[derive(Clone, Debug)]
pub struct GraphEdge<K> {
    pub from: K,
    pub to: K,
    /// The marker's element id on the graph.
    pub marker_id: SharedString,
    /// Its row's element id in the inspector's list.
    pub row_id: SharedString,
    /// Drawn solid only when `Good`; warnings and failures take their colour.
    pub tone: Option<Tone>,
    /// The line's width in dp.
    pub width: f32,
    pub label: SharedString,
    pub tooltip: SharedString,
}

impl<K: PartialEq> GraphEdge<K> {
    fn is(&self, (from, to): &(K, K)) -> bool {
        &self.from == from && &self.to == to
    }

    /// Whether the problem filter keeps it.
    fn problem(&self) -> bool {
        matches!(self.tone, Some(Tone::Warn | Tone::Crit | Tone::Died))
    }
}

/// What a page that shows a graph supplies; the graph keeps the rest.
pub trait GraphSource: Sized + 'static {
    /// A box's identity; a call is the pair of its ends.
    type Key: Clone + Ord + 'static;

    fn graph(&self) -> &GraphState<Self::Key>;
    fn graph_mut(&mut self) -> &mut GraphState<Self::Key>;

    /// A box drawn with the accent border, such as the page's selection.
    fn highlighted(&self) -> Option<&Self::Key> {
        None
    }

    /// A box click.
    fn open(&mut self, node: &Self::Key, window: &mut Window, cx: &mut Context<Self>);

    /// The top of the inspector: what the selected call says, or a hint
    /// while none is selected. The list of calls follows it.
    fn inspector_head(
        &self,
        selected: Option<&GraphEdge<Self::Key>>,
        cx: &mut Context<Self>,
    ) -> AnyElement;

    /// What the graph shows when it has no boxes.
    fn empty(&self, cx: &mut Context<Self>) -> AnyElement;
}

/// A box placed on the current page.
#[derive(Clone, Copy)]
struct Placed {
    node: usize,
    x: f32,
    y: f32,
}

impl Node for Placed {
    fn position(&self) -> (f32, f32) {
        (self.x, self.y)
    }

    fn set_position(&mut self, x: f32, y: f32) {
        (self.x, self.y) = (x, y);
    }
}

/// A call drawn on the current page, along its route.
#[derive(Clone)]
struct Shown {
    edge: usize,
    route: Route,
}

/// The current page, derived when anything it shows changes.
#[derive(Default)]
struct Display {
    placed: Rc<Vec<Placed>>,
    shown: Rc<Vec<Shown>>,
    /// Indexes into `shown` that carry a marker.
    markers: Vec<usize>,
    /// The selected call's index into `shown`, drawn last.
    selected: Option<usize>,
    width: f32,
    height: f32,
    pages: usize,
    summary: SharedString,
    page_label: SharedString,
    count: SharedString,
}

/// The words a graph draws besides its data.
pub struct GraphText {
    /// What the summary calls the boxes and the calls, such as
    /// `("applications", "connections")`.
    pub nouns: (&'static str, &'static str),
    /// The inspector's title.
    pub title: SharedString,
    /// The problem filter's label.
    pub problems: SharedString,
    /// The line after the legend: how to read the graph.
    pub caption: SharedString,
}

/// The parts' element ids: `<prefix>-graph`, `-panel`, `-scroll`,
/// `-previous`, `-next`, `-problems`, `-inspector`, `-connections` and
/// `-empty`.
struct Ids {
    prefix: SharedString,
    graph: SharedString,
    panel: SharedString,
    scroll: SharedString,
    previous: SharedString,
    next: SharedString,
    problems: SharedString,
    inspector: SharedString,
    connections: SharedString,
    empty: SharedString,
}

impl Ids {
    fn new(prefix: &str) -> Self {
        let id = |part: &str| SharedString::from(format!("{prefix}-{part}"));
        Self {
            prefix: prefix.to_string().into(),
            graph: id("graph"),
            panel: id("panel"),
            scroll: id("scroll"),
            previous: id("previous"),
            next: id("next"),
            problems: id("problems"),
            inspector: id("inspector"),
            connections: id("connections"),
            empty: id("empty"),
        }
    }
}

/// What a graph keeps between frames, owned by the page's entity.
pub struct GraphState<K> {
    nodes: Rc<Vec<GraphNode<K>>>,
    edges: Rc<Vec<GraphEdge<K>>>,
    /// Each edge's ends as node indexes; `None` when an end isn't a box.
    ends: Vec<Option<(usize, usize)>>,
    /// The edges the filter keeps, in the source's order.
    visible: Vec<usize>,
    selected: Option<(K, K)>,
    page: usize,
    problems: bool,
    text: GraphText,
    scroll: UniformListScrollHandle,
    ids: Ids,
    display: Display,
    /// How many times the page was derived, so tests can tell that
    /// drawing never derives it.
    #[cfg(test)]
    prepared: usize,
}

impl<K: Clone + Ord> GraphState<K> {
    /// An empty graph whose parts' ids start with `prefix`.
    pub fn new(prefix: &str, text: GraphText) -> Self {
        let mut state = Self {
            nodes: Rc::default(),
            edges: Rc::default(),
            ends: vec![],
            visible: vec![],
            selected: None,
            page: 0,
            problems: false,
            text,
            scroll: UniformListScrollHandle::new(),
            ids: Ids::new(prefix),
            display: Display::default(),
            #[cfg(test)]
            prepared: 0,
        };
        state.prepare();
        state
    }

    /// New boxes and calls. The selection stays while its call does, and
    /// the page stays while it exists.
    pub fn set(&mut self, nodes: Vec<GraphNode<K>>, edges: Vec<GraphEdge<K>>) {
        let index: BTreeMap<_, _> = nodes
            .iter()
            .enumerate()
            .map(|(ix, node)| (&node.key, ix))
            .collect();
        self.ends = edges
            .iter()
            .map(|edge| Some((*index.get(&edge.from)?, *index.get(&edge.to)?)))
            .collect();
        self.nodes = Rc::new(nodes);
        self.edges = Rc::new(edges);
        self.prepare();
    }

    /// No boxes, no calls, the first page and nothing selected.
    pub fn clear(&mut self) {
        self.selected = None;
        self.page = 0;
        self.set(vec![], vec![]);
    }

    /// Empties the graph while new data loads: the first page and nothing
    /// drawn, with the selection kept for the answer, which keeps it while
    /// its call is still there.
    pub fn reload(&mut self) {
        let selected = self.selected.take();
        self.page = 0;
        self.set(vec![], vec![]);
        self.selected = selected;
    }

    /// Selects a call by its ends and shows the page its caller is on.
    /// Returns whether the call exists.
    pub fn select(&mut self, from: &K, to: &K) -> bool {
        let key = (from.clone(), to.clone());
        let Some(ix) = self.edges.iter().position(|edge| edge.is(&key)) else {
            return false;
        };
        if let Some((from, _)) = self.ends[ix] {
            self.page = from / PER_PAGE;
        }
        self.selected = Some(key);
        self.prepare();
        true
    }

    pub fn set_page(&mut self, page: usize) {
        self.page = page;
        self.prepare();
    }

    /// Shows only calls with a problem, or every call.
    pub fn set_problems(&mut self, problems: bool) {
        self.problems = problems;
        self.prepare();
    }

    /// Scrolls the inspector's list to the `row`th call the filter keeps.
    pub fn reveal(&self, row: usize, strategy: ScrollStrategy) {
        self.scroll.scroll_to_item(row, strategy);
    }

    pub fn selected(&self) -> Option<&(K, K)> {
        self.selected.as_ref()
    }

    pub fn selected_edge(&self) -> Option<&GraphEdge<K>> {
        let key = self.selected.as_ref()?;
        self.edges.iter().find(|edge| edge.is(key))
    }

    pub fn page(&self) -> usize {
        self.page
    }

    pub fn pages(&self) -> usize {
        self.display.pages
    }

    pub fn problems(&self) -> bool {
        self.problems
    }

    pub fn nodes(&self) -> &[GraphNode<K>] {
        &self.nodes
    }

    pub fn edges(&self) -> &[GraphEdge<K>] {
        &self.edges
    }

    /// How many calls the filter keeps.
    pub fn visible_count(&self) -> usize {
        self.visible.len()
    }

    /// The boxes on this page with their top-left corners, in dp.
    pub fn placed(&self) -> impl Iterator<Item = (&GraphNode<K>, f32, f32)> {
        self.display
            .placed
            .iter()
            .map(|placed| (&self.nodes[placed.node], placed.x, placed.y))
    }

    /// The calls drawn on this page, along their routes.
    pub fn shown(&self) -> impl Iterator<Item = (&GraphEdge<K>, &Route)> {
        self.display
            .shown
            .iter()
            .map(|shown| (&self.edges[shown.edge], &shown.route))
    }

    /// The calls on this page that carry a marker.
    pub fn markers(&self) -> impl Iterator<Item = &GraphEdge<K>> {
        self.display
            .markers
            .iter()
            .map(|&ix| &self.edges[self.display.shown[ix].edge])
    }

    pub fn summary(&self) -> &SharedString {
        &self.display.summary
    }

    /// The page's size in dp.
    pub fn size(&self) -> (f32, f32) {
        (self.display.width, self.display.height)
    }

    /// The parts' element ids start with this.
    pub fn id(&self, part: &str) -> SharedString {
        format!("{}-{part}", self.ids.prefix).into()
    }

    /// Derives the page: which boxes it shows, where, and the route of
    /// every call between them. It runs when anything shown changes, never
    /// while drawing.
    fn prepare(&mut self) {
        #[cfg(test)]
        {
            self.prepared += 1;
        }
        self.visible = (0..self.edges.len())
            .filter(|&ix| !self.problems || self.edges[ix].problem())
            .collect();
        if let Some(key) = &self.selected
            && !self.edges.iter().any(|edge| edge.is(key))
        {
            self.selected = None;
        }
        let pages = self.nodes.len().div_ceil(PER_PAGE).max(1);
        self.page = self.page.min(pages - 1);
        let start = self.page * PER_PAGE;
        let mut on_page: Vec<usize> = (start..self.nodes.len().min(start + PER_PAGE)).collect();
        // The selected call stays inspectable across pages.
        if let Some(key) = &self.selected
            && let Some((from, to)) = self
                .edges
                .iter()
                .position(|edge| edge.is(key))
                .and_then(|ix| self.ends[ix])
        {
            for node in [from, to] {
                if !on_page.contains(&node) {
                    on_page.push(node);
                }
            }
        }
        let local: BTreeMap<usize, usize> = on_page
            .iter()
            .enumerate()
            .map(|(local, &node)| (node, local))
            .collect();
        let mut placed: Vec<_> = on_page
            .iter()
            .map(|&node| Placed { node, x: 0., y: 0. })
            .collect();
        // Lay out by every call between these boxes, not only the filtered
        // ones, so the filter doesn't move the boxes.
        let mut routed = BTreeMap::new();
        let links: Vec<_> = self
            .ends
            .iter()
            .flatten()
            .filter_map(|(from, to)| Some((*local.get(from)?, *local.get(to)?)))
            .filter(|&link| routed.insert(link, routed.len()).is_none())
            .collect();
        let layout = route(&mut placed, &links);
        let shown: Vec<_> = self
            .visible
            .iter()
            .filter_map(|&edge| {
                let (from, to) = self.ends[edge]?;
                let link = (*local.get(&from)?, *local.get(&to)?);
                Some(Shown {
                    edge,
                    route: layout.routes[routed[&link]].clone(),
                })
            })
            .collect();
        let mut markers: Vec<_> = (0..shown.len().min(MARKERS)).collect();
        let selected = self.selected.as_ref().and_then(|key| {
            shown
                .iter()
                .position(|shown| self.edges[shown.edge].is(key))
        });
        if let Some(ix) = selected
            && !markers.contains(&ix)
        {
            markers.push(ix);
        }
        let (boxes, calls) = self.text.nouns;
        self.display = Display {
            summary: format!(
                "{} of {} {boxes} · {} of {} filtered {calls} drawn",
                placed.len(),
                self.nodes.len(),
                shown.len(),
                self.visible.len()
            )
            .into(),
            page_label: format!("Page {} of {}", self.page + 1, pages).into(),
            count: format!("{} of {} {calls}", self.visible.len(), self.edges.len()).into(),
            placed: Rc::new(placed),
            shown: Rc::new(shown),
            markers,
            selected,
            width: layout.width,
            height: layout.height,
            pages,
        };
    }
}

#[cfg(test)]
mod tests;
