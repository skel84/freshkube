//! Native detailed etcd health screen.
//!
//! The Tokio worker owns collection. The egui thread owns presentation state
//! and accepts only immutable events for its currently selected Talos context.

use std::sync::mpsc;

use eframe::egui::{self, Color32, RichText};
use talos_pilot_core::{
    indicators::QuorumState,
    inspection::{
        EtcdHealthSnapshot, EtcdInspectionRequest, EtcdMemberSnapshot, InspectionTarget,
        collect_etcd_health,
    },
};
use tokio::runtime::Handle;

use crate::screens::ScreenTarget;

const HEALTHY: Color32 = Color32::from_rgb(76, 175, 80);
const WARNING: Color32 = Color32::from_rgb(255, 193, 7);
const ERROR: Color32 = Color32::from_rgb(244, 67, 54);
const UNKNOWN: Color32 = Color32::from_rgb(158, 158, 158);

/// Identity assigned to an in-flight sample.
///
/// The context name prevents a result for a previously selected context from
/// reaching this route, while the sequence discards older refreshes.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RequestIdentity {
    id: u64,
    context: String,
}

impl RequestIdentity {
    fn matches(&self, other: &Self) -> bool {
        self.id == other.id && self.context == other.context
    }
}

fn accepts_event(
    active_context: Option<&str>,
    visible_context: &str,
    active_request: Option<&RequestIdentity>,
    event_request: &RequestIdentity,
) -> bool {
    active_context == Some(visible_context)
        && event_request.context == visible_context
        && active_request.is_some_and(|request| request.matches(event_request))
}

/// Immutable worker result returned to the egui thread.
enum EtcdEvent {
    Collected {
        request: RequestIdentity,
        snapshot: EtcdHealthSnapshot,
    },
    Failed {
        request: RequestIdentity,
        message: String,
    },
}

/// Egui state for the detailed, cluster-scoped etcd health inspection.
pub(crate) struct EtcdScreen {
    event_tx: mpsc::Sender<EtcdEvent>,
    event_rx: mpsc::Receiver<EtcdEvent>,
    target_context: Option<String>,
    next_request_id: u64,
    active_request: Option<RequestIdentity>,
    snapshot: Option<EtcdHealthSnapshot>,
    latest_error: Option<String>,
    member_filter: String,
    selected_member: Option<u64>,
}

impl Default for EtcdScreen {
    fn default() -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        Self {
            event_tx,
            event_rx,
            target_context: None,
            next_request_id: 0,
            active_request: None,
            snapshot: None,
            latest_error: None,
            member_filter: String::new(),
            selected_member: None,
        }
    }
}

impl EtcdScreen {
    /// Render the detailed etcd health route for the selected Talos context.
    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        let context = target.context_name().to_owned();
        if self.target_context.as_deref() != Some(context.as_str()) {
            self.reset_for_target(context);
            self.start_refresh(target, runtime, ctx);
        }
        self.drain_events(target.context_name());

        let refresh_requested = self.draw_toolbar(ui, target.context_name());
        if refresh_requested {
            self.start_refresh(target, runtime, ctx);
        }

        self.draw_content(ui, target.context_name());
    }

    /// Stop accepting work from the prior visit to this route.
    ///
    /// The spawned Tokio task may finish after deactivation, but its tagged
    /// event cannot match a future request identity.
    pub(crate) fn deactivate(&mut self) {
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.target_context = None;
        self.active_request = None;
        self.snapshot = None;
        self.latest_error = None;
        self.member_filter.clear();
        self.selected_member = None;
    }

    fn reset_for_target(&mut self, context: String) {
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.target_context = Some(context);
        self.active_request = None;
        self.snapshot = None;
        self.latest_error = None;
        self.member_filter.clear();
        self.selected_member = None;
    }

    fn start_refresh(&mut self, target: &ScreenTarget<'_>, runtime: &Handle, ctx: &egui::Context) {
        if self.active_request.is_some() {
            return;
        }

        let context = target.context_name().trim();
        if context.is_empty() {
            self.latest_error =
                Some("No Talos context is selected for etcd health collection.".to_owned());
            return;
        }

        let Some(client) = target.cluster_client() else {
            self.latest_error = Some(format!(
                "No Talos client is available for context {context:?}. Refresh the context connection before collecting etcd health."
            ));
            return;
        };
        let Some(address) = target.control_plane_address() else {
            self.latest_error = Some(format!(
                "No control-plane endpoint is available for context {context:?}; etcd health cannot be collected safely."
            ));
            return;
        };

        self.next_request_id = self.next_request_id.wrapping_add(1);
        let request = RequestIdentity {
            id: self.next_request_id,
            context: context.to_owned(),
        };
        self.active_request = Some(request.clone());
        self.latest_error = None;

        let inspection = EtcdInspectionRequest::new(InspectionTarget::new(context, address));
        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        runtime.spawn(async move {
            let event = match collect_etcd_health(client, inspection).await {
                Ok(snapshot) => EtcdEvent::Collected { request, snapshot },
                Err(error) => EtcdEvent::Failed {
                    request,
                    message: error.to_string(),
                },
            };
            let _ = event_tx.send(event);
            repaint.request_repaint();
        });
    }

    fn drain_events(&mut self, visible_context: &str) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                EtcdEvent::Collected { request, snapshot }
                    if self.accepts(&request, visible_context) =>
                {
                    self.active_request = None;
                    self.latest_error = None;
                    self.snapshot = Some(snapshot);
                    self.reconcile_selection();
                }
                EtcdEvent::Failed { request, message }
                    if self.accepts(&request, visible_context) =>
                {
                    self.active_request = None;
                    self.latest_error = Some(message);
                }
                _ => {
                    // This event belongs to a previous context or refresh.
                }
            }
        }
    }

    fn accepts(&self, request: &RequestIdentity, visible_context: &str) -> bool {
        accepts_event(
            self.target_context.as_deref(),
            visible_context,
            self.active_request.as_ref(),
            request,
        )
    }

    fn reconcile_selection(&mut self) {
        if self.selected_member.is_some_and(|member_id| {
            !self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot
                    .members
                    .iter()
                    .any(|member| member.info.id == member_id)
            })
        }) {
            self.selected_member = None;
        }
    }

    fn draw_toolbar(&self, ui: &mut egui::Ui, context: &str) -> bool {
        let mut refresh_requested = false;
        ui.horizontal(|ui| {
            ui.heading("etcd health");
            ui.separator();
            ui.label(RichText::new(format!("Context: {context}")).weak());
            if let Some(request) = &self.active_request {
                ui.spinner();
                ui.label(format!("Refreshing {}…", request.context));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                refresh_requested = ui
                    .add_enabled(
                        self.active_request.is_none(),
                        egui::Button::new("Refresh etcd health"),
                    )
                    .clicked();
            });
        });
        ui.separator();
        refresh_requested
    }

    fn draw_content(&mut self, ui: &mut egui::Ui, context: &str) {
        if self.snapshot.is_none() {
            if let Some(error) = &self.latest_error {
                draw_error(ui, "etcd health unavailable", error);
            } else {
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Loading etcd health for {context}…"));
                });
            }
            return;
        }

        if let Some(error) = &self.latest_error {
            draw_error(ui, "Latest etcd refresh failed", error);
            ui.add_space(8.0);
        }

        let has_members = {
            let snapshot = self.snapshot.as_ref().expect("checked above");
            draw_summary(ui, snapshot);
            draw_partial_warnings(ui, snapshot);
            !snapshot.members.is_empty()
        };

        if !has_members {
            ui.add_space(12.0);
            ui.label(RichText::new("The authoritative etcd member list is empty.").color(UNKNOWN));
            return;
        }

        ui.add_space(12.0);
        ui.heading("Members");
        self.draw_member_controls(ui);
        let snapshot = self.snapshot.as_ref().expect("checked above");
        Self::draw_member_rows(ui, snapshot, &self.member_filter, &mut self.selected_member);
        Self::draw_selected_member(ui, snapshot, self.selected_member);
    }

    fn draw_member_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Filter members:");
            ui.add(
                egui::TextEdit::singleline(&mut self.member_filter)
                    .hint_text("name, ID, address, status"),
            );
            if ui.small_button("Clear filter").clicked() {
                self.member_filter.clear();
            }
        });
    }

    fn draw_member_rows(
        ui: &mut egui::Ui,
        snapshot: &EtcdHealthSnapshot,
        member_filter: &str,
        selected_member: &mut Option<u64>,
    ) {
        let filter = member_filter.trim().to_ascii_lowercase();
        if !snapshot
            .members
            .iter()
            .any(|member| member_matches(member, &filter))
        {
            ui.label(RichText::new("No etcd members match the current filter.").color(UNKNOWN));
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("etcd_member_list")
            .max_height(240.0)
            .show(ui, |ui| {
                egui::Grid::new("etcd_members")
                    .num_columns(5)
                    .striped(true)
                    .min_col_width(86.0)
                    .show(ui, |ui| {
                        ui.strong("Member");
                        ui.strong("Role");
                        ui.strong("Reachability");
                        ui.strong("Leader");
                        ui.strong("Problems");
                        ui.end_row();

                        for member in snapshot
                            .members
                            .iter()
                            .filter(|member| member_matches(member, &filter))
                        {
                            let selected = *selected_member == Some(member.info.id);
                            let label = format!("{} ({:#x})", member.info.hostname, member.info.id);
                            if ui.selectable_label(selected, label).clicked() {
                                *selected_member = Some(member.info.id);
                            }
                            ui.label(if member.info.is_learner {
                                "learner"
                            } else {
                                "voting"
                            });
                            let (reachability, reachability_color) = if member.is_reachable() {
                                ("reported status", HEALTHY)
                            } else {
                                ("no status", WARNING)
                            };
                            ui.colored_label(reachability_color, reachability);
                            ui.colored_label(
                                if member.is_leader() { HEALTHY } else { UNKNOWN },
                                if member.is_leader() { "leader" } else { "—" },
                            );
                            let (problems, problem_color) = if member.has_problems() {
                                ("reported", ERROR)
                            } else {
                                ("none", HEALTHY)
                            };
                            ui.colored_label(problem_color, problems);
                            ui.end_row();
                        }
                    });
            });
    }

    fn draw_selected_member(
        ui: &mut egui::Ui,
        snapshot: &EtcdHealthSnapshot,
        selected_member: Option<u64>,
    ) {
        let Some(member_id) = selected_member else {
            ui.add_space(8.0);
            ui.label(
                RichText::new("Select a member to inspect its reported details.").color(UNKNOWN),
            );
            return;
        };
        let Some(member) = snapshot
            .members
            .iter()
            .find(|member| member.info.id == member_id)
        else {
            return;
        };

        let details = member_details(member);
        ui.add_space(12.0);
        egui::CollapsingHeader::new(format!("Selected member details: {}", member.info.hostname))
            .id_salt(("etcd_member_details", member.info.id))
            .default_open(true)
            .show(ui, |ui| {
                if ui.button("Copy details").clicked() {
                    ui.ctx().copy_text(details.clone());
                }
                ui.add(
                    egui::Label::new(RichText::new(&details).monospace())
                        .selectable(true)
                        .wrap(),
                );
            });
    }
}

fn draw_error(ui: &mut egui::Ui, title: &str, message: &str) {
    ui.colored_label(ERROR, title);
    ui.add(
        egui::Label::new(RichText::new(message).color(ERROR))
            .selectable(true)
            .wrap(),
    );
}

fn draw_summary(ui: &mut egui::Ui, snapshot: &EtcdHealthSnapshot) {
    ui.heading("Cluster health");
    let leader = snapshot
        .leader_id()
        .and_then(|leader_id| {
            snapshot
                .members
                .iter()
                .find(|member| member.info.id == leader_id)
                .map(|member| format!("{} ({leader_id:#x})", member.info.hostname))
                .or_else(|| Some(format!("{leader_id:#x}")))
        })
        .unwrap_or_else(|| {
            if snapshot.reported_leader_ids.is_empty() {
                "no leader reported".to_owned()
            } else {
                format!("conflicting reports: {:?}", snapshot.reported_leader_ids)
            }
        });

    egui::Grid::new("etcd_health_summary")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Sample endpoint");
            ui.label(format!(
                "{} ({})",
                snapshot.target.name, snapshot.target.address
            ));
            ui.end_row();
            ui.label("Quorum");
            ui.colored_label(
                quorum_color(&snapshot.quorum),
                quorum_label(&snapshot.quorum),
            );
            ui.end_row();
            ui.label("Responding voters");
            ui.label(format!(
                "{} of {}",
                snapshot.responding_voting_members, snapshot.voting_members
            ));
            ui.end_row();
            ui.label("Reported leader");
            ui.label(leader);
            ui.end_row();
            ui.label("Largest reported DB");
            ui.label(snapshot.largest_database_size_display());
            ui.end_row();
            ui.label("Largest reported raft index");
            ui.label(snapshot.revision.to_string());
            ui.end_row();
        });
}

fn draw_partial_warnings(ui: &mut egui::Ui, snapshot: &EtcdHealthSnapshot) {
    if !snapshot.is_partial()
        && snapshot.unmatched_statuses.is_empty()
        && snapshot.unmatched_alarms.is_empty()
    {
        return;
    }

    ui.add_space(8.0);
    ui.colored_label(WARNING, "Partial or uncorrelated etcd data");
    for unavailable in &snapshot.unavailable {
        ui.add(
            egui::Label::new(format!("{}: {}", unavailable.source, unavailable.message))
                .selectable(true)
                .wrap(),
        );
    }
    for status in &snapshot.unmatched_statuses {
        ui.add(
            egui::Label::new(format!(
                "Uncorrelated status from {} for member {:#x}.",
                status.node, status.member_id
            ))
            .selectable(true),
        );
    }
    for alarm in &snapshot.unmatched_alarms {
        ui.add(
            egui::Label::new(format!(
                "Uncorrelated {} alarm from {} for member {:#x}.",
                alarm.alarm_type.as_str(),
                alarm.node,
                alarm.member_id
            ))
            .selectable(true),
        );
    }
}

fn member_matches(member: &EtcdMemberSnapshot, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }

    let mut searchable = format!(
        "{} {:#x} {} {}",
        member.info.hostname,
        member.info.id,
        member.info.peer_urls.join(" "),
        member.info.client_urls.join(" "),
    );
    if let Some(status) = &member.status {
        searchable.push_str(&format!(" {} {}", status.node, status.protocol_version));
        for error in &status.errors {
            searchable.push(' ');
            searchable.push_str(error);
        }
    }
    for alarm in &member.alarms {
        searchable.push(' ');
        searchable.push_str(alarm.alarm_type.as_str());
    }
    searchable.to_ascii_lowercase().contains(filter)
}

fn member_details(member: &EtcdMemberSnapshot) -> String {
    let mut details = format!(
        "Member: {}\nID: {:#x}\nRole: {}\nPeer URLs: {}\nClient URLs: {}",
        member.info.hostname,
        member.info.id,
        if member.info.is_learner {
            "learner"
        } else {
            "voting"
        },
        list_or_unavailable(&member.info.peer_urls),
        list_or_unavailable(&member.info.client_urls),
    );

    match &member.status {
        Some(status) => {
            details.push_str(&format!(
                "\n\nStatus reporter: {}\nProtocol version: {}\nLeader ID: {:#x}\nDatabase size: {}\nDatabase size in use: {} ({:.1}%)\nRaft index: {}\nRaft term: {}\nRaft applied index: {}\nStatus role: {}",
                status.node,
                status.protocol_version,
                status.leader_id,
                status.db_size_human(),
                status.db_size_in_use_human(),
                status.db_usage_percent(),
                status.raft_index,
                status.raft_term,
                status.raft_applied_index,
                if status.is_learner { "learner" } else { "voting" },
            ));
            if status.errors.is_empty() {
                details.push_str("\nReported errors: none");
            } else {
                details.push_str("\nReported errors:");
                for error in &status.errors {
                    details.push_str(&format!("\n- {error}"));
                }
            }
        }
        None => details.push_str("\n\nStatus: unavailable for this member."),
    }

    if member.alarms.is_empty() {
        details.push_str("\n\nAlarms: none");
    } else {
        details.push_str("\n\nAlarms:");
        for alarm in &member.alarms {
            details.push_str(&format!(
                "\n- {} reported by {}",
                alarm.alarm_type.as_str(),
                alarm.node
            ));
        }
    }
    details
}

fn list_or_unavailable(values: &[String]) -> String {
    if values.is_empty() {
        "unavailable".to_owned()
    } else {
        values.join(", ")
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
    use super::{RequestIdentity, accepts_event};

    #[test]
    fn stale_context_or_request_is_rejected() {
        let current = RequestIdentity {
            id: 9,
            context: "production".to_owned(),
        };
        assert!(accepts_event(
            Some("production"),
            "production",
            Some(&current),
            &current,
        ));
        assert!(!accepts_event(
            Some("production"),
            "production",
            Some(&current),
            &RequestIdentity {
                id: 8,
                context: "production".to_owned(),
            },
        ));
        assert!(!accepts_event(
            Some("staging"),
            "staging",
            Some(&current),
            &current,
        ));
    }
}
