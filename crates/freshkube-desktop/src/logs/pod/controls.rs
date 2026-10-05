//! The Logs tab's own controls, above the shared filters and search: the
//! container and tail pickers, Stop and Resume, the previous instance and
//! timestamps, then where the stream stands.

use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, Context, Div, Role, SharedString, TestSupportExt,
    component::{
        Disableable, Icon, Sizable,
        button::Button,
        checkbox::Checkbox,
        h_flex,
        menu::{DropdownMenu, PopupMenuItem},
    },
    div,
    prelude::*,
    px,
};

use super::{PodLogPanel, PodLogView, StreamState, TAILS, role_heading};
use crate::palette::palette;
use crate::ui::{self, dp};

/// `Last 1,000`, or `All lines`.
fn tail_label(tail: Option<i64>) -> String {
    let Some(tail) = tail else {
        return "All lines".to_owned();
    };
    let digits = tail.to_string();
    let mut grouped = String::new();
    for (ix, digit) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("Last {grouped}")
}

/// The Logs tab's toolbar rows.
pub(super) trait Controls: Sized + 'static {
    fn render_controls(&self, cx: &mut Context<Self>) -> Vec<AnyElement>;

    /// The container and tail pickers, and Stop or Resume.
    fn render_pickers(&self, cx: &mut Context<Self>) -> Div;

    /// The previous instance and the timestamps column.
    fn render_options(&self, cx: &mut Context<Self>) -> Div;

    /// Where the stream stands: a tag, and a sentence when there is more
    /// to say. A failure says it in the banner below instead.
    fn render_status(&self, cx: &mut Context<Self>) -> AnyElement;

    /// A failure with Retry, or the hint that the container keeps crashing.
    fn render_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement>;
}

impl Controls for PodLogView {
    fn render_controls(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows = vec![
            self.render_pickers(cx).into_any_element(),
            self.render_options(cx).into_any_element(),
            self.render_status(cx).into_any_element(),
        ];
        rows.extend(self.render_banner(cx));
        rows
    }

    fn render_pickers(&self, cx: &mut Context<Self>) -> Div {
        let source = self.source();
        let view = cx.entity().downgrade();
        let choices = source.choices.clone();
        let current = source.container.clone().unwrap_or_default();
        let container = Button::new("pod-logs-container")
            .outline()
            .small()
            .dropdown_caret(true)
            .label(if current.is_empty() {
                SharedString::from("No containers")
            } else {
                SharedString::from(current.clone())
            })
            .accessibility_label("Container")
            .tooltip("The container whose log shows")
            .disabled(choices.is_empty())
            .dropdown_menu({
                let view = view.clone();
                move |mut menu, _, _| {
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
                                    let _ =
                                        view.update(cx, |view, cx| view.choose_container(name, cx));
                                }),
                        );
                    }
                    menu
                }
            });
        let tail = source.tail;
        let tails = Button::new("pod-logs-tail")
            .outline()
            .small()
            .dropdown_caret(true)
            .label(tail_label(tail))
            .accessibility_label("Lines to start with")
            .tooltip("Lines from the end to start with. Changing it reads the log again.")
            .dropdown_menu(move |mut menu, _, _| {
                for choice in TAILS {
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(tail_label(choice))
                            .checked(choice == tail)
                            .on_click(move |_, _, cx| {
                                let _ = view.update(cx, |view, cx| view.set_tail(choice, cx));
                            }),
                    );
                }
                menu
            });
        let running = source.running();
        let stream = Button::new("pod-logs-stream")
            .outline()
            .small()
            .icon(if running {
                IconName::Square
            } else {
                IconName::Play
            })
            .label(if running { "Stop" } else { "Resume" })
            .tooltip(if running {
                "Stop reading the log. What was read stays."
            } else {
                "Read on from the last line"
            })
            .disabled(source.previous || !(running || source.state == StreamState::Stopped))
            .on_click(cx.listener(|view, _, _, cx| {
                if view.source().running() {
                    view.stop(cx);
                } else {
                    view.resume(cx);
                }
            }));
        h_flex()
            .gap_2()
            .child(container)
            .child(tails)
            .child(div().flex_1())
            .child(stream)
    }

    fn render_options(&self, cx: &mut Context<Self>) -> Div {
        let has_previous = self.source().has_previous();
        h_flex()
            .gap_4()
            .child(
                Checkbox::new("pod-logs-previous")
                    .small()
                    .label("Previous instance")
                    .checked(self.source().previous)
                    .disabled(!has_previous)
                    .tooltip(if has_previous {
                        "The log of the instance before this one, read to its end"
                    } else {
                        "No previous instance"
                    })
                    .on_click(
                        cx.listener(|view, checked: &bool, _, cx| view.set_previous(*checked, cx)),
                    ),
            )
            .child(
                Checkbox::new("pod-logs-timestamps")
                    .small()
                    .label("Timestamps")
                    .checked(self.columns().time)
                    .tooltip("Show each line's time. Copy copies what shows.")
                    .on_click(
                        cx.listener(|view, checked: &bool, _, cx| {
                            view.set_timestamps(*checked, cx)
                        }),
                    ),
            )
    }

    fn render_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let status = &self.source().status;
        let failed = matches!(self.source().state, StreamState::Failed(_));
        h_flex()
            .id("pod-logs-status")
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
                this.child(div().flex_1().min_w_0().child(status.text.clone()))
            })
            .into_any_element()
    }

    fn render_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let StreamState::Failed(_) = &self.source().state {
            let p = palette(cx);
            return Some(
                h_flex()
                    .id("pod-logs-failed")
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(self.source().status.text.clone())
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
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.source().status.text.clone()),
                    )
                    .child(
                        Button::new("pod-logs-retry")
                            .outline()
                            .small()
                            .icon(IconName::RefreshCw)
                            .label("Retry")
                            .on_click(cx.listener(|view, _, _, cx| view.resume(cx))),
                    )
                    .into_any_element(),
            );
        }
        let hint = self
            .source()
            .hint
            .clone()
            .filter(|_| !self.source().previous)?;
        Some(
            div()
                .id("pod-logs-hint")
                .test_support()
                .role(Role::Status)
                .aria_label(hint.clone())
                .child(ui::warning_banner(
                    None,
                    hint,
                    Some(
                        Button::new("pod-logs-show-previous")
                            .outline()
                            .small()
                            .label("Show previous instance")
                            .on_click(cx.listener(|view, _, _, cx| view.set_previous(true, cx)))
                            .into_any_element(),
                    ),
                    cx,
                ))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod label_tests {
    use super::tail_label;

    #[test]
    fn tails_read_with_grouped_digits() {
        assert_eq!(tail_label(Some(100)), "Last 100");
        assert_eq!(tail_label(Some(1_000)), "Last 1,000");
        assert_eq!(tail_label(Some(1_234_567)), "Last 1,234,567");
        assert_eq!(tail_label(None), "All lines");
    }
}
