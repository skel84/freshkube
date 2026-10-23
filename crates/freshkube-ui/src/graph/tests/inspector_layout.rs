use super::*;

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
    cx.update(|cx| {
        let sizes = &map.read(cx).sizes;
        assert_eq!(sizes.saves(), 1);
        assert_eq!(sizes.size(inspector::width_key(MAP)), Some(400.));
    });
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
        // Capped to the room, the width the user gave stays saved.
        assert_eq!(map.read(cx).split.width(), 1200.);
        assert_eq!(map.read(cx).sizes.saves(), 0);
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
        let sizes = &map.read(cx).sizes;
        assert_eq!(sizes.saves(), 1);
        let saved = sizes.size(inspector::width_key(MAP)).unwrap();
        // The room inside the page's padding, less the graph and its card.
        let rest = 1968. - graph - 2.;
        assert!(
            (saved - rest).abs() <= 1.,
            "saved {saved}, the room beside the graph is {rest}"
        );
        let lead = window.find("map-lead").bounds();
        let inspector = window.find("map-inspector").bounds();
        assert_eq!(lead.top(), inspector.top(), "still beside");
        assert!(lead.size.width >= crate::ui::dp_px(graph, window));
    })
    .unwrap();
}
