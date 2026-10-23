use super::*;

/// Bare, the table draws no card: only a hairline above and below, on
/// the window's background, which its pinned cells paint too so they
/// stay opaque. Carded, it keeps the rounded card, and its pinned cells
/// paint the card's surface.
#[gpui_kit::test]
fn a_bare_table_draws_hairlines_and_its_pinned_cells_match_it(cx: &mut TestAppContext) {
    for carded in [false, true] {
        let wide = Wide::new(2, 4, true);
        let wide = if carded { wide.carded() } else { wide };
        let (handle, _) = open(cx, wide, 900.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            scroll_right(window, 100., cx);
            let scale = window.scale_factor();
            let view = window.find("wide-table-scroll").bounds().scale(scale);
            let quads = window.painted_quads();
            // What frames the table: a bordered quad around its scroll.
            // GPUI splits a border into clipped strips that share its
            // geometry, and paints the fill as a quad of its own.
            let mut frames: Vec<_> = (quads.iter())
                .filter(|quad| {
                    let (b, w) = (quad.bounds, quad.border_widths);
                    (w.top.0 > 0. || w.left.0 > 0.)
                        && b.left() <= view.left()
                        && b.right() >= view.right()
                        && b.top() <= view.top()
                        && b.bottom() >= view.bottom()
                })
                .collect();
            frames.dedup_by(|a, b| a.bounds == b.bounds && a.border_widths == b.border_widths);
            assert_eq!(frames.len(), 1, "carded {carded}: {frames:#?}");
            let frame = frames[0];
            let (sides, corner) = (frame.border_widths, frame.corner_radii.top_left.0);
            let fill = if carded {
                assert!(sides.left.0 > 0. && sides.right.0 > 0., "{frame:#?}");
                assert!((corner - 12. * scale).abs() < 0.5, "{frame:#?}");
                palette(cx).surface
            } else {
                assert!(sides.top.0 > 0. && sides.bottom.0 > 0., "{frame:#?}");
                assert!(sides.left.0 == 0. && sides.right.0 == 0., "{frame:#?}");
                assert_eq!(corner, 0., "{frame:#?}");
                cx.theme().background
            };
            assert!(
                quads.iter().any(|quad| {
                    quad.bounds == frame.bounds && quad.background == Background::from(fill)
                }),
                "carded {carded}: the table isn't filled with {fill:?}"
            );
            // The pinned run, 400 wide, paints the same fill.
            let run = crate::ui::dp_px(400., window).scale(scale).0;
            assert!(
                quads.iter().any(|quad| {
                    quad.background == Background::from(fill)
                        && (quad.bounds.size.width.0 - run).abs() < 1.
                }),
                "carded {carded}: no pinned run on the table's fill"
            );
        })
        .unwrap();
    }
}

/// The counts and the legend share one 26 row under the rows: the
/// counts first at their width, the legend truncating after them; no
/// row when there is neither.
#[gpui_kit::test]
fn the_footer_holds_the_counts_then_the_legend(cx: &mut TestAppContext) {
    let long = "Meters: band = request · line = use · end = limit ".repeat(8);
    let (handle, _) = open(
        cx,
        Wide::new(0, 4, false).footer(Some(3), Some(&long)),
        600.,
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let footer = window.find("wide-footer");
        assert_eq!(
            footer.bounds().size.height,
            ui::dp_px(FOOTER_HEIGHT, window)
        );
        let rows = window.find("wide-table-scroll").bounds();
        assert!(
            footer.bounds().top() >= rows.bottom(),
            "the footer sits under the rows"
        );
        // Nothing sits between the table's top and its header.
        let header = ui::dp_px(HEADER_HEIGHT, window);
        assert_eq!(rows.top(), window.find("wide-list").bounds().top() - header);
        let (marks, showing, legend) = (
            window.find("wide-marks"),
            window.find("wide-showing"),
            window.find("wide-legend").bounds(),
        );
        assert!(marks.bounds().right() <= showing.bounds().left());
        assert!(showing.bounds().right() <= legend.left());
        // The legend truncates inside the row; the counts keep their width.
        assert!(legend.right() <= footer.bounds().right(), "{legend:?}");
        assert!(showing.bounds().right() < footer.bounds().right());
    })
    .unwrap();
    for wide in [Wide::new(0, 4, false), Wide::new(0, 0, false)] {
        let (handle, _) = open(cx, wide, 600.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("wide-footer").is_none());
        })
        .unwrap();
    }
}

/// A group's glyph sits on its rows' glyphs, and its label starts where
/// the next column's text does, at any text size (#129).
#[gpui_kit::test]
fn group_glyphs_and_labels_line_up_with_the_rows(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, Wide::new(0, 2, true).glyphs(), 1400.);
    for text in [13., 20.] {
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(text, cx);
            window.render_frame(cx);
            let group = ElementId::from("wide-group");
            let slot = window.find((group.clone(), "glyph")).bounds();
            let glyph = window.find(("wide-glyph", 1usize)).bounds();
            assert!(
                (slot.center().x - glyph.center().x).abs() < px(0.25),
                "text {text}: group glyph at {:?}, row glyph at {:?}",
                slot.center().x,
                glyph.center().x
            );
            let label = window.find((group, "label")).bounds().left();
            let name = window.find("Column 1 1").bounds().left() + ui::dp_px(CELL_PAD, window);
            assert!(
                (label - name).abs() < px(0.25),
                "text {text}: group label at {label:?}, row text at {name:?}"
            );
        })
        .unwrap();
    }
}

/// In a table wider than its view, a group's actions stay in view,
/// scrolled or not; in one that fits, its line is as wide as its rows
/// (#128).
#[gpui_kit::test]
fn group_actions_stay_in_view(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, Wide::new(2, 4, true), 900.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for scroll in [0., 100., 400.] {
            scroll_right(window, scroll, cx);
            window.render_frame(cx);
            let viewport = window.find("wide-table-scroll").bounds();
            let action = window.find("wide-group-action").bounds();
            assert!(
                action.right() <= viewport.right() + px(0.5),
                "scrolled {scroll}: action ends at {:?}, view at {:?}",
                action.right(),
                viewport.right()
            );
            assert!(action.left() >= viewport.left());
            assert!(inset(window, "wide-group").abs() <= 1.5);
            let group = window.find("wide-group").bounds();
            assert!((group.size.width - viewport.size.width).abs() <= px(1.5));
        }
    })
    .unwrap();

    let (handle, _) = open(cx, Wide::new(2, 4, true), 1400.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let group = window.find("wide-group").bounds();
        let row = window.find("wide-row-1").bounds();
        assert_eq!(group.left(), row.left());
        assert_eq!(group.size.width, row.size.width);
    })
    .unwrap();
}
