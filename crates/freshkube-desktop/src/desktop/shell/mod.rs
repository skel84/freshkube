//! Window chrome: the header, the icon rail, the navigation column, the
//! status bar and their popovers (docs/DESIGN.md, App frame).
use super::kubernetes_only::{self, KubeConnection};
use super::{
    AUTO_REFRESH, Appearance, Area, COLUMN_WIDTH, ColumnReveal, Page, Pilot, RAIL_WIDTH, clock,
};
use crate::monitoring::page::{Entry, FolderState};
use crate::mutation::Operations;
use crate::palette::palette;
use crate::resources::custom::{CustomGroup, Discovery};
use crate::resources::navigation;
use crate::text_size;
use crate::ui::{self, MONO_FONT, Tone, dp};
use freshkube_core::resources::{Failure, FailureKind};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, TitleBar,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex,
    input::Input,
    popover::Popover,
    scroll::{Scrollbar, ScrollbarMode},
    status_bar::StatusBar,
    switch::Switch,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// Most custom API groups, and kinds per group, the column lists; a row
/// says how many more there are.
const MAX_SIDEBAR_GROUPS: usize = 300;
const MAX_SIDEBAR_KINDS: usize = 200;

impl Pilot {
    fn context_item(
        &self,
        ix: usize,
        context: &str,
        popover: WeakEntity<gpui_kit::component::popover::PopoverState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let current = self.applied.context.as_deref() == Some(context);
        let connection = self.kubernetes_only.as_ref().map(|kube| &kube.connection);
        let (tone, tip) = if !current {
            (Tone::Unknown, "Not loaded yet")
        } else if let Some(connection) = connection {
            match connection {
                KubeConnection::Connected { .. } => (Tone::Good, "Connected"),
                KubeConnection::Failed(_) => (Tone::Crit, "Couldn't connect"),
                KubeConnection::Idle | KubeConnection::Connecting => (Tone::Unknown, "Connecting"),
            }
        } else if self.overview.is_stale() {
            (Tone::Warn, "Last refresh failed")
        } else if self.overview.data().is_some() {
            (Tone::Good, "Connected")
        } else {
            (Tone::Unknown, "Connecting")
        };
        let chosen = context.to_owned();
        h_flex()
            .id(("context", ix))
            .test_support()
            .role(Role::Tab)
            .aria_selected(current)
            .aria_label(context.to_owned())
            .tab_index(0)
            .min_h(dp(32.))
            .py_1()
            .px_2()
            .gap_2p5()
            .rounded(px(8.))
            .cursor_pointer()
            .text_color(if current { p.ink } else { p.ink_2 })
            .when(current, |this| this.bg(p.accent_soft))
            .when(!current, |this| this.hover(|style| style.bg(p.hover)))
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
            })
            .children(ui::status_glyph(tone, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .whitespace_normal()
                    .font_family(MONO_FONT)
                    .text_size(dp(12.5))
                    .child(context.to_owned()),
            )
            .children(self.context_display.counts.get(context).map(|label| {
                div()
                    .text_size(dp(11.5))
                    .text_color(p.muted)
                    .child(label.clone())
            }))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.select_context(chosen.clone(), window, cx);
                view.focus_page(window, cx);
                let handle = window.window_handle();
                let popover = popover.clone();
                cx.defer(move |cx| {
                    _ = handle.update(cx, |_, window, cx| {
                        _ = popover.update(cx, |state, cx| state.dismiss(window, cx));
                    });
                });
            }))
            .into_any_element()
    }

    /// One workspace entry in the switcher: a click opens it as the active
    /// cluster.
    fn cluster_item(
        &self,
        ix: usize,
        item: &super::switch::ClusterItem,
        popover: WeakEntity<gpui_kit::component::popover::PopoverState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let current = self.active_cluster() == Some(item.id.as_ref());
        let chosen = item.id.to_string();
        h_flex()
            .id(("cluster", ix))
            .test_support()
            .role(Role::Tab)
            .aria_selected(current)
            .aria_label(item.id.clone())
            .tab_index(0)
            .min_h(dp(32.))
            .py_1()
            .px_2()
            .gap_2p5()
            .rounded(px(8.))
            .cursor_pointer()
            .text_color(if current { p.ink } else { p.ink_2 })
            .when(current, |this| this.bg(p.accent_soft))
            .when(!current, |this| this.hover(|style| style.bg(p.hover)))
            .tooltip({
                let tip = item.tooltip.clone();
                move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                }
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(MONO_FONT)
                    .text_size(dp(12.5))
                    .child(item.id.clone()),
            )
            .child(
                div()
                    .text_size(dp(11.5))
                    .text_color(p.muted)
                    .child(item.role.clone()),
            )
            .on_click(cx.listener(move |view, _, window, cx| {
                view.switch_cluster(chosen.clone(), window, cx);
                view.focus_page(window, cx);
                let handle = window.window_handle();
                let popover = popover.clone();
                cx.defer(move |cx| {
                    _ = handle.update(cx, |_, window, cx| {
                        _ = popover.update(cx, |state, cx| state.dismiss(window, cx));
                    });
                });
            }))
            .into_any_element()
    }
}

mod chrome;
pub(super) use chrome::ChromeParts;
mod column;
pub(super) use column::Room;
#[cfg(test)]
pub(super) use column::cut_edges;
mod context;
pub(super) use context::{Connection, ContextDisplay};
pub(super) mod fps;
mod frame;
use frame::settings_content;
pub(super) use frame::status_text;
mod header;
mod rail;
mod refresh_tip;
pub(super) use rail::RailMarks;

mod column_state;
pub(super) use column_state::ColumnState;
mod fog_column;
mod source;
pub(super) use source::ColumnKey;
