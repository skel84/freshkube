//! A shell tab drawn: where the session stands, then Start or End, a
//! failure with Retry, and the terminal, which keeps the last session's
//! screen.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{ShellState, ShellView};
use crate::palette::palette;
use crate::ui::{self, dp, dp_px};

/// The controls' row and three of the terminal's lines, in dp.
const LEAST_HEIGHT: f32 = 30. + 3. * 18. + 8.;

impl ShellView {
    /// Start, Start again or End.
    fn render_action(&self, cx: &mut Context<Self>) -> Button {
        if self.running() {
            return Button::new("pod-shell-end")
                .outline()
                .small()
                .icon(IconName::Square)
                .label("End")
                .tooltip("Send Control-C, then Control-D, and close the connection")
                .on_click(cx.listener(|view, _, _, cx| view.end(cx)));
        }
        let again = self.used;
        Button::new("pod-shell-start")
            .primary()
            .small()
            .icon(IconName::SquareTerminal)
            .label(if again { "Start again" } else { "Start" })
            .tooltip("Run a shell in the container. Everything typed reaches it.")
            .disabled(!self.can_start())
            .on_click(cx.listener(|view, _, window, cx| view.start(window, cx)))
    }

    /// Where the session stands: a tag, and a sentence when there is more
    /// to say. A failure says it in the banner below instead.
    fn render_status(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let status = &self.status;
        let failed = self.state == ShellState::Failed;
        h_flex()
            .id("pod-shell-status")
            .test_support()
            .role(Role::Status)
            .aria_label(status.label.clone())
            .flex_1()
            .min_w_0()
            .gap_2()
            .min_h(dp(22.))
            .text_size(dp(12.))
            .text_color(palette(cx).muted)
            .when(!status.tag.is_empty(), |this| {
                this.child(ui::tag(status.tone, None, status.tag.clone(), cx))
            })
            .when(!failed, |this| {
                this.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(status.text.clone()),
                )
            })
    }

    fn render_failure(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if self.state != ShellState::Failed {
            return None;
        }
        let p = palette(cx);
        Some(
            h_flex()
                .id("pod-shell-failed")
                .test_support()
                .role(Role::Alert)
                .aria_label(self.status.label.clone())
                .items_start()
                .gap_2p5()
                .px_3()
                .py_2p5()
                .rounded(px(8.))
                .border_1()
                .border_color(p.crit.opacity(0.45))
                .bg(p.crit_soft)
                .text_size(dp(12.5))
                .text_color(p.ink)
                .child(
                    Icon::new(IconName::CircleX)
                        .size(dp(16.))
                        .text_color(p.crit_ink)
                        .mt(dp(1.)),
                )
                .child(div().flex_1().min_w_0().child(self.status.text.clone()))
                .child(
                    Button::new("pod-shell-retry")
                        .outline()
                        .small()
                        .icon(IconName::RefreshCw)
                        .label("Retry")
                        .disabled(!self.can_start())
                        .on_click(cx.listener(|view, _, window, cx| view.start(window, cx))),
                ),
        )
    }

    /// Before the first session the terminal is covered by what Start does;
    /// it still lays out, so the first session starts at its size.
    fn render_idle(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.state != ShellState::Idle {
            return None;
        }
        let p = palette(cx);
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .bg(p.surface)
                .child(ui::empty_state(
                    IconName::SquareTerminal,
                    "No shell running",
                    "Start runs a shell in the container. It changes nothing by itself, but whatever you type there runs in the pod.",
                    None,
                    Vec::new(),
                    cx,
                )),
        )
    }
}

impl ShellView {
    /// Keeps the least height for the text size drawn; a change asks for
    /// the next frame, as the log view's does, so the dock that sizes the
    /// tab by it hears of it without a notify from render.
    fn note_least(&mut self, least: Pixels, window: &mut Window, cx: &mut Context<Self>) {
        if self.least == least {
            return;
        }
        self.least = least;
        let view = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            _ = view.update(cx, |_, cx| cx.notify());
        });
    }
}

impl Render for ShellView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.note_least(dp_px(LEAST_HEIGHT, window), window, cx);
        let p = palette(cx);
        v_flex()
            .id("pod-shell")
            .test_support()
            .size_full()
            .min_h_0()
            .child(
                v_flex()
                    .gap(dp(6.))
                    .pb(dp(6.))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(self.render_status(cx))
                            .child(self.render_action(cx)),
                    )
                    .children(self.render_failure(cx)),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .border_1()
                    .rounded(px(8.))
                    .overflow_hidden()
                    .border_color(p.line)
                    .child(self.terminal.clone())
                    .children(self.render_idle(cx)),
            )
    }
}
