use super::*;

#[gpui_kit::test]
fn beside_the_inspector_meets_the_table_and_reaches_the_edge(cx: &mut TestAppContext) {
    let (handle, _, _) = open(cx, None, true, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let table = window.find("table").bounds();
        let inspector = window.find("inspector").bounds();
        close("no gap", inspector.left(), table.right());
        close(
            "right edge",
            inspector.right(),
            window.viewport_size().width,
        );
        close("width", inspector.size.width, dp_px(WIDTH, window));
        close("same top", inspector.top(), table.top());
        let title = window.find("inspector-title").bounds();
        close(
            "heading inset",
            title.left() - inspector.left(),
            dp_px(12., window),
        );
        close(
            "heading top",
            title.top() - inspector.top(),
            dp_px(12., window),
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn stacked_the_inspector_spans_the_width_under_the_table(cx: &mut TestAppContext) {
    let (handle, _, _) = open(cx, None, false, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let table = window.find("table").bounds();
        let inspector = window.find("inspector").bounds();
        close("no gap", inspector.top(), table.bottom());
        close("left", inspector.left(), px(0.));
        close("width", inspector.size.width, window.viewport_size().width);
        close(
            "table width",
            table.size.width,
            window.viewport_size().width,
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn without_an_inspector_the_table_fills_the_split(cx: &mut TestAppContext) {
    let (handle, _, _) = open(cx, None, true, false);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("inspector").is_none());
        let table = window.find("table").bounds();
        close("width", table.size.width, window.viewport_size().width);
        close(
            "split",
            window.find("split").bounds().size.width,
            table.size.width,
        );
    })
    .unwrap();
}

/// A page's own stacked heights: the table starts at its `lead`, keeps its
/// least and stops at its most, and a short page gives the split the two
/// leasts.
#[gpui_kit::test]
fn a_page_sets_the_stacked_heights(cx: &mut TestAppContext) {
    let heights = Stacked {
        lead: Pane::new(300.).least(300.).most(700.),
        trail: Pane::new(220.).least(160.),
    };
    assert_ne!(heights, Stacked::default());
    for (tall, table_height) in [(520., 300.), (2000., 700.)] {
        let (handle, page, _) = open_with(cx, None, false, true, heights, tall);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let table = window.find("table").bounds();
            let inspector = window.find("inspector").bounds();
            close("table", table.size.height, px(table_height));
            close("under it", inspector.top(), table.bottom());
            close("to the bottom", inspector.bottom(), px(tall));
            let split = &page.read(cx).split;
            assert_eq!(split.short_height(false, true), 460.);
            assert_eq!(split.short_height(true, true), SHORT_HEIGHT);
        })
        .unwrap();
    }
    assert_eq!(
        short_height(false, true),
        LIST_MIN_HEIGHT + MIN_HEIGHT,
        "the default"
    );
}
