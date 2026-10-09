use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, Bounds, Context, Render, ScrollDelta, TestAppContext, point, px, size,
};

use super::*;
use crate::split_size::{MemorySizes, SizeStore as _, set_store};

/// The page the tests' split saves under.
const PAGE: &str = "incidents";

/// A table, and an inspector with a heading and a body taller than the
/// window, in a split `beside` or stacked.
struct Page {
    split: InspectorSplit,
    beside: bool,
    open: bool,
    /// The table's own height, given to the split while rendering.
    lead: Option<f32>,
    /// The table's least width and the split's room beside it.
    keep: Option<(f32, f32)>,
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(lead) = self.lead {
            self.split.lead_start(lead, cx);
        }
        if let Some((least, room)) = self.keep {
            self.split.keep_lead(least, room);
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
/// inspector `width` dp wide as saved before; and where it saves.
fn open(
    cx: &mut TestAppContext,
    width: Option<f32>,
    beside: bool,
    open: bool,
) -> (AnyWindowHandle, Entity<Page>, Rc<MemorySizes>) {
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
) -> (AnyWindowHandle, Entity<Page>, Rc<MemorySizes>) {
    install(cx);
    let sizes = Rc::new(match width {
        Some(width) => MemorySizes::with(width_key(PAGE), width),
        None => MemorySizes::default(),
    });
    cx.update(|cx| set_store(sizes.clone(), cx));
    let mut page = None;
    let handle = cx.open_window(size(px(1200.), px(tall)), |window, cx| {
        let view = cx.new(|cx| Page {
            split: InspectorSplit::new(PAGE, cx).stacked(heights),
            beside,
            open,
            lead: None,
            keep: None,
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (handle.into(), page.unwrap(), sizes)
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

#[gpui_kit::test]
fn a_remembered_width_is_kept_above_the_least(cx: &mut TestAppContext) {
    let start = |width: f32, cx: &mut TestAppContext| {
        cx.update(|cx| {
            set_store(Rc::new(MemorySizes::with(width_key(PAGE), width)), cx);
            InspectorSplit::new(PAGE, cx).width()
        })
    };
    assert_eq!(start(600., cx), 600.);
    assert_eq!(start(100., cx), MIN_WIDTH);
    assert_eq!(start(f32::NAN, cx), WIDTH);
    assert_eq!(start(f32::INFINITY, cx), WIDTH);
    cx.update(|cx| {
        set_store(Rc::new(MemorySizes::default()), cx);
        assert_eq!(InspectorSplit::new(PAGE, cx).width(), WIDTH);
    });
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

/// `resize_panel` takes the path a drag's end takes; the width is saved
/// once, in dp, so it scales with the text size.
#[gpui_kit::test]
fn a_resize_reports_the_width_once_in_dp(cx: &mut TestAppContext) {
    let (handle, page, sizes) = open(cx, None, true, true);
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
    assert_eq!(sizes.saves(), 1);
    let saved = sizes.size(width_key(PAGE)).unwrap();
    near("saved", saved, 650. * BASE_TEXT / 20.);
    cx.update(|cx| near("kept", page.read(cx).split.width(), saved));
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
    let (handle, page, sizes) = open(cx, None, true, true);
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
    assert_eq!(sizes.saves(), 1, "a drag writes once");
    near("saved", sizes.size(width_key(PAGE)).unwrap(), WIDTH + 100.);
    cx.update(|cx| near("kept", page.read(cx).split.width(), WIDTH + 100.));
}

/// A table that keeps its width leaves the inspector the rest, however
/// wide the user left it before, so it never pushes the table aside.
#[gpui_kit::test]
fn a_kept_table_caps_a_remembered_width(cx: &mut TestAppContext) {
    let (handle, page, sizes) = open(cx, Some(1000.), true, true);
    cx.update(|cx| page.update(cx, |page, _| page.keep = Some((800., 1200.))));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let table = window.find("table").bounds();
        let inspector = window.find("inspector").bounds();
        close("inspector", inspector.size.width, dp_px(400., window));
        close("table", table.size.width, dp_px(800., window));
        close("meeting", inspector.left(), table.right());
    })
    .unwrap();
    cx.run_until_parked();
    // Capped to the room, the width the user gave stays saved.
    cx.update(|cx| assert_eq!(page.read(cx).split.width(), 1000.));
    assert_eq!(sizes.saves(), 0);
    assert_eq!(sizes.size(width_key(PAGE)), Some(1000.));
}

/// A press on the hairline that moves no width, as a click that jitters
/// past the drag threshold does, saves nothing: Kit reports it as a resize,
/// and the width it shows is the clamp's, not the user's.
#[gpui_kit::test]
fn a_drag_that_moves_nothing_keeps_a_clamped_width(cx: &mut TestAppContext) {
    let (handle, page, sizes) = open(cx, Some(1000.), true, true);
    cx.update(|cx| page.update(cx, |page, _| page.keep = Some((800., 1200.))));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let inspector = window.find("inspector").bounds();
        let y = inspector.center().y;
        // Into the kept table, which can't give the inspector more room.
        window.drag(
            point(inspector.left(), y),
            point(inspector.left() - px(3.), y),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(sizes.saves(), 0, "nothing moved, nothing saved");
    assert_eq!(sizes.size(width_key(PAGE)), Some(1000.));
    cx.update(|cx| assert_eq!(page.read(cx).split.width(), 1000.));
}

/// A drag stops where the kept table starts, so the width saved fits the
/// room beside it.
#[gpui_kit::test]
fn a_drag_stops_at_the_kept_table(cx: &mut TestAppContext) {
    let (handle, page, sizes) = open(cx, None, true, true);
    cx.update(|cx| page.update(cx, |page, _| page.keep = Some((700., 1200.))));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let inspector = window.find("inspector").bounds();
        let y = inspector.center().y;
        window.drag(
            point(inspector.left(), y),
            point(inspector.left() - px(300.), y),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(sizes.saves(), 1);
    near("saved", sizes.size(width_key(PAGE)).unwrap(), 500.);
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

/// A short table page: a toolbar, then a table that leads with a group row
/// and has a footer, over an inspector whose first field follows its
/// heading, in a frame that scrolls.
struct ShortPage(InspectorSplit);

impl Render for ShortPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let line = |id: &'static str, height: f32| {
            div()
                .id(id)
                .test_support()
                .flex_none()
                .w_full()
                .h(dp(height))
        };
        // A bare table, with its hairlines above and below.
        let table = v_flex()
            .id("table-pane")
            .test_support()
            .size_full()
            .min_h_0()
            .border_t_1()
            .border_b_1()
            .child(line("header", crate::table::HEADER_HEIGHT))
            .child(
                v_flex()
                    .id("rows")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(line("group-row", crate::table::ROW_HEIGHT))
                    .child(line("data-row", crate::table::ROW_HEIGHT))
                    .child(line("next-row", crate::table::ROW_HEIGHT)),
            )
            .child(line("footer", crate::table::FOOTER_HEIGHT))
            .into_any_element();
        let inspector = Inspector::new("inspector")
            .heading(div().id("inspector-title").test_support().child("checkout"))
            .child(line("inspector-field", 20.))
            .child(line("inspector-tall", 2000.))
            .render(cx)
            .into_any_element();
        v_flex()
            .id("frame")
            .size_full()
            .overflow_y_scroll()
            .child(line("toolbar", 2. * crate::page::TOOLBAR_HEIGHT))
            .child(split(
                "split",
                &self.0,
                false,
                table,
                Some(inspector),
                window,
            ))
    }
}

/// At 760 by 560 with text size 20 a stacked split is at its least
/// heights: the table still shows its header, the group row that leads
/// and one whole data row above its footer, and the inspector its heading
/// and first field, while the frame scrolls for the rest.
#[gpui_kit::test]
fn a_short_stacked_split_keeps_a_whole_row_and_the_first_field(cx: &mut TestAppContext) {
    install(cx);
    cx.update(|cx| {
        set_store(Rc::new(MemorySizes::default()), cx);
        crate::text_size::set(20., cx);
    });
    let handle = cx.open_window(size(px(760.), px(560.)), |window, cx| {
        let view = cx.new(|cx| ShortPage(InspectorSplit::new(PAGE, cx)));
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        settle(window, cx);
        let bounds = |id: &'static str| window.find(id).bounds();
        let split = bounds("split");
        let pane = bounds("table-pane");
        let (rows, row, footer) = (bounds("rows"), bounds("data-row"), bounds("footer"));
        let (inspector, field) = (bounds("inspector"), bounds("inspector-field"));
        assert!(
            split.bottom() > window.viewport_size().height,
            "the frame scrolls: the split ends at {:?}",
            split.bottom()
        );
        close(
            "the table at its least",
            pane.size.height,
            dp_px(LIST_MIN_HEIGHT, window),
        );
        assert!(
            row.bottom() <= rows.bottom() + px(0.5),
            "the first data row is whole: it ends at {:?}, the rows at {:?}",
            row.bottom(),
            rows.bottom()
        );
        assert!(row.bottom() <= footer.top() + px(0.5));
        assert!(
            field.bottom() <= inspector.bottom(),
            "the first field shows: it ends at {:?}, the inspector at {:?}",
            field.bottom(),
            inspector.bottom()
        );
    })
    .unwrap();
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
    /// Wheel events that reached the page behind the inspector.
    page_wheels: usize,
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
        div()
            .size_full()
            .on_scroll_wheel(cx.listener(|this, _, _, _| this.page_wheels += 1))
            .child(
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
            page_wheels: 0,
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

/// Each chevron click shows a tab that wasn't showing: the row moves by
/// more than a tab's padding, or that end stops being cut. A tab whose
/// padding alone lies under the fade doesn't count as cut.
#[gpui_kit::test]
fn every_chevron_click_shows_another_tab(cx: &mut TestAppContext) {
    for width in (230..=420).step_by(7) {
        let (handle, _) = open_strip(cx, width as f32, 0);
        cx.update_window(handle, |_, window, cx| {
            settle(window, cx);
            let padding = dp_px(super::tabs::TAB_PADDING, window);
            for (end, ahead) in [
                ("inspector-tabs-later", -1.),
                ("inspector-tabs-earlier", 1.),
            ] {
                for _ in 0..=LABELS.len() {
                    if window.try_find(end).is_none() {
                        break;
                    }
                    let before = strip_tab(window, 0).left();
                    window.click(end, cx);
                    settle(window, cx);
                    let moved = (strip_tab(window, 0).left() - before) * ahead;
                    assert!(
                        moved > padding || window.try_find(end).is_none(),
                        "{end} at {width} moved the row {moved:?}"
                    );
                }
                assert!(
                    window.try_find(end).is_none(),
                    "{end} at {width} never ends"
                );
            }
        })
        .unwrap();
    }
}

/// A cut strip keeps the wheel that scrolls it, so the page behind doesn't
/// scroll as well; a strip with room lets it through.
#[gpui_kit::test]
fn a_cut_strip_keeps_its_wheel_from_the_page(cx: &mut TestAppContext) {
    for (width, reaches) in [(260., 0), (900., 1)] {
        let (handle, view) = open_strip(cx, width, 0);
        cx.update_window(handle, |_, window, cx| {
            settle(window, cx);
            window.scroll(
                "strip-row",
                ScrollDelta::Pixels(point(px(0.), px(-80.))),
                cx,
            );
        })
        .unwrap();
        assert_eq!(
            view.read_with(cx, |strip, _| strip.page_wheels),
            reaches,
            "at {width}"
        );
    }
}

/// › pages forward: the first tab cut at the right moves to the left, clear
/// of the ‹ fade, so every tab that fits after it comes in. ‹ mirrors it.
#[gpui_kit::test]
fn a_chevron_pages_the_strip(cx: &mut TestAppContext) {
    for width in [230., 260., 300.] {
        let (handle, _) = open_strip(cx, width, 0);
        cx.update_window(handle, |_, window, cx| {
            settle(window, cx);
            let padding = dp_px(super::tabs::TAB_PADDING, window);
            let edge = window.find("inspector-tabs-later").bounds().left();
            let next = (0..LABELS.len())
                .find(|ix| strip_tab(window, *ix).right() > edge + padding + px(0.5))
                .expect("a cut tab");
            window.click("inspector-tabs-later", cx);
            settle(window, cx);
            let earlier = window.find("inspector-tabs-earlier").bounds().right();
            if window.try_find("inspector-tabs-later").is_some() {
                close("paged forward", strip_tab(window, next).left(), earlier);
            } else {
                assert!(whole(window, LABELS.len() - 1), "at {width}");
            }

            let prev = (0..LABELS.len())
                .rev()
                .find(|ix| strip_tab(window, *ix).left() < earlier - padding - px(0.5))
                .expect("a tab cut at the left");
            window.click("inspector-tabs-earlier", cx);
            settle(window, cx);
            if let Some(later) = window.try_find("inspector-tabs-later") {
                let later = later.bounds().left();
                if window.try_find("inspector-tabs-earlier").is_some() {
                    close("paged back", strip_tab(window, prev).right(), later);
                }
            }
            assert!(clear(window, prev), "tab {prev} at {width}");
        })
        .unwrap();
    }
}
