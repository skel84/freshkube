//! Cluster cards and shared attention subjects.
use super::{PAGE_PADDING, Pilot, clock};
use crate::palette::palette;
use crate::presentation::{attention::Destination, overview::CardTarget};
use crate::ui::{self, MONO_FONT, Tone, dp};
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
        let cards =
            div()
                .id("overview-cards")
                .test_support()
                .grid()
                .grid_cols(if Self::content_width(window) < 900. {
                    2
                } else {
                    4
                })
                .gap(dp(12.))
                .children(display.cards.iter().map(|card| {
                    let target = card.target.clone();
                    let content =
                        v_flex()
                            .size_full()
                            .whitespace_normal()
                            .min_w_0()
                            .p(dp(14.))
                            .gap(dp(8.))
                            .id("tile-card-content")
                            .test_support()
                            .child(ui::caption(card.label, cx))
                            .child(
                                div()
                                    .font_weight(ui::TITLE_WEIGHT)
                                    .text_size(dp(25.))
                                    .text_color(match card.tone {
                                        Tone::Good => p.good_ink,
                                        Tone::Crit => p.crit_ink,
                                        Tone::Warn => p.warn_ink,
                                        _ => p.muted,
                                    })
                                    .child(card.figure.clone()),
                            )
                            .child(h_flex().gap(dp(3.)).flex_wrap().children(
                                card.segments.iter().map(|tone| {
                                    div().w(dp(16.)).h(dp(6.)).rounded(px(2.)).bg(match tone {
                                        Tone::Good => p.good,
                                        Tone::Crit => p.crit,
                                        Tone::Warn => p.warn,
                                        _ => p.unk,
                                    })
                                }),
                            ))
                            .when_some(card.meter, |this, (percent, level)| {
                                this.child(ui::meter(percent, level, cx))
                            })
                            .child(
                                div()
                                    .text_size(dp(12.))
                                    .text_color(p.muted)
                                    .child(card.detail.clone()),
                            );
                    Button::new(card.id)
                        .accessibility_label(card.label)
                        .outline()
                        .border_1()
                        .border_color(p.line)
                        .rounded(px(10.))
                        .bg(p.surface)
                        .items_start()
                        .h_auto()
                        .w_full()
                        .min_w_0()
                        .p_0()
                        .child(content)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_card(target.clone(), window, cx)
                        }))
                }));
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

    fn open_card(&mut self, target: CardTarget, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            CardTarget::Page(page) => {
                self.navigate_from_keyboard(page, window, cx);
                if page == super::Page::Health
                    && let Some(screen) = self.active_screen()
                    && let Ok(health) = screen.view().downcast::<crate::screens::WorkloadsScreen>()
                {
                    health.update(cx, |health, cx| health.set_only_unhealthy(true, cx));
                }
            }
            CardTarget::Kind(key, filter) => {
                self.open_builtin(key, window, cx);
                self.resources.update(cx, |resources, cx| {
                    resources.set_filter(&filter, window, cx)
                });
            }
            CardTarget::Destination(target) => self.open_destination(target, window, cx),
            CardTarget::Services => {
                self.navigate_from_keyboard(super::Page::SystemServices, window, cx);
                self.system_services
                    .update(cx, |services, cx| services.show_unhealthy(window, cx));
            }
        }
    }

    pub(in crate::desktop) fn open_destination(
        &mut self,
        target: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            Destination::Page(page) => self.navigate_from_keyboard(page, window, cx),
            Destination::Node(key, tab) => {
                self.open_node(key, window, cx);
                self.show_node_tab(tab, window, cx);
            }
            Destination::Object(key, object, tab) => {
                if let Some(kind) = freshkube_core::resources::builtin(key) {
                    self.open_object(kind, object, tab, window, cx);
                }
            }
            Destination::Service {
                node,
                service,
                logs,
            } => {
                self.open_node_by_name(
                    &node,
                    if logs {
                        super::nodes::NodeTab::Logs
                    } else {
                        super::nodes::NodeTab::Services
                    },
                    window,
                    cx,
                );
                self.selected_service = Some(service.clone());
                if logs {
                    self.logs
                        .update(cx, |view, cx| view.open_service(service, window, cx));
                }
            }
        }
    }

    pub(in crate::desktop) fn render_attention(
        &self,
        node: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let rows = node
            .and_then(|node| self.attention.by_node.get(node))
            .map(Vec::as_slice)
            .unwrap_or(if node.is_some() {
                &[]
            } else {
                &self.attention.rows
            });
        let count = if self.attention_expanded || node.is_some() {
            rows.len()
        } else {
            rows.len().min(8)
        };
        v_flex()
            .id("needs-attention")
            .test_support()
            .gap(dp(10.))
            .child(
                div()
                    .font_weight(ui::HEADING_WEIGHT)
                    .text_size(dp(20.))
                    .child("Needs attention"),
            )
            .when(rows.is_empty(), |this| {
                this.child(div().text_color(p.muted).child("No problems reported"))
            })
            .children(rows[..count].iter().map(|row| {
                let open = row.open.clone();
                let logs = row.logs.clone();
                let node = row.open_node.clone();
                v_flex()
                    .id(row.id.clone())
                    .test_support()
                    .min_w_0()
                    .p(dp(10.))
                    .gap(dp(6.))
                    .border_b_1()
                    .border_color(p.line)
                    .child(
                        h_flex()
                            .gap(dp(8.))
                            .flex_wrap()
                            .child(ui::tag(row.tone, None, row.kind, cx))
                            .child(div().font_family(MONO_FONT).child(row.name.clone())),
                    )
                    .child(
                        div()
                            .text_color(p.muted)
                            .text_size(dp(12.))
                            .child(row.reason.clone()),
                    )
                    .child(
                        h_flex()
                            .gap(dp(6.))
                            .child(
                                Button::new("attention-open")
                                    .small()
                                    .label("Open")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_destination(open.clone(), window, cx)
                                    })),
                            )
                            .when_some(logs, |this, logs| {
                                this.child(
                                    Button::new("attention-logs")
                                        .small()
                                        .label("Logs")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_destination(logs.clone(), window, cx)
                                        })),
                                )
                            })
                            .when_some(node, |this, node| {
                                this.child(
                                    Button::new("attention-open-node")
                                        .small()
                                        .label("Open node")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_destination(node.clone(), window, cx)
                                        })),
                                )
                            }),
                    )
            }))
            .when(
                node.is_none() && !self.attention_expanded && self.attention.total > 8,
                |this| {
                    this.child(
                        Button::new("attention-show-all")
                            .small()
                            .label(self.attention.more.clone())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.attention_expanded = true;
                                cx.notify();
                            })),
                    )
                },
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
