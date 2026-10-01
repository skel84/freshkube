//! Native process inspection screen backed by the shared collector.
//!
//! Collection runs exclusively on the application's Tokio runtime. The egui
//! thread retains presentation state and drains immutable worker results.

use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, RichText};
use talos_pilot_core::inspection::{
    InspectionTarget, ProcessFilter, ProcessInspectionRequest, ProcessInspectionSnapshot,
    ProcessSampleState, ProcessSort, ProcessTree, ProcessView, build_process_display_rows,
    collect_process_inspection,
};
use talos_rs::ProcessState;
use tokio::runtime::Handle;

use crate::screens::ScreenTarget;

const AUTO_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const WARNING: Color32 = Color32::from_rgb(255, 193, 7);
const ERROR: Color32 = Color32::from_rgb(244, 67, 54);
const MUTED: Color32 = Color32::from_rgb(158, 158, 158);

/// Identity of the currently rendered process target.
///
/// The context, node name, and address are all retained because a selected
/// row's name alone is not sufficient to safely apply a worker result.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ProcessTarget {
    context: String,
    inspection: Option<InspectionTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RequestIdentity {
    sequence: u64,
    context: String,
    target: InspectionTarget,
}

impl RequestIdentity {
    fn matches(&self, event: &Self) -> bool {
        self.sequence == event.sequence
            && self.context == event.context
            && self.target == event.target
    }
}

enum ProcessEvent {
    Completed {
        request: RequestIdentity,
        result: Result<ProcessInspectionSnapshot, String>,
    },
}

/// Egui state for node-scoped process inspection.
///
/// Sampling state is owned by this screen so CPU deltas remain meaningful only
/// while the same [`InspectionTarget`] remains selected.
pub(crate) struct ProcessScreen {
    events: mpsc::Receiver<ProcessEvent>,
    event_tx: mpsc::Sender<ProcessEvent>,
    target: Option<ProcessTarget>,
    next_sequence: u64,
    active_request: Option<RequestIdentity>,
    snapshot: Option<ProcessInspectionSnapshot>,
    sample: ProcessSampleState,
    blocked: Option<String>,
    error: Option<String>,
    last_refresh: Option<Instant>,
    refresh_when_active: bool,
    command_filter: String,
    view: ProcessView,
    selected_pid: Option<i32>,
}

impl Default for ProcessScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessScreen {
    pub(crate) fn new() -> Self {
        let (event_tx, events) = mpsc::channel();
        Self {
            events,
            event_tx,
            target: None,
            next_sequence: 0,
            active_request: None,
            snapshot: None,
            sample: ProcessSampleState::default(),
            blocked: None,
            error: None,
            last_refresh: None,
            refresh_when_active: true,
            command_filter: String::new(),
            view: ProcessView {
                sort: ProcessSort::CpuPercent,
                filter: ProcessFilter::default(),
                tree: ProcessTree::Flat,
            },
            selected_pid: None,
        }
    }

    /// Invalidate any pending worker result while this route is not visible.
    ///
    /// Tokio tasks may finish after this call, but their sequence no longer
    /// matches an active request and so cannot alter route-local UI state.
    pub(crate) fn deactivate(&mut self) {
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.active_request = None;
        self.last_refresh = None;
        self.refresh_when_active = true;
    }

    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        self.sync_target(target);
        self.drain_events();

        let manual_refresh = self.draw_header(ui, target.context_name());
        if manual_refresh || self.refresh_when_active || self.auto_refresh_due() {
            self.start_refresh(target, runtime, ctx);
        }
        if self.active_request.is_some() || (self.blocked.is_none() && self.last_refresh.is_some())
        {
            ctx.request_repaint_after(Duration::from_secs(1));
        }

        self.draw_content(ui);
    }

    fn sync_target(&mut self, target: &ScreenTarget<'_>) {
        let next = ProcessTarget {
            context: target.context_name().to_owned(),
            inspection: target_identity(target),
        };
        if self.target.as_ref() == Some(&next) {
            return;
        }

        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.target = Some(next);
        self.active_request = None;
        self.snapshot = None;
        self.sample = ProcessSampleState::default();
        self.blocked = None;
        self.error = None;
        self.last_refresh = None;
        self.refresh_when_active = true;
        self.selected_pid = None;
        if matches!(self.view.tree, ProcessTree::Subtree { .. }) {
            self.view.tree = ProcessTree::Flat;
        }
    }

    fn draw_header(&self, ui: &mut egui::Ui, context: &str) -> bool {
        let identity = self
            .target
            .as_ref()
            .and_then(|target| target.inspection.as_ref());
        ui.horizontal(|ui| {
            match identity {
                Some(target) => ui.heading(format!(
                    "Processes — {context} / {} ({})",
                    target.name, target.address
                )),
                None => ui.heading(format!("Processes — {context}")),
            };
        });

        let mut refresh = false;
        ui.horizontal(|ui| {
            refresh = ui
                .add_enabled(
                    self.active_request.is_none(),
                    egui::Button::new("Refresh processes"),
                )
                .clicked();
            if self.active_request.is_some() {
                ui.spinner();
                ui.label("Collecting process data on a Tokio worker…");
            }
        });
        refresh
    }

    fn start_refresh(&mut self, target: &ScreenTarget<'_>, runtime: &Handle, ctx: &egui::Context) {
        if self.active_request.is_some() {
            return;
        }

        self.refresh_when_active = false;
        self.last_refresh = Some(Instant::now());
        let Some(ProcessTarget {
            context,
            inspection: Some(inspection),
        }) = self.target.clone()
        else {
            self.blocked = Some(target_unavailable_reason(target));
            return;
        };
        let Some(client) = target.node_client() else {
            self.blocked = Some(format!(
                "No Talos client is available for node {:?} in context {:?}.",
                inspection.name, context
            ));
            return;
        };

        self.next_sequence = self.next_sequence.wrapping_add(1);
        let request = RequestIdentity {
            sequence: self.next_sequence,
            context: context.clone(),
            target: inspection.clone(),
        };
        let sample = (self.sample.target.as_ref() == Some(&inspection))
            .then(|| self.sample.clone())
            .unwrap_or_default();
        self.active_request = Some(request.clone());
        self.blocked = None;
        self.error = None;

        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        runtime.spawn(async move {
            let result = collect_process_inspection(
                client,
                ProcessInspectionRequest::new(request.target.clone(), sample),
            )
            .await
            .map_err(|error| error.to_string());
            let _ = event_tx.send(ProcessEvent::Completed { request, result });
            repaint.request_repaint();
        });
    }

    fn drain_events(&mut self) {
        while let Ok(ProcessEvent::Completed { request, result }) = self.events.try_recv() {
            if !self.accepts(&request) {
                continue;
            }
            self.active_request = None;
            match result {
                Ok(snapshot) => {
                    self.sample = snapshot.next_sample.clone();
                    self.reconcile_selection(&snapshot);
                    self.snapshot = Some(snapshot);
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
    }

    fn accepts(&self, event: &RequestIdentity) -> bool {
        self.active_request
            .as_ref()
            .is_some_and(|request| request.matches(event))
            && self.target.as_ref().is_some_and(|target| {
                target.context == event.context && target.inspection.as_ref() == Some(&event.target)
            })
    }

    fn auto_refresh_due(&self) -> bool {
        self.active_request.is_none()
            && self.blocked.is_none()
            && self
                .last_refresh
                .is_some_and(|last| last.elapsed() >= AUTO_REFRESH_INTERVAL)
    }

    fn draw_content(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        if let Some(reason) = &self.blocked {
            ui.colored_label(ERROR, format!("Process inspection unavailable: {reason}"));
            return;
        }
        if let Some(error) = &self.error {
            ui.colored_label(ERROR, format!("Process collection failed: {error}"));
        }
        if self.snapshot.is_none() {
            if self.active_request.is_some() {
                ui.label(RichText::new("Waiting for the first process sample…").color(MUTED));
            } else {
                ui.label(RichText::new("No process sample is available yet.").color(MUTED));
            }
            return;
        }

        self.draw_controls(ui);
        self.draw_summary(ui);
        self.draw_process_rows(ui);
        self.draw_selected_process(ui);
    }

    fn draw_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Command filter:");
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.command_filter)
                    .hint_text("command, executable, or arguments")
                    .desired_width(250.0),
            );
            if response.changed() {
                self.view.filter.text = (!self.command_filter.trim().is_empty())
                    .then(|| self.command_filter.trim().to_owned());
            }
            if ui.small_button("Clear filter").clicked() {
                self.command_filter.clear();
                self.view.filter.text = None;
            }

            egui::ComboBox::from_label("State")
                .selected_text(process_state_filter_label(self.view.filter.state.as_ref()))
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(self.view.filter.state.is_none(), "All states")
                        .clicked()
                    {
                        self.view.filter.state = None;
                    }
                    for (label, state) in process_state_filters() {
                        if ui
                            .selectable_label(
                                self.view.filter.state.as_ref() == Some(&state),
                                label,
                            )
                            .clicked()
                        {
                            self.view.filter.state = Some(state);
                        }
                    }
                });

            egui::ComboBox::from_label("Sort")
                .selected_text(process_sort_label(self.view.sort))
                .show_ui(ui, |ui| {
                    for sort in [
                        ProcessSort::CpuPercent,
                        ProcessSort::CpuTime,
                        ProcessSort::ResidentMemory,
                    ] {
                        ui.selectable_value(&mut self.view.sort, sort, process_sort_label(sort));
                    }
                });

            ui.label("Layout:");
            if ui
                .selectable_label(matches!(self.view.tree, ProcessTree::Flat), "Flat")
                .clicked()
            {
                self.view.tree = ProcessTree::Flat;
            }
            if ui
                .selectable_label(matches!(self.view.tree, ProcessTree::Full), "Tree")
                .clicked()
            {
                self.view.tree = ProcessTree::Full;
            }
            let subtree_selected = matches!(self.view.tree, ProcessTree::Subtree { .. });
            if ui
                .add_enabled(
                    self.selected_pid.is_some(),
                    egui::Button::selectable(subtree_selected, "Selected subtree"),
                )
                .clicked()
            {
                if let Some(root_pid) = self.selected_pid {
                    self.view.tree = ProcessTree::Subtree { root_pid };
                }
            }
        });
    }

    fn draw_summary(&self, ui: &mut egui::Ui) {
        let snapshot = self.snapshot.as_ref().expect("checked by draw_content");
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let cpu = snapshot
                .system
                .cpu
                .as_ref()
                .and_then(|cpu| cpu.usage_display())
                .unwrap_or_else(|| "warming up or unavailable".to_owned());
            let cpu_count = snapshot
                .system
                .cpu_count
                .map(|count| format!(" ({count} CPU(s))"))
                .unwrap_or_default();
            summary_metric(ui, "CPU", format!("{cpu}{cpu_count}"));
            summary_metric(
                ui,
                "Memory",
                snapshot
                    .system
                    .memory
                    .as_ref()
                    .map(|memory| memory.display())
                    .unwrap_or_else(|| "unavailable".to_owned()),
            );
            summary_metric(
                ui,
                "Load",
                snapshot
                    .system
                    .load_average
                    .map(|load| {
                        format!(
                            "{:.2} / {:.2} / {:.2}",
                            load.one_minute, load.five_minutes, load.fifteen_minutes
                        )
                    })
                    .unwrap_or_else(|| "unavailable".to_owned()),
            );
            summary_metric(
                ui,
                "Processes",
                format!(
                    "{} total; {} running, {} sleeping, {} disk wait, {} zombie",
                    snapshot.processes.len(),
                    snapshot.state_counts.running,
                    snapshot.state_counts.sleeping,
                    snapshot.state_counts.disk_sleep,
                    snapshot.state_counts.zombie
                ),
            );
            if let Some(cpu) = &snapshot.system.cpu {
                summary_metric(
                    ui,
                    "Scheduler",
                    format!(
                        "{} running, {} blocked",
                        cpu.running_processes, cpu.blocked_processes
                    ),
                );
            }
        });

        if snapshot.is_partial() {
            ui.colored_label(
                WARNING,
                "Partial metrics: process rows loaded, but one or more supplementary sources are unavailable.",
            );
            ui.collapsing("Unavailable sources", |ui| {
                for unavailable in &snapshot.unavailable {
                    ui.label(format!("{}: {}", unavailable.source, unavailable.message));
                }
            });
        }
    }

    fn draw_process_rows(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.snapshot.as_ref().expect("checked by draw_content");
        let rows = build_process_display_rows(&snapshot.processes, &self.view);
        ui.add_space(8.0);
        ui.heading("Processes");
        if snapshot.processes.is_empty() {
            ui.label(RichText::new("Talos reported no processes for this node.").color(MUTED));
            return;
        }
        if rows.is_empty() {
            ui.label(RichText::new("No processes match the current filters.").color(MUTED));
            return;
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("PID").strong());
            ui.add_space(24.0);
            ui.label(RichText::new("State").strong());
            ui.add_space(8.0);
            ui.label(RichText::new("CPU %").strong());
            ui.add_space(12.0);
            ui.label(RichText::new("CPU time").strong());
            ui.add_space(12.0);
            ui.label(RichText::new("Memory").strong());
            ui.add_space(12.0);
            ui.label(RichText::new("Threads").strong());
            ui.add_space(12.0);
            ui.label(RichText::new("Command (select text to copy)").strong());
        });

        let target_id = self
            .target
            .as_ref()
            .and_then(|target| target.inspection.as_ref())
            .map(|target| (target.name.as_str(), target.address.as_str()));
        let mut selected_pid = None;
        egui::ScrollArea::vertical()
            .id_salt(("process_rows", target_id))
            .max_height(340.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new(("process_table", target_id))
                    .num_columns(7)
                    .striped(true)
                    .min_col_width(58.0)
                    .show(ui, |ui| {
                        for row in &rows {
                            let entry = &snapshot.processes[row.process_index];
                            let process = &entry.process;
                            ui.add(egui::Label::new(process.pid.to_string()).selectable(true));
                            ui.add(egui::Label::new(process.state.short()).selectable(true));
                            ui.add(
                                egui::Label::new(
                                    entry
                                        .cpu_percent_display()
                                        .unwrap_or_else(|| "—".to_owned()),
                                )
                                .selectable(true),
                            );
                            ui.add(egui::Label::new(process.cpu_time_human()).selectable(true));
                            ui.add(
                                egui::Label::new(entry.resident_memory_display()).selectable(true),
                            );
                            ui.add(egui::Label::new(process.threads.to_string()).selectable(true));
                            let command = format!(
                                "{}{}",
                                tree_prefix(row.depth, row.is_last, &row.ancestors_have_siblings),
                                process.display_command()
                            );
                            if ui
                                .add_sized(
                                    [460.0, 0.0],
                                    egui::Button::selectable(
                                        self.selected_pid == Some(process.pid),
                                        RichText::new(command).monospace(),
                                    ),
                                )
                                .clicked()
                            {
                                selected_pid = Some(process.pid);
                            }
                            ui.end_row();
                        }
                    });
            });
        if let Some(pid) = selected_pid {
            self.selected_pid = Some(pid);
        }
    }

    fn draw_selected_process(&mut self, ui: &mut egui::Ui) {
        let Some(pid) = self.selected_pid else {
            return;
        };
        let Some(entry) = self.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .processes
                .iter()
                .find(|entry| entry.process.pid == pid)
        }) else {
            self.selected_pid = None;
            if matches!(self.view.tree, ProcessTree::Subtree { .. }) {
                self.view.tree = ProcessTree::Flat;
            }
            return;
        };
        let process = &entry.process;
        let command = process.display_command().to_owned();

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Process {}", process.pid)).strong());
                ui.label(format!(
                    "{} — {}",
                    process.state.short(),
                    process.state.description()
                ));
                if ui.button("Copy command").clicked() {
                    ui.ctx().copy_text(command.clone());
                }
            });
            egui::Grid::new(("process_detail", process.pid))
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    detail_row(ui, "Parent PID", process.ppid.to_string());
                    detail_row(ui, "Threads", process.threads.to_string());
                    detail_row(ui, "CPU time", process.cpu_time_human());
                    detail_row(
                        ui,
                        "CPU percentage",
                        entry
                            .cpu_percent_display()
                            .unwrap_or_else(|| "warming up".to_owned()),
                    );
                    detail_row(ui, "Resident memory", entry.resident_memory_display());
                    detail_row(ui, "Virtual memory", process.virtual_memory_human());
                    detail_row(ui, "Executable", process.executable.clone());
                    detail_row(ui, "Arguments", process.args.clone());
                    detail_row(ui, "Command", command.clone());
                });
        });
    }

    fn reconcile_selection(&mut self, snapshot: &ProcessInspectionSnapshot) {
        if self.selected_pid.is_some_and(|pid| {
            !snapshot
                .processes
                .iter()
                .any(|entry| entry.process.pid == pid)
        }) {
            self.selected_pid = None;
            if matches!(self.view.tree, ProcessTree::Subtree { .. }) {
                self.view.tree = ProcessTree::Flat;
            }
        }
    }
}

fn target_identity(target: &ScreenTarget<'_>) -> Option<InspectionTarget> {
    Some(InspectionTarget::new(
        target.node_name()?.to_owned(),
        target.node_address()?,
    ))
}

fn target_unavailable_reason(target: &ScreenTarget<'_>) -> String {
    match target.node_name() {
        None => "Select a node before inspecting its processes.".to_owned(),
        Some(node) if target.node_address().is_none() => {
            format!("Node {node:?} has no Talos management address.")
        }
        Some(node) => format!("Node {node:?} has no usable Talos client."),
    }
}

fn process_sort_label(sort: ProcessSort) -> &'static str {
    match sort {
        ProcessSort::CpuPercent => "CPU %",
        ProcessSort::CpuTime => "CPU time",
        ProcessSort::ResidentMemory => "Memory",
    }
}

fn process_state_filter_label(state: Option<&ProcessState>) -> String {
    state.map_or_else(
        || "All states".to_owned(),
        |state| state.description().to_owned(),
    )
}

fn process_state_filters() -> [(&'static str, ProcessState); 7] {
    [
        ("Running", ProcessState::Running),
        ("Sleeping", ProcessState::Sleeping),
        ("Disk sleep", ProcessState::DiskSleep),
        ("Zombie", ProcessState::Zombie),
        ("Stopped", ProcessState::Stopped),
        ("Tracing", ProcessState::TracingStop),
        ("Dead", ProcessState::Dead),
    ]
}

fn tree_prefix(depth: usize, is_last: bool, ancestors_have_siblings: &[bool]) -> String {
    if depth == 0 {
        return String::new();
    }
    let mut prefix = String::with_capacity(depth.saturating_mul(3));
    for continues in ancestors_have_siblings {
        prefix.push_str(if *continues { "│  " } else { "   " });
    }
    prefix.push_str(if is_last { "└─ " } else { "├─ " });
    prefix
}

fn summary_metric(ui: &mut egui::Ui, label: &str, value: String) {
    ui.group(|ui| {
        ui.label(RichText::new(label).strong());
        ui.label(value);
    });
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: String) {
    ui.label(label);
    ui.add(egui::Label::new(RichText::new(value).monospace()).selectable(true));
    ui.end_row();
}

#[cfg(test)]
mod tests {
    use super::RequestIdentity;
    use talos_pilot_core::inspection::InspectionTarget;

    #[test]
    fn stale_request_requires_matching_context_node_and_address() {
        let current = RequestIdentity {
            sequence: 4,
            context: "production".to_owned(),
            target: InspectionTarget::new("control-1", "10.0.0.10"),
        };
        assert!(current.matches(&current));
        assert!(!current.matches(&RequestIdentity {
            sequence: 4,
            context: "staging".to_owned(),
            target: InspectionTarget::new("control-1", "10.0.0.10"),
        }));
        assert!(!current.matches(&RequestIdentity {
            sequence: 4,
            context: "production".to_owned(),
            target: InspectionTarget::new("control-1", "10.0.0.11"),
        }));
        assert!(!current.matches(&RequestIdentity {
            sequence: 4,
            context: "production".to_owned(),
            target: InspectionTarget::new("control-2", "10.0.0.10"),
        }));
    }
}
