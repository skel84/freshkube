use super::*;

#[test]
fn source_order_survives_unpaged_in_either_builder_order() {
    let (nodes, edges) = invented(30, 60);
    let mut a = GraphState::new("map", text()).in_source_order().unpaged();
    let mut b = GraphState::new("map", text()).unpaged().in_source_order();
    a.set(nodes.clone(), edges.clone());
    b.set(nodes, edges);
    assert!(a.metrics.preserve_order);
    assert_eq!(a.metrics, b.metrics);
    assert_eq!(fingerprint(&mut a), fingerprint(&mut b));
    for (ix, (_, x, y)) in boxes(&a).iter().enumerate() {
        for (_, next_x, next_y) in &boxes(&a)[ix + 1..] {
            if x == next_x {
                assert!(y < next_y, "the graph keeps source order in each column");
            }
        }
    }
    assert!(!state(8, 12).metrics.preserve_order);
}

#[test]
fn an_unpaged_graph_draws_every_box_in_columns_that_never_wrap() {
    let (nodes, edges) = invented(120, 300);
    let mut state = GraphState::new("map", text()).unpaged();
    state.set(nodes, edges);
    assert_eq!(state.pages(), 1);
    assert_eq!(state.placed().count(), 120);
    assert_eq!(state.shown().count(), 300);
    assert!(state.summary().starts_with("120 of 120 services · "));
    assert_eq!(state.display.page_label.as_ref(), "Page 1 of 1");
    // Columns sit a labelled gutter apart.
    let pitch = NODE_W + LABELLED_GUTTER;
    for (_, x, _) in state.placed() {
        let columns = (x - 16.) / pitch;
        assert!((columns - columns.round()).abs() < 1e-3);
    }
    // Selecting a call keeps the page and both ends drawn.
    let (from, to) = (state.edges()[0].from, state.edges()[0].to);
    assert!(state.select(&from, &to));
    assert_eq!(state.page(), 0);
    assert_eq!(state.placed().count(), 120);
}

#[test]
fn an_unpaged_graph_with_one_caller_of_forty_keeps_them_in_one_column() {
    let (nodes, _) = invented(41, 0);
    let edges = (1..41)
        .map(|to| GraphEdge {
            from: 0,
            to,
            marker_id: format!("map-edge-0--{to}").into(),
            row_id: format!("map-link-0--{to}").into(),
            tone: Some(Tone::Good),
            width: 1.1,
            label: SharedString::default(),
            tooltip: SharedString::default(),
            style: EdgeStyle::Auto,
            pill: None,
            pill_tone: None,
        })
        .collect();
    let mut state = GraphState::new("map", text()).unpaged();
    state.set(nodes, edges);
    let column: Vec<_> = boxes(&state).into_iter().filter(|b| b.0 > 0).collect();
    assert_eq!(column.len(), 40);
    assert!(column.iter().all(|b| b.1 == column[0].1));
}
