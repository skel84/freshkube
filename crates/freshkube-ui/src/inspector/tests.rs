use std::cell::RefCell;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, Context, Render, ScrollDelta, TestAppContext, point, px, size};

use super::*;

/// A table, and an inspector with a heading and a body taller than the
/// window, in a split `beside` or stacked.
struct Page {
    split: InspectorSplit,
    beside: bool,
    open: bool,
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let table = div()
            .id("table")
            .test_support()
            .size_full()
            .into_any_element();
        let inspector = self.open.then(|| {
            Inspector::new("inspector")
                .heading(div().id("inspector-title").test_support().child("INC-12"))
                .child(
                    div()
                        .id("inspector-tall")
                        .test_support()
                        .flex_none()
                        .h(px(2000.))
                        .w_full(),
                )
                .render(cx)
                .into_any_element()
        });
        div().size_full().flex().child(split(
            "split",
            &self.split,
            self.beside,
            table,
            inspector,
            window,
        ))
    }
}

/// The page in a window 1200 by 600, at the default text size, with the
/// inspector `width` dp wide; and the widths `remember` heard.
fn open(
    cx: &mut TestAppContext,
    width: Option<f32>,
    beside: bool,
    open: bool,
) -> (AnyWindowHandle, Entity<Page>, Rc<RefCell<Vec<f32>>>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        cx.set_reduce_motion(true);
    });
    let heard = Rc::new(RefCell::new(vec![]));
    let mut page = None;
    let handle = cx.open_window(size(px(1200.), px(600.)), |window, cx| {
        let remember = {
            let heard = heard.clone();
            move |width, _: &mut App| heard.borrow_mut().push(width)
        };
        let view = cx.new(|cx| Page {
            split: InspectorSplit::new(width, remember, cx),
            beside,
            open,
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (handle.into(), page.unwrap(), heard)
}

fn close(what: &str, actual: Pixels, expected: Pixels) {
    assert!(
        (actual - expected).abs() <= px(1.),
        "{what}: {actual:?}, expected {expected:?}"
    );
}

fn near(what: &str, actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 1.,
        "{what}: {actual}, expected {expected}"
    );
}

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

#[test]
fn a_remembered_width_is_kept_above_the_least() {
    assert_eq!(start_width(None), WIDTH);
    assert_eq!(start_width(Some(600.)), 600.);
    assert_eq!(start_width(Some(100.)), MIN_WIDTH);
    assert_eq!(start_width(Some(f32::NAN)), WIDTH);
    assert_eq!(start_width(Some(f32::INFINITY)), WIDTH);
}

#[gpui_kit::test]
fn the_inspector_opens_at_the_remembered_width(cx: &mut TestAppContext) {
    let (handle, _, _) = open(cx, Some(600.), true, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let inspector = window.find("inspector").bounds();
        close("width", inspector.size.width, dp_px(600., window));
    })
    .unwrap();
}

/// `resize_panel` takes the path a drag's end takes; the width is heard
/// once, in dp, so it scales with the text size.
#[gpui_kit::test]
fn a_resize_reports_the_width_once_in_dp(cx: &mut TestAppContext) {
    let (handle, page, heard) = open(cx, None, true, true);
    cx.update_window(handle, |_, _, cx| crate::text_size::set(20., cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let state = page.read(cx).split.beside_state().clone();
        state.update(cx, |state, cx| state.resize_panel(1, px(650.), window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let heard = heard.borrow().clone();
    assert_eq!(heard.len(), 1, "{heard:?}");
    near("heard", heard[0], 650. * BASE_TEXT / 20.);
    cx.update(|cx| near("kept", page.read(cx).split.width(), heard[0]));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        close(
            "drawn",
            window.find("inspector").bounds().size.width,
            px(650.),
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn dragging_the_hairline_widens_the_inspector(cx: &mut TestAppContext) {
    let (handle, page, heard) = open(cx, None, true, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let inspector = window.find("inspector").bounds();
        let y = inspector.center().y;
        window.drag(
            point(inspector.left(), y),
            point(inspector.left() - px(100.), y),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    let heard = heard.borrow().clone();
    assert_eq!(heard.len(), 1, "{heard:?}");
    near("heard", heard[0], WIDTH + 100.);
    cx.update(|cx| near("kept", page.read(cx).split.width(), WIDTH + 100.));
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
