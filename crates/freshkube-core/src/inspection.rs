//! Framework-independent collectors and transforms for node inspection.
//!
//! The collection functions in this module take owned requests and a cloneable
//! [`TalosClient`], so a frontend can execute them on a Tokio worker and send
//! their immutable snapshots back to its UI thread. Primary API failures are
//! returned as errors; optional sources are represented explicitly in the
//! resulting snapshot as unavailable data.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    fmt,
    time::{Duration, Instant},
};

use talos_rs::{
    ConnectionCounts, ConnectionInfo, ConnectionState, CpuStat, EtcdAlarm, EtcdMemberInfo,
    EtcdMemberStatus, KubeSpanPeerStatus, NetDevRate, NetDevStats, NetstatFilter, ProcessInfo,
    ProcessState, ServiceInfo, TalosClient, get_kubespan_peers_for_node,
    is_kubespan_enabled_for_node,
};

use crate::{
    constants::MAX_CAPTURE_SIZE,
    errors::{format_talos_error, format_timeout_error},
    formatting::{format_bytes, format_percent},
    indicators::QuorumState,
    network::{ConnectionDirection, classify_connection, port_to_service_u32},
};

/// A stable identity for a request sent to a particular Talos node.
///
/// `name` is presentation identity, while `address` is the address used to
/// target Talos API calls. Keeping both prevents a worker result from being
/// accidentally applied to a newly selected node with the same display slot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InspectionTarget {
    /// Human-readable Talos node name.
    pub name: String,
    /// Talos API target address for this node.
    pub address: String,
}

impl InspectionTarget {
    /// Creates an explicit node target.
    pub fn new(name: impl Into<String>, address: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            address: address.into(),
        }
    }
}

/// A source which did not prevent a primary inspection snapshot from loading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectionUnavailable {
    /// The source that could not be read.
    pub source: InspectionSource,
    /// User-facing explanation derived from the Talos error or timeout.
    pub message: String,
}

/// A data source used while assembling an inspection snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InspectionSource {
    /// The required process list.
    Processes,
    /// Memory usage for the process summary.
    ProcessMemory,
    /// CPU totals and process scheduling counts.
    ProcessSystemStat,
    /// CPU inventory for the process summary.
    ProcessCpuInfo,
    /// Load-average sample for the process summary.
    ProcessLoadAverage,
    /// The required network interface counters.
    NetworkInterfaces,
    /// Supplementary netstat connection data.
    NetworkConnections,
    /// Supplementary Talos service data.
    NetworkServices,
    /// KubeSpan resource data acquired through talosctl.
    KubeSpan,
    /// The required etcd member list.
    EtcdMembers,
    /// Supplementary etcd member statuses.
    EtcdStatus,
    /// Supplementary etcd alarms.
    EtcdAlarms,
}

impl InspectionSource {
    /// A concise source name suitable for status text.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Processes => "processes",
            Self::ProcessMemory => "memory",
            Self::ProcessSystemStat => "system statistics",
            Self::ProcessCpuInfo => "CPU information",
            Self::ProcessLoadAverage => "load average",
            Self::NetworkInterfaces => "network interfaces",
            Self::NetworkConnections => "network connections",
            Self::NetworkServices => "Talos services",
            Self::KubeSpan => "KubeSpan",
            Self::EtcdMembers => "etcd members",
            Self::EtcdStatus => "etcd status",
            Self::EtcdAlarms => "etcd alarms",
        }
    }
}

impl fmt::Display for InspectionSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// An error from a primary source which makes an inspection snapshot invalid.
///
/// Optional sources use [`InspectionUnavailable`] inside their snapshot
/// instead, so a caller can retain and render the authoritative data that was
/// available.
#[derive(Debug, Clone, thiserror::Error)]
pub enum InspectionError {
    /// A required Talos API source returned an error or timed out.
    #[error(
        "{inspection_source} inspection failed for {target_name} ({target_address}): {message}"
    )]
    RequiredSource {
        /// Required source that failed.
        inspection_source: InspectionSource,
        /// Target display name.
        target_name: String,
        /// Target API address.
        target_address: String,
        /// Formatted failure message.
        message: String,
    },
}

const MIN_DELTA_INTERVAL: Duration = Duration::from_millis(100);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

fn unavailable_from_error(
    source: InspectionSource,
    error: talos_rs::TalosError,
) -> InspectionUnavailable {
    InspectionUnavailable {
        source,
        message: format_talos_error(&error),
    }
}

fn unavailable_from_timeout(source: InspectionSource, timeout: Duration) -> InspectionUnavailable {
    InspectionUnavailable {
        source,
        message: format_timeout_error(timeout.as_secs()),
    }
}

fn required_from_error(
    source: InspectionSource,
    target: &InspectionTarget,
    error: talos_rs::TalosError,
) -> InspectionError {
    InspectionError::RequiredSource {
        inspection_source: source,
        target_name: target.name.clone(),
        target_address: target.address.clone(),
        message: format_talos_error(&error),
    }
}

fn required_from_timeout(
    source: InspectionSource,
    target: &InspectionTarget,
    timeout: Duration,
) -> InspectionError {
    InspectionError::RequiredSource {
        inspection_source: source,
        target_name: target.name.clone(),
        target_address: target.address.clone(),
        message: format_timeout_error(timeout.as_secs()),
    }
}

// -----------------------------------------------------------------------------
// Process inspection
// -----------------------------------------------------------------------------

/// Caller-owned state retained between process samples.
///
/// Supplying this state to the next [`ProcessInspectionRequest`] is what makes
/// CPU percentages delta-based instead of cumulative. A state belonging to a
/// different target is deliberately ignored.
#[derive(Debug, Clone, Default)]
pub struct ProcessSampleState {
    /// Target that produced this state.
    pub target: Option<InspectionTarget>,
    /// Time at which the last process counters were sampled.
    pub sampled_at: Option<Instant>,
    /// Cumulative CPU seconds keyed by process ID.
    pub cpu_times: HashMap<i32, f64>,
    /// Cumulative CPU counters used for total node CPU usage.
    pub cpu_total: Option<CpuStat>,
}

/// Request for a process inspection sample.
#[derive(Debug, Clone)]
pub struct ProcessInspectionRequest {
    /// Node to inspect.
    pub target: InspectionTarget,
    /// Previous sample returned by this collector, if any.
    pub sample: ProcessSampleState,
    /// Per-source network timeout.
    pub timeout: Duration,
}

impl ProcessInspectionRequest {
    /// Creates a request with the standard Talos request timeout.
    pub fn new(target: InspectionTarget, sample: ProcessSampleState) -> Self {
        Self {
            target,
            sample,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// Sort order for process display rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessSort {
    /// Highest delta CPU percentage first.
    #[default]
    CpuPercent,
    /// Highest cumulative CPU time first.
    CpuTime,
    /// Largest resident memory first.
    ResidentMemory,
}

/// Process filtering selected by a frontend.
#[derive(Debug, Clone, Default)]
pub struct ProcessFilter {
    /// Case-insensitive text matched against command, executable, and args.
    pub text: Option<String>,
    /// Exact process state to retain.
    pub state: Option<ProcessState>,
}

/// Whether process rows should be displayed flat or as a process tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessTree {
    /// Display the filtered process list without hierarchy.
    #[default]
    Flat,
    /// Display all filtered process roots and their filtered descendants.
    Full,
    /// Display the selected process and its filtered descendants. Processes
    /// whose parent was filtered out remain visible as roots.
    Subtree {
        /// PID of the requested subtree root.
        root_pid: i32,
    },
}

/// A pure display transform for a process snapshot.
#[derive(Debug, Clone, Default)]
pub struct ProcessView {
    /// Requested sort order.
    pub sort: ProcessSort,
    /// Text and state filters.
    pub filter: ProcessFilter,
    /// Requested hierarchy.
    pub tree: ProcessTree,
}

/// Process counts grouped by the states used by the original inspection view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessStateCounts {
    /// Runnable processes.
    pub running: usize,
    /// Interruptible sleeping processes.
    pub sleeping: usize,
    /// Uninterruptible I/O waits.
    pub disk_sleep: usize,
    /// Exited but not reaped processes.
    pub zombie: usize,
}

/// A process together with its delta-based CPU value.
#[derive(Debug, Clone)]
pub struct ProcessSnapshotEntry {
    /// Authoritative Talos process record.
    pub process: ProcessInfo,
    /// Percentage of one CPU consumed since the previous compatible sample.
    /// `None` means this is the first sample, the PID is new, or the samples
    /// were too close together to calculate a reliable rate.
    pub cpu_percent: Option<f32>,
}

impl ProcessSnapshotEntry {
    /// Formats resident memory using the shared core formatter.
    pub fn resident_memory_display(&self) -> String {
        format_bytes(self.process.resident_memory)
    }

    /// Formats the CPU delta when it is available.
    pub fn cpu_percent_display(&self) -> Option<String> {
        self.cpu_percent
            .map(|percent| format_percent(percent as f64))
    }
}

/// Memory usage associated with a process inspection sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessMemorySnapshot {
    /// Total node memory in bytes.
    pub total_bytes: u64,
    /// Node memory in use in bytes.
    pub used_bytes: u64,
    /// Node memory usage percentage.
    pub used_percent: f32,
}

impl ProcessMemorySnapshot {
    /// Formats total and used memory for a compact summary.
    pub fn display(&self) -> String {
        format!(
            "{} / {} ({})",
            format_bytes(self.used_bytes),
            format_bytes(self.total_bytes),
            format_percent(self.used_percent as f64)
        )
    }
}

/// Node CPU counters associated with a process inspection sample.
#[derive(Debug, Clone)]
pub struct ProcessCpuSnapshot {
    /// Current cumulative CPU counters from Talos.
    pub totals: CpuStat,
    /// Delta-based total CPU usage. Missing on the first compatible sample.
    pub usage_percent: Option<f32>,
    /// Number of running processes reported by Talos, when available.
    pub running_processes: u64,
    /// Number of blocked processes reported by Talos, when available.
    pub blocked_processes: u64,
}

impl ProcessCpuSnapshot {
    /// Formats total CPU usage when a delta is available.
    pub fn usage_display(&self) -> Option<String> {
        self.usage_percent
            .map(|percent| format_percent(percent as f64))
    }
}

/// Talos load averages associated with a process inspection sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoadAverageSnapshot {
    /// One-minute load average.
    pub one_minute: f64,
    /// Five-minute load average.
    pub five_minutes: f64,
    /// Fifteen-minute load average.
    pub fifteen_minutes: f64,
}

/// Supplementary host-level process data.
#[derive(Debug, Clone, Default)]
pub struct ProcessSystemSnapshot {
    /// Memory data, when Talos served it.
    pub memory: Option<ProcessMemorySnapshot>,
    /// CPU inventory, independent of whether cumulative system statistics loaded.
    pub cpu_count: Option<usize>,
    /// CPU data, when Talos served system statistics.
    pub cpu: Option<ProcessCpuSnapshot>,
    /// Load averages, when Talos served them.
    pub load_average: Option<LoadAverageSnapshot>,
}

/// Immutable process inspection result.
#[derive(Debug, Clone)]
pub struct ProcessInspectionSnapshot {
    /// Node that was inspected.
    pub target: InspectionTarget,
    /// Time at which process counters were sampled.
    pub sampled_at: Instant,
    /// Authoritative process rows from Talos.
    pub processes: Vec<ProcessSnapshotEntry>,
    /// Counts by process state.
    pub state_counts: ProcessStateCounts,
    /// Supplementary host-level data.
    pub system: ProcessSystemSnapshot,
    /// State to supply to the next request for delta calculations.
    pub next_sample: ProcessSampleState,
    /// Optional Talos sources that were unavailable for this sample.
    pub unavailable: Vec<InspectionUnavailable>,
}

impl ProcessInspectionSnapshot {
    /// Whether any supplementary process data source was unavailable.
    pub fn is_partial(&self) -> bool {
        !self.unavailable.is_empty()
    }

    /// Produces UI-neutral rows for the supplied filter, sort, and tree view.
    pub fn display_rows(&self, view: &ProcessView) -> Vec<ProcessDisplayRow> {
        build_process_display_rows(&self.processes, view)
    }
}

/// A row into [`ProcessInspectionSnapshot::processes`].
///
/// The process remains in the snapshot rather than being copied, which keeps
/// view transforms cheap and leaves all display state with the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessDisplayRow {
    /// Index into the source process snapshot.
    pub process_index: usize,
    /// Zero-based process-tree depth.
    pub depth: usize,
    /// Whether this is the last sibling at its depth.
    pub is_last: bool,
    /// For each ancestor depth, whether its connector continues below this row.
    pub ancestors_have_siblings: Vec<bool>,
}

/// Collects process state and supplementary host metrics from one Talos node.
///
/// Process data is primary. Memory, CPU inventory, system statistics, and load
/// averages remain independently visible as unavailable rather than turning a
/// usable process result into a failed snapshot.
pub async fn collect_process_inspection(
    client: TalosClient,
    request: ProcessInspectionRequest,
) -> Result<ProcessInspectionSnapshot, InspectionError> {
    let target_client = client.with_node(&request.target.address);
    let timeout = request.timeout;

    let (process_result, memory_result, stat_result, cpu_info_result, load_result) = tokio::join!(
        tokio::time::timeout(timeout, target_client.processes()),
        tokio::time::timeout(timeout, target_client.memory()),
        tokio::time::timeout(timeout, target_client.system_stat()),
        tokio::time::timeout(timeout, target_client.cpu_info()),
        tokio::time::timeout(timeout, target_client.load_avg()),
    );

    let raw_processes = match process_result {
        Ok(Ok(nodes)) => nodes
            .into_iter()
            .next()
            .map(|node| node.processes)
            .unwrap_or_default(),
        Ok(Err(error)) => {
            return Err(required_from_error(
                InspectionSource::Processes,
                &request.target,
                error,
            ));
        }
        Err(_) => {
            return Err(required_from_timeout(
                InspectionSource::Processes,
                &request.target,
                timeout,
            ));
        }
    };

    let sampled_at = Instant::now();
    let mut unavailable = Vec::new();
    let mut system = ProcessSystemSnapshot::default();

    match memory_result {
        Ok(Ok(nodes)) => match nodes.into_iter().next().and_then(|node| node.meminfo) {
            Some(memory) => {
                system.memory = Some(ProcessMemorySnapshot {
                    total_bytes: memory.mem_total,
                    used_bytes: memory.mem_total.saturating_sub(memory.mem_available),
                    used_percent: memory.usage_percent(),
                });
            }
            None => unavailable.push(InspectionUnavailable {
                source: InspectionSource::ProcessMemory,
                message: "Talos returned no memory data for the selected node".to_string(),
            }),
        },
        Ok(Err(error)) => unavailable.push(unavailable_from_error(
            InspectionSource::ProcessMemory,
            error,
        )),
        Err(_) => unavailable.push(unavailable_from_timeout(
            InspectionSource::ProcessMemory,
            timeout,
        )),
    }

    let cpu_count = match cpu_info_result {
        Ok(Ok(nodes)) => match nodes.into_iter().next() {
            Some(cpu) => Some(cpu.cpu_count),
            None => {
                unavailable.push(InspectionUnavailable {
                    source: InspectionSource::ProcessCpuInfo,
                    message: "Talos returned no CPU inventory for the selected node".to_string(),
                });
                None
            }
        },
        Ok(Err(error)) => {
            unavailable.push(unavailable_from_error(
                InspectionSource::ProcessCpuInfo,
                error,
            ));
            None
        }
        Err(_) => {
            unavailable.push(unavailable_from_timeout(
                InspectionSource::ProcessCpuInfo,
                timeout,
            ));
            None
        }
    };

    system.cpu_count = cpu_count;

    let cpu_total = match stat_result {
        Ok(Ok(nodes)) => match nodes.into_iter().next() {
            Some(stat) => {
                let usage_percent = compatible_process_sample(&request.sample, &request.target)
                    .and_then(|sample| sample.cpu_total.as_ref())
                    .map(|previous| CpuStat::usage_percent_from(previous, &stat.cpu_total));
                let totals = stat.cpu_total.clone();
                system.cpu = Some(ProcessCpuSnapshot {
                    totals: totals.clone(),
                    usage_percent,
                    running_processes: stat.process_running,
                    blocked_processes: stat.process_blocked,
                });
                Some(totals)
            }
            None => {
                unavailable.push(InspectionUnavailable {
                    source: InspectionSource::ProcessSystemStat,
                    message: "Talos returned no system statistics for the selected node"
                        .to_string(),
                });
                None
            }
        },
        Ok(Err(error)) => {
            unavailable.push(unavailable_from_error(
                InspectionSource::ProcessSystemStat,
                error,
            ));
            None
        }
        Err(_) => {
            unavailable.push(unavailable_from_timeout(
                InspectionSource::ProcessSystemStat,
                timeout,
            ));
            None
        }
    };

    match load_result {
        Ok(Ok(nodes)) => match nodes.into_iter().next() {
            Some(load) => {
                system.load_average = Some(LoadAverageSnapshot {
                    one_minute: load.load1,
                    five_minutes: load.load5,
                    fifteen_minutes: load.load15,
                });
            }
            None => unavailable.push(InspectionUnavailable {
                source: InspectionSource::ProcessLoadAverage,
                message: "Talos returned no load averages for the selected node".to_string(),
            }),
        },
        Ok(Err(error)) => unavailable.push(unavailable_from_error(
            InspectionSource::ProcessLoadAverage,
            error,
        )),
        Err(_) => unavailable.push(unavailable_from_timeout(
            InspectionSource::ProcessLoadAverage,
            timeout,
        )),
    }

    let cpu_percentages = calculate_process_cpu_percentages(
        &raw_processes,
        compatible_process_sample(&request.sample, &request.target),
        sampled_at,
    );
    let state_counts = process_state_counts(&raw_processes);
    let next_sample = ProcessSampleState {
        target: Some(request.target.clone()),
        sampled_at: Some(sampled_at),
        cpu_times: raw_processes
            .iter()
            .map(|process| (process.pid, process.cpu_time))
            .collect(),
        cpu_total,
    };
    let processes = raw_processes
        .into_iter()
        .map(|process| ProcessSnapshotEntry {
            cpu_percent: cpu_percentages.get(&process.pid).copied(),
            process,
        })
        .collect();

    Ok(ProcessInspectionSnapshot {
        target: request.target,
        sampled_at,
        processes,
        state_counts,
        system,
        next_sample,
        unavailable,
    })
}

fn compatible_process_sample<'a>(
    sample: &'a ProcessSampleState,
    target: &InspectionTarget,
) -> Option<&'a ProcessSampleState> {
    (sample.target.as_ref() == Some(target)).then_some(sample)
}

fn calculate_process_cpu_percentages(
    processes: &[ProcessInfo],
    previous: Option<&ProcessSampleState>,
    sampled_at: Instant,
) -> HashMap<i32, f32> {
    let Some(previous) = previous else {
        return HashMap::new();
    };
    let Some(previous_at) = previous.sampled_at else {
        return HashMap::new();
    };
    let Some(elapsed) = sampled_at.checked_duration_since(previous_at) else {
        return HashMap::new();
    };
    if elapsed < MIN_DELTA_INTERVAL {
        return HashMap::new();
    }

    let elapsed_seconds = elapsed.as_secs_f64();
    processes
        .iter()
        .filter_map(|process| {
            previous.cpu_times.get(&process.pid).map(|previous_cpu| {
                let cpu_seconds = (process.cpu_time - previous_cpu).max(0.0);
                (
                    process.pid,
                    ((cpu_seconds / elapsed_seconds) * 100.0) as f32,
                )
            })
        })
        .collect()
}

fn process_state_counts(processes: &[ProcessInfo]) -> ProcessStateCounts {
    processes
        .iter()
        .fold(ProcessStateCounts::default(), |mut counts, process| {
            match process.state {
                ProcessState::Running => counts.running += 1,
                ProcessState::Sleeping => counts.sleeping += 1,
                ProcessState::DiskSleep => counts.disk_sleep += 1,
                ProcessState::Zombie => counts.zombie += 1,
                _ => {}
            }
            counts
        })
}

/// Builds flat, tree, or subtree process rows without any UI framework state.
pub fn build_process_display_rows(
    processes: &[ProcessSnapshotEntry],
    view: &ProcessView,
) -> Vec<ProcessDisplayRow> {
    let text_filter = view.filter.text.as_ref().map(|text| text.to_lowercase());
    let mut indices: Vec<usize> = processes
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            process_matches(entry, text_filter.as_deref(), view.filter.state.as_ref())
        })
        .map(|(index, _)| index)
        .collect();

    indices.sort_by(|left, right| {
        compare_process_entries(&processes[*left], &processes[*right], view.sort)
    });

    match view.tree {
        ProcessTree::Flat => indices
            .into_iter()
            .map(|process_index| ProcessDisplayRow {
                process_index,
                depth: 0,
                is_last: false,
                ancestors_have_siblings: Vec::new(),
            })
            .collect(),
        ProcessTree::Full => build_process_tree_rows(processes, indices, None),
        ProcessTree::Subtree { root_pid } => {
            build_process_tree_rows(processes, indices, Some(root_pid))
        }
    }
}

fn process_matches(
    entry: &ProcessSnapshotEntry,
    text_filter: Option<&str>,
    state_filter: Option<&ProcessState>,
) -> bool {
    let text_matches = text_filter.is_none_or(|filter| {
        entry.process.command.to_lowercase().contains(filter)
            || entry.process.executable.to_lowercase().contains(filter)
            || entry.process.args.to_lowercase().contains(filter)
    });
    let state_matches = state_filter.is_none_or(|state| entry.process.state == *state);
    text_matches && state_matches
}

fn compare_process_entries(
    left: &ProcessSnapshotEntry,
    right: &ProcessSnapshotEntry,
    sort: ProcessSort,
) -> Ordering {
    match sort {
        ProcessSort::CpuPercent => right
            .cpu_percent
            .unwrap_or(0.0)
            .total_cmp(&left.cpu_percent.unwrap_or(0.0)),
        ProcessSort::CpuTime => right.process.cpu_time.total_cmp(&left.process.cpu_time),
        ProcessSort::ResidentMemory => right
            .process
            .resident_memory
            .cmp(&left.process.resident_memory),
    }
}

fn build_process_tree_rows(
    processes: &[ProcessSnapshotEntry],
    sorted_indices: Vec<usize>,
    subtree_root: Option<i32>,
) -> Vec<ProcessDisplayRow> {
    let pid_to_index: HashMap<i32, usize> = processes
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.process.pid, index))
        .collect();
    let all_children = child_map(processes, 0..processes.len());

    let allowed_pids =
        subtree_root.map(|root_pid| descendant_pids(root_pid, processes, &all_children));
    let selected: Vec<usize> = sorted_indices
        .into_iter()
        .filter(|index| {
            allowed_pids
                .as_ref()
                .is_none_or(|pids| pids.contains(&processes[*index].process.pid))
        })
        .collect();
    let selected_set: HashSet<usize> = selected.iter().copied().collect();
    let children = child_map(
        processes,
        selected.iter().copied().filter(|index| {
            pid_to_index
                .get(&processes[*index].process.ppid)
                .is_some_and(|parent| selected_set.contains(parent))
        }),
    );

    let mut roots: Vec<usize> = selected
        .iter()
        .copied()
        .filter(|index| {
            let process = &processes[*index].process;
            subtree_root.is_some_and(|root_pid| process.pid == root_pid)
                || process.ppid == 0
                || process.ppid == process.pid
                || !pid_to_index
                    .get(&process.ppid)
                    .is_some_and(|parent| selected_set.contains(parent))
        })
        .collect();

    let mut rows = Vec::with_capacity(selected.len());
    let mut visited = HashSet::with_capacity(selected.len());
    append_process_tree_rows(
        &mut rows,
        &mut visited,
        &children,
        processes,
        &roots,
        0,
        &[],
    );

    // A malformed process table can contain a parent cycle. Treat each unseen
    // process as a root so every authoritative process remains visible.
    for index in selected {
        if visited.contains(&index) {
            continue;
        }
        roots.clear();
        roots.push(index);
        append_process_tree_rows(
            &mut rows,
            &mut visited,
            &children,
            processes,
            &roots,
            0,
            &[],
        );
    }

    rows
}

fn child_map(
    processes: &[ProcessSnapshotEntry],
    indices: impl IntoIterator<Item = usize>,
) -> HashMap<i32, Vec<usize>> {
    let mut children = HashMap::<i32, Vec<usize>>::new();
    for index in indices {
        children
            .entry(processes[index].process.ppid)
            .or_default()
            .push(index);
    }
    children
}

fn descendant_pids(
    root_pid: i32,
    processes: &[ProcessSnapshotEntry],
    children: &HashMap<i32, Vec<usize>>,
) -> HashSet<i32> {
    let mut descendants = HashSet::new();
    let mut pending = VecDeque::from([root_pid]);
    while let Some(pid) = pending.pop_front() {
        if !descendants.insert(pid) {
            continue;
        }
        if let Some(child_indices) = children.get(&pid) {
            pending.extend(
                child_indices
                    .iter()
                    .map(|index| processes[*index].process.pid),
            );
        }
    }
    descendants
}

fn append_process_tree_rows(
    rows: &mut Vec<ProcessDisplayRow>,
    visited: &mut HashSet<usize>,
    children: &HashMap<i32, Vec<usize>>,
    processes: &[ProcessSnapshotEntry],
    indices: &[usize],
    depth: usize,
    ancestors_have_siblings: &[bool],
) {
    for (position, index) in indices.iter().copied().enumerate() {
        if !visited.insert(index) {
            continue;
        }
        let is_last = position + 1 == indices.len();
        rows.push(ProcessDisplayRow {
            process_index: index,
            depth,
            is_last,
            ancestors_have_siblings: ancestors_have_siblings.to_vec(),
        });

        if let Some(child_indices) = children.get(&processes[index].process.pid) {
            let mut child_ancestors = ancestors_have_siblings.to_vec();
            child_ancestors.push(!is_last);
            append_process_tree_rows(
                rows,
                visited,
                children,
                processes,
                child_indices,
                depth + 1,
                &child_ancestors,
            );
        }
    }
}

// -----------------------------------------------------------------------------
// Network inspection
// -----------------------------------------------------------------------------

/// Caller-owned state retained between network counter samples.
#[derive(Debug, Clone, Default)]
pub struct NetworkSampleState {
    /// Target that produced this state.
    pub target: Option<InspectionTarget>,
    /// Time when the interface counters were captured.
    pub sampled_at: Option<Instant>,
    /// Previous counter values by interface name.
    pub devices: HashMap<String, NetDevStats>,
}

/// Explicit talosctl target (context and talosconfig) for `talosctl get` queries.
///
/// Without one, talosctl falls back to the user's ambient default context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalosctlTarget {
    /// Talos context name passed as `--context`.
    pub context: String,
    /// Talosconfig file passed as `--talosconfig`.
    pub config_path: String,
}

impl TalosctlTarget {
    /// Creates an explicit talosctl target.
    pub fn new(context: impl Into<String>, config_path: impl Into<String>) -> Self {
        Self {
            context: context.into(),
            config_path: config_path.into(),
        }
    }
}

/// Request for a network inspection sample.
#[derive(Debug, Clone)]
pub struct NetworkInspectionRequest {
    /// Node to inspect.
    pub target: InspectionTarget,
    /// Previous interface counter sample.
    pub sample: NetworkSampleState,
    /// Per-source network timeout.
    pub timeout: Duration,
}

impl NetworkInspectionRequest {
    /// Creates a request with the standard timeout.
    pub fn new(target: InspectionTarget, sample: NetworkSampleState) -> Self {
        Self {
            target,
            sample,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// An interface's counters plus its rate derived from the prior sample.
#[derive(Debug, Clone)]
pub struct NetworkInterfaceSnapshot {
    /// Authoritative Talos interface counters.
    pub stats: NetDevStats,
    /// Delta rate, absent until a compatible previous sample is available.
    pub rate: Option<NetDevRate>,
}

impl NetworkInterfaceSnapshot {
    /// Formats received bytes using the shared formatter.
    pub fn received_display(&self) -> String {
        format_bytes(self.stats.rx_bytes)
    }

    /// Formats transmitted bytes using the shared formatter.
    pub fn transmitted_display(&self) -> String {
        format_bytes(self.stats.tx_bytes)
    }

    /// Formats the receive rate when a prior sample was available.
    pub fn receive_rate_display(&self) -> Option<String> {
        self.rate
            .as_ref()
            .map(|rate| format!("{}/s", format_bytes(rate.rx_bytes_per_sec)))
    }

    /// Formats the transmit rate when a prior sample was available.
    pub fn transmit_rate_display(&self) -> Option<String> {
        self.rate
            .as_ref()
            .map(|rate| format!("{}/s", format_bytes(rate.tx_bytes_per_sec)))
    }
}

/// Totals calculated from the collected interface statistics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetworkTotals {
    /// Sum of available RX interface rates.
    pub rx_bytes_per_sec: u64,
    /// Sum of available TX interface rates.
    pub tx_bytes_per_sec: u64,
    /// Cumulative RX and TX errors.
    pub errors: u64,
    /// Cumulative RX and TX drops.
    pub dropped: u64,
}

/// A classified listener socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenerSnapshot {
    /// Port on which the process is listening.
    pub port: u32,
    /// Known Talos/Kubernetes service name, if the port is recognized.
    pub service: Option<&'static str>,
    /// Owning process ID reported by netstat, if available.
    pub process_pid: Option<u32>,
    /// Owning process name reported by netstat, if available.
    pub process_name: Option<String>,
}

/// A netstat connection enriched with core port and direction classification.
#[derive(Debug, Clone)]
pub struct NetworkConnectionSnapshot {
    /// Authoritative Talos netstat record.
    pub connection: ConnectionInfo,
    /// Inbound, outbound, or unknown direction based on known service ports.
    pub direction: ConnectionDirection,
    /// Known local service, if the port is recognized.
    pub local_service: Option<&'static str>,
    /// Known remote service, if the port is recognized.
    pub remote_service: Option<&'static str>,
    /// Listener metadata when the connection is in LISTEN state.
    pub listener: Option<ListenerSnapshot>,
}

/// Supplementary connection data from netstat.
#[derive(Debug, Clone)]
pub struct NetworkConnectionsSnapshot {
    /// Classified connections from the selected node.
    pub connections: Vec<NetworkConnectionSnapshot>,
    /// Counts grouped by TCP state.
    pub counts: ConnectionCounts,
    /// Sockets in LISTEN state, classified by known service port where possible.
    pub listeners: Vec<ListenerSnapshot>,
}

/// KubeSpan data collected through the Talos-supported `talosctl get` path.
#[derive(Debug, Clone)]
pub enum KubeSpanSnapshot {
    /// KubeSpan configuration is not enabled and the peer query was available.
    Disabled,
    /// KubeSpan is enabled, with the current peer records.
    Enabled {
        /// Authoritative KubeSpan peer status resources.
        peers: Vec<KubeSpanPeerStatus>,
    },
    /// KubeSpan resource state could not be determined.
    Unavailable {
        /// Formatted failure explanation.
        message: String,
    },
}

impl KubeSpanSnapshot {
    /// Returns peers only when KubeSpan state was successfully loaded as enabled.
    pub fn peers(&self) -> Option<&[KubeSpanPeerStatus]> {
        match self {
            Self::Enabled { peers } => Some(peers),
            Self::Disabled | Self::Unavailable { .. } => None,
        }
    }
}

/// Framework-neutral metadata for a bounded packet-capture action.
///
/// This intentionally contains no stream receiver, byte buffer, progress
/// indicator, or other UI state. A consumer must stop accepting chunks after
/// `max_bytes` and can use this metadata to render an action confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketCaptureMetadata {
    /// Explicit node target for the capture.
    pub target: InspectionTarget,
    /// Interface requested for capture.
    pub interface: String,
    /// Whether Talos should enable promiscuous mode.
    pub promiscuous: bool,
    /// Per-packet snap length. Zero retains Talos's default of 65535 bytes.
    pub snap_len: u32,
    /// Whether the capture should use Talos's API-port exclusion filter.
    pub excludes_talos_api_traffic: bool,
    /// Maximum pcap bytes a consumer may retain.
    pub max_bytes: usize,
}

/// Requested packet-capture parameters before applying the core retention cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketCaptureRequest {
    /// Explicit node target for the capture.
    pub target: InspectionTarget,
    /// Interface requested for capture.
    pub interface: String,
    /// Whether Talos should enable promiscuous mode. Defaults to `false`.
    pub promiscuous: bool,
    /// Per-packet snap length. Zero delegates to Talos's safe default.
    pub snap_len: u32,
    /// Exclude Talos API traffic to avoid a management-interface feedback loop.
    pub exclude_talos_api_traffic: bool,
    /// Requested retained pcap size, constrained to [`MAX_CAPTURE_SIZE`].
    pub max_bytes: usize,
}

impl PacketCaptureRequest {
    /// Creates a conservative packet-capture request for an interface.
    pub fn new(target: InspectionTarget, interface: impl Into<String>) -> Self {
        Self {
            target,
            interface: interface.into(),
            promiscuous: false,
            snap_len: 0,
            exclude_talos_api_traffic: true,
            max_bytes: MAX_CAPTURE_SIZE,
        }
    }

    /// Returns UI-neutral metadata with the shared pcap retention cap applied.
    pub fn metadata(&self) -> PacketCaptureMetadata {
        PacketCaptureMetadata {
            target: self.target.clone(),
            interface: self.interface.clone(),
            promiscuous: self.promiscuous,
            snap_len: self.snap_len,
            excludes_talos_api_traffic: self.exclude_talos_api_traffic,
            max_bytes: self.max_bytes.min(MAX_CAPTURE_SIZE),
        }
    }
}

/// Immutable network inspection result.
#[derive(Debug, Clone)]
pub struct NetworkInspectionSnapshot {
    /// Node that was inspected.
    pub target: InspectionTarget,
    /// Time at which interface counters were sampled.
    pub sampled_at: Instant,
    /// Required Talos network-interface data.
    pub interfaces: Vec<NetworkInterfaceSnapshot>,
    /// Totals derived from interface counters and available rates.
    pub totals: NetworkTotals,
    /// Supplementary netstat data, if available.
    pub connections: Option<NetworkConnectionsSnapshot>,
    /// Supplementary Talos service data, if available.
    pub services: Option<Vec<ServiceInfo>>,
    /// State to supply to the next request for rate calculations.
    pub next_sample: NetworkSampleState,
    /// Optional sources unavailable during collection.
    pub unavailable: Vec<InspectionUnavailable>,
}

impl NetworkInspectionSnapshot {
    /// Whether primary interface data was enriched only partially.
    pub fn is_partial(&self) -> bool {
        !self.unavailable.is_empty()
    }
}

/// Collects interface counters as primary data and enriches them with optional
/// netstat and service sources.
pub async fn collect_network_inspection(
    client: TalosClient,
    request: NetworkInspectionRequest,
) -> Result<NetworkInspectionSnapshot, InspectionError> {
    let target_client = client.with_node(&request.target.address);
    let timeout = request.timeout;

    let (interface_result, connection_result, service_result) = tokio::join!(
        tokio::time::timeout(timeout, target_client.network_device_stats()),
        tokio::time::timeout(timeout, target_client.netstat(NetstatFilter::All)),
        tokio::time::timeout(timeout, target_client.services()),
    );

    let devices = match interface_result {
        Ok(Ok(nodes)) => nodes
            .into_iter()
            .next()
            .map(|node| node.devices)
            .unwrap_or_default(),
        Ok(Err(error)) => {
            return Err(required_from_error(
                InspectionSource::NetworkInterfaces,
                &request.target,
                error,
            ));
        }
        Err(_) => {
            return Err(required_from_timeout(
                InspectionSource::NetworkInterfaces,
                &request.target,
                timeout,
            ));
        }
    };

    let sampled_at = Instant::now();
    let (interfaces, totals, next_sample) = assemble_network_interfaces(
        devices,
        compatible_network_sample(&request.sample, &request.target),
        request.target.clone(),
        sampled_at,
    );

    let mut unavailable = Vec::new();
    let connections = match connection_result {
        Ok(Ok(nodes)) => Some(inspect_network_connections(
            nodes
                .into_iter()
                .next()
                .map(|node| node.connections)
                .unwrap_or_default(),
        )),
        Ok(Err(error)) => {
            unavailable.push(unavailable_from_error(
                InspectionSource::NetworkConnections,
                error,
            ));
            None
        }
        Err(_) => {
            unavailable.push(unavailable_from_timeout(
                InspectionSource::NetworkConnections,
                timeout,
            ));
            None
        }
    };
    let services = match service_result {
        Ok(Ok(nodes)) => Some(
            nodes
                .into_iter()
                .next()
                .map(|node| node.services)
                .unwrap_or_default(),
        ),
        Ok(Err(error)) => {
            unavailable.push(unavailable_from_error(
                InspectionSource::NetworkServices,
                error,
            ));
            None
        }
        Err(_) => {
            unavailable.push(unavailable_from_timeout(
                InspectionSource::NetworkServices,
                timeout,
            ));
            None
        }
    };

    Ok(NetworkInspectionSnapshot {
        target: request.target,
        sampled_at,
        interfaces,
        totals,
        connections,
        services,
        next_sample,
        unavailable,
    })
}

fn compatible_network_sample<'a>(
    sample: &'a NetworkSampleState,
    target: &InspectionTarget,
) -> Option<&'a NetworkSampleState> {
    (sample.target.as_ref() == Some(target)).then_some(sample)
}

/// Assembles interface rates and totals from authoritative counter samples.
///
/// Supplying `sampled_at` keeps this transform deterministic and makes it safe
/// to unit test without a Tokio runtime.
pub fn assemble_network_interfaces(
    devices: Vec<NetDevStats>,
    previous: Option<&NetworkSampleState>,
    target: InspectionTarget,
    sampled_at: Instant,
) -> (
    Vec<NetworkInterfaceSnapshot>,
    NetworkTotals,
    NetworkSampleState,
) {
    let previous = previous.filter(|sample| sample.target.as_ref() == Some(&target));
    let elapsed = previous
        .and_then(|sample| sample.sampled_at)
        .and_then(|previous_at| sampled_at.checked_duration_since(previous_at))
        .filter(|duration| *duration >= MIN_DELTA_INTERVAL);
    let previous_devices = previous.map(|sample| &sample.devices);

    let interfaces: Vec<NetworkInterfaceSnapshot> = devices
        .iter()
        .cloned()
        .map(|stats| {
            let rate = elapsed.and_then(|duration| {
                previous_devices
                    .and_then(|device_map| device_map.get(&stats.name))
                    .map(|previous_stats| {
                        NetDevRate::from_delta(previous_stats, &stats, duration.as_secs_f64())
                    })
            });
            NetworkInterfaceSnapshot { stats, rate }
        })
        .collect();
    let totals = NetworkTotals {
        rx_bytes_per_sec: interfaces
            .iter()
            .filter_map(|interface| interface.rate.as_ref())
            .map(|rate| rate.rx_bytes_per_sec)
            .sum(),
        tx_bytes_per_sec: interfaces
            .iter()
            .filter_map(|interface| interface.rate.as_ref())
            .map(|rate| rate.tx_bytes_per_sec)
            .sum(),
        errors: interfaces
            .iter()
            .map(|interface| interface.stats.total_errors())
            .sum(),
        dropped: interfaces
            .iter()
            .map(|interface| interface.stats.total_dropped())
            .sum(),
    };
    let next_sample = NetworkSampleState {
        target: Some(target),
        sampled_at: Some(sampled_at),
        devices: devices
            .into_iter()
            .map(|device| (device.name.clone(), device))
            .collect(),
    };

    (interfaces, totals, next_sample)
}

/// Enriches raw Talos netstat data with port, direction, and listener details.
pub fn inspect_network_connections(connections: Vec<ConnectionInfo>) -> NetworkConnectionsSnapshot {
    let counts = ConnectionCounts::count_by_state(&connections);
    let mut listeners = Vec::new();
    let connections = connections
        .into_iter()
        .map(|connection| {
            let local_service = port_to_service_u32(connection.local_port);
            let remote_service = port_to_service_u32(connection.remote_port);
            let listener =
                (connection.state == ConnectionState::Listen).then(|| ListenerSnapshot {
                    port: connection.local_port,
                    service: local_service,
                    process_pid: connection.process_pid,
                    process_name: connection.process_name.clone(),
                });
            if let Some(listener) = &listener {
                listeners.push(listener.clone());
            }
            let direction = if listener.is_some() {
                ConnectionDirection::Inbound
            } else if let (Ok(local_port), Ok(remote_port)) = (
                u16::try_from(connection.local_port),
                u16::try_from(connection.remote_port),
            ) {
                classify_connection(local_port, remote_port)
            } else {
                ConnectionDirection::Unknown
            };

            NetworkConnectionSnapshot {
                connection,
                direction,
                local_service,
                remote_service,
                listener,
            }
        })
        .collect();

    NetworkConnectionsSnapshot {
        connections,
        counts,
        listeners,
    }
}

/// Collects only KubeSpan for a node: two `talosctl get` queries, so callers
/// ask for it when it is shown rather than with every network refresh.
pub async fn collect_kubespan_inspection(
    target: &InspectionTarget,
    talosctl: TalosctlTarget,
) -> KubeSpanSnapshot {
    collect_kubespan_for_target(&target.address, &talosctl, DEFAULT_TIMEOUT).await
}

/// The host of a node address, without its port, for talosctl's `-n`.
fn talosctl_host(address: &str) -> &str {
    talos_rs::target_host(address.trim())
}

/// Rejects values that talosctl would parse as flags or that are empty.
fn valid_talosctl_arg(value: &str) -> bool {
    !value.trim().is_empty() && !value.starts_with('-')
}

async fn collect_kubespan_for_target(
    address: &str,
    target: &TalosctlTarget,
    timeout: Duration,
) -> KubeSpanSnapshot {
    let host = talosctl_host(address);
    if !valid_talosctl_arg(host)
        || !valid_talosctl_arg(&target.context)
        || !valid_talosctl_arg(&target.config_path)
    {
        return KubeSpanSnapshot::Unavailable {
            message: "an explicit Talos context, talosconfig and node address are required to query KubeSpan"
                .to_string(),
        };
    }
    let context = target.context.as_str();
    let config = Some(target.config_path.as_str());

    let queries = async {
        tokio::join!(
            is_kubespan_enabled_for_node(context, host, config),
            get_kubespan_peers_for_node(context, host, config),
        )
    };
    match tokio::time::timeout(timeout, queries).await {
        Ok((enabled, peers)) => kubespan_snapshot_from(enabled, peers),
        Err(_) => KubeSpanSnapshot::Unavailable {
            message: format_timeout_error(timeout.as_secs()),
        },
    }
}

/// Combines the enabled and peer queries. A failed query is never "disabled".
fn kubespan_snapshot_from(
    enabled: Result<bool, talos_rs::TalosError>,
    peers: Result<Vec<KubeSpanPeerStatus>, talos_rs::TalosError>,
) -> KubeSpanSnapshot {
    match (enabled, peers) {
        (_, Err(error)) => KubeSpanSnapshot::Unavailable {
            message: format_talos_error(&error),
        },
        (Ok(true), Ok(peers)) => KubeSpanSnapshot::Enabled { peers },
        (_, Ok(peers)) if !peers.is_empty() => KubeSpanSnapshot::Enabled { peers },
        (Ok(false), Ok(_)) => KubeSpanSnapshot::Disabled,
        // Peers are empty and the configuration couldn't be read.
        (Err(error), Ok(_)) => KubeSpanSnapshot::Unavailable {
            message: format_talos_error(&error),
        },
    }
}

// -----------------------------------------------------------------------------
// Etcd inspection
// -----------------------------------------------------------------------------

/// Request for a detailed etcd health sample.
///
/// The target identifies the context endpoint which initiated the member-list
/// request. Individual status calls are then explicitly targeted to addresses
/// supplied by the authoritative etcd member list.
#[derive(Debug, Clone)]
pub struct EtcdInspectionRequest {
    /// Display name and address of the context endpoint used for this sample.
    pub target: InspectionTarget,
    /// Per-source network timeout.
    pub timeout: Duration,
}

impl EtcdInspectionRequest {
    /// Creates a request with the standard Talos request timeout.
    pub fn new(target: InspectionTarget) -> Self {
        Self {
            target,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// Authoritative etcd-member data correlated with status and alarm responses.
#[derive(Debug, Clone)]
pub struct EtcdMemberSnapshot {
    /// Entry from the required etcd member list.
    pub info: EtcdMemberInfo,
    /// Member status, absent when status collection was unavailable or this
    /// member did not respond.
    pub status: Option<EtcdMemberStatus>,
    /// Alarms whose member ID matches this member.
    pub alarms: Vec<EtcdAlarm>,
}

impl EtcdMemberSnapshot {
    /// Whether Talos returned an etcd status record for this member.
    pub fn is_reachable(&self) -> bool {
        self.status.is_some()
    }

    /// Whether the member is the reported etcd leader.
    pub fn is_leader(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(EtcdMemberStatus::is_leader)
    }

    /// Whether the member has an alarm or a status-reported error.
    pub fn has_problems(&self) -> bool {
        !self.alarms.is_empty()
            || self
                .status
                .as_ref()
                .is_some_and(|status| !status.errors.is_empty())
    }
}

/// Immutable, correlated etcd health result.
#[derive(Debug, Clone)]
pub struct EtcdHealthSnapshot {
    /// Endpoint target that initiated this cluster-wide inspection.
    pub target: InspectionTarget,
    /// Members in the authoritative etcd roster.
    pub members: Vec<EtcdMemberSnapshot>,
    /// Status records that did not correlate to the current member roster.
    pub unmatched_statuses: Vec<EtcdMemberStatus>,
    /// Alarms that did not correlate to the current member roster.
    pub unmatched_alarms: Vec<EtcdAlarm>,
    /// Quorum derived from responding voting members only.
    pub quorum: QuorumState,
    /// Number of members eligible to vote in quorum.
    pub voting_members: usize,
    /// Number of voting members which returned a status record.
    pub responding_voting_members: usize,
    /// Unique non-zero leader IDs reported by current member statuses.
    pub reported_leader_ids: Vec<u64>,
    /// Largest reported member database size in bytes.
    pub largest_database_size: i64,
    /// Largest reported raft index.
    pub revision: u64,
    /// Optional etcd sources unavailable during collection.
    pub unavailable: Vec<InspectionUnavailable>,
}

impl EtcdHealthSnapshot {
    /// Whether status or alarms were unavailable while the member roster loaded.
    pub fn is_partial(&self) -> bool {
        !self.unavailable.is_empty()
    }

    /// The sole reported leader ID, if every status agrees on one leader.
    pub fn leader_id(&self) -> Option<u64> {
        match self.reported_leader_ids.as_slice() {
            [leader] => Some(*leader),
            _ => None,
        }
    }
}

/// Collects authoritative etcd membership followed by optional statuses and
/// alarms. A member-list failure is fatal because no safe correlation basis
/// remains; status and alarm failures are preserved as partial data.
pub async fn collect_etcd_health(
    client: TalosClient,
    request: EtcdInspectionRequest,
) -> Result<EtcdHealthSnapshot, InspectionError> {
    let timeout = request.timeout;
    let member_result = tokio::time::timeout(timeout, client.etcd_members()).await;
    let members = match member_result {
        Ok(Ok(members)) => members,
        Ok(Err(error)) => {
            return Err(required_from_error(
                InspectionSource::EtcdMembers,
                &request.target,
                error,
            ));
        }
        Err(_) => {
            return Err(required_from_timeout(
                InspectionSource::EtcdMembers,
                &request.target,
                timeout,
            ));
        }
    };

    let status_targets: Vec<String> = members
        .iter()
        .map(|member| {
            member
                .ip_address()
                .unwrap_or_else(|| member.hostname.clone())
        })
        .collect();
    let (status_result, alarm_result) = tokio::join!(
        tokio::time::timeout(timeout, client.etcd_status_for_nodes(&status_targets)),
        tokio::time::timeout(timeout, client.etcd_alarms()),
    );

    let mut unavailable = Vec::new();
    let statuses = match status_result {
        Ok(Ok(statuses)) => statuses,
        Ok(Err(error)) => {
            unavailable.push(unavailable_from_error(InspectionSource::EtcdStatus, error));
            Vec::new()
        }
        Err(_) => {
            unavailable.push(unavailable_from_timeout(
                InspectionSource::EtcdStatus,
                timeout,
            ));
            Vec::new()
        }
    };
    let alarms = match alarm_result {
        Ok(Ok(alarms)) => alarms,
        Ok(Err(error)) => {
            unavailable.push(unavailable_from_error(InspectionSource::EtcdAlarms, error));
            Vec::new()
        }
        Err(_) => {
            unavailable.push(unavailable_from_timeout(
                InspectionSource::EtcdAlarms,
                timeout,
            ));
            Vec::new()
        }
    };

    Ok(assemble_etcd_health(
        request.target,
        members,
        statuses,
        alarms,
        unavailable,
    ))
}

/// Correlates authoritative etcd members with independently collected statuses
/// and alarms. This transform performs no I/O and can be used to render a
/// cached sample or unit-test quorum assembly.
pub fn assemble_etcd_health(
    target: InspectionTarget,
    members: Vec<EtcdMemberInfo>,
    statuses: Vec<EtcdMemberStatus>,
    alarms: Vec<EtcdAlarm>,
    unavailable: Vec<InspectionUnavailable>,
) -> EtcdHealthSnapshot {
    let mut statuses_by_member: BTreeMap<u64, Vec<EtcdMemberStatus>> = BTreeMap::new();
    for status in statuses {
        statuses_by_member
            .entry(status.member_id)
            .or_default()
            .push(status);
    }
    let mut alarms_by_member: BTreeMap<u64, Vec<EtcdAlarm>> = BTreeMap::new();
    for alarm in alarms {
        alarms_by_member
            .entry(alarm.member_id)
            .or_default()
            .push(alarm);
    }

    let member_ids: HashSet<u64> = members.iter().map(|member| member.id).collect();
    let mut unmatched_statuses = Vec::new();
    for (member_id, statuses) in &statuses_by_member {
        if !member_ids.contains(member_id) {
            unmatched_statuses.extend(statuses.iter().cloned());
        }
    }
    let mut unmatched_alarms = Vec::new();
    for (member_id, alarms) in &alarms_by_member {
        if !member_ids.contains(member_id) {
            unmatched_alarms.extend(alarms.iter().cloned());
        }
    }

    let snapshots: Vec<EtcdMemberSnapshot> = members
        .into_iter()
        .map(|info| {
            let status = statuses_by_member
                .remove(&info.id)
                .and_then(|mut statuses| statuses.drain(..).next());
            let alarms = alarms_by_member.remove(&info.id).unwrap_or_default();
            EtcdMemberSnapshot {
                info,
                status,
                alarms,
            }
        })
        .collect();
    let voting_members = snapshots
        .iter()
        .filter(|member| !member.info.is_learner)
        .count();
    let responding_voting_members = snapshots
        .iter()
        .filter(|member| !member.info.is_learner && member.status.is_some())
        .count();
    let reported_leader_ids: Vec<u64> = snapshots
        .iter()
        .filter_map(|member| member.status.as_ref().map(|status| status.leader_id))
        .filter(|leader_id| *leader_id != 0)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let largest_database_size = snapshots
        .iter()
        .filter_map(|member| member.status.as_ref())
        .map(|status| status.db_size)
        .max()
        .unwrap_or_default();
    let revision = snapshots
        .iter()
        .filter_map(|member| member.status.as_ref())
        .map(|status| status.raft_index)
        .max()
        .unwrap_or_default();

    EtcdHealthSnapshot {
        target,
        members: snapshots,
        unmatched_statuses,
        unmatched_alarms,
        quorum: QuorumState::from_counts(responding_voting_members, voting_members),
        voting_members,
        responding_voting_members,
        reported_leader_ids,
        largest_database_size,
        revision,
        unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: &str) -> KubeSpanPeerStatus {
        KubeSpanPeerStatus {
            id: id.into(),
            label: id.into(),
            endpoint: None,
            state: "up".into(),
            rtt_ms: None,
            last_handshake: None,
            rx_bytes: 0,
            tx_bytes: 0,
        }
    }

    fn failure() -> talos_rs::TalosError {
        talos_rs::TalosError::Connection("boom".into())
    }

    #[test]
    fn kubespan_unknown_is_never_disabled() {
        assert!(matches!(
            kubespan_snapshot_from(Err(failure()), Ok(vec![])),
            KubeSpanSnapshot::Unavailable { .. }
        ));
        assert!(matches!(
            kubespan_snapshot_from(Ok(false), Err(failure())),
            KubeSpanSnapshot::Unavailable { .. }
        ));
        assert!(matches!(
            kubespan_snapshot_from(Ok(false), Ok(vec![])),
            KubeSpanSnapshot::Disabled
        ));
        assert!(matches!(
            kubespan_snapshot_from(Ok(true), Ok(vec![])),
            KubeSpanSnapshot::Enabled { .. }
        ));
        assert!(matches!(
            kubespan_snapshot_from(Err(failure()), Ok(vec![peer("a")])),
            KubeSpanSnapshot::Enabled { .. }
        ));
    }

    #[tokio::test]
    async fn invalid_talosctl_target_is_unavailable_without_running_talosctl() {
        for (address, context, path) in [
            ("10.0.0.1:50000", "", "/tmp/tc"),
            ("10.0.0.1", "--ctx", "/tmp/tc"),
            ("10.0.0.1", "ctx", " "),
            ("-n", "ctx", "/tmp/tc"),
        ] {
            let snapshot = collect_kubespan_for_target(
                address,
                &TalosctlTarget::new(context, path),
                Duration::from_secs(1),
            )
            .await;
            assert!(matches!(snapshot, KubeSpanSnapshot::Unavailable { .. }));
        }
    }

    #[test]
    fn talosctl_host_strips_port_only() {
        assert_eq!(talosctl_host("10.0.0.1:50000"), "10.0.0.1");
        assert_eq!(talosctl_host("10.0.0.1"), "10.0.0.1");
        assert_eq!(talosctl_host("fd00::1"), "fd00::1");
        assert_eq!(talosctl_host("[fd00::1]:50000"), "fd00::1");
        assert_eq!(talosctl_host(" 10.0.0.1:50000 "), "10.0.0.1");
    }

    fn process(pid: i32, ppid: i32, command: &str) -> ProcessSnapshotEntry {
        ProcessSnapshotEntry {
            process: ProcessInfo {
                pid,
                ppid,
                state: ProcessState::Running,
                threads: 1,
                cpu_time: 0.0,
                virtual_memory: 0,
                resident_memory: 0,
                command: command.to_string(),
                executable: String::new(),
                args: String::new(),
            },
            cpu_percent: None,
        }
    }

    #[test]
    fn subtree_rows_preserve_hierarchy_without_filtered_parent() {
        let processes = vec![
            process(1, 0, "init"),
            process(2, 1, "worker"),
            process(3, 2, "child"),
            process(4, 1, "other"),
        ];
        let rows = build_process_display_rows(
            &processes,
            &ProcessView {
                tree: ProcessTree::Subtree { root_pid: 2 },
                ..ProcessView::default()
            },
        );

        assert_eq!(
            rows.iter()
                .map(|row| (processes[row.process_index].process.pid, row.depth))
                .collect::<Vec<_>>(),
            vec![(2, 0), (3, 1)]
        );
    }

    #[test]
    fn interface_rates_use_only_compatible_prior_counters() {
        let target = InspectionTarget::new("cp-1", "10.0.0.1");
        let sampled_at = Instant::now();
        let previous = NetworkSampleState {
            target: Some(target.clone()),
            sampled_at: Some(sampled_at),
            devices: HashMap::from([(
                "eth0".to_string(),
                NetDevStats {
                    name: "eth0".to_string(),
                    rx_bytes: 100,
                    rx_packets: 0,
                    rx_errors: 2,
                    rx_dropped: 0,
                    tx_bytes: 50,
                    tx_packets: 0,
                    tx_errors: 0,
                    tx_dropped: 3,
                },
            )]),
        };
        let later = sampled_at + Duration::from_secs(2);
        let (interfaces, totals, _) = assemble_network_interfaces(
            vec![NetDevStats {
                name: "eth0".to_string(),
                rx_bytes: 300,
                rx_packets: 0,
                rx_errors: 2,
                rx_dropped: 0,
                tx_bytes: 150,
                tx_packets: 0,
                tx_errors: 0,
                tx_dropped: 3,
            }],
            Some(&previous),
            target,
            later,
        );

        let rate = interfaces[0].rate.as_ref().expect("compatible sample rate");
        assert_eq!(rate.rx_bytes_per_sec, 100);
        assert_eq!(rate.tx_bytes_per_sec, 50);
        assert_eq!(totals.rx_bytes_per_sec, 100);
        assert_eq!(totals.tx_bytes_per_sec, 50);
    }

    #[test]
    fn a_roster_without_a_leader_has_no_leader_id() {
        let member = |id: u64| talos_rs::EtcdMemberInfo {
            id,
            hostname: format!("cp-{id}"),
            peer_urls: Vec::new(),
            client_urls: Vec::new(),
            is_learner: false,
        };
        let status = |id: u64, leader_id: u64| talos_rs::EtcdMemberStatus {
            node: format!("cp-{id}"),
            member_id: id,
            protocol_version: "3.6.0".into(),
            db_size: 0,
            db_size_in_use: 0,
            leader_id,
            raft_index: 1,
            raft_term: 1,
            raft_applied_index: 1,
            errors: Vec::new(),
            is_learner: false,
        };
        let snapshot = |statuses| {
            assemble_etcd_health(
                InspectionTarget::new("cp-1", "10.0.0.1"),
                vec![member(1), member(2)],
                statuses,
                Vec::new(),
                Vec::new(),
            )
        };
        // Every member answered and none follows a leader: a lost quorum.
        assert_eq!(snapshot(vec![status(1, 0), status(2, 0)]).leader_id(), None);
        assert_eq!(snapshot(vec![status(1, 2), status(2, 1)]).leader_id(), None);
        assert_eq!(
            snapshot(vec![status(1, 2), status(2, 2)]).leader_id(),
            Some(2)
        );
    }
}
