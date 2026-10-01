//! Native single-service and Stern-style multi-service Talos log viewer.
//!
//! The egui thread owns filtering, selection, and the bounded core log models.
//! Talos calls and file writes run on the supplied Tokio runtime; workers send
//! immutable events back through this screen-owned bounded channel.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use eframe::egui::{self, Color32, RichText};
use talos_pilot_core::{
    logs::{
        LogBuffer, LogEvent, LogFilters, LogTarget, MultiServiceLogs, ServiceId, SingleServiceLogs,
    },
    types::LogLevel,
};
use tokio::{
    runtime::Handle,
    sync::{mpsc, oneshot},
};

use crate::screens::ScreenTarget;

const INITIAL_TAIL_LINES: i32 = 500;
const STREAM_TAIL_LINES: i32 = 0;
const EVENT_QUEUE_CAPACITY: usize = 512;
const MAX_EVENTS_PER_FRAME: usize = 512;

const ERROR: Color32 = Color32::from_rgb(244, 67, 54);
const WARNING: Color32 = Color32::from_rgb(255, 193, 7);
const INFO: Color32 = Color32::from_rgb(100, 181, 246);
const MUTED: Color32 = Color32::from_rgb(158, 158, 158);

/// The immutable identity every worker event carries.
///
/// Node addresses are used rather than display names because Talos requests are
/// scoped to this exact management endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
struct TargetIdentity {
    context: String,
    node_address: String,
}

impl TargetIdentity {
    fn from_target(target: &ScreenTarget<'_>) -> Option<Self> {
        Some(Self {
            context: target.context_name().to_owned(),
            node_address: target.node_address()?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum LogView {
    #[default]
    Single,
    Multi,
}

impl LogView {
    const ALL: [Self; 2] = [Self::Single, Self::Multi];

    const fn label(self) -> &'static str {
        match self {
            Self::Single => "Single service",
            Self::Multi => "Multiple services",
        }
    }
}

#[derive(Debug)]
enum Availability {
    NoNode,
    NoClient,
    LoadingServices,
    ServiceListError(String),
    Ready,
}

/// Immutable work completed away from the egui thread.
///
/// Every variant carries [`TargetIdentity`] so a result for a prior context or
/// node can never be rendered for the current selection.
enum LogsEvent {
    ServicesLoaded {
        target: TargetIdentity,
        generation: u64,
        result: Result<Vec<ServiceId>, String>,
    },
    InitialLoaded {
        target: TargetIdentity,
        generation: u64,
        service: ServiceId,
        content: String,
    },
    InitialFailed {
        target: TargetIdentity,
        generation: u64,
        service: ServiceId,
        error: String,
    },
    StreamLine {
        target: TargetIdentity,
        generation: u64,
        event: LogEvent,
    },
    StreamStarted {
        target: TargetIdentity,
        generation: u64,
        service: ServiceId,
    },
    StreamFailed {
        target: TargetIdentity,
        generation: u64,
        service: ServiceId,
        error: String,
    },
    StreamEnded {
        target: TargetIdentity,
        generation: u64,
        service: ServiceId,
    },
    ExportCompleted {
        target: TargetIdentity,
        generation: u64,
        path: PathBuf,
        result: Result<(), String>,
    },
}

/// A task-local cancellation sender. Sending it makes the worker exit its
/// `select!`, which drops both its Talos stream receiver and this stop receiver.
struct StreamStop {
    stop: oneshot::Sender<()>,
}

#[derive(Clone, Copy, Debug)]
enum ScrollRequest {
    Entry(usize),
    End,
}

/// Framework-native state for the Logs route.
pub(crate) struct LogsScreen {
    events: mpsc::Receiver<LogsEvent>,
    event_tx: mpsc::Sender<LogsEvent>,
    active_target: Option<TargetIdentity>,
    generation: u64,
    availability: Availability,
    view: LogView,
    services: Vec<ServiceId>,
    selected_service: Option<ServiceId>,
    single: Option<SingleServiceLogs>,
    multi: Option<MultiServiceLogs>,
    stream_stops: Vec<StreamStop>,
    initial_loading: BTreeSet<ServiceId>,
    streaming: BTreeSet<ServiceId>,
    failures: BTreeMap<ServiceId, String>,
    scroll_request: Option<ScrollRequest>,
    notice: Option<String>,
}

impl Default for LogsScreen {
    fn default() -> Self {
        let (event_tx, events) = mpsc::channel(EVENT_QUEUE_CAPACITY);
        Self {
            events,
            event_tx,
            active_target: None,
            generation: 0,
            availability: Availability::NoNode,
            view: LogView::default(),
            services: Vec::new(),
            selected_service: None,
            single: None,
            multi: None,
            stream_stops: Vec::new(),
            initial_loading: BTreeSet::new(),
            streaming: BTreeSet::new(),
            failures: BTreeMap::new(),
            scroll_request: None,
            notice: None,
        }
    }
}

impl LogsScreen {
    /// Render the route and schedule any required Talos work on `runtime`.
    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        self.sync_target(target, runtime, ctx);
        self.drain_events(target, runtime, ctx);

        ui.heading("Logs");
        let Some(identity) = self.active_target.clone() else {
            self.draw_unavailable(ui);
            return;
        };

        ui.horizontal(|ui| {
            ui.label(RichText::new(&identity.context).strong());
            ui.label(
                RichText::new(identity.node_address)
                    .monospace()
                    .color(MUTED),
            );
        });
        ui.separator();

        if !matches!(&self.availability, Availability::Ready) {
            let retry_services = matches!(&self.availability, Availability::ServiceListError(_));
            self.draw_unavailable(ui);
            if retry_services && ui.button("Reload services").clicked() {
                self.reload_services(target, runtime, ctx);
            }
            return;
        }

        if self.services.is_empty() {
            ui.colored_label(
                MUTED,
                "The selected node reported no Talos services. Reload services after the node is ready.",
            );
            if ui.button("Reload services").clicked() {
                self.reload_services(target, runtime, ctx);
            }
            return;
        }

        let mut reload_services = false;
        let mut restart_logs = false;
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_label("View")
                .selected_text(self.view.label())
                .show_ui(ui, |ui| {
                    for view in LogView::ALL {
                        if ui
                            .selectable_value(&mut self.view, view, view.label())
                            .changed()
                        {
                            restart_logs = true;
                        }
                    }
                });

            if self.view == LogView::Single {
                let services = self.services.clone();
                let selected = self.selected_service.as_ref().map_or_else(
                    || "Choose a service".to_owned(),
                    |service| service.as_str().to_owned(),
                );
                egui::ComboBox::from_label("Service")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for service in services {
                            if ui
                                .selectable_value(
                                    &mut self.selected_service,
                                    Some(service.clone()),
                                    service.as_str(),
                                )
                                .changed()
                            {
                                restart_logs = true;
                            }
                        }
                    });
            }

            if ui.button("Refresh logs").clicked() {
                restart_logs = true;
            }
            if ui.button("Reload services").clicked() {
                reload_services = true;
            }
        });

        if reload_services {
            self.reload_services(target, runtime, ctx);
            return;
        }
        if restart_logs {
            self.start_log_session(target, runtime, ctx);
        }

        if self.active_buffer().is_none() {
            self.draw_stream_status(ui);
            return;
        }

        self.draw_stream_toolbar(ui, target, runtime, ctx);
        self.draw_filters(ui, target, runtime, ctx);
        self.draw_stream_status(ui);
        self.draw_log_text(ui);
    }

    /// Stop live workers and invalidate all outstanding work before the route is
    /// hidden. Each stop signal causes its task to drop its Talos receiver.
    pub(crate) fn deactivate(&mut self) {
        self.cancel_streams();
        self.generation = self.generation.wrapping_add(1);
        self.active_target = None;
        self.services.clear();
        self.selected_service = None;
        self.single = None;
        self.multi = None;
        self.failures.clear();
        self.notice = None;
        self.scroll_request = None;
        self.availability = Availability::NoNode;
    }

    fn sync_target(&mut self, target: &ScreenTarget<'_>, runtime: &Handle, ctx: &egui::Context) {
        let Some(identity) = TargetIdentity::from_target(target) else {
            if self.active_target.is_some() {
                self.deactivate();
            }
            self.availability = Availability::NoNode;
            return;
        };

        if self.active_target.as_ref() == Some(&identity) {
            if matches!(&self.availability, Availability::NoClient)
                && target.node_client().is_some()
            {
                self.reload_services(target, runtime, ctx);
            }
            return;
        }

        self.cancel_streams();
        self.generation = self.generation.wrapping_add(1);
        self.active_target = Some(identity);
        self.services.clear();
        self.selected_service = None;
        self.single = None;
        self.multi = None;
        self.failures.clear();
        self.notice = None;
        self.scroll_request = None;

        if target.node_client().is_none() {
            self.availability = Availability::NoClient;
            return;
        }
        self.reload_services(target, runtime, ctx);
    }

    fn reload_services(
        &mut self,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        self.cancel_streams();
        self.generation = self.generation.wrapping_add(1);
        self.services.clear();
        self.selected_service = None;
        self.single = None;
        self.multi = None;
        self.failures.clear();
        self.notice = None;
        self.scroll_request = None;

        let Some(identity) = self.active_target.clone() else {
            self.availability = Availability::NoNode;
            return;
        };
        let Some(client) = target.node_client() else {
            self.availability = Availability::NoClient;
            return;
        };

        self.availability = Availability::LoadingServices;
        let generation = self.generation;
        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        runtime.spawn(async move {
            let result = client.services().await.map_or_else(
                |error| Err(error.to_string()),
                |nodes| {
                    let services = nodes
                        .into_iter()
                        .flat_map(|node| node.services.into_iter().map(|service| service.id))
                        .filter(|service| !service.trim().is_empty())
                        .map(ServiceId::from)
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    Ok(services)
                },
            );
            let _ = event_tx
                .send(LogsEvent::ServicesLoaded {
                    target: identity,
                    generation,
                    result,
                })
                .await;
            repaint.request_repaint();
        });
    }

    fn start_log_session(
        &mut self,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        self.cancel_streams();
        self.generation = self.generation.wrapping_add(1);
        self.failures.clear();
        self.notice = None;
        self.scroll_request = None;

        let Some(identity) = self.active_target.clone() else {
            self.availability = Availability::NoNode;
            return;
        };
        let Some(client) = target.node_client() else {
            self.availability = Availability::NoClient;
            return;
        };

        let requested: Vec<ServiceId> = match self.view {
            LogView::Single => self.selected_service.clone().into_iter().collect(),
            LogView::Multi => self.enabled_multi_services(),
        };

        match self.view {
            LogView::Single => {
                let Some(service) = self.selected_service.clone() else {
                    return;
                };
                self.single = Some(SingleServiceLogs::new(LogTarget::new(
                    identity.node_address.clone(),
                    service,
                )));
                self.multi = None;
            }
            LogView::Multi => {
                let mut logs = MultiServiceLogs::new(identity.node_address.clone());
                let mut filters = LogFilters::default();
                filters.services = Some(requested.iter().cloned().collect());
                logs.buffer_mut().set_filters(filters);
                self.multi = Some(logs);
                self.single = None;
            }
        }

        self.initial_loading = requested.iter().cloned().collect();
        let generation = self.generation;
        for service in requested {
            let (stop, stop_rx) = oneshot::channel();
            self.stream_stops.push(StreamStop { stop });
            spawn_log_worker(
                runtime,
                client.clone(),
                identity.clone(),
                generation,
                service,
                self.event_tx.clone(),
                ctx.clone(),
                stop_rx,
            );
        }
    }

    fn cancel_streams(&mut self) {
        for StreamStop { stop } in self.stream_stops.drain(..) {
            // Sending consumes (and therefore drops) the stop signal. The worker
            // selects it against Talos I/O, then drops its Talos receiver too.
            let _ = stop.send(());
        }
        self.initial_loading.clear();
        self.streaming.clear();
    }

    fn drain_events(&mut self, target: &ScreenTarget<'_>, runtime: &Handle, ctx: &egui::Context) {
        let mut lines = Vec::new();
        for _ in 0..MAX_EVENTS_PER_FRAME {
            let Ok(event) = self.events.try_recv() else {
                break;
            };
            match event {
                LogsEvent::StreamLine {
                    target: event_target,
                    generation,
                    event,
                } if self.accepts_event(&event_target, generation) => lines.push(event),
                LogsEvent::ServicesLoaded {
                    target: event_target,
                    generation,
                    result,
                } if self.accepts_event(&event_target, generation) => match result {
                    Ok(services) => {
                        self.services = services;
                        self.selected_service = self.services.first().cloned();
                        self.availability = Availability::Ready;
                        if !self.services.is_empty() {
                            self.start_log_session(target, runtime, ctx);
                        }
                    }
                    Err(error) => self.availability = Availability::ServiceListError(error),
                },
                LogsEvent::InitialLoaded {
                    target: event_target,
                    generation,
                    service,
                    content,
                } if self.accepts_event(&event_target, generation) => {
                    self.initial_loading.remove(&service);
                    self.append_initial(service, content);
                }
                LogsEvent::InitialFailed {
                    target: event_target,
                    generation,
                    service,
                    error,
                } if self.accepts_event(&event_target, generation) => {
                    self.initial_loading.remove(&service);
                    self.failures
                        .insert(service, format!("Initial tail: {error}"));
                }
                LogsEvent::StreamStarted {
                    target: event_target,
                    generation,
                    service,
                } if self.accepts_event(&event_target, generation) => {
                    self.streaming.insert(service);
                }
                LogsEvent::StreamFailed {
                    target: event_target,
                    generation,
                    service,
                    error,
                } if self.accepts_event(&event_target, generation) => {
                    self.streaming.remove(&service);
                    self.failures
                        .insert(service, format!("Live stream: {error}"));
                }
                LogsEvent::StreamEnded {
                    target: event_target,
                    generation,
                    service,
                } if self.accepts_event(&event_target, generation) => {
                    self.streaming.remove(&service);
                    self.failures
                        .entry(service)
                        .or_insert_with(|| "Live stream ended.".to_owned());
                }
                LogsEvent::ExportCompleted {
                    target: event_target,
                    generation,
                    path,
                    result,
                } if self.accepts_event(&event_target, generation) => {
                    self.notice = Some(match result {
                        Ok(()) => format!("Saved logs to {}", path.display()),
                        Err(error) => format!("Could not save {}: {error}", path.display()),
                    });
                }
                _ => {}
            }
        }

        if !lines.is_empty() {
            let appended = match self.view {
                LogView::Single => self
                    .single
                    .as_mut()
                    .map_or(0, |logs| logs.buffer_mut().append_batch(lines)),
                LogView::Multi => self
                    .multi
                    .as_mut()
                    .map_or(0, |logs| logs.append_batch(lines)),
            };
            if appended > 0
                && self.active_buffer().is_some_and(|buffer| {
                    let stream = buffer.stream_state();
                    stream.following && !stream.paused
                })
            {
                self.scroll_request = Some(ScrollRequest::End);
            }
        }
    }

    fn accepts_event(&self, event_target: &TargetIdentity, generation: u64) -> bool {
        self.active_target.as_ref() == Some(event_target) && self.generation == generation
    }

    fn append_initial(&mut self, service: ServiceId, content: String) {
        let appended = match self.view {
            LogView::Single => self.single.as_mut().map_or(0, |logs| {
                if logs.target.service == service {
                    logs.replace_content(&content);
                    logs.buffer().entries().len()
                } else {
                    0
                }
            }),
            LogView::Multi => self.multi.as_mut().map_or(0, |logs| {
                logs.append_batch(
                    content
                        .lines()
                        .map(|line| LogEvent::new(service.clone(), line)),
                )
            }),
        };
        if appended > 0
            && self.active_buffer().is_some_and(|buffer| {
                let stream = buffer.stream_state();
                stream.following && !stream.paused
            })
        {
            self.scroll_request = Some(ScrollRequest::End);
        }
    }

    fn draw_unavailable(&mut self, ui: &mut egui::Ui) {
        match &self.availability {
            Availability::NoNode => {
                ui.colored_label(MUTED, "Select a node to view its Talos service logs.");
            }
            Availability::NoClient => {
                ui.colored_label(
                    ERROR,
                    "The selected node has no Talos client identity. Reload the context before viewing logs.",
                );
            }
            Availability::LoadingServices => {
                ui.spinner();
                ui.label("Loading Talos services for the selected node…");
            }
            Availability::ServiceListError(error) => {
                ui.colored_label(ERROR, format!("Could not load Talos services: {error}"));
            }
            Availability::Ready => {}
        }
    }

    fn draw_stream_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        let stream_state = self.active_buffer().map(LogBuffer::stream_state);
        let Some(stream_state) = stream_state else {
            return;
        };
        let has_retained_text = self
            .active_buffer()
            .is_some_and(|buffer| !buffer.entries().is_empty());
        let mut copy = false;
        let mut export = false;

        ui.horizontal_wrapped(|ui| {
            if ui
                .button(if stream_state.paused {
                    "Resume"
                } else {
                    "Pause"
                })
                .clicked()
            {
                let paused = !stream_state.paused;
                if let Some(buffer) = self.active_buffer_mut() {
                    buffer.set_paused(paused);
                }
                if !paused && stream_state.following {
                    self.scroll_request = Some(ScrollRequest::End);
                }
            }

            let mut following = stream_state.following;
            if ui.checkbox(&mut following, "Follow").changed() {
                if let Some(buffer) = self.active_buffer_mut() {
                    buffer.set_following(following);
                }
                if following {
                    self.scroll_request = Some(ScrollRequest::End);
                }
            }

            if ui.button("Clear").clicked() {
                self.clear_active_buffer();
            }
            copy = ui
                .add_enabled(has_retained_text, egui::Button::new("Copy visible"))
                .clicked();
            export = ui
                .add_enabled(has_retained_text, egui::Button::new("Save to file…"))
                .clicked();
        });

        let visible_text = (copy || export).then(|| self.visible_text());
        if copy {
            if let Some(text) = visible_text.as_ref().filter(|text| !text.is_empty()) {
                ctx.copy_text(text.clone());
                self.notice = Some("Copied visible log lines to the clipboard.".to_owned());
            } else {
                self.notice = Some("No visible log lines are available to copy.".to_owned());
            }
        }
        if export {
            if let Some(text) = visible_text {
                if text.is_empty() {
                    self.notice = Some("No visible log lines are available to save.".to_owned());
                } else {
                    self.start_export(text, target, runtime, ctx);
                }
            }
        }
    }

    fn draw_filters(
        &mut self,
        ui: &mut egui::Ui,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        let Some(buffer) = self.active_buffer() else {
            return;
        };
        let mut query = buffer.query().to_owned();
        let match_count = buffer.match_indices().len();
        let levels = buffer.filters().levels.clone();

        ui.horizontal_wrapped(|ui| {
            ui.label("Search:");
            if ui
                .add(
                    egui::TextEdit::singleline(&mut query)
                        .hint_text("case-insensitive text")
                        .desired_width(220.0),
                )
                .changed()
            {
                self.set_query(query.clone());
            }
            if ui
                .add_enabled(match_count > 0, egui::Button::new("Previous match"))
                .clicked()
            {
                self.select_match(false);
            }
            if ui
                .add_enabled(match_count > 0, egui::Button::new("Next match"))
                .clicked()
            {
                self.select_match(true);
            }
            if ui
                .add_enabled(!query.is_empty(), egui::Button::new("Clear search"))
                .clicked()
            {
                self.set_query(String::new());
            }
            if !query.is_empty() {
                ui.label(format!("{match_count} match(es)"));
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("Levels:");
            for (label, level, enabled) in [
                ("Error", LogLevel::Error, levels.error),
                ("Warn", LogLevel::Warning, levels.warning),
                ("Info", LogLevel::Info, levels.info),
                ("Debug", LogLevel::Debug, levels.debug),
                ("Other", LogLevel::Unknown, levels.unknown),
            ] {
                let mut enabled = enabled;
                if ui.checkbox(&mut enabled, label).changed() {
                    self.set_level_filter(&level, enabled);
                }
            }
        });

        if self.view == LogView::Multi {
            let selected = self
                .active_buffer()
                .and_then(|buffer| buffer.filters().services.clone())
                .unwrap_or_default();
            let services = self.services.clone();
            let mut changed = false;
            ui.collapsing("Service filters", |ui| {
                ui.horizontal_wrapped(|ui| {
                    for service in services {
                        let mut enabled = selected.contains(&service);
                        if ui.checkbox(&mut enabled, service.as_str()).changed() {
                            self.set_service_filter(&service, enabled);
                            changed = true;
                        }
                    }
                });
            });
            if changed {
                // The active service set defines the Stern-style workers, so any
                // change first stops the old receivers before starting new ones.
                self.start_log_session(target, runtime, ctx);
            }
        }
    }

    fn draw_stream_status(&self, ui: &mut egui::Ui) {
        if !self.initial_loading.is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!(
                    "Fetching initial tail for {} service(s)…",
                    self.initial_loading.len()
                ));
            });
        }
        if !self.streaming.is_empty() {
            ui.colored_label(
                INFO,
                format!("Live streaming {} service(s).", self.streaming.len()),
            );
        } else if self.initial_loading.is_empty() && self.active_buffer().is_some() {
            ui.colored_label(MUTED, "No live service stream is currently active.");
        }
        if self
            .active_buffer()
            .is_some_and(|buffer| buffer.stream_state().paused)
        {
            ui.colored_label(
                WARNING,
                "Paused: new events are retained, but the viewport will not follow.",
            );
        }
        for (service, error) in &self.failures {
            ui.colored_label(ERROR, format!("{} — {error}", service.as_str()));
        }
        if let Some(notice) = &self.notice {
            ui.colored_label(INFO, notice);
        }
        ui.separator();
    }

    fn draw_log_text(&mut self, ui: &mut egui::Ui) {
        let scroll_request = self.scroll_request.take();
        let Some(buffer) = self.active_buffer() else {
            return;
        };
        let visible = buffer.visible_indices();
        if visible.is_empty() {
            if buffer.entries().is_empty() {
                ui.colored_label(MUTED, "No log lines have been received yet.");
            } else {
                ui.colored_label(MUTED, "No retained log lines match the active filters.");
            }
            return;
        }

        let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
        let scroll_offset = match scroll_request {
            Some(ScrollRequest::Entry(entry)) => visible
                .iter()
                .position(|&visible_entry| visible_entry == entry)
                .map(|row| row as f32 * (row_height + ui.spacing().item_spacing.y)),
            Some(ScrollRequest::End) => {
                Some(visible.len() as f32 * (row_height + ui.spacing().item_spacing.y))
            }
            None => None,
        };
        let mut area = egui::ScrollArea::both()
            .id_salt("native_logs_text")
            .auto_shrink([false, false]);
        if let Some(offset) = scroll_offset {
            area = area.vertical_scroll_offset(offset);
        }

        let is_multi = self.view == LogView::Multi;
        area.show_rows(ui, row_height, visible.len(), |ui, rows| {
            for row in rows {
                let entry_index = visible[row];
                let entry = &buffer.entries()[entry_index];
                let text = if is_multi {
                    format!("[{}] {}", entry.service.as_str(), entry.selectable_text())
                } else {
                    entry.selectable_text().to_owned()
                };
                let mut rich = RichText::new(text).monospace();
                if buffer.is_current_match(entry_index) {
                    rich = rich.background_color(Color32::from_rgb(73, 88, 48));
                }
                ui.add(egui::Label::new(rich).selectable(true).extend());
            }
        });
    }

    fn active_buffer(&self) -> Option<&LogBuffer> {
        match self.view {
            LogView::Single => self.single.as_ref().map(SingleServiceLogs::buffer),
            LogView::Multi => self.multi.as_ref().map(MultiServiceLogs::buffer),
        }
    }

    fn active_buffer_mut(&mut self) -> Option<&mut LogBuffer> {
        match self.view {
            LogView::Single => self.single.as_mut().map(SingleServiceLogs::buffer_mut),
            LogView::Multi => self.multi.as_mut().map(MultiServiceLogs::buffer_mut),
        }
    }

    fn enabled_multi_services(&self) -> Vec<ServiceId> {
        self.active_buffer()
            .and_then(|buffer| buffer.filters().services.as_ref())
            .map(|services| services.iter().cloned().collect())
            .unwrap_or_else(|| self.services.clone())
    }

    fn clear_active_buffer(&mut self) {
        if let Some(buffer) = self.active_buffer_mut() {
            buffer.replace(Vec::<LogEvent>::new());
        }
        self.scroll_request = None;
    }

    fn set_query(&mut self, query: String) {
        let current_match = if let Some(buffer) = self.active_buffer_mut() {
            buffer.set_query(query);
            buffer.current_match()
        } else {
            None
        };
        self.scroll_request = current_match.map(ScrollRequest::Entry);
    }

    fn select_match(&mut self, forward: bool) {
        let selected = if let Some(buffer) = self.active_buffer_mut() {
            if forward {
                buffer.next_match()
            } else {
                buffer.previous_match()
            }
        } else {
            None
        };
        self.scroll_request = selected.map(ScrollRequest::Entry);
    }

    fn set_level_filter(&mut self, level: &LogLevel, enabled: bool) {
        if let Some(buffer) = self.active_buffer_mut() {
            let mut filters = buffer.filters().clone();
            filters.levels.set(level, enabled);
            buffer.set_filters(filters);
        }
    }

    fn set_service_filter(&mut self, service: &ServiceId, enabled: bool) {
        let Some(buffer) = self.active_buffer_mut() else {
            return;
        };
        let mut filters = buffer.filters().clone();
        let services = filters.services.get_or_insert_with(BTreeSet::new);
        if enabled {
            services.insert(service.clone());
        } else {
            services.remove(service);
        }
        buffer.set_filters(filters);
    }

    fn visible_text(&self) -> String {
        let Some(buffer) = self.active_buffer() else {
            return String::new();
        };
        let is_multi = self.view == LogView::Multi;
        buffer
            .visible_indices()
            .into_iter()
            .map(|index| {
                let entry = &buffer.entries()[index];
                if is_multi {
                    format!("[{}] {}", entry.service.as_str(), entry.selectable_text())
                } else {
                    entry.selectable_text().to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn start_export(
        &mut self,
        text: String,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save Talos logs")
            .set_file_name(self.export_file_name())
            .save_file()
        else {
            return;
        };
        let Some(identity) = self.active_target.clone() else {
            return;
        };
        if TargetIdentity::from_target(target).as_ref() != Some(&identity) {
            return;
        }

        let generation = self.generation;
        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        let write_runtime = runtime.clone();
        let event_path = path.clone();
        self.notice = Some(format!("Saving logs to {}…", path.display()));
        runtime.spawn(async move {
            let result = write_runtime
                .spawn_blocking(move || {
                    std::fs::write(&path, text).map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("log export task failed: {error}"))
                .and_then(|result| result);
            let _ = event_tx
                .send(LogsEvent::ExportCompleted {
                    target: identity,
                    generation,
                    path: event_path,
                    result,
                })
                .await;
            repaint.request_repaint();
        });
    }

    fn export_file_name(&self) -> String {
        let service = match self.view {
            LogView::Single => self
                .selected_service
                .as_ref()
                .map_or("service", ServiceId::as_str),
            LogView::Multi => "services",
        };
        format!("talos-{service}-logs.txt")
    }
}

impl Drop for LogsScreen {
    fn drop(&mut self) {
        self.deactivate();
    }
}

fn spawn_log_worker(
    runtime: &Handle,
    client: talos_rs::TalosClient,
    target: TargetIdentity,
    generation: u64,
    service: ServiceId,
    event_tx: mpsc::Sender<LogsEvent>,
    repaint: egui::Context,
    mut stop: oneshot::Receiver<()>,
) {
    runtime.spawn(async move {
        let initial = tokio::select! {
            _ = &mut stop => return,
            result = client.logs(service.as_str(), INITIAL_TAIL_LINES) => result,
        };
        let initial_event = match initial {
            Ok(content) => LogsEvent::InitialLoaded {
                target: target.clone(),
                generation,
                service: service.clone(),
                content,
            },
            Err(error) => LogsEvent::InitialFailed {
                target: target.clone(),
                generation,
                service: service.clone(),
                error: error.to_string(),
            },
        };
        if !send_worker_event(&event_tx, initial_event, &repaint, &mut stop).await {
            return;
        }

        // The historical tail has already been fetched above. A zero tail asks
        // the follow stream for only newly produced lines rather than duplicating it.
        let mut talos_rx = tokio::select! {
            _ = &mut stop => return,
            result = client.logs_stream(service.as_str(), STREAM_TAIL_LINES) => match result {
                Ok(receiver) => receiver,
                Err(error) => {
                    let event = LogsEvent::StreamFailed {
                        target: target.clone(),
                        generation,
                        service: service.clone(),
                        error: error.to_string(),
                    };
                    let _ = send_worker_event(&event_tx, event, &repaint, &mut stop).await;
                    return;
                }
            },
        };

        let started = LogsEvent::StreamStarted {
            target: target.clone(),
            generation,
            service: service.clone(),
        };
        if !send_worker_event(&event_tx, started, &repaint, &mut stop).await {
            return;
        }

        loop {
            let line = tokio::select! {
                _ = &mut stop => return,
                line = talos_rx.recv() => line,
            };
            let Some(line) = line else {
                let ended = LogsEvent::StreamEnded {
                    target: target.clone(),
                    generation,
                    service: service.clone(),
                };
                let _ = send_worker_event(&event_tx, ended, &repaint, &mut stop).await;
                return;
            };
            let event = LogsEvent::StreamLine {
                target: target.clone(),
                generation,
                event: LogEvent::new(service.clone(), line),
            };
            if !try_send_stream_line(&event_tx, event, &repaint) {
                return;
            }
        }
    });
}

async fn send_worker_event(
    event_tx: &mpsc::Sender<LogsEvent>,
    event: LogsEvent,
    repaint: &egui::Context,
    stop: &mut oneshot::Receiver<()>,
) -> bool {
    tokio::select! {
        _ = stop => false,
        sent = event_tx.send(event) => {
            if sent.is_ok() {
                repaint.request_repaint();
                true
            } else {
                false
            }
        }
    }
}

/// Keep draining Talos's unbounded receiver even if the screen has fallen
/// behind. The screen-owned channel is bounded, so dropping an overloaded live
/// event is preferable to letting an intermediate transport queue grow without
/// limit; the core buffer still bounds every event that reaches the UI.
fn try_send_stream_line(
    event_tx: &mpsc::Sender<LogsEvent>,
    event: LogsEvent,
    repaint: &egui::Context,
) -> bool {
    match event_tx.try_send(event) {
        Ok(()) => {
            repaint.request_repaint();
            true
        }
        Err(mpsc::error::TrySendError::Full(_)) => true,
        Err(mpsc::error::TrySendError::Closed(_)) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_events_for_a_previous_context_or_node_generation() {
        let mut screen = LogsScreen::default();
        screen.active_target = Some(TargetIdentity {
            context: "production".to_owned(),
            node_address: "10.0.0.10".to_owned(),
        });
        screen.generation = 7;

        assert!(screen.accepts_event(
            &TargetIdentity {
                context: "production".to_owned(),
                node_address: "10.0.0.10".to_owned(),
            },
            7,
        ));
        assert!(!screen.accepts_event(
            &TargetIdentity {
                context: "staging".to_owned(),
                node_address: "10.0.0.10".to_owned(),
            },
            7,
        ));
        assert!(!screen.accepts_event(
            &TargetIdentity {
                context: "production".to_owned(),
                node_address: "10.0.0.11".to_owned(),
            },
            7,
        ));
        assert!(!screen.accepts_event(
            &TargetIdentity {
                context: "production".to_owned(),
                node_address: "10.0.0.10".to_owned(),
            },
            8,
        ));
    }
}
