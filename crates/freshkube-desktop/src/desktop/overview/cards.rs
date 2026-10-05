//! Overview's cards, and what a click on one opens.
use super::Pilot;
use crate::palette::palette;
use crate::presentation::overview::CardTarget;
use crate::ui::{self, Tone, dp};
use gpui_kit::component::{button::Button, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

impl Pilot {
    pub(super) fn render_cards(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let display = &self.overview_display;
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
                let content = v_flex()
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
                    .child(
                        h_flex()
                            .gap(dp(3.))
                            .flex_wrap()
                            .children(card.segments.iter().map(|tone| {
                                div().w(dp(16.)).h(dp(6.)).rounded(px(2.)).bg(match tone {
                                    Tone::Good => p.good,
                                    Tone::Crit => p.crit,
                                    Tone::Warn => p.warn,
                                    _ => p.unk,
                                })
                            })),
                    )
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
            }))
    }

    fn open_card(&mut self, target: CardTarget, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            CardTarget::Page(page) => {
                self.navigate_from_keyboard(page, window, cx);
                if page == crate::desktop::Page::Health
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
                self.navigate_from_keyboard(crate::desktop::Page::SystemServices, window, cx);
                self.system_services
                    .update(cx, |services, cx| services.show_unhealthy(window, cx));
            }
        }
    }
}
