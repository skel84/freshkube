//! Many container logs read into one view at once: each container's
//! stream, the channel they all feed, and their retries. A workload's tab
//! reads every container of its pods this way.
//!
//! Every stream feeds one channel, which one delivery task drains a frame
//! at a time into a single `ingest`, however many containers write; the
//! view places each line by its time among the retained lines. A stream
//! that fails is read again with the watcher's backoff, jittered, while
//! its container is still wanted. One refused for good (403, or 404) waits
//! instead until its source reads it again.
//!
//! A source keeps a [`StreamSet`] and says, through [`StreamSource`], what
//! its lines are tagged and how far back they start; [`StreamReads`] does
//! the rest on any `LogView` of such a source.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use freshkube_core::logs::{LogEvent, ServiceId};
use freshkube_core::resources::{
    Failure, FailureKind, LogPosition, LogRequest, PodLogUpdate, follow_pod_log,
};
use gpui_kit::{App, Context, Task};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use super::{LogSource, LogView};
use crate::backend::{OwnedJob, STREAM_QUEUE_CAPACITY};
use crate::resources::model::ResourceIdentity;
use crate::resources::{KubeAccess, example};
use crate::ui::Tone;

/// How often an example container writes another line; each stream adds
/// a tick or more, so they don't all write at once.
pub(super) const EXAMPLE_INTERVAL: Duration = Duration::from_millis(1_500);
/// The example writer's tick.
const EXAMPLE_TICK: Duration = Duration::from_millis(250);
/// How often the delivery task hands what the streams sent to the view.
const DELIVERY_INTERVAL: Duration = Duration::from_millis(16);
/// A failed stream's first retry, doubling up to the last, as the pod
/// watch backs off.
pub(super) const RETRY_FIRST: Duration = Duration::from_secs(1);
pub(super) const RETRY_MAX: Duration = Duration::from_secs(30);
/// How far a retry's wait strays either way, as a share of it, so streams
/// that fail together don't all read again at once.
pub(super) const RETRY_JITTER: f64 = 0.2;

/// Spreads retry waits by up to [`RETRY_JITTER`] either way: random in the
/// app and seeded in tests, so a test draws the same waits from a clone.
#[derive(Clone, Debug)]
pub(super) struct Jitter(fastrand::Rng);

impl Jitter {
    pub(super) fn new() -> Self {
        #[cfg(test)]
        let rng = fastrand::Rng::with_seed(0x5EED);
        #[cfg(not(test))]
        let rng = fastrand::Rng::new();
        Self(rng)
    }

    /// `wait`, scaled by the next factor in `1 ± RETRY_JITTER`.
    pub(super) fn spread(&mut self, wait: Duration) -> Duration {
        wait.mul_f64(1. - RETRY_JITTER + 2. * RETRY_JITTER * self.0.f64())
    }
}

/// One container's log: which pod incarnation and container.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct StreamKey {
    pub(super) pod: String,
    pub(super) uid: String,
    pub(super) container: String,
}

impl StreamKey {
    /// The `pod/container` tag a workload's lines carry, by which they are
    /// filtered.
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

    /// Refused for good (403, or 404): read again only when the source asks,
    /// never on a timer.
    pub(super) fn refused(&self) -> bool {
        matches!(self, StreamState::Failed(failure) if failure.kind.is_permanent())
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

pub(super) struct Stream {
    pub(super) state: StreamState,
    /// The pod, for a retry and the example data's log.
    pub(super) pod: ResourceIdentity,
    /// Advances with each read; updates from an older one are dropped.
    pub(super) generation: u64,
    /// What this read asked for, `resume` included.
    pub(super) request: LogRequest,
    job: Option<OwnedJob>,
    /// The next read after a failure, once its backoff passes.
    pub(super) retry: Option<Task<()>>,
    /// An example container that goes on writing, and its next line.
    writes_on: bool,
    sequence: u64,
}

/// One update from one container's read, as the shared channel carries it.
pub(super) struct Fed {
    pub(super) key: StreamKey,
    pub(super) generation: u64,
    pub(super) update: PodLogUpdate,
}

/// The channel every stream sends into, and the one task that drains it.
struct Feed {
    sender: mpsc::Sender<Fed>,
    _delivery: Task<()>,
}

/// Wall time by the executor's clock: anchored once, then stepped by it,
/// so tests advance it with the clock instead of sleeping.
#[derive(Clone, Copy)]
pub(super) struct Clock {
    started: Instant,
    at: DateTime<Utc>,
}

impl Clock {
    pub(super) fn new(cx: &App) -> Self {
        Self {
            started: cx.background_executor().now(),
            at: Utc::now(),
        }
    }

    pub(super) fn now(&self, cx: &App) -> DateTime<Utc> {
        let elapsed = cx
            .background_executor()
            .now()
            .saturating_duration_since(self.started);
        self.at + TimeDelta::from_std(elapsed).unwrap_or_default()
    }
}

/// The streams a source reads, and what each keeps between reads.
pub(super) struct StreamSet {
    pub(super) runtime: Handle,
    pub(super) clock: Clock,
    pub(super) streams: BTreeMap<StreamKey, Stream>,
    feed: Option<Feed>,
    /// Writes the example containers' lines while any stream writes on.
    example_writer: Option<Task<()>>,
    /// Failures in a row of each container, for its retry's backoff.
    pub(super) failures: BTreeMap<StreamKey, u32>,
    /// Spreads the retries' waits.
    pub(super) jitter: Jitter,
    /// How far each container was read, kept while the page hides so its
    /// log reads on without repeating a line.
    pub(super) positions: BTreeMap<StreamKey, LogPosition>,
    next_generation: u64,
    /// Failed streams, shown above the lines while the others go on.
    pub(super) errors: BTreeMap<ServiceId, String>,
}

impl StreamSet {
    pub(super) fn new(runtime: Handle, clock: Clock) -> Self {
        Self {
            runtime,
            clock,
            streams: BTreeMap::new(),
            feed: None,
            example_writer: None,
            failures: BTreeMap::new(),
            jitter: Jitter::new(),
            positions: BTreeMap::new(),
            next_generation: 0,
            errors: BTreeMap::new(),
        }
    }

    /// Drops every stream, its feed and the example writer, keeping how
    /// far each was read.
    pub(super) fn drop_streams(&mut self) {
        self.streams.clear();
        self.feed = None;
        self.example_writer = None;
    }

    /// Streams refused for good, with their pods, to read again.
    pub(super) fn refused_streams(&self) -> Vec<(StreamKey, ResourceIdentity)> {
        self.streams
            .iter()
            .filter(|(_, stream)| stream.state.refused())
            .map(|(key, stream)| (key.clone(), stream.pod.clone()))
            .collect()
    }
}

/// A source whose lines come from a [`StreamSet`].
pub(super) trait StreamSource: LogSource + Sized + 'static {
    /// The render probe each delivery hits.
    const APPLY_PROBE: &'static str;

    fn reads(&self) -> &StreamSet;

    fn reads_mut(&mut self) -> &mut StreamSet;

    fn access(&self) -> Option<KubeAccess>;

    /// The tag a container's lines carry.
    fn tag(key: &StreamKey) -> ServiceId;

    /// Lines from the end a new read starts with; `None` is all.
    fn tail(&self) -> Option<i64>;

    /// Whether each container's previous instance is read.
    fn previous(&self) -> bool;

    /// Whether the container is still one to read, so its retry goes on.
    fn listed(&self, key: &StreamKey) -> bool;

    /// A stream's state changed: derive what shows it again.
    fn streams_changed(&mut self);

    /// A stream started, before its first line.
    fn stream_started(view: &mut LogView<Self>, key: &StreamKey);
}

/// Reading a source's streams into its view.
pub(super) trait StreamReads: Sized + 'static {
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
    /// one notify, and what shows the streams derived again only when a
    /// stream's state changed.
    fn apply_fed(&mut self, batch: Vec<Fed>, cx: &mut Context<Self>);

    /// Reads a failed container again once its backoff passes.
    fn schedule_retry(&mut self, key: StreamKey, generation: u64, cx: &mut Context<Self>);

    /// The retry itself, while the stream still failed and its container
    /// is listed.
    fn retry_stream(&mut self, key: StreamKey, generation: u64, cx: &mut Context<Self>);

    /// Example data reads at once; a running example container writes on.
    fn start_example(&mut self, key: StreamKey, generation: u64, cx: &mut Context<Self>);

    /// One tick of the example writer: each running example container
    /// writes at its own pace, into the feed.
    fn example_tick(&mut self, tick: u64, cx: &mut Context<Self>);
}

impl<S: StreamSource> StreamReads for LogView<S> {
    fn start_stream(&mut self, key: StreamKey, pod: ResourceIdentity, cx: &mut Context<Self>) {
        let Some(access) = self.source().access() else {
            return;
        };
        let resume = self
            .source()
            .reads()
            .positions
            .get(&key)
            .filter(|position| position.time().is_some())
            .copied();
        let request = LogRequest {
            namespace: pod.namespace.clone(),
            pod: key.pod.clone(),
            container: key.container.clone(),
            previous: self.source().previous(),
            tail: self.source().tail(),
            resume,
        };
        let reads = self.source_mut().reads_mut();
        reads.next_generation += 1;
        let generation = reads.next_generation;
        reads.streams.insert(
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
        S::stream_started(self, &key);
        if let KubeAccess::Example = access {
            self.start_example(key, generation, cx);
            return;
        }
        let feed = self.feed(cx);
        let stream_key = key.clone();
        let job = self.source().reads().runtime.spawn(async move {
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
        if let Some(stream) = self.source_mut().reads_mut().streams.get_mut(&key) {
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
            .reads()
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
        if let Some(feed) = &self.source().reads().feed {
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
        self.source_mut().reads_mut().feed = Some(Feed {
            sender: sender.clone(),
            _delivery: delivery,
        });
        sender
    }

    fn apply_fed(&mut self, batch: Vec<Fed>, cx: &mut Context<Self>) {
        crate::desktop::probe::hit(S::APPLY_PROBE);
        let now = self.source().reads().clock.now(cx);
        let mut lines = Vec::new();
        let mut changed = false;
        let mut failed = Vec::new();
        let reads = self.source_mut().reads_mut();
        for Fed {
            key,
            generation,
            update,
        } in batch
        {
            let Some(stream) = reads
                .streams
                .get_mut(&key)
                .filter(|stream| stream.generation == generation)
            else {
                continue;
            };
            let service = S::tag(&key);
            let position = reads.positions.entry(key.clone()).or_default();
            match update {
                PodLogUpdate::Line(line) => {
                    position.record(&line);
                    lines.push(LogEvent::new(service, line));
                }
                PodLogUpdate::Streaming => {
                    stream.state = StreamState::Streaming;
                    reads.errors.remove(&service);
                    reads.failures.remove(&key);
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
                    reads.errors.insert(service, failure.to_string());
                    stream.state = StreamState::Failed(failure);
                    // A refusal waits for the source to read it again.
                    if !stream.state.refused() {
                        failed.push((key, generation));
                    }
                    changed = true;
                }
            }
        }
        for (key, generation) in failed {
            self.schedule_retry(key, generation, cx);
        }
        if changed {
            self.source_mut().streams_changed();
        }
        if !lines.is_empty() {
            self.ingest(lines, cx);
        }
        cx.notify();
    }

    fn schedule_retry(&mut self, key: StreamKey, generation: u64, cx: &mut Context<Self>) {
        let reads = self.source_mut().reads_mut();
        let failures = reads.failures.entry(key.clone()).or_default();
        *failures += 1;
        let wait = RETRY_FIRST
            .saturating_mul(1 << (*failures - 1).min(5))
            .min(RETRY_MAX);
        let delay = reads.jitter.spread(wait);
        let retry_key = key.clone();
        let task = cx.spawn(async move |weak, cx| {
            cx.background_executor().timer(delay).await;
            let _ = weak.update(cx, |view, cx| view.retry_stream(retry_key, generation, cx));
        });
        if let Some(stream) = self.source_mut().reads_mut().streams.get_mut(&key) {
            stream.retry = Some(task);
        }
    }

    fn retry_stream(&mut self, key: StreamKey, generation: u64, cx: &mut Context<Self>) {
        let listed = self.source().listed(&key);
        let reads = self.source_mut().reads_mut();
        let Some(stream) = reads
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
        // A refusal schedules no retry; should one still fire, it waits for
        // the source to read it again too.
        if !matches!(stream.state, StreamState::Failed(_)) || stream.state.refused() || !listed {
            return;
        }
        let pod = stream.pod.clone();
        self.start_stream(key, pod, cx);
        self.source_mut().streams_changed();
        cx.notify();
    }

    fn start_example(&mut self, key: StreamKey, generation: u64, cx: &mut Context<Self>) {
        let Some((pod, reading_on)) = self
            .source()
            .reads()
            .streams
            .get(&key)
            .map(|stream| (stream.pod.clone(), stream.request.resume.is_some()))
        else {
            return;
        };
        let now = self.source().reads().clock.now(cx).timestamp();
        let previous = self.source().previous();
        let (mut updates, writes_on) = example::pod_log(&pod, &key.container, previous, now);
        if reading_on {
            // Example history is dated back from the clock, so read again it
            // would pass for new lines. Reading on gets only lines written
            // since.
            updates.retain(|update| !matches!(update, PodLogUpdate::Line(_)));
        }
        if let Some(stream) = self.source_mut().reads_mut().streams.get_mut(&key) {
            stream.writes_on = writes_on;
        }
        self.apply_updates(&key, generation, updates, cx);
        if writes_on && self.source().reads().example_writer.is_none() {
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
            self.source_mut().reads_mut().example_writer = Some(writer);
        }
    }

    fn example_tick(&mut self, tick: u64, cx: &mut Context<Self>) {
        let now = self.source().reads().clock.now(cx);
        let feed = self.feed(cx);
        let base = EXAMPLE_INTERVAL.as_millis() as u64 / EXAMPLE_TICK.as_millis() as u64;
        for (key, stream) in &mut self.source_mut().reads_mut().streams {
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
