//! Services screen: a filterable list for the target node plus a detail pane.
use super::{HealthFilter, NextService, Page, Pilot, PreviousService, clock};
use crate::actions;
use crate::backend;
use crate::mutation::{self, Confirmation, Operations};
use crate::palette::palette;
use crate::presentation::{self, Health};
use crate::ui::{self, MONO_FONT, Tone, dp};
use freshkube_core::diagnostic_runner::{DiagnosticFix, DiagnosticFixAction, DiagnosticTarget};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Icon, Selectable, Sizable,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex,
    input::Input,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;
use std::time::Duration;

const LIST_WIDTH: f32 = 330.;
/// A restart request is one RPC; this only bounds a node that never answers.
const RESTART_DEADLINE: Duration = Duration::from_secs(30);

/// The outcome of the latest restart, shown next to the service it concerned.
/// Kept outside `Pilot` so the page code owns it entirely.
#[derive(Clone)]
struct RestartNotice {
    context: String,
    node: String,
    service: String,
    ok: bool,
    text: String,
}

#[derive(Default)]
struct RestartNotices(Option<RestartNotice>);

impl Global for RestartNotices {}

#[derive(Default)]
pub(super) struct ServiceDisplay {
    pub(super) visible: Vec<talos_rs::ServiceInfo>,
    pub(super) labels: [SharedString; 3],
}

impl Pilot {
    pub(super) fn render_services(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(error) = self.config_error.clone() {
            return self.config_error_state(error, cx);
        }
        if self.overview.data().is_none() {
            if let Some(error) = self
                .overview
                .error()
                .filter(|_| !self.overview.is_loading())
            {
                return self.unreachable_state(error.to_owned(), cx);
            }
            return self.services_skeleton(cx);
        }
        let p = palette(cx);
        let Some(node) = self.selected_summary().cloned() else {
            return ui::empty_state(
                IconName::Server,
                "No node selected",
                "Open a node in Nodes to see its services.",
                None,
                Vec::new(),
                cx,
            )
            .into_any_element();
        };
        if !node.responding && self.services.data().is_none() && !self.services.is_loading() {
            return ui::empty_state(
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
            .into_any_element();
        }
        let all = self.services.data().cloned().unwrap_or_default();
        let visible = self.service_display.visible.clone();
        let header = h_flex()
            .items_end()
            .gap_3()
            .flex_wrap()
            .child(v_flex().gap(dp(7.)).child(ui::page_title("Services")).when(
                self.page != Page::Nodes,
                |this| {
                    this.child(
                        h_flex()
                            .gap_1p5()
                            .text_size(dp(12.5))
                            .text_color(p.muted)
                            .child("on")
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
                            ),
                    )
                },
            ))
            .child(div().flex_1())
            .child(
                Button::new("refresh-services")
                    .outline()
                    .small()
                    .icon(IconName::RefreshCw)
                    .label("Refresh services")
                    .loading(self.services.is_loading())
                    .disabled(self.services.is_loading() || self.selected_node.is_none())
                    .on_click(cx.listener(|view, _, window, cx| view.refresh_services(window, cx))),
            );
        let filter = self.health_filter;
        let toolbar = h_flex()
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
            );
        let list = v_flex()
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
                    } else if all.is_empty() {
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
                    .rounded(px(6.))
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
                        window.focus(&view.service_focus, cx);
                        cx.notify();
                    }))
            }));
        let detail = self.service_detail(&node, cx);
        let wide = Self::content_width(window) >= 760.;
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
        let body = self
            .page_body()
            .children(self.stale_banner(cx))
            .child(header)
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
        self.page_scroll("services-page")
            .child(body)
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
        let tone = ui::health_tone(health);
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
                            view.show_node_tab(super::nodes::NodeTab::Pods, window, cx)
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
                            .rounded(px(6.))
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
        let body = self
            .page_body()
            .child(
                v_flex()
                    .gap_2p5()
                    .child(ui::skeleton(dp(150.), dp(28.)))
                    .child(ui::skeleton(dp(240.), dp(13.))),
            )
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
        self.page_scroll("services-page")
            .child(body)
            .into_any_element()
    }
}

impl Pilot {
    /// The preview identity of restarting `service_id` on the target node: the
    /// context, the target epoch, the node, the service and the state the
    /// service is in now. `None` once any of them is gone.
    fn restart_fingerprint(&self, service_id: &str) -> Option<u64> {
        let cluster = self.overview.data()?;
        let node = self.selected_summary()?;
        let service = self
            .services
            .data()?
            .iter()
            .find(|service| service.id == service_id)?;
        Some(actions::fingerprint((
            &cluster.name,
            self.epoch,
            &node.name,
            &node.address,
            &service.id,
            &service.state,
        )))
    }

    /// Asks before restarting the selected service on the target node.
    fn confirm_restart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(service_id) = self.selected_service.clone() else {
            return;
        };
        let (Some(fingerprint), Some(node), Some(cluster)) = (
            self.restart_fingerprint(&service_id),
            self.selected_summary().cloned(),
            self.overview.data().map(|cluster| cluster.name.clone()),
        ) else {
            return;
        };
        let Some(state) = self
            .services
            .data()
            .and_then(|services| services.iter().find(|service| service.id == service_id))
            .map(|service| service.state.clone())
        else {
            return;
        };
        let critical = actions::is_critical_service(&service_id);
        let this = cx.weak_entity();
        let (current, run) = (this.clone(), this);
        let (current_id, run_id) = (service_id.clone(), service_id.clone());
        let mut warnings: Vec<SharedString> = actions::critical_warning(&service_id)
            .map(|warning| vec![SharedString::from(warning)])
            .unwrap_or_default();
        if self.fixture {
            warnings.push("Example data: nothing will be sent to a node.".into());
        }
        mutation::confirm(
            Confirmation {
                title: format!("Restart {service_id}?").into(),
                summary: format!(
                    "Talos stops {service_id} on {} and starts it again. It is unavailable while it restarts.",
                    node.name
                )
                .into(),
                facts: vec![
                    ("Context".into(), cluster.into()),
                    (
                        "Node".into(),
                        format!("{} · {}", node.name, node.address).into(),
                    ),
                    ("Service".into(), service_id.clone().into()),
                    ("Current state".into(), state.into()),
                ],
                warnings,
                confirm_label: format!("Restart {service_id}").into(),
                destructive: critical,
                fingerprint,
                current: Rc::new(move |cx: &App| {
                    current
                        .upgrade()
                        .and_then(|view| view.read(cx).restart_fingerprint(&current_id))
                }),
                on_confirm: Rc::new(move |window: &mut Window, cx: &mut App| {
                    if let Some(view) = run.upgrade() {
                        let service = run_id.clone();
                        view.update(cx, |view, cx| view.start_restart(service, window, cx));
                    }
                }),
            },
            window,
            cx,
        );
    }

    fn set_restart_notice(&self, service: &str, ok: bool, text: String, cx: &mut Context<Self>) {
        let (Some(cluster), Some(node)) = (self.overview.data(), self.selected_node.clone()) else {
            return;
        };
        cx.set_global(RestartNotices(Some(RestartNotice {
            context: cluster.name.clone(),
            node,
            service: service.to_owned(),
            ok,
            text,
        })));
        cx.notify();
    }

    /// Takes the operation slot and restarts `service_id` on the target node.
    /// Example data only simulates it.
    fn start_restart(&mut self, service_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(node), Some(cluster)) = (
            self.selected_summary().cloned(),
            self.overview.data().map(|cluster| cluster.name.clone()),
        ) else {
            return;
        };
        let operations = Operations::global(cx);
        let label = format!("Restarting {service_id} on {}", node.name);
        let ticket = match operations.update(cx, |operations, cx| operations.begin(label, cx)) {
            Ok(ticket) => ticket,
            Err(running) => {
                self.set_restart_notice(
                    &service_id,
                    false,
                    format!(
                        "{} is still running; try again when it has finished.",
                        running.label
                    ),
                    cx,
                );
                return;
            }
        };
        let fix = DiagnosticFix {
            description: format!("Restart {service_id}"),
            action: DiagnosticFixAction::RestartService {
                service_id: service_id.clone(),
            },
        };
        if self.fixture {
            // Offline: no client, no audit file, no network.
            self.set_restart_notice(&service_id, true, actions::simulated(&fix), cx);
            operations.update(cx, |operations, cx| operations.finish(ticket, cx));
            self.refresh_services(window, cx);
            return;
        }
        let Some(client) = self
            .overview
            .data()
            .and_then(|cluster| cluster.client.clone())
        else {
            self.set_restart_notice(
                &service_id,
                false,
                "No Talos connection is available for this context.".into(),
                cx,
            );
            operations.update(cx, |operations, cx| operations.finish(ticket, cx));
            return;
        };
        operations.update(cx, |operations, cx| {
            operations.progress(
                &ticket,
                format!("Asking {} to restart {service_id}", node.name),
                cx,
            )
        });
        let epoch = self.epoch;
        let target = DiagnosticTarget::new(&node.name, &node.address, "node", &cluster);
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            RESTART_DEADLINE,
            format!("Restarting {service_id} timed out"),
            actions::apply_fix(
                client,
                target,
                "service_restart".to_owned(),
                fix,
                ticket.cancellation(),
            ),
        );
        cx.spawn_in(window, async move |this, cx| {
            // The job lives exactly as long as this task.
            let _job = job;
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("The restart worker stopped".into()));
            // Release the slot first, whatever happened to the view.
            _ = cx.update(|_, cx| {
                operations.update(cx, |operations, cx| operations.finish(ticket, cx))
            });
            _ = this.update_in(cx, |view, window, cx| {
                let (ok, text) = match result {
                    Ok(text) => (true, format!("{service_id} on {}: {text}", node.name)),
                    Err(error) => (false, format!("{service_id} on {}: {error}", node.name)),
                };
                view.set_restart_notice(&service_id, ok, text, cx);
                if view.epoch == epoch {
                    view.refresh_services(window, cx);
                }
            });
        })
        .detach();
        cx.notify();
    }
}
