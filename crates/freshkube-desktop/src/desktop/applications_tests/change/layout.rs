use super::*;

const CHANGE: layout_check::TablePage = layout_check::TablePage {
    page: "change-page",
    title: "change-title",
    title_text: "wonky-otter",
    table: "change-table-scroll",
    list: "change-list",
};

#[gpui_kit::test]
fn the_change_page_is_a_table_page_at_every_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            window.press("escape", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).applications().1, Page::Applications);
            assert!(change(&view, cx).is_some());
            let layout = layout_check::assert_table_page_from(window, cx, &CHANGE, "change-back");
            assert!(layout.group.is_some(), "{layout:#?}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_inspector_sits_beside_a_wide_table_and_under_a_narrow_one(cx: &mut TestAppContext) {
    for (width, height) in [(1600., 900.), (760., 560.)] {
        for size in [None, Some(20.)] {
            let (_runtime, handle, _view) = open(cx, width, height);
            cx.update_window(handle, |_, window, cx| {
                if let Some(size) = size {
                    crate::text_size::set(size, cx);
                }
                window.render_frame(cx);
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
    }
}

/// Stacked, the Inspector shrinks the trail as it opens; the Stage's gate
/// the page opens on is still in view once the table settles, at the
/// default text size and at 20.
#[gpui_kit::test]
fn a_stacked_page_opens_with_its_selection_in_view(cx: &mut TestAppContext) {
    for size in [None, Some(20.)] {
        let (_runtime, handle, view) = on_stage(cx, 760., 560.);
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            window.render_frame(cx);
            window.press("f", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            for _ in 0..10 {
                if window.simulate_next_frame(cx) == 0 {
                    break;
                }
            }
            window.render_frame(cx);
            let page = change(&view, cx).unwrap();
            let key = page.read(cx).selected().cloned().unwrap();
            assert_eq!(key.as_ref(), "prod-ams-eligible");
            let scroll = window.find("change-table-scroll").bounds();
            let row = window.find(hop(&key)).bounds();
            assert!(
                row.top() >= scroll.top() && row.bottom() <= scroll.bottom() + gpui_kit::px(0.5),
                "{size:?}: row {row:?} out of the table {scroll:?}"
            );
        })
        .unwrap();
    }
}

/// #522: beside the Inspector, Detail ends at a whole word, with its whole
/// text in the tooltip, in the row's font and the cell's colour.
#[gpui_kit::test]
fn detail_cuts_at_a_word_beside_the_inspector(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = change(&view, cx).unwrap();
        let ink = freshkube_ui::palette::palette(cx).ink_2;
        let mut cut = 0;
        for row in &page.read(cx).rows {
            let id = SharedString::from(format!("change-hop-{}-detail", row.key));
            let Some(style) = freshkube_ui::table::word_cut_style(id) else {
                continue;
            };
            // Shaped and drawn in the row's monospace and the cell's ink,
            // as the cells beside it are.
            assert_eq!(style.font.as_ref(), crate::ui::MONO_FONT, "{}", row.key);
            assert_eq!(style.size, gpui_kit::px(12.5), "{}", row.key);
            assert_eq!(style.color, ink, "{}", row.key);
            let shown = style.text;
            if shown == row.detail {
                continue;
            }
            cut += 1;
            let kept = shown.strip_suffix('…').expect("an ellipsis");
            assert!(row.detail.starts_with(kept), "{shown} of {}", row.detail);
            let next = row.detail[kept.len()..].chars().next();
            assert!(
                next.is_none_or(|c| c.is_whitespace() || c.is_ascii_punctuation() || c == '·'),
                "{shown:?} cuts a word of {:?}",
                row.detail
            );
        }
        assert!(cut > 0, "some Detail is cut beside the Inspector");
    })
    .unwrap();
}

/// The column headers' labels and bounds, in order.
fn headers(window: &mut gpui_kit::Window) -> Vec<(String, gpui_kit::Bounds<gpui_kit::Pixels>)> {
    (1..6usize)
        .filter_map(|ix| window.try_find(("change-sort", ix)))
        .map(|cell| (cell.label().unwrap_or_default().to_owned(), cell.bounds()))
        .collect()
}

/// Beside the Inspector at 1280, with the Applications column shown, Detail
/// comes right after Link and has the rest of the table's room, 240 dp;
/// Time and Read from follow it, scrolled away. At 760 the Inspector is
/// under the table and Detail shows whole.
#[gpui_kit::test]
fn detail_keeps_its_room_beside_the_inspector(cx: &mut TestAppContext) {
    for width in [1280., 760.] {
        let (_runtime, handle, _view) = open(cx, width, 880.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let scroll = window.find("change-table-scroll").bounds();
            let headers = headers(window);
            let labels: Vec<&str> = headers.iter().map(|(label, _)| label.as_str()).collect();
            assert_eq!(
                labels,
                ["Hop", "Link", "Detail", "Time", "Read from"],
                "{width}"
            );
            let detail = headers[2].1;
            let shown = detail.right().min(scroll.right()) - detail.left().max(scroll.left());
            assert!(
                shown >= gpui_kit::px(239.5),
                "{width}: {shown:?} of Detail shows in {scroll:?}"
            );
        })
        .unwrap();
    }
}

/// Link is as narrow as its words: the widest, with its glyph, still fits
/// inside the cell's padding, at the default text size and the largest.
#[gpui_kit::test]
fn every_link_word_fits_its_column(cx: &mut TestAppContext) {
    use freshkube_delivery::join::Confidence;
    use freshkube_ui::table::{CELL_PAD, TableColumn as _};
    use gpui_kit::{TextRun, font};

    let (_runtime, handle, _view) = open(cx, 1280., 880.);
    let columns = crate::applications::testing::change::columns();
    let link = columns
        .iter()
        .find(|column| column.label().as_ref() == "Link")
        .expect("a Link column")
        .width();
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            window.render_frame(cx);
            let room = crate::ui::dp_px(link - 2. * CELL_PAD, window);
            for confidence in [
                Confidence::Confirmed,
                Confidence::Claimed,
                Confidence::Unknown,
            ] {
                let word = crate::applications::testing::change::link_word(confidence);
                let run = TextRun {
                    len: word.len(),
                    font: font(".SystemUIFont"),
                    color: Default::default(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let text = window
                    .text_system()
                    .shape_line(word.into(), crate::ui::dp_px(12.5, window), &[run], None)
                    .width;
                let glyph = match crate::applications::testing::change::link_glyph(confidence) {
                    Some(_) => crate::ui::dp_px(10. + 5., window),
                    None => gpui_kit::px(0.),
                };
                assert!(
                    glyph + text <= room,
                    "{size:?}: {word} needs {:?} of {room:?}",
                    glyph + text
                );
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_change_toggle_is_remembered_when_the_change_opens_again(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("change-view-chain", cx);
        window.render_frame(cx);
        assert!(window.try_find("change-chain").is_some());
        // Escape with no selection returns to the owning application page.
        window.press("escape", cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(PROD_AMS, cx);
        window.press("f", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("change-chain").is_some());
    })
    .unwrap();
}
