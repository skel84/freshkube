//! The Shell tab's frame: the container picker and Start or End, where the
//! session stands, a failure with Retry, and the terminal, which keeps the
//! last session's screen.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    menu::{DropdownMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{ShellState, ShellView};
use crate::logs::role_heading;
use crate::palette::palette;
use crate::ui::{self, dp};

impl ShellView {
    /// The container picker, then Start or End.
    fn render_controls(&self, cx: &mut Context<Self>) -> Div {
        let view = cx.entity().downgrade();
        let choices = self.choices.clone();
        let current = self.container.clone().unwrap_or_default();
        let running = self.running();
        let container = Button::new("pod-shell-container")
            .outline()
            .small()
            .dropdown_caret(true)
            .label(if current.is_empty() {
                SharedString::from("No containers")
            } else {
                SharedString::from(current.clone())
            })
            .accessibility_label("Container")
            .tooltip(if running {
                "End the shell to choose another container"
            } else {
                "The container the shell runs in; only a running one can take a shell"
            })
            .disabled(choices.is_empty() || running)
            .dropdown_menu(move |mut menu, _, _| {
                let mut role = None;
                for choice in choices.iter() {
                    if role != Some(choice.role) {
                        if role.is_some() {
                            menu = menu.separator();
                        }
                        menu = menu.label(role_heading(choice.role));
                        role = Some(choice.role);
                    }
                    let (view, name) = (view.clone(), choice.name.clone());
                    menu = menu.item(
                        PopupMenuItem::new(choice.label.clone())
                            .checked(choice.name == current)
                            .disabled(!choice.enabled)
                            .on_click(move |_, _, cx| {
                                let name = name.clone();
                                let _ = view.update(cx, |view, cx| view.choose_container(name, cx));
                            }),
                    );
                }
                menu
            });
        let action = if running {
            Button::new("pod-shell-end")
                .outline()
                .small()
                .icon(IconName::Square)
                .label("End")
                .tooltip("Send Control-C, then Control-D, and close the connection")
                .on_click(cx.listener(|view, _, _, cx| view.end(cx)))
        } else {
            Button::new("pod-shell-start")
                .primary()
                .small()
                .icon(IconName::SquareTerminal)
                .label("Start")
                .tooltip("Run a shell in the container. Everything typed reaches it.")
                .disabled(!self.can_start())
                .on_click(cx.listener(|view, _, window, cx| view.start(window, cx)))
        };
        h_flex()
            .gap_2()
            .child(container)
            .child(div().flex_1())
            .child(action)
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
                    "Start runs a shell in the chosen container. It changes nothing by itself, but whatever you type there runs in the pod.",
                    None,
                    Vec::new(),
                    cx,
                )),
        )
    }
}

impl Render for ShellView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        v_flex()
            .id("pod-shell")
            .test_support()
            .size_full()
            .min_h_0()
            .child(
                v_flex()
                    .gap_2()
                    .px(dp(freshkube_ui::page::PANE_PADDING))
                    .pt_2p5()
                    .pb_2()
                    .child(self.render_controls(cx))
                    .child(self.render_status(cx))
                    .children(self.render_failure(cx)),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(p.line)
                    .child(self.terminal.clone())
                    .children(self.render_idle(cx)),
            )
    }
}
