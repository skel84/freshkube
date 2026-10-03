//! Settings' Dashboards section: the folder of the user's own Grafana JSON.
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    popover::PopoverState,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, FontWeight, div};

use super::MonitoringPage;
use crate::palette::palette;
use crate::ui::{self, dp};

pub(crate) fn settings_section(
    page: &Entity<MonitoringPage>,
    popover: Entity<PopoverState>,
    cx: &App,
) -> AnyElement {
    let p = palette(cx);
    let folder = page
        .read(cx)
        .folder()
        .map(|path| path.display().to_string());
    let save_error = page.read(cx).save_error.clone();
    let (choose, forget) = (page.downgrade(), page.downgrade());
    v_flex()
        .gap_1p5()
        .child(
            div()
                .text_size(dp(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Dashboards folder"),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .id("monitoring-folder")
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(ui::MONO_FONT)
                        .text_size(dp(12.))
                        .text_color(if folder.is_some() { p.ink } else { p.muted })
                        .child(folder.clone().unwrap_or_else(|| "None".into())),
                )
                .child(
                    Button::new("monitoring-folder-choose")
                        .outline()
                        .small()
                        .icon(IconName::FolderOpen)
                        .label("Choose…")
                        .on_click(move |_, window, cx| {
                            popover.update(cx, |state, cx| state.dismiss(window, cx));
                            _ = choose.update(cx, |page, cx| page.choose_folder(window, cx));
                        }),
                )
                .child(
                    Button::new("monitoring-folder-forget")
                        .ghost()
                        .small()
                        .label("Forget")
                        .disabled(folder.is_none())
                        .on_click(move |_, _, cx| {
                            _ = forget.update(cx, |page, cx| page.set_folder(None, cx));
                        }),
                ),
        )
        .child(
            div()
                .text_size(dp(12.))
                .text_color(p.muted)
                .child("Grafana JSON exports in this folder join the built-in dashboards under Monitoring. Their colours are not used."),
        )
        .children(save_error.map(|error| {
            div()
                .text_size(dp(12.))
                .text_color(p.crit_ink)
                .child(format!("Couldn't remember this for next launch: {error}"))
        }))
        .into_any_element()
}
