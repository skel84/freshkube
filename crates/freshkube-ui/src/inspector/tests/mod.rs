use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, Bounds, Context, Render, ScrollDelta, ScrollWheelEvent, TestAppContext,
    TouchPhase, point, px, size,
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
    /// What the inspector shows; the body goes back to the top when it changes.
    selection: u32,
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
                .selection(self.selection)
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
            selection: 0,
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

/// A short table page: a toolbar, then a table that leads with a group row
/// and has a footer, over an inspector whose first field follows its
/// heading, in a frame that scrolls.
struct ShortPage {
    split: InspectorSplit,
    /// The height of the inspector's last line, in dp.
    tall: f32,
    selection: u32,
    open: bool,
}

impl ShortPage {
    fn new(split: InspectorSplit) -> Self {
        Self {
            split,
            tall: 2000.,
            selection: 0,
            open: true,
        }
    }
}

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
        let inspector = self.open.then(|| {
            Inspector::new("inspector")
                .selection(self.selection)
                .heading(div().id("inspector-title").test_support().child("checkout"))
                .child(line("inspector-field", 20.))
                .child(line("inspector-tall", self.tall))
                .render(cx)
                .into_any_element()
        });
        v_flex()
            .id("frame")
            .size_full()
            .overflow_y_scroll()
            .child(line("toolbar", 2. * crate::page::TOOLBAR_HEIGHT))
            .child(split("split", &self.split, false, table, inspector, window))
    }
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

/// The short page at 760 by 560 and text size 20, where the frame scrolls
/// around a stacked split.
fn short_page(cx: &mut TestAppContext, tall: f32) -> (AnyWindowHandle, Entity<ShortPage>) {
    install(cx);
    cx.update(|cx| {
        set_store(Rc::new(MemorySizes::default()), cx);
        crate::text_size::set(20., cx);
    });
    let mut page = None;
    let handle = cx.open_window(size(px(760.), px(560.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = ShortPage::new(InspectorSplit::new(PAGE, cx));
            view.tall = tall;
            view
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (handle.into(), page.unwrap())
}

/// Where the body is scrolled to, from the first line's place in it.
fn body_offset(window: &mut Window) -> Pixels {
    window.find("inspector-field").bounds().top() - window.find("inspector-body").bounds().top()
}

fn frame_top(window: &mut Window) -> Pixels {
    window.find("toolbar").bounds().top()
}

fn wheel(window: &mut Window, over: &'static str, delta: ScrollDelta, cx: &mut App) {
    window.scroll(over, delta, cx);
    settle(window, cx);
}

fn up(by: f32) -> ScrollDelta {
    ScrollDelta::Pixels(point(px(0.), px(-by)))
}

/// One event of a trackpad gesture over `over`, in `phase`.
fn gesture(
    window: &mut Window,
    over: &'static str,
    by: f32,
    touch_phase: TouchPhase,
    cx: &mut App,
) {
    use gpui_kit::InputEvent as _;
    window.hover(over, cx);
    window.dispatch_event(
        ScrollWheelEvent {
            position: window.find(over).bounds().center(),
            delta: up(by),
            touch_phase,
            ..Default::default()
        }
        .to_platform_input(),
        cx,
    );
    settle(window, cx);
}

mod body;
mod content;
mod layout;
mod sizing;
mod stacked;
mod strip;
