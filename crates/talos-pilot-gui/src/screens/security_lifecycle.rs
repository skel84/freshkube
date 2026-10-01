//! Native security and lifecycle screens.
//!
//! Collection is always scheduled on the application's Tokio runtime.  The
//! egui thread owns only immutable snapshots and interaction state.

use std::sync::mpsc;

use eframe::egui::{self, Color32, RichText};
use talos_pilot_core::{
    indicators::{HealthIndicator, QuorumState},
    security_lifecycle::{
        CertificateAudit, CertificateExpiryStatus, CertificateSource, EncryptionProvider,
        EtcdPreOperationAudit, LifecycleAlert, LifecycleCollector, LifecycleSnapshot,
        NodeLifecycleSnapshot, RbacAudit, SecurityAuditCollector, SecurityAuditSnapshot,
        SourceSnapshot, TimeSynchronizationAudit, VolumeEncryptionAudit,
    },
};
use tokio::runtime::Handle;

use crate::screens::ScreenTarget;

const HEALTHY: Color32 = Color32::from_rgb(76, 175, 80);
const WARNING: Color32 = Color32::from_rgb(255, 193, 7);
const ERROR: Color32 = Color32::from_rgb(244, 67, 54);
const UNKNOWN: Color32 = Color32::from_rgb(158, 158, 158);
const INFO: Color32 = Color32::from_rgb(100, 181, 246);

#[derive(Debug)]
enum SecurityEvent {
    Collected {
        request: u64,
        context: String,
        snapshot: SecurityAuditSnapshot,
    },
}

/// Egui state for the cluster-wide PKI, RBAC, and volume encryption audit.
///
/// Its receiver is deliberately screen-local so a completed request cannot
/// affect another route or another selected Talos context.
pub(crate) struct SecurityScreen {
    events: mpsc::Receiver<SecurityEvent>,
    event_tx: mpsc::Sender<SecurityEvent>,
    active_context: Option<String>,
    request: u64,
    loading: bool,
    snapshot: Option<SecurityAuditSnapshot>,
}

impl Default for SecurityScreen {
    fn default() -> Self {
        let (event_tx, events) = mpsc::channel();
        Self {
            events,
            event_tx,
            active_context: None,
            request: 0,
            loading: false,
            snapshot: None,
        }
    }
}

impl SecurityScreen {
    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        self.sync_context(target.context_name());
        self.drain_events(target.context_name());

        let refresh = ui
            .add_enabled(!self.loading, egui::Button::new("Refresh security audit"))
            .clicked();
        if self.loading {
            ui.spinner();
            ui.label("Collecting security audit…");
        }
        ui.separator();

        if refresh || (!self.loading && self.snapshot.is_none()) {
            self.start_refresh(target, runtime, ctx);
        }

        let Some(snapshot) = self.snapshot.as_ref() else {
            ui.label(RichText::new("Security audit has not returned data yet.").color(UNKNOWN));
            return;
        };

        draw_identity(ui, &snapshot.identity);
        ui.add_space(8.0);
        ui.heading("PKI certificates");
        draw_certificates(
            ui,
            "Talosconfig certificates",
            &snapshot.talosconfig_certificates,
        );
        draw_certificates(
            ui,
            "Kubeconfig certificates served by Talos",
            &snapshot.talos_kubeconfig_certificates,
        );

        ui.add_space(8.0);
        ui.heading("RBAC identity");
        draw_rbac(ui, &snapshot.rbac);

        ui.add_space(8.0);
        ui.heading("Volume encryption");
        draw_encryption(ui, &snapshot.volume_encryption);
    }

    /// Invalidate any in-flight collection when another route becomes active.
    pub(crate) fn deactivate(&mut self) {
        self.request = self.request.wrapping_add(1);
        self.active_context = None;
        self.loading = false;
        self.snapshot = None;
    }

    fn sync_context(&mut self, context: &str) {
        if self.active_context.as_deref() == Some(context) {
            return;
        }
        self.active_context = Some(context.to_owned());
        self.request = self.request.wrapping_add(1);
        self.loading = false;
        self.snapshot = None;
    }

    fn start_refresh(&mut self, target: &ScreenTarget<'_>, runtime: &Handle, ctx: &egui::Context) {
        let Some(client) = target.cluster_client() else {
            self.loading = false;
            self.snapshot = Some(unavailable_security_snapshot(
                "The selected context has no Talos client identity",
            ));
            return;
        };

        self.loading = true;
        self.request = self.request.wrapping_add(1);
        let request = self.request;
        let context = target.context_name().to_owned();
        let collector = SecurityAuditCollector::new(target.config_path.cloned());
        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        runtime.spawn(async move {
            let snapshot = collector.collect(&client).await;
            let _ = event_tx.send(SecurityEvent::Collected {
                request,
                context,
                snapshot,
            });
            repaint.request_repaint();
        });
    }

    fn drain_events(&mut self, context: &str) {
        while let Ok(SecurityEvent::Collected {
            request,
            context: event_context,
            snapshot,
        }) = self.events.try_recv()
        {
            if accepts_event(
                self.active_context.as_deref(),
                context,
                self.request,
                request,
                &event_context,
            ) {
                self.loading = false;
                self.snapshot = Some(snapshot);
            }
        }
    }
}

#[derive(Debug)]
enum LifecycleEvent {
    Collected {
        request: u64,
        context: String,
        snapshot: LifecycleSnapshot,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum NodeSort {
    #[default]
    Name,
    Version,
    ConfigHash,
}

impl NodeSort {
    const ALL: [Self; 3] = [Self::Name, Self::Version, Self::ConfigHash];

    fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Version => "Version",
            Self::ConfigHash => "Config hash",
        }
    }
}

/// Egui state for lifecycle collection and client-side node exploration.
pub(crate) struct LifecycleScreen {
    events: mpsc::Receiver<LifecycleEvent>,
    event_tx: mpsc::Sender<LifecycleEvent>,
    active_context: Option<String>,
    request: u64,
    loading: bool,
    snapshot: Option<LifecycleSnapshot>,
    node_filter: String,
    node_sort: NodeSort,
}

impl Default for LifecycleScreen {
    fn default() -> Self {
        let (event_tx, events) = mpsc::channel();
        Self {
            events,
            event_tx,
            active_context: None,
            request: 0,
            loading: false,
            snapshot: None,
            node_filter: String::new(),
            node_sort: NodeSort::default(),
        }
    }
}

impl LifecycleScreen {
    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        self.sync_context(target.context_name());
        self.drain_events(target.context_name());

        let refresh = ui
            .add_enabled(!self.loading, egui::Button::new("Refresh lifecycle audit"))
            .clicked();
        if self.loading {
            ui.spinner();
            ui.label("Collecting lifecycle state…");
        }
        ui.separator();

        if refresh || (!self.loading && self.snapshot.is_none()) {
            self.start_refresh(target, runtime, ctx);
        }

        if self.snapshot.is_none() {
            ui.label(RichText::new("Lifecycle audit has not returned data yet.").color(UNKNOWN));
            return;
        }

        {
            let snapshot = self.snapshot.as_ref().expect("checked above");
            draw_identity(ui, &snapshot.identity);
            ui.add_space(8.0);
            ui.heading("Cluster roster sources");
            draw_discovery_roster(ui, &snapshot.talos_discovery);
            draw_kubernetes_roster(ui, &snapshot.kubernetes_roster);

            ui.add_space(8.0);
            ui.heading("Pre-operation etcd safety");
            draw_etcd_safety(ui, &snapshot.etcd_pre_operation);

            ui.add_space(8.0);
            ui.heading("Lifecycle alerts");
            if snapshot.alerts.is_empty() {
                ui.label(
                    RichText::new("No alerts were reported by the collected sources.").color(INFO),
                );
            } else {
                for alert in &snapshot.alerts {
                    ui.colored_label(
                        health_color(alert.health),
                        format!("{} {}", alert.health.symbol(), alert.message),
                    );
                }
            }
        }

        ui.add_space(8.0);
        ui.heading("Versions, synchronization, and configuration drift");
        self.node_controls(ui);
        let snapshot = self.snapshot.as_ref().expect("checked above");
        draw_config_drift(ui, &snapshot.nodes);
        draw_nodes(ui, &snapshot.nodes, &self.node_filter, self.node_sort);
    }

    /// Invalidate any in-flight collection when another route becomes active.
    pub(crate) fn deactivate(&mut self) {
        self.request = self.request.wrapping_add(1);
        self.active_context = None;
        self.loading = false;
        self.snapshot = None;
        self.node_filter.clear();
    }

    fn node_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Filter nodes:");
            ui.add(
                egui::TextEdit::singleline(&mut self.node_filter)
                    .hint_text("name, address, version"),
            );
            if ui.small_button("Clear").clicked() {
                self.node_filter.clear();
            }
            egui::ComboBox::from_label("Sort")
                .selected_text(self.node_sort.label())
                .show_ui(ui, |ui| {
                    for sort in NodeSort::ALL {
                        ui.selectable_value(&mut self.node_sort, sort, sort.label());
                    }
                });
        });
    }

    fn sync_context(&mut self, context: &str) {
        if self.active_context.as_deref() == Some(context) {
            return;
        }
        self.active_context = Some(context.to_owned());
        self.request = self.request.wrapping_add(1);
        self.loading = false;
        self.snapshot = None;
    }

    fn start_refresh(&mut self, target: &ScreenTarget<'_>, runtime: &Handle, ctx: &egui::Context) {
        let Some(client) = target.cluster_client() else {
            self.loading = false;
            self.snapshot = Some(unavailable_lifecycle_snapshot(
                "The selected context has no Talos client identity",
            ));
            return;
        };

        self.loading = true;
        self.request = self.request.wrapping_add(1);
        let request = self.request;
        let context = target.context_name().to_owned();
        let collector = LifecycleCollector::new(target.config_path.cloned());
        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        runtime.spawn(async move {
            let snapshot = collector.collect(&client).await;
            let _ = event_tx.send(LifecycleEvent::Collected {
                request,
                context,
                snapshot,
            });
            repaint.request_repaint();
        });
    }

    fn drain_events(&mut self, context: &str) {
        while let Ok(LifecycleEvent::Collected {
            request,
            context: event_context,
            snapshot,
        }) = self.events.try_recv()
        {
            if accepts_event(
                self.active_context.as_deref(),
                context,
                self.request,
                request,
                &event_context,
            ) {
                self.loading = false;
                self.snapshot = Some(snapshot);
            }
        }
    }
}

fn accepts_event(
    active_context: Option<&str>,
    rendered_context: &str,
    latest_request: u64,
    event_request: u64,
    event_context: &str,
) -> bool {
    active_context == Some(rendered_context)
        && event_context == rendered_context
        && event_request == latest_request
}

fn unavailable<T>(reason: &str) -> SourceSnapshot<T> {
    SourceSnapshot::Unavailable {
        reason: reason.to_owned(),
    }
}

fn unavailable_security_snapshot(reason: &str) -> SecurityAuditSnapshot {
    SecurityAuditSnapshot {
        identity: unavailable(reason),
        talosconfig_certificates: unavailable(reason),
        talos_kubeconfig_certificates: unavailable(reason),
        rbac: unavailable(reason),
        volume_encryption: unavailable(reason),
    }
}

fn unavailable_lifecycle_snapshot(reason: &str) -> LifecycleSnapshot {
    LifecycleSnapshot {
        identity: unavailable(reason),
        talos_discovery: unavailable(reason),
        kubernetes_roster: unavailable(reason),
        nodes: Vec::new(),
        etcd_pre_operation: unavailable(reason),
        alerts: vec![LifecycleAlert {
            health: HealthIndicator::Unknown,
            message: reason.to_owned(),
        }],
    }
}

fn draw_identity(
    ui: &mut egui::Ui,
    identity: &SourceSnapshot<talos_pilot_core::security_lifecycle::ClusterIdentity>,
) {
    ui.horizontal(|ui| {
        ui.strong("Context identity:");
        match identity {
            SourceSnapshot::Available(value) => {
                ui.colored_label(HEALTHY, &value.context_name);
                ui.label(format!(
                    "Endpoints: {}",
                    value.endpoint_addresses.join(", ")
                ));
                ui.label(format!("Targets: {}", value.target_addresses.join(", ")));
            }
            SourceSnapshot::Partial { value, warnings } => {
                ui.colored_label(WARNING, format!("{} (partial)", value.context_name));
                draw_warnings(ui, warnings);
            }
            SourceSnapshot::Unavailable { reason } => {
                ui.colored_label(UNKNOWN, format!("Unavailable: {reason}"));
            }
        }
    });
}

fn draw_certificates(
    ui: &mut egui::Ui,
    title: &str,
    source: &SourceSnapshot<Vec<CertificateAudit>>,
) {
    egui::CollapsingHeader::new(title)
        .default_open(true)
        .show(ui, |ui| match source {
            SourceSnapshot::Available(certificates) => {
                ui.colored_label(HEALTHY, "Available");
                for certificate in certificates {
                    draw_certificate(ui, certificate);
                }
            }
            SourceSnapshot::Partial { value, warnings } => {
                ui.colored_label(WARNING, "Partial certificate data");
                draw_warnings(ui, warnings);
                for certificate in value {
                    draw_certificate(ui, certificate);
                }
            }
            SourceSnapshot::Unavailable { reason } => {
                ui.colored_label(UNKNOWN, format!("Unavailable: {reason}"));
            }
        });
}

fn draw_certificate(ui: &mut egui::Ui, certificate: &CertificateAudit) {
    let expiry = certificate_expiry_label(certificate.expiry);
    let label = format!(
        "{} — {} ({} days remaining)",
        certificate.name, expiry, certificate.days_remaining
    );
    egui::CollapsingHeader::new(RichText::new(label).color(certificate_expiry_color(certificate.expiry)))
        .show(ui, |ui| {
            let details = format!(
                "Name: {}\nSource: {}\nSubject: {}\nIssuer: {}\nValid from: {}\nValid until: {}\nDays remaining: {}\nCertificate authority: {}",
                certificate.name,
                certificate_source_label(certificate.source),
                certificate.subject,
                certificate.issuer,
                certificate.not_before,
                certificate.not_after,
                certificate.days_remaining,
                if certificate.is_ca { "yes" } else { "no" },
            );
            ui.add(egui::Label::new(details).selectable(true).wrap());
        });
}

fn draw_rbac(ui: &mut egui::Ui, source: &SourceSnapshot<RbacAudit>) {
    match source {
        SourceSnapshot::Available(rbac) => draw_rbac_value(ui, rbac, HEALTHY),
        SourceSnapshot::Partial { value, warnings } => {
            draw_rbac_value(ui, value, WARNING);
            draw_warnings(ui, warnings);
        }
        SourceSnapshot::Unavailable { reason } => {
            ui.colored_label(UNKNOWN, format!("RBAC identity unavailable: {reason}"));
        }
    }
}

fn draw_rbac_value(ui: &mut egui::Ui, rbac: &RbacAudit, color: Color32) {
    ui.colored_label(color, format!("Role: {}", rbac.role));
    ui.add(
        egui::Label::new(format!("Certificate subject: {}", rbac.certificate_subject))
            .selectable(true),
    );
}

fn draw_encryption(ui: &mut egui::Ui, source: &SourceSnapshot<Vec<VolumeEncryptionAudit>>) {
    match source {
        SourceSnapshot::Available(volumes) => draw_encryption_values(ui, volumes, false),
        SourceSnapshot::Partial { value, warnings } => {
            ui.colored_label(WARNING, "Partial volume encryption data");
            draw_warnings(ui, warnings);
            draw_encryption_values(ui, value, true);
        }
        SourceSnapshot::Unavailable { reason } => {
            ui.colored_label(UNKNOWN, format!("Volume encryption unavailable: {reason}"));
        }
    }
}

fn draw_encryption_values(ui: &mut egui::Ui, volumes: &[VolumeEncryptionAudit], partial: bool) {
    if volumes.is_empty() {
        ui.colored_label(
            if partial { WARNING } else { UNKNOWN },
            "No STATE or EPHEMERAL volumes were returned.",
        );
        return;
    }
    for volume in volumes {
        ui.horizontal(|ui| {
            ui.label(&volume.volume_name);
            ui.colored_label(
                encryption_color(&volume.provider),
                encryption_label(&volume.provider),
            );
        });
    }
}

fn draw_discovery_roster(
    ui: &mut egui::Ui,
    source: &SourceSnapshot<Vec<talos_pilot_core::security_lifecycle::DiscoveryRosterEntry>>,
) {
    egui::CollapsingHeader::new("Talos discovery roster").show(ui, |ui| match source {
        SourceSnapshot::Available(entries) => {
            ui.colored_label(HEALTHY, format!("Available: {} member(s)", entries.len()));
            for entry in entries {
                ui.add(
                    egui::Label::new(format!(
                        "{} ({}) — {}",
                        entry.name,
                        entry.machine_type,
                        entry.addresses.join(", ")
                    ))
                    .selectable(true),
                );
            }
        }
        SourceSnapshot::Partial { value, warnings } => {
            ui.colored_label(WARNING, format!("Partial: {} member(s)", value.len()));
            draw_warnings(ui, warnings);
            for entry in value {
                ui.add(
                    egui::Label::new(format!("{} ({})", entry.name, entry.addresses.join(", ")))
                        .selectable(true),
                );
            }
        }
        SourceSnapshot::Unavailable { reason } => {
            ui.colored_label(UNKNOWN, format!("Unavailable: {reason}"));
        }
    });
}

fn draw_kubernetes_roster(
    ui: &mut egui::Ui,
    source: &SourceSnapshot<Vec<talos_pilot_core::security_lifecycle::KubernetesNodeRosterEntry>>,
) {
    egui::CollapsingHeader::new("Kubernetes roster (Talos-provided kubeconfig)").show(ui, |ui| {
        match source {
            SourceSnapshot::Available(entries) => {
                ui.colored_label(HEALTHY, format!("Available: {} node(s)", entries.len()));
                for entry in entries {
                    ui.add(
                        egui::Label::new(format!(
                            "{} — {}{}",
                            entry.name,
                            entry
                                .internal_address
                                .as_deref()
                                .unwrap_or("no internal address"),
                            if entry.is_control_plane {
                                " (control plane)"
                            } else {
                                ""
                            },
                        ))
                        .selectable(true),
                    );
                }
            }
            SourceSnapshot::Partial { value, warnings } => {
                ui.colored_label(WARNING, format!("Partial: {} node(s)", value.len()));
                draw_warnings(ui, warnings);
            }
            SourceSnapshot::Unavailable { reason } => {
                ui.colored_label(UNKNOWN, format!("Unavailable: {reason}"));
            }
        }
    });
}

fn draw_etcd_safety(ui: &mut egui::Ui, source: &SourceSnapshot<EtcdPreOperationAudit>) {
    match source {
        SourceSnapshot::Available(audit) => draw_etcd_audit(ui, audit, HEALTHY),
        SourceSnapshot::Partial { value, warnings } => {
            draw_etcd_audit(ui, value, WARNING);
            draw_warnings(ui, warnings);
        }
        SourceSnapshot::Unavailable { reason } => {
            ui.colored_label(
                UNKNOWN,
                format!("Safety indeterminate: etcd data unavailable: {reason}"),
            );
        }
    }
}

fn draw_etcd_audit(ui: &mut egui::Ui, audit: &EtcdPreOperationAudit, source_color: Color32) {
    let safe = audit.safe_for_single_member_operation();
    let safety_color = if safe { HEALTHY } else { ERROR };
    ui.colored_label(
        if source_color == WARNING {
            WARNING
        } else {
            safety_color
        },
        if safe && source_color != WARNING {
            "Safe for a single-member disruptive operation"
        } else if safe {
            "Safety data is partial; do not treat this as a green light"
        } else {
            "Not safe for a single-member disruptive operation"
        },
    );
    ui.label(format!(
        "Responding members: {}/{}; quorum required: {}; remaining failure tolerance: {}",
        audit.responding_members, audit.total_members, audit.quorum_required, audit.can_lose
    ));
    ui.colored_label(
        quorum_color(&audit.quorum),
        format!("Quorum: {}", quorum_label(&audit.quorum)),
    );
}

fn draw_config_drift(ui: &mut egui::Ui, nodes: &[NodeLifecycleSnapshot]) {
    let all_hashes_available = !nodes.is_empty()
        && nodes
            .iter()
            .all(|node| matches!(node.config_hash, SourceSnapshot::Available(_)));
    if !all_hashes_available {
        ui.colored_label(
            UNKNOWN,
            "Configuration drift is indeterminate because one or more node hashes are unavailable or partial.",
        );
        return;
    }
    let mut hashes = nodes
        .iter()
        .filter_map(|node| node.config_hash.value())
        .collect::<Vec<_>>();
    hashes.sort_unstable();
    hashes.dedup();
    if hashes.len() == 1 {
        ui.colored_label(
            HEALTHY,
            "No configuration drift detected across collected nodes.",
        );
    } else {
        ui.colored_label(
            WARNING,
            format!(
                "Configuration drift detected: {} distinct hashes.",
                hashes.len()
            ),
        );
    }
}

fn draw_nodes(ui: &mut egui::Ui, nodes: &[NodeLifecycleSnapshot], filter: &str, sort: NodeSort) {
    let filter = filter.trim().to_ascii_lowercase();
    let mut visible = nodes
        .iter()
        .filter(|node| node_matches_filter(node, &filter))
        .collect::<Vec<_>>();
    visible.sort_by(|left, right| node_sort_value(left, sort).cmp(node_sort_value(right, sort)));

    if visible.is_empty() {
        ui.colored_label(
            if nodes.is_empty() { UNKNOWN } else { INFO },
            if nodes.is_empty() {
                "No lifecycle nodes were collected."
            } else {
                "No nodes match the current filter."
            },
        );
        return;
    }

    for node in visible {
        let version = source_text(&node.version);
        let title = format!("{} — {}", node.name, version);
        egui::CollapsingHeader::new(title).show(ui, |ui| {
            ui.add(
                egui::Label::new(format!(
                    "Address: {}\nMachine type: {}",
                    node.address.as_deref().unwrap_or("unavailable"),
                    node.machine_type.as_deref().unwrap_or("unavailable"),
                ))
                .selectable(true),
            );
            draw_string_source(ui, "Talos version", &node.version);
            draw_string_source(ui, "Platform", &node.platform);
            draw_time_source(ui, &node.time_synchronization);
            draw_string_source(ui, "Configuration hash", &node.config_hash);
        });
    }
}

fn node_matches_filter(node: &NodeLifecycleSnapshot, filter: &str) -> bool {
    filter.is_empty()
        || node.name.to_ascii_lowercase().contains(filter)
        || node
            .address
            .as_deref()
            .is_some_and(|address| address.to_ascii_lowercase().contains(filter))
        || node
            .version
            .value()
            .is_some_and(|version| version.to_ascii_lowercase().contains(filter))
        || node
            .platform
            .value()
            .is_some_and(|platform| platform.to_ascii_lowercase().contains(filter))
}

fn node_sort_value(node: &NodeLifecycleSnapshot, sort: NodeSort) -> &str {
    match sort {
        NodeSort::Name => &node.name,
        NodeSort::Version => source_text(&node.version),
        NodeSort::ConfigHash => source_text(&node.config_hash),
    }
}

fn draw_string_source(ui: &mut egui::Ui, label: &str, source: &SourceSnapshot<String>) {
    match source {
        SourceSnapshot::Available(value) => {
            ui.colored_label(HEALTHY, format!("{label}: {value}"));
        }
        SourceSnapshot::Partial { value, warnings } => {
            ui.colored_label(WARNING, format!("{label}: {value} (partial)"));
            draw_warnings(ui, warnings);
        }
        SourceSnapshot::Unavailable { reason } => {
            ui.colored_label(UNKNOWN, format!("{label}: unavailable — {reason}"));
        }
    };
}

fn draw_time_source(ui: &mut egui::Ui, source: &SourceSnapshot<TimeSynchronizationAudit>) {
    match source {
        SourceSnapshot::Available(value) => draw_time_value(ui, value, HEALTHY),
        SourceSnapshot::Partial { value, warnings } => {
            draw_time_value(ui, value, WARNING);
            draw_warnings(ui, warnings);
        }
        SourceSnapshot::Unavailable { reason } => {
            ui.colored_label(
                UNKNOWN,
                format!("Time synchronization unavailable: {reason}"),
            );
        }
    }
}

fn draw_time_value(ui: &mut egui::Ui, value: &TimeSynchronizationAudit, source_color: Color32) {
    let status_color = if source_color == WARNING {
        WARNING
    } else if value.synced {
        HEALTHY
    } else {
        ERROR
    };
    ui.colored_label(
        status_color,
        format!(
            "Time synchronization: {} — server {}, offset {:.3}s",
            if value.synced {
                "synchronized"
            } else {
                "not synchronized"
            },
            value.server,
            value.offset_seconds,
        ),
    );
}

fn draw_warnings(ui: &mut egui::Ui, warnings: &[String]) {
    for warning in warnings {
        ui.colored_label(WARNING, format!("Partial source warning: {warning}"));
    }
}

fn source_text(source: &SourceSnapshot<String>) -> &str {
    match source {
        SourceSnapshot::Available(value) | SourceSnapshot::Partial { value, .. } => value,
        SourceSnapshot::Unavailable { .. } => "unavailable",
    }
}

fn certificate_expiry_label(expiry: CertificateExpiryStatus) -> &'static str {
    match expiry {
        CertificateExpiryStatus::Valid => "valid",
        CertificateExpiryStatus::ExpiringSoon => "expiring soon",
        CertificateExpiryStatus::ExpiringVerySoon => "expiring very soon",
        CertificateExpiryStatus::Expired => "expired",
    }
}

fn certificate_expiry_color(expiry: CertificateExpiryStatus) -> Color32 {
    match expiry {
        CertificateExpiryStatus::Valid => HEALTHY,
        CertificateExpiryStatus::ExpiringSoon => WARNING,
        CertificateExpiryStatus::ExpiringVerySoon | CertificateExpiryStatus::Expired => ERROR,
    }
}

fn certificate_source_label(source: CertificateSource) -> &'static str {
    match source {
        CertificateSource::TalosConfigCa => "talosconfig CA",
        CertificateSource::TalosConfigClient => "talosconfig client",
        CertificateSource::TalosKubeconfigCa => "Talos kubeconfig CA",
        CertificateSource::TalosKubeconfigClient => "Talos kubeconfig client",
    }
}

fn encryption_label(provider: &EncryptionProvider) -> String {
    match provider {
        EncryptionProvider::None => "no encryption provider reported".to_owned(),
        EncryptionProvider::Static => "static LUKS2".to_owned(),
        EncryptionProvider::NodeId => "node ID encryption".to_owned(),
        EncryptionProvider::Tpm => "TPM-backed encryption".to_owned(),
        EncryptionProvider::Kms => "KMS-backed encryption".to_owned(),
        EncryptionProvider::Other(value) => format!("unrecognized provider: {value}"),
    }
}

fn encryption_color(provider: &EncryptionProvider) -> Color32 {
    match provider {
        EncryptionProvider::Tpm | EncryptionProvider::Kms => HEALTHY,
        EncryptionProvider::None | EncryptionProvider::Static | EncryptionProvider::NodeId => {
            WARNING
        }
        EncryptionProvider::Other(_) => UNKNOWN,
    }
}

fn health_color(health: HealthIndicator) -> Color32 {
    match health {
        HealthIndicator::Healthy => HEALTHY,
        HealthIndicator::Warning => WARNING,
        HealthIndicator::Error => ERROR,
        HealthIndicator::Pending | HealthIndicator::Info => INFO,
        HealthIndicator::Unknown => UNKNOWN,
    }
}

fn quorum_label(quorum: &QuorumState) -> &'static str {
    match quorum {
        QuorumState::Healthy => "healthy",
        QuorumState::Degraded { .. } => "degraded but quorum maintained",
        QuorumState::NoQuorum { .. } => "no quorum",
        QuorumState::Unknown => "unknown",
    }
}

fn quorum_color(quorum: &QuorumState) -> Color32 {
    match quorum {
        QuorumState::Healthy => HEALTHY,
        QuorumState::Degraded { .. } => WARNING,
        QuorumState::NoQuorum { .. } => ERROR,
        QuorumState::Unknown => UNKNOWN,
    }
}

#[cfg(test)]
mod tests {
    use super::accepts_event;

    #[test]
    fn stale_event_is_rejected_after_context_or_request_changes() {
        assert!(accepts_event(
            Some("production"),
            "production",
            8,
            8,
            "production"
        ));
        assert!(!accepts_event(
            Some("staging"),
            "staging",
            8,
            8,
            "production"
        ));
        assert!(!accepts_event(
            Some("production"),
            "production",
            9,
            8,
            "production"
        ));
        assert!(!accepts_event(None, "production", 8, 8, "production"));
    }
}
