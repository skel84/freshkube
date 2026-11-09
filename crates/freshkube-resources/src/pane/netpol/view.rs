use super::*;
use crate::palette::palette;
use crate::ui::{self, dp};
use freshkube_ui::ui::field;
use gpui_kit::component::{Sizable, button::Button, h_flex, v_flex};

impl DetailPane {
    /// The Network policy block of Overview, for a NetworkPolicy, a Pod or a
    /// workload.
    pub(in crate::pane) fn netpol_section(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let view = &self.netpol;
        let scope = view.scope?;
        let title = match scope {
            Scope::Policy => "Network policy",
            Scope::Pod | Scope::Workload => "Network policies",
        };
        let p = palette(cx);
        let direction = |line: &DirectionLine, cx: &App| {
            v_flex()
                .id(line.id.clone())
                .test_support()
                .role(Role::Status)
                .aria_label(line.aria.clone())
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(ui::tag(line.tone, None, line.direction, cx))
                        .child(div().min_w_0().child(line.verdict)),
                )
                .children(line.rules.iter().map(|rule| {
                    div()
                        .id(rule.id.clone())
                        .test_support()
                        .pl(dp(8.))
                        .text_size(dp(12.))
                        .text_color(palette(cx).muted)
                        .child(rule.text.clone())
                }))
        };
        let note = |id: &'static str, text: &'static str| {
            div()
                .id(id)
                .test_support()
                .text_size(dp(12.))
                .text_color(p.muted)
                .child(text)
        };
        let body = match &view.body {
            None if !view.loaded => div()
                .id("netpol-loading")
                .test_support()
                .child("Reading network policies…")
                .into_any_element(),
            None => div().into_any_element(),
            Some(Body::Message { tone, text }) => h_flex()
                .id("netpol-message")
                .test_support()
                .gap_2()
                .child(ui::tag(*tone, None, "Unknown", cx))
                .child(div().min_w_0().child(text.clone()))
                .into_any_element(),
            Some(Body::Policy {
                selector,
                pods,
                pods_tone,
                directions,
            }) => v_flex()
                .gap_2()
                .child(field("Selects", selector.clone(), cx))
                .child(
                    h_flex()
                        .id("netpol-pods")
                        .test_support()
                        .gap_2()
                        .child(ui::tag(*pods_tone, None, "Pods", cx))
                        .child(div().min_w_0().child(pods.clone())),
                )
                .children(directions.iter().map(|line| direction(line, cx)))
                .into_any_element(),
            Some(Body::Applying {
                directions,
                policies,
                host_network,
                ..
            }) => v_flex()
                .gap_2()
                .children(host_network.clone().map(|text| {
                    h_flex()
                        .id("netpol-host-network")
                        .test_support()
                        .gap_2()
                        .child(ui::tag(Tone::Unknown, None, "Host network", cx))
                        .child(div().min_w_0().child(text))
                }))
                .children(directions.iter().map(|line| direction(line, cx)))
                .when(policies.is_empty(), |this| {
                    this.child(note(
                        "netpol-none",
                        "No NetworkPolicy in this namespace selects these pods.",
                    ))
                })
                .children(policies.iter().map(|line| {
                    h_flex()
                        .id(line.id.clone())
                        .test_support()
                        .gap_2()
                        .flex_wrap()
                        .child(ui::tag(Tone::Outline, None, "Policy", cx))
                        .child(line.name.clone())
                        .child(
                            div()
                                .text_size(dp(12.))
                                .text_color(palette(cx).muted)
                                .child(line.text.clone()),
                        )
                }))
                .into_any_element(),
        };
        Some(
            v_flex()
                .id("netpol-section")
                .test_support()
                .gap_3()
                .child(ui::caption(title, cx))
                .child(body)
                .child(note("netpol-scope", SCOPE_NOTE))
                .when(view.has_unknown(), |this| {
                    this.child(
                        h_flex().child(
                            Button::new("netpol-retry")
                                .small()
                                .label("Retry network policies")
                                .on_click(cx.listener(|pane, _, _, cx| pane.retry_netpol(cx))),
                        ),
                    )
                })
                .into_any_element(),
        )
    }
}
