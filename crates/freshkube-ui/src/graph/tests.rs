use std::cell::RefCell;

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
use crate::ui::{Tone, dp};

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

#[test]
fn pages_hold_twenty_four_boxes_and_count_what_they_draw() {
    let state = state(120, 300);
    assert_eq!(state.pages(), 5);
    assert_eq!(state.placed().count(), PER_PAGE);
    assert_eq!(state.visible_count(), 300);
    assert!(state.markers().count() <= MARKERS);
    assert!(state.summary().starts_with("24 of 120 services · "));
    assert!(state.summary().ends_with(" of 300 filtered calls drawn"));
    assert_eq!(state.display.page_label.as_ref(), "Page 1 of 5");
    assert_eq!(state.display.count.as_ref(), "300 of 300 calls");
}

#[test]
fn every_call_can_be_selected_and_its_ends_stay_on_every_page() {
    let mut state = state(120, 300);
    for ix in 0..state.edges().len() {
        let (from, to) = (state.edges()[ix].from, state.edges()[ix].to);
        assert!(state.select(&from, &to));
        assert_eq!(state.page(), from / PER_PAGE);
        for page in 0..state.pages() {
            state.set_page(page);
            let placed = boxes(&state);
            assert!(placed.iter().any(|&(key, ..)| key == from));
            assert!(placed.iter().any(|&(key, ..)| key == to));
            assert!(placed.len() <= PER_PAGE + 2);
            assert!(
                state
                    .markers()
                    .any(|edge| (edge.from, edge.to) == (from, to))
            );
            assert!(state.markers().count() <= MARKERS + 1);
        }
    }
}

#[test]
fn new_data_keeps_a_selection_that_still_exists_and_drops_one_that_does_not() {
    let mut state = state(120, 300);
    let (from, to) = (state.edges()[42].from, state.edges()[42].to);
    state.select(&from, &to);
    let (mut nodes, mut edges) = invented(120, 300);
    nodes.reverse();
    edges.reverse();
    state.set(nodes.clone(), edges);
    assert_eq!(state.selected(), Some(&(from, to)));
    // While new data loads, the selection waits for the answer.
    state.reload();
    assert_eq!((state.page(), state.placed().count()), (0, 0));
    assert_eq!(state.selected(), Some(&(from, to)));
    state.set(nodes, vec![]);
    assert_eq!(state.selected(), None);
    state.set_page(4);
    state.clear();
    assert_eq!((state.page(), state.pages()), (0, 1));
    assert_eq!(state.placed().count(), 0);
    assert!(!state.select(&from, &to));
}

#[test]
fn the_problem_filter_hides_calls_without_moving_a_box() {
    let mut state = state(120, 300);
    let before = boxes(&state);
    state.set_problems(true);
    assert_eq!(boxes(&state), before);
    let problems = state
        .edges()
        .iter()
        .filter(|edge| matches!(edge.tone, Some(Tone::Warn | Tone::Crit)))
        .count();
    assert_eq!(state.visible_count(), problems);
    assert!(state.shown().all(|(edge, _)| edge.tone != Some(Tone::Good)));
}

#[test]
fn no_call_runs_through_a_box_on_any_page() {
    let mut state = state(120, 300);
    for page in 0..state.pages() {
        state.set_page(page);
        let placed = boxes(&state);
        assert!(state.shown().count() > 0);
        for (edge, route) in state.shown() {
            for segment in &route.segments {
                for step in 0..=16 {
                    let (x, y) = point_at(segment, step as f32 / 16.);
                    let inside = placed.iter().any(|&(_, bx, by)| {
                        x > bx + 0.5
                            && x < bx + NODE_W - 0.5
                            && y > by + 0.5
                            && y < by + NODE_H - 0.5
                    });
                    assert!(!inside, "page {page}: {} at {x}, {y}", edge.label);
                }
            }
        }
    }
}

/// A page showing the graph: it records the boxes it was asked to open.
struct Map {
    graph: GraphState<usize>,
    split: InspectorSplit,
    opened: Vec<usize>,
    /// Widths the split was dragged to, as a page would save them.
    remembered: Rc<RefCell<Vec<f32>>>,
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

    fn open(&mut self, node: &usize, _: &mut Window, cx: &mut Context<Self>) {
        self.opened.push(*node);
        cx.notify();
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
    let mut view = None;
    // Tall enough to show the inspector stacked under the graph.
    let handle = cx.open_window(size(px(width), px(2000.)), |window, cx| {
        let remembered = Rc::new(RefCell::new(vec![]));
        let map = cx.new(|cx| Map {
            graph,
            split: inspector_split(
                saved,
                {
                    let remembered = remembered.clone();
                    move |width, _| remembered.borrow_mut().push(width)
                },
                cx,
            ),
            opened: vec![],
            remembered,
        });
        view = Some(map.clone());
        Root::new(map, window, cx)
    });
    cx.run_until_parked();
    (handle.into(), view.unwrap())
}

#[gpui_kit::test]
fn a_box_click_opens_it_and_a_row_click_selects_its_call_and_page(cx: &mut TestAppContext) {
    let (handle, map) = open(cx, state(120, 300), 1280., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("map-node-3", cx);
        assert_eq!(map.read(cx).opened, vec![3]);
        window.click("map-next", cx);
        assert_eq!(map.read(cx).graph.page(), 1);
        window.render_frame(cx);
        window.click("map-previous", cx);
        assert_eq!(map.read(cx).graph.page(), 0);
        // The last call in the list, whose caller is on another page.
        let last = map.read(cx).graph.edges().len() - 1;
        map.read(cx).graph.reveal(last, ScrollStrategy::Bottom);
        window.render_frame(cx);
        let edge = map.read(cx).graph.edges()[last].clone();
        window.click(edge.row_id.clone(), cx);
        let graph = &map.read(cx).graph;
        assert_eq!(graph.selected(), Some(&(edge.from, edge.to)));
        assert_eq!(graph.page(), edge.from / PER_PAGE);
        window.render_frame(cx);
        assert!(window.try_find("map-selected").is_some());
        assert!(window.try_find(edge.marker_id.clone()).is_some());
        window.click("map-problems", cx);
        assert!(map.read(cx).graph.problems());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_marker_click_selects_its_call(cx: &mut TestAppContext) {
    let (handle, map) = open(cx, state(120, 300), 1280., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let edge = map.read(cx).graph.markers().next().unwrap().clone();
        window.click(edge.marker_id.clone(), cx);
        assert_eq!(map.read(cx).graph.selected(), Some(&(edge.from, edge.to)));
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_empty_graph_shows_the_source_s_message(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, GraphState::new("map", text()), 1280., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("map-empty").is_some());
        assert!(window.try_find("map-graph").is_none());
        assert!(window.try_find("map-next").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn drawing_never_derives_the_page(cx: &mut TestAppContext) {
    let (handle, map) = open(cx, state(120, 300), 1280., None);
    let before = cx.update(|cx| map.read(cx).graph.prepared);
    cx.update_window(handle, |_, window, cx| {
        for _ in 0..5 {
            window.render_frame(cx);
        }
    })
    .unwrap();
    assert_eq!(cx.update(|cx| map.read(cx).graph.prepared), before);
    cx.update(|cx| map.update(cx, |map, _| map.graph.set_page(2)));
    assert_eq!(cx.update(|cx| map.read(cx).graph.prepared), before + 1);
}

#[gpui_kit::test]
fn a_narrow_graph_scrolls_inside_its_card_at_large_text(cx: &mut TestAppContext) {
    let (handle, map) = open(cx, state(120, 300), 760., Some(20.));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = window.find("map-panel").bounds();
        let viewport = window.find("map-scroll").bounds();
        let before = window.find("map-graph").bounds();
        assert!(panel.right() <= px(760.));
        // The graph is clipped inside the card's border, inset like its
        // summary, not at the border itself.
        assert!(viewport.left() - panel.left() >= px(14.));
        assert!(panel.right() - viewport.right() >= px(14.));
        assert!(before.size.width > viewport.size.width);
        // Stacked: the inspector sits under the graph, inside its card.
        let inspector = window.find("map-inspector").bounds();
        assert!(inspector.top() >= window.find("map-lead").bounds().bottom());
        assert!(inspector.bottom() <= panel.bottom() + px(1.));
        let node = map
            .read(cx)
            .graph
            .placed()
            .min_by(|a, b| (a.1 + a.2).total_cmp(&(b.1 + b.2)))
            .unwrap()
            .0
            .id
            .clone();
        window.scroll(node, ScrollDelta::Pixels(point(px(-1000.), px(0.))), cx);
        window.render_frame(cx);
        assert!(window.find("map-graph").bounds().left() < before.left());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_graph_that_fits_fills_its_card_without_an_inset(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, state(7, 8), 1280., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = window.find("map-panel").bounds();
        let viewport = window.find("map-scroll").bounds();
        let graph = window.find("map-graph").bounds();
        assert!(
            graph.size.width <= viewport.size.width,
            "graph {:?}, viewport {:?}, panel {:?}",
            graph.size.width,
            viewport.size.width,
            panel.size.width
        );
        assert!(
            viewport.left() - panel.left() < px(2.),
            "viewport {viewport:?}, panel {panel:?}, graph {graph:?}"
        );
        assert!(panel.right() - viewport.right() < px(2.));
    })
    .unwrap();
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

/// A sideways wheel scroll of `x` at `position`, in ten steps with a frame
/// after each, as a trackpad sends it.
fn wheel_x(
    window: &mut Window,
    position: gpui_kit::Point<gpui_kit::Pixels>,
    x: f32,
    cx: &mut gpui_kit::App,
) {
    use gpui_kit::{InputEvent, ScrollWheelEvent, TouchPhase};
    for step in 0..10 {
        window.dispatch_event(
            ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(x / 10.), px(0.))),
                touch_phase: if step == 0 {
                    TouchPhase::Started
                } else {
                    TouchPhase::Moved
                },
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    }
}

thread_local! {
    static BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Counts every tooltip built: Kit's overlay builds its tooltip again each
/// time it draws, and GPUI's own `.tooltip()` builds one when it shows.
fn count_tooltips(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.observe_new::<gpui_kit::component::tooltip::Tooltip>(|_, _, _| {
            BUILT.with(|built| built.set(built.get() + 1))
        })
        .detach()
    });
}

/// How many tooltips have been built so far, and how many times `text`
/// has been drawn through `FollowTooltip`.
fn marks(text: &str) -> (usize, usize) {
    (BUILT.with(|built| built.get()), crate::tooltip::drawn(text))
}

/// Whether a tooltip has shown since `since`, counting the next frame: one
/// was built, or `FollowTooltip` drew `text`. New entities are announced
/// once the update ends, so the count is read after it.
fn shown_since(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    text: &str,
    since: (usize, usize),
) -> bool {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    let (built, drawn) = marks(text);
    built > since.0 || drawn > since.1
}

/// Opens the narrow graph at text size 20, rests the pointer near the top
/// right corner of its top-left box, clear of the markers, and returns that
/// box's tooltip and the pointer's position.
fn hover_first_box(
    cx: &mut TestAppContext,
) -> (
    AnyWindowHandle,
    Entity<Map>,
    SharedString,
    gpui_kit::Point<gpui_kit::Pixels>,
) {
    let (handle, map) = open(cx, state(120, 300), 760., Some(20.));
    count_tooltips(cx);
    let (tooltip, position) = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let node = map
                .read(cx)
                .graph
                .placed()
                .min_by(|a, b| (a.1 + a.2).total_cmp(&(b.1 + b.2)))
                .unwrap()
                .0
                .clone();
            let bounds = window.find(node.id.clone()).bounds();
            let position = point(bounds.right() - px(6.), bounds.top() + px(8.));
            hover(window, position, cx);
            (node.tooltip, position)
        })
        .unwrap();
    (handle, map, tooltip, position)
}

/// Whether a box or a marker of the graph is under `position`.
fn under(window: &Window, map: &Map, position: gpui_kit::Point<gpui_kit::Pixels>) -> bool {
    let boxes = map.graph.placed().map(|(node, ..)| node.id.clone());
    let markers = map.graph.markers().map(|edge| edge.marker_id.clone());
    boxes
        .chain(markers)
        .filter_map(|id| window.try_find(id))
        .any(|found| found.bounds().contains(&position))
}

/// A box's tooltip hides when a sideways scroll moves the box from under a
/// pointer that stays still.
#[gpui_kit::test]
fn a_wheel_scroll_hides_a_shown_box_tooltip(cx: &mut TestAppContext) {
    let (handle, map, text, position) = hover_first_box(cx);
    let before = marks(&text);
    // Past the tooltip's show delay.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    assert!(
        shown_since(cx, handle, &text, before),
        "the box's tooltip never showed"
    );
    cx.update_window(handle, |_, window, cx| {
        wheel_x(window, position, -30., cx);
        assert!(!under(window, map.read(cx), position), "still over a box");
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    assert!(
        !shown_since(cx, handle, &text, marks(&text)),
        "the tooltip stayed after the scroll"
    );
}

/// A tooltip still waiting to show when a sideways scroll moves its box from
/// under a still pointer never shows (#192).
#[gpui_kit::test]
fn a_box_tooltip_waiting_to_show_does_not_show_after_its_box_scrolls_away(cx: &mut TestAppContext) {
    let (handle, map, text, position) = hover_first_box(cx);
    let before = marks(&text);
    // Before the show delay ends.
    cx.update_window(handle, |_, window, cx| {
        wheel_x(window, position, -30., cx);
        assert!(!under(window, map.read(cx), position), "still over a box");
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    assert!(
        !shown_since(cx, handle, &text, before),
        "the box's tooltip showed after it scrolled away"
    );
}

/// Rests the pointer on the middle of element `id` past the show delay, and
/// returns whether a tooltip showed.
fn hover_shows(cx: &mut TestAppContext, handle: AnyWindowHandle, id: SharedString) -> bool {
    let before = marks("");
    cx.update_window(handle, |_, window, cx| {
        let position = window.find(id).bounds().center();
        hover(window, position, cx);
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    shown_since(cx, handle, "", before)
}

/// A box, marker or row with an empty tooltip shows none, not an empty
/// popup; a box with text still shows its tooltip.
#[gpui_kit::test]
fn an_empty_tooltip_shows_nothing(cx: &mut TestAppContext) {
    let mut graph = GraphState::new("map", text());
    let (mut nodes, edges) = invented(7, 8);
    nodes[0].tooltip = SharedString::default();
    assert!(edges.iter().all(|edge| edge.tooltip.is_empty()));
    let marker = edges[0].marker_id.clone();
    let row = edges[0].row_id.clone();
    graph.set(nodes, edges);
    let (handle, _) = open(cx, graph, 1280., None);
    count_tooltips(cx);
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    for id in ["map-node-0".into(), marker, row] {
        assert!(
            !hover_shows(cx, handle, id.clone()),
            "{id} showed a tooltip"
        );
    }
    assert!(
        hover_shows(cx, handle, "map-node-1".into()),
        "a box with text showed no tooltip"
    );
}

/// An element's bounds.
fn bounds(window: &mut Window, id: &'static str) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    window.find(id).bounds()
}

/// The inspector sits beside a graph that fits next to it on a wide page;
/// a graph too wide for that, or a page under 900, puts it under the
/// graph, in the same card.
#[gpui_kit::test]
fn the_inspector_sits_beside_only_a_graph_that_fits(cx: &mut TestAppContext) {
    // Four columns, 744 dp wide, fit beside the inspector at 1280.
    let (handle, _) = open(cx, state(8, 10), 1280., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let (lead, inspector) = (bounds(window, "map-lead"), bounds(window, "map-inspector"));
        let panel = bounds(window, "map-panel");
        assert!(inspector.left() >= lead.right(), "{lead:?} {inspector:?}");
        assert!((inspector.top() - lead.top()).abs() < px(1.));
        assert!(inspector.right() <= panel.right() + px(1.));
        // Nothing of the graph's side is cut off.
        assert!(bounds(window, "map-legend").bottom() <= lead.bottom() + px(1.));
    })
    .unwrap();
    // Six columns, 1120 dp wide, don't fit beside it at 1280; the graph's
    // side then keeps its least height.
    let (handle, _) = open(cx, state(7, 8), 1280., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let (lead, inspector) = (bounds(window, "map-lead"), bounds(window, "map-inspector"));
        assert!(inspector.top() >= lead.bottom(), "{lead:?} {inspector:?}");
        assert!((inspector.left() - lead.left()).abs() < px(1.));
        assert!((lead.size.height - px(300.)).abs() < px(1.), "{lead:?}");
        assert!(bounds(window, "map-legend").bottom() <= lead.bottom() + px(1.));
    })
    .unwrap();
    // Under 900 the inspector opens below even a graph that would fit.
    let (handle, _) = open(cx, state(3, 2), 880., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let (lead, inspector) = (bounds(window, "map-lead"), bounds(window, "map-inspector"));
        assert!(inspector.top() >= lead.bottom(), "{lead:?} {inspector:?}");
    })
    .unwrap();
}

/// Stacked, the graph's side starts as tall as the graph with its summary,
/// legend and scrollbar, within 300 and 700, and the inspector takes 380
/// under it. Past 700 the side scrolls down to its legend.
#[gpui_kit::test]
fn stacked_the_graph_s_side_follows_the_graph_s_height(cx: &mut TestAppContext) {
    // Six rows, 466 dp high and 744 wide, so it scrolls sideways at 760.
    let (handle, map) = open(cx, state(18, 9), 760., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let graph = map.read(cx).graph.display.height;
        let (lead, inspector) = (bounds(window, "map-lead"), bounds(window, "map-inspector"));
        let expected = px(graph + 2. * 48. + 10.);
        assert!(
            (lead.size.height - expected).abs() < px(1.),
            "{lead:?} for a graph {graph} high"
        );
        assert!(
            (inspector.size.height - px(380.)).abs() < px(1.),
            "{inspector:?}"
        );
        assert!(bounds(window, "map-legend").bottom() <= lead.bottom() + px(1.));
    })
    .unwrap();
    // Eight rows, 618 high: the side stops at 700 and scrolls.
    let (handle, _) = open(cx, state(120, 300), 760., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let lead = bounds(window, "map-lead");
        assert!((lead.size.height - px(700.)).abs() < px(1.), "{lead:?}");
        assert!(bounds(window, "map-legend").bottom() > lead.bottom());
        // A wheel over the summary; over the graph it scrolls sideways.
        use gpui_kit::{InputEvent, ScrollWheelEvent};
        window.dispatch_event(
            ScrollWheelEvent {
                position: lead.origin + point(px(20.), px(20.)),
                delta: ScrollDelta::Pixels(point(px(0.), px(-200.))),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(bounds(window, "map-legend").bottom() <= lead.bottom() + px(1.));
    })
    .unwrap();
}

/// The footer shows the source's action for the selected call, and none
/// while nothing is selected.
#[gpui_kit::test]
fn the_footer_acts_on_the_selected_call(cx: &mut TestAppContext) {
    let (handle, map) = open(cx, state(8, 10), 1280., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("map-inspector-footer").is_none());
        let edge = map.read(cx).graph.markers().next().unwrap().clone();
        window.click(edge.marker_id.clone(), cx);
        window.render_frame(cx);
        assert!(window.try_find("map-inspector-footer").is_some());
        window.click("map-open", cx);
        assert_eq!(map.read(cx).opened, vec![edge.from]);
    })
    .unwrap();
}

/// A drag on the hairline beside the graph is reported once, in dp, for
/// the source to save.
#[gpui_kit::test]
fn a_dragged_inspector_width_is_reported_once(cx: &mut TestAppContext) {
    let (handle, map) = open(cx, state(8, 10), 1280., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(map.read(cx).split.width(), inspector::MIN_WIDTH);
        let state = map.read(cx).split.beside_state().clone();
        state.update(cx, |state, cx| state.resize_panel(1, px(400.), window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(*map.read(cx).remembered.borrow(), vec![400.]));
}

/// A saved width wider than the room beside the graph used to stack the
/// inspector for good, with no handle to narrow it (#425). It opens beside
/// the whole graph and takes the rest.
#[gpui_kit::test]
fn an_oversized_saved_width_opens_beside_the_graph_capped(cx: &mut TestAppContext) {
    let (handle, map) = open_saved(cx, state(12, 20), 2000., None, Some(1200.));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let graph = map.read(cx).graph.display.width;
        let lead = window.find("map-lead").bounds();
        let inspector = window.find("map-inspector").bounds();
        assert!(
            graph + 2. + inspector::MIN_WIDTH <= 1968.,
            "the graph ({graph} dp) leaves the inspector room"
        );
        assert_eq!(lead.top(), inspector.top(), "beside, not stacked");
        assert!(
            lead.size.width >= crate::ui::dp_px(graph, window),
            "the whole graph shows: {lead:?} for {graph} dp"
        );
        assert!(inspector.left() >= lead.right() - px(1.));
    })
    .unwrap();
}

/// A drag stops where the graph starts, so the width saved fits beside it.
#[gpui_kit::test]
fn a_drag_stops_at_the_graph(cx: &mut TestAppContext) {
    let (handle, map) = open(cx, state(12, 20), 2000., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let inspector = window.find("map-inspector").bounds();
        let y = inspector.center().y;
        window.drag(point(inspector.left(), y), point(px(40.), y), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let graph = map.read(cx).graph.display.width;
        let remembered = map.read(cx).remembered.borrow().clone();
        assert_eq!(remembered.len(), 1, "{remembered:?}");
        // The room inside the page's padding, less the graph and its card.
        let rest = 1968. - graph - 2.;
        assert!(
            (remembered[0] - rest).abs() <= 1.,
            "saved {}, the room beside the graph is {rest}",
            remembered[0]
        );
        let lead = window.find("map-lead").bounds();
        let inspector = window.find("map-inspector").bounds();
        assert_eq!(lead.top(), inspector.top(), "still beside");
        assert!(lead.size.width >= crate::ui::dp_px(graph, window));
    })
    .unwrap();
}
