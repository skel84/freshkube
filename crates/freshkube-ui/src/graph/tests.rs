use freshkube_graph::layout::{NODE_H, NODE_W, point_at};
use gpui_kit::component::Root;
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
    opened: Vec<usize>,
    width: f32,
}

impl GraphSource for Map {
    type Key = usize;

    fn graph(&self) -> &GraphState<usize> {
        &self.graph
    }

    fn graph_mut(&mut self) -> &mut GraphState<usize> {
        &mut self.graph
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

    fn empty(&self, _: &mut Context<Self>) -> AnyElement {
        div().child("No services").into_any_element()
    }
}

impl Render for Map {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p(dp(16.))
            .child(GraphView::new(self.width).render(self, cx))
    }
}

fn open(
    cx: &mut TestAppContext,
    graph: GraphState<usize>,
    width: f32,
    text: Option<f32>,
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
        // The content width in dp, as a page hands it over.
        let available = (width - 32.) * crate::ui::BASE_TEXT / f32::from(window.rem_size());
        let map = cx.new(|_| Map {
            graph,
            opened: vec![],
            width: available,
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
        assert!(viewport.left() >= panel.left());
        assert!(viewport.right() <= panel.right());
        assert!(before.size.width > viewport.size.width);
        // Stacked: the inspector sits under the graph's card.
        assert!(window.find("map-inspector").bounds().top() >= panel.bottom());
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
