//! The app's Kit dialog frame. Kit caps the surface to the viewport and
//! scrolls its body; actions belong in its fixed footer, outside that scroll.
use gpui_kit::component::dialog::Dialog;
use gpui_kit::{IntoElement, Window};

use crate::ui::dp_px;

pub fn frame(dialog: Dialog, width: f32, actions: impl IntoElement, window: &Window) -> Dialog {
    dialog
        .w(dp_px(width, window).min(window.viewport_size().width - dp_px(32., window)))
        .overlay_closable(false)
        .footer(actions)
}
