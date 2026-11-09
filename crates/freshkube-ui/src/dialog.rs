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
