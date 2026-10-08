//! A pod log's own tools, first in the toolbar's row: the container and
//! tail pickers, Previous, Timestamps, Stop or Resume and where the stream
//! stands; under the row, one note when there is something to say.

use freshkube_core::group_digits;
use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, Context, Role, SharedString, TestSupportExt,
    component::{
        Disableable, Icon, Selectable, Sizable,
        button::Button,
        h_flex,
        menu::{DropdownMenu, PopupMenuItem},
        tooltip::Tooltip,
    },
    div,
    prelude::*,
    px,
};

use freshkube_ui::tooltip::FollowTooltip as _;

use super::{PodLogView, Stream, StreamState, TAILS, role_heading};
use crate::palette::palette;
use crate::ui::{self, dp};

/// The picker's entry that reads every app container at once.
const ALL_CONTAINERS: &str = "All containers";

/// `Last 1,000`, or `All lines`.
fn tail_label(tail: Option<i64>) -> String {
    let Some(tail) = tail else {
        return "All lines".to_owned();
    };
    format!("Last {}", group_digits(tail.unsigned_abs()))
}

/// The pod's tools in the toolbar's row, and the note under it.
pub(super) trait Controls: Sized + 'static {
    /// The container and tail pickers, Previous, Timestamps, Stop or
    /// Resume, and where the stream stands.
    fn render_tools(&self, cx: &mut Context<Self>) -> Vec<AnyElement>;

    /// A failure with Retry, or one line, cut short with a tooltip, on why
    /// the log waits and how the container last ended, when there is
    /// something to say.
    fn render_notes(&self, cx: &mut Context<Self>) -> Vec<AnyElement>;
}

impl Controls for PodLogView {
    fn render_tools(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let source = self.source();
        let view = cx.entity().downgrade();
        let choices = source.choices.clone();
        let (offers_all, all) = (source.offers_all, source.all);
        let current = source.container.clone().unwrap_or_default();
        let container = Button::new("pod-logs-container")
            .outline()
            .small()
            .dropdown_caret(true)
            .label(if all {
                SharedString::from(ALL_CONTAINERS)
            } else if current.is_empty() {
                SharedString::from("No containers")
            } else {
                SharedString::from(current.clone())
            })
            .accessibility_label("Container")
            .tooltip("The container whose log shows, or all of them")
            .disabled(choices.is_empty())
            .dropdown_menu({
                let view = view.clone();
                move |mut menu, _, _| {
                    if offers_all {
                        let view = view.clone();
                        menu = menu
                            .item(PopupMenuItem::new(ALL_CONTAINERS).checked(all).on_click(
                                move |_, _, cx| {
                                    let _ = view.update(cx, |view, cx| view.choose_all(cx));
                                },
                            ))
                            .separator();
                    }
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
                                .checked(!all && choice.name == current)
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
        let has_previous = source.has_previous();
        let previous = Button::new("pod-logs-previous")
            .outline()
            .small()
            .icon(IconName::RotateCcw)
            // Labelled even in a compact toolbar: in a crash loop it is
            // what the note points to.
            .label("Previous")
            .toggled(source.previous)
            .selected(source.previous)
            .accessibility_label("Previous instance")
            .tooltip(match (has_previous, source.previous, all) {
                (false, _, _) => "No previous instance",
                (true, false, false) => "Show the instance before this one, read to its end",
                (true, false, true) => {
                    "Show each container's instance before this one, read to its end"
                }
                (true, true, false) => "Back to the running instance",
                (true, true, true) => "Back to the running instances",
            })
            .disabled(!has_previous && !source.previous)
            .on_click(cx.listener(|view, _, _, cx| {
                let previous = !view.source().previous;
                view.set_previous(previous, cx)
            }));
        let time = self.columns().time;
        let timestamps = crate::logs::timestamps_button("pod-logs-timestamps", time)
            .on_click(cx.listener(move |view, _, _, cx| view.set_timestamps(!time, cx)));
        let running = source.running();
        let stream = Button::new("pod-logs-stream")
            .outline()
            .small()
            .icon(if running {
                IconName::Square
            } else {
                IconName::Play
            })
            .accessibility_label(if running { "Stop" } else { "Resume" })
            .tooltip(if running {
                "Stop reading the log. What was read stays."
            } else {
                "Resume: read on from the last line"
            })
            .disabled(source.previous || !(running || source.state == StreamState::Stopped))
            .on_click(cx.listener(|view, _, _, cx| {
                if view.source().running() {
                    view.stop(cx);
                } else {
                    view.resume(cx);
                }
            }));
        let status = &source.status;
        // Compact, the tag is its glyph alone, as the buttons are their
        // icons: its words stay in the tooltip and the label.
        let compact = self.compact();
        let words = status.label.clone();
        let tag = h_flex()
            .id("pod-logs-status")
            .test_support()
            .role(Role::Status)
            .aria_label(status.label.clone())
            .when(!status.tag.is_empty() && !compact, |this| {
                this.child(ui::tag(status.tone, None, status.tag.clone(), cx))
            })
            .when(!status.tag.is_empty() && compact, |this| {
                this.w(dp(20.))
                    .self_stretch()
                    .flex_none()
                    .justify_center()
                    .children(ui::status_glyph(status.tone, cx))
                    .follow_tooltip(words)
            });
        vec![
            container.into_any_element(),
            tails.into_any_element(),
            h_flex()
                .gap_1()
                .child(previous)
                .child(timestamps)
                .child(stream)
                .into_any_element(),
            tag.into_any_element(),
        ]
    }

    fn render_notes(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let source = self.source();
        if let StreamState::Failed(_) = &source.state {
            let p = palette(cx);
            return vec![
                h_flex()
                    .id("pod-logs-failed")
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(source.status.text.clone())
                    .items_start()
                    .gap_2p5()
                    .px_3()
                    .py_2()
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
                    .child(div().flex_1().min_w_0().child(source.status.text.clone()))
                    .child(
                        Button::new("pod-logs-retry")
                            .outline()
                            .small()
                            .icon(IconName::RefreshCw)
                            .label("Retry")
                            .on_click(cx.listener(|view, _, _, cx| view.resume(cx))),
                    )
                    .into_any_element(),
            ];
        }
        let Some(note) = source.note.clone() else {
            return Vec::new();
        };
        let p = palette(cx);
        vec![
            h_flex()
                .id("pod-logs-note")
                .test_support()
                .role(Role::Status)
                .aria_label(note.text.clone())
                // One line: a long note ends in an ellipsis, and its
                // tooltip says it all, wrapped to stay inside the window.
                .tooltip({
                    let text = note.text.clone();
                    move |window, cx| {
                        let text = text.clone();
                        Tooltip::element(move |_, _| div().max_w(dp(360.)).child(text.clone()))
                            .build(window, cx)
                    }
                })
                .items_center()
                .gap_2()
                .text_size(dp(12.))
                .line_height(dp(16.))
                .text_color(if note.warn { p.warn_ink } else { p.muted })
                .when(note.warn, |this| {
                    this.child(Icon::new(IconName::TriangleAlert).size(dp(14.)).flex_none())
                })
                .child(div().flex_1().min_w_0().truncate().child(note.text))
                .into_any_element(),
        ]
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
