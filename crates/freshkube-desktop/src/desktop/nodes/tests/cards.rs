use super::*;

/// Hovers the point `at` picks from the first card's bounds and the list's, in
/// the Nodes cards (fixture 1280×560, text size 20), and
/// returns the first card's id and name and the pointer's position.
fn hover_first_card(
    cx: &mut TestAppContext,
    at: impl FnOnce(
        gpui_kit::Bounds<gpui_kit::Pixels>,
        gpui_kit::Bounds<gpui_kit::Pixels>,
    ) -> gpui_kit::Point<gpui_kit::Pixels>,
) -> (
    tokio::runtime::Runtime,
    gpui_kit::AnyWindowHandle,
    gpui_kit::SharedString,
    gpui_kit::SharedString,
    gpui_kit::Point<gpui_kit::Pixels>,
) {
    use gpui_kit::{InputEvent, MouseMoveEvent};
    let (runtime, handle, pilot) = fixture(cx, 1280., 560.);
    let (first, name, position) = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("nav-nodes", cx);
            crate::text_size::set(20., cx);
            crate::desktop::tests::settle_header(window, cx);
            window.click("nodes-view-cards", cx);
            window.render_frame(cx);
            let first = pilot.read(cx).node_workspace.lines[0];
            let row = &pilot.read(cx).node_workspace.rows[first];
            let (first, name) = (row.id.clone(), row.name.clone());
            let card = window.find(first.clone()).bounds();
            let position = at(card, window.find("nodes-cards").bounds());
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            (first, name, position)
        })
        .unwrap();
    (runtime, handle, first, name, position)
}

/// A card's tooltip hides when a wheel scroll moves another card under a
/// pointer that stays still.
#[gpui_kit::test]
fn a_wheel_scroll_hides_a_shown_card_tooltip(cx: &mut TestAppContext) {
    use gpui_kit::{point, px};
    let (_runtime, handle, _, name, position) =
        hover_first_card(cx, |card, _| card.origin + point(px(24.), px(24.)));
    // Past the tooltip's show delay.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let before = drawn(&name);
        window.render_frame(cx);
        assert!(drawn(&name) > before, "the card's tooltip never showed");
        // A row and its gap, so the next row's card is under the pointer.
        wheel(window, position, -400., cx);
        let before = drawn(&name);
        window.render_frame(cx);
        assert_eq!(drawn(&name), before, "the tooltip stayed after the scroll");
    })
    .unwrap();
}

/// A tooltip still waiting to show when a wheel scroll moves its card away
/// from a still pointer never shows over the card now there (#192).
#[gpui_kit::test]
fn a_card_tooltip_waiting_to_show_does_not_show_after_its_card_scrolls_away(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{point, px};
    // Near the bottom of what shows of the card: the list is shorter than
    // a card at this size, so a short scroll brings the next row's card
    // under the pointer while this one is still drawn.
    let (_runtime, handle, first, name, position) = hover_first_card(cx, |card, list| {
        point(
            card.origin.x + px(24.),
            card.bottom().min(list.bottom()) - px(10.),
        )
    });
    // Before the show delay ends.
    cx.update_window(handle, |_, window, cx| {
        wheel(window, position, -120., cx);
        let card = window.find(first).bounds();
        assert!(
            !card.contains(&position),
            "the card is still under the pointer: {card:?} {position:?}"
        );
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let before = drawn(&name);
        window.render_frame(cx);
        assert_eq!(
            drawn(&name),
            before,
            "the first card's tooltip showed over another card"
        );
    })
    .unwrap();
}
