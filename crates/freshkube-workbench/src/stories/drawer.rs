//! The drawer over a list of invented objects: a row opens it and swaps
//! what it shows, a click beside the rows closes it, and its left edge
//! resizes it within its bounds. Under 600 dp of page it takes the whole
//! page. The story owns the selection, the width and whether it is open,
//! as the Resources page does; `freshkube_ui::drawer` only draws it.

use std::cell::Cell;
use std::rc::Rc;

use freshkube_ui::drawer::{self, Frame};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::table::ROW_HEIGHT;
use freshkube_ui::ui::{self, dp, dp_px};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AnyView, App, Context, SharedString, TestSupportExt, Window, canvas, div,
};

/// The id prefix of everything the story draws.
const PREFIX: &str = "drawer";
/// The invented objects the list shows.
const OBJECTS: [&str; 8] = [
    "api-7f9c6d5b8-x2kqp",
    "api-7f9c6d5b8-m4ttz",
    "worker-5c8d7-hq4tn",
    "postgres-0",
    "postgres-1",
    "ingress-nginx-controller-6b9f-lm2vw",
    "nightly-report-28870",
    "redis-0",
];

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(DrawerStory::new).into()
}

pub struct DrawerStory {
    /// The selected row; the drawer shows it while `open`.
    selected: Option<usize>,
    open: bool,
    width: f32,
    /// The list's width in dp as last drawn, which bounds the drawer.
    room: Rc<Cell<Option<f32>>>,
}

impl DrawerStory {
    fn new(_: &mut Context<Self>) -> Self {
        Self {
            selected: None,
            open: false,
            width: drawer::WIDTH,
            room: Rc::default(),
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selected = Some(ix);
        self.open = true;
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        cx.notify();
    }

    /// The list's width in dp, or the window's before it is drawn.
    fn room(&self, window: &Window) -> f32 {
        self.room
            .get()
            .unwrap_or_else(|| window.viewport_size().width / dp_px(1., window))
    }

    fn fit(&self, window: &Window) -> drawer::Fit {
        drawer::fit(self.width, self.room(window))
    }

    /// A drag of the left edge, within the drawer's bounds.
    pub fn resize(&mut self, width: f32, window: &Window, cx: &mut Context<Self>) {
        let fit = drawer::fit(width, self.room(window));
        if fit.full || fit.width == self.width {
            return;
        }
        self.width = fit.width;
        cx.notify();
    }

    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let fit = self.fit(window);
        let meta = if !self.open {
            "Closed · pick a row".to_string()
        } else if fit.full {
            "Takes the whole page under 600 dp".to_string()
        } else {
            format!("{} dp · drag the left edge", fit.width.round())
        };
        PageHeader::new(PREFIX, "Drawer")
            .meta([div().child(meta).into_any_element()])
            .render(window, cx)
            .into_any_element()
    }

    fn render_rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.selected;
        let hover = cx.theme().list_hover;
        let active = cx.theme().list_active;
        v_flex()
            .id(SharedString::from(format!("{PREFIX}-rows")))
            .children(OBJECTS.iter().enumerate().map(|(ix, name)| {
                div()
                    .id(SharedString::from(format!("{PREFIX}-row-{ix}")))
                    .test_support()
                    .h(dp(ROW_HEIGHT))
                    .px(dp(16.))
                    .flex()
                    .items_center()
                    .when(selected == Some(ix), |row| row.bg(active))
                    .hover(move |row| row.bg(hover))
                    .child(*name)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.select(ix, cx);
                    }))
            }))
            .into_any_element()
    }

    /// Learns the list's width as it lays out, and draws again when it
    /// changed.
    fn measure(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let room = self.room.clone();
        let this = cx.entity().downgrade();
        canvas(
            move |bounds, window, _| {
                let width = bounds.size.width / dp_px(1., window);
                if room.get().is_some_and(|last| (last - width).abs() < 0.5) {
                    return;
                }
                room.set(Some(width));
                let this = this.clone();
                window.on_next_frame(move |_, cx| {
                    _ = this.update(cx, |_, cx| cx.notify());
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    fn render_drawer(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ix = self.selected.filter(|_| self.open)?;
        let this = cx.entity().downgrade();
        let body = v_flex()
            .id(SharedString::from(format!("{PREFIX}-details")))
            .size_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .p(dp(16.))
            .gap(dp(8.))
            .child(
                div()
                    .id(SharedString::from(format!("{PREFIX}-object")))
                    .test_support()
                    .aria_label(OBJECTS[ix])
                    .text_size(dp(15.))
                    .child(OBJECTS[ix]),
            )
            .children((0..30).map(|line| {
                ui::caption(&format!("Invented detail {line} of {}", OBJECTS[ix]), cx)
            }));
        Some(
            drawer::frame(
                Frame {
                    id: SharedString::from(format!("{PREFIX}-frame")),
                    label: "Details".into(),
                    fit: self.fit(window),
                    body: body.into_any_element(),
                    on_resize: Rc::new(move |width, window, cx| {
                        _ = this.update(cx, |story, cx| story.resize(width, window, cx));
                    }),
                },
                cx,
            )
            .into_any_element(),
        )
    }
}

impl Render for DrawerStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("workbench.drawer");
        let header = self.render_header(window, cx);
        let rows = self.render_rows(cx);
        let open = self.open;
        page::page(SharedString::from(format!("{PREFIX}-page")))
            .flex_1()
            .min_h_0()
            .child(page::toolbar(cx).child(header))
            .child(
                div()
                    .id(SharedString::from(format!("{PREFIX}-body")))
                    .test_support()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id(SharedString::from(format!("{PREFIX}-list")))
                            .test_support()
                            .size_full()
                            .child(rows)
                            .when(open, |list| {
                                list.on_click(cx.listener(|this, _, _, cx| this.close(cx)))
                            }),
                    )
                    .child(self.measure(cx))
                    .children(self.render_drawer(window, cx)),
            )
    }
}
