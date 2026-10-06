use std::cell::RefCell;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, Bounds, Context, Render, ScrollDelta, TestAppContext, point, px, size,
};

use super::*;

/// A table, and an inspector with a heading and a body taller than the
/// window, in a split `beside` or stacked.
struct Page {
    split: InspectorSplit,
    beside: bool,
    open: bool,
    /// The table's own height, given to the split while rendering.
    lead: Option<f32>,
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(lead) = self.lead {
            self.split.lead_start(lead, cx);
        }
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

fn install(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        cx.set_reduce_motion(true);
    });
}

/// The page in a window 1200 by 600, at the default text size, with the
/// inspector `width` dp wide; and the widths `remember` heard.
fn open(
    cx: &mut TestAppContext,
    width: Option<f32>,
    beside: bool,
    open: bool,
) -> (AnyWindowHandle, Entity<Page>, Rc<RefCell<Vec<f32>>>) {
    open_with(cx, width, beside, open, Stacked::default(), 600.)
}

/// [`open`], with the split stacked at `heights` in a window `tall` high.
fn open_with(
    cx: &mut TestAppContext,
    width: Option<f32>,
    beside: bool,
    open: bool,
    heights: Stacked,
    tall: f32,
) -> (AnyWindowHandle, Entity<Page>, Rc<RefCell<Vec<f32>>>) {
    install(cx);
    let heard = Rc::new(RefCell::new(vec![]));
    let mut page = None;
    let handle = cx.open_window(size(px(1200.), px(tall)), |window, cx| {
        let remember = {
            let heard = heard.clone();
            move |width, _: &mut App| heard.borrow_mut().push(width)
        };
        let view = cx.new(|cx| Page {
            split: InspectorSplit::new(width, remember, cx).stacked(heights),
            beside,
            open,
            lead: None,
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

/// An inspector with tabs: a heading, a banner, the tab strip, content
/// that lays itself out, and a footer.
struct Tabbed(TabStrip);

impl Render for Tabbed {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        Inspector::new("inspector")
            .heading(div().id("inspector-title").test_support().child("api-7d9"))
            .banner(Some(div().h(px(30.)).child("Showing it as last read.")))
            .tabs(
                &self.0,
                self.0
                    .row("tabs-row", 0)
                    .child(tab("tab-overview", "Overview", true, cx))
                    .child(tab("tab-yaml", "YAML", false, cx)),
            )
            .child(div().id("unused").test_support())
            .content(
                div()
                    .id("content-tall")
                    .test_support()
                    .flex_none()
                    .h(px(2000.))
                    .w_full(),
            )
            .footer(Some("Copied the document"))
            .render(cx)
    }
}

#[gpui_kit::test]
fn tabs_sit_under_the_heading_and_the_content_fills_the_rest(cx: &mut TestAppContext) {
    install(cx);
    let handle = cx.open_window(size(px(600.), px(500.)), |window, cx| {
        let view = cx.new(|_| Tabbed(TabStrip::default()));
        Root::new(view, window, cx)
    });
    let handle: AnyWindowHandle = handle.into();
    cx.run_until_parked();
    for text_size in [13., 20.] {
        cx.update_window(handle, |_, _, cx| crate::text_size::set(text_size, cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let inspector = window.find("inspector").bounds();
            let heading = window.find("inspector-heading").bounds();
            let banner = window.find("inspector-banner").bounds();
            let tabs = window.find("inspector-tabs").bounds();
            let overview = window.find("tab-overview").bounds();
            let yaml = window.find("tab-yaml").bounds();
            let content = window.find("inspector-content").bounds();
            let footer = window.find("inspector-footer").bounds();
            assert!(banner.top() >= heading.bottom(), "{heading:?} {banner:?}");
            assert!(tabs.top() >= banner.bottom(), "{banner:?} {tabs:?}");
            close(
                "tab height",
                overview.size.height,
                dp_px(TAB_HEIGHT, window),
            );
            close("same row", yaml.top(), overview.top());
            close(
                "tabs inset",
                overview.left() - inspector.left(),
                dp_px(PANE_PADDING, window),
            );
            close("strip", tabs.bottom(), overview.bottom() + px(1.));
            close("content under the strip", content.top(), tabs.bottom());
            close("footer under the content", footer.top(), content.bottom());
            close("footer at the bottom", footer.bottom(), inspector.bottom());
            close("fills", inspector.bottom(), window.viewport_size().height);
            // The content lays itself out: the inspector scrolls nothing
            // and draws no body.
            close(
                "tall content starts at the top",
                window.find("content-tall").bounds().top(),
                content.top(),
            );
            assert!(window.try_find("inspector-body").is_none());
            assert!(window.try_find("unused").is_none());
            assert_eq!(window.find("tab-overview").selected(), Some(true));
            assert_eq!(window.find("tab-yaml").selected(), Some(false));
        })
        .unwrap();
    }
}

/// A table whose height follows its data gives it while rendering: each
/// new height lays the split out again, within the lead's least and most,
/// until the user drags it.
#[gpui_kit::test]
fn a_given_start_applies_until_the_user_drags(cx: &mut TestAppContext) {
    let heights = Stacked {
        lead: Pane::new(300.).least(300.).most(700.),
        trail: Pane::new(380.).least(220.),
    };
    let (handle, page, _) = open_with(cx, None, false, true, heights, 1000.);
    let table_after = |lead: f32, cx: &mut TestAppContext| {
        page.update(cx, |page, cx| {
            page.lead = Some(lead);
            cx.notify();
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let table = window.find("table").bounds();
            let inspector = window.find("inspector").bounds();
            close("under it", inspector.top(), table.bottom());
            close("to the bottom", inspector.bottom(), px(1000.));
            table.size.height
        })
        .unwrap()
    };
    close("given", table_after(400., cx), px(400.));
    close("a new height", table_after(520., cx), px(520.));
    close("its least", table_after(86., cx), px(300.));
    close("its most", table_after(900., cx), px(700.));

    cx.update_window(handle, |_, window, cx| {
        let state = page.read(cx).split.stacked_state().clone();
        state.update(cx, |state, cx| state.resize_panel(0, px(450.), window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    close("dragged", table_after(900., cx), px(450.));
    close("still the user's", table_after(400., cx), px(450.));
}

/// An inspector `width` wide whose strip holds seven tabs, `active` shown.
struct Strip {
    strip: TabStrip,
    width: f32,
    active: usize,
}

const LABELS: [&str; 7] = [
    "Overview", "YAML", "Events", "Logs", "Shell", "Ports", "Metrics",
];

impl Render for Strip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = self
            .strip
            .row("strip-row", self.active)
            .children(LABELS.iter().enumerate().map(|(ix, label)| {
                tab(
                    SharedString::from(format!("strip-tab-{ix}")),
                    *label,
                    ix == self.active,
                    cx,
                )
            }));
        div().size_full().child(
            div().w(px(self.width)).h_full().child(
                Inspector::new("inspector")
                    .heading(div().child("api-7d9"))
                    .tabs(&self.strip, row)
                    .content(div())
                    .render(cx),
            ),
        )
    }
}

fn open_strip(
    cx: &mut TestAppContext,
    width: f32,
    active: usize,
) -> (AnyWindowHandle, Entity<Strip>) {
    install(cx);
    let mut page = None;
    let handle = cx.open_window(size(px(1000.), px(400.)), |window, cx| {
        let view = cx.new(|_| Strip {
            strip: TabStrip::default(),
            width,
            active,
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (handle.into(), page.unwrap())
}

/// Draws until the strip stops asking for frames.
fn settle(window: &mut Window, cx: &mut App) {
    for _ in 0..4 {
        window.render_frame(cx);
        if window.simulate_next_frame(cx) == 0 {
            return;
        }
    }
    panic!("the tab strip keeps moving");
}

fn strip_tab(window: &mut Window, ix: usize) -> Bounds<Pixels> {
    window
        .find(SharedString::from(format!("strip-tab-{ix}")))
        .bounds()
}

/// Whether tab `ix` shows whole in the row.
fn whole(window: &mut Window, ix: usize) -> bool {
    let row = window.find("strip-row").bounds();
    let item = strip_tab(window, ix);
    item.left() >= row.left() - px(0.5) && item.right() <= row.right() + px(0.5)
}

/// Whether tab `ix` shows whole and clear of the cut ends' fades.
fn clear(window: &mut Window, ix: usize) -> bool {
    let item = strip_tab(window, ix);
    whole(window, ix)
        && window
            .try_find("inspector-tabs-earlier")
            .is_none_or(|end| item.left() >= end.bounds().right() - px(0.5))
        && window
            .try_find("inspector-tabs-later")
            .is_none_or(|end| item.right() <= end.bounds().left() + px(0.5))
}

#[gpui_kit::test]
fn a_strip_with_room_shows_every_tab_and_no_chevron(cx: &mut TestAppContext) {
    let (handle, view) = open_strip(cx, 900., 0);
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        for ix in 0..LABELS.len() {
            assert!(whole(window, ix), "tab {ix} is cut");
        }
        assert!(window.try_find("inspector-tabs-earlier").is_none());
        assert!(window.try_find("inspector-tabs-later").is_none());
        assert_eq!(view.read(cx).strip.edges(), Edges::default());
    })
    .unwrap();
}

/// A strip learns it is cut while drawing, and asks for the frame that
/// shows its chevron itself: nothing else would draw one.
#[gpui_kit::test]
fn a_narrow_strip_marks_its_cut_end_without_input(cx: &mut TestAppContext) {
    let (handle, view) = open_strip(cx, 260., 0);
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        assert_eq!(
            view.read(cx).strip.edges(),
            Edges {
                earlier: false,
                later: true
            }
        );
        assert!(window.try_find("inspector-tabs-earlier").is_none());
        let tabs = window.find("inspector-tabs").bounds();
        let later = window.find("inspector-tabs-later").bounds();
        close("chevron at the edge", later.right(), tabs.right());
        close(
            "chevron as tall as a tab",
            later.size.height,
            dp_px(TAB_HEIGHT, window),
        );
        // The hairline under the strip stays drawn.
        assert!(
            later.bottom() <= tabs.bottom() - px(1.) + px(0.5),
            "{later:?} {tabs:?}"
        );
        assert!(whole(window, 0));
        assert!(!whole(window, LABELS.len() - 1));
    })
    .unwrap();
    // Unlike a test's frame, an app's frame only comes when asked for.
    view.update(cx, |strip, cx| {
        strip.strip = TabStrip::default();
        cx.notify();
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.simulate_next_frame(cx) > 0,
            "the strip asked for no frame"
        );
    })
    .unwrap();
}

/// The arrows and Command-Shift-] move the active tab; the strip brings
/// it in clear of the fades, wherever it is.
#[gpui_kit::test]
fn the_active_tab_comes_into_view_when_the_keyboard_moves_to_it(cx: &mut TestAppContext) {
    let (handle, view) = open_strip(cx, 260., 0);
    cx.update_window(handle, |_, window, cx| settle(window, cx))
        .unwrap();
    for active in [5, LABELS.len() - 1, 2, 0] {
        view.update(cx, |strip, cx| {
            strip.active = active;
            cx.notify();
        });
        cx.update_window(handle, |_, window, cx| {
            settle(window, cx);
            assert!(clear(window, active), "tab {active} isn't in view");
        })
        .unwrap();
    }
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("inspector-tabs-earlier").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_active_tab_comes_back_into_view_after_a_resize(cx: &mut TestAppContext) {
    let (handle, view) = open_strip(cx, 900., 5);
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        assert!(clear(window, 5));
    })
    .unwrap();
    for width in [260., 200., 320.] {
        view.update(cx, |strip, cx| {
            strip.width = width;
            cx.notify();
        });
        cx.update_window(handle, |_, window, cx| {
            settle(window, cx);
            assert!(clear(window, 5), "tab 5 isn't in view at {width}");
            assert!(window.try_find("inspector-tabs-earlier").is_some());
        })
        .unwrap();
    }
}

/// A chevron brings in the first tab cut at its end, and leaves the active
/// tab as it was; clicking back reaches the first tab and drops the chevron.
#[gpui_kit::test]
fn a_chevron_brings_in_the_next_cut_tab(cx: &mut TestAppContext) {
    let (handle, view) = open_strip(cx, 260., 0);
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        let next = (0..LABELS.len())
            .find(|ix| !clear(window, *ix))
            .expect("a cut tab");
        window.click("inspector-tabs-later", cx);
        settle(window, cx);
        assert!(clear(window, next), "tab {next} isn't in view");
        assert!(window.try_find("inspector-tabs-earlier").is_some());
        for _ in 0..LABELS.len() {
            if window.try_find("inspector-tabs-earlier").is_none() {
                break;
            }
            window.click("inspector-tabs-earlier", cx);
            settle(window, cx);
        }
        assert!(window.try_find("inspector-tabs-earlier").is_none());
        assert!(whole(window, 0));
        assert_eq!(window.find("strip-tab-0").selected(), Some(true));
    })
    .unwrap();
    assert_eq!(view.read_with(cx, |strip, _| strip.active), 0);
}

/// A plain wheel scrolls the strip sideways, and it stays where it was
/// left until the active tab or the width changes.
#[gpui_kit::test]
fn a_wheel_scroll_stays_until_the_active_tab_changes(cx: &mut TestAppContext) {
    let (handle, view) = open_strip(cx, 260., 0);
    let left = cx
        .update_window(handle, |_, window, cx| {
            settle(window, cx);
            let left = strip_tab(window, 0).left();
            window.scroll(
                "strip-row",
                ScrollDelta::Pixels(point(px(0.), px(-80.))),
                cx,
            );
            settle(window, cx);
            assert!(
                strip_tab(window, 0).left() < left - px(40.),
                "the wheel moved nothing"
            );
            assert!(window.try_find("inspector-tabs-earlier").is_some());
            strip_tab(window, 0).left()
        })
        .unwrap();
    view.update(cx, |_, cx| cx.notify());
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        close("left as it was", strip_tab(window, 0).left(), left);
    })
    .unwrap();
}
