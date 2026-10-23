use super::*;

#[test]
fn a_pill_replaces_the_marker_and_every_call_with_one_is_drawn() {
    let mut state = chain();
    assert_eq!(state.markers().count(), 0);
    assert_eq!(state.display.pills.len(), 5);
    // Past the marker cap, pills are still all drawn.
    let (nodes, mut edges) = invented(60, 200);
    for edge in &mut edges {
        edge.pill = Some("key".into());
    }
    state.set(nodes, edges);
    assert_eq!(state.display.pills.len(), state.shown().count());
    assert_eq!(state.markers().count(), 0);
    // A call with no pill keeps its marker, within the cap.
    let (nodes, mut edges) = invented(60, 200);
    edges[0].pill = Some("key".into());
    state.set(nodes, edges);
    assert_eq!(state.display.pills.len(), 1);
    assert_eq!(state.markers().count(), MARKERS);
}

#[test]
fn a_chain_keeps_its_side_branch_in_the_column_after_its_anchor() {
    let state = chain();
    let at = |key: usize| boxes(&state).into_iter().find(|b| b.0 == key).unwrap();
    // The branch hangs from hop 1, so it shares hop 2's column.
    assert_eq!(at(5).1, at(2).1);
    assert!(at(5).2 != at(2).2);
    assert!(at(1).1 < at(5).1);
}

#[test]
fn the_state_walks_boxes_and_calls_among_what_it_draws() {
    let state = chain();
    let mut at = None;
    let mut seen = vec![];
    while let Some(next) = state.step(at.as_ref(), Step::Next) {
        if let Cursor::Node(key) = &next {
            seen.push(*key);
        }
        at = Some(next);
    }
    assert_eq!(seen.len(), 6);
    assert_eq!(&seen[..2], &[0, 1]);
    // The branch is a column-mate of hop 2.
    assert!(seen.contains(&5));
    assert_eq!(
        state.step(Some(&Cursor::Node(5)), Step::Back),
        Some(Cursor::Link(1, 5))
    );
    assert_eq!(
        state.step(Some(&Cursor::Link(1, 5)), Step::Back),
        Some(Cursor::Node(1))
    );
}

#[gpui_kit::test]
fn a_pill_selects_its_call_and_a_ghost_is_a_box_like_any_other(cx: &mut TestAppContext) {
    let (handle, map) = open_chain(cx, 1280., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for edge in map.read(cx).graph.edges().to_vec() {
            assert!(window.try_find(edge.marker_id.clone()).is_some());
        }
        window.click("map-edge-1--2", cx);
        assert_eq!(map.read(cx).graph.selected(), Some(&(1, 2)));
        window.click("map-node-5", cx);
        assert_eq!(map.read(cx).opened, vec![5]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn each_pill_sits_in_its_gutter_and_a_long_key_is_cut_to_fit_at_text_20(cx: &mut TestAppContext) {
    let mut graph = chain();
    let (nodes, mut edges) = (graph.nodes().to_vec(), graph.edges().to_vec());
    edges[1].pill =
        Some("sha256:9f3a7c1d0b2e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a".into());
    graph.set(nodes, edges);
    let graph = cx.update(|cx| graph.with_keys(cx));
    let (handle, _) = open(cx, graph, 1280., Some(20.));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let unit = crate::ui::dp_px(1., window);
        for (from, to) in [(0, 1), (1, 2), (2, 3), (3, 4), (1, 5)] {
            let pill = window.find(format!("map-edge-{from}--{to}")).bounds();
            let (a, b) = (
                window.find(format!("map-node-{from}")).bounds(),
                window.find(format!("map-node-{to}")).bounds(),
            );
            assert!(pill.left() >= a.right(), "{from}->{to} clear of its caller");
            assert!(pill.right() <= b.left(), "{from}->{to} clear of its callee");
            assert!(
                pill.size.width <= unit * 100. + px(1.),
                "{from}->{to} fits 100 dp"
            );
        }
    })
    .unwrap();
}
