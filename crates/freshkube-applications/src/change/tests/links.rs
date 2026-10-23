use super::*;

#[gpui_kit::test]
fn selecting_a_link_reveals_its_comparison_after_reading_a_long_stage(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 760., 700., Shape::Whole, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        page.update(cx, |page, cx| page.select("dev-eligible", window, cx));
        window.render_frame(cx);
        window.scroll(
            "change-detail-body",
            ScrollDelta::Pixels(point(px(0.), px(-300.))),
            cx,
        );
        window.render_frame(cx);
        assert!(page.read(cx).detail_scroll.offset().y < px(0.));
        window.press("left", cx);
        window.render_frame(cx);
        assert!(page.read(cx).link_selected);
        for view in [ChangeView::Chain, ChangeView::List] {
            page.update(cx, |page, cx| page.set_view(view, window, cx));
            window.render_frame(cx);
            let body = window.find("change-detail-body").bounds();
            let link = window.find("change-detail-link").bounds();
            assert!(link.top() >= body.top());
            assert!(link.bottom() <= body.bottom());
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn selecting_a_link_keeps_the_supplied_unlinked_reason(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        let hop = page.read(cx).change.hop("prod-ams-argocd").unwrap();
        let Shows::Hop(detail) = &hop.shows else {
            panic!("Application");
        };
        let why = detail.unlinked.clone().unwrap();
        let edge = page
            .read(cx)
            .trail
            .edges
            .iter()
            .find(|edge| edge.to == "prod-ams-argocd")
            .unwrap()
            .clone();
        assert_eq!(edge.tooltip.as_ref(), why);
        page.update(cx, |page, cx| {
            page.step_to(Cursor::Link(edge.from, edge.to), window, cx)
        });
        window.render_frame(cx);
        assert!(
            window
                .find("change-detail-link")
                .label()
                .unwrap()
                .contains(&why)
        );
        window.click("change-view-list", cx);
        window.render_frame(cx);
        assert!(
            window
                .find("change-detail-link")
                .label()
                .unwrap()
                .contains(&why)
        );
    })
    .unwrap();
}
