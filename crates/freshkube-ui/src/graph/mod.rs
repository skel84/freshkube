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

use freshkube_graph::layout::{Metrics, Route, route_with};
use gpui_kit::{
    AnyElement, App, Context, FocusHandle, KeyBinding, ScrollHandle, ScrollStrategy, SharedString,
    UniformListScrollHandle, Window,
};

use crate::inspector::{self, InspectorSplit, Pane, Stacked};
use crate::split_size::SplitSize;
use crate::ui::{self, Tone};

mod canvas;
mod keys;
mod view;

pub use keys::{Cursor, Step};
pub use view::GraphView;

/// Boxes on one page of the graph.
pub const PER_PAGE: usize = 24;
/// The gutter between columns of a graph drawn with [`GraphState::unpaged`],
/// wide enough for a label on each line.
pub const LABELLED_GUTTER: f32 = 84.;
/// The room `reveal_node` leaves beside a box it scrolls to, in dp.
const REVEAL_MARGIN: f32 = 24.;
/// Calls on a page that carry a marker; the selected call always does.
pub const MARKERS: usize = 24;

gpui_kit::actions!(graph, [WalkBack, WalkForward, WalkPrevious, WalkNext]);

/// The key context of a graph drawn with [`GraphState::with_keys`].
pub(super) const CONTEXT: &str = "Graph";

/// Binds the arrow keys in [`CONTEXT`]. A graph answers them only once it
/// has keys; the app calls this where it draws such a graph.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", WalkBack, Some(CONTEXT)),
        KeyBinding::new("right", WalkForward, Some(CONTEXT)),
        KeyBinding::new("up", WalkPrevious, Some(CONTEXT)),
        KeyBinding::new("down", WalkNext, Some(CONTEXT)),
    ]);
}

/// Stacked, the graph starts as tall as it draws, within 300 and 700, and
/// the inspector under it starts 380 high, keeping 220.
pub const STACKED: Stacked = Stacked {
    lead: Pane::new(300.).least(300.).most(700.),
    trail: Pane::new(inspector::STACKED_HEIGHT).least(inspector::MIN_HEIGHT),
};

/// The split a graph's source keeps on `page`: its inspector starts at the
/// width the user left it at, saved under [`inspector::width_key`], or the
/// least an inspector keeps, so a graph of three columns still fits beside
/// it at 1280.
pub fn inspector_split(page: &'static str, cx: &mut App) -> InspectorSplit {
    let key = inspector::width_key(page);
    let least = inspector::MIN_WIDTH;
    InspectorSplit::with_width(SplitSize::new(key, least, least, cx), cx).stacked(STACKED)
}

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
    /// Empty shows no tooltip.
    pub tooltip: SharedString,
    /// A box for something that should be there and was not found: its
    /// outline is dashed. `false` draws an ordinary box.
    pub ghost: bool,
}

/// How a call's line is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EdgeStyle {
    /// Solid only when the call's tone is `Good`, dashed otherwise: what
    /// the service map draws.
    #[default]
    Auto,
    Solid,
    Dashed,
    Dotted,
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
    /// The marker's and the row's tooltip; empty shows none.
    pub tooltip: SharedString,
    /// How the line is drawn.
    pub style: EdgeStyle,
    /// A label in a pill at the line's middle, which selects the call when
    /// clicked; the call then carries no marker. `None` draws the marker.
    pub pill: Option<SharedString>,
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
    /// The split between the graph and its inspector, from
    /// [`inspector_split`].
    fn graph_split(&self) -> &InspectorSplit;

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

    /// The inspector's footer for the selected call, such as its one
    /// action; none by default.
    fn inspector_footer(
        &self,
        _selected: Option<&GraphEdge<Self::Key>>,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    /// Where the keyboard is: the selected box or call, for a graph with
    /// keys. `None` before the first key.
    fn cursor(&self) -> Option<Cursor<Self::Key>> {
        None
    }

    /// An arrow key moved the keyboard to `cursor`. The source selects it:
    /// a call is `graph_mut().select(..)`; what a box selects is its own.
    /// The graph scrolls the box or the call's far end into view after it.
    fn step_to(
        &mut self,
        _cursor: Cursor<Self::Key>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }

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
    /// Indexes into `shown` that carry a label pill instead.
    pills: Vec<usize>,
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

/// The parts' element ids: `<prefix>-graph`, `-panel` (the card),
/// `-split`, `-lead` (the graph's side of it), `-scroll`, `-scrollbar`,
/// `-legend`, `-previous`, `-next`, `-problems`, `-inspector` (with the
/// inspector's `-heading`, `-content` and `-footer`), `-connections` and
/// `-empty`.
struct Ids {
    prefix: SharedString,
    graph: SharedString,
    panel: SharedString,
    split: SharedString,
    lead: SharedString,
    legend: SharedString,
    scroll: SharedString,
    scrollbar: SharedString,
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
            split: id("split"),
            lead: id("lead"),
            legend: id("legend"),
            scroll: id("scroll"),
            scrollbar: id("scrollbar"),
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
    /// Boxes on one page.
    per_page: usize,
    /// The sizes the layout uses.
    metrics: Metrics,
    problems: bool,
    text: GraphText,
    scroll: UniformListScrollHandle,
    /// The graph's sideways scroll inside its card.
    pan: ScrollHandle,
    /// Present once the graph has keys.
    focus: Option<FocusHandle>,
    ids: Ids,
    display: Display,
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
            per_page: PER_PAGE,
            metrics: Metrics::MAP,
            problems: false,
            text,
            scroll: UniformListScrollHandle::new(),
            pan: ScrollHandle::new(),
            focus: None,
            ids: Ids::new(prefix),
            display: Display::default(),
        };
        state.prepare();
        state
    }

    /// Draws every box on one page, in columns that never wrap and a gutter
    /// wide enough for a label on each line. A graph is paged and wraps by
    /// default, as the service map does.
    #[must_use]
    pub fn unpaged(mut self) -> Self {
        self.per_page = usize::MAX;
        self.metrics = Metrics::labelled(LABELLED_GUTTER);
        self.prepare();
        self
    }

    /// Answers the arrow keys bound by [`bind_keys`]: the graph takes focus
    /// as one stop, and a key asks its source to move the cursor.
    #[must_use]
    pub fn with_keys(mut self, cx: &mut App) -> Self {
        self.focus = Some(cx.focus_handle().tab_stop(true));
        self
    }

    /// The graph's focus handle, once it has keys.
    pub fn focus_handle(&self) -> Option<&FocusHandle> {
        self.focus.as_ref()
    }

    /// Where `step` goes from `from`, among the boxes and calls on this
    /// page. `None` where the key goes nowhere.
    pub(super) fn step(&self, from: Option<&Cursor<K>>, step: Step) -> Option<Cursor<K>> {
        let nodes: Vec<_> = self
            .placed()
            .map(|(node, x, y)| (node.key.clone(), x, y))
            .collect();
        let calls: Vec<_> = self
            .display
            .shown
            .iter()
            .map(|shown| &self.edges[shown.edge])
            .map(|edge| (edge.from.clone(), edge.to.clone()))
            .collect();
        keys::Walk {
            nodes: &nodes,
            calls: &calls,
        }
        .step(from, step)
    }

    /// Scrolls the graph sideways so the box `key` is in view, with a
    /// little room beside it. It does nothing while the graph fits or the
    /// box isn't on this page.
    pub(super) fn reveal_node(&self, key: &K, window: &Window) {
        let Some((_, x, _)) = self.placed().find(|(node, ..)| node.key == *key) else {
            return;
        };
        let unit = ui::dp_px(1., window);
        let view = self.pan.bounds().size.width;
        // Before the first layout the pan has no width to scroll within.
        if view <= gpui_kit::px(0.) {
            return;
        }
        let offset = self.pan.offset();
        let shown = keys::scroll_to_show(
            -offset.x / unit,
            view / unit,
            x,
            x + freshkube_graph::layout::NODE_W,
            REVEAL_MARGIN,
        );
        self.pan
            .set_offset(gpui_kit::point(-shown * unit, offset.y));
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
            self.page = from / self.per_page;
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
        freshkube_probe::probe::hit("graph.prepare");
        self.visible = (0..self.edges.len())
            .filter(|&ix| !self.problems || self.edges[ix].problem())
            .collect();
        if let Some(key) = &self.selected
            && !self.edges.iter().any(|edge| edge.is(key))
        {
            self.selected = None;
        }
        let pages = self.nodes.len().div_ceil(self.per_page).max(1);
        self.page = self.page.min(pages - 1);
        let start = self.page * self.per_page;
        let end = self.nodes.len().min(start.saturating_add(self.per_page));
        let mut on_page: Vec<usize> = (start..end).collect();
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
        let mut corners = vec![(0., 0.); on_page.len()];
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
        let layout = route_with(&self.metrics, &mut corners, &links);
        let placed: Vec<_> = on_page
            .iter()
            .zip(&corners)
            .map(|(&node, &(x, y))| Placed { node, x, y })
            .collect();
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
        let has_pill: Vec<bool> = shown
            .iter()
            .map(|shown| self.edges[shown.edge].pill.is_some())
            .collect();
        let pills: Vec<_> = (0..shown.len()).filter(|&ix| has_pill[ix]).collect();
        let mut markers: Vec<_> = (0..shown.len())
            .filter(|&ix| !has_pill[ix])
            .take(MARKERS)
            .collect();
        let selected = self.selected.as_ref().and_then(|key| {
            shown
                .iter()
                .position(|shown| self.edges[shown.edge].is(key))
        });
        if let Some(ix) = selected
            && !markers.contains(&ix)
            && !has_pill[ix]
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
            pills,
            selected,
            width: layout.width,
            height: layout.height,
            pages,
        };
    }
}

#[cfg(test)]
mod tests;
