use super::*;

#[gpui_kit::test]
fn all_hops_and_the_inspector_keep_the_wide_frame(cx: &mut TestAppContext) {
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
        page.update(cx, |page, cx| page.select("freight", window, cx));
        window.render_frame(cx);
        settle(window, cx);
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
            "change-table",
            "change-detail",
            "change-detail-title",
        );
    })
    .unwrap();
}

/// The trail at a narrow window: nothing in it is cut where it meets the
/// column header or the right edge (#823): no row shows a sliver above the
/// first whole one, and the columns fit the width the table has.
fn assert_trail_whole(cx: &mut TestAppContext, width: f32, height: f32, text: f32, pick: usize) {
    let (handle, page) = open(cx, width, height, Shape::Whole, Duration::ZERO, text);
    cx.update_window(handle, |_, window, cx| {
        let key = {
            let rows = &page.read(cx).rows;
            rows[pick.min(rows.len() - 1)].key.to_string()
        };
        page.update(cx, |page, cx| page.select(&key, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        for _ in 0..8 {
            window.render_frame(cx);
            window.simulate_next_frame(cx);
        }
        let list = window.find("change-list").bounds();
        let view = window.find("change-table-scroll").bounds();
        let what = format!("{width}x{height} at text {text}, hop #{pick}");
        assert!(
            list.size.width <= view.size.width + px(0.5),
            "{what}: the table is {:?} wide in {:?}, so its last columns are cut",
            list.size.width,
            view.size.width,
        );
        let (rows, groups) = {
            let p = page.read(cx);
            (
                p.rows.iter().map(|r| format!("change-hop-{}", r.key)).collect::<Vec<_>>(),
                (0..p.groups.len()).map(|g| format!("change-group-{g}")).collect::<Vec<_>>(),
            )
        };
        let mut shown = 0;
        for id in rows.into_iter().chain(groups) {
            let Some(line) = window.try_find(id.clone()) else { continue };
            let line = line.bounds();
            // The window is a whole number of rows plus the hairlines'
            // rounding, under two pixels; a row showing that much is not cut.
            if line.bottom() <= list.top() + px(2.) {
                continue;
            }
            shown += 1;
            assert!(
                line.top() >= list.top() - px(1.),
                "{what}: {id} starts {:?} above the header's edge at {:?}, a row cut under the column header",
                list.top() - line.top(),
                list.top(),
            );
        }
        assert!(shown > 0, "{what}: no row shows");
    })
    .unwrap();
}

macro_rules! trail_whole {
    ($($name:ident $width:expr, $height:expr, $text:expr, $pick:expr;)*) => {
        $( #[gpui_kit::test] fn $name(cx: &mut TestAppContext) { assert_trail_whole(cx, $width, $height, $text, $pick); } )*
    };
}

trail_whole! {
    trail_is_whole_at_760_by_560_and_text_13_on_the_last_hop 760., 560., 13., usize::MAX;
    trail_is_whole_at_760_by_560_and_text_16_on_the_last_hop 760., 560., 16., usize::MAX;
    trail_is_whole_at_760_by_560_and_text_20_on_the_last_hop 760., 560., 20., usize::MAX;
    trail_is_whole_at_760_by_560_and_text_20_on_a_middle_hop 760., 560., 20., 12;
    trail_is_whole_at_760_by_560_and_text_20_on_the_first_hop 760., 560., 20., 0;
    trail_is_whole_in_a_page_narrower_than_the_columns 520., 560., 13., usize::MAX;
}
