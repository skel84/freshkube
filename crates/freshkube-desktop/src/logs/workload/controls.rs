//! A workload's Logs tab controls, above the shared filters and search:
//! where the pod watch stands, the notices, then one row of chips, one
//! per container read, which shows or hides lines. The chips wrap to two
//! rows and scroll past them, as Talos services do.

use freshkube_ui::tooltip::FollowTooltip as _;
use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, Context, Role, SharedString, TestSupportExt, Toggled,
    component::{Icon, Sizable, button::Button, h_flex},
    div,
    prelude::*,
};

use super::{MAX_STREAMS, PodsState, Streams, WorkloadLogView, plural};
use crate::palette::palette;
use crate::ui::{self, Tone, dp};

/// The Logs tab's toolbar rows.
pub(super) trait Controls: Sized + 'static {
    fn render_controls(&self, cx: &mut Context<Self>) -> Vec<AnyElement>;

    /// Where the watch stands: a tag, and the counts.
    fn render_status(&self, cx: &mut Context<Self>) -> AnyElement;

    /// A watch failure with Retry, or the cap's notice.
    fn render_notices(&self, cx: &mut Context<Self>) -> Vec<AnyElement>;

    /// One chip per container read: its state, and whether its lines show.
    fn render_streams(&self, cx: &mut Context<Self>) -> Option<AnyElement>;
}

impl Controls for WorkloadLogView {
    fn render_controls(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows = vec![self.render_status(cx)];
        rows.extend(self.render_notices(cx));
        rows.extend(self.render_streams(cx));
        rows
    }

    fn render_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let status = &self.source().status;
        let failed = matches!(self.source().pods_state, PodsState::Failed(_));
        h_flex()
            .id("workload-logs-status")
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

    fn render_notices(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut notices = Vec::new();
        if let PodsState::Failed(_) = &self.source().pods_state {
            let text = self.source().status.text.clone();
            notices.push(
                div()
                    .id("workload-logs-failed")
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(text.clone())
                    .child(ui::banner(
                        Tone::Crit,
                        None,
                        text,
                        Some(
                            Button::new("workload-logs-retry")
                                .outline()
                                .small()
                                .icon(IconName::RefreshCw)
                                .label("Retry")
                                .on_click(cx.listener(|view, _, _, cx| view.retry(cx)))
                                .into_any_element(),
                        ),
                        cx,
                    ))
                    .into_any_element(),
            );
        }
        let left_out = self.source().left_out;
        if left_out > 0 {
            let text: SharedString = format!(
                "Reading {MAX_STREAMS} of {} containers, the newest pods first.",
                MAX_STREAMS + left_out
            )
            .into();
            notices.push(
                div()
                    .id("workload-logs-capped")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(text.clone())
                    .child(ui::warning_banner(None, text, None, cx))
                    .into_any_element(),
            );
        }
        notices
    }

    fn render_streams(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let chips = self.source().chips.clone();
        if chips.is_empty() {
            return None;
        }
        let p = palette(cx);
        Some(
            h_flex()
                .id("workload-logs-streams")
                .test_support()
                .role(Role::Group)
                .aria_label(plural(chips.len(), "container", "containers"))
                .w_full()
                .min_w_0()
                .flex_wrap()
                .max_h(dp(26. * 2. + 6.))
                .overflow_y_scroll()
                .restrict_scroll_to_axis()
                .gap(dp(6.))
                .children(chips.iter().map(|chip| {
                    let service = chip.service.clone();
                    let tooltip = chip.tooltip.clone();
                    h_flex()
                        .id(chip.id.clone())
                        .test_support()
                        .role(Role::CheckBox)
                        .aria_toggled(if chip.shown {
                            Toggled::True
                        } else {
                            Toggled::False
                        })
                        .aria_label(tooltip.clone())
                        .tab_index(0)
                        .flex_none()
                        .h(dp(26.))
                        .pl(dp(8.))
                        .pr(dp(9.))
                        .gap(dp(6.))
                        .rounded_full()
                        .border_1()
                        .border_color(p.line_strong)
                        .bg(p.surface)
                        .cursor_pointer()
                        .font_family(ui::MONO_FONT)
                        .text_size(dp(12.))
                        .text_color(if chip.shown { p.ink } else { p.muted })
                        .children(ui::status_glyph(chip.tone, cx))
                        .child(
                            div()
                                .when(!chip.shown, |this| this.line_through())
                                .child(chip.label.clone()),
                        )
                        .child(
                            Icon::new(if chip.shown {
                                IconName::Eye
                            } else {
                                IconName::EyeOff
                            })
                            .size(dp(13.))
                            .text_color(p.muted),
                        )
                        .follow_tooltip(tooltip)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.toggle_stream(service.clone(), cx)
                        }))
                }))
                .into_any_element(),
        )
    }
}
