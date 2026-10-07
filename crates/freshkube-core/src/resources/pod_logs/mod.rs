//! A pod's containers, to choose a log from, and one container's log: the
//! previous instance's read to its end, or the current one followed across
//! dropped connections and restarts. Read-only: it gets the pod and reads
//! logs, nothing else.

use std::fmt;
use std::pin::{Pin, pin};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::io::{AsyncBufRead, AsyncBufReadExt};
use k8s_openapi::api::core::v1::Pod;
use kube::api::LogParams;
use kube::{Api, Client};
use serde_yaml::Value;
use tokio::sync::mpsc;

use super::failure::{Failure, FailureKind};
use super::kinds::builtin;
use super::object::{JSON, json_get, sequence, text, time};

/// The annotation `kubectl logs` reads for the container to show first.
const DEFAULT_CONTAINER: &str = "kubectl.kubernetes.io/default-container";
/// Longer lines are cut here; a viewer can still count them as too long.
pub const MAX_LINE_BYTES: usize = 128 * 1024;
/// Failures in a row, with no line read between them, before giving up.
pub const MAX_ATTEMPTS: u32 = 5;
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// How often a container that hasn't started is looked at again.
const MIN_WAIT: Duration = Duration::from_secs(2);
const MAX_WAIT: Duration = Duration::from_secs(10);
/// How long getting the pod, or opening its log, may take.
const REQUEST_DEADLINE: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerRole {
    Init,
    App,
    /// Added later to debug the pod.
    Ephemeral,
}

/// How one instance of a container ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Termination {
    pub exit_code: i64,
    /// Such as `Completed`, `Error` or `OOMKilled`; may be empty.
    pub reason: String,
    pub finished: Option<DateTime<Utc>>,
}

impl fmt::Display for Termination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "exit {}", self.exit_code)?;
        if !self.reason.is_empty() {
            write!(f, " ({})", self.reason)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContainerState {
    /// Not running yet, or between restarts, with the kubelet's reason
    /// (empty when the pod reports no status for it yet).
    Waiting(String),
    Running(Option<DateTime<Utc>>),
    Terminated(Termination),
}

/// When the kubelet starts a container again after it exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Restart {
    Always,
    OnFailure,
    Never,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Container {
    pub name: String,
    pub image: String,
    pub role: ContainerRole,
    pub state: ContainerState,
    pub restarts: u32,
    /// How the instance before the current one ended: the log a `previous`
    /// read returns.
    pub last_termination: Option<Termination>,
    restart: Restart,
}

impl Container {
    /// Whether it has a log yet: it ran, or runs.
    pub fn started(&self) -> bool {
        !matches!(self.state, ContainerState::Waiting(_))
            || self.last_termination.is_some()
            || self.restarts > 0
    }

    pub fn has_previous(&self) -> bool {
        self.last_termination.is_some() || self.restarts > 0
    }

    /// Whether the kubelet starts it again after this ending.
    pub fn restarts_after(&self, termination: &Termination) -> bool {
        match self.restart {
            Restart::Always => true,
            Restart::OnFailure => termination.exit_code != 0,
            Restart::Never => false,
        }
    }
}

/// A pod's containers as its spec lists them, with their status.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PodContainers {
    /// Init containers in the order they run, then the app containers, then
    /// ephemeral ones.
    pub containers: Vec<Container>,
    pub node: Option<String>,
    /// The one to show first: the `kubectl.kubernetes.io/default-container`
    /// annotation's, else the first app container.
    pub default: Option<String>,
}

impl PodContainers {
    pub fn get(&self, name: &str) -> Option<&Container> {
        self.containers
            .iter()
            .find(|container| container.name == name)
    }
}

/// The containers of a pod written out as YAML or JSON.
pub fn pod_containers(object: &Value) -> PodContainers {
    let spec = object.get("spec");
    let status = object.get("status");
    let phase = text(status.and_then(|status| status.get("phase")));
    // A pod that finished starts nothing again.
    let finished = matches!(phase.as_str(), "Succeeded" | "Failed");
    let policy = match text(spec.and_then(|spec| spec.get("restartPolicy"))).as_str() {
        _ if finished => Restart::Never,
        "Never" => Restart::Never,
        "OnFailure" => Restart::OnFailure,
        _ => Restart::Always,
    };
    let mut containers = Vec::new();
    for (role, spec_key, status_key) in [
        (
            ContainerRole::Init,
            "initContainers",
            "initContainerStatuses",
        ),
        (ContainerRole::App, "containers", "containerStatuses"),
        (
            ContainerRole::Ephemeral,
            "ephemeralContainers",
            "ephemeralContainerStatuses",
        ),
    ] {
        let statuses = sequence(status.and_then(|status| status.get(status_key)));
        for declared in sequence(spec.and_then(|spec| spec.get(spec_key))) {
            let name = text(declared.get("name"));
            let status = statuses
                .iter()
                .find(|status| text(status.get("name")) == name);
            let restart = match role {
                ContainerRole::Ephemeral => Restart::Never,
                // A sidecar: an init container that keeps running.
                ContainerRole::Init if text(declared.get("restartPolicy")) == "Always" => {
                    if finished {
                        Restart::Never
                    } else {
                        Restart::Always
                    }
                }
                ContainerRole::Init if policy == Restart::Never => Restart::Never,
                ContainerRole::Init => Restart::OnFailure,
                ContainerRole::App => policy,
            };
            containers.push(Container {
                state: status
                    .and_then(|status| state(status.get("state")))
                    .unwrap_or_else(|| ContainerState::Waiting(String::new())),
                restarts: status
                    .and_then(|status| status.get("restartCount"))
                    .and_then(Value::as_u64)
                    .map_or(0, |count| count.min(u64::from(u32::MAX)) as u32),
                last_termination: status
                    .and_then(|status| status.get("lastState"))
                    .and_then(|last| last.get("terminated"))
                    .map(termination),
                image: text(declared.get("image")),
                name,
                role,
                restart,
            });
        }
    }
    let annotated = object
        .get("metadata")
        .and_then(|metadata| metadata.get("annotations"))
        .and_then(|annotations| annotations.get(DEFAULT_CONTAINER))
        .and_then(Value::as_str)
        .filter(|name| containers.iter().any(|container| container.name == *name))
        .map(str::to_owned);
    let default = annotated.or_else(|| {
        containers
            .iter()
            .find(|container| container.role == ContainerRole::App)
            .map(|container| container.name.clone())
    });
    PodContainers {
        node: spec
            .and_then(|spec| spec.get("nodeName"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        containers,
        default,
    }
}

fn state(value: Option<&Value>) -> Option<ContainerState> {
    let value = value?;
    if let Some(terminated) = value.get("terminated") {
        return Some(ContainerState::Terminated(termination(terminated)));
    }
    if let Some(running) = value.get("running") {
        return Some(ContainerState::Running(time(running.get("startedAt"))));
    }
    value
        .get("waiting")
        .map(|waiting| ContainerState::Waiting(text(waiting.get("reason"))))
}

fn termination(value: &Value) -> Termination {
    Termination {
        exit_code: value.get("exitCode").and_then(Value::as_i64).unwrap_or(0),
        reason: text(value.get("reason")),
        finished: time(value.get("finishedAt")),
    }
}

/// How far a log was read: the last line's timestamp, and how many lines
/// carried exactly that timestamp. Resuming from it reads on without
/// repeating a line, as Kubernetes resumes only to the second.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LogPosition {
    time: Option<DateTime<Utc>>,
    at_time: usize,
}

impl LogPosition {
    pub fn time(&self) -> Option<DateTime<Utc>> {
        self.time
    }

    /// Counts a line read, as Kubernetes writes it with `timestamps=true`.
    /// A line without a timestamp leaves the position where it was.
    pub fn record(&mut self, line: &str) {
        if let Some(time) = line_time(line) {
            self.record_time(time);
        }
    }

    fn record_time(&mut self, time: DateTime<Utc>) {
        if self.time == Some(time) {
            self.at_time += 1;
        } else {
            self.time = Some(time);
            self.at_time = 1;
        }
    }
}

/// The RFC 3339 timestamp a Kubernetes log line starts with.
fn line_time(line: &str) -> Option<DateTime<Utc>> {
    let token = line.split(' ').next()?;
    DateTime::parse_from_rfc3339(token)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

/// Which log to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRequest {
    pub namespace: String,
    pub pod: String,
    pub container: String,
    /// The previous instance's log, which has ended, instead of following
    /// the current one.
    pub previous: bool,
    /// Lines from the end to start with; `None` reads the whole log.
    pub tail: Option<i64>,
    /// Read on from here instead of starting with the tail.
    pub resume: Option<LogPosition>,
}

/// What reading a log reports, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum PodLogUpdate {
    /// The container hasn't started, with the kubelet's reason; its log
    /// opens once it does.
    Waiting(String),
    /// The log is open; lines follow.
    Streaming,
    /// One line: its RFC 3339 timestamp, a space, then the text, cut to
    /// [`MAX_LINE_BYTES`].
    Line(String),
    /// The log closed while the container still runs, or couldn't be read
    /// for now. It is read again after `delay`, from the last line.
    Reconnecting {
        attempt: u32,
        delay: Duration,
        failure: Failure,
    },
    /// The instance being read ended and the kubelet starts another, whose
    /// log follows.
    Restarting(Option<Termination>),
    /// The log ended: the container exited for good, with how, or the
    /// previous instance's log was read to its end (`None`).
    Ended(Option<Termination>),
    /// Reading stopped for good: refused (403), the pod or container is
    /// gone (404), or [`MAX_ATTEMPTS`] failures in a row.
    Failed(Failure),
}

/// Reads one container's log into `sink` until it ends, fails for good, or
/// `sink` closes. Run it on Tokio and abort it to stop.
pub async fn follow_pod_log(client: Client, request: LogRequest, sink: mpsc::Sender<PodLogUpdate>) {
    let api: Api<Pod> = Api::namespaced(client.clone(), &request.namespace);
    if request.previous {
        read_previous(&api, &request, &sink).await;
        return;
    }
    let mut follower = Follower {
        client,
        api,
        position: request.resume.unwrap_or_default(),
        request,
        sink,
        failures: 0,
        backoff: MIN_BACKOFF,
    };
    while follower.read_instance().await {}
}

/// Follows the current instance of a container, and the ones after it.
struct Follower {
    client: Client,
    api: Api<Pod>,
    request: LogRequest,
    sink: mpsc::Sender<PodLogUpdate>,
    position: LogPosition,
    /// Failures in a row with no line read between them.
    failures: u32,
    backoff: Duration,
}

impl Follower {
    /// Reads the container's current instance until its log closes, or
    /// waits for one. Returns false once following is over.
    async fn read_instance(&mut self) -> bool {
        let container = match self.container().await {
            Ok(container) => container,
            Err(failure) => return self.retry(failure).await,
        };
        if !container.started() {
            return self.wait_for(None).await;
        }
        let params = LogParams {
            container: Some(self.request.container.clone()),
            follow: true,
            timestamps: true,
            tail_lines: self.request.tail.filter(|_| self.position.time.is_none()),
            since_time: self.position.time,
            ..LogParams::default()
        };
        let opened = tokio::time::timeout(
            REQUEST_DEADLINE,
            self.api.log_stream(&self.request.pod, &params),
        )
        .await;
        let reader = match opened {
            Ok(Ok(reader)) => reader,
            Ok(Err(error)) => {
                let failure = Failure::from_kube(error);
                // It stopped between getting the pod and opening its log.
                return if failure.message.contains("is waiting to start") {
                    self.wait_for(None).await
                } else {
                    self.retry(failure).await
                };
            }
            Err(_) => return self.retry(Failure::timeout("Opening the log")).await,
        };
        if self.sink.send(PodLogUpdate::Streaming).await.is_err() {
            return false;
        }
        let Some(read) = pump(reader, &mut self.position, &self.sink).await else {
            return false;
        };
        if read.lines > 0 {
            self.failures = 0;
            self.backoff = MIN_BACKOFF;
        }
        self.closed(&container, read.error).await
    }

    /// The log of `instance` closed: the container says whether that
    /// instance ended, and what follows.
    async fn closed(&mut self, instance: &Container, error: Option<Failure>) -> bool {
        let now = match self.container().await {
            Ok(now) => now,
            Err(failure) => return self.retry(failure).await,
        };
        let same = now.restarts == instance.restarts;
        if same && matches!(now.state, ContainerState::Running(_)) {
            let failure = error
                .unwrap_or_else(|| Failure::new(FailureKind::Unreachable, "The log stream closed"));
            return self.retry(failure).await;
        }
        let ended = match &now.state {
            ContainerState::Terminated(termination) if same => Some(termination.clone()),
            _ => now.last_termination.clone(),
        };
        let again = !same
            || ended
                .as_ref()
                .is_some_and(|termination| now.restarts_after(termination));
        if !again {
            let _ = self.sink.send(PodLogUpdate::Ended(ended)).await;
            return false;
        }
        if self
            .sink
            .send(PodLogUpdate::Restarting(ended))
            .await
            .is_err()
        {
            return false;
        }
        !same || self.wait_for(Some(instance.restarts)).await
    }

    /// Reports a failure, then waits out the backoff. Returns false when it
    /// shouldn't be tried again, or `sink` closed.
    async fn retry(&mut self, failure: Failure) -> bool {
        self.failures += 1;
        if failure.kind.is_permanent() || self.failures >= MAX_ATTEMPTS {
            let _ = self.sink.send(PodLogUpdate::Failed(failure)).await;
            return false;
        }
        let delay = self.backoff;
        let update = PodLogUpdate::Reconnecting {
            attempt: self.failures,
            delay,
            failure,
        };
        if self.sink.send(update).await.is_err() {
            return false;
        }
        tokio::time::sleep(delay).await;
        self.backoff = (self.backoff * 2).min(MAX_BACKOFF);
        true
    }

    /// Waits until the container has an instance to read: once it has
    /// started, or, given the restarts it had, once it has started again.
    /// Returns false when following is over.
    async fn wait_for(&mut self, restarted_from: Option<u32>) -> bool {
        let mut wait = MIN_WAIT;
        let mut said = None;
        loop {
            let container = match self.container().await {
                Ok(container) => container,
                Err(failure) => {
                    if !self.retry(failure).await {
                        return false;
                    }
                    continue;
                }
            };
            let ready = match restarted_from {
                Some(restarts) => container.restarts > restarts,
                None => container.started(),
            };
            if ready {
                return true;
            }
            let reason = match &container.state {
                ContainerState::Terminated(termination) => {
                    if !container.restarts_after(termination) {
                        let ended = PodLogUpdate::Ended(Some(termination.clone()));
                        let _ = self.sink.send(ended).await;
                        return false;
                    }
                    "Exited".to_owned()
                }
                ContainerState::Waiting(reason) if !reason.is_empty() => reason.clone(),
                _ => "Not started".to_owned(),
            };
            if said.as_ref() != Some(&reason) {
                let update = PodLogUpdate::Waiting(reason.clone());
                if self.sink.send(update).await.is_err() {
                    return false;
                }
                said = Some(reason);
            }
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(MAX_WAIT);
        }
    }

    /// Gets the pod and finds the container in it.
    async fn container(&self) -> Result<Container, Failure> {
        let pod = read_pod(&self.client, &self.request.namespace, &self.request.pod).await?;
        let name = &self.request.container;
        pod_containers(&pod)
            .containers
            .into_iter()
            .find(|container| &container.name == name)
            .ok_or_else(|| {
                Failure::new(
                    FailureKind::NotFound,
                    format!("The pod has no container named {name}"),
                )
            })
    }
}

/// Reads the previous instance's log to its end.
async fn read_previous(api: &Api<Pod>, request: &LogRequest, sink: &mpsc::Sender<PodLogUpdate>) {
    let params = LogParams {
        container: Some(request.container.clone()),
        previous: true,
        timestamps: true,
        tail_lines: request.tail,
        ..LogParams::default()
    };
    let opened =
        tokio::time::timeout(REQUEST_DEADLINE, api.log_stream(&request.pod, &params)).await;
    let reader = match opened {
        Ok(Ok(reader)) => reader,
        Ok(Err(error)) => {
            let _ = sink
                .send(PodLogUpdate::Failed(Failure::from_kube(error)))
                .await;
            return;
        }
        Err(_) => {
            let failure = Failure::timeout("Opening the log");
            let _ = sink.send(PodLogUpdate::Failed(failure)).await;
            return;
        }
    };
    if sink.send(PodLogUpdate::Streaming).await.is_err() {
        return;
    }
    let mut position = LogPosition::default();
    let Some(read) = pump(reader, &mut position, sink).await else {
        return;
    };
    let update = match read.error {
        Some(failure) => PodLogUpdate::Failed(failure),
        None => PodLogUpdate::Ended(None),
    };
    let _ = sink.send(update).await;
}

/// What one open log gave before it closed.
struct Read {
    /// New lines, not counting any skipped as already read.
    lines: usize,
    /// Why it closed, when it broke rather than ended.
    error: Option<Failure>,
}

/// Sends each new line of `reader` to `sink`, skipping lines at or before
/// `position`, and advances it. `None` when `sink` closed.
async fn pump(
    reader: impl AsyncBufRead,
    position: &mut LogPosition,
    sink: &mpsc::Sender<PodLogUpdate>,
) -> Option<Read> {
    let mut reader = pin!(reader);
    let resumed_from = *position;
    let mut skipped_at_time = 0;
    let mut lines = 0;
    let mut line = Vec::new();
    loop {
        match next_line(reader.as_mut(), &mut line).await {
            Ok(false) => return Some(Read { lines, error: None }),
            Ok(true) => {}
            Err(error) => {
                let error = Some(Failure::new(FailureKind::Unreachable, error.to_string()));
                return Some(Read { lines, error });
            }
        }
        let text = String::from_utf8_lossy(&line).into_owned();
        if let Some(time) = line_time(&text) {
            // Kubernetes resumes to the second: drop what was read before.
            if let Some(from) = resumed_from.time {
                if time < from {
                    continue;
                }
                if time == from && skipped_at_time < resumed_from.at_time {
                    skipped_at_time += 1;
                    continue;
                }
            }
            position.record_time(time);
        }
        lines += 1;
        if sink.send(PodLogUpdate::Line(text)).await.is_err() {
            return None;
        }
    }
}

/// Reads one line into `line`, without its line ending and cut to
/// [`MAX_LINE_BYTES`]. Returns false at the end of the log.
async fn next_line<R: AsyncBufRead>(
    mut reader: Pin<&mut R>,
    line: &mut Vec<u8>,
) -> std::io::Result<bool> {
    line.clear();
    let mut any = false;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(any);
        }
        any = true;
        let (chunk, used, done) = match available.iter().position(|byte| *byte == b'\n') {
            Some(end) => (&available[..end], end + 1, true),
            None => (available, available.len(), false),
        };
        let room = MAX_LINE_BYTES.saturating_sub(line.len());
        line.extend_from_slice(&chunk[..chunk.len().min(room)]);
        reader.consume_unpin(used);
        if done {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(true);
        }
    }
}

/// Reads one pod in full, within the request deadline.
pub(crate) async fn read_pod(
    client: &Client,
    namespace: &str,
    pod: &str,
) -> Result<Value, Failure> {
    let kind = builtin("pods").expect("pods are built in");
    let request = json_get(kind.object_path(Some(namespace), pod), JSON)?;
    tokio::time::timeout(REQUEST_DEADLINE, client.request(request))
        .await
        .map_err(|_| Failure::timeout("Reading the pod"))?
        .map_err(Failure::from_kube)
}

#[cfg(test)]
mod tests;
