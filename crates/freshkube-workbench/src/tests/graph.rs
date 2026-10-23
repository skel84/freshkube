use super::*;

#[gpui_kit::test]
fn the_graph_story_lays_out_each_shape(cx: &mut TestAppContext) {
    let graph = super::stories::find("graph").unwrap();
    let (handle, workbench) = open(cx, graph);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let story = workbench
            .read(cx)
            .view()
            .clone()
            .downcast::<GraphStory>()
            .unwrap();
        let canvas = window.find("graph-graph").bounds();
        // shop-web calls shop-api: the caller sits left of the callee.
        let web = window.find("graph-node-0").bounds();
        let api = window.find("graph-node-1").bounds();
        assert!(web.right() < api.left());
        assert!(canvas.contains(&web.origin) && canvas.contains(&api.origin));
        for (shape, id) in [
            (Shape::FanOut, "fan-out"),
            (Shape::Cycle, "cycle"),
            (Shape::Loose, "loose"),
            (Shape::Shop, "shop"),
        ] {
            window.click(format!("graph-shape-{id}"), cx);
            window.render_frame(cx);
            assert_eq!(story.read(cx).shape(), shape);
            let positions = story.read(cx).positions();
            for (ix, (label, x, y)) in positions.iter().enumerate() {
                assert!(
                    positions[ix + 1..]
                        .iter()
                        .all(|(_, x2, y2)| (x, y) != (x2, y2)),
                    "{label} shares its place in {id}"
                );
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_graph_story_routes_its_fan_out_without_crossings(cx: &mut TestAppContext) {
    let graph = super::stories::find("graph").unwrap();
    let (handle, workbench) = open(cx, graph);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let story = workbench
            .read(cx)
            .view()
            .clone()
            .downcast::<GraphStory>()
            .unwrap();
        window.click("graph-shape-fan-out", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).shape(), Shape::FanOut);
        assert_eq!(story.read(cx).crossings(), 0);
        // The wrapped column is drawn too.
        assert!(window.find("graph-node-12").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_graph_story_selects_a_call_and_filters_problems(cx: &mut TestAppContext) {
    use freshkube_ui::graph::GraphSource;
    let graph = super::stories::find("graph").unwrap();
    let (handle, workbench) = open(cx, graph);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let story = workbench
            .read(cx)
            .view()
            .clone()
            .downcast::<GraphStory>()
            .unwrap();
        // checkout calls payments-db, and that call is failing.
        window.click("graph-call-2--6", cx);
        assert_eq!(story.read(cx).graph().selected(), Some(&(2, 6)));
        window.render_frame(cx);
        window.click("graph-problems", cx);
        window.render_frame(cx);
        let graph = story.read(cx).graph();
        assert_eq!(graph.visible_count(), 2);
        assert_eq!(graph.placed().count(), 8);
        assert!(window.try_find("graph-call-0--1").is_none());
        assert!(window.try_find("graph-call-2--6").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_graph_story_draws_a_chain_unwrapped_with_labels_and_a_side_branch(cx: &mut TestAppContext) {
    let graph = super::stories::find("graph").unwrap();
    let (handle, workbench) = open(cx, graph);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let story = story::<GraphStory>(&workbench, cx);
        // Twenty callees in one column, on one page.
        window.click("graph-shape-unwrapped", cx);
        window.render_frame(cx);
        let positions = story.read(cx).positions();
        assert_eq!(positions.len(), 21);
        let column: Vec<_> = positions.iter().skip(1).map(|p| p.1).collect();
        assert!(column.iter().all(|&x| x == column[0]));
        assert!(window.try_find("graph-next").is_none());
        // Every call of the chain carries the key it joined on.
        window.click("graph-shape-labels", cx);
        window.render_frame(cx);
        for call in ["0--1", "2--3", "4--6"] {
            assert!(window.try_find(format!("graph-call-{call}")).is_some());
        }
        assert!(window.try_find("graph-node-7").is_none());
        // The branch is a box one column right of the box it hangs from.
        window.click("graph-shape-branch", cx);
        window.render_frame(cx);
        let branch = window.find("graph-node-7").bounds();
        let anchor = window.find("graph-node-2").bounds();
        let next = window.find("graph-node-3").bounds();
        assert!(anchor.right() < branch.left());
        assert_eq!(branch.left(), next.left());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_graph_story_walks_its_chain_with_the_arrow_keys(cx: &mut TestAppContext) {
    use freshkube_ui::graph::GraphSource;
    let graph = super::stories::find("graph").unwrap();
    let (handle, workbench) = open(cx, graph);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let story = story::<GraphStory>(&workbench, cx);
        window.click("graph-shape-keys", cx);
        window.render_frame(cx);
        let focus = story.read(cx).graph().focus_handle().unwrap().clone();
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("down", cx);
        window.press("right", cx);
        assert_eq!(story.read(cx).graph().selected(), Some(&(0, 1)));
        window.press("left", cx);
        window.press("down", cx);
        window.press("left", cx);
        assert_eq!(story.read(cx).graph().selected(), Some(&(0, 1)));
    })
    .unwrap();
}
