use super::*;
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, dp};
use freshkube_ui::ui::field;
use gpui_kit::component::{Sizable, button::Button};

impl DetailPane {
    /// The Traffic block of Overview, for a Service or a workload.
    pub(in crate::pane) fn traffic_section(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let view = &self.traffic;
        let scope = view.scope?;
        let title = match scope {
            Scope::Service => "Published endpoints",
            Scope::Workload => "Traffic",
        };
        let p = palette(cx);
        let step = |step: &StepLine, cx: &App| {
            v_flex()
                .id(step.id.clone())
                .test_support()
                .role(Role::Status)
                .aria_label(format!("{} {}: {}", step.kind, step.subject, step.summary))
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(ui::tag(step.tone, None, step.kind, cx))
                        .child(
                            div()
                                .text_size(dp(12.))
                                .text_color(palette(cx).muted)
                                .child(format!("{} · {}", step.evidence, step.subject)),
                        )
                        .child(div().min_w_0().child(step.summary.clone())),
                )
                .children(step.detail.clone().map(|detail| {
                    div()
                        .pl(dp(8.))
                        .text_size(dp(12.))
                        .text_color(palette(cx).muted)
                        .child(detail)
                }))
        };
        let group = |group: &GroupLine, cx: &App| {
            v_flex()
                .id(group.id.clone())
                .test_support()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .child(ui::tag(group.tone, None, group.title, cx))
                        .child(format!("{}", group.count)),
                )
                .children(group.hint.map(|hint| {
                    div()
                        .id(SharedString::from(format!("{}-hint", group.id)))
                        .test_support()
                        .pl(dp(8.))
                        .text_size(dp(12.))
                        .text_color(palette(cx).muted)
                        .child(hint)
                }))
                .children(group.rows.iter().map(|(id, text)| {
                    div()
                        .id(id.clone())
                        .test_support()
                        .pl(dp(8.))
                        .font_family(MONO_FONT)
                        .text_size(dp(12.))
                        .child(text.clone())
                }))
                .when(group.more > 0, |this| {
                    this.child(
                        div()
                            .pl(dp(8.))
                            .text_size(dp(12.))
                            .text_color(palette(cx).muted)
                            .child(format!("{} more not shown", group.more)),
                    )
                })
        };
        Some(
            v_flex()
                .id("traffic-section")
                .test_support()
                .gap_3()
                .child(ui::caption(title, cx))
                .when(!view.loaded, |this| {
                    this.child(
                        div()
                            .id("traffic-loading")
                            .test_support()
                            .child("Reading Services and endpoints…"),
                    )
                })
                .children(view.steps.iter().map(|line| step(line, cx)))
                .children(
                    view.source
                        .clone()
                        .map(|source| field("Answered by", source, cx)),
                )
                .children(view.groups.iter().map(|line| group(line, cx)))
                .when(!view.pods.is_empty(), |this| {
                    this.child(ui::caption("Selected but not published", cx))
                        .children(view.pods.iter().map(|pod| {
                            h_flex()
                                .id(pod.id.clone())
                                .test_support()
                                .gap_2()
                                .flex_wrap()
                                .child(ui::tag(Tone::Warn, None, pod.reason, cx))
                                .child(div().font_family(MONO_FONT).child(pod.pod.clone()))
                                .child(
                                    div()
                                        .text_size(dp(12.))
                                        .text_color(p.muted)
                                        .child(pod.detail.clone()),
                                )
                        }))
                })
                .children(view.pods_more.clone().map(|more| {
                    div()
                        .id("traffic-pods-more")
                        .test_support()
                        .pl(dp(8.))
                        .text_size(dp(12.))
                        .text_color(p.muted)
                        .child(more)
                }))
                .children(view.pods_note.clone().map(|note| {
                    div()
                        .id("traffic-pods-unknown")
                        .test_support()
                        .child(ui::warning_banner(None, note, None, cx))
                }))
                .when(view.has_unknown(), |this| {
                    this.child(
                        h_flex().child(
                            Button::new("traffic-retry")
                                .small()
                                .label("Retry traffic")
                                .on_click(cx.listener(|pane, _, _, cx| pane.retry_traffic(cx))),
                        ),
                    )
                })
                .into_any_element(),
        )
    }
}
