//! An interactive shell in one container, as `kubectl exec -it` runs it. It
//! is the one write the browsing pages make: it creates a `pods/exec`, and
//! only when the user starts a session (docs/POD_EXEC.md).
//!
//! [`start_exec`] checks the pod, opens the exec and sends the terminal's
//! size before anything else, then returns a session: input in, output out,
//! and one [`ExecEnd`] that says why it stopped.

use std::fmt;
use std::time::Duration;

use futures::SinkExt;
use futures::channel::mpsc as size_channel;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;
use kube::api::{AttachParams, TerminalSize};
use kube::client::UpgradeConnectionError;
use kube::{Api, Client};
use serde_yaml::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use super::failure::{Failure, FailureKind};
use super::kinds::builtin;
use super::object::{JSON, json_get, text};
use super::pod_logs::{ContainerState, pod_containers};

/// Runs the first shell the container has, so its exit ends the session.
pub const SHELL: &str = "command -v bash >/dev/null && exec bash; \
    command -v ash >/dev/null && exec ash; exec sh";
/// Output goes out in batches of at most this many bytes…
pub const BATCH_BYTES: usize = 64 * 1024;
/// …or whatever arrived this long after a batch's first byte.
pub const BATCH_TIME: Duration = Duration::from_millis(8);
/// How long getting the pod, or opening the exec, may take.
const REQUEST_DEADLINE: Duration = Duration::from_secs(30);
/// How long the server may take to close after the user ends a session.
const CLOSE_DEADLINE: Duration = Duration::from_secs(2);
/// Between Control-C and Control-D when a session ends, so the shell is
/// back at its prompt when Control-D arrives.
const INTERRUPT_PAUSE: Duration = Duration::from_millis(200);
const INPUT_QUEUE: usize = 256;
const OUTPUT_QUEUE: usize = 64;

/// A terminal's size in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecSize {
    pub columns: u16,
    pub rows: u16,
}

/// Which container to run a shell in. The UID makes sure it is still the
/// pod the user chose, not a new one with the same name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecRequest {
    pub namespace: String,
    pub pod: String,
    pub uid: String,
    pub container: String,
    /// The terminal's size, sent before any input.
    pub size: ExecSize,
}

/// What goes to the shell, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecInput {
    Bytes(Vec<u8>),
    /// The terminal changed size. A newer one queued right behind it
    /// replaces it.
    Resize(ExecSize),
}

/// What comes from the shell, in order: byte batches, then one end.
#[derive(Clone, Debug, PartialEq)]
pub enum ExecOutput {
    /// Up to [`BATCH_BYTES`]; with a TTY, stderr arrives here too.
    Bytes(Vec<u8>),
    End(ExecEnd),
}

/// Why a session stopped.
#[derive(Clone, Debug, PartialEq)]
pub enum ExecEnd {
    /// The shell exited with this code.
    Exited(i32),
    /// The pod was deleted or replaced, or the container stopped or
    /// restarted.
    PodGone(String),
    /// The connection dropped while the container still runs.
    Disconnected(Failure),
    /// The user ended it by closing the input.
    Closed,
    /// The exec itself failed once open, such as a container with no shell.
    Failed(ExecFailure),
}

/// What kind of failure starting a shell hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecFailureKind {
    /// The request failed as a read would: refused, not found, unreachable.
    Request(FailureKind),
    /// The container exists but isn't running.
    NotRunning,
    /// The container has no shell to run.
    NoShell,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecFailure {
    pub kind: ExecFailureKind,
    pub message: String,
}

impl ExecFailure {
    pub fn new(kind: ExecFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Retrying won't help until something outside the app changes.
    pub fn is_permanent(&self) -> bool {
        match self.kind {
            ExecFailureKind::Request(kind) => kind.is_permanent(),
            ExecFailureKind::NotRunning => false,
            ExecFailureKind::NoShell => true,
        }
    }

    /// Classifies a failed exec request. A refused upgrade carries only its
    /// status code: kube drops the body.
    fn from_kube(error: kube::Error) -> Self {
        let failure = match error {
            kube::Error::UpgradeConnection(UpgradeConnectionError::ProtocolSwitch(status)) => {
                let kind = match status.as_u16() {
                    401 => FailureKind::Unauthorized,
                    403 => FailureKind::Forbidden,
                    404 => FailureKind::NotFound,
                    408 | 504 => FailureKind::Timeout,
                    _ => FailureKind::Other,
                };
                let message = match kind {
                    FailureKind::Forbidden => "Not allowed to run a shell in this pod".to_owned(),
                    FailureKind::NotFound => "The pod or container was not found".to_owned(),
                    _ => format!("The API server refused the shell ({status})"),
                };
                Failure::new(kind, message)
            }
            kube::Error::UpgradeConnection(error) => {
                Failure::new(FailureKind::Unreachable, error.to_string())
            }
            other => Failure::from_kube(other),
        };
        failure.into()
    }
}

impl From<Failure> for ExecFailure {
    fn from(failure: Failure) -> Self {
        Self::new(ExecFailureKind::Request(failure.kind), failure.message)
    }
}

impl fmt::Display for ExecFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ExecFailureKind::Request(kind) => Failure::new(kind, self.message.clone()).fmt(f),
            ExecFailureKind::NotRunning => write!(f, "Not running · {}", self.message),
            ExecFailureKind::NoShell => write!(f, "No shell · {}", self.message),
        }
    }
}

impl std::error::Error for ExecFailure {}

/// An open shell. Closing `input` ends it politely, with
/// [`ExecEnd::Closed`]; so does [`ExecGuard::close`], whether or not anyone
/// still reads the output. Dropping `guard` stops it at once.
///
/// Politely means as a user at the keyboard would: Control-C stops what runs
/// in the foreground, and Control-D then ends the shell at its prompt.
/// Closing the connection alone stops neither: the container runtime keeps
/// the terminal open, so both would run on in the container. A program that
/// ignores those keys, such as an open editor, still does.
pub struct ExecSession {
    pub input: mpsc::Sender<ExecInput>,
    pub output: mpsc::Receiver<ExecOutput>,
    pub guard: ExecGuard,
}

/// Owns a session's work on Tokio.
pub struct ExecGuard {
    task: Option<JoinHandle<()>>,
    close: Option<oneshot::Sender<()>>,
}

impl ExecGuard {
    /// Ends the session politely in the background and lets it finish on
    /// its own, within a couple of seconds. The handle completes when it
    /// has; dropping the handle doesn't stop it.
    pub fn close(mut self) -> JoinHandle<()> {
        if let Some(close) = self.close.take() {
            let _ = close.send(());
        }
        self.task
            .take()
            .expect("a guard owns its task until closed")
    }
}

impl Drop for ExecGuard {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Starts a shell in the container. Returns once the API server has
/// accepted the exec and the size has gone out, never before. Run it on
/// Tokio.
pub async fn start_exec(client: Client, request: ExecRequest) -> Result<ExecSession, ExecFailure> {
    let restarts = match check_container(&client, &request).await? {
        Check::Running { restarts } => restarts,
        Check::Gone(reason) => {
            return Err(ExecFailure::new(
                ExecFailureKind::Request(FailureKind::NotFound),
                reason,
            ));
        }
        Check::NotRunning(reason) => {
            return Err(ExecFailure::new(ExecFailureKind::NotRunning, reason));
        }
    };
    let api: Api<Pod> = Api::namespaced(client.clone(), &request.namespace);
    let params = AttachParams {
        container: Some(request.container.clone()),
        max_stdin_buf_size: Some(BATCH_BYTES),
        max_stdout_buf_size: Some(BATCH_BYTES),
        ..AttachParams::interactive_tty()
    };
    let mut process = tokio::time::timeout(
        REQUEST_DEADLINE,
        api.exec(&request.pod, ["sh", "-c", SHELL], &params),
    )
    .await
    .map_err(|_| ExecFailure::from(Failure::timeout("Starting the shell")))?
    .map_err(ExecFailure::from_kube)?;
    let lost = || ExecFailure::from(Failure::new(FailureKind::Unreachable, "The exec closed"));
    let mut sizes = process.terminal_size().ok_or_else(lost)?;
    sizes
        .send(terminal_size(request.size))
        .await
        .map_err(|_| lost())?;
    let stdin = process.stdin().ok_or_else(lost)?;
    let stdout = process.stdout().ok_or_else(lost)?;
    let status = process.take_status().ok_or_else(lost)?;
    let (input, input_queue) = mpsc::channel(INPUT_QUEUE);
    let (output_sink, output) = mpsc::channel(OUTPUT_QUEUE);
    let (close, closing) = oneshot::channel();
    let task = tokio::spawn(async move {
        let streams = Streams {
            input: input_queue,
            stdin,
            sizes,
            stdout,
            output: output_sink.clone(),
            close: closing,
        };
        let Some(ran) = streams.run().await else {
            process.abort();
            return;
        };
        let end = if ran.closed {
            process.abort();
            ExecEnd::Closed
        } else {
            match status.await {
                Some(status) => classify_status(&status, ran.output_seen),
                None => {
                    let broke = process.join().await.err().map(|error| error.to_string());
                    ended_without_status(&client, &request, restarts, broke).await
                }
            }
        };
        let _ = output_sink.send(ExecOutput::End(end)).await;
    });
    Ok(ExecSession {
        input,
        output,
        guard: ExecGuard {
            task: Some(task),
            close: Some(close),
        },
    })
}

fn terminal_size(size: ExecSize) -> TerminalSize {
    TerminalSize {
        width: size.columns,
        height: size.rows,
    }
}

/// The two directions of one open exec.
struct Streams<W, R> {
    input: mpsc::Receiver<ExecInput>,
    stdin: W,
    sizes: size_channel::Sender<TerminalSize>,
    stdout: R,
    output: mpsc::Sender<ExecOutput>,
    /// The guard asks to end politely.
    close: oneshot::Receiver<()>,
}

/// Why the streams stop moving bytes both ways.
enum Stop {
    /// The user or the guard asked to end.
    Close,
    /// The exec stopped taking input.
    Broken,
    /// The output ended, or `None` when nobody reads it any more.
    Read(Option<bool>),
}

/// How the streams stopped.
struct Ran {
    /// The user closed the input.
    closed: bool,
    /// The shell wrote something, so it started.
    output_seen: bool,
}

impl<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> Streams<W, R> {
    /// Moves bytes both ways until the output ends, or the user closes the
    /// input or the guard asks to close, which ends the session politely.
    /// `None` when nobody reads the output any more.
    async fn run(self) -> Option<Ran> {
        let Streams {
            input,
            mut stdin,
            sizes,
            mut stdout,
            output,
            mut close,
        } = self;
        let mut reading = Box::pin(read_output(&mut stdout, &output));
        let mut writing = Box::pin(write_input(input, &mut stdin, sizes));
        let stop = tokio::select! {
            biased;
            _ = &mut close => Stop::Close,
            closed = &mut writing => if closed { Stop::Close } else { Stop::Broken },
            read = &mut reading => Stop::Read(read),
        };
        drop(writing);
        match stop {
            Stop::Read(Some(output_seen)) => Some(Ran {
                closed: false,
                output_seen,
            }),
            // The output says why the exec stopped taking input.
            Stop::Broken => reading.await.map(|output_seen| Ran {
                closed: false,
                output_seen,
            }),
            Stop::Close => {
                // Shows what the shell writes as it ends, and stops once it
                // has exited, or at the deadline.
                let deadline = tokio::time::Instant::now() + INTERRUPT_PAUSE + CLOSE_DEADLINE;
                let (_, read) = tokio::join!(
                    end_politely(&mut stdin),
                    tokio::time::timeout_at(deadline, &mut reading),
                );
                match read {
                    Ok(Some(output_seen)) => Some(Ran {
                        closed: true,
                        output_seen,
                    }),
                    Err(_) => Some(Ran {
                        closed: true,
                        output_seen: true,
                    }),
                    Ok(None) => {
                        drop(reading);
                        let _ = tokio::time::timeout_at(deadline, drain(&mut stdout)).await;
                        None
                    }
                }
            }
            Stop::Read(None) => {
                drop(reading);
                let deadline = tokio::time::Instant::now() + INTERRUPT_PAUSE + CLOSE_DEADLINE;
                let _ = tokio::time::timeout_at(deadline, async {
                    tokio::join!(end_politely(&mut stdin), drain(&mut stdout))
                })
                .await;
                None
            }
        }
    }
}

/// Ends the shell as a user at the keyboard would: Control-C, then
/// Control-D once the shell is back at its prompt.
async fn end_politely(mut stdin: impl AsyncWrite + Unpin) {
    if stdin.write_all(b"\x03").await.is_err() || stdin.flush().await.is_err() {
        return;
    }
    tokio::time::sleep(INTERRUPT_PAUSE).await;
    let _ = stdin.write_all(b"\x04").await;
    let _ = stdin.flush().await;
}

/// Reads the output to its end, for nobody.
async fn drain(mut stdout: impl AsyncRead + Unpin) {
    let mut buffer = [0; 4096];
    while matches!(stdout.read(&mut buffer).await, Ok(read) if read > 0) {}
}

/// Writes queued input in order. Returns true once the user closed the
/// input, false when the exec stopped taking it.
async fn write_input(
    mut input: mpsc::Receiver<ExecInput>,
    mut stdin: impl AsyncWrite + Unpin,
    mut sizes: size_channel::Sender<TerminalSize>,
) -> bool {
    let mut queued = Vec::new();
    loop {
        let Some(first) = input.recv().await else {
            return true;
        };
        queued.push(first);
        while let Ok(next) = input.try_recv() {
            queued.push(next);
        }
        for item in coalesce(std::mem::take(&mut queued)) {
            let written = match item {
                ExecInput::Bytes(bytes) => stdin.write_all(&bytes).await.is_ok(),
                ExecInput::Resize(size) => sizes.send(terminal_size(size)).await.is_ok(),
            };
            if !written {
                return false;
            }
        }
    }
}

/// Joins neighbouring byte chunks, and keeps only the last of neighbouring
/// resizes. Order is kept otherwise, so bytes typed before a resize still
/// reach the shell at the old size.
pub(crate) fn coalesce(items: Vec<ExecInput>) -> Vec<ExecInput> {
    let mut joined: Vec<ExecInput> = Vec::with_capacity(items.len());
    for item in items {
        match (joined.last_mut(), item) {
            (Some(ExecInput::Bytes(last)), ExecInput::Bytes(bytes)) => last.extend(bytes),
            (Some(ExecInput::Resize(last)), ExecInput::Resize(size)) => *last = size,
            (_, item) => joined.push(item),
        }
    }
    joined
}

/// Sends the shell's output in batches of up to [`BATCH_BYTES`], or what
/// arrived within [`BATCH_TIME`] of a batch's first byte. Returns whether
/// any output came, or `None` when `sink` closed.
pub(crate) async fn read_output(
    mut stdout: impl AsyncRead + Unpin,
    sink: &mpsc::Sender<ExecOutput>,
) -> Option<bool> {
    let mut seen = false;
    let mut buffer = vec![0; BATCH_BYTES];
    loop {
        let mut filled = match stdout.read(&mut buffer).await {
            Ok(0) | Err(_) => return Some(seen),
            Ok(read) => read,
        };
        seen = true;
        let deadline = tokio::time::Instant::now() + BATCH_TIME;
        let mut ended = false;
        while filled < BATCH_BYTES {
            match tokio::time::timeout_at(deadline, stdout.read(&mut buffer[filled..])).await {
                Err(_) => break,
                Ok(Ok(0) | Err(_)) => {
                    ended = true;
                    break;
                }
                Ok(Ok(read)) => filled += read,
            }
        }
        let batch = ExecOutput::Bytes(buffer[..filled].to_vec());
        if sink.send(batch).await.is_err() {
            return None;
        }
        if ended {
            return Some(seen);
        }
    }
}

/// How an exec's final status reads. A shell that wrote nothing and exits
/// 126 or 127 never started: the fallback chain found no shell to run.
pub(crate) fn classify_status(status: &Status, output_seen: bool) -> ExecEnd {
    if status.status.as_deref() == Some("Success") {
        return ExecEnd::Exited(0);
    }
    let message = status.message.clone().unwrap_or_default();
    let code = status
        .details
        .as_ref()
        .and_then(|details| details.causes.as_ref())
        .into_iter()
        .flatten()
        .find(|cause| cause.reason.as_deref() == Some("ExitCode"))
        .and_then(|cause| cause.message.as_deref()?.trim().parse::<i32>().ok());
    let missing = ["executable file not found", "no such file or directory"]
        .iter()
        .any(|text| message.to_lowercase().contains(text));
    match code {
        Some(126 | 127) if !output_seen => ExecEnd::Failed(no_shell(&message)),
        Some(code) => ExecEnd::Exited(code),
        None if missing => ExecEnd::Failed(no_shell(&message)),
        None => ExecEnd::Failed(ExecFailure::new(
            ExecFailureKind::Request(FailureKind::Other),
            if message.is_empty() {
                "The shell failed".to_owned()
            } else {
                message
            },
        )),
    }
}

fn no_shell(message: &str) -> ExecFailure {
    let message = if message.is_empty() {
        "The container has no sh to run".to_owned()
    } else {
        message.to_owned()
    };
    ExecFailure::new(ExecFailureKind::NoShell, message)
}

/// What the pod says about the container.
#[derive(Debug, PartialEq, Eq)]
enum Check {
    Running {
        restarts: u32,
    },
    /// The container exists but doesn't run, with why.
    NotRunning(String),
    /// The pod was deleted or replaced, or has no such container.
    Gone(String),
}

async fn check_container(client: &Client, request: &ExecRequest) -> Result<Check, ExecFailure> {
    let kind = builtin("pods").expect("pods are built in");
    let path = kind.object_path(Some(&request.namespace), &request.pod);
    let get = json_get(path, JSON)?;
    let pod: Result<Value, kube::Error> =
        tokio::time::timeout(REQUEST_DEADLINE, client.request(get))
            .await
            .map_err(|_| Failure::timeout("Reading the pod"))?;
    let pod = match pod {
        Ok(pod) => pod,
        Err(error) => {
            let failure = Failure::from_kube(error);
            if failure.kind == FailureKind::NotFound {
                return Ok(Check::Gone("The pod was deleted".into()));
            }
            return Err(failure.into());
        }
    };
    let uid = text(pod.get("metadata").and_then(|metadata| metadata.get("uid")));
    if uid != request.uid {
        return Ok(Check::Gone("The pod was replaced".into()));
    }
    let Some(container) = pod_containers(&pod)
        .containers
        .into_iter()
        .find(|container| container.name == request.container)
    else {
        return Ok(Check::Gone(format!(
            "The pod has no container named {}",
            request.container
        )));
    };
    Ok(match container.state {
        ContainerState::Running(_) => Check::Running {
            restarts: container.restarts,
        },
        ContainerState::Waiting(reason) if !reason.is_empty() => Check::NotRunning(reason),
        ContainerState::Waiting(_) => Check::NotRunning("It hasn't started".into()),
        ContainerState::Terminated(termination) => {
            Check::NotRunning(format!("It exited ({termination})"))
        }
    })
}

/// The exec closed without a status: the pod says whether it went away.
async fn ended_without_status(
    client: &Client,
    request: &ExecRequest,
    restarts: u32,
    broke: Option<String>,
) -> ExecEnd {
    let dropped = || {
        let message = broke
            .clone()
            .unwrap_or_else(|| "The connection closed".to_owned());
        ExecEnd::Disconnected(Failure::new(FailureKind::Unreachable, message))
    };
    match check_container(client, request).await {
        Ok(Check::Gone(reason)) => ExecEnd::PodGone(reason),
        Ok(Check::NotRunning(_)) => ExecEnd::PodGone("The container stopped".into()),
        Ok(Check::Running { restarts: now }) if now != restarts => {
            ExecEnd::PodGone("The container restarted".into())
        }
        Ok(Check::Running { .. }) | Err(_) => dropped(),
    }
}

#[cfg(test)]
mod tests;
