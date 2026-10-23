use super::*;

/// A sideways wheel scrolls the columns and leaves the rows where they
/// were: the list inside doesn't take it for a vertical scroll (#93).
#[gpui_kit::test]
fn a_sideways_wheel_scrolls_only_the_columns(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, Wide::new(2, 30, true), 900.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let top = top_of(window, "Column 4 1");
        wheel(window, -100., 0., cx);
        let left = inset(window, "Column 4 1");
        assert!((left - 700.).abs() <= 1.5, "no sideways scroll: {left}");
        let moved = top - top_of(window, "Column 4 1");
        assert!(moved.abs() <= 0.5, "the rows moved {moved}");
    })
    .unwrap();
}

/// A plain wheel over a table wider than its window scrolls the rows and
/// leaves the columns at the left edge: the sideways scroll around them
/// doesn't take it for a sideways one.
#[gpui_kit::test]
fn a_vertical_wheel_scrolls_only_the_rows(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, Wide::new(2, 30, true), 900.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let top = top_of(window, "Column 4 8");
        wheel(window, 0., -100., cx);
        let moved = top - top_of(window, "Column 4 8");
        assert!(
            (moved - 100.).abs() <= 1.5,
            "the rows moved {moved}, not 100"
        );
        let left = inset(window, ("wide-sort", 4usize));
        assert!((left - 800.).abs() <= 1.5, "the columns moved: {left}");
        assert!(window.try_find("wide-pinned-header").is_none());
    })
    .unwrap();
}

/// A swipe that is mostly sideways keeps to that axis, so a trackpad's
/// slight drift doesn't move the rows too.
#[gpui_kit::test]
fn a_mostly_sideways_swipe_keeps_to_its_axis(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, Wide::new(2, 30, true), 900.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let top = top_of(window, "Column 4 1");
        wheel(window, -100., -10., cx);
        let left = inset(window, "Column 4 1");
        assert!((left - 700.).abs() <= 1.5, "no sideways scroll: {left}");
        let moved = top - top_of(window, "Column 4 1");
        assert!(moved.abs() <= 0.5, "the rows moved {moved}");
    })
    .unwrap();
}

/// A filter that leaves no rows after a sideways scroll still shows why,
/// inside the table's width.
#[gpui_kit::test]
fn the_empty_state_shows_after_a_sideways_scroll(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        cx.set_reduce_motion(true);
    });
    let (handle, wide) = open(cx, Wide::new(0, 3, false), 600.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.scroll(
            "wide-table-scroll",
            ScrollDelta::Pixels(point(px(-500.), px(0.))),
            cx,
        );
        let viewport = window.find("wide-table-scroll").bounds();
        let row = window.find("wide-row-0").bounds();
        assert!(
            row.left() < viewport.left() - px(400.),
            "the rows didn't scroll sideways: {row:?} in {viewport:?}"
        );
    })
    .unwrap();
    cx.update(|cx| {
        wide.update(cx, |wide, cx| {
            wide.rows = 0;
            cx.notify();
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let viewport = window.find("wide-table-scroll").bounds();
        let empty = window.find("wide-empty");
        let bounds = empty.bounds();
        assert!(
            bounds.left() >= viewport.left() - px(1.)
                && bounds.right() <= viewport.right() + px(1.),
            "the empty state at {bounds:?} leaves the table's width {viewport:?}"
        );
        assert!(empty.visible());
    })
    .unwrap();
}
