use std::{
    ops::Range,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use dioxus::prelude::*;
use talos_pilot_core::{
    formatting::format_bytes,
    inspection::{
        ProcessInspectionRequest, ProcessInspectionSnapshot, ProcessSampleState,
        ProcessSnapshotEntry, ProcessSort, ProcessTree, ProcessView, collect_process_inspection,
    },
};
use talos_rs::{
    ProcessState,
    talosctl::{DiskInfo, VolumeStatus, get_disks_for_node, get_volume_status_for_node},
};

use crate::{
    feature::{FeatureContext, LogRequest, copy_text},
    maintenance::text_review_region,
};

const POLL_INTERVAL: Duration = Duration::from_secs(10);
const COLLECTION_TIMEOUT: Duration = Duration::from_secs(30);
const PAGE_SIZE: usize = 80;
const LIST_STYLE: &str = "overflow:auto;max-height:52vh;min-width:0;";
const DETAILS_STYLE: &str = "overflow:auto;max-height:52vh;min-width:0;overflow-wrap:anywhere;";

/// Clamping here, rather than indexing with UI state, also handles lists which
/// shrink between refreshes. Only one page is ever put into the DOM.
fn page_range(len: usize, page: usize) -> Range<usize> {
    let last = len.saturating_sub(1) / PAGE_SIZE;
    let start = page.min(last) * PAGE_SIZE;
    start..start.saturating_add(PAGE_SIZE).min(len)
}

fn short_text(text: &str) -> String {
    let mut chars = text.chars();
    let mut short: String = chars.by_ref().take(160).collect();
    if chars.next().is_some() {
        short.push('…');
    }
    short
}

fn selected_process(
    entries: &[ProcessSnapshotEntry],
    selected: Option<i32>,
) -> Option<&ProcessSnapshotEntry> {
    entries
        .iter()
        .find(|entry| Some(entry.process.pid) == selected)
}

#[derive(Default)]
struct ProcessesState {
    snapshot: Option<Arc<ProcessInspectionSnapshot>>,
    sample: ProcessSampleState,
    selected: Option<i32>,
    loading: bool,
    error: Option<String>,
}

impl ProcessesState {
    fn finish(&mut self, result: Result<ProcessInspectionSnapshot, String>) {
        self.loading = false;
        match result {
            Ok(snapshot) => {
                if selected_process(&snapshot.processes, self.selected).is_none() {
                    self.selected = None;
                }
                // Retain the collector's delta state through both automatic and
                // manual refreshes. A key-remount, not a cluster revision, resets it.
                self.sample = snapshot.next_sample.clone();
                self.snapshot = Some(Arc::new(snapshot));
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
}

fn state_filter(value: &str) -> Option<ProcessState> {
    match value {
        "zombie" => Some(ProcessState::Zombie),
        "disk" => Some(ProcessState::DiskSleep),
        "running" => Some(ProcessState::Running),
        "sleeping" => Some(ProcessState::Sleeping),
        "stopped" => Some(ProcessState::Stopped),
        "tracing" => Some(ProcessState::TracingStop),
        "dead" => Some(ProcessState::Dead),
        _ => None,
    }
}

#[component]
pub(crate) fn ProcessesPanel(ctx: FeatureContext, on_logs: EventHandler<LogRequest>) -> Element {
    let _ = on_logs;
    let mut state = use_signal(ProcessesState::default);
    let mut refresh = use_signal(|| 0_u64);
    let mut data_revision = use_signal(|| 0_u64);
    let mut view = use_signal(ProcessView::default);
    let mut page = use_signal(|| 0_usize);
    let mut clipboard_status = use_signal(String::new);
    let poll_ctx = ctx.clone();
    use_future(move || {
        let poll_ctx = poll_ctx.clone();
        async move {
            loop {
                if poll_ctx
                    .run(async { tokio::time::sleep(POLL_INTERVAL).await })
                    .await
                    .is_err()
                {
                    break;
                }
                if !state.peek().loading {
                    refresh.with_mut(|generation| *generation = generation.wrapping_add(1));
                }
            }
        }
    });
    let request_ctx = ctx.clone();
    use_resource(move || {
        let _generation = *refresh.read();
        let request_ctx = request_ctx.clone();
        let request = ProcessInspectionRequest::new(
            request_ctx.inspection_target(),
            state.peek().sample.clone(),
        );
        state.write().loading = true;
        async move {
            let client = request_ctx.client.clone();
            let result = request_ctx
                .run(async move {
                    tokio::time::timeout(
                        COLLECTION_TIMEOUT,
                        collect_process_inspection(client, request),
                    )
                    .await
                    .map_err(|_| {
                        "Process collection timed out; previous data is retained and stale."
                            .to_string()
                    })?
                    .map_err(|error| error.to_string())
                })
                .await
                .and_then(|result| result);
            let succeeded = result.is_ok();
            state.write().finish(result);
            if succeeded {
                data_revision.with_mut(|revision| *revision = revision.wrapping_add(1));
            }
        }
    });
    let display_ctx = ctx.clone();
    let display_resource = use_resource(move || {
        let _revision = *data_revision.read();
        let snapshot = state.peek().snapshot.clone();
        let view = view.read().clone();
        let display_ctx = display_ctx.clone();
        async move {
            match snapshot {
                Some(snapshot) => display_ctx
                    .run(async move {
                        tokio::task::spawn_blocking(move || {
                            let rows = snapshot.display_rows(&view);
                            (snapshot, rows, format!("{view:?}"))
                        })
                        .await
                        .map(Some)
                        .map_err(|error| format!("Process display worker failed: {error}"))
                    })
                    .await
                    .and_then(|result| result),
                None => Ok(None),
            }
        }
    });

    let current = state.read();
    let loading = current.loading;
    let error = current.error.clone();
    let active_view = view.read().clone();
    let query = active_view.filter.text.clone().unwrap_or_default();
    let sort_value = match active_view.sort {
        ProcessSort::CpuPercent => "cpu",
        ProcessSort::CpuTime => "time",
        ProcessSort::ResidentMemory => "memory",
    };
    let state_value = match active_view.filter.state.as_ref() {
        Some(ProcessState::Zombie) => "zombie",
        Some(ProcessState::DiskSleep) => "disk",
        Some(ProcessState::Running) => "running",
        Some(ProcessState::Sleeping) => "sleeping",
        Some(ProcessState::Stopped) => "stopped",
        Some(ProcessState::TracingStop) => "tracing",
        Some(ProcessState::Dead) => "dead",
        _ => "all",
    };
    let tree_value = match active_view.tree {
        ProcessTree::Flat => "flat",
        ProcessTree::Full => "tree",
        ProcessTree::Subtree { .. } => "subtree",
    };
    let selected_pid = current.selected;
    let display_result = display_resource.read();
    let display_error = display_result
        .as_ref()
        .and_then(|result| result.as_ref().err())
        .cloned();
    let displayed = display_result
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .and_then(Option::as_ref);
    let display_pending = current.snapshot.is_some()
        && displayed.is_none_or(|(snapshot, _, signature)| {
            signature != &format!("{active_view:?}")
                || current
                    .snapshot
                    .as_ref()
                    .is_some_and(|latest| latest.sampled_at != snapshot.sampled_at)
        });
    let (rows, matching, total, age, summary, unavailable, details, command) = if let Some((
        snapshot,
        display,
        _,
    )) = displayed
    {
        let range = page_range(display.len(), *page.read());
        let rows = display[range]
            .iter()
            .map(|row| {
                let entry = &snapshot.processes[row.process_index];
                let process = &entry.process;
                (
                    process.pid,
                    format!(
                        "{}{}",
                        "  ".repeat(row.depth.min(16)),
                        if row.depth > 0 { "↳ " } else { "" }
                    ),
                    process.state.short().to_owned(),
                    entry
                        .cpu_percent_display()
                        .unwrap_or_else(|| "sampling…".into()),
                    process.cpu_time_human(),
                    entry.resident_memory_display(),
                    short_text(process.display_command()),
                )
            })
            .collect::<Vec<_>>();
        let details: Vec<(String, String)> = selected_process(&snapshot.processes, selected_pid)
            .map(|entry| {
                let process = &entry.process;
                vec![
                    (
                        "PID / parent PID".into(),
                        format!("{} / {}", process.pid, process.ppid),
                    ),
                    ("State".into(), process.state.description().into()),
                    ("Threads".into(), process.threads.to_string()),
                    (
                        "CPU delta".into(),
                        entry.cpu_percent_display().unwrap_or_else(|| {
                            "Unavailable until a compatible second sample".into()
                        }),
                    ),
                    (
                        "Cumulative CPU time".into(),
                        format!(
                            "{} ({:.3} seconds)",
                            process.cpu_time_human(),
                            process.cpu_time
                        ),
                    ),
                    (
                        "Resident memory".into(),
                        format_bytes(process.resident_memory),
                    ),
                    (
                        "Virtual memory".into(),
                        format_bytes(process.virtual_memory),
                    ),
                    ("Command".into(), process.command.clone()),
                    ("Executable".into(), process.executable.clone()),
                    ("Arguments".into(), process.args.clone()),
                ]
            })
            .unwrap_or_default();
        let command = selected_process(&snapshot.processes, selected_pid)
            .map(|entry| entry.process.display_command().to_owned());
        let memory = snapshot
            .system
            .memory
            .as_ref()
            .map(|memory| memory.display())
            .unwrap_or_else(|| "unavailable".into());
        let cpu = snapshot
            .system
            .cpu
            .as_ref()
            .and_then(|cpu| cpu.usage_display())
            .unwrap_or_else(|| "unavailable / sampling".into());
        let load = snapshot
            .system
            .load_average
            .as_ref()
            .map(|load| {
                format!(
                    "{:.2} / {:.2} / {:.2}",
                    load.one_minute, load.five_minutes, load.fifteen_minutes
                )
            })
            .unwrap_or_else(|| "unavailable".into());
        let summary = format!(
            "CPU {cpu} · Memory {memory} · Load {load} · {} running / {} sleeping / {} disk-wait / {} zombies",
            snapshot.state_counts.running,
            snapshot.state_counts.sleeping,
            snapshot.state_counts.disk_sleep,
            snapshot.state_counts.zombie
        );
        let unavailable = snapshot
            .unavailable
            .iter()
            .map(|source| format!("{}: {}", source.source, source.message))
            .collect::<Vec<_>>();
        (
            rows,
            display.len(),
            snapshot.processes.len(),
            Some(snapshot.sampled_at.elapsed().as_secs()),
            summary,
            unavailable,
            details,
            command,
        )
    } else {
        (
            Vec::new(),
            0,
            0,
            None,
            "Waiting for authoritative process data.".into(),
            Vec::new(),
            Vec::new(),
            None,
        )
    };
    drop(current);
    drop(display_result);
    let actual_page = page_range(matching, *page.read()).start / PAGE_SIZE;
    let pages = matching.div_ceil(PAGE_SIZE).max(1);
    let copy_ctx = ctx.clone();
    rsx! {
        section { class: "panel processes-panel",
            div { class: "card-heading", h2 { "Processes" } span { class: "muted", "{ctx.context} · {ctx.node} · {ctx.address}" } }
            div { class: "log-toolbar",
                button { disabled: loading, onclick: move |_| refresh.with_mut(|generation| *generation = generation.wrapping_add(1)), "Refresh" }
                span { role: "status", if loading { "Loading… · previous snapshot retained" } else { "Auto-refresh every 10 seconds" } }
                if let Some(age) = age { span { class: "muted", "Snapshot {age}s old" } }
            }
            if display_pending { p { class: "muted", role: "status", "Preparing filtered/sorted process display… · last rendered view retained" } }
            if let Some(error) = display_error { p { class: "error", role: "alert", "{error}" } }
            if let Some(error) = error { p { class: "error", role: "alert", "{error} · last successful data, if any, is stale" } }
            p { class: "muted", "{summary}" }
            for reason in unavailable { p { class: "muted", "Source unavailable: {reason}" } }
            div { class: "log-toolbar",
                input { aria_label: "Process text filter", value: "{query}", placeholder: "Filter command, executable or arguments",
                    oninput: move |event| { view.write().filter.text = Some(event.value().chars().take(4096).collect()); page.set(0); } }
                select { aria_label: "Process sort", value: sort_value,
                    onchange: move |event| { view.write().sort = match event.value().as_str() { "time" => ProcessSort::CpuTime, "memory" => ProcessSort::ResidentMemory, _ => ProcessSort::CpuPercent }; page.set(0); },
                    option { value: "cpu", "CPU %" } option { value: "time", "CPU time" } option { value: "memory", "Resident memory" }
                }
                select { aria_label: "Process state filter", value: state_value,
                    onchange: move |event| { view.write().filter.state = state_filter(&event.value()); page.set(0); },
                    option { value: "all", "All states" } option { value: "zombie", "Zombie" } option { value: "disk", "Disk wait" }
                    option { value: "running", "Running" } option { value: "sleeping", "Sleeping" } option { value: "stopped", "Stopped" }
                    option { value: "tracing", "Tracing stop" } option { value: "dead", "Dead" }
                }
                select { aria_label: "Process hierarchy", value: tree_value,
                    onchange: move |event| {
                        view.write().tree = match event.value().as_str() {
                            "tree" => ProcessTree::Full,
                            "subtree" => selected_pid.map(|root_pid| ProcessTree::Subtree { root_pid }).unwrap_or(ProcessTree::Flat),
                            _ => ProcessTree::Flat,
                        };
                        page.set(0);
                    },
                    option { value: "flat", "Flat" } option { value: "tree", "Full tree" }
                    option { value: "subtree", disabled: selected_pid.is_none(), "Selected subtree" }
                }
                if let ProcessTree::Subtree { root_pid } = active_view.tree { span { class: "muted", "Subtree rooted at PID {root_pid}" } }
            }
            div { style: "display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,380px),1fr));gap:16px;min-width:0;",
                div {
                    Pagination { page, count: matching }
                    p { class: "muted", "{matching} matching / {total} total · page {actual_page + 1} / {pages}" }
                    div { style: LIST_STYLE,
                        table { class: "data-table",
                            thead { tr { th { "PID" } th { "State" } th { "CPU %" } th { "CPU time" } th { "Memory" } th { "Command" } } }
                            tbody {
                                for (pid, prefix, process_state, cpu, time, memory, command) in rows {
                                    tr { key: "{pid}", class: if Some(pid) == selected_pid { "selected-row" } else { "" },
                                        td { button { aria_label: "Select process {pid}", onclick: move |_| {
                                            let mut current = state.write();
                                            if current.snapshot.as_ref().is_some_and(|snapshot| selected_process(&snapshot.processes, Some(pid)).is_some()) {
                                                current.selected = Some(pid);
                                            }
                                        }, "{pid}" } }
                                        td { "{process_state}" } td { "{cpu}" } td { "{time}" } td { "{memory}" }
                                        td { style: "white-space:pre-wrap;overflow-wrap:anywhere;", "{prefix}{command}" }
                                    }
                                }
                            }
                        }
                        if matching == 0 { p { class: "empty", "No matching processes (or process source has not loaded)." } }
                    }
                }
                div { style: "min-width:0;",
                    h3 { "Selected process" }
                    button { disabled: command.is_none(), onclick: move |_| {
                        if let Some(command) = command.clone() {
                            let copy_ctx = copy_ctx.clone();
                            spawn(async move {
                                clipboard_status.set(match copy_text(copy_ctx, command).await { Ok(()) => "Command copied".into(), Err(error) => format!("Copy failed: {error}") });
                            });
                        }
                    }, "Copy full command" }
                    p { role: "status", "{clipboard_status}" }
                    if details.is_empty() { p { class: "muted", "Select a PID to inspect its full command, memory and CPU details." } }
                    if !details.is_empty() {
                        {text_review_region(
                            "Selected process details",
                            DETAILS_STYLE,
                            rsx! { for (label, value) in details {
                                div { strong { "{label}" } pre { style: "white-space:pre-wrap;overflow-wrap:anywhere;", "{value}" } }
                            } },
                        )}
                    }
                }
            }
        }
    }
}

#[component]
fn Pagination(mut page: Signal<usize>, count: usize) -> Element {
    let actual = page_range(count, *page.read()).start / PAGE_SIZE;
    let last = count.saturating_sub(1) / PAGE_SIZE;
    rsx! {
        div { class: "log-toolbar",
            button { disabled: actual == 0, onclick: move |_| page.set(actual.saturating_sub(1)), "Previous page" }
            button { disabled: actual >= last, onclick: move |_| page.set(actual + 1), "Next page" }
        }
    }
}

/// Freeze the exact applied configuration along with node/context identity.
/// In particular, no missing-path fallback to TALOSCONFIG or current-context
/// may be performed by a refresh subprocess.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StorageRequest {
    key: String,
    context: String,
    address: String,
    config_path: String,
}

impl StorageRequest {
    fn new(
        key: String,
        context: String,
        address: String,
        config_path: Option<&Path>,
    ) -> Result<Self, String> {
        if context.trim().is_empty()
            || context.starts_with('-')
            || address.trim().is_empty()
            || address.starts_with('-')
        {
            return Err("Storage source unavailable: an explicit valid Talos context and node address are required.".into());
        }
        let config_path = config_path.and_then(Path::to_str).filter(|path| !path.is_empty() && !path.starts_with('-'))
            .ok_or_else(|| "Storage source unavailable: the exact applied Talosconfig path is missing or cannot be represented for talosctl. Reload an explicit Talosconfig; ambient configuration is not used.".to_string())?
            .to_owned();
        Ok(Self {
            key,
            context,
            address,
            config_path,
        })
    }
}

struct StorageSource<T> {
    data: Option<Vec<T>>,
    error: Option<String>,
    updated: Option<Instant>,
}

impl<T> Default for StorageSource<T> {
    fn default() -> Self {
        Self {
            data: None,
            error: None,
            updated: None,
        }
    }
}

impl<T> StorageSource<T> {
    fn finish(&mut self, result: Result<Vec<T>, String>) {
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
                self.updated = Some(Instant::now());
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn status(&self, source: &str) -> String {
        match (&self.data, &self.error) {
            (Some(_), Some(error)) => {
                format!("{source} source unavailable: {error}. Retained data is stale.")
            }
            (None, Some(error)) => {
                format!("{source} source unavailable: {error}. No authoritative data.")
            }
            (Some(data), None) => format!(
                "{} {source} · sampled {}s ago",
                data.len(),
                self.updated
                    .map(|time| time.elapsed().as_secs())
                    .unwrap_or(0)
            ),
            (None, None) => format!("{source}: not loaded"),
        }
    }
}

#[derive(Default)]
struct StorageState {
    target_key: Option<String>,
    disks: StorageSource<DiskInfo>,
    volumes: StorageSource<VolumeStatus>,
    selected_disk: Option<String>,
    selected_volume: Option<String>,
    loading: bool,
}

fn reconcile_storage_selection(
    selected: &mut Option<String>,
    mut ids: impl Iterator<Item = String>,
) {
    if selected
        .as_ref()
        .is_some_and(|selected| !ids.any(|id| &id == selected))
    {
        *selected = None;
    }
}

impl StorageState {
    fn begin(&mut self, key: &str) {
        if self.target_key.as_deref() != Some(key) {
            *self = Self::default();
            self.target_key = Some(key.to_owned());
        }
        self.loading = true;
    }

    fn finish(
        &mut self,
        key: &str,
        disks: Result<Vec<DiskInfo>, String>,
        volumes: Result<Vec<VolumeStatus>, String>,
    ) {
        if self.target_key.as_deref() != Some(key) {
            return;
        }
        if let Ok(disks) = &disks {
            reconcile_storage_selection(
                &mut self.selected_disk,
                disks.iter().map(|disk| disk.id.clone()),
            );
        }
        if let Ok(volumes) = &volumes {
            reconcile_storage_selection(
                &mut self.selected_volume,
                volumes.iter().map(|volume| volume.id.clone()),
            );
        }
        self.disks.finish(disks);
        self.volumes.finish(volumes);
        self.loading = false;
    }
}

async fn collect_storage(
    request: StorageRequest,
) -> (
    Result<Vec<DiskInfo>, String>,
    Result<Vec<VolumeStatus>, String>,
) {
    // Each COSI source is independent: failure of one must never turn the other
    // into an empty-success result. Helpers use talosctl, not inaccessible COSI.
    let disks = async {
        tokio::time::timeout(
            Duration::from_secs(12),
            get_disks_for_node(
                &request.context,
                &request.address,
                Some(&request.config_path),
            ),
        )
        .await
        .map_err(|_| "talosctl disk query timed out".to_string())?
        .map_err(|error| error.to_string())
    };
    let volumes = async {
        tokio::time::timeout(
            Duration::from_secs(12),
            get_volume_status_for_node(
                &request.context,
                &request.address,
                Some(&request.config_path),
            ),
        )
        .await
        .map_err(|_| "talosctl volume query timed out".to_string())?
        .map_err(|error| error.to_string())
    };
    tokio::join!(disks, volumes)
}

#[component]
pub(crate) fn StoragePanel(ctx: FeatureContext, on_logs: EventHandler<LogRequest>) -> Element {
    let _ = on_logs;
    let mut state = use_signal(StorageState::default);
    let mut refresh = use_signal(|| 0_u64);
    let mut mode = use_signal(|| "disks".to_string());
    let disk_page = use_signal(|| 0_usize);
    let volume_page = use_signal(|| 0_usize);
    let poll_ctx = ctx.clone();
    use_future(move || {
        let poll_ctx = poll_ctx.clone();
        async move {
            loop {
                if poll_ctx
                    .run(async { tokio::time::sleep(POLL_INTERVAL).await })
                    .await
                    .is_err()
                {
                    break;
                }
                if !state.peek().loading {
                    refresh.with_mut(|generation| *generation = generation.wrapping_add(1));
                }
            }
        }
    });
    let request_ctx = ctx.clone();
    use_resource(move || {
        let _generation = *refresh.read();
        let request_ctx = request_ctx.clone();
        let request = StorageRequest::new(
            request_ctx.key(),
            request_ctx.context.clone(),
            request_ctx.address.clone(),
            request_ctx.config_path.as_deref(),
        );
        let key = request
            .as_ref()
            .map(|request| request.key.clone())
            .unwrap_or_else(|_| request_ctx.key());
        state.write().begin(&key);
        async move {
            let result = match request {
                Ok(request) => request_ctx.run(collect_storage(request)).await,
                Err(error) => Err(error),
            };
            let (disks, volumes) = result.unwrap_or_else(|error| (Err(error.clone()), Err(error)));
            state.write().finish(&key, disks, volumes);
        }
    });
    let current = state.read();
    let loading = current.loading;
    let disks_status = current.disks.status("disks");
    let volumes_status = current.volumes.status("volumes");
    let disks = current.disks.data.as_deref().unwrap_or_default();
    let volumes = current.volumes.data.as_deref().unwrap_or_default();
    let disk_count = disks.len();
    let volume_count = volumes.len();
    let disk_rows = disks[page_range(disks.len(), *disk_page.read())]
        .iter()
        .map(|disk| {
            (
                disk.id.clone(),
                disk.dev_path.clone(),
                disk.size_pretty.clone(),
                disk.model.clone().unwrap_or_else(|| "unknown model".into()),
                disk.transport
                    .clone()
                    .unwrap_or_else(|| "unknown transport".into()),
                if disk.rotational {
                    "HDD"
                } else {
                    "SSD / non-rotational"
                },
                current.selected_disk.as_deref() == Some(&disk.id),
            )
        })
        .collect::<Vec<_>>();
    let volume_rows = volumes[page_range(volumes.len(), *volume_page.read())]
        .iter()
        .map(|volume| {
            (
                volume.id.clone(),
                volume.phase.clone(),
                volume.size.clone(),
                volume
                    .filesystem
                    .clone()
                    .unwrap_or_else(|| "unavailable".into()),
                volume
                    .encryption_provider
                    .clone()
                    .unwrap_or_else(|| "none reported".into()),
                current.selected_volume.as_deref() == Some(&volume.id),
            )
        })
        .collect::<Vec<_>>();
    let disk_details: Vec<(String, String)> = disks
        .iter()
        .find(|disk| current.selected_disk.as_deref() == Some(&disk.id))
        .map(|disk| {
            vec![
                ("Disk ID".into(), disk.id.clone()),
                ("Device path".into(), disk.dev_path.clone()),
                (
                    "Size".into(),
                    format!("{} · {} bytes", disk.size_pretty, disk.size),
                ),
                (
                    "Model".into(),
                    disk.model.clone().unwrap_or_else(|| "Not reported".into()),
                ),
                (
                    "Serial number".into(),
                    disk.serial.clone().unwrap_or_else(|| "Not reported".into()),
                ),
                (
                    "Transport".into(),
                    disk.transport
                        .clone()
                        .unwrap_or_else(|| "Not reported".into()),
                ),
                (
                    "Rotational / read-only / CD-ROM".into(),
                    format!("{} / {} / {}", disk.rotational, disk.readonly, disk.cdrom),
                ),
                (
                    "World Wide ID".into(),
                    disk.wwid.clone().unwrap_or_else(|| "Not reported".into()),
                ),
                (
                    "Bus path".into(),
                    disk.bus_path
                        .clone()
                        .unwrap_or_else(|| "Not reported".into()),
                ),
            ]
        })
        .unwrap_or_default();
    let volume_details: Vec<(String, String)> = volumes
        .iter()
        .find(|volume| current.selected_volume.as_deref() == Some(&volume.id))
        .map(|volume| {
            vec![
                ("Volume ID".into(), volume.id.clone()),
                ("Phase".into(), volume.phase.clone()),
                ("Size".into(), volume.size.clone()),
                (
                    "Filesystem".into(),
                    volume
                        .filesystem
                        .clone()
                        .unwrap_or_else(|| "Not reported".into()),
                ),
                (
                    "Mount location".into(),
                    volume
                        .mount_location
                        .clone()
                        .unwrap_or_else(|| "Not reported".into()),
                ),
                (
                    "Encryption provider".into(),
                    volume
                        .encryption_provider
                        .clone()
                        .unwrap_or_else(|| "None reported".into()),
                ),
            ]
        })
        .unwrap_or_default();
    drop(current);
    let disk_mode = mode.read().as_str() == "disks";
    rsx! {
        section { class: "panel storage-panel",
            div { class: "card-heading", h2 { "Storage" } span { class: "muted", "{ctx.context} · {ctx.node} · {ctx.address}" } }
            div { class: "log-toolbar",
                button { disabled: loading, onclick: move |_| refresh.with_mut(|generation| *generation = generation.wrapping_add(1)), "Refresh" }
                span { role: "status", if loading { "Loading… · previous data retained" } else { "Auto-refresh every 10 seconds · explicit Talosconfig/context/node" } }
                button { aria_pressed: "{disk_mode}", onclick: move |_| mode.set("disks".into()), "Disks ({disk_count})" }
                button { aria_pressed: "{!disk_mode}", onclick: move |_| mode.set("volumes".into()), "Volumes ({volume_count})" }
            }
            p { role: "status", "{disks_status}" }
            p { role: "status", "{volumes_status}" }
            p { class: "muted", "COSI resources read through talosctl. Unavailable sources are not empty-success results." }
            div { style: "display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,380px),1fr));gap:16px;min-width:0;",
                div {
                    if disk_mode {
                        Pagination { page: disk_page, count: disk_count }
                        div { style: LIST_STYLE,
                            table { class: "data-table",
                                thead { tr { th { "Disk" } th { "Device" } th { "Size" } th { "Model" } th { "Transport" } th { "Type" } } }
                                tbody { for (id, path, size, model, transport, kind, selected) in disk_rows {
                                    tr { key: "{id}", class: if selected { "selected-row" } else { "" },
                                        td { button { onclick: { let id = id.clone(); move |_| state.write().selected_disk = Some(id.clone()) }, "{id}" } }
                                        td { "{path}" } td { "{size}" } td { "{model}" } td { "{transport}" } td { "{kind}" }
                                    }
                                } }
                            }
                            if disk_count == 0 { p { class: "empty", "No disk rows available; check the source status above." } }
                        }
                    } else {
                        Pagination { page: volume_page, count: volume_count }
                        div { style: LIST_STYLE,
                            table { class: "data-table",
                                thead { tr { th { "Volume" } th { "Phase" } th { "Size" } th { "Filesystem" } th { "Encryption" } } }
                                tbody { for (id, phase, size, filesystem, encryption, selected) in volume_rows {
                                    tr { key: "{id}", class: if selected { "selected-row" } else { "" },
                                        td { button { onclick: { let id = id.clone(); move |_| state.write().selected_volume = Some(id.clone()) }, "{id}" } }
                                        td { "{phase}" } td { "{size}" } td { "{filesystem}" } td { "{encryption}" }
                                    }
                                } }
                            }
                            if volume_count == 0 { p { class: "empty", "No volume rows available; check the source status above." } }
                        }
                    }
                }
                div { style: "min-width:0;",
                    h3 { if disk_mode { "Selected disk" } else { "Selected volume" } }
                    if disk_mode && disk_details.is_empty() {
                        p { class: "muted", "Select a disk to read its full device, model, serial and bus details." }
                    } else if !disk_mode && volume_details.is_empty() {
                        p { class: "muted", "Select a volume to read its mount, filesystem, encryption and phase details." }
                    } else {
                        {text_review_region(
                            if disk_mode { "Selected disk details" } else { "Selected volume details" },
                            DETAILS_STYLE,
                            rsx! { for (label, value) in if disk_mode { disk_details } else { volume_details } {
                                div { strong { "{label}" } pre { style: "white-space:pre-wrap;overflow-wrap:anywhere;", "{value}" } }
                            } },
                        )}
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talos_pilot_core::inspection::{
        InspectionTarget, ProcessFilter, ProcessStateCounts, ProcessSystemSnapshot,
        build_process_display_rows,
    };
    use talos_rs::ProcessInfo;

    fn process(
        pid: i32,
        ppid: i32,
        state: ProcessState,
        cpu: f32,
        time: f64,
        memory: u64,
        command: &str,
    ) -> ProcessSnapshotEntry {
        ProcessSnapshotEntry {
            process: ProcessInfo {
                pid,
                ppid,
                state,
                threads: 1,
                cpu_time: time,
                virtual_memory: memory * 2,
                resident_memory: memory,
                command: command.into(),
                executable: format!("/bin/{command}"),
                args: format!("{command} --fixture"),
            },
            cpu_percent: Some(cpu),
        }
    }

    fn fixture() -> Vec<ProcessSnapshotEntry> {
        vec![
            process(1, 0, ProcessState::Sleeping, 1.0, 50.0, 300, "init"),
            process(2, 1, ProcessState::Zombie, 0.0, 1.0, 0, "zombie"),
            process(3, 1, ProcessState::DiskSleep, 8.0, 12.0, 900, "disk-worker"),
            process(4, 3, ProcessState::Running, 20.0, 2.0, 500, "worker"),
        ]
    }

    #[test]
    fn process_state_and_case_insensitive_text_filters_use_shared_transform() {
        let entries = fixture();
        let view = ProcessView {
            filter: ProcessFilter {
                text: Some("/BIN/DISK".into()),
                state: state_filter("disk"),
            },
            ..Default::default()
        };
        let rows = build_process_display_rows(&entries, &view);
        assert_eq!(rows.len(), 1);
        assert_eq!(entries[rows[0].process_index].process.pid, 3);
        let zombies = build_process_display_rows(
            &entries,
            &ProcessView {
                filter: ProcessFilter {
                    text: None,
                    state: state_filter("zombie"),
                },
                ..Default::default()
            },
        );
        assert_eq!(entries[zombies[0].process_index].process.pid, 2);
        assert_eq!(state_filter("all"), None);
    }

    #[test]
    fn all_three_process_sorts_are_reachable_and_distinct() {
        let entries = fixture();
        for (sort, first) in [
            (ProcessSort::CpuPercent, 4),
            (ProcessSort::CpuTime, 1),
            (ProcessSort::ResidentMemory, 3),
        ] {
            let rows = build_process_display_rows(
                &entries,
                &ProcessView {
                    sort,
                    ..Default::default()
                },
            );
            assert_eq!(entries[rows[0].process_index].process.pid, first);
        }
    }

    #[test]
    fn tree_and_selected_subtree_keep_hierarchy_and_exclude_other_roots() {
        let entries = fixture();
        let tree = build_process_display_rows(
            &entries,
            &ProcessView {
                tree: ProcessTree::Full,
                ..Default::default()
            },
        );
        assert_eq!(entries[tree[0].process_index].process.pid, 1);
        assert!(
            tree.iter()
                .any(|row| entries[row.process_index].process.pid == 4 && row.depth == 2)
        );
        let subtree = build_process_display_rows(
            &entries,
            &ProcessView {
                tree: ProcessTree::Subtree { root_pid: 3 },
                ..Default::default()
            },
        );
        assert_eq!(
            subtree
                .iter()
                .map(|row| entries[row.process_index].process.pid)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
    }

    #[test]
    fn selection_is_pid_based_not_sort_or_page_index() {
        let mut entries = fixture();
        assert_eq!(
            selected_process(&entries, Some(3)).unwrap().process.command,
            "disk-worker"
        );
        entries.reverse();
        assert_eq!(
            selected_process(&entries, Some(3)).unwrap().process.command,
            "disk-worker"
        );
        entries.retain(|entry| entry.process.pid != 3);
        assert!(selected_process(&entries, Some(3)).is_none());
    }

    #[test]
    fn successful_process_refresh_retains_delta_state_and_failure_retains_snapshot() {
        let target = InspectionTarget::new("fixture-node", "192.0.2.10");
        let sampled_at = Instant::now();
        let next_sample = ProcessSampleState {
            target: Some(target.clone()),
            sampled_at: Some(sampled_at),
            cpu_times: [(1, 50.0), (3, 12.0)].into_iter().collect(),
            ..Default::default()
        };
        let snapshot = ProcessInspectionSnapshot {
            target: target.clone(),
            sampled_at,
            processes: fixture(),
            state_counts: ProcessStateCounts::default(),
            system: ProcessSystemSnapshot::default(),
            next_sample,
            unavailable: Vec::new(),
        };
        let mut state = ProcessesState {
            selected: Some(3),
            loading: true,
            ..Default::default()
        };
        state.finish(Ok(snapshot));
        let retained = state.snapshot.clone().unwrap();
        assert_eq!(state.selected, Some(3));
        assert_eq!(state.sample.cpu_times.get(&3), Some(&12.0));
        let request = ProcessInspectionRequest::new(target, state.sample.clone());
        assert_eq!(request.sample.sampled_at, Some(sampled_at));
        state.loading = true;
        state.finish(Err("fixture disconnected".into()));
        assert!(!state.loading);
        assert!(Arc::ptr_eq(&retained, state.snapshot.as_ref().unwrap()));
        assert_eq!(state.sample.cpu_times.get(&3), Some(&12.0));
        assert_eq!(state.error.as_deref(), Some("fixture disconnected"));
        let mut refreshed = (*retained).clone();
        refreshed.processes.retain(|entry| entry.process.pid != 3);
        refreshed.next_sample.cpu_times.insert(1, 75.0);
        state.finish(Ok(refreshed));
        assert!(state.selected.is_none());
        assert_eq!(state.sample.cpu_times.get(&1), Some(&75.0));
        assert!(state.error.is_none());
    }

    #[test]
    fn pagination_bounds_rendering_after_shrink_and_empty_results() {
        assert_eq!(page_range(0, usize::MAX), 0..0);
        assert_eq!(page_range(161, 1), 80..160);
        assert_eq!(page_range(2, usize::MAX), 0..2);
        assert_eq!(page_range(161, usize::MAX), 160..161);
        assert_eq!(short_text(&"界".repeat(180)).chars().count(), 161);
    }

    #[test]
    fn storage_source_failure_retains_stale_data_instead_of_empty_success() {
        let mut source = StorageSource::<String>::default();
        source.finish(Err("permission denied".into()));
        assert!(source.data.is_none());
        assert!(source.status("disks").contains("No authoritative data"));
        source.finish(Ok(vec!["sda".into()]));
        let updated = source.updated;
        source.finish(Err("disconnected".into()));
        assert_eq!(source.data.as_ref().unwrap(), &vec!["sda".to_string()]);
        assert_eq!(source.updated, updated);
        assert!(source.status("disks").contains("stale"));
        source.finish(Ok(vec![]));
        assert!(source.error.is_none());
        assert!(source.status("disks").starts_with("0 disks"));
    }

    #[test]
    fn storage_selection_survives_reorder_but_not_missing_identity() {
        let mut selection = Some("EPHEMERAL".to_string());
        reconcile_storage_selection(
            &mut selection,
            ["STATE", "EPHEMERAL"].into_iter().map(str::to_string),
        );
        assert_eq!(selection.as_deref(), Some("EPHEMERAL"));
        reconcile_storage_selection(&mut selection, ["STATE"].into_iter().map(str::to_string));
        assert!(selection.is_none());
    }

    #[test]
    fn storage_requests_require_frozen_configuration_and_explicit_target() {
        let request = StorageRequest::new(
            "epoch7/context/node".into(),
            "selected-context".into(),
            "192.0.2.10".into(),
            Some(Path::new("/chosen/talosconfig")),
        )
        .unwrap();
        assert_eq!(request.context, "selected-context");
        assert_eq!(request.address, "192.0.2.10");
        assert_eq!(request.config_path, "/chosen/talosconfig");
        assert_eq!(request.key, "epoch7/context/node");
        assert!(StorageRequest::new("k".into(), "ctx".into(), "192.0.2.1".into(), None).is_err());
        assert!(
            StorageRequest::new(
                "k".into(),
                "".into(),
                "192.0.2.1".into(),
                Some(Path::new("/chosen"))
            )
            .is_err()
        );
        assert!(
            StorageRequest::new(
                "k".into(),
                "ctx".into(),
                "--nodes=other".into(),
                Some(Path::new("/chosen"))
            )
            .is_err()
        );
        assert!(
            StorageRequest::new(
                "k".into(),
                "ctx".into(),
                "".into(),
                Some(Path::new("/chosen"))
            )
            .is_err()
        );
    }

    #[test]
    fn late_storage_results_cannot_publish_into_new_target_state() {
        let mut state = StorageState::default();
        state.begin("epoch1/context/node-a");
        state.begin("epoch2/context/node-b");
        state.finish(
            "epoch1/context/node-a",
            Ok(Vec::new()),
            Err("old-node error".into()),
        );
        assert!(state.loading);
        assert!(state.disks.data.is_none());
        assert!(state.volumes.error.is_none());
        state.finish(
            "epoch2/context/node-b",
            Ok(Vec::new()),
            Err("new-node unavailable".into()),
        );
        assert!(!state.loading);
        assert!(state.disks.data.as_ref().unwrap().is_empty());
        assert_eq!(state.volumes.error.as_deref(), Some("new-node unavailable"));
    }
}
