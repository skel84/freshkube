//! Native node operations, rolling operations, and audit-history screens.
//!
//! Every request in this module is bound to an explicit context plus Kubernetes/Talos
//! node identity. The egui thread owns only presentation state: Kubernetes, Talos,
//! etcd, and local audit-file work run on the application Tokio runtime and return
//! owned snapshots through screen-local channels.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use eframe::egui::{self, Color32, RichText};
use talos_pilot_core::{
    cluster_overview::{KubeconfigSource, create_k8s_client_with_source},
    indicators::SafetyStatus,
    inspection::{
        EtcdHealthSnapshot, EtcdInspectionRequest, InspectionTarget, collect_etcd_health,
    },
    operations::{
        AuditEntry, AuditLog, DrainOptions, EtcdQuorumImpact, NodeOperationRequest,
        NodeOperationResult, NodeTarget, OperationConfirmation, OperationContext, OperationKind,
        OperationsEvent, RollingNode, RollingOperationRequest, RollingOperationResult,
        evaluate_operation_safety, run_node_operation, run_rolling_operation,
    },
};
use tokio::{runtime::Handle, sync::mpsc, task::JoinHandle};

use crate::screens::ScreenTarget;

const HEALTHY: Color32 = Color32::from_rgb(76, 175, 80);
const WARNING: Color32 = Color32::from_rgb(255, 193, 7);
const ERROR: Color32 = Color32::from_rgb(244, 67, 54);
const UNKNOWN: Color32 = Color32::from_rgb(158, 158, 158);
const INFO: Color32 = Color32::from_rgb(100, 181, 246);

/// A Tokio task with cooperative cancellation owned by its route.
///
/// Dropping a `JoinHandle` detaches a task, so deactivation first flips this
/// flag. Core operation runners observe it between mutations and can carry out
/// their required compensating scheduling restore before they finish.
struct ActiveTask {
    cancellation: Arc<AtomicBool>,
    join: JoinHandle<()>,
}

impl ActiveTask {
    fn request_cancel(&self) {
        self.cancellation.store(true, Ordering::Release);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NodeIdentity {
    context: String,
    target: NodeTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RollingIdentity {
    context: String,
    nodes: Vec<NodeTarget>,
}

#[derive(Clone)]
struct SinglePreflight {
    kubernetes_source: String,
    etcd_summary: String,
    impact: EtcdQuorumImpact,
}

#[derive(Clone)]
struct RollingPreflight {
    kubernetes_source: String,
    etcd_summary: String,
    impacts: Vec<EtcdQuorumImpact>,
}

#[derive(Clone)]
struct ProgressSnapshot {
    scope: String,
    target: String,
    phase: String,
    step: String,
    message: String,
}

impl ProgressSnapshot {
    fn from_event(event: OperationsEvent) -> Self {
        match event {
            OperationsEvent::Operation(event) => Self {
                scope: event.operation.label().to_owned(),
                target: format!("{} ({})", event.target.name, event.target.address),
                phase: format!("{:?}", event.phase),
                step: format!("{:?}", event.step),
                message: event.message,
            },
            OperationsEvent::Rolling(event) => Self {
                scope: format!(
                    "{} {}/{}",
                    event.operation.label(),
                    event.current_node_index.saturating_add(1),
                    event.total_nodes
                ),
                target: event
                    .target
                    .map(|target| format!("{} ({})", target.name, target.address))
                    .unwrap_or_else(|| "rolling coordinator".to_owned()),
                phase: format!("{:?}", event.phase),
                step: format!("{:?}", event.step),
                message: event.message,
            },
        }
    }
}

/// The single-node worker-to-egui boundary. Every payload is owned and has a
/// request identity so a late result cannot affect another context or node.
enum SingleEvent {
    Preflight {
        request: u64,
        identity: NodeIdentity,
        preflight: SinglePreflight,
    },
    Failed {
        request: u64,
        identity: NodeIdentity,
        message: String,
    },
    Progress {
        request: u64,
        identity: NodeIdentity,
        progress: ProgressSnapshot,
    },
    Completed {
        request: u64,
        identity: NodeIdentity,
        result: NodeOperationResult,
    },
}

#[derive(Clone)]
struct SingleConfirmation {
    identity: NodeIdentity,
    operation: OperationKind,
}

/// Native single-node operations route.
///
/// A confirmation dialog is deliberately separate from starting the Tokio
/// worker. The worker receives `OperationConfirmation::confirmed()` only after
/// that dialog names the exact context, Kubernetes node, and Talos address.
pub(crate) struct OperationsScreen {
    event_tx: mpsc::UnboundedSender<SingleEvent>,
    event_rx: mpsc::UnboundedReceiver<SingleEvent>,
    identity: Option<NodeIdentity>,
    active_task: Option<ActiveTask>,
    next_request: u64,
    active_request: Option<u64>,
    preflight: Option<SinglePreflight>,
    preflight_error: Option<String>,
    confirmation: Option<SingleConfirmation>,
    progress: Vec<ProgressSnapshot>,
    result: Option<NodeOperationResult>,
    status_message: Option<String>,
}

impl Default for OperationsScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl OperationsScreen {
    pub(crate) fn new() -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            event_tx,
            event_rx,
            identity: None,
            active_task: None,
            next_request: 0,
            active_request: None,
            preflight: None,
            preflight_error: None,
            confirmation: None,
            progress: Vec::new(),
            result: None,
            status_message: None,
        }
    }

    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        let Some(identity) = selected_node_identity(target) else {
            self.reset_for_no_node();
            draw_node_selection_required(ui, "Operations");
            return;
        };

        if self.identity.as_ref() != Some(&identity) {
            self.reset_for_identity(identity.clone());
            self.start_preflight(target, runtime, ctx, identity.clone());
        }
        self.clear_finished_task();
        self.drain_events(&identity);

        let refresh_requested = self.draw(ui, &identity);
        if refresh_requested {
            self.start_preflight(target, runtime, ctx, identity.clone());
        }

        if let Some(confirmation) = self.draw_confirmation(ctx) {
            if confirmation.identity != identity {
                self.status_message = Some(
                    "The node selection changed while the confirmation was open; no operation was started."
                        .to_owned(),
                );
            } else {
                self.start_operation(target, runtime, ctx, confirmation);
            }
        }
    }

    /// Request cancellation, discard route-local receivers, and invalidate all
    /// identities before a future activation can accept an old worker event.
    pub(crate) fn deactivate(&mut self) {
        self.cancel_and_detach_task();
        self.replace_event_channel();
        self.identity = None;
        self.preflight = None;
        self.preflight_error = None;
        self.confirmation = None;
        self.progress.clear();
        self.result = None;
        self.status_message = None;
    }

    fn reset_for_no_node(&mut self) {
        if self.identity.is_none() {
            return;
        }
        self.cancel_and_detach_task();
        self.replace_event_channel();
        self.identity = None;
        self.preflight = None;
        self.preflight_error = None;
        self.confirmation = None;
        self.progress.clear();
        self.result = None;
        self.status_message = None;
    }

    fn reset_for_identity(&mut self, identity: NodeIdentity) {
        self.cancel_and_detach_task();
        self.identity = Some(identity);
        self.preflight = None;
        self.preflight_error = None;
        self.confirmation = None;
        self.progress.clear();
        self.result = None;
        self.status_message = None;
    }

    fn replace_event_channel(&mut self) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        self.event_tx = event_tx;
        self.event_rx = event_rx;
    }

    fn clear_finished_task(&mut self) {
        if self.active_request.is_none()
            && self
                .active_task
                .as_ref()
                .is_some_and(|task| task.join.is_finished())
        {
            self.active_task = None;
        }
    }

    fn request_cancel(&self) {
        if let Some(task) = &self.active_task {
            task.request_cancel();
        }
    }

    fn cancel_and_detach_task(&mut self) {
        if let Some(task) = self.active_task.take() {
            task.request_cancel();
        }
        self.active_request = None;
    }

    fn start_preflight(
        &mut self,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
        identity: NodeIdentity,
    ) {
        self.cancel_and_detach_task();
        self.preflight = None;
        self.preflight_error = None;
        self.result = None;
        self.progress.clear();

        let Some(client) = target.cluster_client() else {
            self.preflight_error = Some(format!(
                "No Talos client is available for context {:?}.",
                identity.context
            ));
            return;
        };
        let Some(control_plane) = target.control_plane_address() else {
            self.preflight_error = Some(format!(
                "No control-plane address is available for context {:?}; Kubernetes and etcd preflight cannot be scoped safely.",
                identity.context
            ));
            return;
        };

        self.next_request = self.next_request.wrapping_add(1);
        let request = self.next_request;
        self.active_request = Some(request);
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = cancellation.clone();
        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        let join = runtime.spawn(async move {
            let result = collect_single_preflight(
                client,
                control_plane,
                identity.target.clone(),
                worker_cancellation.clone(),
            )
            .await;
            let event = if cancelled(&worker_cancellation) {
                SingleEvent::Failed {
                    request,
                    identity,
                    message: "Preflight cancelled.".to_owned(),
                }
            } else {
                match result {
                    Ok(preflight) => SingleEvent::Preflight {
                        request,
                        identity,
                        preflight,
                    },
                    Err(message) => SingleEvent::Failed {
                        request,
                        identity,
                        message,
                    },
                }
            };
            let _ = event_tx.send(event);
            repaint.request_repaint();
        });
        self.active_task = Some(ActiveTask { cancellation, join });
    }

    fn start_operation(
        &mut self,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
        confirmation: SingleConfirmation,
    ) {
        if self.active_task.is_some() {
            return;
        }

        let Some(client) = target.cluster_client() else {
            self.status_message =
                Some("The selected context no longer has a Talos client.".to_owned());
            return;
        };
        let Some(control_plane) = target.control_plane_address() else {
            self.status_message = Some(
                "The selected context no longer has a control-plane address; no operation was started."
                    .to_owned(),
            );
            return;
        };

        self.progress.clear();
        self.result = None;
        self.status_message = Some(format!(
            "Starting confirmed {} for {}…",
            confirmation.operation.label(),
            confirmation.identity.target.name
        ));

        self.next_request = self.next_request.wrapping_add(1);
        let request = self.next_request;
        self.active_request = Some(request);
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = cancellation.clone();
        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        let identity = confirmation.identity.clone();
        let operation = confirmation.operation;
        let join = runtime.spawn(async move {
            let kubeconfig_client = client.with_node(&control_plane);
            let execution = async {
                let (kubernetes, source) =
                    create_k8s_client_with_source(&client, None, Some(&kubeconfig_client))
                        .await
                        .map_err(|error| {
                            format!("Could not create a Talos-pinned Kubernetes client: {error}")
                        })?;
                ensure_pinned_source(source)?;
                if cancelled(&worker_cancellation) {
                    return Err("Operation cancelled before core execution started.".to_owned());
                }

                let etcd = collect_etcd_health(
                    client.clone(),
                    EtcdInspectionRequest::new(InspectionTarget::new(
                        identity.context.clone(),
                        control_plane.clone(),
                    )),
                )
                .await
                .map_err(|error| format!("Could not complete detailed etcd preflight: {error}"))?;
                let impact = etcd_impact_for_target(&etcd, &identity.target);

                let audit = AuditLog::new(audit_path(&identity.context), identity.context.clone());
                let node_client = client.with_node(&identity.target.address);
                let cancellation_for_core = worker_cancellation.clone();
                let is_cancelled = move || cancelled(&cancellation_for_core);
                let operation_context = OperationContext {
                    kubernetes: &kubernetes,
                    talos: Some(&node_client),
                    audit: &audit,
                    is_cancelled: &is_cancelled,
                };
                let mut operation_request =
                    NodeOperationRequest::new(operation, identity.target.clone());
                operation_request.etcd_impact = impact;
                operation_request.drain_options = DrainOptions::default();
                let progress_tx = event_tx.clone();
                let progress_identity = identity.clone();
                let progress_repaint = repaint.clone();
                let mut progress = move |event: OperationsEvent| {
                    let _ = progress_tx.send(SingleEvent::Progress {
                        request,
                        identity: progress_identity.clone(),
                        progress: ProgressSnapshot::from_event(event),
                    });
                    progress_repaint.request_repaint();
                };
                Ok::<NodeOperationResult, String>(
                    run_node_operation(
                        operation_request,
                        &operation_context,
                        OperationConfirmation::confirmed(),
                        &mut progress,
                    )
                    .await,
                )
            }
            .await;

            let event = match execution {
                Ok(result) => SingleEvent::Completed {
                    request,
                    identity,
                    result,
                },
                Err(message) => SingleEvent::Failed {
                    request,
                    identity,
                    message,
                },
            };
            let _ = event_tx.send(event);
            repaint.request_repaint();
        });
        self.active_task = Some(ActiveTask { cancellation, join });
    }

    fn drain_events(&mut self, visible: &NodeIdentity) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                SingleEvent::Preflight {
                    request,
                    identity,
                    preflight,
                } if &identity == visible && self.active_request == Some(request) => {
                    self.active_task = None;
                    self.active_request = None;
                    self.preflight = Some(preflight);
                    self.preflight_error = None;
                    self.status_message =
                        Some("Safety preflight completed from the selected context.".to_owned());
                }
                SingleEvent::Failed {
                    request,
                    identity,
                    message,
                } if &identity == visible && self.active_request == Some(request) => {
                    self.active_task = None;
                    self.active_request = None;
                    self.preflight_error = Some(message.clone());
                    self.status_message = Some(message);
                }
                SingleEvent::Progress {
                    request,
                    identity,
                    progress,
                } if &identity == visible && self.active_request == Some(request) => {
                    self.progress.push(progress);
                    if self.progress.len() > 80 {
                        self.progress.remove(0);
                    }
                }
                SingleEvent::Completed {
                    request,
                    identity,
                    result,
                } if &identity == visible && self.active_request == Some(request) => {
                    self.active_task = None;
                    self.active_request = None;
                    self.status_message = Some(result.message.clone());
                    self.result = Some(result);
                }
                _ => {
                    // Dropped: the worker belongs to a former context/node or a
                    // detached route receiver.
                }
            }
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui, identity: &NodeIdentity) -> bool {
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.heading("Operations");
            ui.separator();
            ui.label(RichText::new(format!("Context: {}", identity.context)).weak());
            ui.label(RichText::new(format!("Target: {}", identity.target.name)).strong());
            ui.label(RichText::new(format!("Talos: {}", identity.target.address)).monospace());
            if self.active_task.is_some() {
                ui.spinner();
                if ui.button("Cancel active operation").clicked() {
                    self.request_cancel();
                    self.status_message =
                        Some("Cancellation requested; waiting for core cleanup.".to_owned());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                refresh = ui
                    .add_enabled(
                        self.active_task.is_none(),
                        egui::Button::new("Refresh preflight"),
                    )
                    .clicked();
            });
        });
        ui.separator();

        self.draw_preflight(ui);
        if let Some(message) = &self.status_message {
            ui.colored_label(INFO, message);
        }
        if let Some(error) = &self.preflight_error {
            draw_error(ui, "Preflight unavailable", error);
        }

        let ready = self.active_task.is_none() && self.preflight.is_some();
        ui.add_space(8.0);
        ui.label(
            RichText::new("Every action opens an exact-target confirmation before core execution.")
                .weak(),
        );
        ui.horizontal_wrapped(|ui| {
            for operation in [
                OperationKind::Cordon,
                OperationKind::Uncordon,
                OperationKind::Drain,
                OperationKind::Reboot,
                OperationKind::Shutdown,
            ] {
                let safety_allows_operation = !operation.is_destructive()
                    || self.preflight.as_ref().is_some_and(|preflight| {
                        matches!(
                            evaluate_operation_safety(operation, &preflight.impact),
                            SafetyStatus::Safe | SafetyStatus::Warning(_)
                        )
                    });
                if ui
                    .add_enabled(
                        ready && safety_allows_operation,
                        egui::Button::new(operation.label()),
                    )
                    .clicked()
                {
                    self.confirmation = Some(SingleConfirmation {
                        identity: identity.clone(),
                        operation,
                    });
                }
            }
        });

        self.draw_progress(ui);
        self.draw_result(ui);
        refresh
    }

    fn draw_preflight(&self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.label(RichText::new("Safety preflight").strong());
            match &self.preflight {
                Some(preflight) => {
                    ui.colored_label(HEALTHY, format!("Kubernetes client: {}", preflight.kubernetes_source));
                    ui.label(&preflight.etcd_summary);
                    let safety = evaluate_operation_safety(OperationKind::Reboot, &preflight.impact);
                    draw_safety(ui, "Reboot / shutdown safety", &safety);
                    if let EtcdQuorumImpact::Unavailable { reason } = &preflight.impact {
                        ui.colored_label(
                            ERROR,
                            format!("Destructive operations remain blocked: {reason}"),
                        );
                    }
                }
                None if self.active_task.is_some() => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Creating a Talos-pinned Kubernetes client and collecting detailed etcd facts…");
                    });
                }
                None => {
                    ui.colored_label(UNKNOWN, "No current preflight. Refresh before performing an operation.");
                }
            }
        });
    }

    fn draw_progress(&self, ui: &mut egui::Ui) {
        if self.progress.is_empty() {
            return;
        }
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Core progress")
            .default_open(self.active_task.is_some())
            .show(ui, |ui| {
                for progress in &self.progress {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&progress.scope).strong());
                        ui.label(RichText::new(&progress.target).monospace());
                        ui.label(
                            RichText::new(format!("{} / {}", progress.phase, progress.step)).weak(),
                        );
                        ui.label(&progress.message);
                    });
                }
            });
    }

    fn draw_result(&self, ui: &mut egui::Ui) {
        let Some(result) = &self.result else {
            return;
        };
        ui.add_space(8.0);
        ui.group(|ui| {
            let color = result_color(result.status.is_success());
            ui.colored_label(
                color,
                format!("{}: {:?}", result.operation.label(), result.status),
            );
            ui.add(egui::Label::new(&result.message).selectable(true));
            ui.label(RichText::new(format!("Scheduling: {:?}", result.scheduling)).weak());
            if let Some(drain) = &result.drain {
                ui.label(format!(
                    "Drain: {} eligible, {} evicted, {} force-deleted, {} failed",
                    drain.eligible_pods,
                    drain.evicted_pods,
                    drain.force_deleted_pods.len(),
                    drain.failed_pods.len()
                ));
            }
            if let Some(error) = &result.audit_error {
                draw_error(ui, "Operation completed but audit write failed", error);
            } else {
                ui.colored_label(
                    HEALTHY,
                    "Structured operation audit was requested at the context-local audit path.",
                );
            }
        });
    }

    fn draw_confirmation(&mut self, ctx: &egui::Context) -> Option<SingleConfirmation> {
        let confirmation = self.confirmation.clone()?;
        let mut open = true;
        let mut approved = false;
        let mut rejected = false;
        egui::Window::new(format!("Confirm {}", confirmation.operation.label()))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.colored_label(
                    WARNING,
                    format!(
                        "Confirm {} for the exact target below.",
                        confirmation.operation.label()
                    ),
                );
                ui.label(format!("Context: {}", confirmation.identity.context));
                ui.label(format!(
                    "Kubernetes node: {}",
                    confirmation.identity.target.name
                ));
                ui.label(format!(
                    "Talos address: {}",
                    confirmation.identity.target.address
                ));
                ui.add_space(8.0);
                ui.label(operation_consequences(confirmation.operation));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    rejected = ui.button("Cancel").clicked();
                    approved = ui
                        .add(egui::Button::new(format!(
                            "Confirm {}",
                            confirmation.operation.label()
                        )))
                        .clicked();
                });
            });
        if !open || rejected {
            self.confirmation = None;
            return None;
        }
        if approved {
            self.confirmation = None;
            return Some(confirmation);
        }
        None
    }
}

/// The rolling worker-to-egui boundary. It never carries clients or mutable UI
/// state across the thread boundary.
enum RollingEvent {
    Preflight {
        request: u64,
        identity: RollingIdentity,
        preflight: RollingPreflight,
    },
    Failed {
        request: u64,
        identity: RollingIdentity,
        message: String,
    },
    Progress {
        request: u64,
        identity: RollingIdentity,
        progress: ProgressSnapshot,
    },
    Completed {
        request: u64,
        identity: RollingIdentity,
        result: RollingOperationResult,
    },
}

#[derive(Clone)]
struct RollingConfirmation {
    identity: RollingIdentity,
    operation: OperationKind,
    delay_seconds: u64,
    stop_on_failure: bool,
}

/// Native rolling-operation route. Node selection order is stored explicitly
/// rather than inferred from the cluster response order.
pub(crate) struct RollingOperationsScreen {
    event_tx: mpsc::UnboundedSender<RollingEvent>,
    event_rx: mpsc::UnboundedReceiver<RollingEvent>,
    active_context: Option<String>,
    selected_names: Vec<String>,
    operation: OperationKind,
    delay_seconds: u64,
    stop_on_failure: bool,
    active_task: Option<ActiveTask>,
    next_request: u64,
    active_request: Option<u64>,
    preflight: Option<(RollingIdentity, RollingPreflight)>,
    preflight_error: Option<String>,
    confirmation: Option<RollingConfirmation>,
    progress: Vec<ProgressSnapshot>,
    result: Option<RollingOperationResult>,
    status_message: Option<String>,
}

impl Default for RollingOperationsScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl RollingOperationsScreen {
    pub(crate) fn new() -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            event_tx,
            event_rx,
            active_context: None,
            selected_names: Vec::new(),
            operation: OperationKind::Reboot,
            delay_seconds: 30,
            stop_on_failure: true,
            active_task: None,
            next_request: 0,
            active_request: None,
            preflight: None,
            preflight_error: None,
            confirmation: None,
            progress: Vec::new(),
            result: None,
            status_message: None,
        }
    }

    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        self.sync_context(target.context_name());
        let available = rolling_targets(target);
        self.selected_names.retain(|name| {
            available
                .iter()
                .any(|node| node.name.as_str() == name.as_str())
        });
        let mut identity =
            rolling_identity(target.context_name(), &self.selected_names, &available);
        self.clear_finished_task();
        self.drain_events(&identity);

        let selection_changed = self.draw(ui, target.context_name(), &available);
        identity = rolling_identity(target.context_name(), &self.selected_names, &available);
        if selection_changed {
            self.invalidate_preflight();
        }

        if !identity.nodes.is_empty()
            && self.active_task.is_none()
            && self
                .preflight
                .as_ref()
                .is_none_or(|(preflight_identity, _)| preflight_identity != &identity)
        {
            self.start_preflight(target, runtime, ctx, identity.clone());
        }

        if let Some(confirmation) = self.draw_confirmation(ctx) {
            if confirmation.identity != identity {
                self.status_message = Some(
                    "The rolling selection changed while the confirmation was open; no operation was started."
                        .to_owned(),
                );
            } else {
                self.start_operation(target, runtime, ctx, confirmation);
            }
        }
    }

    /// Cancels active work and drops the receiver which represents this route's
    /// state. Its detached worker only owns a cancelled atomic flag and sender.
    pub(crate) fn deactivate(&mut self) {
        self.cancel_and_detach_task();
        self.replace_event_channel();
        self.active_context = None;
        self.selected_names.clear();
        self.preflight = None;
        self.preflight_error = None;
        self.confirmation = None;
        self.progress.clear();
        self.result = None;
        self.status_message = None;
    }

    fn sync_context(&mut self, context: &str) {
        if self.active_context.as_deref() == Some(context) {
            return;
        }
        self.cancel_and_detach_task();
        self.replace_event_channel();
        self.active_context = Some(context.to_owned());
        self.selected_names.clear();
        self.preflight = None;
        self.preflight_error = None;
        self.confirmation = None;
        self.progress.clear();
        self.result = None;
        self.status_message = None;
    }

    fn invalidate_preflight(&mut self) {
        self.cancel_and_detach_task();
        self.preflight = None;
        self.preflight_error = None;
        self.confirmation = None;
        self.result = None;
        self.progress.clear();
    }

    fn replace_event_channel(&mut self) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        self.event_tx = event_tx;
        self.event_rx = event_rx;
    }

    fn clear_finished_task(&mut self) {
        if self.active_request.is_none()
            && self
                .active_task
                .as_ref()
                .is_some_and(|task| task.join.is_finished())
        {
            self.active_task = None;
        }
    }

    fn request_cancel(&self) {
        if let Some(task) = &self.active_task {
            task.request_cancel();
        }
    }

    fn cancel_and_detach_task(&mut self) {
        if let Some(task) = self.active_task.take() {
            task.request_cancel();
        }
        self.active_request = None;
    }

    fn start_preflight(
        &mut self,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
        identity: RollingIdentity,
    ) {
        let Some(client) = target.cluster_client() else {
            self.preflight_error = Some(format!(
                "No Talos client is available for context {:?}.",
                identity.context
            ));
            return;
        };
        let Some(control_plane) = target.control_plane_address() else {
            self.preflight_error = Some(format!(
                "No control-plane address is available for context {:?}; rolling preflight cannot be scoped safely.",
                identity.context
            ));
            return;
        };

        self.next_request = self.next_request.wrapping_add(1);
        let request = self.next_request;
        self.active_request = Some(request);
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = cancellation.clone();
        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        let join = runtime.spawn(async move {
            let result = collect_rolling_preflight(
                client,
                control_plane,
                identity.nodes.clone(),
                worker_cancellation.clone(),
            )
            .await;
            let event = if cancelled(&worker_cancellation) {
                RollingEvent::Failed {
                    request,
                    identity,
                    message: "Rolling preflight cancelled.".to_owned(),
                }
            } else {
                match result {
                    Ok(preflight) => RollingEvent::Preflight {
                        request,
                        identity,
                        preflight,
                    },
                    Err(message) => RollingEvent::Failed {
                        request,
                        identity,
                        message,
                    },
                }
            };
            let _ = event_tx.send(event);
            repaint.request_repaint();
        });
        self.active_task = Some(ActiveTask { cancellation, join });
    }

    fn start_operation(
        &mut self,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
        confirmation: RollingConfirmation,
    ) {
        if self.active_task.is_some() || confirmation.identity.nodes.is_empty() {
            return;
        }
        let Some(client) = target.cluster_client() else {
            self.status_message =
                Some("The selected context no longer has a Talos client.".to_owned());
            return;
        };
        let Some(control_plane) = target.control_plane_address() else {
            self.status_message = Some(
                "The selected context no longer has a control-plane address; no rolling operation was started."
                    .to_owned(),
            );
            return;
        };

        self.progress.clear();
        self.result = None;
        self.status_message = Some(format!(
            "Starting confirmed rolling {} for {} node(s)…",
            confirmation.operation.label(),
            confirmation.identity.nodes.len()
        ));

        self.next_request = self.next_request.wrapping_add(1);
        let request = self.next_request;
        self.active_request = Some(request);
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = cancellation.clone();
        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        let identity = confirmation.identity.clone();
        let operation = confirmation.operation;
        let delay_seconds = confirmation.delay_seconds;
        let stop_on_failure = confirmation.stop_on_failure;
        let join = runtime.spawn(async move {
            let kubeconfig_client = client.with_node(&control_plane);
            let execution = async {
                let (kubernetes, source) =
                    create_k8s_client_with_source(&client, None, Some(&kubeconfig_client))
                        .await
                        .map_err(|error| {
                            format!("Could not create a Talos-pinned Kubernetes client: {error}")
                        })?;
                ensure_pinned_source(source)?;
                if cancelled(&worker_cancellation) {
                    return Err(
                        "Rolling operation cancelled before core execution started.".to_owned()
                    );
                }

                let etcd = collect_etcd_health(
                    client.clone(),
                    EtcdInspectionRequest::new(InspectionTarget::new(
                        identity.context.clone(),
                        control_plane.clone(),
                    )),
                )
                .await
                .map_err(|error| format!("Could not complete detailed etcd preflight: {error}"))?;
                let rolling_nodes = identity
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(index, node)| RollingNode {
                        target: node.clone(),
                        etcd_impact: etcd_impact_for_target(&etcd, node),
                        selection_order: index + 1,
                    })
                    .collect();
                let mut operation_request = RollingOperationRequest::new(operation, rolling_nodes);
                operation_request.drain_options = DrainOptions::default();
                operation_request.delay_between_nodes = Duration::from_secs(delay_seconds);
                operation_request.stop_on_failure = stop_on_failure;

                let audit = AuditLog::new(audit_path(&identity.context), identity.context.clone());
                let cancellation_for_core = worker_cancellation.clone();
                let is_cancelled = move || cancelled(&cancellation_for_core);
                let operation_context = OperationContext {
                    kubernetes: &kubernetes,
                    talos: Some(&client),
                    audit: &audit,
                    is_cancelled: &is_cancelled,
                };
                let progress_tx = event_tx.clone();
                let progress_identity = identity.clone();
                let progress_repaint = repaint.clone();
                let mut progress = move |event: OperationsEvent| {
                    let _ = progress_tx.send(RollingEvent::Progress {
                        request,
                        identity: progress_identity.clone(),
                        progress: ProgressSnapshot::from_event(event),
                    });
                    progress_repaint.request_repaint();
                };
                Ok::<RollingOperationResult, String>(
                    run_rolling_operation(
                        operation_request,
                        &operation_context,
                        OperationConfirmation::confirmed(),
                        &mut progress,
                    )
                    .await,
                )
            }
            .await;

            let event = match execution {
                Ok(result) => RollingEvent::Completed {
                    request,
                    identity,
                    result,
                },
                Err(message) => RollingEvent::Failed {
                    request,
                    identity,
                    message,
                },
            };
            let _ = event_tx.send(event);
            repaint.request_repaint();
        });
        self.active_task = Some(ActiveTask { cancellation, join });
    }

    fn drain_events(&mut self, visible: &RollingIdentity) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                RollingEvent::Preflight {
                    request,
                    identity,
                    preflight,
                } if &identity == visible && self.active_request == Some(request) => {
                    self.active_task = None;
                    self.active_request = None;
                    self.preflight = Some((identity, preflight));
                    self.preflight_error = None;
                    self.status_message = Some(
                        "Rolling safety preflight completed for every selected node.".to_owned(),
                    );
                }
                RollingEvent::Failed {
                    request,
                    identity,
                    message,
                } if &identity == visible && self.active_request == Some(request) => {
                    self.active_task = None;
                    self.active_request = None;
                    self.preflight_error = Some(message.clone());
                    self.status_message = Some(message);
                }
                RollingEvent::Progress {
                    request,
                    identity,
                    progress,
                } if &identity == visible && self.active_request == Some(request) => {
                    self.progress.push(progress);
                    if self.progress.len() > 120 {
                        self.progress.remove(0);
                    }
                }
                RollingEvent::Completed {
                    request,
                    identity,
                    result,
                } if &identity == visible && self.active_request == Some(request) => {
                    self.active_task = None;
                    self.active_request = None;
                    self.status_message = Some(result.message.clone());
                    self.result = Some(result);
                }
                _ => {
                    // A previous selection or dropped route receiver owns this event.
                }
            }
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui, context: &str, available: &[NodeTarget]) -> bool {
        let mut selection_changed = false;
        ui.horizontal(|ui| {
            ui.heading("Rolling operations");
            ui.separator();
            ui.label(RichText::new(format!("Context: {context}")).weak());
            if self.active_task.is_some() {
                ui.spinner();
                if ui.button("Cancel active rolling operation").clicked() {
                    self.request_cancel();
                    self.status_message =
                        Some("Cancellation requested; waiting for core cleanup.".to_owned());
                }
            }
        });
        ui.separator();

        ui.horizontal(|ui| {
            ui.radio_value(&mut self.operation, OperationKind::Reboot, "Rolling reboot");
            ui.radio_value(
                &mut self.operation,
                OperationKind::Shutdown,
                "Rolling shutdown",
            );
            ui.separator();
            ui.label("Delay between nodes (seconds)");
            ui.add(egui::DragValue::new(&mut self.delay_seconds).range(0..=86_400));
            ui.checkbox(&mut self.stop_on_failure, "Stop on failure");
        });
        ui.add_space(6.0);
        ui.label(
            RichText::new("Selected order is the execution order. Use arrows to change it.").weak(),
        );

        if available.is_empty() {
            ui.colored_label(
                UNKNOWN,
                "No context nodes are available. Refresh the selected Talos context before selecting a rolling target.",
            );
        } else {
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    for node in available {
                        let selected_index = self
                            .selected_names
                            .iter()
                            .position(|name| name == &node.name);
                        let mut selected = selected_index.is_some();
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut selected, "").changed() {
                                if selected {
                                    self.selected_names.push(node.name.clone());
                                } else {
                                    self.selected_names.retain(|name| name != &node.name);
                                }
                                selection_changed = true;
                            }
                            let order = self
                                .selected_names
                                .iter()
                                .position(|name| name == &node.name)
                                .map(|index| format!("{}.", index + 1))
                                .unwrap_or_else(|| "—".to_owned());
                            ui.label(RichText::new(order).monospace());
                            ui.label(RichText::new(&node.name).strong());
                            ui.label(RichText::new(&node.address).monospace().weak());
                            if let Some(index) = self
                                .selected_names
                                .iter()
                                .position(|name| name == &node.name)
                            {
                                if ui.add_enabled(index > 0, egui::Button::new("↑")).clicked() {
                                    self.selected_names.swap(index, index - 1);
                                    selection_changed = true;
                                }
                                if ui
                                    .add_enabled(
                                        index + 1 < self.selected_names.len(),
                                        egui::Button::new("↓"),
                                    )
                                    .clicked()
                                {
                                    self.selected_names.swap(index, index + 1);
                                    selection_changed = true;
                                }
                            }
                        });
                    }
                });
        }

        self.draw_preflight(ui);
        if let Some(message) = &self.status_message {
            ui.colored_label(INFO, message);
        }
        if let Some(error) = &self.preflight_error {
            draw_error(ui, "Rolling preflight unavailable", error);
        }

        let identity = rolling_identity(context, &self.selected_names, available);
        let ready = self.active_task.is_none()
            && rolling_preflight_allows(&self.preflight, &identity, self.operation);
        ui.add_space(8.0);
        if ui
            .add_enabled(
                ready,
                egui::Button::new(format!("Review rolling {}", self.operation.label())),
            )
            .clicked()
        {
            self.confirmation = Some(RollingConfirmation {
                identity,
                operation: self.operation,
                delay_seconds: self.delay_seconds,
                stop_on_failure: self.stop_on_failure,
            });
        }
        if !ready && !self.selected_names.is_empty() && self.active_task.is_none() {
            ui.colored_label(
                WARNING,
                "A complete, safe preflight is required for every selected destructive target.",
            );
        }

        self.draw_progress(ui);
        self.draw_result(ui);
        selection_changed
    }

    fn draw_preflight(&self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.label(RichText::new("Rolling safety preflight").strong());
            match &self.preflight {
                Some((identity, preflight)) => {
                    ui.colored_label(HEALTHY, format!("Kubernetes client: {}", preflight.kubernetes_source));
                    ui.label(&preflight.etcd_summary);
                    for (index, impact) in preflight.impacts.iter().enumerate() {
                        let safety = evaluate_operation_safety(self.operation, impact);
                        let target = identity
                            .nodes
                            .get(index)
                            .map(|node| format!("{}. {} ({})", index + 1, node.name, node.address))
                            .unwrap_or_else(|| format!("Selected node {}", index + 1));
                        draw_safety(ui, &target, &safety);
                        if let EtcdQuorumImpact::Unavailable { reason } = impact {
                            ui.colored_label(ERROR, reason);
                        }
                    }
                }
                None if self.active_task.is_some() => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Collecting one detailed etcd sample and validating every selected node…");
                    });
                }
                None if self.selected_names.is_empty() => {
                    ui.colored_label(UNKNOWN, "Select one or more nodes to collect rolling safety facts.");
                }
                None => {
                    ui.colored_label(UNKNOWN, "No current rolling preflight.");
                }
            }
        });
    }

    fn draw_progress(&self, ui: &mut egui::Ui) {
        if self.progress.is_empty() {
            return;
        }
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Core rolling progress")
            .default_open(self.active_task.is_some())
            .show(ui, |ui| {
                for progress in &self.progress {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&progress.scope).strong());
                        ui.label(RichText::new(&progress.target).monospace());
                        ui.label(
                            RichText::new(format!("{} / {}", progress.phase, progress.step)).weak(),
                        );
                        ui.label(&progress.message);
                    });
                }
            });
    }

    fn draw_result(&self, ui: &mut egui::Ui) {
        let Some(result) = &self.result else {
            return;
        };
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.colored_label(
                result_color(result.status.is_success()),
                format!(
                    "Rolling {}: {:?} ({} completed)",
                    result.operation.label(),
                    result.status,
                    result.completed_nodes()
                ),
            );
            ui.add(egui::Label::new(&result.message).selectable(true));
            for node in &result.nodes {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&node.target.name).strong());
                    ui.label(RichText::new(&node.target.address).monospace());
                    ui.label(format!("{:?}", node.result.status));
                    ui.label(&node.result.message);
                });
            }
            if let Some(error) = &result.audit_error {
                draw_error(
                    ui,
                    "Rolling operation completed but audit write failed",
                    error,
                );
            } else {
                ui.colored_label(
                    HEALTHY,
                    "Per-node structured results were sent to the context-local audit log.",
                );
            }
        });
    }

    fn draw_confirmation(&mut self, ctx: &egui::Context) -> Option<RollingConfirmation> {
        let confirmation = self.confirmation.clone()?;
        let mut open = true;
        let mut approved = false;
        let mut rejected = false;
        egui::Window::new(format!(
            "Confirm rolling {}",
            confirmation.operation.label()
        ))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.colored_label(
                WARNING,
                format!(
                    "This confirmed rolling {} will drain and act on every target below in order.",
                    confirmation.operation.label()
                ),
            );
            ui.label(format!("Context: {}", confirmation.identity.context));
            ui.label(format!(
                "Delay: {} seconds; stop on failure: {}",
                confirmation.delay_seconds, confirmation.stop_on_failure
            ));
            for (index, node) in confirmation.identity.nodes.iter().enumerate() {
                ui.label(format!(
                    "{}. Kubernetes node {} — Talos {}",
                    index + 1,
                    node.name,
                    node.address
                ));
            }
            ui.add_space(8.0);
            ui.label(operation_consequences(confirmation.operation));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                rejected = ui.button("Cancel").clicked();
                approved = ui
                    .add(egui::Button::new(format!(
                        "Confirm rolling {}",
                        confirmation.operation.label()
                    )))
                    .clicked();
            });
        });
        if !open || rejected {
            self.confirmation = None;
            return None;
        }
        if approved {
            self.confirmation = None;
            return Some(confirmation);
        }
        None
    }
}

/// Audit-history worker result. Parsing stays in `AuditLog`, so malformed YAML
/// is reported as a structured core decode error instead of being ignored.
enum AuditEvent {
    Loaded {
        request: u64,
        context: String,
        path: PathBuf,
        entries: Vec<AuditEntry>,
    },
    Failed {
        request: u64,
        context: String,
        path: PathBuf,
        message: String,
    },
}

/// Native structured operation audit screen.
pub(crate) struct AuditScreen {
    event_tx: mpsc::UnboundedSender<AuditEvent>,
    event_rx: mpsc::UnboundedReceiver<AuditEvent>,
    active_context: Option<String>,
    active_task: Option<ActiveTask>,
    next_request: u64,
    active_request: Option<u64>,
    path: Option<PathBuf>,
    entries: Vec<AuditEntry>,
    error: Option<String>,
    filter: String,
    selected_entry: Option<usize>,
}

impl Default for AuditScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditScreen {
    pub(crate) fn new() -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            event_tx,
            event_rx,
            active_context: None,
            active_task: None,
            next_request: 0,
            active_request: None,
            path: None,
            entries: Vec::new(),
            error: None,
            filter: String::new(),
            selected_entry: None,
        }
    }

    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        self.sync_context(target.context_name());
        self.clear_finished_task();
        self.drain_events(target.context_name());

        let refresh = self.draw(ui, target.context_name());
        if refresh || (self.active_task.is_none() && self.path.is_none() && self.error.is_none()) {
            self.start_refresh(target.context_name(), runtime, ctx);
        }
    }

    /// Audit reads have the same lifecycle discipline as network work: request
    /// cancellation and drop the route receiver so late disk results are inert.
    pub(crate) fn deactivate(&mut self) {
        self.cancel_and_detach_task();
        self.replace_event_channel();
        self.active_context = None;
        self.path = None;
        self.entries.clear();
        self.error = None;
        self.filter.clear();
        self.selected_entry = None;
    }

    fn sync_context(&mut self, context: &str) {
        if self.active_context.as_deref() == Some(context) {
            return;
        }
        self.cancel_and_detach_task();
        self.replace_event_channel();
        self.active_context = Some(context.to_owned());
        self.path = None;
        self.entries.clear();
        self.error = None;
        self.filter.clear();
        self.selected_entry = None;
    }

    fn replace_event_channel(&mut self) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        self.event_tx = event_tx;
        self.event_rx = event_rx;
    }

    fn clear_finished_task(&mut self) {
        if self.active_request.is_none()
            && self
                .active_task
                .as_ref()
                .is_some_and(|task| task.join.is_finished())
        {
            self.active_task = None;
        }
    }

    fn cancel_and_detach_task(&mut self) {
        if let Some(task) = self.active_task.take() {
            task.request_cancel();
        }
        self.active_request = None;
    }

    fn start_refresh(&mut self, context: &str, runtime: &Handle, ctx: &egui::Context) {
        if self.active_task.is_some() {
            return;
        }
        let context = context.to_owned();
        if context.trim().is_empty() {
            self.error = Some("No Talos context is selected for audit history.".to_owned());
            return;
        }
        let path = audit_path(&context);
        self.error = None;
        self.next_request = self.next_request.wrapping_add(1);
        let request = self.next_request;
        self.active_request = Some(request);
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = cancellation.clone();
        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        let worker_path = path.clone();
        let join = runtime.spawn(async move {
            let read_context = context.clone();
            let read_path = worker_path.clone();
            let result = tokio::task::spawn_blocking(move || {
                AuditLog::new(read_path, read_context).read_entries()
            })
            .await
            .map_err(|error| format!("Audit read worker failed: {error}"))
            .and_then(|result| result.map_err(|error| error.to_string()));
            if cancelled(&worker_cancellation) {
                return;
            }
            let event = match result {
                Ok(entries) => AuditEvent::Loaded {
                    request,
                    context,
                    path: worker_path,
                    entries,
                },
                Err(message) => AuditEvent::Failed {
                    request,
                    context,
                    path: worker_path,
                    message,
                },
            };
            let _ = event_tx.send(event);
            repaint.request_repaint();
        });
        self.active_task = Some(ActiveTask { cancellation, join });
    }

    fn drain_events(&mut self, visible_context: &str) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                AuditEvent::Loaded {
                    request,
                    context,
                    path,
                    entries,
                } if context == visible_context && self.active_request == Some(request) => {
                    self.active_task = None;
                    self.active_request = None;
                    self.path = Some(path);
                    self.entries = entries;
                    self.error = None;
                    self.selected_entry = None;
                }
                AuditEvent::Failed {
                    request,
                    context,
                    path,
                    message,
                } if context == visible_context && self.active_request == Some(request) => {
                    self.active_task = None;
                    self.active_request = None;
                    self.path = Some(path);
                    self.error = Some(message);
                    self.selected_entry = None;
                }
                _ => {
                    // The event came from a deactivated route or another context.
                }
            }
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui, context: &str) -> bool {
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.heading("Audit history");
            ui.separator();
            ui.label(RichText::new(format!("Context: {context}")).weak());
            if self.active_task.is_some() {
                ui.spinner();
                ui.label("Reading context-local YAML audit history…");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                refresh = ui
                    .add_enabled(
                        self.active_task.is_none(),
                        egui::Button::new("Refresh audit"),
                    )
                    .clicked();
            });
        });
        if let Some(path) = &self.path {
            ui.label(
                RichText::new(format!("Path: {}", path.display()))
                    .monospace()
                    .weak(),
            );
        }
        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.text_edit_singleline(&mut self.filter);
        });
        ui.separator();

        if let Some(error) = &self.error {
            draw_error(ui, "Audit history unavailable or malformed", error);
            return refresh;
        }
        if self.path.is_none() {
            ui.colored_label(UNKNOWN, "Audit history has not returned yet.");
            return refresh;
        }
        if self.entries.is_empty() {
            ui.colored_label(
                UNKNOWN,
                "No audit records exist for this context yet. Mutating operations append structured YAML here.",
            );
            return refresh;
        }

        let filter = self.filter.to_lowercase();
        let filtered: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| audit_matches(entry, &filter).then_some(index))
            .collect();
        if filtered.is_empty() {
            ui.colored_label(UNKNOWN, "No audit records match the current filter.");
            return refresh;
        }

        ui.columns(2, |columns| {
            egui::ScrollArea::vertical().show(&mut columns[0], |ui| {
                for index in filtered {
                    let entry = &self.entries[index];
                    let selected = self.selected_entry == Some(index);
                    let label = format!(
                        "{}  {}  {}  {}",
                        entry.timestamp.to_rfc3339(),
                        entry.operation.label(),
                        entry.target.name,
                        format!("{:?}", entry.phase)
                    );
                    if ui.selectable_label(selected, label).clicked() {
                        self.selected_entry = Some(index);
                    }
                }
            });
            if let Some(index) = self
                .selected_entry
                .filter(|index| self.entries.get(*index).is_some())
            {
                draw_audit_detail(&mut columns[1], &self.entries[index]);
            } else {
                columns[1].colored_label(UNKNOWN, "Select an audit record for structured details.");
            }
        });
        refresh
    }
}

fn selected_node_identity(target: &ScreenTarget<'_>) -> Option<NodeIdentity> {
    let name = target.node_name()?.trim();
    if name.is_empty() {
        return None;
    }
    let address = target.node_address()?;
    if address.trim().is_empty() || target.context_name().trim().is_empty() {
        return None;
    }
    Some(NodeIdentity {
        context: target.context_name().to_owned(),
        target: NodeTarget::new(name, address),
    })
}

fn rolling_targets(target: &ScreenTarget<'_>) -> Vec<NodeTarget> {
    let mut nodes = BTreeMap::new();
    for (name, address) in &target.cluster.node_ips {
        if !name.trim().is_empty() && !address.trim().is_empty() {
            nodes.insert(name.clone(), address.clone());
        }
    }
    for version in &target.cluster.versions {
        if version.node.trim().is_empty() {
            continue;
        }
        nodes
            .entry(version.node.clone())
            .or_insert_with(|| version.node.clone());
    }
    nodes
        .into_iter()
        .map(|(name, address)| NodeTarget::new(name, address))
        .collect()
}

fn rolling_identity(
    context: &str,
    selected_names: &[String],
    available: &[NodeTarget],
) -> RollingIdentity {
    let nodes = selected_names
        .iter()
        .filter_map(|name| {
            available
                .iter()
                .find(|node| node.name.as_str() == name.as_str())
                .cloned()
        })
        .collect();
    RollingIdentity {
        context: context.to_owned(),
        nodes,
    }
}

async fn collect_single_preflight(
    client: talos_rs::TalosClient,
    control_plane: String,
    target: NodeTarget,
    cancellation: Arc<AtomicBool>,
) -> Result<SinglePreflight, String> {
    let kubeconfig_client = client.with_node(&control_plane);
    let (_, source) = create_k8s_client_with_source(&client, None, Some(&kubeconfig_client))
        .await
        .map_err(|error| format!("Could not create a Talos-pinned Kubernetes client: {error}"))?;
    let kubernetes_source = ensure_pinned_source(source)?;
    if cancelled(&cancellation) {
        return Err("Preflight cancelled.".to_owned());
    }
    let etcd = collect_etcd_health(
        client,
        EtcdInspectionRequest::new(InspectionTarget::new("operations preflight", control_plane)),
    )
    .await
    .map_err(|error| format!("Could not complete detailed etcd preflight: {error}"))?;
    let impact = etcd_impact_for_target(&etcd, &target);
    Ok(SinglePreflight {
        kubernetes_source,
        etcd_summary: etcd_summary(&etcd),
        impact,
    })
}

async fn collect_rolling_preflight(
    client: talos_rs::TalosClient,
    control_plane: String,
    targets: Vec<NodeTarget>,
    cancellation: Arc<AtomicBool>,
) -> Result<RollingPreflight, String> {
    let kubeconfig_client = client.with_node(&control_plane);
    let (_, source) = create_k8s_client_with_source(&client, None, Some(&kubeconfig_client))
        .await
        .map_err(|error| format!("Could not create a Talos-pinned Kubernetes client: {error}"))?;
    let kubernetes_source = ensure_pinned_source(source)?;
    if cancelled(&cancellation) {
        return Err("Rolling preflight cancelled.".to_owned());
    }
    let etcd = collect_etcd_health(
        client,
        EtcdInspectionRequest::new(InspectionTarget::new("rolling preflight", control_plane)),
    )
    .await
    .map_err(|error| format!("Could not complete detailed etcd preflight: {error}"))?;
    let impacts = targets
        .iter()
        .map(|target| etcd_impact_for_target(&etcd, target))
        .collect();
    Ok(RollingPreflight {
        kubernetes_source,
        etcd_summary: etcd_summary(&etcd),
        impacts,
    })
}

fn ensure_pinned_source(source: KubeconfigSource) -> Result<String, String> {
    match source {
        KubeconfigSource::TalosNode(node) => Ok(format!("Talos-pinned kubeconfig from {node}")),
        KubeconfigSource::Environment => Err(
            "Refused an ambient KUBECONFIG client; operations require a Talos-pinned kubeconfig."
                .to_owned(),
        ),
        KubeconfigSource::Unavailable(reason) => Err(format!(
            "No Talos-pinned Kubernetes client is available: {reason}"
        )),
    }
}

/// Produces an impact only from a complete detailed etcd sample. Any optional
/// source failure becomes explicit unavailability, never an optimistic quorum.
fn etcd_impact_for_target(snapshot: &EtcdHealthSnapshot, target: &NodeTarget) -> EtcdQuorumImpact {
    if !snapshot.unavailable.is_empty() {
        let unavailable = snapshot
            .unavailable
            .iter()
            .map(|source| format!("{}: {}", source.source, source.message))
            .collect::<Vec<_>>()
            .join("; ");
        return EtcdQuorumImpact::unavailable(format!(
            "Detailed etcd data is incomplete for {}: {unavailable}",
            target.name
        ));
    }
    if snapshot.voting_members == 0 {
        return EtcdQuorumImpact::unavailable(
            "Detailed etcd member list contains no voting members; quorum impact is unknown.",
        );
    }

    let target_member = snapshot.members.iter().find(|member| {
        member.info.hostname.eq_ignore_ascii_case(&target.name)
            || member
                .info
                .ip_address()
                .is_some_and(|address| address == target.address)
    });
    let target_is_member = target_member.is_some_and(|member| !member.info.is_learner);
    let target_is_healthy = target_member.is_some_and(|member| {
        !member.info.is_learner && member.is_reachable() && !member.has_problems()
    });
    let healthy_members = snapshot
        .members
        .iter()
        .filter(|member| !member.info.is_learner && member.is_reachable() && !member.has_problems())
        .count();
    EtcdQuorumImpact::known(
        target_is_member,
        target_is_healthy,
        healthy_members,
        snapshot.voting_members,
    )
}

fn etcd_summary(snapshot: &EtcdHealthSnapshot) -> String {
    let healthy_members = snapshot
        .members
        .iter()
        .filter(|member| !member.info.is_learner && member.is_reachable() && !member.has_problems())
        .count();
    format!(
        "Detailed etcd sample: {healthy_members}/{} healthy voting members; {} optional source warning(s).",
        snapshot.voting_members,
        snapshot.unavailable.len()
    )
}

fn rolling_preflight_allows(
    preflight: &Option<(RollingIdentity, RollingPreflight)>,
    identity: &RollingIdentity,
    operation: OperationKind,
) -> bool {
    let Some((preflight_identity, preflight)) = preflight else {
        return false;
    };
    preflight_identity == identity
        && preflight.impacts.len() == identity.nodes.len()
        && preflight.impacts.iter().all(|impact| {
            matches!(
                evaluate_operation_safety(operation, impact),
                SafetyStatus::Safe | SafetyStatus::Warning(_)
            )
        })
}

fn cancelled(cancellation: &AtomicBool) -> bool {
    cancellation.load(Ordering::Acquire)
}

/// Context names participate in the local file name but are sanitized so they
/// cannot escape the `.talos-pilot` directory selected for the current user.
fn audit_path(context: &str) -> PathBuf {
    let safe_context: String = context
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let directory = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".talos-pilot");
    directory.join(format!(
        "operations-{}.yaml",
        if safe_context.is_empty() {
            "default"
        } else {
            &safe_context
        }
    ))
}

fn operation_consequences(operation: OperationKind) -> &'static str {
    match operation {
        OperationKind::Cordon => {
            "Cordon prevents new Kubernetes workloads from scheduling on this node."
        }
        OperationKind::Uncordon => {
            "Uncordon allows Kubernetes to schedule new workloads on this node."
        }
        OperationKind::Drain => {
            "Drain cordons this node and evicts eligible workloads, respecting PodDisruptionBudgets and configured core safety defaults."
        }
        OperationKind::Reboot => {
            "Reboot drains eligible workloads, then asks Talos to reboot the exact address after etcd quorum safety passes."
        }
        OperationKind::Shutdown => {
            "Shutdown drains eligible workloads, then asks Talos to shut down the exact address after etcd quorum safety passes."
        }
    }
}

fn draw_node_selection_required(ui: &mut egui::Ui, title: &str) {
    ui.heading(title);
    ui.separator();
    ui.colored_label(
        UNKNOWN,
        "Select a node in the cluster sidebar. No operation can be prepared without both a Kubernetes node name and Talos address.",
    );
}

fn draw_safety(ui: &mut egui::Ui, label: &str, safety: &SafetyStatus) {
    let (color, detail) = match safety {
        SafetyStatus::Safe => (HEALTHY, "safe".to_owned()),
        SafetyStatus::Warning(reason) => (WARNING, format!("warning: {reason}")),
        SafetyStatus::Unsafe(reason) => (ERROR, format!("unsafe: {reason}")),
        SafetyStatus::Unknown => (
            UNKNOWN,
            "unknown — destructive execution is blocked".to_owned(),
        ),
    };
    ui.colored_label(color, format!("{label}: {detail}"));
}

fn draw_error(ui: &mut egui::Ui, title: &str, message: &str) {
    ui.group(|ui| {
        ui.colored_label(ERROR, title);
        ui.add(egui::Label::new(message).selectable(true));
    });
}

fn result_color(success: bool) -> Color32 {
    if success { HEALTHY } else { ERROR }
}

fn audit_matches(entry: &AuditEntry, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    [
        entry.timestamp.to_rfc3339(),
        entry.cluster.clone(),
        entry.actor.clone(),
        entry.operation.label().to_owned(),
        entry.target.name.clone(),
        entry.target.address.clone(),
        format!("{:?}", entry.phase),
        format!("{:?}", entry.step),
        entry.message.clone(),
    ]
    .iter()
    .any(|value| value.to_lowercase().contains(filter))
}

fn draw_audit_detail(ui: &mut egui::Ui, entry: &AuditEntry) {
    let details = format!(
        "Timestamp: {}\nContext: {}\nActor: {}\nOperation: {}\nKubernetes node: {}\nTalos address: {}\nPhase: {:?}\nStep: {:?}\nMessage: {}",
        entry.timestamp.to_rfc3339(),
        entry.cluster,
        entry.actor,
        entry.operation.label(),
        entry.target.name,
        entry.target.address,
        entry.phase,
        entry.step,
        entry.message,
    );
    ui.horizontal(|ui| {
        ui.label(RichText::new("Structured record").strong());
        if ui.button("Copy details").clicked() {
            ui.ctx().copy_text(details.clone());
        }
    });
    ui.add(egui::Label::new(RichText::new(details).monospace()).selectable(true));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_audit_path_is_local_and_sanitized() {
        let path = audit_path("prod/us-east");
        assert!(path.ends_with(".talos-pilot/operations-prod_us-east.yaml"));
    }

    #[test]
    fn rolling_identity_preserves_explicit_selection_order() {
        let available = vec![
            NodeTarget::new("one", "10.0.0.1"),
            NodeTarget::new("two", "10.0.0.2"),
        ];
        let identity =
            rolling_identity("context", &["two".to_owned(), "one".to_owned()], &available);
        assert_eq!(identity.nodes[0].name, "two");
        assert_eq!(identity.nodes[1].name, "one");
    }
}
