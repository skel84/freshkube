use super::*;

#[gpui_kit::test]
fn selecting_a_link_reveals_its_comparison_after_reading_a_long_stage(cx: &mut TestAppContext) {
    let (handle, page) = open_overview(cx, 760., 700., Shape::Whole, Duration::ZERO, 13.);
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| page.select("dev-eligible", window, cx));
        window.render_frame(cx);
        window.scroll(
            "change-detail-body",
            ScrollDelta::Pixels(point(px(0.), px(-300.))),
            cx,
        );
        window.render_frame(cx);
        assert!(page.read(cx).detail_scroll.offset().y < px(0.));
        // The join into the Freight is a button: its key, and under it the
        // word for how sure the link is.
        window.click("change-join-freight-key", cx);
        window.render_frame(cx);
        assert!(page.read(cx).link_selected);
        assert_eq!(
            page.read(cx).selected().map(|s| s.as_ref()),
            Some("freight")
        );
        for view in [ChangeView::Overview, ChangeView::Hops] {
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
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        let hop = page.read(cx).change.hop("prod-ams-argocd").unwrap();
        let Shows::Hop(detail) = &hop.shows else {
            panic!("Application");
        };
        let why = detail.unlinked.clone().unwrap();
        page.update(cx, |page, cx| {
            page.select_link("prod-ams-argocd", window, cx)
        });
        window.render_frame(cx);
        assert!(
            window
                .find("change-detail-link")
                .label()
                .unwrap()
                .contains(&why)
        );
        window.click("change-view-hops", cx);
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
