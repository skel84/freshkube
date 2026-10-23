use super::*;

#[gpui_kit::test]
fn chain_and_inspector_keep_the_wide_frame(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        layout_check::assert_table_page_from(
            window,
            cx,
            &layout_check::TablePage {
                page: "change-page",
                title: "change-title",
                title_text: "wonky-otter",
                table: "change-table-scroll",
                list: "change-list",
            },
            "change-back",
        );
        chain(&page, window, cx);
        page.update(cx, |page, cx| page.select("freight", window, cx));
        window.render_frame(cx);
        settle_chain(window, cx);
        layout_check::settle_header(window, cx);
        layout_check::assert_edge_frame_from(
            window,
            cx,
            &layout_check::PageFrame {
                page: "change-page",
                title: "change-title",
                title_text: "wonky-otter",
                content: "change-split",
            },
            "change-back",
        );
        layout_check::assert_inspector(
            window,
            cx,
            "change-split",
            "change-chain",
            "change-detail",
            "change-detail-title",
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn narrow_chain_at_text_20_reveals_a_far_card_and_stacks_the_inspector(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 760., 560., Shape::Whole, Duration::ZERO, 20.);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        page.update(cx, |page, cx| page.select("prod-lon-pods", window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        settle_chain(window, cx);
        layout_check::settle_header(window, cx);
        layout_check::assert_inspector(
            window,
            cx,
            "change-split",
            "change-chain",
            "change-detail",
            "change-detail-title",
        );
        let pane = window.find("change-chain-scroll").bounds();
        let card = window.find("change-card-prod-lon-pods").bounds();
        assert!(card.left() >= pane.left() - px(1.));
        assert!(card.right() <= pane.right() + px(1.));
        assert!(
            window.find("change-detail").bounds().top()
                >= window.find("change-chain").bounds().bottom() - px(1.)
        );
    })
    .unwrap();
}

fn assert_crossing_pill_clearance(cx: &mut TestAppContext, width: f32, height: f32, text: f32) {
    let (handle, page) = open(cx, width, height, Shape::Whole, Duration::ZERO, text);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        let ams = window.find("change-link-prod-ams-eligible").bounds();
        let lon = window.find("change-link-prod-lon-eligible").bounds();
        let ams_card = window.find("change-card-prod-ams-eligible").bounds();
        let lon_card = window.find("change-card-prod-lon-eligible").bounds();
        assert!(
            (ams.center().y - ams_card.center().y).abs()
                < (ams.center().y - lon_card.center().y).abs()
        );
        assert!(
            (lon.center().y - lon_card.center().y).abs()
                < (lon.center().y - ams_card.center().y).abs()
        );
        assert!(ams.bottom() < lon.top() || lon.bottom() < ams.top());
        assert!((ams.center().y - ams_card.center().y).abs() < px(1.));
        assert!((lon.center().y - lon_card.center().y).abs() < px(1.));
        let origin = window.find("change-chain-graph").bounds().origin;
        let unit = freshkube_ui::ui::dp_px(1., window);
        let p = page.read(cx);
        for (key, pill) in [("prod-ams-eligible", lon), ("prod-lon-eligible", ams)] {
        let (_, route) = p.graph.shown().find(|(edge, _)| edge.to == key).unwrap();
        // Both curves, including their strokes, clear the other pill.
        for segment in &route.segments {
            for step in 0..=1000 {
                let t = step as f32 / 1000.;
                let u = 1. - t;
                let weights = [u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t];
                let (x, y) = weights
                    .iter()
                    .zip(segment)
                    .fold((0., 0.), |(x, y), (w, p)| (x + w * p.0, y + w * p.1));
                let at = origin + point(unit * x, unit * y);
                let margin = unit * 4.;
                assert!(
                    at.x < pill.left() - margin
                        || at.x > pill.right() + margin
                        || at.y < pill.top() - margin
                        || at.y > pill.bottom() + margin,
                    "{key} at {at:?} needs clearance from the other pill {pill:?}; route {route:?}"
                );
            }
        }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn crossing_links_keep_their_pills_beside_their_own_targets(cx: &mut TestAppContext) {
    assert_crossing_pill_clearance(cx, 1600., 1000., 13.);
}

#[gpui_kit::test]
fn narrow_chain_clears_pills_in_both_directions(cx: &mut TestAppContext) {
    assert_crossing_pill_clearance(cx, 760., 560., 13.);
}

#[gpui_kit::test]
fn narrow_chain_at_text_20_clears_pills_in_both_directions(cx: &mut TestAppContext) {
    assert_crossing_pill_clearance(cx, 760., 560., 20.);
}
