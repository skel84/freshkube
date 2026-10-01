//! Native network inspection and bounded packet-capture screen.
//!
//! All node I/O happens on the application-owned Tokio runtime. Workers return
//! owned, target-tagged events; this type applies them only from egui's thread.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, RichText};
use talos_pilot_core::{
    constants::{MAX_CAPTURE_SIZE, refresh_intervals},
    formatting::format_bytes,
    inspection::{
        InspectionTarget, KubeSpanSnapshot, NetworkInspectionRequest, NetworkInspectionSnapshot,
        NetworkSampleState, PacketCaptureRequest, collect_network_inspection,
    },
};
use tokio::{runtime::Handle, sync::mpsc, task::JoinHandle};

use crate::screens::ScreenTarget;

const WARNING_COLOR: Color32 = Color32::from_rgb(255, 193, 7);
const ERROR_COLOR: Color32 = Color32::from_rgb(244, 67, 54);
const AUTO_REFRESH: Duration = Duration::from_secs(refresh_intervals::FAST);
const SOURCE_TIMEOUT: Duration = Duration::from_secs(10);

/// A complete identity rather than a selected-row index. The context is part of
/// the identity because the same node name and address can exist in two configs.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TargetIdentity {
    context: String,
    node: InspectionTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RequestIdentity {
    serial: u64,
    target: TargetIdentity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InterfaceSort {
    Traffic,
    Errors,
}

impl Default for InterfaceSort {
    fn default() -> Self {
        Self::Traffic
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapturePhase {
    Idle,
    Starting,
    Capturing,
    Saving,
}

struct CaptureState {
    phase: CapturePhase,
    identity: Option<RequestIdentity>,
    task: Option<JoinHandle<()>>,
    save_task: Option<JoinHandle<()>>,
    bytes: Vec<u8>,
    message: Option<String>,
}

impl Default for CaptureState {
    fn default() -> Self {
        Self {
            phase: CapturePhase::Idle,
            identity: None,
            task: None,
            save_task: None,
            bytes: Vec::new(),
            message: None,
        }
    }
}

/// Direct, authoritative node files are deliberately separate from the core
/// snapshot because they are optional sources and not interpreted as health.
#[derive(Clone)]
struct NodeTextSources {
    dns: Result<String, String>,
    routes: Result<String, String>,
}

enum NetworkEvent {
    Refreshed {
        request: RequestIdentity,
        snapshot: NetworkInspectionSnapshot,
        sources: NodeTextSources,
    },
    RefreshFailed {
        request: RequestIdentity,
        message: String,
    },
    CaptureStarted {
        request: RequestIdentity,
    },
    CaptureChunk {
        request: RequestIdentity,
        bytes: Vec<u8>,
    },
    CaptureFinished {
        request: RequestIdentity,
        capped: bool,
    },
    CaptureFailed {
        request: RequestIdentity,
        message: String,
    },
    Saved {
        request: RequestIdentity,
        path: PathBuf,
        bytes: usize,
    },
    SaveFailed {
        request: RequestIdentity,
        message: String,
        bytes: Vec<u8>,
    },
}

/// Egui presentation state for inspecting a selected Talos node's network.
pub(crate) struct NetworkScreen {
    events_tx: mpsc::UnboundedSender<NetworkEvent>,
    events_rx: mpsc::UnboundedReceiver<NetworkEvent>,
    target: Option<TargetIdentity>,
    next_request: u64,
    refresh: Option<RequestIdentity>,
    snapshot: Option<NetworkInspectionSnapshot>,
    sources: Option<NodeTextSources>,
    sample: NetworkSampleState,
    last_refresh_started: Option<Instant>,
    refresh_error: Option<String>,
    interface_sort: InterfaceSort,
    capture_interface: String,
    connection_state: String,
    listening_only: bool,
    connection_interface_filter: String,
    connection_text_filter: String,
    capture_excludes_api: bool,
    capture_promiscuous: bool,
    pending_capture: Option<PacketCaptureRequest>,
    capture: CaptureState,
}

impl Default for NetworkScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkScreen {
    pub(crate) fn new() -> Self {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        Self {
            events_tx,
            events_rx,
            target: None,
            next_request: 0,
            refresh: None,
            snapshot: None,
            sources: None,
            sample: NetworkSampleState::default(),
            last_refresh_started: None,
            refresh_error: None,
            interface_sort: InterfaceSort::default(),
            capture_interface: String::new(),
            connection_state: "All states".to_owned(),
            listening_only: false,
            connection_interface_filter: String::new(),
            connection_text_filter: String::new(),
            capture_excludes_api: true,
            capture_promiscuous: false,
            pending_capture: None,
            capture: CaptureState::default(),
        }
    }

    /// Cancels an active capture stream and any pending pcap write.
    ///
    /// Aborting the capture worker drops its stream receiver, which in turn
    /// stops the Talos client's forwarding task. No worker ever owns egui data.
    pub(crate) fn deactivate(&mut self) {
        self.refresh = None;
        self.pending_capture = None;
        if let Some(task) = self.capture.task.take() {
            task.abort();
        }
        if let Some(task) = self.capture.save_task.take() {
            task.abort();
        }
        self.capture.identity = None;
        self.capture.phase = CapturePhase::Idle;
    }

    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        screen_target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        let visible_target = Self::target_from(screen_target);
        if self.target != visible_target {
            self.reset_for_target(visible_target.clone());
        }
        self.drain_events(visible_target.as_ref());

        if visible_target.is_some()
            && self.refresh.is_none()
            && self
                .last_refresh_started
                .is_none_or(|started| started.elapsed() >= AUTO_REFRESH)
        {
            self.start_refresh(screen_target, runtime, ctx);
        }
        if visible_target.is_some() {
            ctx.request_repaint_after(AUTO_REFRESH);
        }

        let refresh_clicked = self.draw_toolbar(ui, screen_target, visible_target.as_ref());
        if refresh_clicked {
            self.start_refresh(screen_target, runtime, ctx);
        }
        self.draw_content(ui, runtime, ctx);
        self.draw_capture_confirmation(ctx, screen_target, runtime);
    }

    fn target_from(target: &ScreenTarget<'_>) -> Option<TargetIdentity> {
        let context = target.context_name().trim();
        let node = target.node_name()?.trim();
        let address = target.node_address()?;
        (!context.is_empty() && !node.is_empty() && !address.trim().is_empty()).then(|| {
            TargetIdentity {
                context: context.to_owned(),
                node: InspectionTarget::new(node, address),
            }
        })
    }

    fn reset_for_target(&mut self, target: Option<TargetIdentity>) {
        self.deactivate();
        self.next_request = self.next_request.wrapping_add(1);
        self.target = target;
        self.snapshot = None;
        self.sources = None;
        self.sample = NetworkSampleState::default();
        self.last_refresh_started = None;
        self.refresh_error = if self.target.is_none() {
            Some("Select a Talos context and node before inspecting its network.".to_owned())
        } else {
            None
        };
        self.capture_interface.clear();
        self.capture.bytes.clear();
    }

    fn next_identity(&mut self, target: &TargetIdentity) -> RequestIdentity {
        self.next_request = self.next_request.wrapping_add(1);
        RequestIdentity {
            serial: self.next_request,
            target: target.clone(),
        }
    }

    fn start_refresh(&mut self, target: &ScreenTarget<'_>, runtime: &Handle, ctx: &egui::Context) {
        if self.refresh.is_some() {
            return;
        }
        let Some(identity) = self.target.clone() else {
            return;
        };
        let Some(client) = target.node_client() else {
            self.refresh_error = Some(format!(
                "No Talos client is available for node {} in context {}.",
                identity.node.name, identity.context
            ));
            return;
        };

        let request = self.next_identity(&identity);
        self.refresh = Some(request.clone());
        self.refresh_error = None;
        self.last_refresh_started = Some(Instant::now());
        let inspection = NetworkInspectionRequest::new(identity.node.clone(), self.sample.clone());
        let events = self.events_tx.clone();
        let repaint = ctx.clone();
        runtime.spawn(async move {
            let dns_client = client.clone();
            let routes_client = client.clone();
            let (inspection_result, dns, routes) = tokio::join!(
                collect_network_inspection(client, inspection),
                read_node_file(dns_client, "/etc/resolv.conf"),
                read_node_file(routes_client, "/proc/net/route"),
            );
            let event = match inspection_result {
                Ok(snapshot) => NetworkEvent::Refreshed {
                    request,
                    snapshot,
                    sources: NodeTextSources { dns, routes },
                },
                Err(error) => NetworkEvent::RefreshFailed {
                    request,
                    message: error.to_string(),
                },
            };
            let _ = events.send(event);
            repaint.request_repaint();
        });
    }

    fn drain_events(&mut self, visible_target: Option<&TargetIdentity>) {
        while let Ok(event) = self.events_rx.try_recv() {
            match event {
                NetworkEvent::Refreshed {
                    request,
                    snapshot,
                    sources,
                } if self.accepts_refresh(&request, visible_target) => {
                    self.sample = snapshot.next_sample.clone();
                    if self.capture_interface.is_empty() {
                        self.capture_interface = snapshot
                            .interfaces
                            .first()
                            .map(|interface| interface.stats.name.clone())
                            .unwrap_or_default();
                    }
                    self.snapshot = Some(snapshot);
                    self.sources = Some(sources);
                    self.refresh = None;
                    self.refresh_error = None;
                }
                NetworkEvent::RefreshFailed { request, message }
                    if self.accepts_refresh(&request, visible_target) =>
                {
                    self.refresh = None;
                    self.refresh_error = Some(message);
                }
                NetworkEvent::CaptureStarted { request }
                    if self.accepts_capture_worker(&request, visible_target) =>
                {
                    self.capture.phase = CapturePhase::Capturing;
                    self.capture.message = Some("Capturing packets…".to_owned());
                }
                NetworkEvent::CaptureChunk { request, mut bytes }
                    if self.accepts_capture_worker(&request, visible_target) =>
                {
                    let remaining = MAX_CAPTURE_SIZE.saturating_sub(self.capture.bytes.len());
                    bytes.truncate(remaining);
                    self.capture.bytes.extend_from_slice(&bytes);
                }
                NetworkEvent::CaptureFinished { request, capped }
                    if self.accepts_capture_worker(&request, visible_target) =>
                {
                    self.capture.task = None;
                    self.capture.phase = CapturePhase::Idle;
                    self.capture.message = Some(if capped {
                        format!(
                            "Capture stopped at the {} retention limit. Save the pcap or clear it before another capture.",
                            format_bytes(MAX_CAPTURE_SIZE as u64)
                        )
                    } else {
                        "Packet capture stream ended.".to_owned()
                    });
                }
                NetworkEvent::CaptureFailed { request, message }
                    if self.accepts_capture_worker(&request, visible_target) =>
                {
                    self.capture.task = None;
                    self.capture.phase = CapturePhase::Idle;
                    self.capture.message = Some(format!("Capture failed: {message}"));
                }
                NetworkEvent::Saved {
                    request,
                    path,
                    bytes,
                } if self.accepts_save(&request, visible_target) => {
                    self.capture.save_task = None;
                    self.capture.phase = CapturePhase::Idle;
                    self.capture.message = Some(format!(
                        "Saved {} to {}.",
                        format_bytes(bytes as u64),
                        path.display()
                    ));
                }
                NetworkEvent::SaveFailed {
                    request,
                    message,
                    bytes,
                } if self.accepts_save(&request, visible_target) => {
                    self.capture.save_task = None;
                    self.capture.phase = CapturePhase::Idle;
                    self.capture.bytes = bytes;
                    self.capture.message = Some(format!("Could not save pcap: {message}"));
                }
                _ => {
                    // A result from a prior context/node/address or superseded
                    // request is intentionally discarded.
                }
            }
        }
    }

    fn accepts_refresh(&self, request: &RequestIdentity, visible: Option<&TargetIdentity>) -> bool {
        visible == Some(&request.target)
            && self.target.as_ref() == Some(&request.target)
            && self.refresh.as_ref() == Some(request)
    }

    fn accepts_capture(&self, request: &RequestIdentity, visible: Option<&TargetIdentity>) -> bool {
        visible == Some(&request.target)
            && self.target.as_ref() == Some(&request.target)
            && self.capture.identity.as_ref() == Some(request)
    }

    fn accepts_capture_worker(
        &self,
        request: &RequestIdentity,
        visible: Option<&TargetIdentity>,
    ) -> bool {
        self.accepts_capture(request, visible)
            && matches!(
                self.capture.phase,
                CapturePhase::Starting | CapturePhase::Capturing
            )
    }

    fn accepts_save(&self, request: &RequestIdentity, visible: Option<&TargetIdentity>) -> bool {
        self.accepts_capture(request, visible) && self.capture.phase == CapturePhase::Saving
    }

    fn draw_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        target: &ScreenTarget<'_>,
        identity: Option<&TargetIdentity>,
    ) -> bool {
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.heading("Network");
            ui.separator();
            ui.label(RichText::new(format!("Context: {}", target.context_name())).weak());
            match identity {
                Some(identity) => ui.label(
                    RichText::new(format!(
                        "Node: {} ({})",
                        identity.node.name, identity.node.address
                    ))
                    .weak(),
                ),
                None => ui.colored_label(WARNING_COLOR, "No node selected"),
            };
            if self.refresh.is_some() {
                ui.spinner();
                ui.label("Refreshing…");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                refresh = ui
                    .add_enabled(
                        identity.is_some() && self.refresh.is_none(),
                        egui::Button::new("Refresh"),
                    )
                    .clicked();
            });
        });
        ui.separator();
        refresh
    }

    fn draw_content(&mut self, ui: &mut egui::Ui, runtime: &Handle, ctx: &egui::Context) {
        if self.snapshot.is_none() {
            if let Some(error) = &self.refresh_error {
                ui.colored_label(ERROR_COLOR, error);
            } else {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading network interfaces…");
                });
            }
            return;
        }
        if let Some(error) = &self.refresh_error {
            ui.colored_label(
                WARNING_COLOR,
                format!("Latest refresh unavailable: {error}"),
            );
            ui.add_space(6.0);
        }
        self.draw_summary(ui);
        self.draw_interfaces(ui);
        self.draw_connections(ui);
        self.draw_kubespan(ui);
        self.draw_node_files(ui);
        self.draw_capture(ui, runtime, ctx);
    }

    fn draw_summary(&self, ui: &mut egui::Ui) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Traffic summary").strong());
            ui.separator();
            ui.label(format!(
                "RX {}/s",
                format_bytes(snapshot.totals.rx_bytes_per_sec)
            ));
            ui.label(format!(
                "TX {}/s",
                format_bytes(snapshot.totals.tx_bytes_per_sec)
            ));
            if snapshot.totals.errors > 0 {
                ui.colored_label(ERROR_COLOR, format!("{} errors", snapshot.totals.errors));
            } else {
                ui.label("0 errors");
            }
            if snapshot.totals.dropped > 0 {
                ui.colored_label(
                    WARNING_COLOR,
                    format!("{} dropped", snapshot.totals.dropped),
                );
            } else {
                ui.label("0 dropped");
            }
        });
        if snapshot.is_partial() {
            egui::CollapsingHeader::new("Partial network data")
                .id_salt("network_partial_sources")
                .show(ui, |ui| {
                    ui.colored_label(
                        WARNING_COLOR,
                        "Some optional authoritative sources were unavailable.",
                    );
                    for unavailable in &snapshot.unavailable {
                        ui.add(
                            egui::Label::new(format!(
                                "{}: {}",
                                unavailable.source, unavailable.message
                            ))
                            .selectable(true)
                            .wrap(),
                        );
                    }
                });
        }
        ui.add_space(8.0);
    }

    fn draw_interfaces(&mut self, ui: &mut egui::Ui) {
        let Some(mut interfaces) = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.interfaces.clone())
        else {
            return;
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new("Interfaces").strong());
            if ui
                .selectable_label(
                    self.interface_sort == InterfaceSort::Traffic,
                    "Sort: traffic",
                )
                .clicked()
            {
                self.interface_sort = InterfaceSort::Traffic;
            }
            if ui
                .selectable_label(self.interface_sort == InterfaceSort::Errors, "Sort: errors")
                .clicked()
            {
                self.interface_sort = InterfaceSort::Errors;
            }
        });
        interfaces.sort_by(|left, right| match self.interface_sort {
            InterfaceSort::Traffic => right.stats.total_traffic().cmp(&left.stats.total_traffic()),
            InterfaceSort::Errors => right
                .stats
                .total_errors()
                .saturating_add(right.stats.total_dropped())
                .cmp(
                    &left
                        .stats
                        .total_errors()
                        .saturating_add(left.stats.total_dropped()),
                )
                .then_with(|| right.stats.total_traffic().cmp(&left.stats.total_traffic())),
        });
        egui::Grid::new("network_interfaces")
            .striped(true)
            .min_col_width(80.0)
            .show(ui, |ui| {
                ui.strong("Interface");
                ui.strong("RX");
                ui.strong("TX");
                ui.strong("RX rate");
                ui.strong("TX rate");
                ui.strong("Errors / drops");
                ui.end_row();
                for interface in interfaces {
                    let stats = &interface.stats;
                    let selected = self.capture_interface == stats.name;
                    if ui.selectable_label(selected, &stats.name).clicked() {
                        self.capture_interface = stats.name.clone();
                    }
                    ui.label(interface.received_display());
                    ui.label(interface.transmitted_display());
                    ui.label(
                        interface
                            .receive_rate_display()
                            .unwrap_or_else(|| "—".to_owned()),
                    );
                    ui.label(
                        interface
                            .transmit_rate_display()
                            .unwrap_or_else(|| "—".to_owned()),
                    );
                    let errors = stats.total_errors();
                    let dropped = stats.total_dropped();
                    if errors > 0 {
                        ui.colored_label(ERROR_COLOR, format!("{errors} / {dropped}"));
                    } else if dropped > 0 {
                        ui.colored_label(WARNING_COLOR, format!("{errors} / {dropped}"));
                    } else {
                        ui.label("0 / 0");
                    }
                    ui.end_row();
                }
            });
        ui.add_space(10.0);
    }

    fn draw_connections(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Connection analysis").strong());
        let Some((connections, services)) = self.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .connections
                .clone()
                .map(|connections| (connections, snapshot.services.clone()))
        }) else {
            ui.colored_label(
                WARNING_COLOR,
                "Talos netstat data is unavailable for this sample.",
            );
            return;
        };
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("network_connection_state")
                .selected_text(&self.connection_state)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.connection_state,
                        "All states".to_owned(),
                        "All states",
                    );
                    for state in [
                        "ESTABLISHED",
                        "LISTEN",
                        "SYN_SENT",
                        "SYN_RECV",
                        "TIME_WAIT",
                        "CLOSE_WAIT",
                        "CLOSE",
                        "FIN_WAIT1",
                        "FIN_WAIT2",
                        "LAST_ACK",
                        "CLOSING",
                        "UNKNOWN",
                    ] {
                        ui.selectable_value(&mut self.connection_state, state.to_owned(), state);
                    }
                });
            ui.checkbox(&mut self.listening_only, "Listening only");
            ui.label("Local address:");
            ui.add(
                egui::TextEdit::singleline(&mut self.connection_interface_filter)
                    .hint_text("address filter"),
            );
            ui.label("Text:");
            ui.add(
                egui::TextEdit::singleline(&mut self.connection_text_filter)
                    .hint_text("service, process, endpoint"),
            );
        });
        ui.label(RichText::new(format!(
            "{} sockets; {} listeners. Talos netstat does not report interface ownership, so the local-address filter is exact source data rather than an inferred interface mapping.",
            connections.counts.total(),
            connections.listeners.len()
        )).weak());
        let state = self.connection_state.as_str();
        let local_filter = self.connection_interface_filter.to_lowercase();
        let text_filter = self.connection_text_filter.to_lowercase();
        let mut rows: Vec<_> = connections
            .connections
            .iter()
            .filter(|row| {
                let connection = &row.connection;
                (state == "All states" || connection.state.short_name() == state)
                    && (!self.listening_only || connection.is_listening())
                    && (local_filter.is_empty()
                        || connection
                            .local_addr()
                            .to_lowercase()
                            .contains(&local_filter))
                    && (text_filter.is_empty() || connection_matches_text(row, &text_filter))
            })
            .collect();
        rows.sort_by(|left, right| {
            left.connection
                .local_port
                .cmp(&right.connection.local_port)
                .then_with(|| {
                    left.connection
                        .remote_port
                        .cmp(&right.connection.remote_port)
                })
        });
        egui::ScrollArea::vertical()
            .id_salt("network_connections_scroll")
            .max_height(260.0)
            .show(ui, |ui| {
                egui::Grid::new("network_connections")
                    .striped(true)
                    .min_col_width(70.0)
                    .show(ui, |ui| {
                        ui.strong("Proto / state");
                        ui.strong("Local");
                        ui.strong("Remote");
                        ui.strong("Service / direction");
                        ui.strong("Owner");
                        ui.end_row();
                        for row in rows {
                            let connection = &row.connection;
                            ui.label(format!(
                                "{} {}",
                                connection.protocol,
                                connection.state.short_name()
                            ));
                            ui.label(connection.local_addr());
                            ui.label(connection.remote_addr());
                            let service = row
                                .local_service
                                .or(row.remote_service)
                                .unwrap_or("unknown");
                            ui.label(format!("{service} / {:?}", row.direction));
                            ui.label(match (&connection.process_name, connection.process_pid) {
                                (Some(name), Some(pid)) => format!("{name} ({pid})"),
                                (Some(name), None) => name.clone(),
                                (_, Some(pid)) => format!("pid {pid}"),
                                _ => "—".to_owned(),
                            });
                            ui.end_row();
                        }
                    });
            });
        if !connections.listeners.is_empty() {
            egui::CollapsingHeader::new("Listeners and Talos services")
                .id_salt("network_listeners")
                .show(ui, |ui| {
                    for listener in &connections.listeners {
                        let service_name = listener.service.unwrap_or("unrecognized port");
                        let service_status = services
                            .as_ref()
                            .and_then(|services| {
                                services.iter().find(|service| service.id == service_name)
                            })
                            .map(|service| {
                                let health =
                                    service.health.as_ref().map_or("health unknown", |health| {
                                        if health.unknown {
                                            "health unknown"
                                        } else if health.healthy {
                                            "healthy"
                                        } else {
                                            "unhealthy"
                                        }
                                    });
                                format!("{} ({health})", service.state)
                            })
                            .unwrap_or_else(|| "service state unavailable".to_owned());
                        ui.label(format!(
                            "{} on port {} — {} — {}",
                            listener
                                .process_name
                                .as_deref()
                                .unwrap_or("owner unavailable"),
                            listener.port,
                            service_name,
                            service_status
                        ));
                    }
                });
        }
        ui.add_space(10.0);
    }

    fn draw_kubespan(&self, ui: &mut egui::Ui) {
        ui.label(RichText::new("KubeSpan").strong());
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        match &snapshot.kubespan {
            KubeSpanSnapshot::NotRequested => {
                ui.label("KubeSpan was not requested for this sample.");
            }
            KubeSpanSnapshot::Disabled => {
                ui.label("KubeSpan is disabled.");
            }
            KubeSpanSnapshot::Unavailable { message } => {
                ui.colored_label(
                    WARNING_COLOR,
                    format!("KubeSpan state unavailable: {message}"),
                );
            }
            KubeSpanSnapshot::Enabled { peers } => {
                ui.label(format!("Enabled — {} peer records", peers.len()));
                egui::Grid::new("network_kubespan_peers")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Peer");
                        ui.strong("Endpoint");
                        ui.strong("State");
                        ui.strong("RTT");
                        ui.strong("Traffic");
                        ui.end_row();
                        for peer in peers {
                            ui.label(if peer.label.is_empty() {
                                &peer.id
                            } else {
                                &peer.label
                            });
                            ui.label(peer.endpoint.as_deref().unwrap_or("—"));
                            ui.label(&peer.state);
                            ui.label(
                                peer.rtt_ms
                                    .map_or_else(|| "—".to_owned(), |rtt| format!("{rtt:.1} ms")),
                            );
                            ui.label(format!(
                                "RX {} / TX {}",
                                format_bytes(peer.rx_bytes),
                                format_bytes(peer.tx_bytes)
                            ));
                            ui.end_row();
                        }
                    });
            }
        }
        ui.add_space(10.0);
    }

    fn draw_node_files(&self, ui: &mut egui::Ui) {
        ui.label(RichText::new("DNS and routes").strong());
        let Some(sources) = &self.sources else {
            ui.label("Direct node files have not been collected yet.");
            return;
        };
        Self::draw_text_source(ui, "DNS configuration (/etc/resolv.conf)", &sources.dns);
        Self::draw_text_source(ui, "Routing table (/proc/net/route)", &sources.routes);
        ui.add_space(10.0);
    }

    fn draw_text_source(ui: &mut egui::Ui, title: &str, source: &Result<String, String>) {
        egui::CollapsingHeader::new(title).show(ui, |ui| match source {
            Ok(text) if text.is_empty() => ui.label("The authoritative source is empty."),
            Ok(text) => ui.add(egui::Label::new(text).selectable(true).wrap()),
            Err(message) => ui.colored_label(WARNING_COLOR, format!("Unavailable: {message}")),
        });
    }

    fn draw_capture(&mut self, ui: &mut egui::Ui, runtime: &Handle, ctx: &egui::Context) {
        ui.label(RichText::new("Packet capture").strong());
        let interfaces: Vec<String> = self
            .snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .interfaces
                    .iter()
                    .map(|interface| interface.stats.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let can_configure =
            self.capture.phase == CapturePhase::Idle && self.pending_capture.is_none();
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("network_capture_interface")
                .selected_text(if self.capture_interface.is_empty() {
                    "Choose interface"
                } else {
                    &self.capture_interface
                })
                .show_ui(ui, |ui| {
                    for interface in &interfaces {
                        ui.selectable_value(
                            &mut self.capture_interface,
                            interface.clone(),
                            interface,
                        );
                    }
                });
            ui.add_enabled_ui(can_configure, |ui| {
                ui.checkbox(
                    &mut self.capture_excludes_api,
                    "Exclude Talos API port 50000 (BPF)",
                );
                ui.checkbox(&mut self.capture_promiscuous, "Promiscuous mode");
            });
            match self.capture.phase {
                CapturePhase::Starting => {
                    ui.spinner();
                    ui.label("Starting capture…");
                }
                CapturePhase::Capturing => {
                    ui.spinner();
                    ui.label(format!(
                        "Capturing {}",
                        format_bytes(self.capture.bytes.len() as u64)
                    ));
                }
                CapturePhase::Saving => {
                    ui.spinner();
                    ui.label("Saving pcap…");
                }
                CapturePhase::Idle => {}
            }
        });
        ui.label(RichText::new(format!(
            "Captured pcap is retained in memory up to {}. Reaching that bound stops the capture; packets after the bound are not retained. The Talos API BPF exclusion prevents management traffic from being captured, but also omits API-port 50000 packets.",
            format_bytes(MAX_CAPTURE_SIZE as u64)
        )).weak());
        ui.horizontal(|ui| {
            let start = ui
                .add_enabled(
                    can_configure && !self.capture_interface.is_empty(),
                    egui::Button::new("Start capture…"),
                )
                .clicked();
            if start {
                self.open_capture_confirmation();
            }
            if ui
                .add_enabled(
                    self.capture.phase == CapturePhase::Capturing
                        || self.capture.phase == CapturePhase::Starting,
                    egui::Button::new("Stop"),
                )
                .clicked()
            {
                self.stop_capture("Capture stopped by user.");
            }
            if ui
                .add_enabled(
                    self.capture.phase == CapturePhase::Idle && !self.capture.bytes.is_empty(),
                    egui::Button::new("Save pcap…"),
                )
                .clicked()
            {
                if let Some(path) = self.request_save_destination() {
                    self.start_save(path, runtime, ctx);
                }
            }
            if ui
                .add_enabled(
                    self.capture.phase == CapturePhase::Idle && !self.capture.bytes.is_empty(),
                    egui::Button::new("Clear"),
                )
                .clicked()
            {
                self.capture.bytes.clear();
                self.capture.message = Some("Retained packet capture cleared.".to_owned());
            }
        });
        if let Some(message) = &self.capture.message {
            ui.label(message);
        }
        ui.add_space(10.0);
    }

    fn open_capture_confirmation(&mut self) {
        let Some(target) = self.target.as_ref() else {
            return;
        };
        let mut request =
            PacketCaptureRequest::new(target.node.clone(), self.capture_interface.clone());
        request.promiscuous = self.capture_promiscuous;
        request.exclude_talos_api_traffic = self.capture_excludes_api;
        request.max_bytes = MAX_CAPTURE_SIZE;
        self.pending_capture = Some(request);
    }

    fn draw_capture_confirmation(
        &mut self,
        ctx: &egui::Context,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
    ) {
        let Some(request) = self.pending_capture.as_ref() else {
            return;
        };
        let metadata = request.metadata();
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Confirm packet capture")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.colored_label(WARNING_COLOR, "Packet capture can contain sensitive application traffic.");
                ui.label(format!("Node: {} ({})", metadata.target.name, metadata.target.address));
                ui.label(format!("Interface: {}", metadata.interface));
                ui.label(format!("Promiscuous mode: {}", if metadata.promiscuous { "enabled" } else { "disabled" }));
                ui.label(format!(
                    "Talos API BPF exclusion: {}.",
                    if metadata.excludes_talos_api_traffic {
                        "enabled — packets to or from port 50000 are excluded"
                    } else {
                        "disabled — management API traffic can be captured"
                    }
                ));
                ui.label(format!(
                    "Retention bound: {}. Capture stops at this bound; later packets are discarded and the pcap may be incomplete.",
                    format_bytes(metadata.max_bytes as u64)
                ));
                ui.horizontal(|ui| {
                    confirm = ui.button("Confirm and start").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if cancel {
            self.pending_capture = None;
        } else if confirm {
            let request = self.pending_capture.take();
            if let Some(request) = request {
                self.start_capture(request, target, runtime, ctx);
            }
        }
    }

    fn start_capture(
        &mut self,
        request: PacketCaptureRequest,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        if self.capture.phase != CapturePhase::Idle {
            return;
        }
        let Some(current) = self.target.clone() else {
            return;
        };
        let Some(client) = target.node_client() else {
            self.capture.message =
                Some("No Talos client is available for packet capture.".to_owned());
            return;
        };
        let identity = self.next_identity(&current);
        let metadata = request.metadata();
        self.capture.bytes.clear();
        self.capture.identity = Some(identity.clone());
        self.capture.phase = CapturePhase::Starting;
        self.capture.message = None;
        let events = self.events_tx.clone();
        let repaint = ctx.clone();
        self.capture.task = Some(runtime.spawn(async move {
            let receiver = if metadata.excludes_talos_api_traffic {
                client
                    .packet_capture_exclude_api(
                        &metadata.interface,
                        metadata.promiscuous,
                        metadata.snap_len,
                    )
                    .await
            } else {
                client
                    .packet_capture(&metadata.interface, metadata.promiscuous, metadata.snap_len)
                    .await
            };
            let mut receiver = match receiver {
                Ok(receiver) => receiver,
                Err(error) => {
                    let _ = events.send(NetworkEvent::CaptureFailed {
                        request: identity,
                        message: error.to_string(),
                    });
                    repaint.request_repaint();
                    return;
                }
            };
            let _ = events.send(NetworkEvent::CaptureStarted {
                request: identity.clone(),
            });
            repaint.request_repaint();
            let mut retained = 0usize;
            while let Some(mut chunk) = receiver.recv().await {
                let remaining = metadata.max_bytes.saturating_sub(retained);
                if remaining == 0 {
                    break;
                }
                let capped = chunk.len() > remaining;
                chunk.truncate(remaining);
                retained += chunk.len();
                let _ = events.send(NetworkEvent::CaptureChunk {
                    request: identity.clone(),
                    bytes: chunk,
                });
                repaint.request_repaint();
                if capped || retained == metadata.max_bytes {
                    let _ = events.send(NetworkEvent::CaptureFinished {
                        request: identity,
                        capped: true,
                    });
                    repaint.request_repaint();
                    return;
                }
            }
            let _ = events.send(NetworkEvent::CaptureFinished {
                request: identity,
                capped: false,
            });
            repaint.request_repaint();
        }));
    }

    fn stop_capture(&mut self, message: &str) {
        if let Some(task) = self.capture.task.take() {
            task.abort();
        }
        self.capture.phase = CapturePhase::Idle;
        self.capture.message = Some(message.to_owned());
    }

    fn request_save_destination(&mut self) -> Option<PathBuf> {
        let suggested = format!(
            "{}-capture.pcap",
            if self.capture_interface.is_empty() {
                "talos"
            } else {
                &self.capture_interface
            }
        );
        rfd::FileDialog::new()
            .set_title("Save packet capture")
            .set_file_name(&suggested)
            .add_filter("pcap", &["pcap"])
            .save_file()
    }

    fn start_save(&mut self, path: PathBuf, runtime: &Handle, ctx: &egui::Context) {
        let Some(request) = self.capture.identity.clone() else {
            self.capture.message = Some("No target is associated with this capture.".to_owned());
            return;
        };
        if self.capture.bytes.is_empty() {
            self.capture.message = Some("There is no retained pcap data to save.".to_owned());
            return;
        }
        let bytes = std::mem::take(&mut self.capture.bytes);
        let byte_count = bytes.len();
        self.capture.phase = CapturePhase::Saving;
        self.capture.message = Some(format!("Saving to {}…", path.display()));
        let events = self.events_tx.clone();
        let repaint = ctx.clone();
        self.capture.save_task = Some(runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || match std::fs::write(&path, &bytes) {
                Ok(()) => Ok((path, byte_count)),
                Err(error) => Err((error.to_string(), bytes)),
            })
            .await;
            let event = match result {
                Ok(Ok((path, bytes))) => NetworkEvent::Saved {
                    request,
                    path,
                    bytes,
                },
                Ok(Err((message, bytes))) => NetworkEvent::SaveFailed {
                    request,
                    message,
                    bytes,
                },
                Err(error) => NetworkEvent::SaveFailed {
                    request,
                    message: format!("pcap write task failed: {error}"),
                    bytes: Vec::new(),
                },
            };
            let _ = events.send(event);
            repaint.request_repaint();
        }));
    }
}

impl Drop for NetworkScreen {
    fn drop(&mut self) {
        self.deactivate();
    }
}

async fn read_node_file(
    client: talos_rs::TalosClient,
    path: &'static str,
) -> Result<String, String> {
    match tokio::time::timeout(SOURCE_TIMEOUT, client.read_file(path)).await {
        Ok(Ok(contents)) => Ok(contents),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err(format!(
            "Timed out after {} seconds",
            SOURCE_TIMEOUT.as_secs()
        )),
    }
}

fn connection_matches_text(
    row: &talos_pilot_core::inspection::NetworkConnectionSnapshot,
    filter: &str,
) -> bool {
    let connection = &row.connection;
    [
        connection.protocol.as_str(),
        connection.local_ip.as_str(),
        connection.remote_ip.as_str(),
        connection.process_name.as_deref().unwrap_or_default(),
        row.local_service.unwrap_or_default(),
        row.remote_service.unwrap_or_default(),
    ]
    .into_iter()
    .any(|value| value.to_lowercase().contains(filter))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_identity_requires_matching_context_node_and_address() {
        let first = TargetIdentity {
            context: "lab-a".to_owned(),
            node: InspectionTarget::new("controlplane", "10.0.0.1"),
        };
        let same_name_other_context = TargetIdentity {
            context: "lab-b".to_owned(),
            node: InspectionTarget::new("controlplane", "10.0.0.1"),
        };
        let same_name_other_address = TargetIdentity {
            context: "lab-a".to_owned(),
            node: InspectionTarget::new("controlplane", "10.0.0.2"),
        };
        assert_ne!(first, same_name_other_context);
        assert_ne!(first, same_name_other_address);
    }
}
