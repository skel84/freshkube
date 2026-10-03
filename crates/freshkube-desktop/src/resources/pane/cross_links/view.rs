use super::*;
use crate::{
    palette::palette,
    ui::{self, MONO_FONT, dp},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};

impl DetailPane {
    pub(in crate::resources::pane) fn owner_button(
        &self,
        owner: &OwnerLink,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let intent = owner.intent.clone();
        Button::new(owner.id.clone())
            .ghost()
            .small()
            .child(ui::reference(owner.label.clone(), &palette(cx)))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(DetailEvent::Link(intent.clone()))))
    }
    pub(in crate::resources::pane) fn pod_sections(&self, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let links = &self.cross_links;
        if !links.pod {
            return div().into_any_element();
        }
        let section = |label: &str| {
            v_flex()
                .min_w_0()
                .p(dp(14.))
                .gap(dp(10.))
                .rounded(px(12.))
                .bg(p.surface)
                .border_1()
                .border_color(p.line)
                .child(
                    div()
                        .text_size(dp(14.))
                        .font_weight(ui::HEADING_WEIGHT)
                        .child(label.to_owned()),
                )
        };
        let runs_on = section("Runs on")
            .when_some(links.node.as_ref(), |this, node| {
                let name = node.name.clone();
                let service_node = node.name.clone();
                this.child(
                    h_flex()
                        .gap(dp(8.))
                        .flex_wrap()
                        .child(
                            Button::new("pod-runs-on")
                                .ghost()
                                .small()
                                .child(ui::reference(node.name.clone(), &p))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(DetailEvent::Link(ResourceLink::Node(
                                        name.clone(),
                                        NodeTab::Overview,
                                    )))
                                })),
                        )
                        .child(ui::tag(node.tone, None, node.ready.clone(), cx))
                        .children(node.kubelet.as_ref().map(|(tone, text)| {
                            div().id("pod-node-kubelet").test_support().child(ui::tag(
                                *tone,
                                None,
                                text.clone(),
                                cx,
                            ))
                        })),
                )
                .when_some(node.memory, |this, memory| this.child(
                    h_flex().id("pod-node-memory").gap(dp(8.)).text_size(dp(12.)).text_color(p.ink_2)
                        .child("Memory").child(crate::meters::bullet(crate::meters::Resource::Memory,
                            memory.used as f64, None, Some(memory.total as f64), node.memory_stale, &p))
                        .child(node.memory_label.clone())
                        .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Node memory used / physical capacity · requests not available in this snapshot").build(window, cx))))
                .children(node.problems.iter().map(|problem| {
                    div()
                        .text_size(dp(12.))
                        .text_color(p.warn_ink)
                        .child(problem.clone())
                }))
                .when(node.services, |this| {
                    this.child(
                        Button::new("pod-node-services")
                            .ghost()
                            .small()
                            .label("System services on this node")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(DetailEvent::Link(ResourceLink::Node(
                                    service_node.clone(),
                                    NodeTab::Services,
                                )))
                            })),
                    )
                })
            })
            .when(links.node.is_none(), |this| {
                this.child(div().text_color(p.muted).child("Not scheduled on a node"))
            });
        let relation = |label: &str| {
            h_flex().items_start().gap(dp(10.)).child(
                div()
                    .w(dp(104.))
                    .flex_none()
                    .pt(dp(4.))
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child(label.to_owned()),
            )
        };
        let controlled_by = v_flex()
            .flex_1()
            .min_w_0()
            .children(
                links
                    .owners
                    .iter()
                    .map(|owner| self.owner_button(owner, cx)),
            )
            .when(links.owners.is_empty(), |this| {
                this.child(div().pt(dp(4.)).text_color(p.muted).child("No controller"))
            });
        let selected_by = v_flex()
            .flex_1()
            .min_w_0()
            .children(links.services.iter().map(|service| {
                let open = service.object.clone();
                let forward = service.object.clone();
                h_flex()
                    .id(service.id.clone())
                    .test_support()
                    .gap(dp(8.))
                    .flex_wrap()
                    .child(
                        Button::new("pod-service-open")
                            .ghost()
                            .small()
                            .child(ui::reference(service.label.clone(), &p))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(DetailEvent::Link(ResourceLink::Object(
                                    freshkube_core::resources::builtin("services").unwrap(),
                                    open.clone(),
                                    Tab::Overview,
                                )))
                            })),
                    )
                    .child(
                        Button::new("pod-service-forward")
                            .outline()
                            .small()
                            .label("Forward")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(DetailEvent::Link(ResourceLink::Object(
                                    freshkube_core::resources::builtin("services").unwrap(),
                                    forward.clone(),
                                    Tab::Ports,
                                )))
                            })),
                    )
            }))
            .when(!links.services_note.is_empty(), |this| {
                this.child(
                    div()
                        .pt(dp(4.))
                        .text_color(p.muted)
                        .child(links.services_note.clone()),
                )
            });
        let relations = section("Relations")
            .id("pod-relations")
            .test_support()
            .child(relation("Controlled by").child(controlled_by))
            .child(relation("Selected by").child(selected_by))
            .children(links.facts.iter().map(|(label, value)| {
                relation(label).child(
                    div()
                        .pt(dp(4.))
                        .min_w_0()
                        .font_family(MONO_FONT)
                        .text_size(dp(12.5))
                        .child(value.clone()),
                )
            }));
        let logs = links.logs.clone();
        let primary = h_flex().gap(dp(8.)).child(
            Button::new("pod-open-logs")
                .primary()
                .small()
                .icon(IconName::ScrollText)
                .label(match &logs {
                    Some(name) if links.containers.len() > 1 => format!("Logs of {name}"),
                    _ => "Logs".to_owned(),
                })
                .on_click(cx.listener(move |pane, _, _, cx| match &logs {
                    Some(name) => pane.container_logs(name.clone(), false, cx),
                    None => pane.set_tab(Tab::Logs, cx),
                })),
        );
        let containers = section("Containers").children(links.containers.iter().map(|container| {
            let name = container.name.clone();
            let previous_name = container.name.clone();
            v_flex()
                .id(container.id.clone())
                .test_support()
                .gap(dp(5.))
                .pb(dp(7.))
                .border_b_1()
                .border_color(p.line)
                .child(div().font_family(MONO_FONT).child(container.name.clone()))
                .child(
                    div()
                        .font_family(MONO_FONT)
                        .text_size(dp(11.))
                        .text_color(p.muted)
                        .child(container.image.clone()),
                )
                .child(
                    h_flex()
                        .gap(dp(8.))
                        .flex_wrap()
                        .child(container.state.clone())
                        .child(container.restarts.clone())
                        .child(
                            Button::new("pod-container-logs")
                                .small()
                                .label("Logs")
                                .disabled(!container.started)
                                .on_click(cx.listener(move |pane, _, _, cx| {
                                    pane.container_logs(name.clone(), false, cx)
                                })),
                        )
                        .when(container.previous, |this| {
                            this.child(
                                Button::new("pod-container-previous")
                                    .small()
                                    .label("Previous instance")
                                    .on_click(cx.listener(move |pane, _, _, cx| {
                                        pane.container_logs(previous_name.clone(), true, cx)
                                    })),
                            )
                        }),
                )
        }));
        let containers = containers.when(!links.containers_note.is_empty(), |this| {
            this.child(
                div()
                    .text_color(p.muted)
                    .child(links.containers_note.clone()),
            )
        });
        let recent = section("Recent events")
            .children(links.recent.iter().map(|event| {
                v_flex()
                    .id(event.id.clone())
                    .gap(dp(4.))
                    .text_size(dp(12.))
                    .child(ui::tag(
                        crate::ui::Tone::Warn,
                        None,
                        event.reason.clone(),
                        cx,
                    ))
                    .child(event.message.clone())
            }))
            .when(links.recent.is_empty(), |this| {
                this.child(div().text_color(p.muted).child("No recent warning events"))
            })
            .child(
                Button::new("pod-all-events")
                    .ghost()
                    .small()
                    .label("All events")
                    .on_click(cx.listener(|pane, _, _, cx| pane.set_tab(Tab::Events, cx))),
            );
        v_flex()
            .id("pod-cross-links")
            .test_support()
            .min_w_0()
            .gap(dp(18.))
            .child(primary)
            .children(links.cause.as_ref().map(|card| self.cause_card(card, cx)))
            .when(self.history.read(cx).shows(), |this| {
                this.child(self.history.clone())
            })
            .children(
                links
                    .timeline
                    .as_ref()
                    .map(|timeline| self.timeline(timeline, cx)),
            )
            .child(containers)
            .child(runs_on)
            .child(relations)
            .child(recent)
            .children(
                links
                    .errors
                    .iter()
                    .map(|error| ui::warning_banner(None, error.clone(), None, cx)),
            )
            .when(!links.errors.is_empty(), |this| {
                this.child(
                    Button::new("pod-links-retry")
                        .small()
                        .label("Retry relationships")
                        .on_click(cx.listener(|pane, _, _, cx| pane.retry_links(cx))),
                )
            })
            .into_any_element()
    }
}
