//! A viewport-bounded dialog below the app's title bar, with a fixed title
//! and actions around a body that scrolls independently.
use gpui_kit::component::{
    dialog::{Dialog, DialogTitle},
    scroll::{Scrollbar, ScrollbarMode},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{IntoElement, ScrollHandle, TestSupportExt, Window, div, px};

use crate::{
    page::APP_HEADER_HEIGHT,
    ui::{dp, dp_px},
};

pub fn frame(
    dialog: Dialog,
    width: f32,
    title: impl IntoElement,
    body: impl IntoElement,
    actions: impl IntoElement,
    window: &Window,
) -> Dialog {
    frame_with(dialog, width, title, body, actions, None, window)
}

/// [`frame`] whose body shows its scrollbar whenever it overflows, so nobody
/// confirms without seeing that more follows. `scroll` is the body's scroll
/// handle, which the caller keeps between draws.
pub fn frame_scrolled(
    dialog: Dialog,
    width: f32,
    title: impl IntoElement,
    body: impl IntoElement,
    actions: impl IntoElement,
    scroll: &ScrollHandle,
    window: &Window,
) -> Dialog {
    frame_with(dialog, width, title, body, actions, Some(scroll), window)
}

fn frame_with(
    dialog: Dialog,
    width: f32,
    title: impl IntoElement,
    body: impl IntoElement,
    actions: impl IntoElement,
    scroll: Option<&ScrollHandle>,
    window: &Window,
) -> Dialog {
    let top = dp_px(APP_HEADER_HEIGHT + 12., window).max(window.viewport_size().height / 10.);
    // Kit adds 16 px of padding above and below its content. Bound the
    // complete contents too: Kit otherwise snaps a tall surface upwards.
    let height = (window.viewport_size().height - top - dp_px(16., window) - px(32.)).max(px(0.));
    dialog
        .w(dp_px(width, window).min(window.viewport_size().width - dp_px(32., window)))
        .margin_top(top)
        .overlay_closable(false)
        .child(
            v_flex()
                .max_h(height)
                .min_h_0()
                .gap(dp(12.))
                .child(
                    div()
                        .flex_none()
                        .pr_6()
                        .child(DialogTitle::new().child(title)),
                )
                .child(match scroll {
                    None => div()
                        .id("app-dialog-body")
                        .test_support()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .restrict_scroll_to_axis()
                        .child(body)
                        .into_any_element(),
                    Some(scroll) => v_flex()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        // Clipped, not left visible: the wrapper then shrinks
                        // to the dialog's height and the body scrolls inside.
                        .overflow_hidden()
                        .child(
                            div()
                                .id("app-dialog-body")
                                .test_support()
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .restrict_scroll_to_axis()
                                .track_scroll(scroll)
                                // Room for the scrollbar's lane.
                                .pr(dp(10.))
                                .child(body),
                        )
                        .child(
                            Scrollbar::vertical(scroll)
                                .id("app-dialog-scrollbar")
                                .mode(ScrollbarMode::Always),
                        )
                        .into_any_element(),
                })
                .child(div().flex_none().child(actions)),
        )
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::{Root, WindowExt};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext, Context, Render, ScrollDelta, ScrollHandle, TestAppContext, point, size,
    };

    use super::*;

    struct Host;

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    /// At 760x560 with text 20 a tall body stays inside the dialog, scrolls
    /// to its last row whole, and follows the handle its scrollbar uses.
    #[gpui_kit::test]
    fn a_tall_body_scrolls_whole_under_the_scrollbar_s_handle(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            crate::text_size::set(20., cx);
            cx.set_reduce_motion(true);
        });
        let scroll = ScrollHandle::new();
        let window = cx
            .open_window(size(px(760.), px(560.)), move |window, cx| {
                Root::new(cx.new(|_| Host), window, cx)
            })
            .into();
        cx.run_until_parked();
        let handle = scroll.clone();
        cx.update_window(window, |_, window, cx| {
            window.open_dialog(cx, move |dialog, window, _| {
                let rows = (0..12).map(|ix| {
                    div()
                        .id(gpui_kit::SharedString::from(format!("row-{ix}")))
                        .test_support()
                        .child(format!("Row {ix}"))
                });
                frame_scrolled(
                    dialog,
                    520.,
                    "A tall dialog",
                    v_flex().children(rows),
                    div().id("tall-actions").test_support().child("Actions"),
                    &handle,
                    window,
                )
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let body = window.find("app-dialog-body").bounds();
            let actions = window.find("tall-actions").bounds();
            assert!(actions.top() >= body.bottom(), "{actions:?} over {body:?}");
            assert!(
                scroll.max_offset().y > px(0.),
                "the body has nothing to scroll"
            );
            let first = window.find("row-0").bounds();
            assert!(first.top() >= body.top() && first.bottom() <= body.bottom());
            window.scroll(
                "app-dialog-body",
                ScrollDelta::Pixels(point(px(0.), px(-5000.))),
                cx,
            );
            window.render_frame(cx);
            let (body, last) = (
                window.find("app-dialog-body").bounds(),
                window.find("row-11").bounds(),
            );
            assert!(
                last.top() >= body.top() && last.bottom() <= body.bottom(),
                "{last:?} cut by {body:?}"
            );
            assert!(scroll.offset().y < px(0.));
        })
        .unwrap();
    }
}
