use super::*;

#[gpui_kit::test]
fn table_and_inspector_follow_shared_layout_at_wide_narrow_and_large_text(cx: &mut TestAppContext) {
    for width in [1280., 760.] {
        let (_runtime, page, window) = mount(cx, width);
        for text in [13., 20.] {
            cx.update_window(window.into(), |_, window, cx| {
                crate::text_size::set(text, cx);
                window.render_frame(cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                layout_check::settle_header(window, cx);
                window.render_frame(cx);
                layout_check::assert_table_page(
                    window,
                    cx,
                    &layout_check::TablePage {
                        page: "databases-page",
                        title: "databases-title",
                        title_text: "Databases",
                        table: "databases-table-scroll",
                        list: "databases-list",
                    },
                );
                let id = row_id(&page, "orders-db", cx);
                window.click(id, cx);
                window.render_frame(cx);
                layout_check::assert_inspector(
                    window,
                    cx,
                    "databases-split",
                    "databases-table",
                    "databases-detail",
                    "databases-detail-title",
                );
                // The heading takes the Inspector's width, not one letter's.
                let title = window.find("databases-detail-title").bounds();
                let inspector = window.find("databases-detail").bounds();
                assert!(
                    title.size.width > inspector.size.width / 2.,
                    "heading collapsed: {title:?} in {inspector:?}"
                );
                assert!(title.size.height < px(40. * text / 13.), "{title:?}");
                window.press("escape", cx);
                window.render_frame(cx);
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn the_engine_moves_into_the_inspector_header_when_the_table_has_no_room(cx: &mut TestAppContext) {
    for (width, in_table) in [(1280., false), (1800., true)] {
        let (_runtime, page, window) = mount(cx, width);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            // Without a selection the table has the room and shows it.
            assert!(page.read(cx).display.engine_shown());
            let id = row_id(&page, "orders-db", cx);
            window.click(id, cx);
            window.render_frame(cx);
            assert_eq!(page.read(cx).display.engine_shown(), in_table, "{width}");
            let columns = &page.read(cx).display.columns.0;
            assert_eq!(columns.iter().any(|c| c.label_text() == "Engine"), in_table);
            assert_eq!(
                window.try_find("databases-detail-engine").is_some(),
                !in_table
            );
            if !in_table {
                assert_eq!(
                    window.find("databases-detail-engine").label(),
                    Some("PostgreSQL 16")
                );
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_stacked_inspector_stays_reachable_under_a_long_coverage_banner(cx: &mut TestAppContext) {
    for shape in ["stale", "engine-refused"] {
        // The shell applies the text size to the window; a bare page does not.
        let (_runtime, window, pilot) = crate::desktop::tests::fixture(cx, 760., 560.);
        cx.update_window(window, |_, window, cx| {
            crate::text_size::set(20., cx);
            window.press("secondary-shift-d", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let page = cx.read(|cx| pilot.read(cx).databases());
        cx.update_window(window, |_, window, cx| {
            page.update(cx, |page, cx| {
                page.shape = shape.into();
                page.loader.reset();
                page.read(cx);
            });
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let id = page.read(cx).display.rows[0].id.clone();
            window.click(id, cx);
            window.render_frame(cx);
            assert_eq!(window.rem_size(), px(20.));
            let banner = window.find("databases-notices").bounds();
            assert!(
                banner.size.height <= px(72. * 20. / 13.) + px(1.),
                "{shape}: banner {banner:?}"
            );
            let before = window.find("databases-detail").bounds().origin.y;
            window.scroll(
                "databases-notices",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-2000.))),
                cx,
            );
            window.render_frame(cx);
            let inspector = window.find("databases-detail").bounds();
            assert!(
                inspector.origin.y < before,
                "{shape}: the page scrolls to the Inspector: {before:?} then {:?}",
                inspector.origin.y
            );
            // The split keeps its least height and the page scrolls instead.
            let least = px(freshkube_ui::inspector::MIN_HEIGHT * 20. / 13.) - px(1.);
            assert!(
                inspector.size.height >= least,
                "{shape}: the Inspector was squeezed: {inspector:?}"
            );
            // The first thing under the heading comes fully into view.
            let verdict = window.find("databases-detail-verdict").bounds();
            assert!(
                verdict.top() >= px(0.) && verdict.bottom() <= px(560.),
                "{shape}: the verdict is out of the window: {verdict:?}"
            );
        })
        .unwrap();
    }
}
