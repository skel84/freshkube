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
//! The streams are `logs/streams.rs`'s: every stream feeds one channel,
//! which one delivery task drains a frame at a time into a single
//! `ingest`, as Talos services do, however many containers write. A stream
//! that fails is read again with the watcher's backoff, jittered, while its
//! pod is listed. One refused for good (403, or 404) waits instead until
//! the pod list changes or the user asks.
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

use chrono::{DateTime, Utc};
use freshkube_core::logs::{LogEvent, ServiceId};
use freshkube_core::pluralize;
use freshkube_core::resources::{
    ContainerRole, Failure, FailureKind, PodSelector, WorkloadPod, WorkloadPods, follow_pods,
};
use gpui_kit::{AnyElement, Context, Pixels, SharedString, Task, Window};
use tokio::runtime::Handle;
use tokio::sync::watch;

#[cfg(any(test, feature = "stress"))]
use super::streams::Fed;
use super::streams::{Clock, StreamKey, StreamReads, StreamSet, StreamSource};
#[cfg(test)]
use super::streams::{EXAMPLE_INTERVAL, Jitter, RETRY_FIRST, RETRY_JITTER, RETRY_MAX, StreamState};
use super::{Columns, DownloadLines, LogSource, LogView};
use crate::backend::OwnedJob;
use crate::resources::model::ResourceIdentity;
use crate::resources::{KubeAccess, example};
use crate::stream_status::Status;
use crate::ui::Tone;
use controls::Controls;
#[cfg(feature = "stress")]
use freshkube_core::resources::PodLogUpdate;

/// The detail pane's Logs tab for a workload.
pub(crate) type WorkloadLogView = LogView<WorkloadLogs>;

/// The most container logs read at once. The newest pods come first; the
/// rest are counted in a notice.
pub(crate) use super::streams::MAX_STREAMS;
/// Lines from the end each container's log starts with.
const TAIL: i64 = 100;
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

/// One entry of the Pod select, derived when the pods or the streams
/// change.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PodChoice {
    pub(super) name: String,
    /// The menu's words: the pod's short label, and why its lines may be
    /// missing.
    pub(super) label: SharedString,
}

pub(crate) struct WorkloadLogs {
    /// The containers' streams, and how far each was read.
    pub(super) reads: StreamSet,
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
    /// A `stress` run's flood into the feed.
    #[cfg(feature = "stress")]
    stress: Option<OwnedJob>,
    /// Containers not read because of [`MAX_STREAMS`].
    pub(super) left_out: usize,
    /// Tags the user hid; tags seen in this review, hidden or not.
    hidden: BTreeSet<ServiceId>,
    seen: BTreeSet<ServiceId>,
    pub(super) chips: Rc<Vec<Chip>>,
    /// The pod whose lines show, by name, or every pod. It is a filter:
    /// every stream reads on, so All pods shows them again at once. A pod
    /// that leaves keeps the pick, with what it wrote.
    pub(super) pod: Option<String>,
    /// The Pod select's entries: the pods newest first, and a picked pod
    /// that left.
    pub(super) pod_choices: Rc<Vec<PodChoice>>,
    /// What the Pod select's button reads, and says to assistive
    /// technology.
    pub(super) pod_label: SharedString,
    pub(super) pod_aria: SharedString,
    /// Why the list is empty when the picked pod is one the cap leaves
    /// unread; it wins over every other empty text.
    not_read: Option<SharedString>,
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
    /// What "+N" reads, and the hidden and total counts it was derived for.
    pub(super) more_label: SharedString,
    pub(super) more_for: Option<(usize, usize)>,
    /// The streams row's accessible name: how many containers it holds.
    pub(super) streams_label: SharedString,
    /// The cap's notice while containers are left out, which also closes
    /// the Pod select's menu.
    pub(super) capped: Option<SharedString>,
    /// The refused streams' notice, while any is refused.
    pub(super) refused_note: Option<SharedString>,
    /// What the source column and the chips show for each tag: the pod
    /// name without the prefix every pod shares, derived with the chips.
    labels: BTreeMap<ServiceId, SharedString>,
    pub(super) status: Status,
    empty: SharedString,
}

impl WorkloadLogs {
    fn new(runtime: Handle, clock: Clock) -> Self {
        let mut source = Self {
            reads: StreamSet::new(runtime, clock),
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
            #[cfg(feature = "stress")]
            stress: None,
            left_out: 0,
            hidden: BTreeSet::new(),
            seen: BTreeSet::new(),
            chips: Rc::default(),
            pod: None,
            pod_choices: Rc::default(),
            pod_label: SharedString::default(),
            pod_aria: SharedString::default(),
            not_read: None,
            chips_revision: 0,
            chip_widths: Vec::new(),
            more_width: Pixels::ZERO,
            measured: None,
            fits: usize::MAX,
            more_open: false,
            more_label: SharedString::default(),
            more_for: None,
            streams_label: SharedString::default(),
            capped: None,
            refused_note: None,
            labels: BTreeMap::new(),
            status: Status::default(),
            empty: SharedString::default(),
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
        self.reads.drop_streams();
        // Nothing is read, so the cap leaves nothing out until the pods
        // are listed again.
        self.left_out = 0;
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
            .chain(self.reads.streams.keys().map(StreamKey::service));
        self.labels = short_labels(services);
        let chips: Vec<Chip> = self
            .reads
            .streams
            .iter()
            .filter(|(key, _)| self.pod.as_ref().is_none_or(|pod| key.pod == *pod))
            .map(|(key, stream)| {
                let service = key.service();
                let name = service.as_str();
                let shown = !self.hidden.contains(&service);
                let retrying = if stream.retry.is_some() {
                    ", reading again soon"
                } else if stream.state.refused() {
                    ", read again when the pods change"
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
        self.streams_label = pluralize(self.chips.len(), "container", "containers").into();
        self.capped = (self.left_out > 0).then(|| capped_note(MAX_STREAMS + self.left_out).into());
        let refused = self
            .reads
            .streams
            .values()
            .filter(|stream| stream.state.refused())
            .count();
        self.refused_note = (refused > 0).then(|| {
            format!(
                "Logs refused for {}: read again when the pods change.",
                pluralize(refused, "container", "containers")
            )
            .into()
        });
        let pods = self.pods.len();
        let streams = self.reads.streams.len();
        // A pick narrows the counts; the tag and the banners stay the
        // workload's, since every stream reads on whatever is picked.
        let counts = match &self.pod {
            None => format!(
                "{} · {}",
                pluralize(pods, "pod", "pods"),
                pluralize(streams, "container", "containers")
            ),
            Some(pod) => {
                let present = self.pods.iter().any(|seen| seen.name == *pod);
                let read = self
                    .reads
                    .streams
                    .keys()
                    .filter(|key| key.pod == *pod)
                    .count();
                format!(
                    "{} of {} · {read} of {}",
                    usize::from(present),
                    pluralize(pods, "pod", "pods"),
                    pluralize(streams, "container", "containers")
                )
            }
        };
        self.describe_pods();
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
                if self.reads.errors.is_empty() {
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
        self.status = Status::new(tone, tag, &text);
        self.empty = empty.into();
        self.not_read = self.pod_not_read().map(Into::into);
    }

    /// The Pod select's entries and its button's words. A pod none of
    /// whose containers is read says so, so its filter never looks empty
    /// for a reason it doesn't give.
    fn describe_pods(&mut self) {
        let mut names: Vec<&str> = self.pods.iter().map(|pod| pod.name.as_str()).collect();
        let gone = self.pod.as_deref().filter(|picked| !names.contains(picked));
        names.extend(gone);
        let labels = short_labels(names.iter().map(|name| ServiceId::new(*name)));
        let label = |name: &str| {
            labels
                .get(&ServiceId::new(name))
                .map(|label| label.to_string())
                .unwrap_or_else(|| name.to_owned())
        };
        let choices: Vec<PodChoice> = names
            .iter()
            .map(|&name| {
                let mut words = label(name);
                if Some(name) == gone {
                    words.push_str(" (gone)");
                } else if self.left_out > 0 && !self.reads.streams.keys().any(|key| key.pod == name)
                {
                    words.push_str(" · not read");
                }
                PodChoice {
                    name: name.to_owned(),
                    label: words.into(),
                }
            })
            .collect();
        self.pod_label = match &self.pod {
            None => "All pods".into(),
            Some(pod) => choices
                .iter()
                .find(|choice| choice.name == *pod)
                .map(|choice| choice.label.clone())
                .unwrap_or_else(|| pod.clone().into()),
        };
        self.pod_aria = format!("Pod: {}", self.pod_label).into();
        self.pod_choices = Rc::new(choices);
    }

    /// The cap's words, when the picked pod is one the cap leaves unread.
    fn pod_not_read(&self) -> Option<String> {
        let pod = self.pod.as_ref()?;
        let present = self.pods.iter().any(|seen| seen.name == *pod);
        let read = self.reads.streams.keys().any(|key| key.pod == *pod);
        (present && !read && self.left_out > 0).then(|| {
            format!(
                "{pod} isn't read. {}",
                capped_note(MAX_STREAMS + self.left_out)
            )
        })
    }

    /// A note between lines: a pod joined or left.
    fn marker(&mut self, pod: &str, text: String, at: DateTime<Utc>) -> LogEvent {
        let service = ServiceId::new(pod);
        self.seen.insert(service.clone());
        LogEvent::marker(service, at.fixed_offset(), text)
    }

    /// Streams refused for good, with their pods, to read again.
    fn refused_streams(&self) -> Vec<(StreamKey, ResourceIdentity)> {
        self.reads.refused_streams()
    }

    /// The tags shown: every tag seen but those the user hid, of the
    /// picked pod when there is one.
    fn shown(&self) -> BTreeSet<ServiceId> {
        self.seen
            .difference(&self.hidden)
            .filter(|service| {
                self.pod
                    .as_ref()
                    .is_none_or(|pod| split(service).0 == pod.as_str())
            })
            .cloned()
            .collect()
    }
}

/// What the cap leaves out, in the words of the note under the toolbar.
fn capped_note(containers: usize) -> String {
    format!("Reading {MAX_STREAMS} of {containers} containers, the newest pods first.")
}

impl LogSource for WorkloadLogs {
    fn download_name(view: &WorkloadLogView, lines: DownloadLines) -> String {
        let source = view.source();
        let mut name = match &source.workload {
            Some(workload) => format!("{}-{}", workload.namespace, workload.name),
            None => "workload".into(),
        };
        // Visible lines are the picked pod's; every retained line isn't.
        if lines == DownloadLines::Visible
            && let Some(pod) = &source.pod
        {
            name = format!("{name}-{pod}");
        }
        name
    }

    fn prepare_controls(
        view: &mut WorkloadLogView,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<WorkloadLogView>,
    ) {
        view.fit_chips(width, window, cx);
    }

    fn tools(view: &WorkloadLogView, cx: &mut Context<WorkloadLogView>) -> Vec<AnyElement> {
        view.render_tools(cx)
    }

    fn notes(view: &WorkloadLogView, cx: &mut Context<WorkloadLogView>) -> Vec<AnyElement> {
        view.render_controls(cx)
    }

    fn empty_message(view: &WorkloadLogView) -> SharedString {
        // The other pods' lines are retained, so this comes first.
        if let Some(not_read) = &view.source().not_read {
            not_read.clone()
        } else if !view.has_lines() {
            view.source().empty.clone()
        } else {
            "No retained lines pass the filters.".into()
        }
    }

    fn panel_label(_view: &WorkloadLogView) -> SharedString {
        "Workload logs panel".into()
    }

    fn errors(&self) -> &BTreeMap<ServiceId, String> {
        &self.reads.errors
    }

    fn source_label(&self, service: &ServiceId) -> Option<SharedString> {
        self.labels.get(service).cloned()
    }

    fn tags_lines(&self) -> bool {
        true
    }
}

impl StreamSource for WorkloadLogs {
    const APPLY_PROBE: &'static str = "workload-logs.apply";

    fn reads(&self) -> &StreamSet {
        &self.reads
    }

    fn reads_mut(&mut self) -> &mut StreamSet {
        &mut self.reads
    }

    fn access(&self) -> Option<KubeAccess> {
        self.access.clone()
    }

    fn tag(key: &StreamKey) -> ServiceId {
        key.service()
    }

    fn tail(&self) -> Option<i64> {
        Some(TAIL)
    }

    fn previous(&self) -> bool {
        false
    }

    fn listed(&self, key: &StreamKey) -> bool {
        self.pods
            .iter()
            .any(|pod| pod.name == key.pod && pod.uid == key.uid)
    }

    fn streams_changed(&mut self) {
        self.describe();
    }

    fn stream_started(view: &mut WorkloadLogView, key: &StreamKey) {
        view.source_mut().seen.insert(key.service());
        // Its lines show unless the user hid them.
        view.clear_shown();
    }
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
        source.reads.positions.clear();
        source.reads.failures.clear();
        source.left_out = 0;
        source.hidden.clear();
        source.seen.clear();
        source.pod = None;
        source.reads.errors.clear();
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

    /// Shows one pod's lines, or every pod's.
    fn pick_pod(&mut self, pod: Option<String>, cx: &mut Context<Self>);

    /// Shows or hides each line's time.
    fn set_timestamps(&mut self, shown: bool, cx: &mut Context<Self>);

    /// Starts the pod watch once the tab is wanted, the page shows and the
    /// selector is known.
    fn start_watch(&mut self, cx: &mut Context<Self>);

    /// Watches the pods again after a failure.
    fn retry(&mut self, cx: &mut Context<Self>);

    /// Reads every refused stream again, as Retry asks.
    fn retry_refused(&mut self, cx: &mut Context<Self>);

    /// Takes the pods as last seen by the watch of `epoch`. Returns false
    /// when that watch is gone, which ends its delivery.
    fn apply_pods(&mut self, epoch: u64, pods: WorkloadPods, cx: &mut Context<Self>) -> bool;
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

    fn pick_pod(&mut self, pod: Option<String>, cx: &mut Context<Self>) {
        if self.source().pod == pod {
            return;
        }
        let source = self.source_mut();
        source.pod = pod;
        source.describe();
        self.capture_anchor();
        self.clear_shown();
        cx.notify();
    }

    fn set_timestamps(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.set_columns(
            Columns {
                time: shown,
                ..self.columns()
            },
            cx,
        );
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
            let now = self.source().reads.clock.now(cx).timestamp();
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
        let job = self.source().reads.runtime.spawn(async move {
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

    fn retry_refused(&mut self, cx: &mut Context<Self>) {
        let refused = self.source().refused_streams();
        if refused.is_empty() {
            return;
        }
        for (key, pod) in refused {
            // A fresh read: a failure after it backs off from the start.
            self.source_mut().reads.failures.remove(&key);
            self.start_stream(key, pod, cx);
        }
        self.source_mut().describe();
        cx.notify();
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
        // A pod added, removed or replaced: what was refused may be read now.
        let changed = source.known.as_ref().is_some_and(|known| *known != now);
        if let Some(known) = source.known.take() {
            for (pod, _) in known.difference(&now) {
                markers.push((pod.clone(), format!("Pod {pod} left")));
            }
            for (pod, _) in now.difference(&known) {
                markers.push((pod.clone(), format!("Pod {pod} joined")));
            }
        }
        source.known = Some(now);
        let at = source.reads.clock.now(cx);
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
            .reads
            .streams
            .keys()
            .filter(|key| !keep.contains(key))
            .cloned()
            .collect();
        for key in gone {
            source.reads.streams.remove(&key);
            source.reads.errors.remove(&key.service());
        }
        let alive: BTreeSet<(String, String)> = source
            .pods
            .iter()
            .map(|pod| (pod.name.clone(), pod.uid.clone()))
            .collect();
        source
            .reads
            .positions
            .retain(|key, _| alive.contains(&(key.pod.clone(), key.uid.clone())));
        source
            .reads
            .failures
            .retain(|key, _| alive.contains(&(key.pod.clone(), key.uid.clone())));
        let new: Vec<_> = wanted
            .into_iter()
            .filter(|(key, _)| !source.reads.streams.contains_key(key))
            .collect();
        // Taken before the new streams start, so one refused at once isn't
        // read twice.
        let refused = if changed {
            source.refused_streams()
        } else {
            Vec::new()
        };
        // Each is a fresh read: a failure after it backs off from the start.
        for (key, _) in &refused {
            source.reads.failures.remove(key);
        }
        if !markers.is_empty() {
            self.clear_shown();
            self.ingest(markers, cx);
        }
        for (key, pod) in new.into_iter().chain(refused) {
            self.start_stream(key, pod, cx);
        }
        self.clear_shown();
        self.source_mut().describe();
        cx.notify();
        true
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
        .reads
        .streams
        .iter()
        .map(|(key, stream)| (key.clone(), stream.generation))
        .collect();
    if streams.is_empty() {
        return;
    }
    let feed = view.feed(cx);
    let job = view.source().reads.runtime.spawn(crate::stress::line_flood(
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
