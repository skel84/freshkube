//! A viewport-bounded dialog below the app's title bar, with a fixed title
//! and actions around a body that scrolls independently.
use gpui_kit::component::{
    dialog::{Dialog, DialogTitle},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{IntoElement, TestSupportExt, Window, div, px};

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
                .child(
                    div()
                        .id("app-dialog-body")
                        .test_support()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .restrict_scroll_to_axis()
                        .child(body),
                )
                .child(div().flex_none().child(actions)),
        )
}
