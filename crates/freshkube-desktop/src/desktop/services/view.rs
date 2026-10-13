//! The tab's drawing: its states, toolbar, list and the selected service's
//! details.
use super::super::{HealthFilter, NextService, Page, Pilot, PreviousService, clock};
use super::restart::RestartNotices;
use crate::mutation::Operations;
use crate::palette::palette;
use crate::presentation::{self, Health};
use crate::ui::{self, MONO_FONT, Tone, dp};
use freshkube_core::HasHealth;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, ElementExt, Icon, Selectable, Sizable,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex,
    input::Input,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

const LIST_WIDTH: f32 = 330.;

impl Pilot {
    pub(in crate::desktop) fn render_services(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let node = match self.services_state(cx) {
            Ok(node) => node,
            Err(state) => {
                // Nothing to reveal; a later list mustn't jump for it.
                self.reveal_service = false;
                return state;
            }
        };
        let toolbar = self.services_toolbar(cx);
        let list = self.services_list(cx);
        let detail = self.service_detail(&node, cx);
        // In the node inspector, by its own width.
        let wide = crate::screens::embedded_width(window) >= 760.;
        // Stacked, the details sit under the list: a new selection brings
        // them into view, or nothing would show that it changed.
        let reveal = std::mem::take(&mut self.reveal_service) && !wide;
        let detail = if reveal {
            self.revealing(detail, cx)
        } else {
            detail
        };
        let split = if wide {
            h_flex()
                .items_start()
                .gap(dp(14.))
                .child(div().w(dp(LIST_WIDTH)).flex_none().child(list))
                .child(div().flex_1().min_w_0().child(detail))
                .into_any_element()
        } else {
            v_flex()
                .gap(dp(14.))
                .child(list)
                .child(detail)
                .into_any_element()
        };
        // The inspector's tab names the screen and its heading the node, so
        // the body starts with its banners and toolbar, in the inspector's
        // inset.
        let body = v_flex()
            .gap(dp(12.))
            .children(super::super::states::stale_banner(
                super::super::states::StaleTalos::of(&self.overview).as_ref(),
                &self.on_config_action(cx),
                cx,
            ))
            .children(
                self.services
                    .error()
                    .filter(|_| !self.overview.is_stale())
                    .map(|error| {
                        ui::warning_banner(
                            Some("Services didn't refresh.".into()),
                            error.to_owned(),
                            None,
                            cx,
                        )
                    }),
            )
            .child(toolbar)
            .child(split);
        super::super::states::page_scroll("services-page")
            .track_scroll(&self.service_scroll)
            .test_support()
            .child(body)
            .into_any_element()
    }

    /// Wraps the stacked details so that, once laid out, the page scrolls
    /// the least that shows them whole, or their top when they are taller
    /// than the page.
    fn revealing(&self, detail: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let scroll = self.service_scroll.clone();
        let view = cx.entity().downgrade();
        div()
            .on_prepaint(move |details, window, _| {
                let shown = scroll.bounds();
                let by = if details.size.height > shown.size.height || details.top() < shown.top() {
                    details.top() - shown.top()
                } else {
                    (details.bottom() - shown.bottom()).max(px(0.))
                };
                if by == px(0.) {
                    return;
                }
                let offset = scroll.offset();
                let y = (offset.y - by).clamp(-scroll.max_offset().y, px(0.));
                scroll.set_offset(point(offset.x, y));
                let view = view.clone();
                window.on_next_frame(move |_, cx| {
                    let _ = view.update(cx, |_, cx| cx.notify());
                });
            })
            .child(detail)
            .into_any_element()
    }

    /// The node whose services show, or what shows instead: a config
    /// error, an unreachable cluster, the skeleton while loading, no node
    /// or a node that isn't responding.
    fn services_state(
        &self,
        cx: &mut Context<Self>,
    ) -> Result<presentation::NodeSummary, AnyElement> {
        if let Some(error) = self.config_error.clone() {
            return Err(super::super::states::config_error_state(
                error,
                self.kubernetes_only.is_some(),
                &self.on_config_action(cx),
                cx,
            ));
        }
        if self.overview.data().is_none() {
            if let Some(error) = self
                .overview
                .error()
                .filter(|_| !self.overview.is_loading())
            {
                return Err(super::super::states::unreachable_state(
                    self.applied.context.as_deref().unwrap_or_default(),
                    error.to_owned(),
                    &self.on_config_action(cx),
                    cx,
                ));
            }
            return Err(self.services_skeleton(cx));
        }
        let Some(node) = self.selected_summary().cloned() else {
            return Err(ui::empty_state(
                IconName::Server,
                "No node selected",
                "Open a node in Nodes to see its services.",
                None,
                Vec::new(),
                cx,
            )
            .into_any_element());
        };
        if !node.responding && self.services.data().is_none() && !self.services.is_loading() {
            return Err(ui::empty_state(
                IconName::Unplug,
                format!("{} isn't responding", node.name),
                format!(
                    "Services need the Talos API at {}:50000. Open another node in Nodes, or wait for the next refresh.",
                    node.address
                ),
                None,
                vec![
                    Button::new("back-to-overview")
                        .primary()
                        .icon(IconName::LayoutDashboard)
                        .label("Back to Overview")
                        .on_click(cx.listener(|view, _, window, cx| view.navigate(Page::Overview, window, cx)))
                        .into_any_element(),
                ],
                cx,
            )
            .into_any_element());
        }
        Ok(node)
    }

    fn services_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let filter = self.health_filter;
        h_flex()
            .items_center()
            .gap_2p5()
            .flex_wrap()
            .child(
                div().w(dp(240.)).child(
                    Input::new(&self.service_filter)
                        .id("service-filter")
                        .aria_label("Filter services by name")
                        .small()
                        .prefix(Icon::new(IconName::Search).size(dp(14.))),
                ),
            )
            .child(
                ButtonGroup::new("health-filter")
                    .outline()
                    .small()
                    .child(
                        Button::new("health-all")
                            .label(self.service_display.labels[0].clone())
                            .selected(filter == HealthFilter::All),
                    )
                    .child(
                        Button::new("health-unhealthy")
                            .label(self.service_display.labels[1].clone())
                            .selected(filter == HealthFilter::Unhealthy),
                    )
                    .child(
                        Button::new("health-unknown")
                            .label(self.service_display.labels[2].clone())
                            .selected(filter == HealthFilter::Unknown),
                    )
                    .on_click(cx.listener(|view, selected: &Vec<usize>, _, cx| {
                        view.health_filter = match selected.first() {
                            Some(1) => HealthFilter::Unhealthy,
                            Some(2) => HealthFilter::Unknown,
                            _ => HealthFilter::All,
                        };
                        view.rebuild_service_rows(cx);
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                Button::new("refresh-services")
                    .ghost()
                    .small()
                    .size(dp(ui::CONTROL_HEIGHT))
                    .icon(ui::refresh_icon(self.services.is_loading(), cx))
                    .accessibility_label("Refresh services")
                    .tooltip(if self.services.is_loading() {
                        "Refreshing…"
                    } else {
                        "Refresh services"
                    })
                    .disabled(self.services.is_loading() || self.selected_node.is_none())
                    .on_click(cx.listener(|view, _, window, cx| view.refresh_services(window, cx))),
            )
    }

    fn services_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let visible = &self.service_display.visible;
        let reported = self.services.data().is_some_and(|all| !all.is_empty());
        v_flex()
            .id("services-region")
            .test_support()
            .aria_label("Services on the target node; use Up and Down to select")
            .key_context("TalosServices")
            .track_focus(&self.service_focus)
            .on_action(cx.listener(|view, _: &PreviousService, _, cx| view.step_service(-1, cx)))
            .on_action(cx.listener(|view, _: &NextService, _, cx| view.step_service(1, cx)))
            .p_1()
            .gap(dp(1.))
            .rounded(px(10.))
            .border_1()
            .border_color(p.line)
            .bg(p.surface)
            .when(visible.is_empty(), |this| {
                this.child(div().px_2p5().py_3p5().text_color(p.muted).child(
                    if self.services.is_loading() {
                        "Loading services…"
                    } else if !reported {
                        "This node didn't report any services."
                    } else {
                        "No services match this filter."
                    },
                ))
            })
            .children(visible.iter().map(|service| {
                let health = presentation::service_health(service);
                let selected = self.selected_service.as_ref() == Some(&service.id);
                let id = service.id.clone();
                h_flex()
                    .id(SharedString::from(service.id.clone()))
                    .test_support()
                    .role(Role::ListBoxOption)
                    .aria_selected(selected)
                    .aria_label(format!(
                        "{} · {} · {}",
                        service.id,
                        service.state,
                        presentation::health_text(&health)
                    ))
                    .min_h(dp(32.))
                    .px_2p5()
                    .py_1()
                    .gap_2p5()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .when(selected, |this| this.bg(p.accent_soft))
                    .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
                    .child(ui::health_mark(
                        SharedString::from(format!("service-health-{}", service.id)),
                        health,
                        cx,
                    ))
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(dp(12.5))
                            .when(selected, |this| {
                                this.text_color(p.accent).font_weight(FontWeight::SEMIBOLD)
                            })
                            .child(service.id.clone()),
                    )
                    .child(div().flex_1())
                    .map(|this| match health {
                        Health::Unhealthy => this.child(ui::tag(Tone::Crit, None, "Unhealthy", cx)),
                        Health::Unknown => this.child(
                            div()
                                .text_size(dp(12.))
                                .text_color(p.muted)
                                .child("Not reported"),
                        ),
                        Health::Healthy => this,
                    })
                    .child(
                        div()
                            .min_w(dp(52.))
                            .text_size(dp(12.))
                            .text_color(p.muted)
                            .text_right()
                            .child(service.state.clone()),
                    )
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.selected_service = Some(id.clone());
                        view.reveal_service = true;
                        window.focus(&view.service_focus, cx);
                        cx.notify();
                    }))
            }))
            .into_any_element()
    }

    fn service_detail(
        &self,
        node: &presentation::NodeSummary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let frame = v_flex()
            .id("selected-service")
            .test_support()
            .gap(dp(18.))
            .px(dp(20.))
            .py(dp(18.))
            .rounded(px(10.))
            .border_1()
            .border_color(p.line)
            .bg(p.surface)
            .min_w_0();
        let Some(service) = self
            .services
            .data()
            .and_then(|services| {
                presentation::selected_service(services, self.selected_service.as_deref())
            })
            .cloned()
        else {
            return frame
                .aria_label("No service selected")
                .child(
                    div()
                        .text_size(dp(12.5))
                        .text_color(p.muted)
                        .child("Select a service with the arrow keys or a click to see its full health message."),
                )
                .into_any_element();
        };
        let health = presentation::service_health(&service);
        let tone = ui::health_tone(health.health()).0;
        let message = service
            .health
            .as_ref()
            .map(|health| health.last_message.clone())
            .filter(|message| !message.is_empty())
            .unwrap_or_else(|| "No health message available".into());
        let snapshot = self
            .services
            .last_successful()
            .map(clock)
            .unwrap_or_else(|| "not yet refreshed".into());
        let row = |label: &'static str, value: AnyElement| {
            h_flex()
                .items_start()
                .gap_4()
                .child(
                    div()
                        .w(dp(118.))
                        .flex_none()
                        .pt(dp(1.))
                        .text_size(dp(12.))
                        .text_color(p.muted)
                        .child(label),
                )
                .child(div().flex_1().min_w_0().text_size(dp(13.)).child(value))
        };
        let copy = message.clone();
        let restart_notice = cx
            .try_global::<RestartNotices>()
            .and_then(|notices| notices.0.clone())
            .filter(|notice| {
                notice.node == node.name
                    && notice.service == service.id
                    && self
                        .overview
                        .data()
                        .is_some_and(|cluster| cluster.name == notice.context)
            });
        let busy = Operations::current(cx);
        let restart_blocked = if !node.responding {
            Some("The node isn't answering the Talos API".to_owned())
        } else if self.services.is_loading() || self.services.is_stale() {
            Some("Wait for a fresh service list before restarting".to_owned())
        } else {
            busy.as_ref()
                .map(|running| format!("{} is still running", running.label))
        };
        let restart_label = format!("Restart {}", service.id);
        frame
            .when(service.id == "kubelet", |this| {
                this.child(
                    Button::new("kubelet-node-pods")
                        .link()
                        .small()
                        .label(
                            self.node_workspace
                                .row()
                                .map(|row| row.kubelet_pods.clone())
                                .unwrap_or("Pods on this node".into()),
                        )
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.show_node_tab(super::super::nodes::NodeTab::Pods, window, cx)
                        })),
                )
            })
            .aria_label(format!(
                "Selected service: {} · {} · {} · {}",
                service.id,
                service.state,
                presentation::health_text(&health),
                message
            ))
            .child(
                h_flex()
                    .gap_2p5()
                    .flex_wrap()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(dp(19.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(service.id.clone()),
                    )
                    .child(ui::tag(tone, None, presentation::health_text(&health), cx)),
            )
            .child(
                v_flex()
                    .gap(dp(11.))
                    .child(row(
                        "State",
                        div().child(service.state.clone()).into_any_element(),
                    ))
                    .child(row(
                        "Health",
                        div()
                            .child(match health {
                                Health::Unknown => "This service doesn't report a health check.",
                                other => presentation::health_text(&other),
                            })
                            .into_any_element(),
                    ))
                    .child(row(
                        if health == Health::Unknown {
                            "Last event"
                        } else {
                            "Health message"
                        },
                        div()
                            .px_2p5()
                            .py_2()
                            .rounded(px(8.))
                            .bg(p.surface_2)
                            .font_family(MONO_FONT)
                            .text_size(dp(12.))
                            .line_height(dp(19.))
                            .child(message)
                            .into_any_element(),
                    ))
                    .child(row(
                        "Node",
                        h_flex()
                            .gap_1p5()
                            .flex_wrap()
                            .child(
                                div()
                                    .font_family(MONO_FONT)
                                    .text_size(dp(12.))
                                    .child(node.name.clone()),
                            )
                            .child("·")
                            .child(
                                div()
                                    .font_family(MONO_FONT)
                                    .text_size(dp(12.))
                                    .child(node.address.clone()),
                            )
                            .into_any_element(),
                    ))
                    .child(row(
                        "Snapshot",
                        h_flex()
                            .gap_2()
                            .child(snapshot)
                            .when(self.services.is_stale(), |this| {
                                this.child(ui::tag(Tone::Warn, None, "Stale", cx))
                            })
                            .into_any_element(),
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("service-logs")
                            .primary()
                            .small()
                            .icon(IconName::ScrollText)
                            .label(format!("Open {} logs", service.id))
                            .on_click(
                                cx.listener(|view, _, window, cx| view.open_logs(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("service-restart")
                            .outline()
                            .small()
                            .icon(IconName::RotateCw)
                            .label(restart_label)
                            .disabled(restart_blocked.is_some())
                            .when_some(restart_blocked, |this, reason| this.tooltip(reason))
                            .on_click(
                                cx.listener(|view, _, window, cx| view.confirm_restart(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("copy-health-message")
                            .outline()
                            .small()
                            .icon(IconName::Copy)
                            .label("Copy message")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                            }),
                    ),
            )
            .children(restart_notice.map(|notice| {
                let (tone, lead) = if notice.ok {
                    (Tone::Good, "Restart")
                } else {
                    (Tone::Crit, "Restart failed")
                };
                h_flex()
                    .id("service-restart-result")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(format!("{lead}: {}", notice.text))
                    .items_start()
                    .gap_2()
                    .child(ui::tag(tone, None, lead, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(dp(12.5))
                            .child(notice.text),
                    )
            }))
            .into_any_element()
    }

    fn services_skeleton(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let body = v_flex()
            .gap(dp(12.))
            .child(ui::skeleton(dp(240.), dp(ui::CONTROL_HEIGHT)))
            .child(
                h_flex()
                    .items_start()
                    .gap(dp(14.))
                    .child(
                        v_flex()
                            .w(dp(LIST_WIDTH))
                            .p_2()
                            .gap_3()
                            .rounded(px(10.))
                            .border_1()
                            .border_color(p.line)
                            .bg(p.surface)
                            .children((0..9).map(|_| ui::skeleton(relative(0.6), dp(12.)))),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .p_4()
                            .gap_3()
                            .rounded(px(10.))
                            .border_1()
                            .border_color(p.line)
                            .bg(p.surface)
                            .child(ui::skeleton(dp(120.), dp(19.)))
                            .child(ui::skeleton(relative(1.), dp(90.))),
                    ),
            );
        super::super::states::page_scroll("services-page")
            .track_scroll(&self.service_scroll)
            .test_support()
            .child(body)
            .into_any_element()
    }
}
