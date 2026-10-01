//! Native egui desktop interface for Talos cluster monitoring.
//!
//! The UI thread owns egui state. Talos and Kubernetes work runs on the caller's
//! Tokio runtime and sends completed snapshots back over a channel.

mod screens;

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use color_eyre::{Result, eyre::eyre};
use eframe::egui::{self, Color32, RichText};
use screens::{Route, ScreenTarget};
use talos_pilot_core::cluster_overview::{
    ClusterConnectionStatus, ClusterOverview, ClusterOverviewCollector,
};
use tokio::{runtime::Handle, sync::mpsc};

const NODE_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// Settings supplied by the command-line launcher.
#[derive(Debug, Clone)]
pub struct GuiOptions {
    /// Optional talosconfig path.
    pub config_path: Option<PathBuf>,
    /// Optional Talos context filter.
    pub context: Option<String>,
    /// Maintenance-mode endpoint selected by the launcher for the Bootstrap
    /// route. A present endpoint enables the native insecure bootstrap flow.
    pub maintenance_endpoint: Option<String>,
}

/// Opens the native desktop dashboard.
///
/// The supplied Tokio runtime must outlive this blocking call. `eframe` owns the
/// native event loop while the runtime executes Talos requests on worker threads.
pub fn run(options: GuiOptions, runtime: Handle) -> Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("talos-pilot")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "talos-pilot",
        native_options,
        Box::new(move |creation_context| {
            Ok(Box::new(GuiApp::new(creation_context, options, runtime)))
        }),
    )
    .map_err(|error| eyre!(error.to_string()))
}

enum GuiEvent {
    Loaded {
        request_id: u64,
        result: Result<Vec<ClusterOverview>, String>,
    },
    Refreshed {
        request_id: u64,
        clusters: Vec<ClusterOverview>,
    },
    NodeRefreshed {
        request_id: u64,
        context: String,
        cluster: Box<ClusterOverview>,
    },
}

#[derive(Clone, Copy)]
enum NodeHealth {
    Healthy,
    Warning,
    Error,
}

impl NodeHealth {
    fn color(self) -> Color32 {
        match self {
            Self::Healthy => Color32::from_rgb(76, 175, 80),
            Self::Warning => Color32::from_rgb(255, 193, 7),
            Self::Error => Color32::from_rgb(244, 67, 54),
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::Healthy => "●",
            Self::Warning => "◐",
            Self::Error => "✗",
        }
    }
}

struct GuiApp {
    runtime: Handle,
    collector: ClusterOverviewCollector,
    config_path: Option<PathBuf>,
    maintenance_endpoint: Option<String>,
    event_tx: mpsc::UnboundedSender<GuiEvent>,
    event_rx: mpsc::UnboundedReceiver<GuiEvent>,
    clusters: Vec<ClusterOverview>,
    selected_cluster: usize,
    selected_node: Option<String>,
    request_id: u64,
    request_in_flight: bool,
    request_error: Option<String>,
    auto_refresh: bool,
    last_completed_refresh: Option<Instant>,
    route: Route,
    logs_screen: screens::logs::LogsScreen,
    processes_screen: screens::processes::ProcessScreen,
    network_screen: screens::network::NetworkScreen,
    etcd_screen: screens::etcd::EtcdScreen,
    workloads_screen: screens::workloads::WorkloadsScreen,
    diagnostics_screen: screens::diagnostics::DiagnosticsScreen,
    security_screen: screens::security_lifecycle::SecurityScreen,
    lifecycle_screen: screens::security_lifecycle::LifecycleScreen,
    operations_screen: screens::operations::OperationsScreen,
    rolling_operations_screen: screens::operations::RollingOperationsScreen,
    audit_screen: screens::operations::AuditScreen,
    bootstrap_screen: screens::bootstrap::BootstrapScreen,
}

impl GuiApp {
    fn new(
        creation_context: &eframe::CreationContext<'_>,
        options: GuiOptions,
        runtime: Handle,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let config_path = options.config_path.clone();
        let maintenance_endpoint = options.maintenance_endpoint.clone();
        let collector = ClusterOverviewCollector::new(options.config_path, options.context);
        let mut app = Self {
            runtime,
            collector,
            config_path,
            maintenance_endpoint: maintenance_endpoint.clone(),
            event_tx,
            event_rx,
            clusters: Vec::new(),
            selected_cluster: 0,
            selected_node: None,
            request_id: 0,
            request_in_flight: false,
            request_error: None,
            auto_refresh: true,
            last_completed_refresh: None,
            route: if maintenance_endpoint.is_some() {
                Route::Bootstrap
            } else {
                Route::Overview
            },
            logs_screen: Default::default(),
            processes_screen: Default::default(),
            network_screen: Default::default(),
            etcd_screen: Default::default(),
            workloads_screen: Default::default(),
            diagnostics_screen: Default::default(),
            security_screen: Default::default(),
            lifecycle_screen: Default::default(),
            operations_screen: Default::default(),
            rolling_operations_screen: Default::default(),
            audit_screen: Default::default(),
            bootstrap_screen: Default::default(),
        };
        app.start_load(&creation_context.egui_ctx);
        app
    }

    /// Switch native routes while making streaming and long-running workers
    /// unable to apply results after their screen is no longer visible.
    fn select_route(&mut self, route: Route) {
        if self.route == route {
            return;
        }
        self.deactivate_route(self.route);
        self.route = route;
    }

    fn deactivate_route(&mut self, route: Route) {
        match route {
            Route::Overview => {}
            Route::Logs => self.logs_screen.deactivate(),
            Route::Processes => self.processes_screen.deactivate(),
            Route::Network => self.network_screen.deactivate(),
            Route::Etcd => self.etcd_screen.deactivate(),
            Route::Workloads => self.workloads_screen.deactivate(),
            Route::Diagnostics => self.diagnostics_screen.deactivate(),
            Route::Security => self.security_screen.deactivate(),
            Route::Lifecycle => self.lifecycle_screen.deactivate(),
            Route::Operations => self.operations_screen.deactivate(),
            Route::RollingOperations => self.rolling_operations_screen.deactivate(),
            Route::Audit => self.audit_screen.deactivate(),
            Route::Bootstrap => self.bootstrap_screen.deactivate(),
        }
    }

    fn deactivate_all_feature_screens(&mut self) {
        for route in Route::ALL {
            self.deactivate_route(route);
        }
    }

    fn start_load(&mut self, ctx: &egui::Context) {
        if self.request_in_flight {
            return;
        }

        self.request_in_flight = true;
        self.request_error = None;
        self.request_id = self.request_id.wrapping_add(1);
        let request_id = self.request_id;
        let collector = self.collector.clone();
        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();

        self.runtime.spawn(async move {
            let result = match collector.connect().await {
                Ok(mut clusters) => {
                    for cluster in &mut clusters {
                        collector.refresh(cluster).await;
                    }
                    Ok(clusters)
                }
                Err(error) => Err(error.to_string()),
            };
            let _ = event_tx.send(GuiEvent::Loaded { request_id, result });
            repaint.request_repaint();
        });
    }

    fn start_full_refresh(&mut self, ctx: &egui::Context) {
        if self.request_in_flight {
            return;
        }
        if self.clusters.is_empty() {
            self.start_load(ctx);
            return;
        }

        self.request_in_flight = true;
        self.request_error = None;
        self.request_id = self.request_id.wrapping_add(1);
        let request_id = self.request_id;
        let collector = self.collector.clone();
        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        let mut clusters = self.clusters.clone();

        self.runtime.spawn(async move {
            for cluster in &mut clusters {
                collector.refresh(cluster).await;
            }
            let _ = event_tx.send(GuiEvent::Refreshed {
                request_id,
                clusters,
            });
            repaint.request_repaint();
        });
    }

    fn start_selected_node_refresh(&mut self, ctx: &egui::Context) {
        if self.request_in_flight {
            return;
        }
        let Some(node_name) = self.selected_node.clone() else {
            return;
        };
        let Some(mut cluster) = self.clusters.get(self.selected_cluster).cloned() else {
            return;
        };

        self.request_in_flight = true;
        self.request_id = self.request_id.wrapping_add(1);
        let request_id = self.request_id;
        let context = cluster.name.clone();
        let collector = self.collector.clone();
        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();

        self.runtime.spawn(async move {
            collector.refresh_node(&mut cluster, &node_name).await;
            let _ = event_tx.send(GuiEvent::NodeRefreshed {
                request_id,
                context,
                cluster: Box::new(cluster),
            });
            repaint.request_repaint();
        });
    }

    fn drain_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                GuiEvent::Loaded { request_id, result } if request_id == self.request_id => {
                    self.request_in_flight = false;
                    match result {
                        Ok(clusters) => {
                            self.replace_clusters(clusters);
                            self.request_error = None;
                        }
                        Err(error) => {
                            self.clusters.clear();
                            self.selected_cluster = 0;
                            self.selected_node = None;
                            self.request_error = Some(error);
                        }
                    }
                    self.last_completed_refresh = Some(Instant::now());
                }
                GuiEvent::Refreshed {
                    request_id,
                    clusters,
                } if request_id == self.request_id => {
                    self.request_in_flight = false;
                    self.replace_clusters(clusters);
                    self.last_completed_refresh = Some(Instant::now());
                }
                GuiEvent::NodeRefreshed {
                    request_id,
                    context,
                    cluster,
                } if request_id == self.request_id => {
                    self.request_in_flight = false;
                    if let Some(index) = self
                        .clusters
                        .iter()
                        .position(|existing| existing.name == context)
                    {
                        self.clusters[index] = *cluster;
                    }
                    self.reconcile_selection();
                    self.last_completed_refresh = Some(Instant::now());
                }
                _ => {}
            }
        }
    }

    fn reconcile_selection(&mut self) {
        if self.clusters.is_empty() {
            self.selected_cluster = 0;
            self.selected_node = None;
            return;
        }

        self.selected_cluster = self.selected_cluster.min(self.clusters.len() - 1);
        if let Some(node_name) = self.selected_node.as_deref()
            && !self.clusters[self.selected_cluster]
                .versions
                .iter()
                .any(|node| node.node == node_name)
        {
            self.selected_node = None;
        }
    }

    fn replace_clusters(&mut self, clusters: Vec<ClusterOverview>) {
        let selected_context = self
            .clusters
            .get(self.selected_cluster)
            .map(|cluster| cluster.name.clone());
        self.clusters = clusters;
        self.selected_cluster = Self::selected_cluster_index(
            &self.clusters,
            selected_context.as_deref(),
            self.selected_cluster,
        );
        self.reconcile_selection();
    }

    fn selected_cluster_index(
        clusters: &[ClusterOverview],
        selected_context: Option<&str>,
        fallback_index: usize,
    ) -> usize {
        selected_context
            .and_then(|context| clusters.iter().position(|cluster| cluster.name == context))
            .unwrap_or_else(|| fallback_index.min(clusters.len().saturating_sub(1)))
    }

    fn auto_refresh_due(&self) -> bool {
        self.auto_refresh
            && self.selected_node.is_some()
            && !self.request_in_flight
            && self
                .last_completed_refresh
                .is_some_and(|last| last.elapsed() >= NODE_REFRESH_INTERVAL)
    }

    fn choose_config(&mut self, ctx: &egui::Context) {
        let mut dialog = rfd::FileDialog::new().set_title("Choose talosconfig");
        if let Some(directory) = self.config_path.as_ref().and_then(|path| path.parent()) {
            dialog = dialog.set_directory(directory);
        }

        let Some(path) = dialog.pick_file() else {
            return;
        };

        self.collector.set_config_path(Some(path.clone()));
        self.config_path = Some(path);
        self.deactivate_all_feature_screens();
        self.clusters.clear();
        self.selected_cluster = 0;
        self.selected_node = None;
        self.last_completed_refresh = None;
        self.start_load(ctx);
    }

    fn draw_top_bar(&mut self, ctx: &egui::Context) {
        let mut choose_config = false;
        let mut refresh = false;
        let mut reload = false;
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("talos-pilot");
                ui.separator();
                ui.label(format!("Desktop • {}", self.route.label()));
                ui.separator();
                if self.request_in_flight {
                    ui.spinner();
                    ui.label("Refreshing cluster state…");
                } else {
                    ui.label(self.overall_status());
                }
                let config_path = self
                    .config_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "~/.talos/config".to_string());
                let config_name = self
                    .config_path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "default talosconfig".to_string());
                ui.label(RichText::new(format!("Config: {config_name}")).weak())
                    .on_hover_text(config_path);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    choose_config = ui
                        .add_enabled(
                            !self.request_in_flight,
                            egui::Button::new("Choose talosconfig…"),
                        )
                        .clicked();
                    reload = ui
                        .add_enabled(
                            !self.request_in_flight,
                            egui::Button::new("Reload contexts"),
                        )
                        .clicked();
                    refresh = ui
                        .add_enabled(!self.request_in_flight, egui::Button::new("Refresh"))
                        .clicked();
                    ui.checkbox(&mut self.auto_refresh, "Auto-refresh selected node");
                });
            });
        });

        if choose_config {
            self.choose_config(ctx);
        } else if reload {
            self.start_load(ctx);
        } else if refresh {
            self.start_full_refresh(ctx);
        }
    }

    fn draw_cluster_sidebar(&mut self, ctx: &egui::Context) {
        let mut next_selection = None;
        let mut next_route = None;
        egui::SidePanel::left("clusters")
            .resizable(true)
            .min_width(250.0)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.heading("Navigation");
                for route in Route::ALL {
                    if ui
                        .selectable_label(self.route == route, route.label())
                        .clicked()
                    {
                        next_route = Some(route);
                    }
                }
                ui.separator();
                ui.heading("Contexts and nodes");
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (index, cluster) in self.clusters.iter().enumerate() {
                        let (status, color) = Self::connection_badge(cluster);
                        let selected =
                            index == self.selected_cluster && self.selected_node.is_none();
                        let label =
                            format!("{status} {} ({})", cluster.name, cluster.versions.len());
                        if ui
                            .selectable_label(selected, RichText::new(label).color(color))
                            .clicked()
                        {
                            next_selection = Some((index, None));
                        }

                        if let Some(error) = cluster.connection.error() {
                            ui.indent(index, |ui| {
                                ui.colored_label(Color32::from_rgb(244, 67, 54), error);
                            });
                            ui.separator();
                            continue;
                        }

                        ui.indent(index, |ui| {
                            ui.collapsing("Control plane", |ui| {
                                for node in cluster
                                    .versions
                                    .iter()
                                    .filter(|node| cluster.node_is_controlplane(&node.node))
                                {
                                    if Self::node_selector(
                                        ui,
                                        cluster,
                                        &node.node,
                                        self.selected_node.as_deref(),
                                    ) {
                                        next_selection = Some((index, Some(node.node.clone())));
                                    }
                                }
                            });
                            ui.collapsing("Workers", |ui| {
                                for node in cluster
                                    .versions
                                    .iter()
                                    .filter(|node| !cluster.node_is_controlplane(&node.node))
                                {
                                    if Self::node_selector(
                                        ui,
                                        cluster,
                                        &node.node,
                                        self.selected_node.as_deref(),
                                    ) {
                                        next_selection = Some((index, Some(node.node.clone())));
                                    }
                                }
                            });
                            if let Some(warning) = &cluster.discovery_warning {
                                ui.colored_label(Color32::from_rgb(255, 193, 7), warning);
                            }
                            if let Some(warning) = &cluster.kubeconfig_warning {
                                ui.colored_label(Color32::from_rgb(255, 193, 7), warning);
                            }
                        });
                        ui.separator();
                    }
                });
            });

        if let Some(route) = next_route {
            self.select_route(route);
        }
        if let Some((cluster, node)) = next_selection {
            self.selected_cluster = cluster;
            self.selected_node = node;
        }
    }

    fn node_selector(
        ui: &mut egui::Ui,
        cluster: &ClusterOverview,
        node_name: &str,
        selected_node: Option<&str>,
    ) -> bool {
        let health = Self::node_health(cluster, node_name);
        ui.selectable_label(
            selected_node == Some(node_name),
            RichText::new(format!("{} {node_name}", health.symbol())).color(health.color()),
        )
        .clicked()
    }

    fn draw_main_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.route == Route::Bootstrap {
                self.bootstrap_screen.ui(
                    ui,
                    self.maintenance_endpoint.as_deref(),
                    self.config_path.as_ref(),
                    &self.runtime,
                    ctx,
                );
                return;
            }

            if let Some(error) = &self.request_error {
                ui.heading("Unable to load Talos contexts");
                ui.add_space(8.0);
                ui.colored_label(Color32::from_rgb(244, 67, 54), error);
                ui.add_space(8.0);
                ui.label(
                    "Check the talosconfig path, selected context, and local file permissions.",
                );
                return;
            }

            let Some(cluster) = self.clusters.get(self.selected_cluster).cloned() else {
                ui.heading("No Talos contexts found");
                ui.label("Add a context to talosconfig, then reload contexts.");
                return;
            };
            let selected_node = self.selected_node.clone();
            let config_path = self.config_path.clone();
            let runtime = self.runtime.clone();
            let target = ScreenTarget {
                cluster: &cluster,
                selected_node: selected_node.as_deref(),
                config_path: config_path.as_ref(),
            };

            match self.route {
                Route::Overview => {
                    if let Some(node_name) = selected_node.as_deref() {
                        Self::draw_node_details(ui, &cluster, node_name);
                    } else {
                        Self::draw_cluster_summary(ui, &cluster);
                    }
                }
                Route::Logs => self.logs_screen.ui(ui, &target, &runtime, ctx),
                Route::Processes => self.processes_screen.ui(ui, &target, &runtime, ctx),
                Route::Network => self.network_screen.ui(ui, &target, &runtime, ctx),
                Route::Etcd => self.etcd_screen.ui(ui, &target, &runtime, ctx),
                Route::Workloads => self.workloads_screen.ui(ui, &target, &runtime, ctx),
                Route::Diagnostics => self.diagnostics_screen.ui(ui, &target, &runtime, ctx),
                Route::Security => self.security_screen.ui(ui, &target, &runtime, ctx),
                Route::Lifecycle => self.lifecycle_screen.ui(ui, &target, &runtime, ctx),
                Route::Operations => self.operations_screen.ui(ui, &target, &runtime, ctx),
                Route::RollingOperations => self
                    .rolling_operations_screen
                    .ui(ui, &target, &runtime, ctx),
                Route::Audit => self.audit_screen.ui(ui, &target, &runtime, ctx),
                Route::Bootstrap => {}
            }
        });
    }

    fn draw_cluster_summary(ui: &mut egui::Ui, cluster: &ClusterOverview) {
        let (status, color) = Self::connection_badge(cluster);
        ui.heading(&cluster.name);
        ui.horizontal(|ui| {
            ui.colored_label(color, status);
            ui.label(format!("{} discovered node(s)", cluster.versions.len()));
            if let Some(etcd) = &cluster.etcd_summary {
                let etcd_color = if etcd.has_quorum && etcd.healthy == etcd.total {
                    Color32::from_rgb(76, 175, 80)
                } else if etcd.has_quorum {
                    Color32::from_rgb(255, 193, 7)
                } else {
                    Color32::from_rgb(244, 67, 54)
                };
                ui.separator();
                ui.colored_label(etcd_color, format!("etcd {}/{}", etcd.healthy, etcd.total));
            }
        });
        ui.separator();

        if !cluster.endpoints.is_empty() {
            ui.label(RichText::new("Configured endpoints").strong());
            for endpoint in &cluster.endpoints {
                ui.monospace(endpoint);
            }
        }

        if let Some(warning) = &cluster.discovery_warning {
            ui.add_space(8.0);
            ui.colored_label(Color32::from_rgb(255, 193, 7), warning);
        }
        if let Some(warning) = &cluster.kubeconfig_warning {
            ui.add_space(8.0);
            ui.colored_label(Color32::from_rgb(255, 193, 7), warning);
        }

        if cluster.versions.is_empty() && cluster.connection.is_connected() {
            ui.add_space(16.0);
            ui.label("Connected, but no node statistics are available yet.");
        }
    }

    fn draw_node_details(ui: &mut egui::Ui, cluster: &ClusterOverview, node_name: &str) {
        let Some(node) = cluster.versions.iter().find(|node| node.node == node_name) else {
            Self::draw_cluster_summary(ui, cluster);
            return;
        };
        let role = if cluster.node_is_controlplane(node_name) {
            "Control plane"
        } else {
            "Worker"
        };
        let health = Self::node_health(cluster, node_name);
        ui.horizontal(|ui| {
            ui.heading(node_name);
            ui.colored_label(health.color(), format!("{} {role}", health.symbol()));
        });
        if let Some(address) = cluster.node_ips.get(node_name) {
            ui.monospace(address);
        }
        ui.separator();

        egui::Grid::new("node_metrics")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Talos version");
                ui.label(&node.version);
                ui.end_row();

                if let Some(memory) = cluster
                    .memory
                    .iter()
                    .find(|memory| memory.node == node_name)
                    .and_then(|memory| memory.meminfo.as_ref())
                {
                    let used_gib =
                        (memory.mem_total - memory.mem_available) as f64 / 1024.0 / 1024.0 / 1024.0;
                    let total_gib = memory.mem_total as f64 / 1024.0 / 1024.0 / 1024.0;
                    ui.label("Memory");
                    ui.label(format!(
                        "{used_gib:.1}/{total_gib:.1} GiB ({:.0}%)",
                        memory.usage_percent()
                    ));
                    ui.end_row();
                }

                if let Some(load) = cluster.load_avg.iter().find(|load| load.node == node_name) {
                    ui.label("Load average");
                    ui.label(format!(
                        "{:.2} / {:.2} / {:.2}",
                        load.load1, load.load5, load.load15
                    ));
                    ui.end_row();
                }

                if let Some(cpu) = cluster.cpu_info.iter().find(|cpu| cpu.node == node_name) {
                    ui.label("CPU");
                    ui.label(format!("{} cores @ {:.0} MHz", cpu.cpu_count, cpu.mhz));
                    ui.end_row();
                }
            });

        let services = cluster
            .services
            .iter()
            .find(|services| services.node == node_name)
            .map(|services| services.services.as_slice())
            .unwrap_or_default();
        ui.add_space(12.0);
        ui.label(RichText::new(format!("Talos services ({})", services.len())).strong());
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt(("services", node_name))
            .show(ui, |ui| {
                egui::Grid::new(("service_grid", node_name))
                    .num_columns(3)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Health").strong());
                        ui.label(RichText::new("Service").strong());
                        ui.label(RichText::new("State").strong());
                        ui.end_row();
                        for service in services {
                            let healthy = service
                                .health
                                .as_ref()
                                .map(|health| health.healthy)
                                .unwrap_or(true);
                            let health = if healthy {
                                NodeHealth::Healthy
                            } else {
                                NodeHealth::Error
                            };
                            ui.colored_label(health.color(), health.symbol());
                            ui.monospace(&service.id);
                            ui.label(&service.state);
                            ui.end_row();
                        }
                    });
            });
    }

    fn node_health(cluster: &ClusterOverview, node_name: &str) -> NodeHealth {
        let services_healthy = cluster
            .services
            .iter()
            .find(|services| services.node == node_name)
            .map(|services| {
                services.services.iter().all(|service| {
                    service
                        .health
                        .as_ref()
                        .map(|health| health.healthy)
                        .unwrap_or(true)
                })
            })
            .unwrap_or(true);
        if !services_healthy {
            return NodeHealth::Error;
        }

        let memory_percent = cluster
            .memory
            .iter()
            .find(|memory| memory.node == node_name)
            .and_then(|memory| memory.meminfo.as_ref())
            .map(|memory| memory.usage_percent())
            .unwrap_or_default();
        if memory_percent >= 90.0 {
            NodeHealth::Warning
        } else {
            NodeHealth::Healthy
        }
    }

    fn connection_badge(cluster: &ClusterOverview) -> (&'static str, Color32) {
        match &cluster.connection {
            ClusterConnectionStatus::Connected => ("● Connected", Color32::from_rgb(76, 175, 80)),
            ClusterConnectionStatus::Disconnected => ("○ Disconnected", Color32::GRAY),
            ClusterConnectionStatus::Unreachable(_) => {
                ("✗ Unreachable", Color32::from_rgb(244, 67, 54))
            }
        }
    }

    fn overall_status(&self) -> String {
        let connected = self
            .clusters
            .iter()
            .filter(|cluster| cluster.connection.is_connected())
            .count();
        match (connected, self.clusters.len()) {
            (_, 0) => "No contexts loaded".to_string(),
            (count, total) if count == total => format!("{count}/{total} connected"),
            (count, total) => format!("{count}/{total} connected (partial)"),
        }
    }
}

impl Drop for GuiApp {
    fn drop(&mut self) {
        self.deactivate_all_feature_screens();
    }
}

impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        if self.auto_refresh_due() {
            self.start_selected_node_refresh(ctx);
        }

        self.draw_top_bar(ctx);
        self.draw_cluster_sidebar(ctx);
        self.draw_main_panel(ctx);

        // Repaint for the auto-refresh deadline; request_repaint from workers
        // wakes immediately when a Talos request completes.
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster(name: &str) -> ClusterOverview {
        ClusterOverview {
            name: name.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn preserves_selected_context_when_reload_reorders_contexts() {
        let reloaded = vec![cluster("staging"), cluster("production")];
        assert_eq!(
            GuiApp::selected_cluster_index(&reloaded, Some("production"), 0),
            1
        );
    }
}
