//! Cluster cards and shared attention subjects.
use super::{PAGE_PADDING, Pilot, clock};
use crate::palette::palette;
use crate::ui::{self, Tone, dp};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

mod attention;
mod cards;

impl Pilot {
    pub(super) fn page_scroll(&self, id: &'static str) -> Stateful<Div> {
        div().id(id).size_full().overflow_y_scroll()
    }

    pub(super) fn page_body(&self) -> Div {
        v_flex()
            .px(dp(PAGE_PADDING))
            .pt(dp(22.))
            .pb(dp(30.))
            .gap(dp(20.))
    }

    /// The talosconfig couldn't be read; shown on every screen.
    pub(super) fn config_error_state(&self, error: String, cx: &mut Context<Self>) -> AnyElement {
        ui::empty_state(
            IconName::FolderOpen,
            "Freshkube can't read your talosconfig",
            "Choose a talosconfig file, or set the TALOSCONFIG environment variable and retry.",
            Some(error),
            vec![
                Button::new("browse-config-empty")
                    .primary()
                    .icon(IconName::FolderOpen)
                    .label("Browse…")
                    .on_click(cx.listener(|view, _, window, cx| view.browse_config(window, cx)))
                    .into_any_element(),
                Button::new("retry-config")
                    .outline()
                    .label("Retry")
                    .on_click(
                        cx.listener(|view, _, window, cx| view.load_configuration(window, cx)),
                    )
                    .into_any_element(),
            ],
            cx,
        )
        .into_any_element()
    }

    /// Unreachable cluster with no earlier snapshot to fall back to.
    pub(super) fn unreachable_state(&self, error: String, cx: &mut Context<Self>) -> AnyElement {
        let context = self.applied.context.clone().unwrap_or_default();
        ui::empty_state(
            IconName::Unplug,
            format!("Can't reach {context}"),
            "Check the selected talosconfig, context and network access to the Talos API (port 50000), then retry.",
            Some(error),
            vec![
                Button::new("retry")
                    .primary()
                    .icon(IconName::RefreshCw)
                    .label("Retry")
                    .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx)))
                    .into_any_element(),
                Button::new("browse-config-unreachable")
                    .outline()
                    .icon(IconName::FolderOpen)
                    .label("Choose talosconfig…")
                    .on_click(cx.listener(|view, _, window, cx| view.browse_config(window, cx)))
                    .into_any_element(),
            ],
            cx,
        )
        .into_any_element()
    }

    pub(super) fn stale_banner(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.overview.is_stale() {
            return None;
        }
        let failed = self
            .overview
            .last_failure()
            .map(clock)
            .unwrap_or_else(|| "the last attempt".into());
        let kept = self
            .overview
            .last_successful()
            .map(clock)
            .unwrap_or_else(|| "earlier".into());
        let reason = self
            .overview
            .error()
            .unwrap_or("The refresh failed.")
            .to_owned();
        Some(ui::warning_banner(
            Some(format!("Refresh failed at {failed}.").into()),
            format!("You're looking at the snapshot from {kept}. {reason}"),
            Some(
                Button::new("retry")
                    .small()
                    .icon(IconName::RefreshCw)
                    .label("Retry")
                    .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx)))
                    .into_any_element(),
            ),
            cx,
        ))
    }

    pub(super) fn render_overview(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(error) = self.config_error.clone() {
            return self.config_error_state(error, cx);
        }
        if self.kubernetes_only.is_none()
            && self.overview.data().is_none()
            && let Some(error) = self
                .overview
                .error()
                .filter(|_| !self.overview.is_loading())
        {
            return self.unreachable_state(error.to_owned(), cx);
        }
        let p = palette(cx);
        let context = self.applied.context.clone().unwrap_or_default();
        let display = &self.overview_display;
        let tip = display.roster_tip.clone();
        let drift_tip = display.drift_tip.clone();
        let header = v_flex()
            .gap(dp(7.))
            .child(
                h_flex()
                    .gap(dp(10.))
                    .flex_wrap()
                    .child(ui::page_title(context))
                    .child(ui::tag(
                        if self.overview.is_stale() || self.kubernetes_summary.is_stale() {
                            Tone::Warn
                        } else {
                            Tone::Good
                        },
                        None,
                        if self.overview.is_stale() || self.kubernetes_summary.is_stale() {
                            "Stale snapshot"
                        } else {
                            "Connected"
                        },
                        cx,
                    ))
                    .when_some(display.drift.clone(), |this, drift| {
                        this.child(
                            div()
                                .id("version-drift")
                                .tooltip(move |window, cx| {
                                    Tooltip::new(drift_tip.clone()).build(window, cx)
                                })
                                .child(ui::tag(Tone::Warn, None, drift, cx)),
                        )
                    }),
            )
            .child(div().text_color(p.muted).child(display.subtitle.clone()))
            .child(
                div()
                    .id("roster")
                    .test_support()
                    .aria_label(tip.clone())
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .child(ui::tag(Tone::Outline, None, display.roster.clone(), cx)),
            );
        let cards = self.render_cards(window, cx);
        let warnings = display
            .warnings
            .iter()
            .map(|warning| ui::warning_banner(None, warning.clone(), None, cx))
            .collect::<Vec<_>>();
        let attention = self.render_attention(None, cx);
        let body = self
            .page_body()
            .children(self.stale_banner(cx))
            .child(header)
            .children(warnings)
            .child(cards)
            .child(attention);
        self.page_scroll("overview-page")
            .test_support()
            .child(body)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
