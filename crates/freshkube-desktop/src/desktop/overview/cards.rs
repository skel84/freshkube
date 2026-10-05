//! Overview's cards, and what a click on one opens.
use super::Pilot;
use crate::presentation::overview::{Card, CardState, CardTarget};
use crate::ui::{self, dp};
use freshkube_ui::card::{self, CardHeader, Figure, StatCard};
use gpui_kit::component::{ThemeStyled, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

impl Pilot {
    pub(super) fn render_cards(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
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
            .gap(dp(8.))
            .children(
                display
                    .cards
                    .iter()
                    .map(|card| self.render_card(card, window, cx)),
            )
    }

    /// A `StatCard` that opens its target: from a click, or from Enter or
    /// Space once Tab has reached it.
    fn render_card(&self, card: &Card, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let stale = match &card.state {
            CardState::LastKnown(reason) => Some(reason.clone()),
            _ => None,
        };
        let body = match card.state {
            CardState::Waiting => v_flex()
                .id("card-skeleton")
                .test_support()
                .gap(dp(8.))
                .px(dp(14.))
                .pt(dp(4.))
                .pb(dp(10.))
                .child(ui::skeleton(relative(0.4), dp(22.)))
                .child(ui::skeleton(relative(0.7), dp(12.)))
                .into_any_element(),
            _ => card::figures(
                &[Figure {
                    name: None,
                    value: &card.figure,
                    note: None,
                    tone: Some(card.tone),
                    gauge: card.meter.map(|(percent, _)| (percent / 100.) as f32),
                    segments: (!card.segments.is_empty()).then_some(&card.segments[..]),
                    spark: None,
                    detail: Some(&card.detail),
                    tinted: true,
                }],
                cx,
            ),
        };
        // The id the startup checks align the cards' contents by.
        let body = div()
            .id("tile-card-content")
            .test_support()
            .size_full()
            .child(body);
        let target = card.target.clone();
        let focus = window
            .use_keyed_state(
                SharedString::from(format!("{}-focus", card.id)),
                cx,
                // A tracked handle carries its own tab stop.
                |_, cx| cx.focus_handle().tab_stop(true),
            )
            .read(cx)
            .clone();
        let focused = focus.is_focused(window);
        StatCard::new(CardHeader::new(card.id, card.label).stale(stale))
            .render(body, cx)
            .role(Role::Button)
            .aria_label(card.label)
            .track_focus(&focus)
            .cursor_pointer()
            .hover(|this| this.bg(crate::palette::palette(cx).hover))
            // A click opens the card without leaving a focus ring on it.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_click(
                cx.listener(move |this, _, window, cx| this.open_card(target.clone(), window, cx)),
            )
            .when(focused, |this| this.focus_ring_style(window, cx))
            .into_any_element()
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
