use super::*;

#[gpui_kit::test]
fn the_body_keeps_its_padding_outside_the_scroll(cx: &mut TestAppContext) {
    let (handle, page, _) = open(cx, None, true, true);
    for text in [13., 20.] {
        for beside in [true, false] {
            cx.update_window(handle, |_, window, cx| {
                crate::text_size::set(text, cx);
                page.update(cx, |page, cx| {
                    page.beside = beside;
                    cx.notify();
                });
                settle(window, cx);
                let inspector = window.find("inspector").bounds();
                let heading = window.find("inspector-heading").bounds();
                let body = window.find("inspector-body").bounds();
                let pad = dp_px(PANE_PADDING, window);
                close("above the scroll", body.top(), heading.bottom() + pad);
                close("below the scroll", body.bottom(), inspector.bottom() - pad);
                close("left inset", body.left(), inspector.left() + pad);
                close("right inset", body.right(), inspector.right() - pad);
                window.scroll(
                    "inspector-body",
                    ScrollDelta::Pixels(point(px(0.), px(-100.))),
                    cx,
                );
                window.render_frame(cx);
                assert_eq!(window.find("inspector-body").bounds(), body);
                assert_eq!(window.find("inspector-heading").bounds(), heading);
                assert!(window.find("inspector-tall").bounds().top() < body.top());
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn the_body_scrolls_under_a_heading_that_stays(cx: &mut TestAppContext) {
    let (handle, _, _) = open(cx, None, true, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let body = window.find("inspector-body").bounds();
        assert!(body.bottom() <= window.viewport_size().height);
        let title = window.find("inspector-title").bounds().top();
        let tall = window.find("inspector-tall").bounds().top();
        window.scroll(
            "inspector-body",
            ScrollDelta::Pixels(point(px(0.), px(-300.))),
            cx,
        );
        window.render_frame(cx);
        close(
            "heading stays",
            window.find("inspector-title").bounds().top(),
            title,
        );
        close(
            "body moved",
            window.find("inspector-tall").bounds().top(),
            tall - px(300.),
        );
    })
    .unwrap();
}

/// What was scrolled in the body stays with the body only until the
/// selection moves or the inspector closes: a new selection starts at the
/// top, and so does the inspector when it opens again.
#[gpui_kit::test]
fn the_body_goes_back_to_the_top_on_another_selection_and_on_reopening(cx: &mut TestAppContext) {
    let (handle, page) = short_page(cx, 2000.);
    let scrolled = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            settle(window, cx);
            wheel(window, "inspector-body", up(40.), cx);
            body_offset(window)
        })
        .unwrap()
    };
    let top = cx
        .update_window(handle, |_, window, cx| {
            settle(window, cx);
            body_offset(window)
        })
        .unwrap();
    close("scrolled", scrolled(cx), top - px(40.));
    page.update(cx, |page, cx| {
        page.selection = 1;
        cx.notify();
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        close("another selection", body_offset(window), top);
    })
    .unwrap();
    close("scrolled again", scrolled(cx), top - px(40.));
    for open in [false, true] {
        page.update(cx, |page, cx| {
            page.open = open;
            cx.notify();
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| settle(window, cx))
            .unwrap();
    }
    cx.update_window(handle, |_, window, _| {
        close("reopened", body_offset(window), top);
    })
    .unwrap();
}

/// A body that fits has nothing to scroll, so the wheel over it is the
/// page's.
#[gpui_kit::test]
fn a_body_that_fits_passes_the_wheel_to_the_page(cx: &mut TestAppContext) {
    let (handle, _) = short_page(cx, 4.);
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        let (top, frame) = (body_offset(window), frame_top(window));
        wheel(window, "inspector-body", up(40.), cx);
        assert!(frame_top(window) < frame, "the page scrolls");
        close("the body stays", body_offset(window), top);
    })
    .unwrap();
}

/// A wheel that turns sideways more than up is not the body's.
#[gpui_kit::test]
fn a_sideways_wheel_passes_through_the_body(cx: &mut TestAppContext) {
    let (handle, _) = short_page(cx, 2000.);
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        let top = body_offset(window);
        wheel(
            window,
            "inspector-body",
            ScrollDelta::Pixels(point(px(-60.), px(-10.))),
            cx,
        );
        close("the body stays", body_offset(window), top);
    })
    .unwrap();
}

/// A mouse wheel's line deltas move the body by lines.
#[gpui_kit::test]
fn a_wheel_in_lines_scrolls_the_body(cx: &mut TestAppContext) {
    let (handle, _) = short_page(cx, 2000.);
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        let (top, frame) = (body_offset(window), frame_top(window));
        let lines = window.line_height() * 3.;
        wheel(
            window,
            "inspector-body",
            ScrollDelta::Lines(point(0., -3.)),
            cx,
        );
        close("the body moved", body_offset(window), top - lines);
        close("the page stays", frame_top(window), frame);
    })
    .unwrap();
}

/// Beside the table nothing scrolls around the split: the wheel over the
/// body moves the body, and over the table it leaves the body alone.
#[gpui_kit::test]
fn beside_the_table_the_wheel_moves_what_it_is_over(cx: &mut TestAppContext) {
    let (handle, _, _) = open(cx, None, true, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let tall = window.find("inspector-tall").bounds().top();
        window.scroll("table", up(100.), cx);
        window.render_frame(cx);
        close(
            "the table's wheel",
            window.find("inspector-tall").bounds().top(),
            tall,
        );
        window.scroll("inspector-body", up(100.), cx);
        window.render_frame(cx);
        close(
            "the body's wheel",
            window.find("inspector-tall").bounds().top(),
            tall - px(100.),
        );
        // At its end the body keeps the wheel from nothing: there is no
        // page behind it to take it.
        window.scroll("inspector-body", up(5000.), cx);
        window.render_frame(cx);
        window.scroll("inspector-body", up(100.), cx);
        window.render_frame(cx);
        assert!(
            window.find("inspector-tall").bounds().bottom()
                <= window.viewport_size().height + px(1.)
        );
    })
    .unwrap();
}

/// A flick that starts on a body with room stays on it to the end of its
/// momentum, and a gesture that starts at the body's end is the page's.
#[gpui_kit::test]
fn a_gesture_stays_with_whoever_it_started_on(cx: &mut TestAppContext) {
    let (handle, _) = short_page(cx, 2000.);
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        let frame = frame_top(window);
        gesture(window, "inspector-body", 10., TouchPhase::Started, cx);
        gesture(window, "inspector-body", 5000., TouchPhase::Moved, cx);
        let end = body_offset(window);
        // The fingers lift and the momentum goes on past the body's end.
        gesture(window, "inspector-body", 10., TouchPhase::Ended, cx);
        gesture(window, "inspector-body", 300., TouchPhase::Moved, cx);
        close("the flick stays on the body", frame_top(window), frame);
        close("at its end", body_offset(window), end);
        // A new gesture at the end belongs to the page.
        gesture(window, "inspector-body", 10., TouchPhase::Started, cx);
        gesture(window, "inspector-body", 100., TouchPhase::Moved, cx);
        assert!(frame_top(window) < frame, "the page scrolls");
    })
    .unwrap();
}

/// The page's momentum is not taken by a body it passes over.
#[gpui_kit::test]
fn a_gesture_that_started_on_the_page_is_not_taken_by_the_body(cx: &mut TestAppContext) {
    let (handle, _) = short_page(cx, 2000.);
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        let (top, frame) = (body_offset(window), frame_top(window));
        gesture(window, "toolbar", 10., TouchPhase::Started, cx);
        gesture(window, "inspector-body", 20., TouchPhase::Moved, cx);
        close("the body stays", body_offset(window), top);
        assert!(frame_top(window) < frame, "the page goes on");
    })
    .unwrap();
}
