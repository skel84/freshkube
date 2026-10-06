//! The dock: a full-width strip of tabs under the page, for sessions that
//! outlive what the page shows, such as logs ([docs/DESIGN.md](../../docs/DESIGN.md#frame)).
//! Its top edge resizes it; its bar holds the tabs and, at the right, its
//! chrome. The caller owns the tabs, their content and the height; this
//! module only draws them.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{HandleEdge, ObservedElement as Observed, resize_handle};
use gpui_kit::component::resizable::resize_handle_appearance;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Axis, Context, Div, DragMoveEvent, ElementId, IntoElement, Render, Role,
    SharedString, Stateful, TestSupportExt, Window, div,
};

use crate::inspector::{TAB_HEIGHT, TabStrip, bare_strip};
use crate::palette::palette;
use crate::ui::{dp, dp_px};

/// The height a dock opens at, in dp.
pub const DEFAULT_HEIGHT: f32 = 300.;
/// The least height an open dock keeps, in dp; dragging below it
/// minimizes the dock.
pub const MIN_HEIGHT: f32 = 100.;
/// The bar's height: one row of tabs.
pub const BAR_HEIGHT: f32 = TAB_HEIGHT;

/// What a drag of the dock's top edge carries.
#[derive(Clone)]
pub struct DockDrag;

impl Render for DockDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// One of the dock's tabs: an inspector tab with a close button after its
/// label. The caller adds the click that selects it and the close button's
/// action, which `close` names.
pub fn tab(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    close: Button,
    cx: &App,
) -> Observed<Stateful<Div>> {
    crate::inspector::tab(id, label, active, cx)
        .pr(dp(4.))
        .child(close)
}

/// A tab's close button: a small ghost ×.
pub fn close_button(id: impl Into<ElementId>, label: &str) -> Button {
    Button::new(id.into())
        .ghost()
        .xsmall()
        .icon(Icon::new(IconName::X).size(dp(12.)))
        .tooltip(SharedString::from(format!("Close {label}")))
}

/// A button of the dock's chrome, at the bar's right.
pub fn chrome_button(
    id: impl Into<ElementId>,
    icon: IconName,
    tooltip: impl Into<SharedString>,
) -> Button {
    Button::new(id.into())
        .ghost()
        .xsmall()
        .icon(Icon::new(icon).size(dp(14.)))
        .tooltip(tooltip.into())
}

/// Asked with the height, in dp, a drag of the dock's top edge wants.
pub type OnResize = Rc<dyn Fn(f32, &mut Window, &mut App)>;

/// Where the dock stands, as [`frame`] draws it.
pub struct Frame {
    pub id: SharedString,
    /// The open dock's height in dp, bar included; `None` draws the bar
    /// alone, as a minimized dock.
    pub height: Option<f32>,
    /// The tabs' row, from [`TabStrip::row`].
    pub tabs: AnyElement,
    pub chrome: Vec<AnyElement>,
    /// The selected tab's content, when the dock is open.
    pub body: Option<AnyElement>,
    /// Called with the height, in dp, a drag of the top edge asks for; it
    /// may be below [`MIN_HEIGHT`].
    pub on_resize: OnResize,
}

/// The dock: its top edge, its bar of tabs and chrome, and the selected
/// tab's content filling the rest. A hairline above it parts it from the
/// page.
pub fn frame(frame: Frame, strip: &TabStrip, cx: &App) -> Observed<Stateful<Div>> {
    let p = palette(cx);
    let part = |part: &str| SharedString::from(format!("{}-{part}", frame.id));
    let open = frame.height.is_some();
    let on_resize = frame.on_resize.clone();
    v_flex()
        .id(frame.id.clone())
        .test_support()
        .role(Role::Region)
        .aria_label("Dock")
        .relative()
        .flex_none()
        .w_full()
        .min_w_0()
        .map(|this| match frame.height {
            Some(height) => this.h(dp(height)),
            // The bar alone, at its own height.
            None => this,
        })
        .bg(cx.theme().background)
        .border_t_1()
        .border_color(p.line)
        .on_drag_move::<DockDrag>(move |event: &DragMoveEvent<DockDrag>, window, cx| {
            let bottom = event.bounds.bottom();
            let height = (bottom - event.event.position.y) / dp_px(1., window);
            on_resize(height, window, cx);
        })
        .child(
            h_flex()
                .id(part("bar"))
                .test_support()
                .flex_none()
                .min_w_0()
                .h(dp(BAR_HEIGHT))
                .when(open, |this| this.border_b_1().border_color(p.line))
                .child(
                    bare_strip(part("tabs"), strip, frame.tabs, cx)
                        .flex_1()
                        .min_w_0(),
                )
                .child(
                    h_flex()
                        .id(part("chrome"))
                        .test_support()
                        .flex_none()
                        .gap(dp(2.))
                        .px(dp(6.))
                        .children(frame.chrome),
                ),
        )
        .children(frame.body.filter(|_| open).map(|body| {
            div()
                .id(part("body"))
                .test_support()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(body)
        }))
        .child(
            resize_handle::<DockDrag, DockDrag>(part("edge"), Axis::Vertical)
                .inside(HandleEdge::Leading)
                .with_appearance(resize_handle_appearance())
                .on_drag(DockDrag, |drag, _, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| (*drag).clone())
                }),
        )
}
