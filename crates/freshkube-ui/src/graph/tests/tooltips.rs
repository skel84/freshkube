use super::*;

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
