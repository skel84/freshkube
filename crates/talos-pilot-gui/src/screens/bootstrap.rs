//! Native maintenance-mode bootstrap wizard.
//!
//! The egui thread owns the [`BootstrapSession`] snapshot and only advances it
//! by reducing immutable events. Every Talos, Kubernetes, and subprocess action
//! is executed on the caller-provided Tokio runtime, then returned here through
//! a screen-local channel.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Instant,
};

use eframe::egui::{self, Color32, RichText};
use talos_pilot_core::maintenance::{
    AuthenticatedTarget, BOOTSTRAP_POLL_INTERVAL, BootstrapCommand, BootstrapPaths, BootstrapPhase,
    BootstrapPlan, BootstrapReadinessEvidence, BootstrapSession, ConfigurationApplicationRequest,
    ConfigurationGenerationRequest, EtcdReadinessEvidence, InstallTarget,
    KubernetesCredentialsSource, KubernetesReadinessEvidence, MachineRole, MaintenanceAction,
    MaintenanceEndpoint, MaintenanceEvent, SourceAvailability, TalosReadinessEvidence,
    execute_maintenance_action,
};
use tokio::runtime::Handle;

const HEALTHY: Color32 = Color32::from_rgb(76, 175, 80);
const WARNING: Color32 = Color32::from_rgb(255, 193, 7);
const ERROR: Color32 = Color32::from_rgb(244, 67, 54);
const MUTED: Color32 = Color32::from_rgb(158, 158, 158);
const INFO: Color32 = Color32::from_rgb(100, 181, 246);

/// Settings collected before the first insecure maintenance request.
///
/// Paths are populated only by the launcher or native file dialogs. In
/// particular, this screen never reads `KUBECONFIG` from the environment.
#[derive(Default)]
struct BootstrapForm {
    cluster_name: String,
    kubernetes_endpoint: String,
    talos_context: String,
    kubernetes_node_name: String,
    output_dir: Option<PathBuf>,
    talosconfig_path: Option<PathBuf>,
    kubeconfig_path: Option<PathBuf>,
    role: MachineRole,
}

struct ActiveWork {
    id: u64,
    cancellation: Arc<AtomicBool>,
}

/// Immutable items delivered to the UI thread.
///
/// Local reductions use the same queue as completed worker requests. This keeps
/// every session transition in `drain_events`, including confirmations and
/// cancellation, rather than mutating a session from button handlers.
enum BootstrapEvent {
    Reduce {
        epoch: u64,
        event: MaintenanceEvent,
        follow_up: Option<MaintenanceAction>,
    },
    Completed {
        epoch: u64,
        work_id: u64,
        event: MaintenanceEvent,
    },
}

/// Egui state for a maintenance-mode Talos bootstrap.
///
/// A session is deliberately local to this route. Its epoch and each worker's
/// cancellation flag ensure a late maintenance result cannot affect a later
/// visit, endpoint, or retry.
pub(crate) struct BootstrapScreen {
    event_tx: mpsc::Sender<BootstrapEvent>,
    event_rx: mpsc::Receiver<BootstrapEvent>,
    endpoint_identity: Option<String>,
    epoch: u64,
    next_work_id: u64,
    active_work: Option<ActiveWork>,
    next_poll_at: Option<Instant>,
    session: Option<BootstrapSession>,
    form: BootstrapForm,
    local_error: Option<String>,
    apply_confirmed: bool,
    bootstrap_confirmed: bool,
}

impl Default for BootstrapScreen {
    fn default() -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        Self {
            event_tx,
            event_rx,
            endpoint_identity: None,
            epoch: 0,
            next_work_id: 0,
            active_work: None,
            next_poll_at: None,
            session: None,
            form: BootstrapForm::default(),
            local_error: None,
            apply_confirmed: false,
            bootstrap_confirmed: false,
        }
    }
}

impl BootstrapScreen {
    /// Render the maintenance bootstrap wizard.
    ///
    /// `maintenance_endpoint` is supplied exclusively by `--insecure
    /// --endpoint`. A selected secure config can seed the explicit talosconfig
    /// picker, but never substitutes for the maintenance endpoint.
    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        maintenance_endpoint: Option<&str>,
        config_path: Option<&PathBuf>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        self.sync_maintenance_endpoint(maintenance_endpoint);
        self.seed_talosconfig(config_path);
        self.drain_events(runtime, ctx);

        let Some(endpoint) = maintenance_endpoint
            .map(str::trim)
            .filter(|endpoint| !endpoint.is_empty())
        else {
            self.draw_missing_endpoint(ui);
            return;
        };

        self.maybe_start_readiness_poll(runtime, ctx);
        self.draw_header(ui, endpoint);
        if let Some(error) = &self.local_error {
            ui.colored_label(ERROR, error);
            ui.add_space(6.0);
        }

        if self.session.is_none() {
            self.draw_setup(ui, endpoint, runtime, ctx);
            return;
        }

        self.draw_session(ui, endpoint, runtime, ctx);
    }

    /// Cancel and invalidate all work for this route.
    ///
    /// A task already inside a remote request cannot necessarily be interrupted,
    /// but it observes the atomic before its next step and its result is ignored
    /// after the epoch changes.
    pub(crate) fn deactivate(&mut self) {
        self.invalidate_active_work();
        self.session = None;
        self.local_error = None;
        self.apply_confirmed = false;
        self.bootstrap_confirmed = false;
    }

    fn sync_maintenance_endpoint(&mut self, endpoint: Option<&str>) {
        let next = endpoint
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        if self.endpoint_identity == next {
            return;
        }

        self.deactivate();
        self.endpoint_identity = next.clone();
        if self.form.kubernetes_endpoint.trim().is_empty()
            && let Some(endpoint) = next
                .as_deref()
                .and_then(|value| MaintenanceEndpoint::parse(value).ok())
        {
            self.form.kubernetes_endpoint = endpoint.kubernetes_api_url();
        }
    }

    fn seed_talosconfig(&mut self, config_path: Option<&PathBuf>) {
        if self.form.talosconfig_path.is_none() {
            self.form.talosconfig_path = config_path.cloned();
        }
    }

    fn draw_missing_endpoint(&self, ui: &mut egui::Ui) {
        ui.heading("Bootstrap a Talos cluster");
        ui.add_space(8.0);
        ui.colored_label(
            WARNING,
            "Maintenance bootstrap is unavailable because no --insecure --endpoint was supplied.",
        );
        ui.add_space(4.0);
        ui.label(
            "Start the GUI with an explicit maintenance-mode Talos node endpoint. This route will not infer a node from the selected secure context or create mock maintenance data.",
        );
    }

    fn draw_header(&self, ui: &mut egui::Ui, endpoint: &str) {
        ui.horizontal(|ui| {
            ui.heading("Bootstrap a Talos cluster");
            ui.separator();
            ui.label("Maintenance endpoint:");
            ui.monospace(endpoint);
            if self.active_work.is_some() {
                ui.spinner();
            }
        });
        ui.colored_label(
            WARNING,
            "This workflow can write machine configuration and install Talos to the explicitly selected disk.",
        );
        ui.separator();
    }

    fn draw_setup(
        &mut self,
        ui: &mut egui::Ui,
        endpoint: &str,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        ui.heading("1. Configure an explicit bootstrap plan");
        ui.label(
            "These values define both the generated configuration and the authenticated readiness target. Nothing is read from ambient TALOSCONFIG or KUBECONFIG.",
        );
        ui.add_space(6.0);
        let talosconfig_path = self.form.talosconfig_path.clone();
        let output_dir = self.form.output_dir.clone();
        let kubeconfig_path = self.form.kubeconfig_path.clone();

        egui::Grid::new("bootstrap_plan_form")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Cluster name");
                ui.text_edit_singleline(&mut self.form.cluster_name)
                    .on_hover_text("Required; passed to the target-aware configuration generator.");
                ui.end_row();

                ui.label("Kubernetes API endpoint");
                ui.text_edit_singleline(&mut self.form.kubernetes_endpoint)
                    .on_hover_text("Required; the generated cluster API endpoint.");
                ui.end_row();

                ui.label("Talos context");
                ui.text_edit_singleline(&mut self.form.talos_context).on_hover_text(
                    "Required exact context name in the explicitly selected talosconfig. It is used for talosctl bootstrap and secure readiness probes.",
                );
                ui.end_row();

                ui.label("Kubernetes node name");
                ui.text_edit_singleline(&mut self.form.kubernetes_node_name).on_hover_text(
                    "Optional exact Kubernetes node that must report Ready. Leave empty to require at least one Ready node.",
                );
                ui.end_row();

                ui.label("Machine role");
                ui.label("Control plane (required for this initial cluster bootstrap)");
                ui.end_row();

                ui.label("Talos configuration");
                self.draw_path_picker(
                    ui,
                    talosconfig_path.as_ref(),
                    "Choose talosconfig…",
                    "Choose the talosconfig containing the exact context above",
                    PathPicker::File,
                    PathSlot::Talosconfig,
                );
                ui.end_row();

                ui.label("Configuration output directory");
                self.draw_path_picker(
                    ui,
                    output_dir.as_ref(),
                    "Choose output directory…",
                    "Choose where target-aware generated configuration files will be written",
                    PathPicker::Directory,
                    PathSlot::OutputDirectory,
                );
                ui.end_row();

                ui.label("Kubernetes configuration");
                self.draw_path_picker(
                    ui,
                    kubeconfig_path.as_ref(),
                    "Choose kubeconfig (optional)…",
                    "Optionally use this explicit kubeconfig for readiness. Otherwise credentials are fetched from the authenticated Talos API.",
                    PathPicker::File,
                    PathSlot::Kubeconfig,
                );
                ui.end_row();
            });

        if self.form.kubeconfig_path.is_none() {
            ui.colored_label(
                INFO,
                "No kubeconfig is selected. Kubernetes readiness will request credentials from the authenticated Talos API for this node.",
            );
        }

        ui.add_space(10.0);
        if ui.button("Load insecure maintenance data").clicked() {
            self.start_session(endpoint, runtime, ctx);
        }
    }

    fn draw_path_picker(
        &mut self,
        ui: &mut egui::Ui,
        current: Option<&PathBuf>,
        choose_label: &str,
        dialog_title: &str,
        picker: PathPicker,
        slot: PathSlot,
    ) {
        ui.horizontal(|ui| {
            match current {
                Some(path) => ui.monospace(path.display().to_string()),
                None if matches!(slot, PathSlot::Kubeconfig) => {
                    ui.label(RichText::new("Talos API (not selected)").color(MUTED))
                }
                None => ui.label(RichText::new("Not selected").color(WARNING)),
            };
            if ui.button(choose_label).clicked() {
                let dialog = rfd::FileDialog::new().set_title(dialog_title);
                let path = match picker {
                    PathPicker::File => dialog.pick_file(),
                    PathPicker::Directory => dialog.pick_folder(),
                };
                if let Some(path) = path {
                    match slot {
                        PathSlot::Talosconfig => self.form.talosconfig_path = Some(path),
                        PathSlot::OutputDirectory => self.form.output_dir = Some(path),
                        PathSlot::Kubeconfig => self.form.kubeconfig_path = Some(path),
                    }
                }
            }
            if current.is_some()
                && matches!(slot, PathSlot::Kubeconfig)
                && ui.small_button("Use Talos API instead").clicked()
            {
                self.form.kubeconfig_path = None;
            }
        });
    }

    fn start_session(&mut self, endpoint: &str, runtime: &Handle, ctx: &egui::Context) {
        self.invalidate_active_work();
        self.local_error = None;
        self.apply_confirmed = false;
        self.bootstrap_confirmed = false;

        let plan = match self.build_plan(endpoint) {
            Ok(plan) => plan,
            Err(error) => {
                self.session = None;
                self.local_error = Some(error);
                return;
            }
        };
        let session = BootstrapSession::new(plan);
        let Some(action) = session.initial_collection_action() else {
            self.local_error = Some(
                "Bootstrap session did not provide an insecure collection action.".to_string(),
            );
            return;
        };
        self.session = Some(session);
        self.start_worker(action, runtime, ctx);
    }

    fn build_plan(&self, endpoint: &str) -> Result<BootstrapPlan, String> {
        let endpoint = MaintenanceEndpoint::parse(endpoint).map_err(|error| error.to_string())?;
        let output_dir = self.form.output_dir.clone().ok_or_else(|| {
            "Choose a configuration output directory before continuing.".to_string()
        })?;
        let talosconfig_path = self
            .form
            .talosconfig_path
            .clone()
            .ok_or_else(|| "Choose a talosconfig before continuing.".to_string())?;
        let kubernetes_node_name = (!self.form.kubernetes_node_name.trim().is_empty())
            .then(|| self.form.kubernetes_node_name.trim().to_string());
        let authenticated_target = AuthenticatedTarget::new(
            self.form.talos_context.trim(),
            endpoint.clone(),
            kubernetes_node_name,
        )
        .map_err(|error| error.to_string())?;

        BootstrapPlan::new(
            endpoint,
            self.form.cluster_name.trim(),
            self.form.kubernetes_endpoint.trim(),
            output_dir,
            self.form.role,
            BootstrapPaths {
                talosconfig_path,
                kubeconfig_path: self.form.kubeconfig_path.clone(),
            },
            authenticated_target,
        )
        .map_err(|error| error.to_string())
    }

    fn draw_session(
        &mut self,
        ui: &mut egui::Ui,
        endpoint: &str,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        let Some(session) = self.session.clone() else {
            return;
        };

        ui.label(RichText::new(format!("Phase: {}", phase_label(&session.phase))).strong());
        if let Some(message) = &session.last_message {
            ui.colored_label(WARNING, message);
        }
        ui.add_space(6.0);

        match &session.phase {
            BootstrapPhase::CollectingInsecureData => self.draw_collecting(ui, ctx),
            BootstrapPhase::SelectingInstallTarget => self.draw_install_target(ui, &session, ctx),
            BootstrapPhase::Configuring => self.draw_configuring(ui, &session, endpoint, ctx),
            BootstrapPhase::GeneratingConfiguration => self.draw_generating(ui, ctx),
            BootstrapPhase::ConfigurationReady => self.draw_configuration_ready(ui, &session, ctx),
            BootstrapPhase::AwaitingApplyConfirmation => {
                self.draw_apply_confirmation(ui, &session, ctx)
            }
            BootstrapPhase::ApplyingConfiguration => self.draw_applying(ui, ctx),
            BootstrapPhase::WaitingForTalos => self.draw_waiting_for_talos(ui, &session, ctx),
            BootstrapPhase::ReadyToBootstrap => self.draw_bootstrap_confirmation(ui, &session, ctx),
            BootstrapPhase::Bootstrapping => self.draw_bootstrapping(ui, ctx),
            BootstrapPhase::WaitingForKubernetes => {
                self.draw_waiting_for_kubernetes(ui, &session, ctx)
            }
            BootstrapPhase::Complete => self.draw_complete(ui, &session, endpoint, runtime, ctx),
            BootstrapPhase::Cancelled => self.draw_terminal_actions(
                ui,
                endpoint,
                runtime,
                ctx,
                "The maintenance workflow was cancelled.",
            ),
            BootstrapPhase::Failed(error) => self.draw_terminal_actions(
                ui,
                endpoint,
                runtime,
                ctx,
                &format!("Maintenance workflow failed: {error}"),
            ),
        }
    }

    fn draw_collecting(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.spinner();
        ui.label("Loading version, disk, and volume information from the insecure Talos maintenance APIs…");
        self.draw_cancel_button(ui, ctx);
    }

    fn draw_install_target(
        &mut self,
        ui: &mut egui::Ui,
        session: &BootstrapSession,
        ctx: &egui::Context,
    ) {
        ui.heading("2. Review maintenance data and select an install target");
        ui.label("Disk inventory comes directly from Talos. Read-only and optical devices cannot be selected.");
        let Some(snapshot) = &session.insecure_snapshot else {
            ui.colored_label(
                ERROR,
                "The maintenance response did not include a disk snapshot.",
            );
            return;
        };

        ui.add_space(6.0);
        ui.label(RichText::new("Insecure Talos version").strong());
        match &snapshot.version {
            SourceAvailability::Available(version) => {
                ui.colored_label(
                    INFO,
                    format!(
                        "{} ({})",
                        version.tag,
                        if version.maintenance_mode {
                            "maintenance mode"
                        } else {
                            "maintenance mode not reported"
                        }
                    ),
                );
            }
            SourceAvailability::Unavailable { reason } => {
                ui.colored_label(WARNING, format!("Version API unavailable: {reason}"));
            }
        }

        ui.add_space(6.0);
        ui.label(RichText::new("Talos disk inventory").strong());
        let mut selected_device = None;
        egui::Grid::new("maintenance_disks")
            .num_columns(6)
            .striped(true)
            .show(ui, |ui| {
                ui.label(RichText::new("Device").strong());
                ui.label(RichText::new("Size").strong());
                ui.label(RichText::new("Model").strong());
                ui.label(RichText::new("Transport").strong());
                ui.label(RichText::new("Status").strong());
                ui.label("");
                ui.end_row();

                for disk in &snapshot.disks {
                    let installable = !disk.readonly
                        && !disk.cdrom
                        && !disk.id.is_empty()
                        && !disk.dev_path.is_empty();
                    ui.monospace(&disk.dev_path);
                    ui.label(&disk.size_pretty);
                    ui.label(disk.model.as_deref().unwrap_or("Unknown"));
                    ui.label(disk.transport.as_deref().unwrap_or("Unknown"));
                    let status = if disk.readonly {
                        "Read-only"
                    } else if disk.cdrom {
                        "Optical"
                    } else if disk.id.is_empty() || disk.dev_path.is_empty() {
                        "Incomplete identity"
                    } else {
                        "Installable"
                    };
                    ui.colored_label(if installable { HEALTHY } else { WARNING }, status);
                    if ui
                        .add_enabled(installable, egui::Button::new("Select disk"))
                        .clicked()
                    {
                        selected_device = Some(disk.dev_path.clone());
                    }
                    ui.end_row();
                }
            });

        if snapshot.disks.is_empty() {
            ui.colored_label(
                ERROR,
                "Talos returned no disks, so no installation target can be selected.",
            );
        }

        ui.add_space(6.0);
        ui.label(RichText::new("Talos volumes").strong());
        match &snapshot.volumes {
            SourceAvailability::Available(volumes) if volumes.is_empty() => {
                ui.label(RichText::new("Talos reported no volume status resources.").color(MUTED));
            }
            SourceAvailability::Available(volumes) => {
                egui::Grid::new("maintenance_volumes")
                    .num_columns(5)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Volume").strong());
                        ui.label(RichText::new("Phase").strong());
                        ui.label(RichText::new("Size").strong());
                        ui.label(RichText::new("Filesystem").strong());
                        ui.label(RichText::new("Encryption").strong());
                        ui.end_row();
                        for volume in volumes {
                            ui.monospace(&volume.id);
                            ui.label(&volume.phase);
                            ui.label(&volume.size);
                            ui.label(volume.filesystem.as_deref().unwrap_or("Unknown"));
                            ui.label(
                                volume
                                    .encryption_provider
                                    .as_deref()
                                    .unwrap_or("None reported"),
                            );
                            ui.end_row();
                        }
                    });
            }
            SourceAvailability::Unavailable { reason } => {
                ui.colored_label(WARNING, format!("Volume status unavailable: {reason}"));
            }
        }

        if let Some(device_path) = selected_device {
            match InstallTarget::select(&snapshot.disks, &device_path) {
                Ok(target) => {
                    self.queue_reduction(MaintenanceEvent::InstallTargetSelected(target), None, ctx)
                }
                Err(error) => self.local_error = Some(error.to_string()),
            }
        }
        self.draw_cancel_button(ui, ctx);
    }

    fn draw_configuring(
        &mut self,
        ui: &mut egui::Ui,
        session: &BootstrapSession,
        endpoint: &str,
        ctx: &egui::Context,
    ) {
        ui.heading("3. Generate configuration for the selected disk");
        self.draw_plan_summary(ui, session);
        let Some(target) = session.install_target.clone() else {
            ui.colored_label(
                ERROR,
                "No validated install target is present in this session.",
            );
            return;
        };
        ui.colored_label(
            WARNING,
            format!(
                "The generated {} must set machine.install.disk to {}. No disk is inferred or omitted.",
                session.plan.role.configuration_filename(),
                target.device_path()
            ),
        );
        ui.add_space(6.0);
        if ui
            .button(format!(
                "Generate configuration for {}",
                target.device_path()
            ))
            .clicked()
        {
            let action = MaintenanceAction::GenerateConfiguration(ConfigurationGenerationRequest {
                plan: session.plan.clone(),
                install_target: target,
            });
            self.queue_reduction(
                MaintenanceEvent::ConfigurationGenerationStarted,
                Some(action),
                ctx,
            );
        }
        ui.horizontal(|ui| {
            if ui.button("Return to bootstrap settings").clicked() {
                self.session = None;
                self.local_error = None;
                self.apply_confirmed = false;
                self.bootstrap_confirmed = false;
                ctx.request_repaint();
            }
            self.draw_cancel_button(ui, ctx);
        });
        ui.label(RichText::new(format!("Maintenance endpoint remains {endpoint}.")).color(MUTED));
    }

    fn draw_generating(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.spinner();
        ui.label("Generating a target-aware machine configuration on a Tokio worker…");
        ui.label("The worker passes the validated machine.install.disk value to the configuration generator.");
        self.draw_cancel_button(ui, ctx);
    }

    fn draw_configuration_ready(
        &mut self,
        ui: &mut egui::Ui,
        session: &BootstrapSession,
        ctx: &egui::Context,
    ) {
        ui.heading("4. Review generated configuration");
        let Some(configuration) = &session.generated_configuration else {
            ui.colored_label(
                ERROR,
                "The generator completed without a configuration snapshot.",
            );
            return;
        };
        egui::Grid::new("generated_configuration_review")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Maintenance endpoint");
                ui.monospace(session.plan.endpoint.to_string());
                ui.end_row();
                ui.label("Machine configuration");
                ui.monospace(configuration.machine_config_path.display().to_string());
                ui.end_row();
                ui.label("Talos configuration");
                ui.monospace(configuration.talosconfig_path.display().to_string());
                ui.end_row();
                ui.label("Install disk");
                ui.monospace(configuration.install_target.device_path());
                ui.end_row();
            });
        ui.colored_label(
            WARNING,
            "Applying this configuration is destructive. A separate confirmation presents the exact endpoint, disk, and file path before the insecure apply request is started.",
        );
        if ui.button("Review and confirm insecure apply").clicked() {
            match ConfigurationApplicationRequest::new(
                session.plan.endpoint.clone(),
                configuration.machine_config_path.clone(),
                configuration.install_target.clone(),
            ) {
                Ok(request) => self.queue_reduction(
                    MaintenanceEvent::ConfigurationApplicationRequested(request),
                    None,
                    ctx,
                ),
                Err(error) => self.local_error = Some(error.to_string()),
            }
        }
        self.draw_cancel_button(ui, ctx);
    }

    fn draw_apply_confirmation(
        &mut self,
        ui: &mut egui::Ui,
        session: &BootstrapSession,
        ctx: &egui::Context,
    ) {
        ui.heading("5. Explicitly confirm insecure configuration apply");
        let Some(confirmation) = session.pending_confirmation.clone() else {
            ui.colored_label(
                ERROR,
                "No reviewed configuration apply request is available.",
            );
            return;
        };
        ui.colored_label(
            ERROR,
            "This sends an insecure Talos apply request and may install to the selected disk.",
        );
        egui::Grid::new("apply_confirmation")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Endpoint receiving insecure apply");
                ui.monospace(confirmation.endpoint().to_string());
                ui.end_row();
                ui.label("Configuration file");
                ui.monospace(confirmation.config_path().display().to_string());
                ui.end_row();
                ui.label("Selected install disk");
                ui.monospace(confirmation.install_target().device_path());
                ui.end_row();
            });
        ui.checkbox(
            &mut self.apply_confirmed,
            format!(
                "I confirm applying {} to {} for installation on {}.",
                confirmation.config_path().display(),
                confirmation.endpoint(),
                confirmation.install_target().device_path()
            ),
        );
        if ui
            .add_enabled(
                self.apply_confirmed,
                egui::Button::new("Apply reviewed configuration insecurely"),
            )
            .clicked()
        {
            self.queue_reduction(
                MaintenanceEvent::ConfigurationApplicationConfirmed(confirmation.clone()),
                Some(MaintenanceAction::ApplyConfiguration(confirmation)),
                ctx,
            );
        }
        self.draw_cancel_button(ui, ctx);
    }

    fn draw_applying(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.spinner();
        ui.label("Applying the reviewed configuration through the insecure Talos API…");
        self.draw_cancel_button(ui, ctx);
    }

    fn draw_waiting_for_talos(
        &mut self,
        ui: &mut egui::Ui,
        session: &BootstrapSession,
        ctx: &egui::Context,
    ) {
        ui.heading("6. Wait for the secure Talos API after reboot");
        ui.label(format!(
            "Readiness poll {} targets secure Talos API endpoint {} using context {}.",
            session.poll_attempts.saturating_add(1),
            session.plan.authenticated_target.node,
            session.plan.authenticated_target.talos_context
        ));
        self.draw_readiness_evidence(ui, session.latest_readiness.as_ref());
        self.draw_cancel_button(ui, ctx);
    }

    fn draw_bootstrap_confirmation(
        &mut self,
        ui: &mut egui::Ui,
        session: &BootstrapSession,
        ctx: &egui::Context,
    ) {
        ui.heading("7. Explicitly confirm cluster bootstrap");
        ui.colored_label(
            WARNING,
            "Secure Talos API evidence is available. Bootstrap has not been run yet.",
        );
        egui::Grid::new("bootstrap_confirmation")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Exact Talos context");
                ui.monospace(&session.plan.authenticated_target.talos_context);
                ui.end_row();
                ui.label("Exact Talos endpoint");
                ui.monospace(session.plan.authenticated_target.node.to_string());
                ui.end_row();
                ui.label("Talos configuration");
                ui.monospace(session.plan.paths.talosconfig_path.display().to_string());
                ui.end_row();
            });
        self.draw_readiness_evidence(ui, session.latest_readiness.as_ref());
        ui.checkbox(
            &mut self.bootstrap_confirmed,
            format!(
                "I confirm talosctl bootstrap for context {} at {}.",
                session.plan.authenticated_target.talos_context,
                session.plan.authenticated_target.node
            ),
        );
        if ui
            .add_enabled(
                self.bootstrap_confirmed,
                egui::Button::new("Run talosctl bootstrap"),
            )
            .clicked()
        {
            let command = BootstrapCommand {
                paths: session.plan.paths.clone(),
                target: session.plan.authenticated_target.clone(),
            };
            self.queue_reduction(
                MaintenanceEvent::BootstrapStarted,
                Some(MaintenanceAction::Bootstrap(command)),
                ctx,
            );
        }
        self.draw_cancel_button(ui, ctx);
    }

    fn draw_bootstrapping(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.spinner();
        ui.label(
            "Running talosctl bootstrap against the explicitly confirmed context and endpoint…",
        );
        self.draw_cancel_button(ui, ctx);
    }

    fn draw_waiting_for_kubernetes(
        &mut self,
        ui: &mut egui::Ui,
        session: &BootstrapSession,
        ctx: &egui::Context,
    ) {
        ui.heading("8. Wait for etcd and Kubernetes API readiness");
        ui.label(format!(
            "Readiness poll {} checks the secure Talos Version API, Talos etcd status, and Kubernetes Node API.",
            session.poll_attempts.saturating_add(1)
        ));
        self.draw_readiness_evidence(ui, session.latest_readiness.as_ref());
        self.draw_cancel_button(ui, ctx);
    }

    fn draw_complete(
        &mut self,
        ui: &mut egui::Ui,
        session: &BootstrapSession,
        endpoint: &str,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        ui.heading("Bootstrap complete");
        ui.colored_label(
            HEALTHY,
            "Secure Talos, etcd, and Kubernetes API readiness evidence has been collected.",
        );
        self.draw_readiness_evidence(ui, session.latest_readiness.as_ref());
        self.draw_terminal_actions(
            ui,
            endpoint,
            runtime,
            ctx,
            "Start another bootstrap session only after reviewing the selected target.",
        );
    }

    fn draw_terminal_actions(
        &mut self,
        ui: &mut egui::Ui,
        endpoint: &str,
        runtime: &Handle,
        ctx: &egui::Context,
        message: &str,
    ) {
        ui.colored_label(
            if self
                .session
                .as_ref()
                .is_some_and(|session| matches!(&session.phase, BootstrapPhase::Failed(_)))
            {
                ERROR
            } else {
                WARNING
            },
            message,
        );
        ui.horizontal(|ui| {
            if ui
                .button("Retry from insecure maintenance discovery")
                .clicked()
            {
                self.start_session(endpoint, runtime, ctx);
            }
            if ui.button("Edit bootstrap settings").clicked() {
                self.session = None;
                self.local_error = None;
                self.apply_confirmed = false;
                self.bootstrap_confirmed = false;
                ctx.request_repaint();
            }
        });
    }

    fn draw_cancel_button(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if ui.button("Cancel maintenance workflow").clicked() {
            self.cancel_workflow();
            ctx.request_repaint();
        }
    }

    fn draw_plan_summary(&self, ui: &mut egui::Ui, session: &BootstrapSession) {
        egui::Grid::new("bootstrap_plan_summary")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Cluster name");
                ui.monospace(&session.plan.cluster_name);
                ui.end_row();
                ui.label("Kubernetes API endpoint");
                ui.monospace(&session.plan.kubernetes_endpoint);
                ui.end_row();
                ui.label("Output directory");
                ui.monospace(session.plan.output_dir.display().to_string());
                ui.end_row();
                ui.label("Talos context");
                ui.monospace(&session.plan.authenticated_target.talos_context);
                ui.end_row();
                ui.label("Authenticated node");
                ui.monospace(session.plan.authenticated_target.node.to_string());
                ui.end_row();
            });
    }

    fn draw_readiness_evidence(
        &self,
        ui: &mut egui::Ui,
        evidence: Option<&BootstrapReadinessEvidence>,
    ) {
        ui.add_space(8.0);
        ui.label(RichText::new("API-backed readiness evidence").strong());
        let Some(evidence) = evidence else {
            ui.label(RichText::new("No readiness poll has completed yet.").color(MUTED));
            return;
        };

        ui.label(format!(
            "Target: context {} / node {}",
            evidence.target.talos_context, evidence.target.node
        ));
        match &evidence.talos {
            TalosReadinessEvidence::Available { versions, etcd } => {
                ui.colored_label(HEALTHY, "Secure Talos Version API responded.");
                if versions.is_empty() {
                    ui.colored_label(WARNING, "Talos returned no version entries.");
                } else {
                    for version in versions {
                        ui.label(format!("Talos node {}: {}", version.node, version.version));
                    }
                }
                match etcd {
                    EtcdReadinessEvidence::Ready { responding_members } => {
                        ui.colored_label(
                            HEALTHY,
                            format!("etcd ready: {responding_members} responding member(s)."),
                        );
                    }
                    EtcdReadinessEvidence::NotReady {
                        responding_members,
                        errors,
                    } => {
                        ui.colored_label(
                            WARNING,
                            format!("etcd not ready: {responding_members} responding member(s)."),
                        );
                        if errors.is_empty() {
                            ui.label(
                                RichText::new("Talos etcd status returned no member errors.")
                                    .color(MUTED),
                            );
                        } else {
                            for error in errors {
                                ui.colored_label(WARNING, format!("etcd: {error}"));
                            }
                        }
                    }
                    EtcdReadinessEvidence::Unavailable { reason } => {
                        ui.colored_label(WARNING, format!("etcd evidence unavailable: {reason}"));
                    }
                }
            }
            TalosReadinessEvidence::Unavailable { reason } => {
                ui.colored_label(WARNING, format!("Secure Talos API unavailable: {reason}"));
            }
        }

        match &evidence.kubernetes {
            KubernetesReadinessEvidence::Ready {
                credentials_source,
                nodes,
            } => {
                ui.colored_label(
                    HEALTHY,
                    format!(
                        "Kubernetes Node API ready using {}.",
                        credentials_source_label(credentials_source)
                    ),
                );
                self.draw_kubernetes_nodes(ui, nodes);
            }
            KubernetesReadinessEvidence::NotReady {
                credentials_source,
                nodes,
            } => {
                ui.colored_label(
                    WARNING,
                    format!(
                        "Kubernetes Node API responded but readiness is incomplete using {}.",
                        credentials_source_label(credentials_source)
                    ),
                );
                self.draw_kubernetes_nodes(ui, nodes);
            }
            KubernetesReadinessEvidence::Unavailable {
                credentials_source,
                reason,
            } => {
                ui.colored_label(
                    WARNING,
                    format!(
                        "Kubernetes evidence unavailable using {}: {reason}",
                        credentials_source_label(credentials_source)
                    ),
                );
            }
            KubernetesReadinessEvidence::NotChecked => {
                ui.label(
                    RichText::new(
                        "Kubernetes was not queried because secure Talos evidence is unavailable.",
                    )
                    .color(MUTED),
                );
            }
        }
    }

    fn draw_kubernetes_nodes(
        &self,
        ui: &mut egui::Ui,
        nodes: &[talos_pilot_core::maintenance::KubernetesNodeEvidence],
    ) {
        if nodes.is_empty() {
            ui.colored_label(WARNING, "Kubernetes returned no Node resources.");
            return;
        }
        egui::Grid::new("bootstrap_kubernetes_nodes")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label(RichText::new("Kubernetes node").strong());
                ui.label(RichText::new("Ready").strong());
                ui.end_row();
                for node in nodes {
                    ui.monospace(&node.name);
                    ui.colored_label(
                        if node.ready { HEALTHY } else { WARNING },
                        if node.ready { "Ready" } else { "Not Ready" },
                    );
                    ui.end_row();
                }
            });
    }

    fn queue_reduction(
        &mut self,
        event: MaintenanceEvent,
        follow_up: Option<MaintenanceAction>,
        ctx: &egui::Context,
    ) {
        let _ = self.event_tx.send(BootstrapEvent::Reduce {
            epoch: self.epoch,
            event,
            follow_up,
        });
        ctx.request_repaint();
    }

    fn cancel_workflow(&mut self) {
        if let Some(work) = self.active_work.take() {
            work.cancellation.store(true, Ordering::Release);
        }
        // Invalidate a queued local reduction too: it may have a follow-up
        // action that has not reached `start_worker` yet.
        self.epoch = self.epoch.wrapping_add(1);
        self.next_poll_at = None;
        let _ = self.event_tx.send(BootstrapEvent::Reduce {
            epoch: self.epoch,
            event: MaintenanceEvent::Cancelled,
            follow_up: None,
        });
    }

    fn drain_events(&mut self, runtime: &Handle, ctx: &egui::Context) {
        while let Ok(item) = self.event_rx.try_recv() {
            match item {
                BootstrapEvent::Reduce {
                    epoch,
                    event,
                    follow_up,
                } if epoch == self.epoch => {
                    if self.reduce_event(event)
                        && let Some(action) = follow_up
                    {
                        self.start_worker(action, runtime, ctx);
                    }
                }
                BootstrapEvent::Completed {
                    epoch,
                    work_id,
                    event,
                } if epoch == self.epoch
                    && self
                        .active_work
                        .as_ref()
                        .is_some_and(|work| work.id == work_id) =>
                {
                    self.active_work = None;
                    self.reduce_event(event);
                }
                _ => {
                    // The event belonged to a prior endpoint, retry, route visit,
                    // or cancelled action and must not reach the current session.
                }
            }
        }
    }

    fn reduce_event(&mut self, event: MaintenanceEvent) -> bool {
        let Some(current) = self.session.as_ref() else {
            return false;
        };
        let was_waiting = is_readiness_wait(&current.phase);
        match current.reduce(event) {
            Ok(next) => {
                let is_waiting = is_readiness_wait(&next.phase);
                self.next_poll_at = is_waiting.then(|| {
                    let delay = if was_waiting {
                        BOOTSTRAP_POLL_INTERVAL
                    } else {
                        std::time::Duration::ZERO
                    };
                    Instant::now() + delay
                });
                self.session = Some(next);
                self.local_error = None;
                true
            }
            Err(error) => {
                self.local_error = Some(format!("Unable to advance bootstrap workflow: {error}"));
                false
            }
        }
    }

    fn maybe_start_readiness_poll(&mut self, runtime: &Handle, ctx: &egui::Context) {
        if self.active_work.is_some() {
            return;
        }
        let Some(next_poll) = self.next_poll_at else {
            return;
        };
        let now = Instant::now();
        if now < next_poll {
            ctx.request_repaint_after(next_poll.duration_since(now));
            return;
        }
        let Some(action) = self
            .session
            .as_ref()
            .and_then(BootstrapSession::polling_action)
        else {
            self.next_poll_at = None;
            return;
        };
        self.next_poll_at = None;
        self.start_worker(action, runtime, ctx);
    }

    fn start_worker(&mut self, action: MaintenanceAction, runtime: &Handle, ctx: &egui::Context) {
        if self.active_work.is_some() {
            self.local_error = Some("A maintenance action is already in progress.".to_string());
            return;
        }

        self.next_work_id = self.next_work_id.wrapping_add(1);
        let work_id = self.next_work_id;
        let epoch = self.epoch;
        let cancellation = Arc::new(AtomicBool::new(false));
        self.active_work = Some(ActiveWork {
            id: work_id,
            cancellation: cancellation.clone(),
        });
        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        runtime.spawn(async move {
            let event =
                execute_maintenance_action(action, || cancellation.load(Ordering::Acquire)).await;
            let _ = event_tx.send(BootstrapEvent::Completed {
                epoch,
                work_id,
                event,
            });
            repaint.request_repaint();
        });
    }

    fn invalidate_active_work(&mut self) {
        if let Some(work) = self.active_work.take() {
            work.cancellation.store(true, Ordering::Release);
        }
        self.epoch = self.epoch.wrapping_add(1);
        self.next_poll_at = None;
    }
}

impl Drop for BootstrapScreen {
    fn drop(&mut self) {
        if let Some(work) = self.active_work.take() {
            work.cancellation.store(true, Ordering::Release);
        }
    }
}

#[derive(Clone, Copy)]
enum PathPicker {
    File,
    Directory,
}

#[derive(Clone, Copy)]
enum PathSlot {
    Talosconfig,
    OutputDirectory,
    Kubeconfig,
}

fn is_readiness_wait(phase: &BootstrapPhase) -> bool {
    matches!(
        phase,
        BootstrapPhase::WaitingForTalos | BootstrapPhase::WaitingForKubernetes
    )
}

fn phase_label(phase: &BootstrapPhase) -> &'static str {
    match phase {
        BootstrapPhase::CollectingInsecureData => "Loading insecure maintenance data",
        BootstrapPhase::SelectingInstallTarget => "Select install target",
        BootstrapPhase::Configuring => "Generate configuration",
        BootstrapPhase::GeneratingConfiguration => "Generating configuration",
        BootstrapPhase::ConfigurationReady => "Review configuration",
        BootstrapPhase::AwaitingApplyConfirmation => "Confirm insecure apply",
        BootstrapPhase::ApplyingConfiguration => "Applying configuration",
        BootstrapPhase::WaitingForTalos => "Waiting for secure Talos",
        BootstrapPhase::ReadyToBootstrap => "Confirm bootstrap",
        BootstrapPhase::Bootstrapping => "Bootstrapping",
        BootstrapPhase::WaitingForKubernetes => "Waiting for etcd and Kubernetes",
        BootstrapPhase::Complete => "Complete",
        BootstrapPhase::Cancelled => "Cancelled",
        BootstrapPhase::Failed(_) => "Failed",
    }
}

fn credentials_source_label(source: &KubernetesCredentialsSource) -> String {
    match source {
        KubernetesCredentialsSource::SelectedPath(path) => {
            format!("selected kubeconfig {}", path.display())
        }
        KubernetesCredentialsSource::TalosApi => "authenticated Talos API kubeconfig".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use talos_pilot_core::maintenance::{
        AuthenticatedTarget, BootstrapPaths, BootstrapPhase, BootstrapPlan, BootstrapSession,
        MachineRole, MaintenanceEndpoint, MaintenanceEvent,
    };

    #[test]
    fn session_reduces_cancelled_event_without_worker_state() {
        let endpoint = MaintenanceEndpoint::parse("192.0.2.10").unwrap();
        let plan = BootstrapPlan::new(
            endpoint.clone(),
            "bootstrap-test",
            endpoint.kubernetes_api_url(),
            PathBuf::from("generated"),
            MachineRole::ControlPlane,
            BootstrapPaths {
                talosconfig_path: PathBuf::from("generated/talosconfig"),
                kubeconfig_path: None,
            },
            AuthenticatedTarget::new("bootstrap-test", endpoint, None).unwrap(),
        )
        .unwrap();

        let session = BootstrapSession::new(plan);
        let cancelled = session.reduce(MaintenanceEvent::Cancelled).unwrap();
        assert_eq!(cancelled.phase, BootstrapPhase::Cancelled);
        assert_eq!(
            cancelled.last_message.as_deref(),
            Some("Maintenance operation cancelled")
        );
    }
}
