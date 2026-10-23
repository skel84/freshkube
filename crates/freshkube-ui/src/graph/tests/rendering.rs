use super::*;

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
    let before = freshkube_probe::probe::count("graph.prepare");
    cx.update_window(handle, |_, window, cx| {
        for _ in 0..5 {
            window.render_frame(cx);
        }
    })
    .unwrap();
    assert_eq!(freshkube_probe::probe::count("graph.prepare"), before);
    cx.update(|cx| map.update(cx, |map, _| map.graph.set_page(2)));
    assert_eq!(freshkube_probe::probe::count("graph.prepare"), before + 1);
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
