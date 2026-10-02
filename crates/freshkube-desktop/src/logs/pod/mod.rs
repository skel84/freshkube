//! The detail pane's pod logs: one container's log, followed live or its
//! previous instance read to the end. The stream lives while the pod stays
//! selected, whichever tab shows, and stops when another object opens, the
//! pane closes, the page hides or the connection changes. Read-only: it
//! gets the pod and reads logs, nothing else.
//!
//! This file holds the source's state and its stream; `controls` draws the
//! container picker, the stream controls and the state of the stream.

mod controls;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use freshkube_core::logs::{LogEvent, ServiceId};
use freshkube_core::resources::{
    Container, ContainerRole, ContainerState, Failure, FailureKind, LogPosition, LogRequest,
    PodContainers, PodLogUpdate, Termination, follow_pod_log,
};
use gpui_kit::{AnyElement, Context, SharedString, Task, Window};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use super::{Columns, LogSource, LogView};
use crate::backend::{OwnedJob, STREAM_QUEUE_CAPACITY};
use crate::resources::model::ResourceIdentity;
use crate::resources::{KubeAccess, example, live};
use crate::ui::Tone;

/// The detail pane's Logs tab.
pub(crate) type PodLogView = LogView<PodLogs>;

/// How many lines from the end a new stream starts with; `None` is all.
pub(super) const TAILS: [Option<i64>; 5] = [Some(100), Some(500), Some(1_000), Some(5_000), None];
const DEFAULT_TAIL: Option<i64> = Some(500);
/// How often an example container writes another line.
const EXAMPLE_INTERVAL: Duration = Duration::from_secs(2);
/// A container that restarted this recently, after failing, is shown as
/// crash-looping even while it runs.
const RECENT_FAILURE: chrono::TimeDelta = chrono::TimeDelta::minutes(15);

/// Where the stream of the chosen container stands.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum StreamState {
    /// Nothing asked for yet: the Logs tab hasn't been shown for this pod,
    /// or its containers aren't known yet.
    Idle,
    Connecting,
    /// The container hasn't started, with the kubelet's reason.
    Waiting(String),
    Streaming,
    Reconnecting {
        attempt: u32,
        delay: Duration,
        since: DateTime<Utc>,
    },
    /// The log ended: the container exited for good, or the previous
    /// instance was read to its end.
    Ended(Option<Termination>),
    /// The user stopped it.
    Stopped,
    Failed(Failure),
}

/// One entry of the container picker.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Choice {
    pub(super) name: String,
    pub(super) role: ContainerRole,
    pub(super) label: SharedString,
    /// An init container that hasn't started has no log to choose.
    pub(super) enabled: bool,
}

/// What the controls say about the stream, derived when it changes.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Status {
    pub(super) tone: Tone,
    pub(super) tag: SharedString,
    pub(super) text: SharedString,
    /// Both, for assistive technology and tests.
    pub(super) label: SharedString,
}

pub(crate) struct PodLogs {
    runtime: Handle,
    access: Option<KubeAccess>,
    pod: Option<ResourceIdentity>,
    containers: PodContainers,
    /// Whether `containers` was read for this pod yet.
    known: bool,
    pub(super) choices: Rc<Vec<Choice>>,
    pub(super) container: Option<String>,
    pub(super) tail: Option<i64>,
    /// Read the previous instance instead of following the current one.
    pub(super) previous: bool,
    /// The Logs tab was shown for this pod, so its log is read.
    wanted: bool,
    active: bool,
    /// The stream ran when the page hid; showing it reads on.
    suspended: bool,
    pub(super) state: StreamState,
    position: LogPosition,
    /// Lines read into this review, markers aside.
    lines: usize,
    /// When the connection was lost, while reconnecting.
    lost_at: Option<DateTime<Utc>>,
    /// Advances with each stream; updates from an older one are dropped.
    stream: u64,
    job: Option<OwnedJob>,
    delivery: Option<Task<()>>,
    pub(super) status: Status,
    /// Says the container keeps crashing, and offers its previous instance.
    pub(super) hint: Option<SharedString>,
    empty: SharedString,
    /// Stream failures show in the controls, so the view's list stays empty.
    errors: BTreeMap<ServiceId, String>,
}

impl PodLogs {
    fn new(runtime: Handle) -> Self {
        let mut source = Self {
            runtime,
            access: None,
            pod: None,
            containers: PodContainers::default(),
            known: false,
            choices: Rc::default(),
            container: None,
            tail: DEFAULT_TAIL,
            previous: false,
            wanted: false,
            active: false,
            suspended: false,
            state: StreamState::Idle,
            position: LogPosition::default(),
            lines: 0,
            lost_at: None,
            stream: 0,
            job: None,
            delivery: None,
            status: Status {
                tone: Tone::Unknown,
                tag: SharedString::default(),
                text: SharedString::default(),
                label: SharedString::default(),
            },
            hint: None,
            empty: SharedString::default(),
            errors: BTreeMap::new(),
        };
        source.describe();
        source
    }

    /// The pod's `namespace/name`, which the review is of.
    fn address(&self) -> String {
        self.pod
            .as_ref()
            .map(ResourceIdentity::address)
            .unwrap_or_default()
    }

    /// Whether the stream is open or about to be.
    pub(super) fn running(&self) -> bool {
        matches!(
            self.state,
            StreamState::Connecting
                | StreamState::Waiting(_)
                | StreamState::Streaming
                | StreamState::Reconnecting { .. }
        )
    }

    pub(super) fn chosen(&self) -> Option<&freshkube_core::resources::Container> {
        self.containers.get(self.container.as_deref()?)
    }

    /// Whether the chosen container ran before its current instance.
    pub(super) fn has_previous(&self) -> bool {
        self.chosen()
            .is_some_and(|container| container.has_previous())
    }

    /// Drops the stream and its delivery; anything still on the way is
    /// dropped with them.
    fn drop_stream(&mut self) {
        self.stream += 1;
        self.job = None;
        self.delivery = None;
    }

    /// Derives the picker's entries and the crash-loop hint from the
    /// containers.
    fn derive_containers(&mut self) {
        self.choices = Rc::new(
            self.containers
                .containers
                .iter()
                .map(|container| Choice {
                    name: container.name.clone(),
                    role: container.role,
                    label: choice_label(container).into(),
                    enabled: container.role != ContainerRole::Init || container.started(),
                })
                .collect(),
        );
        self.hint = self.chosen().and_then(|container| {
            let last = container.last_termination.as_ref()?;
            let backing_off =
                matches!(&container.state, ContainerState::Waiting(reason) if reason == "CrashLoopBackOff");
            let recent = last.exit_code != 0
                && last
                    .finished
                    .is_some_and(|finished| Utc::now() - finished < RECENT_FAILURE);
            (backing_off || recent).then(|| {
                let times = match container.restarts {
                    1 => "once".to_owned(),
                    count => format!("{count} times"),
                };
                format!(
                    "{} restarted {times}, last {}.",
                    container.name,
                    exited(last)
                )
                .into()
            })
        });
    }

    /// Derives what the controls and the empty list say.
    fn describe(&mut self) {
        let name = self.container.clone().unwrap_or_default();
        let (tone, tag, text, empty) = match &self.state {
            StreamState::Idle if self.pod.is_some() && !self.known => (
                Tone::Unknown,
                "Reading",
                String::new(),
                "Reading the pod".to_owned(),
            ),
            StreamState::Idle if self.known && self.container.is_none() => (
                Tone::Unknown,
                "No containers",
                String::new(),
                "This pod has no containers.".to_owned(),
            ),
            StreamState::Idle => (Tone::Unknown, "Idle", String::new(), String::new()),
            StreamState::Connecting => (
                Tone::Unknown,
                "Connecting",
                String::new(),
                "Waiting for the first line".to_owned(),
            ),
            StreamState::Waiting(reason) => (
                Tone::Warn,
                "Waiting",
                format!("{name} hasn't started ({reason}). Its log opens once it does."),
                format!("{name} hasn't started yet."),
            ),
            StreamState::Streaming => (
                Tone::Good,
                "Streaming",
                String::new(),
                format!("{name} has written nothing yet"),
            ),
            StreamState::Reconnecting {
                attempt,
                delay,
                since,
            } => (
                Tone::Warn,
                "Reconnecting",
                format!(
                    "Connection lost at {}, reconnecting (attempt {attempt}, next in {} s)",
                    clock(*since),
                    delay.as_secs()
                ),
                "Reconnecting".to_owned(),
            ),
            StreamState::Ended(_) if self.previous => {
                let how = self
                    .chosen()
                    .and_then(|container| container.last_termination.as_ref())
                    .map(|last| format!(" · {}", exited(last)))
                    .unwrap_or_default();
                (
                    Tone::Unknown,
                    "Previous instance",
                    format!("Previous instance{how} · complete"),
                    "The previous instance wrote nothing.".to_owned(),
                )
            }
            StreamState::Ended(ended) => (
                Tone::Unknown,
                "Ended",
                match ended {
                    Some(ended) => format!("Stream ended: {name} {}", exited(ended)),
                    None => "Stream ended".to_owned(),
                },
                format!("{name} wrote nothing."),
            ),
            StreamState::Stopped => (
                Tone::Unknown,
                "Stopped",
                "Resume reads on from the last line.".to_owned(),
                "Stopped before the first line.".to_owned(),
            ),
            StreamState::Failed(failure) => (
                Tone::Crit,
                "Failed",
                failure.to_string(),
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

    /// A note between lines, placed after the last line read.
    fn marker(&self, text: String) -> LogEvent {
        let at = self.position.time().unwrap_or_else(Utc::now);
        LogEvent::marker(self.container.clone().unwrap_or_default(), at, text)
    }
}

/// A container picker's entry: `app · Running`, `migrate · Not started`.
pub(crate) fn choice_label(container: &Container) -> String {
    let state = match &container.state {
        ContainerState::Running(_) => "Running".to_owned(),
        ContainerState::Waiting(reason)
            if reason.is_empty()
                || (container.role == ContainerRole::Init && !container.started()) =>
        {
            "Not started".to_owned()
        }
        ContainerState::Waiting(reason) => reason.clone(),
        ContainerState::Terminated(ended) => match ended.reason.as_str() {
            "" => format!("Exited {}", ended.exit_code),
            reason => reason.to_owned(),
        },
    };
    format!("{} · {state}", container.name)
}

/// The heading over a container picker's entries of one role.
pub(crate) fn role_heading(role: ContainerRole) -> &'static str {
    match role {
        ContainerRole::Init => "INIT CONTAINERS",
        ContainerRole::App => "CONTAINERS",
        ContainerRole::Ephemeral => "EPHEMERAL",
    }
}

/// `exited 137 (OOMKilled) at 12:04:10`.
fn exited(ended: &Termination) -> String {
    let mut text = format!("exited {}", ended.exit_code);
    if !ended.reason.is_empty() {
        text.push_str(&format!(" ({})", ended.reason));
    }
    if let Some(finished) = ended.finished {
        text.push_str(&format!(" at {}", clock(finished)));
    }
    text
}

/// A time of day in local time, with the date when it isn't today.
fn clock(time: DateTime<Utc>) -> String {
    let local = time.with_timezone(&Local);
    if local.date_naive() == Local::now().date_naive() {
        local.format("%H:%M:%S").to_string()
    } else {
        local.format("%Y-%m-%d %H:%M:%S").to_string()
    }
}

impl LogSource for PodLogs {
    fn controls(view: &PodLogView, cx: &mut Context<PodLogView>) -> Vec<AnyElement> {
        view.render_controls(cx)
    }

    fn empty_message(view: &PodLogView) -> SharedString {
        if view.review.logs.buffer().entries().is_empty() {
            view.source.empty.clone()
        } else {
            "No retained lines pass the level filter.".into()
        }
    }

    fn live(view: &PodLogView) -> bool {
        !view.source.previous
    }

    fn errors(&self) -> &BTreeMap<ServiceId, String> {
        &self.errors
    }
}

impl LogView<PodLogs> {
    pub(crate) fn for_pods(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self::with_source(PodLogs::new(runtime), window, cx);
        // One container at a time: its name would repeat on every row.
        view.columns.source = false;
        view
    }

    /// Fresh handles for the same connection.
    pub(crate) fn set_access(&mut self, access: KubeAccess) {
        self.source.access = Some(access);
    }

    /// Shows the logs of `pod`, or of none. Another pod starts over: its
    /// containers are read again and nothing streams until it is wanted.
    pub(crate) fn show_pod(
        &mut self,
        pod: Option<ResourceIdentity>,
        access: Option<KubeAccess>,
        cx: &mut Context<Self>,
    ) {
        if access.is_some() {
            self.source.access = access;
        }
        if self.source.pod == pod {
            return;
        }
        self.source.drop_stream();
        self.reset_lines(&pod.as_ref().map(|pod| pod.address()).unwrap_or_default());
        let source = &mut self.source;
        source.pod = pod;
        source.containers = PodContainers::default();
        source.known = false;
        source.container = None;
        source.previous = false;
        source.wanted = false;
        source.suspended = false;
        source.state = StreamState::Idle;
        source.position = LogPosition::default();
        source.lines = 0;
        source.lost_at = None;
        source.derive_containers();
        source.describe();
        cx.notify();
    }

    /// The pod's containers as last read. The first read picks the default
    /// container, and starts its log if it is wanted.
    pub(crate) fn set_containers(&mut self, containers: PodContainers, cx: &mut Context<Self>) {
        let source = &mut self.source;
        if source.pod.is_none() || (source.known && source.containers == containers) {
            return;
        }
        source.known = true;
        if source
            .container
            .as_deref()
            .is_none_or(|name| containers.get(name).is_none())
        {
            source.container = containers.default.clone();
        }
        source.containers = containers;
        source.derive_containers();
        source.describe();
        if source.wanted && source.state == StreamState::Idle {
            self.start(true, cx);
        }
        cx.notify();
    }

    /// The Logs tab shows: read the log from now on, while this pod stays.
    pub(crate) fn want(&mut self, cx: &mut Context<Self>) {
        if self.source.wanted || self.source.pod.is_none() {
            return;
        }
        self.source.wanted = true;
        if self.source.known && self.source.state == StreamState::Idle {
            self.start(true, cx);
        }
    }

    /// Hiding the page stops the stream; showing it again reads on from the
    /// last line.
    pub(crate) fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.source.active == active {
            return;
        }
        self.source.active = active;
        if !active {
            if self.source.running() {
                self.flush_backlog(cx);
                self.source.drop_stream();
                self.source.suspended = true;
                self.source.state = StreamState::Idle;
                self.source.describe();
            }
        } else if std::mem::take(&mut self.source.suspended) {
            self.start(false, cx);
        }
    }

    /// Whether a stream is open or about to be, for the pane's tests.
    #[cfg(test)]
    pub(crate) fn streaming(&self) -> bool {
        self.source.running()
    }

    pub(super) fn choose_container(&mut self, name: String, cx: &mut Context<Self>) {
        if self.source.container.as_ref() == Some(&name)
            || self.source.containers.get(&name).is_none()
        {
            return;
        }
        self.source.container = Some(name);
        self.source.previous = false;
        self.source.derive_containers();
        self.restart(cx);
    }

    pub(super) fn set_tail(&mut self, tail: Option<i64>, cx: &mut Context<Self>) {
        if self.source.tail != tail {
            self.source.tail = tail;
            self.restart(cx);
        }
    }

    pub(super) fn set_previous(&mut self, previous: bool, cx: &mut Context<Self>) {
        if self.source.previous == previous || (previous && !self.source.has_previous()) {
            return;
        }
        self.source.previous = previous;
        self.restart(cx);
    }

    pub(super) fn set_timestamps(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.set_columns(
            Columns {
                time: shown,
                ..self.columns
            },
            cx,
        );
    }

    /// The user stops the stream; what was read stays.
    pub(super) fn stop(&mut self, cx: &mut Context<Self>) {
        if !self.source.running() {
            return;
        }
        self.flush_backlog(cx);
        self.source.drop_stream();
        self.source.state = StreamState::Stopped;
        self.source.describe();
        cx.notify();
    }

    /// Reads on from the last line, after Stop or a failure.
    pub(super) fn resume(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.source.state,
            StreamState::Stopped | StreamState::Failed(_)
        ) {
            self.start(false, cx);
        }
    }

    /// Starts a fresh view of the chosen log, once it is wanted.
    fn restart(&mut self, cx: &mut Context<Self>) {
        if self.source.wanted && self.source.known {
            self.start(true, cx);
        } else {
            self.source.drop_stream();
            self.reset_lines(&self.source.address());
            self.source.position = LogPosition::default();
            self.source.lines = 0;
            self.source.describe();
            cx.notify();
        }
    }

    /// Opens the chosen log: from the tail with a fresh view, or from the
    /// last line read.
    fn start(&mut self, fresh: bool, cx: &mut Context<Self>) {
        self.flush_backlog(cx);
        self.source.drop_stream();
        self.source.suspended = false;
        self.source.lost_at = None;
        if fresh {
            self.reset_lines(&self.source.address());
            self.source.position = LogPosition::default();
            self.source.lines = 0;
        }
        if !self.source.active {
            // Hidden: read once shown.
            self.source.suspended = true;
            self.source.state = StreamState::Idle;
            self.source.describe();
            return;
        }
        let (Some(pod), Some(container), Some(access)) = (
            self.source.pod.clone(),
            self.source.container.clone(),
            self.source.access.clone(),
        ) else {
            self.source.state = StreamState::Idle;
            self.source.describe();
            cx.notify();
            return;
        };
        self.source.state = StreamState::Connecting;
        self.source.describe();
        cx.notify();
        let stream = self.source.stream;
        let resume = self.source.position.time().map(|_| self.source.position);
        if let KubeAccess::Example = access {
            self.start_example(&pod, &container, resume.is_some(), cx);
            return;
        }
        let request = LogRequest {
            namespace: pod.namespace.clone(),
            pod: pod.name.clone(),
            container,
            previous: self.source.previous,
            tail: self.source.tail,
            resume,
        };
        let (sender, mut receiver) = mpsc::channel(STREAM_QUEUE_CAPACITY);
        let job = self.source.runtime.spawn(async move {
            match access.client().await {
                Ok(client) => follow_pod_log(client, request, sender).await,
                Err(error) => {
                    access.forget();
                    let failure = Failure::new(FailureKind::Other, error);
                    let _ = sender.send(PodLogUpdate::Failed(failure)).await;
                }
            }
        });
        self.source.job = Some(OwnedJob::new(job));
        self.source.delivery = Some(cx.spawn(async move |weak, cx| {
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
                    .update(cx, |view, cx| view.apply_updates(stream, batch, cx))
                    .unwrap_or(false);
                if !applied {
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
            }
        }));
    }

    /// Example data reads at once; a running example container writes on.
    fn start_example(
        &mut self,
        pod: &ResourceIdentity,
        container: &str,
        reading_on: bool,
        cx: &mut Context<Self>,
    ) {
        let stream = self.source.stream;
        let (mut updates, writes_on) =
            example::pod_log(pod, container, self.source.previous, live::now());
        if reading_on {
            // Example history is dated back from the clock, so read again a
            // second later it would pass for new lines. Reading on gets only
            // lines written since, and an example writes none while stopped.
            updates.retain(|update| !matches!(update, PodLogUpdate::Line(_)));
        }
        self.apply_updates(stream, updates, cx);
        if !writes_on || self.source.stream != stream {
            return;
        }
        self.source.delivery = Some(cx.spawn(async move |weak, cx| {
            for sequence in 0u64.. {
                cx.background_executor().timer(EXAMPLE_INTERVAL).await;
                let line = example::pod_log_line(sequence, Utc::now());
                let applied = weak
                    .update(cx, |view, cx| {
                        view.apply_updates(stream, vec![PodLogUpdate::Line(line)], cx)
                    })
                    .unwrap_or(false);
                if !applied {
                    return;
                }
            }
        }));
    }

    /// Applies updates from `stream`, if it is still the current one.
    /// Returns false when it isn't, which ends its delivery.
    pub(super) fn apply_updates(
        &mut self,
        stream: u64,
        updates: Vec<PodLogUpdate>,
        cx: &mut Context<Self>,
    ) -> bool {
        if stream != self.source.stream {
            return false;
        }
        let name = self.source.container.clone().unwrap_or_default();
        let mut lines = Vec::new();
        for update in updates {
            let source = &mut self.source;
            match update {
                PodLogUpdate::Line(line) => {
                    source.position.record(&line);
                    source.lines += 1;
                    lines.push(LogEvent::new(name.as_str(), line));
                }
                PodLogUpdate::Streaming => {
                    if let Some(lost) = source.lost_at.take() {
                        lines.push(source.marker(format!(
                            "Connection lost at {}, reading on from the last line",
                            clock(lost)
                        )));
                    }
                    source.state = StreamState::Streaming;
                }
                PodLogUpdate::Waiting(reason) => source.state = StreamState::Waiting(reason),
                PodLogUpdate::Reconnecting { attempt, delay, .. } => {
                    let since = *source.lost_at.get_or_insert_with(Utc::now);
                    source.state = StreamState::Reconnecting {
                        attempt,
                        delay,
                        since,
                    };
                }
                PodLogUpdate::Restarting(ended) => {
                    let how = ended
                        .map(|ended| format!(" ({})", ended))
                        .unwrap_or_default();
                    lines.push(
                        source.marker(format!("{name} restarted{how}, showing the new instance")),
                    );
                }
                PodLogUpdate::Ended(ended) => {
                    source.job = None;
                    source.state = StreamState::Ended(ended);
                }
                PodLogUpdate::Failed(failure) => {
                    source.job = None;
                    source.state = StreamState::Failed(failure);
                }
            }
        }
        self.source.describe();
        self.ingest(lines, cx);
        cx.notify();
        true
    }
}
