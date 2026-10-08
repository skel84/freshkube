//! The navigation column's rows (docs/DESIGN.md, App frame), shared by the
//! app's column and the workbench's story list so they can't drift.
use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::{Icon, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, ElementId, Rems, Role, SharedString, Stateful, TestSupportExt, div, px};

use crate::palette::palette;
use crate::ui::{HEADING_WEIGHT, dp};

/// A page or kind in the column, its icon and label indented by `indent`;
/// the one shown is raised. The caller adds a tooltip, anything after the
/// label and the click.
pub fn item(
    id: impl Into<ElementId>,
    label: SharedString,
    icon: IconName,
    indent: Rems,
    active: bool,
    cx: &App,
) -> Observed<Stateful<Div>> {
    let p = palette(cx);
    h_flex()
        .id(id)
        .test_support()
        .role(Role::Tab)
        .aria_selected(active)
        .aria_label(label.clone())
        .tab_index(0)
        .h(dp(30.))
        .flex_none()
        .pl(indent)
        .pr(dp(8.))
        .gap_2()
        .rounded(px(8.))
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .cursor_pointer()
        .text_size(dp(13.))
        .text_color(if active { p.ink } else { p.ink_2 })
        .when(active, |this| {
            this.bg(p.surface_2)
                .border_color(p.line_strong)
                .font_weight(HEADING_WEIGHT)
        })
        .when(!active, |this| this.hover(|style| style.bg(p.hover)))
        .child(Icon::new(icon).size(dp(15.)).text_color(p.muted))
        .child(div().min_w_0().truncate().child(label))
}
