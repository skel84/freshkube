use super::*;

#[gpui_kit::test]
fn arrow_keys_walk_the_chain_and_leave_the_page_alone_without_keys(cx: &mut TestAppContext) {
    let (handle, map) = open_chain(cx, 1280., None);
    cx.update_window(handle, |_, window, cx| {
        let focus = map.read(cx).graph.focus_handle().unwrap().clone();
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("down", cx);
        assert_eq!(map.read(cx).cursor, Some(Cursor::Node(0)));
        window.press("right", cx);
        assert_eq!(map.read(cx).cursor, Some(Cursor::Link(0, 1)));
        assert_eq!(map.read(cx).graph.selected(), Some(&(0, 1)));
        window.press("right", cx);
        assert_eq!(map.read(cx).cursor, Some(Cursor::Node(1)));
        window.press("left", cx);
        window.press("left", cx);
        assert_eq!(map.read(cx).cursor, Some(Cursor::Node(0)));
        // The origin has no call into it: the key goes nowhere.
        window.press("left", cx);
        assert_eq!(map.read(cx).cursor, Some(Cursor::Node(0)));
        window.press("down", cx);
        window.press("down", cx);
        assert_eq!(map.read(cx).cursor, Some(Cursor::Node(2)));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_graph_without_keys_ignores_the_arrows(cx: &mut TestAppContext) {
    cx.update(bind_keys);
    let (handle, map) = open(cx, chain(), 1280., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.press("right", cx);
        assert_eq!(map.read(cx).cursor, None);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_key_scrolls_a_far_box_into_view_at_large_text(cx: &mut TestAppContext) {
    let (handle, map) = open_chain(cx, 760., Some(20.));
    cx.update_window(handle, |_, window, cx| {
        let focus = map.read(cx).graph.focus_handle().unwrap().clone();
        window.focus(&focus, cx);
        window.render_frame(cx);
        let pan = map.read(cx).graph.pan.clone();
        assert!(pan.bounds().size.width > px(0.));
        assert_eq!(pan.offset().x, px(0.));
        // Walk to the last column.
        for _ in 0..8 {
            window.press("down", cx);
            window.render_frame(cx);
        }
        let last = map.read(cx).cursor.clone();
        assert!(matches!(last, Some(Cursor::Node(_))));
        let view = pan.bounds();
        assert!(pan.offset().x < px(0.), "the graph scrolled");
        let node = match last {
            Some(Cursor::Node(key)) => key,
            _ => unreachable!(),
        };
        let bounds = window.find(format!("map-node-{node}")).bounds();
        assert!(bounds.left() >= view.left() - px(1.));
        assert!(bounds.right() <= view.right() + px(1.));
    })
    .unwrap();
}

/// Presses Tab `presses` times from the start and returns, for each press,
/// whether focus is inside the graph's canvas and whether it is on the
/// graph itself.
fn tab_through(
    map: &Entity<Map>,
    window: &mut Window,
    cx: &mut gpui_kit::App,
    presses: usize,
) -> Vec<(bool, bool)> {
    let focus = map.read(cx).graph.focus_handle().cloned();
    (0..presses)
        .map(|_| {
            window.focus_next(cx);
            match &focus {
                Some(focus) => (focus.contains_focused(window, cx), focus.is_focused(window)),
                None => (false, false),
            }
        })
        .collect()
}

#[gpui_kit::test]
fn a_graph_with_keys_is_one_tab_stop_and_boxes_and_pills_are_not(cx: &mut TestAppContext) {
    let (handle, map) = open_chain(cx, 1280., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The toolbar's button comes first; the graph is one stop after it.
        let steps = tab_through(&map, window, cx, 4);
        let inside: Vec<_> = steps.iter().filter(|s| s.0).collect();
        assert_eq!(inside.len(), 1, "{steps:?}");
        assert!(inside[0].1, "the one stop is the graph: {steps:?}");
    })
    .unwrap();
}

fn cursor_ring(window: &Window, key: usize) -> bool {
    let bounds = window
        .find(format!("map-node-{key}"))
        .bounds()
        .scale(window.scale_factor());
    window.painted_quads().iter().any(|quad| {
        quad.bounds == bounds && quad.border_widths.left.0 >= 2. * window.scale_factor()
    })
}

fn press_tab(handle: AnyWindowHandle, key: &str, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| window.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn tab_entry_rings_a_card_without_selecting_and_mouse_entry_does_not(cx: &mut TestAppContext) {
    let (handle, map) = open_chain(cx, 1280., None);
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        window.blur(cx);
        window.render_frame(cx);
        // Start at the toolbar, as an already-focused window does.
        window.focus_next(cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    for _ in 0..8 {
        press_tab(handle, "tab", cx);
        let focused = cx
            .update_window(handle, |_, window, cx| {
                map.read(cx)
                    .graph
                    .focus_handle()
                    .unwrap()
                    .is_focused(window)
            })
            .unwrap();
        if focused {
            break;
        }
    }
    cx.update_window(handle, |_, window, cx| {
        assert!(
            map.read(cx)
                .graph
                .focus_handle()
                .unwrap()
                .is_focused(window)
        );
        assert!(
            window.simulate_next_frame(cx) > 0,
            "keyboard focus requests the ring's frame"
        );
        window.render_frame(cx);
        assert!(
            cursor_ring(window, 0),
            "Tab shows focus even without a selection"
        );
        assert_eq!(map.read(cx).cursor, None);
        assert!(map.read(cx).opened.is_empty());
        window.click("map-node-0", cx);
        window.simulate_next_frame(cx);
        window.render_frame(cx);
        assert!(
            !cursor_ring(window, 0),
            "mouse entry clears the keyboard ring"
        );
    })
    .unwrap();
    press_tab(handle, "shift-tab", cx);
    press_tab(handle, "tab", cx);
    cx.update_window(handle, |_, window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
        assert!(
            cursor_ring(window, 0),
            "Tab restores the ring on the selected card"
        );
        assert_eq!(map.read(cx).cursor, Some(Cursor::Node(0)));
        window.click("map-edge-0--1", cx);
    })
    .unwrap();
    press_tab(handle, "shift-tab", cx);
    press_tab(handle, "tab", cx);
    cx.update_window(handle, |_, window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
        let pill = window
            .find("map-edge-0--1")
            .bounds()
            .scale(window.scale_factor());
        assert!(
            window.painted_quads().iter().any(|quad| quad.bounds == pill
                && quad.border_widths.left.0 >= 2. * window.scale_factor()),
            "Tab also rings a selected link"
        );
        assert_eq!(map.read(cx).cursor, Some(Cursor::Link(0, 1)));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_graph_without_keys_keeps_its_boxes_and_markers_as_tab_stops(cx: &mut TestAppContext) {
    let (handle, map) = open(cx, chain(), 1280., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let before = window.find("map-problems").focused();
        window.focus_next(cx);
        // Tabbing past the toolbar enters the graph's buttons one by one.
        let moved = (0..4).any(|_| {
            window.focus_next(cx);
            window.render_frame(cx);
            (0..6).any(|ix| window.find(format!("map-node-{ix}")).focused() == Some(true))
                || (0..5).any(|ix| {
                    let edge = &map.read(cx).graph.edges()[ix];
                    window.find(edge.marker_id.clone()).focused() == Some(true)
                })
        });
        assert!(
            moved,
            "a box or a pill takes the Tab (problems was {before:?})"
        );
    })
    .unwrap();
}

/// The keyboard ring is painted only while the canvas has focus. These
/// inspect paint output, so the assertion covers the visible border.
#[gpui_kit::test]
fn a_background_click_only_focuses_and_keyboard_rings_the_cursor_until_blur(
    cx: &mut TestAppContext,
) {
    let (handle, map) = open_chain(cx, 1280., None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let before = map.read(cx).graph.pan.offset();
        window.click_at("map-graph", point(px(4.), px(4.)), cx);
        window.simulate_next_frame(cx);
        assert_eq!(map.read(cx).cursor, None);
        assert_eq!(map.read(cx).graph.pan.offset(), before);
        let graph = window
            .find("map-graph")
            .bounds()
            .scale(window.scale_factor());
        let tint = crate::palette::palette(cx).accent_soft;
        assert!(
            !window
                .painted_quads()
                .iter()
                .any(|q| q.bounds == graph && q.background == tint.into())
        );
        window.press("down", cx);
        window.simulate_next_frame(cx);
        assert!(
            map.read(cx)
                .graph
                .focus_handle()
                .unwrap()
                .is_focused(window)
        );
        let Some(Cursor::Node(key)) = map.read(cx).cursor else {
            panic!("an arrow key shows a cursor");
        };
        let ring = |window: &Window| {
            let bounds = window
                .find(format!("map-node-{key}"))
                .bounds()
                .scale(window.scale_factor());
            window
                .painted_quads()
                .iter()
                .any(|q| q.bounds == bounds && q.border_widths.left.0 >= 2. * window.scale_factor())
        };
        assert!(ring(window), "cursor has a visible border immediately");
        window.focus_next(cx);
        assert!(
            !map.read(cx)
                .graph
                .focus_handle()
                .unwrap()
                .is_focused(window)
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.simulate_next_frame(cx);
        let Some(Cursor::Node(key)) = map.read(cx).cursor else {
            panic!("cursor is retained");
        };
        let bounds = window
            .find(format!("map-node-{key}"))
            .bounds()
            .scale(window.scale_factor());
        assert!(
            !window.painted_quads().iter().any(|q| q.bounds == bounds
                && q.border_widths.left.0 >= 2. * window.scale_factor()),
            "the cursor's border clears when focus leaves"
        );
    })
    .unwrap();
}
