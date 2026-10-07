//! The drawer: one object's details over the right edge of a page's list,
//! as Freelens has it ([docs/DESIGN.md](../../docs/DESIGN.md#frame)). It
//! overlays its parent, which must be `relative`, from top to bottom, with
//! a shadow on its left edge, and its left edge resizes it. The caller owns
//! what it shows, its width and whether it is open; this module only lays
//! it out and draws it.

use std::rc::Rc;

use gpui_kit::base::{HandleEdge, ObservedElement as Observed, resize_handle};
use gpui_kit::component::resizable::resize_handle_appearance;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Axis, BoxShadow, Context, Div, DragMoveEvent, IntoElement, Render, Role,
    SharedString, Stateful, TestSupportExt, Window, div, point, px,
};

use crate::ui::{dp, dp_px};

/// The width a drawer opens at, in dp: about 725 px at the default text
/// size.
pub const WIDTH: f32 = 725.;
/// The least width a drawer keeps, in dp.
pub const MIN_WIDTH: f32 = 300.;
/// The width of the list a drawer leaves beside it, in dp.
pub const LIST_KEEPS: f32 = 280.;
/// The most of the page a drawer takes beside the list.
pub const MOST_SHARE: f32 = 0.9;
/// Below this page width, in dp, the drawer takes the whole page and
/// can't be resized.
pub const FULL_BELOW: f32 = 600.;

/// How a drawer lays out on a page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    /// Its width in dp.
    pub width: f32,
    /// Whether it takes the whole page, with no edge to drag.
    pub full: bool,
}

/// How a drawer the user left `width` dp wide lays out on a page `page` dp
/// wide: within [`MIN_WIDTH`] and the least of [`MOST_SHARE`] of the page
/// and the page less [`LIST_KEEPS`]; the whole page below [`FULL_BELOW`].
pub fn fit(width: f32, page: f32) -> Fit {
    if page < FULL_BELOW {
        return Fit {
            width: page.max(0.),
            full: true,
        };
    }
    let most = (page * MOST_SHARE).min(page - LIST_KEEPS).max(MIN_WIDTH);
    let width = if width.is_finite() { width } else { WIDTH };
    Fit {
        width: width.clamp(MIN_WIDTH, most),
        full: false,
    }
}

/// A remembered width, or [`WIDTH`]; never narrower than [`MIN_WIDTH`].
pub fn start_width(width: Option<f32>) -> f32 {
    match width {
        Some(width) if width.is_finite() => width.max(MIN_WIDTH),
        _ => WIDTH,
    }
}

/// What a drag of the drawer's left edge carries.
#[derive(Clone)]
pub struct DrawerDrag;

impl Render for DrawerDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Asked with the width, in dp, a drag of the drawer's left edge wants.
pub type OnResize = Rc<dyn Fn(f32, &mut Window, &mut App)>;

/// What [`frame`] draws.
pub struct Frame {
    pub id: SharedString,
    /// Its accessible name, such as "Details".
    pub label: SharedString,
    /// How it lays out, from [`fit`].
    pub fit: Fit,
    /// What it shows; it fills the drawer and scrolls itself.
    pub body: AnyElement,
    /// Called with the width, in dp, a drag of the left edge asks for,
    /// before [`fit`]; never called while the drawer takes the whole page.
    pub on_resize: OnResize,
}

/// The drawer over its parent's right edge, top to bottom. It takes the
/// pointer from what it covers, so rows under it neither hover nor click.
pub fn frame(frame: Frame, cx: &App) -> Observed<Stateful<Div>> {
    let part = |part: &str| SharedString::from(format!("{}-{part}", frame.id));
    let Fit { width, full } = frame.fit;
    let on_resize = frame.on_resize.clone();
    v_flex()
        .id(frame.id.clone())
        .test_support()
        .role(Role::Region)
        .aria_label(frame.label.clone())
        .absolute()
        .top_0()
        .bottom_0()
        .right_0()
        .w(dp(width))
        .min_h_0()
        .occlude()
        .bg(cx.theme().background)
        .when(!full, |this| {
            // The resize handle draws the edge's hairline.
            this.shadow(edge_shadow(cx))
                // Kit's handle drags its value in an `Rc`.
                .on_drag_move::<Rc<DrawerDrag>>(
                    move |event: &DragMoveEvent<Rc<DrawerDrag>>, window, cx| {
                        let right = event.bounds.right();
                        let width = (right - event.event.position.x) / dp_px(1., window);
                        on_resize(width, window, cx);
                    },
                )
        })
        .child(
            div()
                .id(part("body"))
                .test_support()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(frame.body),
        )
        .when(!full, |this| {
            this.child(
                resize_handle::<DrawerDrag, DrawerDrag>(part("edge"), Axis::Horizontal)
                    .inside(HandleEdge::Leading)
                    .with_appearance(resize_handle_appearance())
                    .on_drag(DrawerDrag, |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| (*drag).clone())
                    }),
            )
        })
}

/// Kit's middle elevation, cast to the left over the list the drawer
/// covers. None when the theme turns shadows off.
fn edge_shadow(cx: &App) -> Vec<BoxShadow> {
    let mut shadow = cx.theme().shadow_tokens().md;
    for layer in &mut shadow {
        layer.offset = point(-layer.offset.y, px(0.));
    }
    shadow
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_page_keeps_the_width_within_its_bounds() {
        // 1280×880 with the column open leaves a page about 1008 wide.
        assert_eq!(fit(WIDTH, 1008.).width, WIDTH);
        assert!(!fit(WIDTH, 1008.).full);
        assert_eq!(fit(100., 1008.).width, MIN_WIDTH);
        // The list keeps 280 beside it.
        assert_eq!(fit(2000., 1008.).width, 1008. - LIST_KEEPS);
        // On a very wide page, 90% is the most.
        assert_eq!(fit(4000., 3000.).width, 2700.);
    }

    #[test]
    fn a_narrow_page_gives_the_drawer_all_of_it() {
        assert_eq!(
            fit(WIDTH, 560.),
            Fit {
                width: 560.,
                full: true
            }
        );
        let edge = fit(WIDTH, FULL_BELOW);
        assert!(!edge.full);
        assert_eq!(edge.width, FULL_BELOW - LIST_KEEPS);
    }

    #[test]
    fn a_bad_width_starts_at_the_default() {
        assert_eq!(start_width(None), WIDTH);
        assert_eq!(start_width(Some(f32::NAN)), WIDTH);
        assert_eq!(start_width(Some(120.)), MIN_WIDTH);
        assert_eq!(fit(f32::INFINITY, 1008.).width, WIDTH);
    }
}
