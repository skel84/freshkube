//! The inspector's tabs: a row under its heading, each 28 high.
use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::h_flex;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Div, ElementId, FontWeight, Role, SharedString, Stateful, TestSupportExt,
    transparent_black,
};

use crate::palette::palette;
use crate::ui::dp;

/// A tab's height, and the space at its sides.
pub const TAB_HEIGHT: f32 = 28.;
const TAB_PADDING: f32 = 10.;

/// One of an inspector's tabs: 28 high, its label 12.5, and a 2 px accent
/// underline with semibold ink when `active`, muted ink otherwise. The
/// caller adds what it does: focus, a tooltip, a click, a count or mark.
pub fn tab(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    cx: &App,
) -> Observed<Stateful<Div>> {
    let p = palette(cx);
    let label = label.into();
    h_flex()
        .id(id.into())
        .test_support()
        .role(Role::Tab)
        .aria_selected(active)
        .aria_label(label.clone())
        .focus_visible(|style| style.bg(p.hover))
        .flex_none()
        .h(dp(TAB_HEIGHT))
        .px(dp(TAB_PADDING))
        .gap(dp(6.))
        .cursor_pointer()
        .text_size(dp(12.5))
        .border_b_2()
        .map(|this| {
            if active {
                this.border_color(p.accent)
                    .text_color(p.ink)
                    .font_weight(FontWeight::SEMIBOLD)
            } else {
                this.border_color(transparent_black())
                    .text_color(p.muted)
                    .hover(|style| style.text_color(p.ink))
            }
        })
        .child(label)
}
