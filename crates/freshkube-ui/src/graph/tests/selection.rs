use super::*;

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

#[gpui_kit::test]
fn the_service_map_keeps_its_highlighted_box_and_hover_paint(cx: &mut TestAppContext) {
    let (handle, map) = open(cx, state(8, 12), 1280., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let bounds = window
            .find("map-node-0")
            .bounds()
            .scale(window.scale_factor());
        let paint = |window: &Window| {
            window
                .painted_quads()
                .iter()
                .filter(|quad| quad.bounds == bounds)
                .map(|quad| (quad.background, quad.border_color, quad.border_widths))
                .collect::<Vec<_>>()
        };
        window.hover("map-node-0", cx);
        let ordinary_hover = paint(window);
        assert!(!ordinary_hover.is_empty());
        map.update(cx, |map, cx| {
            map.highlighted = Some(0);
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(
            paint(window),
            ordinary_hover,
            "highlighted map boxes retain the original hover treatment"
        );
        hover(window, point(px(2.), px(2.)), cx);
        let p = crate::palette::palette(cx);
        assert!(
            paint(window)
                .iter()
                .any(|(_, color, widths)| *color == p.accent_line
                    && widths.left.0 == window.scale_factor())
        );
        assert!(
            paint(window)
                .iter()
                .any(|(background, _, _)| *background == p.surface_2.into())
        );
    })
    .unwrap();
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
