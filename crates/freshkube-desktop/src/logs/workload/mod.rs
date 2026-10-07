//! A workload's logs in the dock: every container of every pod a
//! Deployment, StatefulSet, DaemonSet, ReplicaSet or Job runs, in one view.
//! Lines from all streams are interleaved by their timestamps and tagged
//! `pod/container`. Read-only: it watches the workload's pods and reads
//! their logs, nothing else.
//!
//! Pods are found by the workload's selector (core's
//! `follow_pods`) and followed as they come and go; each container
//! is read by core's `follow_pod_log`, at most [`MAX_STREAMS`] at once. A
//! line that arrives late, from a stream behind the others, is placed by
//! its time among the retained lines, as with Talos services; a paused
//! review keeps its place and the selection keeps its lines.
//!
//! Every stream feeds one channel, which one delivery task drains a frame
//! at a time into a single `ingest`, as Talos services do, however many
//! containers write. A stream that fails is read again with the watcher's
//! backoff while its pod is listed.
//!
//! Nothing is read until the dock's tab for the workload first shows
//! (`desktop/dock/`). Then the watch and the streams live while the tab
//! stays open, whatever page shows, and stop when it closes or the
//! connection changes.
//!
//! This file holds the source's state and its streams; `controls` draws
//! the status, the streams and their notices.

mod controls;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use freshkube_core::logs::{LogEvent, ServiceId};
use freshkube_core::resources::{
    ContainerRole, Failure, FailureKind, LogPosition, LogRequest, PodLogUpdate, PodSelector,
    WorkloadPod, WorkloadPods, follow_pod_log, follow_pods,
};
use gpui_kit::{AnyElement, App, Context, Pixels, SharedString, Task, Window};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};

use super::{Columns, LogSource, LogView};
use crate::backend::{OwnedJob, STREAM_QUEUE_CAPACITY};
use crate::resources::model::ResourceIdentity;
use crate::resources::{KubeAccess, example};
use crate::ui::Tone;
use controls::Controls;

/// The detail pane's Logs tab for a workload.
pub(crate) type WorkloadLogView = LogView<WorkloadLogs>;

/// The most container logs read at once. The newest pods come first; the
/// rest are counted in a notice.
pub(crate) const MAX_STREAMS: usize = 20;
/// Lines from the end each container's log starts with.
const TAIL: i64 = 100;
/// How often an example container writes another line; each stream adds
/// a tick or more, so they don't all write at once.
const EXAMPLE_INTERVAL: Duration = Duration::from_millis(1_500);
/// The example writer's tick.
const EXAMPLE_TICK: Duration = Duration::from_millis(250);
/// How often the delivery task hands what the streams sent to the view.
const DELIVERY_INTERVAL: Duration = Duration::from_millis(16);
/// A failed stream's first retry, doubling up to the last, as the pod
/// watch backs off.
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);

/// One container's log: which pod incarnation and container.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct StreamKey {
    pub(super) pod: String,
    pub(super) uid: String,
    pub(super) container: String,
}

impl StreamKey {
    /// The tag its lines carry, by which they are filtered.
    pub(super) fn service(&self) -> ServiceId {
        ServiceId::new(format!("{}/{}", self.pod, self.container))
    }
}

/// Where one container's log stands.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum StreamState {
    Connecting,
    /// The container hasn't started, with the kubelet's reason.
    Waiting(String),
    Streaming,
    Reconnecting(u32),
    /// The container exited for good.
    Ended,
    Failed(Failure),
}

impl StreamState {
    pub(super) fn tone(&self) -> Tone {
        match self {
            StreamState::Streaming => Tone::Good,
            StreamState::Waiting(_) | StreamState::Reconnecting(_) => Tone::Warn,
            StreamState::Failed(_) => Tone::Crit,
            StreamState::Connecting | StreamState::Ended => Tone::Unknown,
        }
    }

    pub(super) fn label(&self) -> String {
        match self {
            StreamState::Connecting => "Connecting".into(),
            StreamState::Waiting(reason) if reason.is_empty() => "Waiting".into(),
            StreamState::Waiting(reason) => format!("Waiting ({reason})"),
            StreamState::Streaming => "Streaming".into(),
            StreamState::Reconnecting(attempt) => format!("Reconnecting (attempt {attempt})"),
            StreamState::Ended => "Ended".into(),
            StreamState::Failed(_) => "Failed".into(),
        }
    }
}

struct Stream {
    state: StreamState,
    /// The pod, for a retry and the example data's log.
    pod: ResourceIdentity,
    /// Advances with each read; updates from an older one are dropped.
    generation: u64,
    /// What this read asked for, `resume` included.
    request: LogRequest,
    job: Option<OwnedJob>,
    /// The next read after a failure, once its backoff passes.
    retry: Option<Task<()>>,
    /// An example container that goes on writing, and its next line.
    writes_on: bool,
    sequence: u64,
}

/// One update from one container's read, as the shared channel carries it.
pub(super) struct Fed {
    key: StreamKey,
    generation: u64,
    update: PodLogUpdate,
}

/// The channel every stream sends into, and the one task that drains it.
struct Feed {
    sender: mpsc::Sender<Fed>,
    _delivery: Task<()>,
}

/// Wall time by the executor's clock: anchored once, then stepped by it,
/// so tests advance it with the clock instead of sleeping.
#[derive(Clone, Copy)]
struct Clock {
    started: Instant,
    at: DateTime<Utc>,
}

impl Clock {
    fn new(cx: &App) -> Self {
        Self {
            started: cx.background_executor().now(),
            at: Utc::now(),
        }
    }

    fn now(&self, cx: &App) -> DateTime<Utc> {
        let elapsed = cx
            .background_executor()
            .now()
            .saturating_duration_since(self.started);
        self.at + TimeDelta::from_std(elapsed).unwrap_or_default()
    }
}

/// Where the pod watch stands.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum PodsState {
    /// Nothing asked for yet, or the page is hidden.
    Idle,
    /// The workload's document hasn't been read, or its pods not listed.
    Finding,
    Watching,
    /// The workload selects no pods.
    NoSelector,
    Failed(Failure),
}

/// One entry of the streams row, derived when the streams change.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Chip {
    pub(super) service: ServiceId,
    pub(super) id: SharedString,
    /// Its id in the list of every container, behind "+N".
    pub(super) list_id: SharedString,
    pub(super) label: SharedString,
    /// The full tag, its state and what a click does.
    pub(super) tooltip: SharedString,
    pub(super) tone: Tone,
    pub(super) shown: bool,
}

/// What the controls say, derived when anything changes.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Status {
    pub(super) tone: Tone,
    pub(super) tag: SharedString,
    pub(super) text: SharedString,
    /// Both, for assistive technology and tests.
    pub(super) label: SharedString,
}

pub(crate) struct WorkloadLogs {
    runtime: Handle,
    clock: Clock,
    access: Option<KubeAccess>,
    workload: Option<ResourceIdentity>,
    /// The selector from the workload's document: `None` until it is read,
    /// `Some(None)` when it selects nothing.
    selector: Option<Option<String>>,
    /// The Logs tab was shown for this workload, so its pods are read.
    wanted: bool,
    active: bool,
    pub(super) pods_state: PodsState,
    /// The pods as last seen, newest first.
    pods: Vec<WorkloadPod>,
    /// Pods seen so far, by name and uid, to tell which joined and left.
    /// The first listing of a workload marks none.
    known: Option<BTreeSet<(String, String)>>,
    /// Advances whenever the watch is dropped; its deliveries carry it.
    epoch: u64,
    watch_job: Option<OwnedJob>,
    watch_delivery: Option<Task<()>>,
    streams: BTreeMap<StreamKey, Stream>,
    feed: Option<Feed>,
    /// Writes the example containers' lines while any stream writes on.
    example_writer: Option<Task<()>>,
    /// A `stress` run's flood into the feed.
    #[cfg(feature = "stress")]
    stress: Option<OwnedJob>,
    /// Failures in a row of each container, for its retry's backoff.
    failures: BTreeMap<StreamKey, u32>,
    /// How far each container was read, kept while the page hides so its
    /// log reads on without repeating a line.
    positions: BTreeMap<StreamKey, LogPosition>,
    next_generation: u64,
    /// Containers not read because of [`MAX_STREAMS`].
    pub(super) left_out: usize,
    /// Tags the user hid; tags seen in this review, hidden or not.
    hidden: BTreeSet<ServiceId>,
    seen: BTreeSet<ServiceId>,
    pub(super) chips: Rc<Vec<Chip>>,
    /// Advances whenever the chips are derived again, so their measured
    /// widths are taken again.
    chips_revision: u64,
    /// The chips' widths, and the "+N" chip's, for the revision and text
    /// size they were measured at.
    chip_widths: Vec<Pixels>,
    more_width: Pixels,
    measured: Option<(u64, Pixels)>,
    /// How many chips the rows have room for; "+N" lists the rest.
    pub(super) fits: usize,
    /// Whether the list of every container is open.
    pub(super) more_open: bool,
    /// What the source column and the chips show for each tag: the pod
    /// name without the prefix every pod shares, derived with the chips.
    labels: BTreeMap<ServiceId, SharedString>,
    pub(super) status: Status,
    empty: SharedString,
    /// Failed streams, shown above the lines while the others go on.
    errors: BTreeMap<ServiceId, String>,
}

impl WorkloadLogs {
    fn new(runtime: Handle, clock: Clock) -> Self {
        let mut source = Self {
            runtime,
            clock,
            access: None,
            workload: None,
            selector: None,
            wanted: false,
            active: false,
            pods_state: PodsState::Idle,
            pods: Vec::new(),
            known: None,
            epoch: 0,
            watch_job: None,
            watch_delivery: None,
            streams: BTreeMap::new(),
            feed: None,
            example_writer: None,
            #[cfg(feature = "stress")]
            stress: None,
            failures: BTreeMap::new(),
            positions: BTreeMap::new(),
            next_generation: 0,
            left_out: 0,
            hidden: BTreeSet::new(),
            seen: BTreeSet::new(),
            chips: Rc::default(),
            chips_revision: 0,
            chip_widths: Vec::new(),
            more_width: Pixels::ZERO,
            measured: None,
            fits: usize::MAX,
            more_open: false,
            labels: BTreeMap::new(),
            status: Status {
                tone: Tone::Unknown,
                tag: SharedString::default(),
                text: SharedString::default(),
                label: SharedString::default(),
            },
            empty: SharedString::default(),
            errors: BTreeMap::new(),
        };
        source.describe();
        source
    }

    /// Whether the watch is open or about to be.
    pub(super) fn watching(&self) -> bool {
        self.watch_job.is_some() || self.watch_delivery.is_some()
    }

    /// Drops the watch and every stream, keeping how far each was read.
    fn drop_all(&mut self) {
        self.epoch += 1;
        self.watch_job = None;
        self.watch_delivery = None;
        self.streams.clear();
        self.feed = None;
        self.example_writer = None;
        #[cfg(feature = "stress")]
        {
            self.stress = None;
        }
    }

    /// The containers to read: app containers of the newest pods first, at
    /// most [`MAX_STREAMS`], and how many are left out.
    fn wanted_streams(&self) -> (Vec<(StreamKey, &WorkloadPod)>, usize) {
        let all: Vec<_> = self
            .pods
            .iter()
            .flat_map(|pod| {
                pod.containers
                    .containers
                    .iter()
                    .filter(|container| container.role == ContainerRole::App)
                    .map(move |container| {
                        (
                            StreamKey {
                                pod: pod.name.clone(),
                                uid: pod.uid.clone(),
                                container: container.name.clone(),
                            },
                            pod,
                        )
                    })
            })
            .collect();
        let left_out = all.len().saturating_sub(MAX_STREAMS);
        (all.into_iter().take(MAX_STREAMS).collect(), left_out)
    }

    /// Derives the streams row, the status and the empty message.
    fn describe(&mut self) {
        let services = self
            .seen
            .iter()
            .cloned()
            .chain(self.streams.keys().map(StreamKey::service));
        self.labels = short_labels(services);
        let chips: Vec<Chip> = self
            .streams
            .iter()
            .map(|(key, stream)| {
                let service = key.service();
                let name = service.as_str();
                let shown = !self.hidden.contains(&service);
                let retrying = if stream.retry.is_some() {
                    ", reading again soon"
                } else {
                    ""
                };
                Chip {
                    id: format!("workload-logs-stream-{name}").into(),
                    list_id: format!("workload-logs-list-{name}").into(),
                    label: self
                        .labels
                        .get(&service)
                        .cloned()
                        .unwrap_or_else(|| name.to_owned().into()),
                    tooltip: format!(
                        "{name}: {}{retrying}. {} its lines.",
                        stream.state.label(),
                        if shown { "Hide" } else { "Show" }
                    )
                    .into(),
                    tone: stream.state.tone(),
                    shown,
                    service,
                }
            })
            .collect();
        // Another set of containers closes the list of them, so it never
        // opens again by itself on a new set.
        let same_set = chips
            .iter()
            .map(|chip| &chip.service)
            .eq(self.chips.iter().map(|chip| &chip.service));
        if !same_set {
            self.more_open = false;
        }
        self.chips = Rc::new(chips);
        self.chips_revision += 1;
        let pods = self.pods.len();
        let streams = self.streams.len();
        let counts = format!(
            "{} · {}",
            plural(pods, "pod", "pods"),
            plural(streams, "container", "containers")
        );
        let (tone, tag, text, empty) = match &self.pods_state {
            PodsState::Idle => (Tone::Unknown, "Idle", String::new(), String::new()),
            PodsState::Finding => (
                Tone::Unknown,
                "Finding pods",
                String::new(),
                "Finding the workload's pods".to_owned(),
            ),
            PodsState::NoSelector => (
                Tone::Unknown,
                "No selector",
                String::new(),
                "This workload selects no pods.".to_owned(),
            ),
            PodsState::Watching if pods == 0 => (
                Tone::Unknown,
                "No pods",
                "Pods show here as they start.".to_owned(),
                "The workload runs no pods.".to_owned(),
            ),
            PodsState::Watching => (
                if self.errors.is_empty() {
                    Tone::Good
                } else {
                    Tone::Warn
                },
                "Following",
                counts,
                "Waiting for the first line".to_owned(),
            ),
            PodsState::Failed(failure) => (
                Tone::Crit,
                "Failed",
                format!("Watching the pods failed: {failure}"),
                "Nothing was read.".to_owned(),
            ),
        };
        let label = if text.is_empty() {
            tag.to_owned()
        } else {
            format!("{tag}: {text}")
        };
        self.status = Status {
            tone,
            tag: tag.into(),
            text: text.into(),
            label: label.into(),
        };
        self.empty = empty.into();
    }

    /// A note between lines: a pod joined or left.
    fn marker(&mut self, pod: &str, text: String, at: DateTime<Utc>) -> LogEvent {
        let service = ServiceId::new(pod);
        self.seen.insert(service.clone());
        LogEvent::marker(service, at.fixed_offset(), text)
    }

    /// The tags shown: every tag seen but those the user hid.
    fn shown(&self) -> BTreeSet<ServiceId> {
        self.seen.difference(&self.hidden).cloned().collect()
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

impl LogSource for WorkloadLogs {
    fn prepare_controls(
        view: &mut WorkloadLogView,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<WorkloadLogView>,
    ) {
        view.fit_chips(width, window, cx);
    }

    fn notes(view: &WorkloadLogView, cx: &mut Context<WorkloadLogView>) -> Vec<AnyElement> {
        view.render_controls(cx)
    }

    fn empty_message(view: &WorkloadLogView) -> SharedString {
        if !view.has_lines() {
            view.source().empty.clone()
        } else {
            "No retained lines pass the filters.".into()
        }
    }

    fn panel_label(_view: &WorkloadLogView) -> SharedString {
        "Workload logs panel".into()
    }

    fn errors(&self) -> &BTreeMap<ServiceId, String> {
        &self.errors
    }

    fn source_label(&self, service: &ServiceId) -> Option<SharedString> {
        self.labels.get(service).cloned()
    }
}

/// How many chips fit in `rows` rows of `room`, leaving the last row room
/// for the "+N" chip when some don't. Every chip fits when they all do.
fn chips_that_fit(
    widths: &[Pixels],
    more: Pixels,
    gap: Pixels,
    room: Pixels,
    rows: usize,
) -> usize {
    // Where each chip lands, packed left to right and row by row: its row
    // and where it ends.
    let mut placed: Vec<(usize, Pixels)> = Vec::with_capacity(widths.len());
    for &width in widths {
        let (row, end) = match placed.last() {
            None => (0, width),
            Some(&(row, end)) if end + gap + width <= room => (row, end + gap + width),
            Some(&(row, _)) => (row + 1, width),
        };
        if row >= rows {
            break;
        }
        placed.push((row, end));
    }
    if placed.len() == widths.len() {
        return widths.len();
    }
    // Some don't fit: take chips off the end until "+N" fits after the
    // last one, or alone at the start of its row.
    while let Some(&(row, end)) = placed.last() {
        if end + gap + more <= room {
            break;
        }
        placed.pop();
        if placed.last().is_none_or(|&(previous, _)| previous < row) {
            break;
        }
    }
    placed.len()
}

/// Short labels for `pod/container` tags: each pod name without the prefix
/// all of them share, up to a `-` (a ReplicaSet's pods keep their random
/// suffix, a StatefulSet's their ordinal), and the container only when
/// the tags name more than one. A pod's own tag, which its markers carry,
/// gets the pod's label.
fn short_labels(services: impl Iterator<Item = ServiceId>) -> BTreeMap<ServiceId, SharedString> {
    let tags: BTreeSet<ServiceId> = services.collect();
    let mut names = tags.iter().map(|service| split(service).0);
    let Some(first) = names.next() else {
        return BTreeMap::new();
    };
    let shared = names.fold(first.len(), |shared, name| {
        first
            .bytes()
            .zip(name.bytes())
            .take(shared)
            .take_while(|(a, b)| a == b)
            .count()
    });
    // Up to the last `-` that still leaves every name something.
    let shortest = tags
        .iter()
        .map(|service| split(service).0.len())
        .min()
        .unwrap_or(0);
    let cut = first[..shared.min(shortest.saturating_sub(1))]
        .rfind('-')
        .map_or(0, |dash| dash + 1);
    // A pod's own tag, for its joined and left markers, names no container.
    let containers: BTreeSet<&str> = tags
        .iter()
        .map(|service| split(service).1)
        .filter(|container| !container.is_empty())
        .collect();
    tags.iter()
        .map(|service| {
            let (pod, container) = split(service);
            let pod = &pod[cut..];
            let label = if containers.len() > 1 && !container.is_empty() {
                format!("{pod}/{container}")
            } else {
                pod.to_owned()
            };
            (service.clone(), label.into())
        })
        .collect()
}

/// A tag's pod and container.
fn split(service: &ServiceId) -> (&str, &str) {
    let tag = service.as_str();
    tag.rsplit_once('/').unwrap_or((tag, ""))
}

/// What the detail pane and the tests ask of a workload's Logs tab.
pub(crate) trait WorkloadLogPanel: Sized + 'static {
    fn for_workloads(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self;

    /// Fresh handles for the same connection.
    fn set_access(&mut self, access: KubeAccess);

    /// Shows the logs of `workload`, or of none. Another workload starts
    /// over: nothing is read until it is wanted and its selector is known.
    fn show_workload(
        &mut self,
        workload: Option<ResourceIdentity>,
        access: Option<KubeAccess>,
        cx: &mut Context<Self>,
    );

    /// The selector from the workload's document as last read.
    fn set_selector(&mut self, selector: Option<String>, cx: &mut Context<Self>);

    /// The Logs tab shows: read the pods' logs while this workload stays.
    fn want(&mut self, cx: &mut Context<Self>);

    /// Hiding the page stops the watch and every stream; showing it again
    /// reads each container on from its last line.
    fn set_active(&mut self, active: bool, cx: &mut Context<Self>);

    /// Whether the pods are watched, for the pane's tests.
    #[cfg(test)]
    fn reading(&self) -> bool;
}

impl WorkloadLogPanel for WorkloadLogView {
    fn for_workloads(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let clock = Clock::new(cx);
        Self::with_source(WorkloadLogs::new(runtime, clock), window, cx).with_columns(Columns {
            time: true,
            source: true,
        })
    }

    fn set_access(&mut self, access: KubeAccess) {
        self.source_mut().access = Some(access);
    }

    fn show_workload(
        &mut self,
        workload: Option<ResourceIdentity>,
        access: Option<KubeAccess>,
        cx: &mut Context<Self>,
    ) {
        if access.is_some() {
            self.source_mut().access = access;
        }
        if self.source().workload == workload {
            return;
        }
        self.source_mut().drop_all();
        self.reset_lines(
            &workload
                .as_ref()
                .map(ResourceIdentity::address)
                .unwrap_or_default(),
        );
        let source = self.source_mut();
        source.workload = workload;
        source.selector = None;
        source.wanted = false;
        source.pods_state = PodsState::Idle;
        source.pods.clear();
        source.known = None;
        source.positions.clear();
        source.failures.clear();
        source.left_out = 0;
        source.hidden.clear();
        source.seen.clear();
        source.errors.clear();
        source.more_open = false;
        source.describe();
        self.clear_shown();
        cx.notify();
    }

    fn set_selector(&mut self, selector: Option<String>, cx: &mut Context<Self>) {
        if self.source().workload.is_none() || self.source().selector.as_ref() == Some(&selector) {
            return;
        }
        let changed = self.source().selector.is_some();
        self.source_mut().selector = Some(selector);
        if changed {
            // Another selector picks other pods: read them afresh.
            self.source_mut().drop_all();
        }
        self.start_watch(cx);
    }

    fn want(&mut self, cx: &mut Context<Self>) {
        if self.source().wanted || self.source().workload.is_none() {
            return;
        }
        self.source_mut().wanted = true;
        self.start_watch(cx);
    }

    fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.source().active == active {
            return;
        }
        self.source_mut().active = active;
        if active {
            self.start_watch(cx);
        } else {
            self.flush_backlog(cx);
            let source = self.source_mut();
            source.drop_all();
            if source.pods_state != PodsState::NoSelector {
                source.pods_state = PodsState::Idle;
            }
            source.describe();
            cx.notify();
        }
    }

    #[cfg(test)]
    fn reading(&self) -> bool {
        self.source().watching()
    }
}

/// The watch and the streams, used only by this tab's own controls and
/// tests.
trait Streams: Sized + 'static {
    /// Shows every tag seen but those the user hid.
    fn clear_shown(&mut self);

    /// Shows or hides one stream's lines.
    fn toggle_stream(&mut self, service: ServiceId, cx: &mut Context<Self>);

    /// Starts the pod watch once the tab is wanted, the page shows and the
    /// selector is known.
    fn start_watch(&mut self, cx: &mut Context<Self>);

    /// Watches the pods again after a failure.
    fn retry(&mut self, cx: &mut Context<Self>);

    /// Takes the pods as last seen by the watch of `epoch`. Returns false
    /// when that watch is gone, which ends its delivery.
    fn apply_pods(&mut self, epoch: u64, pods: WorkloadPods, cx: &mut Context<Self>) -> bool;

    /// Starts reading one container, on from where it was left.
    fn start_stream(&mut self, key: StreamKey, pod: ResourceIdentity, cx: &mut Context<Self>);

    /// Applies updates from one container's read of `generation`. Returns
    /// false when that read is gone.
    fn apply_updates(
        &mut self,
        key: &StreamKey,
        generation: u64,
        updates: Vec<PodLogUpdate>,
        cx: &mut Context<Self>,
    ) -> bool;

    /// The channel every stream sends into, opened with its delivery task
    /// on first use.
    fn feed(&mut self, cx: &mut Context<Self>) -> mpsc::Sender<Fed>;

    /// Applies what every stream sent since the last delivery: one ingest,
    /// one notify, and the streams row derived again only when a stream's
    /// state changed.
    fn apply_fed(&mut self, batch: Vec<Fed>, cx: &mut Context<Self>);

    /// Reads a failed container again once its backoff passes.
    fn schedule_retry(&mut self, key: StreamKey, generation: u64, cx: &mut Context<Self>);

    /// The retry itself, while the stream still failed and its pod is listed.
    fn retry_stream(&mut self, key: StreamKey, generation: u64, cx: &mut Context<Self>);

    /// Example data reads at once; a running example container writes on.
    fn start_example(&mut self, key: StreamKey, generation: u64, cx: &mut Context<Self>);

    /// One tick of the example writer: each running example container
    /// writes at its own pace, into the feed.
    fn example_tick(&mut self, tick: u64, cx: &mut Context<Self>);
}

impl Streams for WorkloadLogView {
    fn clear_shown(&mut self) {
        let shown = self.source().shown();
        if *self.shown() != shown {
            self.set_shown(shown);
        }
    }

    fn toggle_stream(&mut self, service: ServiceId, cx: &mut Context<Self>) {
        let source = self.source_mut();
        if !source.hidden.remove(&service) {
            source.hidden.insert(service);
        }
        source.describe();
        self.capture_anchor();
        self.clear_shown();
        cx.notify();
    }

    fn start_watch(&mut self, cx: &mut Context<Self>) {
        if !self.source().wanted || !self.source().active || self.source().watching() {
            return;
        }
        let Some(selector) = self.source().selector.clone() else {
            let source = self.source_mut();
            source.pods_state = PodsState::Finding;
            source.describe();
            cx.notify();
            return;
        };
        let Some(selector) = selector else {
            let source = self.source_mut();
            source.pods_state = PodsState::NoSelector;
            source.describe();
            cx.notify();
            return;
        };
        let (Some(workload), Some(access)) =
            (self.source().workload.clone(), self.source().access.clone())
        else {
            return;
        };
        let source = self.source_mut();
        source.drop_all();
        source.pods_state = PodsState::Finding;
        source.describe();
        cx.notify();
        let epoch = self.source().epoch;
        if let KubeAccess::Example = access {
            // Example pods don't come and go; the watch counts as running.
            self.source_mut().watch_delivery = Some(Task::ready(()));
            let now = self.source().clock.now(cx).timestamp();
            let pods = example::workload_pods(&workload, &selector, now);
            self.apply_pods(
                epoch,
                WorkloadPods {
                    listed: true,
                    pods,
                    failure: None,
                },
                cx,
            );
            #[cfg(feature = "stress")]
            stress_flood(self, cx);
            return;
        }
        let (sender, mut receiver) = watch::channel(WorkloadPods::default());
        let namespace = workload.namespace.clone();
        let job = self.source().runtime.spawn(async move {
            match access.client().await {
                Ok(client) => {
                    follow_pods(client, namespace, PodSelector::Labels(selector), sender).await
                }
                Err(error) => {
                    access.forget();
                    sender.send_modify(|pods| {
                        pods.failure = Some(Failure::new(FailureKind::Other, error))
                    });
                    // Keep the sender until the receiver has seen it.
                    sender.closed().await;
                }
            }
        });
        let source = self.source_mut();
        source.watch_job = Some(OwnedJob::new(job));
        source.watch_delivery = Some(cx.spawn(async move |weak, cx| {
            while receiver.changed().await.is_ok() {
                let pods = receiver.borrow_and_update().clone();
                let applied = weak
                    .update(cx, |view, cx| view.apply_pods(epoch, pods, cx))
                    .unwrap_or(false);
                if !applied {
                    return;
                }
            }
        }));
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        if matches!(self.source().pods_state, PodsState::Failed(_)) {
            self.source_mut().drop_all();
            self.start_watch(cx);
        }
    }

    fn apply_pods(
        &mut self,
        epoch: u64,
        mut snapshot: WorkloadPods,
        cx: &mut Context<Self>,
    ) -> bool {
        if epoch != self.source().epoch {
            return false;
        }
        if let Some(failure) = snapshot.failure.take() {
            // The streams read on; a watch that can retry lists again by
            // itself, and its next listing says Following again.
            let source = self.source_mut();
            source.pods_state = PodsState::Failed(failure);
            source.describe();
            cx.notify();
            return true;
        }
        if !snapshot.listed {
            return true;
        }
        snapshot.pods.sort_by(|left, right| {
            right
                .created
                .cmp(&left.created)
                .then_with(|| left.name.cmp(&right.name))
        });
        let workload = self.source().workload.clone();
        let source = self.source_mut();
        source.pods = snapshot.pods;
        source.pods_state = PodsState::Watching;
        // Pods that joined and left since the last listing, by incarnation.
        let now: BTreeSet<(String, String)> = source
            .pods
            .iter()
            .map(|pod| (pod.name.clone(), pod.uid.clone()))
            .collect();
        let mut markers = Vec::new();
        if let Some(known) = source.known.take() {
            for (pod, _) in known.difference(&now) {
                markers.push((pod.clone(), format!("Pod {pod} left")));
            }
            for (pod, _) in now.difference(&known) {
                markers.push((pod.clone(), format!("Pod {pod} joined")));
            }
        }
        source.known = Some(now);
        let at = source.clock.now(cx);
        let markers: Vec<LogEvent> = markers
            .into_iter()
            .map(|(pod, text)| source.marker(&pod, text, at))
            .collect();
        let (wanted, left_out) = source.wanted_streams();
        let wanted: Vec<(StreamKey, ResourceIdentity)> = wanted
            .into_iter()
            .map(|(key, pod)| {
                let identity = ResourceIdentity {
                    connection: workload
                        .as_ref()
                        .map(|workload| workload.connection.clone())
                        .unwrap_or_default(),
                    resource: "pods".into(),
                    namespace: workload
                        .as_ref()
                        .map(|workload| workload.namespace.clone())
                        .unwrap_or_default(),
                    name: pod.name.clone(),
                    uid: pod.uid.clone(),
                };
                (key, identity)
            })
            .collect();
        source.left_out = left_out;
        // Streams of pods that left, or past the cap, stop; what they wrote
        // stays.
        let keep: BTreeSet<&StreamKey> = wanted.iter().map(|(key, _)| key).collect();
        let gone: Vec<StreamKey> = source
            .streams
            .keys()
            .filter(|key| !keep.contains(key))
            .cloned()
            .collect();
        for key in gone {
            source.streams.remove(&key);
            source.errors.remove(&key.service());
        }
        let alive: BTreeSet<(String, String)> = source
            .pods
            .iter()
            .map(|pod| (pod.name.clone(), pod.uid.clone()))
            .collect();
        source
            .positions
            .retain(|key, _| alive.contains(&(key.pod.clone(), key.uid.clone())));
        source
            .failures
            .retain(|key, _| alive.contains(&(key.pod.clone(), key.uid.clone())));
        let new: Vec<_> = wanted
            .into_iter()
            .filter(|(key, _)| !source.streams.contains_key(key))
            .collect();
        if !markers.is_empty() {
            self.clear_shown();
            self.ingest(markers, cx);
        }
        for (key, pod) in new {
            self.start_stream(key, pod, cx);
        }
        self.clear_shown();
        self.source_mut().describe();
        cx.notify();
        true
    }

    fn start_stream(&mut self, key: StreamKey, pod: ResourceIdentity, cx: &mut Context<Self>) {
        let Some(access) = self.source().access.clone() else {
            return;
        };
        let resume = self
            .source()
            .positions
            .get(&key)
            .filter(|position| position.time().is_some())
            .copied();
        let request = LogRequest {
            namespace: pod.namespace.clone(),
            pod: key.pod.clone(),
            container: key.container.clone(),
            previous: false,
            tail: Some(TAIL),
            resume,
        };
        let source = self.source_mut();
        source.next_generation += 1;
        let generation = source.next_generation;
        source.seen.insert(key.service());
        source.streams.insert(
            key.clone(),
            Stream {
                state: StreamState::Connecting,
                pod,
                generation,
                request: request.clone(),
                job: None,
                retry: None,
                writes_on: false,
                sequence: generation,
            },
        );
        // Its lines show unless the user hid them.
        self.clear_shown();
        if let KubeAccess::Example = access {
            self.start_example(key, generation, cx);
            return;
        }
        let feed = self.feed(cx);
        let stream_key = key.clone();
        let job = self.source().runtime.spawn(async move {
            match access.client().await {
                Ok(client) => {
                    // The container's updates, tagged on their way into the
                    // shared channel.
                    let (sender, mut receiver) = mpsc::channel(STREAM_QUEUE_CAPACITY);
                    let forward = async {
                        while let Some(update) = receiver.recv().await {
                            let fed = Fed {
                                key: stream_key.clone(),
                                generation,
                                update,
                            };
                            if feed.send(fed).await.is_err() {
                                return;
                            }
                        }
                    };
                    tokio::join!(follow_pod_log(client, request, sender), forward);
                }
                Err(error) => {
                    access.forget();
                    let update = PodLogUpdate::Failed(Failure::new(FailureKind::Other, error));
                    let _ = feed
                        .send(Fed {
                            key: stream_key,
                            generation,
                            update,
                        })
                        .await;
                }
            }
        });
        if let Some(stream) = self.source_mut().streams.get_mut(&key) {
            stream.job = Some(OwnedJob::new(job));
        }
    }

    fn apply_updates(
        &mut self,
        key: &StreamKey,
        generation: u64,
        updates: Vec<PodLogUpdate>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .source()
            .streams
            .get(key)
            .is_none_or(|stream| stream.generation != generation)
        {
            return false;
        }
        let batch = updates
            .into_iter()
            .map(|update| Fed {
                key: key.clone(),
                generation,
                update,
            })
            .collect();
        self.apply_fed(batch, cx);
        true
    }

    fn feed(&mut self, cx: &mut Context<Self>) -> mpsc::Sender<Fed> {
        if let Some(feed) = &self.source().feed {
            return feed.sender.clone();
        }
        let (sender, mut receiver) = mpsc::channel::<Fed>(STREAM_QUEUE_CAPACITY);
        let delivery = cx.spawn(async move |weak, cx| {
            while let Some(first) = receiver.recv().await {
                let mut batch = vec![first];
                // At most a full queue per turn, then yield, however busy
                // the containers.
                while batch.len() < STREAM_QUEUE_CAPACITY {
                    let Ok(fed) = receiver.try_recv() else {
                        break;
                    };
                    batch.push(fed);
                }
                if weak
                    .update(cx, |view, cx| view.apply_fed(batch, cx))
                    .is_err()
                {
                    return;
                }
                cx.background_executor().timer(DELIVERY_INTERVAL).await;
            }
        });
        self.source_mut().feed = Some(Feed {
            sender: sender.clone(),
            _delivery: delivery,
        });
        sender
    }

    fn apply_fed(&mut self, batch: Vec<Fed>, cx: &mut Context<Self>) {
        crate::desktop::probe::hit("workload-logs.apply");
        let now = self.source().clock.now(cx);
        let mut lines = Vec::new();
        let mut changed = false;
        let mut failed = Vec::new();
        let source = self.source_mut();
        for Fed {
            key,
            generation,
            update,
        } in batch
        {
            let Some(stream) = source
                .streams
                .get_mut(&key)
                .filter(|stream| stream.generation == generation)
            else {
                continue;
            };
            let service = key.service();
            let position = source.positions.entry(key.clone()).or_default();
            match update {
                PodLogUpdate::Line(line) => {
                    position.record(&line);
                    lines.push(LogEvent::new(service, line));
                }
                PodLogUpdate::Streaming => {
                    stream.state = StreamState::Streaming;
                    source.errors.remove(&service);
                    source.failures.remove(&key);
                    changed = true;
                }
                PodLogUpdate::Waiting(reason) => {
                    stream.state = StreamState::Waiting(reason);
                    changed = true;
                }
                PodLogUpdate::Reconnecting { attempt, .. } => {
                    stream.state = StreamState::Reconnecting(attempt);
                    changed = true;
                }
                PodLogUpdate::Restarting(ended) => {
                    let how = ended.map(|ended| format!(" ({ended})")).unwrap_or_default();
                    let at = position.time().unwrap_or(now);
                    lines.push(LogEvent::marker(
                        service,
                        at.fixed_offset(),
                        format!("{} restarted{how}", key.container),
                    ));
                }
                PodLogUpdate::Ended(_) => {
                    stream.job = None;
                    stream.state = StreamState::Ended;
                    changed = true;
                }
                PodLogUpdate::Failed(failure) => {
                    stream.job = None;
                    source.errors.insert(service, failure.to_string());
                    stream.state = StreamState::Failed(failure);
                    failed.push((key, generation));
                    changed = true;
                }
            }
        }
        for (key, generation) in failed {
            self.schedule_retry(key, generation, cx);
        }
        if changed {
            self.source_mut().describe();
        }
        if !lines.is_empty() {
            self.ingest(lines, cx);
        }
        cx.notify();
    }

    fn schedule_retry(&mut self, key: StreamKey, generation: u64, cx: &mut Context<Self>) {
        let source = self.source_mut();
        let failures = source.failures.entry(key.clone()).or_default();
        *failures += 1;
        let delay = RETRY_FIRST
            .saturating_mul(1 << (*failures - 1).min(5))
            .min(RETRY_MAX);
        let retry_key = key.clone();
        let task = cx.spawn(async move |weak, cx| {
            cx.background_executor().timer(delay).await;
            let _ = weak.update(cx, |view, cx| view.retry_stream(retry_key, generation, cx));
        });
        if let Some(stream) = self.source_mut().streams.get_mut(&key) {
            stream.retry = Some(task);
        }
    }

    fn retry_stream(&mut self, key: StreamKey, generation: u64, cx: &mut Context<Self>) {
        let source = self.source_mut();
        let Some(stream) = source
            .streams
            .get_mut(&key)
            .filter(|stream| stream.generation == generation)
        else {
            return;
        };
        // This runs in the retry's own task: let it finish.
        if let Some(task) = stream.retry.take() {
            task.detach();
        }
        let listed = source
            .pods
            .iter()
            .any(|pod| pod.name == key.pod && pod.uid == key.uid);
        if !matches!(stream.state, StreamState::Failed(_)) || !listed {
            return;
        }
        let pod = stream.pod.clone();
        self.start_stream(key, pod, cx);
        self.source_mut().describe();
        cx.notify();
    }

    fn start_example(&mut self, key: StreamKey, generation: u64, cx: &mut Context<Self>) {
        let Some((pod, reading_on)) = self
            .source()
            .streams
            .get(&key)
            .map(|stream| (stream.pod.clone(), stream.request.resume.is_some()))
        else {
            return;
        };
        let now = self.source().clock.now(cx).timestamp();
        let (mut updates, writes_on) = example::pod_log(&pod, &key.container, false, now);
        if reading_on {
            // Example history is dated back from the clock, so read again it
            // would pass for new lines. Reading on gets only lines written
            // since.
            updates.retain(|update| !matches!(update, PodLogUpdate::Line(_)));
        }
        if let Some(stream) = self.source_mut().streams.get_mut(&key) {
            stream.writes_on = writes_on;
        }
        self.apply_updates(&key, generation, updates, cx);
        if writes_on && self.source().example_writer.is_none() {
            let writer = cx.spawn(async move |weak, cx| {
                for tick in 1u64.. {
                    cx.background_executor().timer(EXAMPLE_TICK).await;
                    if weak
                        .update(cx, |view, cx| view.example_tick(tick, cx))
                        .is_err()
                    {
                        return;
                    }
                }
            });
            self.source_mut().example_writer = Some(writer);
        }
    }

    fn example_tick(&mut self, tick: u64, cx: &mut Context<Self>) {
        let now = self.source().clock.now(cx);
        let feed = self.feed(cx);
        let base = EXAMPLE_INTERVAL.as_millis() as u64 / EXAMPLE_TICK.as_millis() as u64;
        for (key, stream) in &mut self.source_mut().streams {
            let writing = matches!(
                stream.state,
                StreamState::Streaming | StreamState::Connecting
            );
            if !stream.writes_on || !writing || !tick.is_multiple_of(base + stream.generation % 5) {
                continue;
            }
            stream.sequence += 1;
            let line = example::pod_log_line(stream.sequence, now);
            // A full channel drops an example line, nothing more.
            let _ = feed.try_send(Fed {
                key: key.clone(),
                generation: stream.generation,
                update: PodLogUpdate::Line(line),
            });
        }
    }
}

/// A `stress` run (`FRESHKUBE_STRESS_WORKLOAD_RATE`) floods the example
/// streams through the same channel live streams use.
#[cfg(feature = "stress")]
fn stress_flood(view: &mut WorkloadLogView, cx: &mut Context<WorkloadLogView>) {
    let Some(rate) = crate::stress::workload_rate() else {
        return;
    };
    let streams: Vec<(StreamKey, u64)> = view
        .source()
        .streams
        .iter()
        .map(|(key, stream)| (key.clone(), stream.generation))
        .collect();
    if streams.is_empty() {
        return;
    }
    let feed = view.feed(cx);
    let job = view.source().runtime.spawn(crate::stress::line_flood(
        rate,
        streams.len(),
        move |ix, line| {
            let (key, generation) = streams[ix].clone();
            let feed = feed.clone();
            async move {
                feed.send(Fed {
                    key,
                    generation,
                    update: PodLogUpdate::Line(line),
                })
                .await
                .is_ok()
            }
        },
    ));
    view.source_mut().stress = Some(OwnedJob::new(job));
}
