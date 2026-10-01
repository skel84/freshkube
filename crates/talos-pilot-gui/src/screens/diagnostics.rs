//! Native diagnostics screen backed exclusively by the core diagnostic runner.
//!
//! Every Talos and Kubernetes request is made by the core collector on the
//! caller-supplied Tokio runtime. The egui thread only owns presentation state
//! and receives owned, target-tagged events from workers.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

use eframe::egui::{self, Color32, RichText};
use talos_pilot_core::{
    diagnostic_runner::{
        ConfirmedDiagnosticFix, DiagnosticCheck, DiagnosticCollector, DiagnosticFixAction,
        DiagnosticFixRequest, DiagnosticSnapshot, DiagnosticTarget, FixExecution,
        FixExecutionPhase, execute_confirmed_fix,
    },
    diagnostics::{CheckCategory, CheckStatus},
};
use tokio::runtime::Handle;

use crate::screens::ScreenTarget;

const HEALTHY: Color32 = Color32::from_rgb(76, 175, 80);
const WARNING: Color32 = Color32::from_rgb(255, 193, 7);
const ERROR: Color32 = Color32::from_rgb(244, 67, 54);
const UNKNOWN: Color32 = Color32::from_rgb(158, 158, 158);
const PENDING: Color32 = Color32::from_rgb(100, 181, 246);
const CATEGORIES: [CheckCategory; 5] = [
    CheckCategory::System,
    CheckCategory::Kubernetes,
    CheckCategory::Cni,
    CheckCategory::Services,
    CheckCategory::Addons,
];

/// Immutable identity for the target rendered by this screen.
///
/// A diagnostic target retains the selected node name and address. The
/// enclosing context avoids accepting a result from another Talos
/// configuration that happens to use the same node name.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DiagnosticsTarget {
    context: String,
    diagnostic: Option<DiagnosticTarget>,
}

/// Identity assigned to one collection request.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RequestIdentity {
    sequence: u64,
    context: String,
    target: DiagnosticTarget,
}

impl RequestIdentity {
    fn matches(&self, event: &Self) -> bool {
        self.sequence == event.sequence
            && self.context == event.context
            && self.target == event.target
    }
}

/// Identity assigned to one confirmed mutation.
///
/// The check ID is included so a delayed result cannot be presented as the
/// outcome for a different remediation on the same node.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FixRequestIdentity {
    sequence: u64,
    context: String,
    target: DiagnosticTarget,
    check_id: String,
}

impl FixRequestIdentity {
    fn matches(&self, event: &Self) -> bool {
        self.sequence == event.sequence
            && self.context == event.context
            && self.target == event.target
            && self.check_id == event.check_id
    }
}

struct ActiveFix {
    request: FixRequestIdentity,
    cancellation: Arc<AtomicBool>,
    cancellation_requested: bool,
}

/// All values crossing from a Tokio worker into egui are immutable and owned.
enum DiagnosticsEvent {
    Collected {
        request: RequestIdentity,
        snapshot: DiagnosticSnapshot,
    },
    FixCompleted {
        request: FixRequestIdentity,
        result: Result<FixExecution, String>,
    },
}

struct FixFeedback {
    title: String,
    content: String,
    color: Color32,
}

/// Egui presentation state for node-scoped diagnostics.
pub(crate) struct DiagnosticsScreen {
    event_tx: mpsc::Sender<DiagnosticsEvent>,
    event_rx: mpsc::Receiver<DiagnosticsEvent>,
    target: Option<DiagnosticsTarget>,
    next_sequence: u64,
    active_collection: Option<RequestIdentity>,
    active_fix: Option<ActiveFix>,
    snapshot: Option<DiagnosticSnapshot>,
    collection_error: Option<String>,
    fix_feedback: Option<FixFeedback>,
    search: String,
    selected_check_id: Option<String>,
    pending_fix: Option<DiagnosticFixRequest>,
}

impl Default for DiagnosticsScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl DiagnosticsScreen {
    pub(crate) fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        Self {
            event_tx,
            event_rx,
            target: None,
            next_sequence: 0,
            active_collection: None,
            active_fix: None,
            snapshot: None,
            collection_error: None,
            fix_feedback: None,
            search: String::new(),
            selected_check_id: None,
            pending_fix: None,
        }
    }

    /// Renders diagnostics for the explicitly selected node.
    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        let target_changed = self.sync_target(target);
        self.drain_events();
        if target_changed {
            self.start_collection(target, runtime, ctx);
        }

        let refresh_requested = self.draw_toolbar(ui);
        if refresh_requested {
            self.start_collection(target, runtime, ctx);
        }

        self.draw_content(ui);
        self.draw_confirmation_modal(target, runtime, ctx);
    }

    /// Cooperatively cancel a running remediation and invalidate every worker
    /// result that may arrive after this route is no longer visible.
    pub(crate) fn deactivate(&mut self) {
        self.invalidate_outstanding_requests();
        self.target = None;
        self.snapshot = None;
        self.collection_error = None;
        self.fix_feedback = None;
        self.search.clear();
        self.selected_check_id = None;
        self.pending_fix = None;
    }

    fn sync_target(&mut self, target: &ScreenTarget<'_>) -> bool {
        let next = DiagnosticsTarget {
            context: target.context_name().to_owned(),
            diagnostic: diagnostic_target(target),
        };
        if self.target.as_ref() == Some(&next) {
            return false;
        }

        self.invalidate_outstanding_requests();
        self.target = Some(next);
        self.snapshot = None;
        self.collection_error = None;
        self.fix_feedback = None;
        self.selected_check_id = None;
        self.pending_fix = None;
        true
    }

    fn invalidate_outstanding_requests(&mut self) {
        self.next_sequence = self.next_sequence.wrapping_add(1);
        if let Some(active_fix) = &self.active_fix {
            active_fix.cancellation.store(true, Ordering::Release);
        }
        self.active_collection = None;
        self.active_fix = None;
    }

    fn draw_toolbar(&mut self, ui: &mut egui::Ui) -> bool {
        let target = self
            .target
            .as_ref()
            .expect("target synchronized before rendering");
        let diagnostic = target.diagnostic.as_ref();
        ui.horizontal_wrapped(|ui| {
            match diagnostic {
                Some(target) => ui.heading(format!(
                    "Diagnostics — {} / {} ({})",
                    target.config_identity, target.node_name, target.node_address
                )),
                None => ui.heading(format!("Diagnostics — {}", target.context)),
            };
            if self.active_collection.is_some() {
                ui.spinner();
                ui.label("Collecting on a Tokio worker…");
            }
            if let Some(active_fix) = &self.active_fix {
                ui.spinner();
                ui.label(if active_fix.cancellation_requested {
                    "Cancellation requested; waiting for the active Talos request boundary…"
                } else {
                    "Applying confirmed remediation on a Tokio worker…"
                });
            }
        });

        let mut refresh = false;
        ui.horizontal_wrapped(|ui| {
            refresh = ui
                .add_enabled(
                    self.active_collection.is_none() && self.active_fix.is_none(),
                    egui::Button::new("Refresh diagnostics"),
                )
                .clicked();
            if let Some(active_fix) = &mut self.active_fix {
                if ui
                    .add_enabled(
                        !active_fix.cancellation_requested,
                        egui::Button::new("Cancel remediation"),
                    )
                    .clicked()
                {
                    active_fix.cancellation.store(true, Ordering::Release);
                    active_fix.cancellation_requested = true;
                }
            }
        });
        ui.separator();
        refresh
    }

    fn start_collection(
        &mut self,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        if self.active_collection.is_some() || self.active_fix.is_some() {
            return;
        }

        let Some(screen_target) = self.target.as_ref() else {
            return;
        };
        let context = screen_target.context.clone();
        let Some(diagnostic) = screen_target.diagnostic.clone() else {
            self.collection_error = Some(target_unavailable_reason(target));
            return;
        };
        if context.trim().is_empty() {
            self.collection_error =
                Some("No Talos context is selected for diagnostic collection.".to_owned());
            return;
        }
        let Some(client) = target.node_client() else {
            self.collection_error = Some(format!(
                "No Talos client is available for node {:?} in context {:?}.",
                diagnostic.node_name, context
            ));
            return;
        };

        self.next_sequence = self.next_sequence.wrapping_add(1);
        let request = RequestIdentity {
            sequence: self.next_sequence,
            context,
            target: diagnostic.clone(),
        };
        self.active_collection = Some(request.clone());
        self.collection_error = None;

        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        runtime.spawn(async move {
            let snapshot = DiagnosticCollector::new(diagnostic).collect(&client).await;
            let _ = event_tx.send(DiagnosticsEvent::Collected { request, snapshot });
            repaint.request_repaint();
        });
    }

    fn draw_content(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = &self.collection_error {
            draw_message(ui, "Diagnostics unavailable", error, UNKNOWN);
            if self.snapshot.is_none() {
                return;
            }
            ui.add_space(8.0);
        }

        if self.snapshot.is_none() {
            if self.active_collection.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Waiting for the first diagnostic snapshot…");
                });
            } else if self
                .target
                .as_ref()
                .is_some_and(|target| target.diagnostic.is_none())
            {
                ui.colored_label(UNKNOWN, "Select a node to collect diagnostics.");
            } else {
                ui.colored_label(UNKNOWN, "No diagnostic snapshot is available yet.");
            }
            return;
        }

        Self::draw_filter(ui, &mut self.search);
        let snapshot = self.snapshot.as_ref().expect("checked above");
        Self::draw_snapshot_summary(ui, snapshot);
        Self::draw_check_categories(ui, snapshot, &self.search, &mut self.selected_check_id);

        let selected_check = self
            .selected_check_id
            .as_deref()
            .and_then(|id| snapshot.checks.iter().find(|check| check.id == id))
            .cloned();
        if self.selected_check_id.is_some() && selected_check.is_none() {
            self.selected_check_id = None;
        }
        if let Some(request) = Self::draw_selected_check(
            ui,
            selected_check,
            &snapshot.context.target,
            self.active_fix.is_none() && self.active_collection.is_none(),
        ) {
            self.pending_fix = Some(request);
        }
        Self::draw_fix_feedback(ui, self.fix_feedback.as_ref());
    }

    fn draw_snapshot_summary(ui: &mut egui::Ui, snapshot: &DiagnosticSnapshot) {
        let mut passed = 0usize;
        let mut warnings = 0usize;
        let mut failed = 0usize;
        let mut unavailable = 0usize;
        let mut checking = 0usize;
        for check in &snapshot.checks {
            match &check.status {
                CheckStatus::Pass => passed += 1,
                CheckStatus::Warn => warnings += 1,
                CheckStatus::Fail => failed += 1,
                CheckStatus::Unknown => unavailable += 1,
                CheckStatus::Checking => checking += 1,
            }
        }

        ui.horizontal_wrapped(|ui| {
            metric(ui, "Pass", passed.to_string(), HEALTHY);
            metric(ui, "Warning", warnings.to_string(), WARNING);
            metric(ui, "Failed", failed.to_string(), ERROR);
            metric(ui, "Unavailable", unavailable.to_string(), UNKNOWN);
            if checking > 0 {
                metric(ui, "Checking", checking.to_string(), PENDING);
            }
        });

        if unavailable > 0 || checking > 0 {
            ui.colored_label(
                UNKNOWN,
                format!(
                    "Partial diagnostics: {unavailable} check(s) are unavailable and {checking} check(s) are still checking. These are not reported as failures."
                ),
            );
        }
    }

    fn draw_filter(ui: &mut egui::Ui, search: &mut String) {
        ui.horizontal(|ui| {
            ui.label("Filter checks:");
            ui.add(
                egui::TextEdit::singleline(search)
                    .hint_text("name, status, message, details, category")
                    .desired_width(330.0),
            );
            if ui.small_button("Clear filter").clicked() {
                search.clear();
            }
        });
    }

    fn draw_check_categories(
        ui: &mut egui::Ui,
        snapshot: &DiagnosticSnapshot,
        search: &str,
        selected_check_id: &mut Option<String>,
    ) {
        if snapshot.checks.is_empty() {
            ui.add_space(12.0);
            ui.colored_label(
                UNKNOWN,
                "The core collector completed but returned no diagnostic checks.",
            );
            return;
        }

        let filter = search.trim().to_ascii_lowercase();
        let mut selected = None;
        let mut has_matching_check = false;
        for category in CATEGORIES {
            let has_category = snapshot
                .checks
                .iter()
                .any(|check| check.category == category && check_matches(check, &filter));
            if !has_category && !filter.is_empty() {
                continue;
            }

            ui.add_space(8.0);
            ui.heading(category.title());
            if !has_category {
                ui.colored_label(UNKNOWN, "No checks were returned for this category.");
                continue;
            }

            has_matching_check = true;
            egui::Grid::new(("diagnostic_checks", category.title()))
                .num_columns(3)
                .striped(true)
                .min_col_width(110.0)
                .show(ui, |ui| {
                    ui.strong("Status");
                    ui.strong("Check");
                    ui.strong("Summary");
                    ui.end_row();
                    for check in snapshot
                        .checks
                        .iter()
                        .filter(|check| check.category == category && check_matches(check, &filter))
                    {
                        let selected_row = selected_check_id.as_deref() == Some(&check.id);
                        if ui
                            .add(egui::Button::selectable(
                                selected_row,
                                RichText::new(status_label(&check.status))
                                    .color(status_color(&check.status)),
                            ))
                            .clicked()
                        {
                            selected = Some(check.id.clone());
                        }
                        if ui
                            .add(egui::Button::selectable(
                                selected_row,
                                RichText::new(&check.name).strong(),
                            ))
                            .clicked()
                        {
                            selected = Some(check.id.clone());
                        }
                        if ui
                            .add(egui::Button::selectable(
                                selected_row,
                                RichText::new(&check.message).color(status_color(&check.status)),
                            ))
                            .clicked()
                        {
                            selected = Some(check.id.clone());
                        }
                        ui.end_row();
                    }
                });
        }
        if !has_matching_check && !filter.is_empty() {
            ui.add_space(12.0);
            ui.colored_label(UNKNOWN, "No diagnostic checks match the current filter.");
        }
        if let Some(id) = selected {
            *selected_check_id = Some(id);
        }
    }

    fn draw_selected_check(
        ui: &mut egui::Ui,
        selected_check: Option<DiagnosticCheck>,
        selected_target: &DiagnosticTarget,
        can_review_remediation: bool,
    ) -> Option<DiagnosticFixRequest> {
        let Some(check) = selected_check else {
            ui.add_space(10.0);
            ui.colored_label(
                UNKNOWN,
                "Select a diagnostic check to inspect its evidence.",
            );
            return None;
        };

        let details = check_details(&check);
        let check_id = check.id;
        let name = check.name;
        let status = check.status;
        let fix = check.fix;
        let mut remediation = None;
        ui.add_space(12.0);
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&name).strong());
                ui.colored_label(status_color(&status), status_label(&status));
                if ui.button("Copy details").clicked() {
                    ui.ctx().copy_text(details.clone());
                }
            });
            ui.add(
                egui::Label::new(RichText::new(&details).monospace())
                    .selectable(true)
                    .wrap(),
            );

            if let Some(fix) = fix {
                ui.separator();
                match &fix.action {
                    DiagnosticFixAction::CopyGuidance { title, text } => {
                        ui.label(RichText::new(title).strong());
                        ui.add(
                            egui::Label::new(RichText::new(text).monospace())
                                .selectable(true)
                                .wrap(),
                        );
                        if ui.button("Copy guidance").clicked() {
                            ui.ctx().copy_text(text.clone());
                        }
                    }
                    action if action.is_executable() => {
                        ui.label(format!("Supported remediation: {}", fix.description));
                        if ui
                            .add_enabled(
                                can_review_remediation,
                                egui::Button::new("Review remediation"),
                            )
                            .clicked()
                        {
                            remediation = Some(fix);
                        }
                    }
                    _ => {}
                }
            }
        });
        remediation.map(|fix| DiagnosticFixRequest::new(check_id, selected_target.clone(), fix))
    }

    fn draw_fix_feedback(ui: &mut egui::Ui, feedback: Option<&FixFeedback>) {
        let Some(feedback) = feedback else {
            return;
        };
        ui.add_space(10.0);
        ui.group(|ui| {
            ui.colored_label(feedback.color, RichText::new(&feedback.title).strong());
            if ui.button("Copy remediation result").clicked() {
                ui.ctx().copy_text(feedback.content.clone());
            }
            ui.add(
                egui::Label::new(RichText::new(&feedback.content).monospace())
                    .selectable(true)
                    .wrap(),
            );
        });
    }

    fn draw_confirmation_modal(
        &mut self,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        let Some(request) = self.pending_fix.as_ref().cloned() else {
            return;
        };
        let preview = request.preview();
        let action = action_label(&preview.action);
        if !preview.confirmation_required {
            self.pending_fix = None;
            return;
        }

        let mut open = true;
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Confirm diagnostic remediation")
            .id(egui::Id::new((
                "diagnostic_fix_confirmation",
                &preview.check_id,
            )))
            .collapsible(false)
            .resizable(true)
            .default_width(600.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("This action affects the explicitly selected Talos node.")
                        .strong()
                        .color(WARNING),
                );
                egui::Grid::new(("diagnostic_fix_preview", &preview.check_id))
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        modal_row(ui, "Context", &preview.target.config_identity);
                        modal_row(ui, "Node", &preview.target.node_name);
                        modal_row(ui, "Talos address", &preview.target.node_address);
                        modal_row(ui, "Check", &preview.check_id);
                        modal_row(ui, "Action", &action);
                        modal_row(ui, "Remediation", &preview.description);
                        modal_row(
                            ui,
                            "Reboot consequence",
                            if preview.requires_reboot {
                                "This configuration patch requests an immediate reboot."
                            } else {
                                "This action does not request a reboot."
                            },
                        );
                    });

                ui.add_space(8.0);
                let content_label = if matches!(
                    &preview.action,
                    DiagnosticFixAction::ApplyConfigPatch { .. }
                ) {
                    "YAML preview"
                } else {
                    "Action preview"
                };
                ui.label(RichText::new(content_label).strong());
                let content = preview
                    .content
                    .as_deref()
                    .unwrap_or("The core runner did not provide a preview.");
                if ui.button("Copy preview").clicked() {
                    ui.ctx().copy_text(content.to_owned());
                }
                ui.add(
                    egui::Label::new(RichText::new(content).monospace())
                        .selectable(true)
                        .wrap(),
                );

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .add_enabled(
                            self.active_fix.is_none(),
                            egui::Button::new("Confirm and execute"),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });

        if confirm {
            self.pending_fix = None;
            self.start_fix(request, target, runtime, ctx);
        } else if cancel || !open {
            self.pending_fix = None;
        }
    }

    fn start_fix(
        &mut self,
        request: DiagnosticFixRequest,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        if self.active_fix.is_some() {
            return;
        }
        let Some(screen_target) = self.target.as_ref() else {
            return;
        };
        if !modal_targets_current(screen_target, &request) {
            self.fix_feedback = Some(FixFeedback {
                title: "Remediation not started".to_owned(),
                content: "The selected node changed before confirmation. Review the current diagnostic check again.".to_owned(),
                color: WARNING,
            });
            return;
        }
        let context = screen_target.context.clone();
        if !request.fix.action.is_executable() {
            return;
        }
        let Some(client) = target.node_client() else {
            self.fix_feedback = Some(FixFeedback {
                title: "Remediation unavailable".to_owned(),
                content: format!(
                    "No Talos client is available for {} ({}).",
                    request.target.node_name, request.target.node_address
                ),
                color: ERROR,
            });
            return;
        };

        self.next_sequence = self.next_sequence.wrapping_add(1);
        let event_request = FixRequestIdentity {
            sequence: self.next_sequence,
            context,
            target: request.target.clone(),
            check_id: request.check_id.clone(),
        };
        let cancellation = Arc::new(AtomicBool::new(false));
        self.active_fix = Some(ActiveFix {
            request: event_request.clone(),
            cancellation: Arc::clone(&cancellation),
            cancellation_requested: false,
        });
        self.fix_feedback = None;

        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        let confirmed: ConfirmedDiagnosticFix = request.confirm();
        runtime.spawn(async move {
            let mut is_cancelled = || cancellation.load(Ordering::Acquire);
            let result = execute_confirmed_fix(&client, confirmed, &mut is_cancelled)
                .await
                .map_err(|error| error.to_string());
            let _ = event_tx.send(DiagnosticsEvent::FixCompleted {
                request: event_request,
                result,
            });
            repaint.request_repaint();
        });
    }

    fn drain_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                DiagnosticsEvent::Collected { request, snapshot }
                    if self.accepts_collection(&request) =>
                {
                    self.active_collection = None;
                    self.collection_error = None;
                    self.reconcile_selection(&snapshot);
                    self.snapshot = Some(snapshot);
                }
                DiagnosticsEvent::FixCompleted { request, result }
                    if self.accepts_fix(&request) =>
                {
                    self.active_fix = None;
                    self.fix_feedback = Some(match result {
                        Ok(execution) => fix_feedback(execution),
                        Err(error) => FixFeedback {
                            title: "Remediation failed".to_owned(),
                            content: error,
                            color: ERROR,
                        },
                    });
                }
                _ => {
                    // Stale context, node, address, check, or request sequence.
                    // The event is deliberately dropped before it can alter egui state.
                }
            }
        }
    }

    fn accepts_collection(&self, event: &RequestIdentity) -> bool {
        self.active_collection
            .as_ref()
            .is_some_and(|active| active.matches(event))
            && self.target.as_ref().is_some_and(|target| {
                target.context == event.context && target.diagnostic.as_ref() == Some(&event.target)
            })
    }

    fn accepts_fix(&self, event: &FixRequestIdentity) -> bool {
        self.active_fix
            .as_ref()
            .is_some_and(|active| active.request.matches(event))
            && self.target.as_ref().is_some_and(|target| {
                target.context == event.context && target.diagnostic.as_ref() == Some(&event.target)
            })
    }

    fn reconcile_selection(&mut self, snapshot: &DiagnosticSnapshot) {
        if self
            .selected_check_id
            .as_deref()
            .is_some_and(|selected| !snapshot.checks.iter().any(|check| check.id == selected))
        {
            self.selected_check_id = None;
        }
    }
}

fn diagnostic_target(target: &ScreenTarget<'_>) -> Option<DiagnosticTarget> {
    let node_name = target.node_name()?.to_owned();
    let node_address = target.node_address()?;
    let node_role = if target.selected_node_is_controlplane() == Some(true) {
        "controlplane"
    } else {
        "worker"
    };
    let diagnostic = DiagnosticTarget::new(
        node_name,
        node_address,
        node_role,
        target.context_name().to_owned(),
    );
    Some(match target.control_plane_address() {
        Some(address) => diagnostic.with_control_plane_address(address),
        None => diagnostic,
    })
}

fn target_unavailable_reason(target: &ScreenTarget<'_>) -> String {
    match target.node_name() {
        None => "Select a node before collecting diagnostics.".to_owned(),
        Some(node) if target.node_address().is_none() => {
            format!("Node {node:?} has no Talos management address.")
        }
        Some(node) => format!("Node {node:?} has no usable Talos client."),
    }
}

fn check_matches(check: &DiagnosticCheck, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let details = check.details.as_deref().unwrap_or_default();
    check.name.to_ascii_lowercase().contains(filter)
        || check.message.to_ascii_lowercase().contains(filter)
        || details.to_ascii_lowercase().contains(filter)
        || check.id.to_ascii_lowercase().contains(filter)
        || check.category.title().to_ascii_lowercase().contains(filter)
        || status_label(&check.status)
            .to_ascii_lowercase()
            .contains(filter)
}

fn status_label(status: &CheckStatus) -> &'static str {
    match status {
        CheckStatus::Pass => "Pass",
        CheckStatus::Warn => "Warning",
        CheckStatus::Fail => "Failed",
        CheckStatus::Unknown => "Unavailable",
        CheckStatus::Checking => "Checking",
    }
}

fn status_color(status: &CheckStatus) -> Color32 {
    match status {
        CheckStatus::Pass => HEALTHY,
        CheckStatus::Warn => WARNING,
        CheckStatus::Fail => ERROR,
        CheckStatus::Unknown => UNKNOWN,
        CheckStatus::Checking => PENDING,
    }
}

fn check_details(check: &DiagnosticCheck) -> String {
    let details = check
        .details
        .as_deref()
        .unwrap_or("No additional evidence was supplied.");
    format!(
        "Check: {}\nCategory: {}\nStatus: {}\nSummary: {}\n\nEvidence:\n{}",
        check.name,
        check.category.title(),
        status_label(&check.status),
        check.message,
        details
    )
}

fn action_label(action: &DiagnosticFixAction) -> String {
    match action {
        DiagnosticFixAction::RestartService { service_id } => {
            format!("Restart Talos service {service_id}")
        }
        DiagnosticFixAction::ApplyConfigPatch { .. } => {
            "Apply Talos configuration patch".to_owned()
        }
        DiagnosticFixAction::CopyGuidance { title, .. } => format!("Copy guidance: {title}"),
    }
}

fn modal_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).strong());
    ui.add(egui::Label::new(RichText::new(value).monospace()).selectable(true));
    ui.end_row();
}

fn metric(ui: &mut egui::Ui, label: &str, value: String, color: Color32) {
    ui.group(|ui| {
        ui.label(RichText::new(label).strong());
        ui.colored_label(color, value);
    });
}

fn draw_message(ui: &mut egui::Ui, title: &str, message: &str, color: Color32) {
    ui.colored_label(color, RichText::new(title).strong());
    ui.add(
        egui::Label::new(RichText::new(message).color(color))
            .selectable(true)
            .wrap(),
    );
}

fn modal_targets_current(
    screen_target: &DiagnosticsTarget,
    request: &DiagnosticFixRequest,
) -> bool {
    screen_target.diagnostic.as_ref() == Some(&request.target)
}

fn fix_feedback(execution: FixExecution) -> FixFeedback {
    match execution {
        FixExecution::Applied {
            results,
            requires_reboot,
        } => {
            let mut content = String::new();
            for result in results {
                if !content.is_empty() {
                    content.push_str("\n\n");
                }
                content.push_str(&format!(
                    "Node: {}\nResult: {}",
                    result.node, result.message
                ));
                if !result.warnings.is_empty() {
                    content.push_str("\nWarnings:");
                    for warning in result.warnings {
                        content.push_str("\n- ");
                        content.push_str(&warning);
                    }
                }
            }
            if content.is_empty() {
                content.push_str(
                    "Talos accepted the remediation request but returned no result rows.",
                );
            }
            if requires_reboot {
                content.push_str("\n\nThe applied configuration requested a reboot.");
            }
            FixFeedback {
                title: "Remediation applied".to_owned(),
                content,
                color: HEALTHY,
            }
        }
        FixExecution::Cancelled {
            phase,
            requires_reboot,
        } => FixFeedback {
            title: "Remediation cancelled".to_owned(),
            content: format!(
                "Cancellation was observed {}.{}",
                fix_phase_label(phase),
                if requires_reboot {
                    " The original configuration patch would have requested a reboot."
                } else {
                    ""
                }
            ),
            color: WARNING,
        },
        FixExecution::CopyOnly { text } => FixFeedback {
            title: "Copy-only guidance".to_owned(),
            content: text,
            color: UNKNOWN,
        },
    }
}

fn fix_phase_label(phase: FixExecutionPhase) -> &'static str {
    match phase {
        FixExecutionPhase::BeforeServiceRestart => "before the service restart request",
        FixExecutionPhase::BeforeConfigValidation => "before configuration validation",
        FixExecutionPhase::BeforeConfigApply => "after validation but before configuration apply",
    }
}

#[cfg(test)]
mod tests {
    use super::{DiagnosticsTarget, modal_targets_current};
    use talos_pilot_core::diagnostic_runner::{
        DiagnosticFix, DiagnosticFixAction, DiagnosticFixRequest, DiagnosticTarget,
    };

    fn diagnostic_target(address: &str) -> DiagnosticTarget {
        DiagnosticTarget::new("control-1", address, "controlplane", "production")
    }

    #[test]
    fn stale_modal_target_cannot_execute_against_a_reselected_node() {
        let modal_request = DiagnosticFixRequest::new(
            "service_kubelet",
            diagnostic_target("10.0.0.10"),
            DiagnosticFix {
                description: "Restart kubelet".to_owned(),
                action: DiagnosticFixAction::RestartService {
                    service_id: "kubelet".to_owned(),
                },
            },
        );
        let current = DiagnosticsTarget {
            context: "production".to_owned(),
            diagnostic: Some(diagnostic_target("10.0.0.11")),
        };

        assert!(!modal_targets_current(&current, &modal_request));
    }
}
