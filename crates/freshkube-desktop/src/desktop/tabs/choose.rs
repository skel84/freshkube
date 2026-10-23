//! The page a tab shows when its cluster entry is no longer in
//! `freshkube.yaml` (#878): a dashed ring and "Choose a cluster". It never
//! shows the cluster that happens to be open instead.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::prelude::*;
use gpui_kit::{Role, TestSupportExt as _};

use super::*;
use crate::ui::{self, dp};

/// The most clusters the page offers as buttons.
const OFFERED: usize = 8;

impl Pilot {
    /// The page area of a tab whose entry is gone.
    pub(in crate::desktop) fn render_choose_cluster(&self, cx: &mut Context<Self>) -> AnyElement {
        let gone = self.tabs.set.current().entry.clone().unwrap_or_default();
        let offered: Vec<String> = self
            .settings_page
            .read(cx)
            .workspace()
            .clusters
            .iter()
            .take(OFFERED)
            .map(|entry| entry.id.clone())
            .collect();
        let buttons = offered
            .into_iter()
            .map(|id| {
                let pick = id.clone();
                Button::new(format!("choose-cluster-{id}"))
                    .outline()
                    .label(id)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.choose_cluster_for_tab(pick.clone(), window, cx)
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .size_full()
            .min_h_0()
            .pt(dp(22.))
            .px(dp(PAGE_PADDING))
            .pb(dp(18.))
            .child(
                ui::empty_state(
                    IconName::CircleDashed,
                    "Choose a cluster",
                    format!(
                        "This tab belongs to {gone}, which freshkube.yaml no longer lists. \
                         Nothing is shown in its place. Pick a cluster to open the tab on, \
                         keeping its page."
                    ),
                    None,
                    buttons,
                    cx,
                )
                .id("choose-cluster")
                .test_support()
                .role(Role::Status),
            )
            .into_any_element()
    }
}
