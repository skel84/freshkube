//! The detail pane's workload logs: every container of every pod a
//! Deployment, StatefulSet, DaemonSet, ReplicaSet or Job runs, in one view.
//! Lines from all streams are interleaved by their timestamps and tagged
//! `pod/container`. Read-only: it watches the workload's pods and reads
//! their logs, nothing else.
//!
//! Pods are found by the workload's selector (core's
//! `follow_workload_pods`) and followed as they come and go; each container
//! is read by core's `follow_pod_log`, at most [`MAX_STREAMS`] at once. A
//! line that arrives late, from a stream behind the others, is placed by
//! its time among the retained lines, as with Talos services; a paused
//! review keeps its place and the selection keeps its lines.
//!
//! Nothing is read until the Logs tab first shows for the workload. Then
//! the watch and the streams live while it stays open, whichever tab shows,
//! and stop when another object opens, the pane closes, the page hides or
//! the connection changes. Showing the page again reads each container on
//! from its last line.
//!
//! This file holds the source's state and its streams; `controls` draws
//! the status, the streams and their notices.

mod controls;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use freshkube_core::logs::{LogEvent, ServiceId};
use freshkube_core::resources::{
    ContainerRole, Failure, FailureKind, LogPosition, LogRequest, PodLogUpdate, WorkloadPod,
    WorkloadPods, follow_pod_log, follow_workload_pods,
};
use gpui_kit::{AnyElement, Context, SharedString, Task, Window};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};

use super::{Columns, LogSource, LogView};
use crate::backend::{OwnedJob, STREAM_QUEUE_CAPACITY};
use crate::resources::model::ResourceIdentity;
use crate::resources::{KubeAccess, example, live};
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
/// a little, so they don't all write at once.
const EXAMPLE_INTERVAL: Duration = Duration::from_millis(1_500);

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
    /// The pod, for the example data's log.
    pod: ResourceIdentity,
    /// Advances with each read; updates from an older one are dropped.
    generation: u64,
    job: Option<OwnedJob>,
    delivery: Option<Task<()>>,
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
    pub(super) label: SharedString,
    pub(super) tone: Tone,
    pub(super) state: SharedString,
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
    /// What the source column and the chips show for each tag: the pod
    /// name without the prefix every pod shares, derived with the chips.
    labels: BTreeMap<ServiceId, SharedString>,
    pub(super) status: Status,
    empty: SharedString,
    /// Failed streams, shown above the lines while the others go on.
    errors: BTreeMap<ServiceId, String>,
}

impl WorkloadLogs {
    fn new(runtime: Handle) -> Self {
        let mut source = Self {
            runtime,
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
            positions: BTreeMap::new(),
            next_generation: 0,
            left_out: 0,
            hidden: BTreeSet::new(),
            seen: BTreeSet::new(),
            chips: Rc::default(),
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
                Chip {
                    label: self
                        .labels
                        .get(&service)
                        .cloned()
                        .unwrap_or_else(|| service.as_str().to_owned().into()),
                    tone: stream.state.tone(),
                    state: stream.state.label().into(),
                    shown: !self.hidden.contains(&service),
                    service,
                }
            })
            .collect();
        self.chips = Rc::new(chips);
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
    fn marker(&mut self, pod: &str, text: String) -> LogEvent {
        let service = ServiceId::new(pod);
        self.seen.insert(service.clone());
        LogEvent::marker(service, Utc::now().fixed_offset(), text)
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
    fn controls(view: &WorkloadLogView, cx: &mut Context<WorkloadLogView>) -> Vec<AnyElement> {
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

/// Short labels for `pod/container` tags: each pod name without the prefix
/// all of them share, up to a `-` (a ReplicaSet's pods keep their random
/// suffix, a StatefulSet's their ordinal), and the container only when
/// the tags name more than one.
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
    let containers: BTreeSet<&str> = tags.iter().map(|service| split(service).1).collect();
    tags.iter()
        .map(|service| {
            let (pod, container) = split(service);
            let pod = &pod[cut..];
            let label = if containers.len() > 1 {
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
        Self::with_source(WorkloadLogs::new(runtime), window, cx).with_columns(Columns {
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
        source.left_out = 0;
        source.hidden.clear();
        source.seen.clear();
        source.errors.clear();
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

    /// Example data reads at once; a running example container writes on.
    fn start_example(&mut self, key: StreamKey, generation: u64, cx: &mut Context<Self>);
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
            let pods = example::workload_pods(&workload, &selector, live::now());
            self.apply_pods(
                epoch,
                WorkloadPods {
                    listed: true,
                    pods,
                    failure: None,
                },
                cx,
            );
            return;
        }
        let (sender, mut receiver) = watch::channel(WorkloadPods::default());
        let namespace = workload.namespace.clone();
        let job = self.source().runtime.spawn(async move {
            match access.client().await {
                Ok(client) => follow_workload_pods(client, namespace, selector, sender).await,
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
        let markers: Vec<LogEvent> = markers
            .into_iter()
            .map(|(pod, text)| source.marker(&pod, text))
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
        let source = self.source_mut();
        source.next_generation += 1;
        let generation = source.next_generation;
        source.seen.insert(key.service());
        source.streams.insert(
            key.clone(),
            Stream {
                state: StreamState::Connecting,
                pod: pod.clone(),
                generation,
                job: None,
                delivery: None,
            },
        );
        // Its lines show unless the user hid them.
        self.clear_shown();
        if let KubeAccess::Example = access {
            self.start_example(key, generation, cx);
            return;
        }
        let request = LogRequest {
            namespace: pod.namespace.clone(),
            pod: key.pod.clone(),
            container: key.container.clone(),
            previous: false,
            tail: Some(TAIL),
            resume,
        };
        let (sender, mut receiver) = mpsc::channel(STREAM_QUEUE_CAPACITY);
        let job = self.source().runtime.spawn(async move {
            match access.client().await {
                Ok(client) => follow_pod_log(client, request, sender).await,
                Err(error) => {
                    access.forget();
                    let failure = Failure::new(FailureKind::Other, error);
                    let _ = sender.send(PodLogUpdate::Failed(failure)).await;
                }
            }
        });
        let delivery_key = key.clone();
        let delivery = cx.spawn(async move |weak, cx| {
            while let Some(first) = receiver.recv().await {
                let mut batch = vec![first];
                // At most a full queue per turn, then yield, however busy
                // the container.
                for _ in 1..STREAM_QUEUE_CAPACITY {
                    let Ok(update) = receiver.try_recv() else {
                        break;
                    };
                    batch.push(update);
                }
                let applied = weak
                    .update(cx, |view, cx| {
                        view.apply_updates(&delivery_key, generation, batch, cx)
                    })
                    .unwrap_or(false);
                if !applied {
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
            }
        });
        if let Some(stream) = self.source_mut().streams.get_mut(&key) {
            stream.job = Some(OwnedJob::new(job));
            stream.delivery = Some(delivery);
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
        let service = key.service();
        let mut lines = Vec::new();
        let source = self.source_mut();
        for update in updates {
            let position = source.positions.entry(key.clone()).or_default();
            let Some(stream) = source.streams.get_mut(key) else {
                break;
            };
            match update {
                PodLogUpdate::Line(line) => {
                    position.record(&line);
                    lines.push(LogEvent::new(service.clone(), line));
                }
                PodLogUpdate::Streaming => {
                    stream.state = StreamState::Streaming;
                    source.errors.remove(&service);
                }
                PodLogUpdate::Waiting(reason) => stream.state = StreamState::Waiting(reason),
                PodLogUpdate::Reconnecting { attempt, .. } => {
                    stream.state = StreamState::Reconnecting(attempt)
                }
                PodLogUpdate::Restarting(ended) => {
                    let how = ended.map(|ended| format!(" ({ended})")).unwrap_or_default();
                    let at = position.time().unwrap_or_else(Utc::now);
                    lines.push(LogEvent::marker(
                        service.clone(),
                        at.fixed_offset(),
                        format!("{} restarted{how}", key.container),
                    ));
                }
                PodLogUpdate::Ended(_) => {
                    stream.job = None;
                    stream.state = StreamState::Ended;
                }
                PodLogUpdate::Failed(failure) => {
                    stream.job = None;
                    source.errors.insert(service.clone(), failure.to_string());
                    stream.state = StreamState::Failed(failure);
                }
            }
        }
        source.describe();
        self.ingest(lines, cx);
        cx.notify();
        true
    }

    fn start_example(&mut self, key: StreamKey, generation: u64, cx: &mut Context<Self>) {
        let Some(pod) = self
            .source()
            .streams
            .get(&key)
            .map(|stream| stream.pod.clone())
        else {
            return;
        };
        let reading_on = self
            .source()
            .positions
            .get(&key)
            .is_some_and(|position| position.time().is_some());
        let (mut updates, writes_on) = example::pod_log(&pod, &key.container, false, live::now());
        if reading_on {
            // Example history is dated back from the clock, so read again it
            // would pass for new lines. Reading on gets only lines written
            // since.
            updates.retain(|update| !matches!(update, PodLogUpdate::Line(_)));
        }
        self.apply_updates(&key, generation, updates, cx);
        if !writes_on {
            return;
        }
        // Each container writes at its own pace, so their lines interleave.
        let interval = EXAMPLE_INTERVAL + Duration::from_millis(250 * (generation % 5));
        let delivery = cx.spawn(async move |weak, cx| {
            for sequence in generation.. {
                cx.background_executor().timer(interval).await;
                let line = example::pod_log_line(sequence, Utc::now());
                let applied = weak
                    .update(cx, |view, cx| {
                        view.apply_updates(&key, generation, vec![PodLogUpdate::Line(line)], cx)
                    })
                    .unwrap_or(false);
                if !applied {
                    return;
                }
            }
        });
        let source = self.source_mut();
        if let Some(stream) = source
            .streams
            .values_mut()
            .find(|stream| stream.generation == generation)
        {
            stream.delivery = Some(delivery);
        }
    }
}
