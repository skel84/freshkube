//! Restarting the selected service: asked first, then run on the app-wide
//! operation ticket, detached from any view.
use super::super::Pilot;
use crate::actions;
use crate::backend;
use crate::mutation::{self, Confirmation, Operations};
use freshkube_core::diagnostic_runner::{DiagnosticFix, DiagnosticFixAction, DiagnosticTarget};
use gpui_kit::*;
use std::rc::Rc;
use std::time::Duration;

/// A restart request is one RPC; this only bounds a node that never answers.
const RESTART_DEADLINE: Duration = Duration::from_secs(30);

/// The outcome of the latest restart, shown next to the service it concerned.
/// Kept outside `Pilot` so the page code owns it entirely.
#[derive(Clone)]
pub(super) struct RestartNotice {
    pub(super) context: String,
    pub(super) node: String,
    pub(super) service: String,
    pub(super) ok: bool,
    pub(super) text: String,
}

#[derive(Default)]
pub(super) struct RestartNotices(pub(super) Option<RestartNotice>);

impl Global for RestartNotices {}

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
    pub(super) fn confirm_restart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let critical = actions::critical_warning(&service_id).is_some();
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
