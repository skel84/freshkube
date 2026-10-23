use super::*;

#[gpui_kit::test]
fn the_toggle_preserves_rows_cards_links_and_a_gate_selection(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("change-hop-push-run", cx);
        window.click("change-view-chain", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).selected().map(|s| s.as_ref()),
            Some("push-run")
        );
        assert!(window.try_find("change-chain").is_some());
        window.click("change-link-signature", cx);
        assert!(page.read(cx).link_selected);
        window.click("change-view-list", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).selected().map(|s| s.as_ref()),
            Some("signature")
        );
        assert!(page.read(cx).link_selected);
        assert!(window.find("change-hop-signature").visible());
        page.update(cx, |page, cx| page.select("dev-verification", window, cx));
        chain(&page, window, cx);
        assert_eq!(
            page.read(cx).cursor(),
            Some(Cursor::Node("dev-eligible".into()))
        );
        assert_eq!(
            page.read(cx).selected().map(|s| s.as_ref()),
            Some("dev-verification")
        );
        assert!(window.try_find("change-detail-verification").is_some());
        page.update(cx, |page, cx| page.back(cx));
        page.update(cx, |page, cx| page.set_view(ChangeView::List, window, cx));
        chain(&page, window, cx);
        assert!(
            page.read(cx).selected.is_none(),
            "switching never invents a selection"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_chain_keys_walk_links_and_cards_open_clear_back_and_switch(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    let events = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
    let recorded = events.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&page, move |_, event, _| {
            recorded.borrow_mut().push(match event {
                ChangeEvent::Back => "back",
                ChangeEvent::Open(_) | ChangeEvent::Switch(..) => "open",
                _ => "other",
            })
        })
    });
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        window.press("down", cx);
        assert_eq!(page.read(cx).selected().map(|s| s.as_ref()), Some("pr"));
        window.press("right", cx);
        assert_eq!(
            page.read(cx).cursor(),
            Some(Cursor::Link("pr".into(), "pr-run".into()))
        );
        window.press("right", cx);
        assert_eq!(page.read(cx).selected().map(|s| s.as_ref()), Some("pr-run"));
        window.press("left", cx);
        window.press("left", cx);
        assert_eq!(page.read(cx).selected().map(|s| s.as_ref()), Some("pr"));
        window.press("down", cx);
        window.press("up", cx);
        assert_eq!(page.read(cx).selected().map(|s| s.as_ref()), Some("pr"));
        page.update(cx, |page, cx| page.select("freight", window, cx));
        window.press("o", cx);
        window.press("enter", cx);
        window.press("v", cx);
        assert_eq!(page.read(cx).view, ChangeView::List);
        window.press("v", cx);
        assert_eq!(page.read(cx).view, ChangeView::Chain);
        window.press("escape", cx);
        assert!(page.read(cx).selected.is_none());
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(&*events.borrow(), &["open", "open", "back"]);
}

#[gpui_kit::test]
fn the_status_filter_dims_chain_cards_and_a_switch_reveals_its_selection(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        page.update(cx, |page, cx| {
            page.toggle_filter(Tone::Crit, cx);
            page.select("dev-verification", window, cx);
        });
        assert_eq!(
            page.read(cx).graph.nodes().len(),
            page.read(cx).trail.nodes.len()
        );
        page.update(cx, |page, cx| page.set_view(ChangeView::List, window, cx));
        window.render_frame(cx);
        assert_eq!(page.read(cx).filter, None);
        assert!(!page.read(cx).folded.contains(&2));
        assert!(window.find("change-hop-dev-verification").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn background_clicks_never_select_or_scroll_and_outside_clicks_blur(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        page.update(cx, |page, cx| page.back(cx));
        window.simulate_next_frame(cx);
        let before = window.find("change-card-pr").bounds();
        window.click_at("change-chain-graph", point(px(4.), px(4.)), cx);
        window.simulate_next_frame(cx);
        assert!(page.read(cx).selected.is_none());
        assert!(page.read(cx).detail.is_none());
        assert_eq!(window.find("change-card-pr").bounds(), before);
        assert!(
            page.read(cx)
                .graph
                .focus_handle()
                .unwrap()
                .is_focused(window)
        );
        window.click_at(
            "change-chain",
            point(
                px(4.),
                window.find("change-chain").bounds().size.height - px(4.),
            ),
            cx,
        );
        window.simulate_next_frame(cx);
        assert!(
            !page
                .read(cx)
                .graph
                .focus_handle()
                .unwrap()
                .is_focused(window)
        );
    })
    .unwrap();
}

fn has_cursor_ring(key: &str, window: &Window) -> bool {
    let bounds = window
        .find(SharedString::from(format!("change-card-{key}")))
        .bounds()
        .scale(window.scale_factor());
    window.painted_quads().iter().any(|quad| {
        quad.bounds == bounds && quad.border_widths.left.0 >= 2. * window.scale_factor()
    })
}

#[gpui_kit::test]
fn mouse_selection_keeps_the_selected_border_and_only_keyboard_navigation_rings_it(
    cx: &mut TestAppContext,
) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        chain(&page, window, cx);
        window.click("change-card-pr", cx);
        settle_chain(window, cx);
        assert_eq!(page.read(cx).selected().map(|key| key.as_ref()), Some("pr"));
        assert!(page.read(cx).focus_handle().is_focused(window));
        assert!(
            !has_cursor_ring("pr", window),
            "mouse selection has no keyboard ring"
        );
        let bounds = window
            .find("change-card-pr")
            .bounds()
            .scale(window.scale_factor());
        let accent = freshkube_ui::palette::palette(cx).accent_line;
        assert!(
            window
                .painted_quads()
                .iter()
                .any(|q| q.bounds == bounds && q.border_color == accent)
        );
        window.press("down", cx);
        settle_chain(window, cx);
        assert!(
            has_cursor_ring("pr-run", window),
            "keyboard navigation rings its card"
        );
        window.click("change-card-pr", cx);
        settle_chain(window, cx);
        assert!(
            !has_cursor_ring("pr", window),
            "mouse selection clears the ring"
        );
        window.press("down", cx);
        settle_chain(window, cx);
        assert!(has_cursor_ring("pr-run", window));
    })
    .unwrap();
    // Focus subscriptions become active after the render's update ends.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.blur(cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
        assert!(!has_cursor_ring("pr-run", window), "blur clears the ring");
        page.update(cx, |page, cx| page.focus(window, cx));
        window.render_frame(cx);
        assert!(
            !has_cursor_ring("pr-run", window),
            "focus alone does not restore the ring"
        );
    })
    .unwrap();
}
