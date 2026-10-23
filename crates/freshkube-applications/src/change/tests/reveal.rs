use super::*;

#[gpui_kit::test]
fn a_change_opened_in_remembered_chain_reveals_its_initial_stage(cx: &mut TestAppContext) {
    let (first, a) = fixture(cx);
    cx.update_window(first, |_, window, cx| chain(&a, window, cx))
        .unwrap();
    let (handle, page) = open_stage(
        cx,
        760.,
        560.,
        Shape::Whole,
        Duration::ZERO,
        20.,
        Some("prod-lon"),
    );
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(page.read(cx).view, ChangeView::Chain);
        window.render_frame(cx);
        window.simulate_next_frame(cx);
        window.render_frame(cx);
        let pane = window.find("change-chain-scroll").bounds();
        let card = window.find("change-card-prod-lon-eligible").bounds();
        assert!(card.left() >= pane.left() - px(1.));
        assert!(card.right() <= pane.right() + px(1.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_first_switch_to_chain_reveals_the_list_selection_after_layout(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.select("prod-ams-argocd", window, cx));
        window.click("change-view-chain", cx);
        window.simulate_next_frame(cx);
        window.render_frame(cx);
        window.simulate_next_frame(cx);
        window.render_frame(cx);
        let pane = window.find("change-chain-scroll").bounds();
        let card = window.find("change-card-prod-ams-argocd").bounds();
        assert!(card.left() >= pane.left() - px(1.), "{card:?} in {pane:?}");
        assert!(
            card.right() <= pane.right() + px(1.),
            "{card:?} in {pane:?}"
        );
    })
    .unwrap();
}

fn assert_card_in_canvas(key: &str, window: &Window) {
    let horizontal = window.find("change-chain-scroll").bounds();
    let vertical = window.find("change-chain-canvas").bounds();
    let card = window
        .find(SharedString::from(format!("change-card-{key}")))
        .bounds();
    assert!(
        card.left() >= horizontal.left() - px(1.),
        "{key}: {card:?} in {horizontal:?}"
    );
    assert!(
        card.right() <= horizontal.right() + px(1.),
        "{key}: {card:?} in {horizontal:?}"
    );
    assert!(
        card.top() >= vertical.top() - px(1.),
        "{key}: {card:?} in {vertical:?}"
    );
    assert!(
        card.bottom() <= vertical.bottom() + px(1.),
        "{key}: {card:?} in {vertical:?}"
    );
}

fn assert_stacked_chain_reveal(cx: &mut TestAppContext, width: f32, height: f32, text: f32) {
    let (handle, page) = open(cx, width, height, Shape::Whole, Duration::ZERO, text);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.select("prod-ams-eligible", window, cx));
        page.update(cx, |page, cx| page.focus(window, cx));
        // The header folds the toggle into its menu at narrow widths.
        window.press("v", cx);
        settle_chain(window, cx);
        assert_card_in_canvas("prod-ams-eligible", window);
        assert!(
            window.find("change-detail").bounds().top()
                >= window.find("change-chain").bounds().bottom() - px(1.)
        );

        // Selection in Chain and keyboard navigation also reveal both axes.
        page.update(cx, |page, cx| page.select("prod-lon-eligible", window, cx));
        window.render_frame(cx);
        settle_chain(window, cx);
        assert_card_in_canvas("prod-lon-eligible", window);
        page.update(cx, |page, cx| page.focus(window, cx));
        window.press("up", cx);
        settle_chain(window, cx);
        assert_eq!(
            page.read(cx).cursor(),
            Some(Cursor::Node("prod-ams-eligible".into()))
        );
        assert_card_in_canvas("prod-ams-eligible", window);
    })
    .unwrap();
}

#[gpui_kit::test]
fn narrow_chain_reveals_both_axes_after_layout(cx: &mut TestAppContext) {
    assert_stacked_chain_reveal(cx, 760., 560., 13.);
}

#[gpui_kit::test]
fn narrow_chain_at_text_20_reveals_both_axes_after_layout(cx: &mut TestAppContext) {
    assert_stacked_chain_reveal(cx, 760., 560., 20.);
}

#[gpui_kit::test]
fn wide_chain_at_text_20_reveals_both_axes_after_layout(cx: &mut TestAppContext) {
    assert_stacked_chain_reveal(cx, 1280., 880., 20.);
}

#[gpui_kit::test]
fn chain_reveals_again_when_the_stacked_canvas_shrinks_on_a_later_frame(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 760., 880., Shape::Whole, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        page.update(cx, |page, cx| page.select("prod-lon-eligible", window, cx));
        window.render_frame(cx);
        assert!(window.simulate_next_frame(cx) > 0);
        window.render_frame(cx);
        let before = window.find("change-chain-canvas").bounds().size.height;
        // Model the split applying a smaller height after the first reveal.
        let state = page.read(cx).split.stacked_state().clone();
        state.update(cx, |state, cx| state.resize_panel(0, px(180.), window, cx));
        window.render_frame(cx);
        settle_chain(window, cx);
        assert!(window.find("change-chain-canvas").bounds().size.height < before);
        assert_card_in_canvas("prod-lon-eligible", window);
        assert_eq!(
            window.simulate_next_frame(cx),
            0,
            "settled reveal stops requesting frames"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn remembered_chain_reveals_its_initial_stage_on_both_axes(cx: &mut TestAppContext) {
    let (first, a) = fixture(cx);
    cx.update_window(first, |_, window, cx| chain(&a, window, cx))
        .unwrap();
    for (width, height) in [(760., 560.), (1280., 880.)] {
        let (handle, page) = open_stage(
            cx,
            width,
            height,
            Shape::Whole,
            Duration::ZERO,
            20.,
            Some("prod-ams"),
        );
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            settle_chain(window, cx);
            assert_eq!(page.read(cx).view, ChangeView::Chain);
            assert_card_in_canvas("prod-ams-eligible", window);
        })
        .unwrap();
    }
}
