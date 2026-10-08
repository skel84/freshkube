//! One forward as the app holds it: its request, where it stands and what
//! the status bar and the Ports tab say about it, derived when it changes.

use freshkube_core::pluralize;
use std::time::Duration;

use freshkube_core::resources::{
    FailureKind, ForwardEnd, ForwardFailure, ForwardFailureKind, ForwardGuard, ForwardRequest,
    ForwardState, ForwardStatus, ForwardTarget, Listeners, start_forward,
};
use gpui_kit::*;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::example::{self, ExampleForward, ExampleGuard, Route};
use super::{Listen, Watches};
use crate::backend::{self, OwnedJob};
use crate::resources::KubeAccess;
use crate::resources::model::ResourceIdentity;
use crate::ui::Tone;

/// How long starting may take: reading the target, waiting for a Ready pod
/// (10 s), and listening.
const START_DEADLINE: Duration = Duration::from_secs(50);
/// Status changes reach the UI at most this often.
pub(super) const UPDATE_INTERVAL: Duration = Duration::from_secs(1);

/// What starting a forward needs.
#[derive(Clone)]
pub(crate) struct ForwardSpec {
    pub(crate) runtime: Handle,
    pub(crate) access: KubeAccess,
    /// The object forwarded; its connection is part of it.
    pub(crate) identity: ResourceIdentity,
    pub(crate) target: ForwardTarget,
    /// The context it was started in, which the list names.
    pub(crate) context: String,
    /// A container port, or a Service's port.
    pub(crate) port: u16,
    /// A port the user typed; `None` is automatic.
    pub(crate) local_port: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Starting,
    Running,
    Ended,
}

/// What the list and the tab say about a forward.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Display {
    /// "Service shop/web".
    pub(crate) target: SharedString,
    pub(crate) context: SharedString,
    /// "localhost:10080 → 80", and the pod once known.
    pub(crate) route: SharedString,
    pub(crate) tone: Tone,
    pub(crate) tag: SharedString,
    /// Connections while it runs; the reason once it ended.
    pub(crate) detail: SharedString,
    pub(crate) error: Option<SharedString>,
    /// All of it, for assistive technology and tests.
    pub(crate) label: SharedString,
}

/// What a row shows, read out so it can be drawn while the app is
/// borrowed elsewhere.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub(crate) id: u64,
    pub(crate) running: bool,
    pub(crate) address: Option<String>,
    pub(crate) port_was_taken: bool,
    pub(crate) display: Display,
}

enum Guard {
    Live(ForwardGuard),
    Example(ExampleGuard),
}

impl Guard {
    fn stop(self) -> JoinHandle<()> {
        match self {
            Guard::Live(guard) => guard.stop(),
            Guard::Example(guard) => guard.stop(),
        }
    }
}

enum Session {
    Starting {
        _job: OwnedJob,
        _task: Task<()>,
    },
    /// An example forward waiting for its port, bound on Tokio's blocking
    /// pool. Dropping it leaves the bind to finish and drops its listeners.
    Binding {
        _task: Task<()>,
    },
    Running {
        guard: Guard,
        _delivery: Task<()>,
    },
}

pub(crate) struct ForwardView {
    pub(crate) id: u64,
    spec: ForwardSpec,
    watches: Watches,
    listen: Listen,
    /// The port it listened on last, which Start again asks for first.
    pub(crate) local_port: Option<u16>,
    pub(crate) phase: Phase,
    status: Option<ForwardStatus>,
    /// Why it ended, once it has.
    end: Option<ForwardEnd>,
    session: Option<Session>,
    pub(crate) display: Display,
    /// Advances with each start; anything from an older one is dropped.
    seq: u64,
}

impl ForwardView {
    pub(super) fn new(id: u64, spec: ForwardSpec, watches: Watches, listen: Listen) -> Self {
        let mut view = Self {
            id,
            spec,
            watches,
            listen,
            local_port: None,
            phase: Phase::Starting,
            status: None,
            end: None,
            session: None,
            display: Display::default(),
            seq: 0,
        };
        view.describe();
        view
    }

    pub(crate) fn running(&self) -> bool {
        self.phase != Phase::Ended
    }

    /// The port it forwards, when it forwards the object `identity`.
    pub(crate) fn port_of(&self, identity: &ResourceIdentity) -> Option<u16> {
        (self.spec.identity == *identity).then_some(self.spec.port)
    }

    pub(crate) fn row(&self) -> Row {
        Row {
            id: self.id,
            running: self.running(),
            address: self.address(),
            port_was_taken: self.port_was_taken(),
            display: self.display.clone(),
        }
    }

    pub(crate) fn address(&self) -> Option<String> {
        self.local_port.map(|port| format!("localhost:{port}"))
    }

    /// The typed local port was taken, or Windows reserves it, so the
    /// automatic one may do instead.
    pub(crate) fn port_was_taken(&self) -> bool {
        matches!(
            &self.end,
            Some(ForwardEnd::Failed(ForwardFailure {
                kind: ForwardFailureKind::PortInUse | ForwardFailureKind::PortReserved,
                ..
            }))
        )
    }

    /// Starts as asked.
    pub(super) fn start(&mut self, cx: &mut Context<Self>) {
        let request = self.request(self.spec.local_port);
        self.begin(request, None, cx);
    }

    /// Starts again on the port it had, or as first asked when that is
    /// taken now.
    pub(super) fn start_again(&mut self, cx: &mut Context<Self>) {
        if self.running() {
            return;
        }
        let first = self.request(self.local_port.or(self.spec.local_port));
        let fallback = (self.spec.local_port.is_none() && self.local_port.is_some())
            .then(|| self.request(None));
        self.begin(first, fallback, cx);
    }

    /// Starts again on the automatic port, after the typed one was taken
    /// or reserved.
    pub(super) fn use_automatic(&mut self, cx: &mut Context<Self>) {
        if self.running() {
            return;
        }
        self.spec.local_port = None;
        self.local_port = None;
        let request = self.request(None);
        self.begin(request, None, cx);
    }

    fn request(&self, local_port: Option<u16>) -> ForwardRequest {
        ForwardRequest {
            namespace: self.spec.identity.namespace.clone(),
            target: self.spec.target.clone(),
            port: self.spec.port,
            local_port,
        }
    }

    fn begin(
        &mut self,
        request: ForwardRequest,
        fallback: Option<ForwardRequest>,
        cx: &mut Context<Self>,
    ) {
        self.session = None;
        self.seq += 1;
        let seq = self.seq;
        self.phase = Phase::Starting;
        self.status = None;
        self.end = None;
        if let KubeAccess::Example = self.spec.access {
            return self.begin_example(seq, request, fallback, cx);
        }
        let access = self.spec.access.clone();
        let watches = self.watches.clone();
        let connection = self.spec.identity.connection.clone();
        let (job, receiver) = backend::spawn_job(
            &self.spec.runtime,
            START_DEADLINE,
            format!("No answer within {} seconds", START_DEADLINE.as_secs()),
            async move {
                let client = match access.client().await {
                    Ok(client) => client,
                    Err(error) => {
                        access.forget();
                        return Ok(Err(ForwardFailure::new(
                            ForwardFailureKind::Request(FailureKind::Other),
                            error,
                        )));
                    }
                };
                let pods = watches.of(&connection, client);
                let result = match (start_forward(&pods, request).await, fallback) {
                    (Err(failure), Some(fallback))
                        if failure.kind == ForwardFailureKind::PortInUse =>
                    {
                        start_forward(&pods, fallback).await
                    }
                    (result, _) => result,
                };
                if let Err(ForwardFailure {
                    kind: ForwardFailureKind::Request(kind),
                    ..
                }) = &result
                    && !kind.is_permanent()
                {
                    access.forget();
                    watches.forget(&connection);
                }
                Ok(result)
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = match receiver.await {
                Ok(Ok(result)) => result,
                Ok(Err(message)) => Err(ForwardFailure::new(
                    ForwardFailureKind::Request(FailureKind::Timeout),
                    message,
                )),
                Err(_) => return,
            };
            _ = this.update(cx, |view, cx| view.started(seq, result, cx));
        });
        self.session = Some(Session::Starting {
            _job: job,
            _task: task,
        });
        self.describe();
        cx.notify();
    }

    /// Resolves an example target at once, then listens on Tokio's blocking
    /// pool, since asking whether a port is free can take seconds on
    /// Windows; the click that started it returns before then.
    fn begin_example(
        &mut self,
        seq: u64,
        request: ForwardRequest,
        fallback: Option<ForwardRequest>,
        cx: &mut Context<Self>,
    ) {
        let route = match example::route(
            &self.spec.context,
            &self.spec.identity,
            &self.spec.target,
            self.spec.port,
        ) {
            Ok(route) => route,
            Err(failure) => return self.finish(ForwardEnd::Failed(failure), cx),
        };
        let listen = self.listen.clone();
        let binding = self.spec.runtime.spawn_blocking(move || {
            match (listen(request.local_port, request.port), fallback) {
                (Err(failure), Some(fallback)) if failure.kind == ForwardFailureKind::PortInUse => {
                    listen(fallback.local_port, fallback.port)
                }
                (result, _) => result,
            }
        });
        let task = cx.spawn(async move |this, cx| {
            let bound = binding.await.unwrap_or_else(|error| {
                Err(ForwardFailure::new(
                    ForwardFailureKind::Request(FailureKind::Other),
                    format!("Couldn't listen on the loopback · {error}"),
                ))
            });
            _ = this.update(cx, |view, cx| view.bound(seq, bound, route, cx));
        });
        self.session = Some(Session::Binding { _task: task });
        self.describe();
        cx.notify();
    }

    /// An example forward's port answered: it serves there, unless it was
    /// stopped or started again meanwhile, which drops the listeners.
    fn bound(
        &mut self,
        seq: u64,
        bound: Result<Listeners, ForwardFailure>,
        route: Option<Route>,
        cx: &mut Context<Self>,
    ) {
        if seq != self.seq || self.phase != Phase::Starting {
            return;
        }
        match bound {
            Ok(listeners) => {
                let ExampleForward {
                    local_port,
                    status,
                    guard,
                } = example::serve_on(&self.spec.runtime, listeners, route);
                self.run(seq, local_port, status, Guard::Example(guard), cx);
            }
            Err(failure) => self.finish(ForwardEnd::Failed(failure), cx),
        }
    }

    fn started(
        &mut self,
        seq: u64,
        result: Result<freshkube_core::resources::Forward, ForwardFailure>,
        cx: &mut Context<Self>,
    ) {
        if seq != self.seq || self.phase != Phase::Starting {
            return;
        }
        match result {
            Ok(forward) => self.run(
                seq,
                forward.local_port,
                forward.status,
                Guard::Live(forward.guard),
                cx,
            ),
            Err(failure) => self.finish(ForwardEnd::Failed(failure), cx),
        }
    }

    /// It listens: status changes come through at most once a second.
    fn run(
        &mut self,
        seq: u64,
        local_port: u16,
        mut status: watch::Receiver<ForwardStatus>,
        guard: Guard,
        cx: &mut Context<Self>,
    ) {
        self.local_port = Some(local_port);
        self.phase = Phase::Running;
        self.status = Some(status.borrow_and_update().clone());
        let delivery = cx.spawn(async move |this, cx| {
            loop {
                if status.changed().await.is_err() {
                    break;
                }
                let latest = status.borrow_and_update().clone();
                if this
                    .update(cx, |view, cx| view.apply(seq, latest, cx))
                    .is_err()
                {
                    break;
                }
                cx.background_executor().timer(UPDATE_INTERVAL).await;
            }
        });
        self.session = Some(Session::Running {
            guard,
            _delivery: delivery,
        });
        self.describe();
        cx.notify();
    }

    fn apply(&mut self, seq: u64, status: ForwardStatus, cx: &mut Context<Self>) {
        if seq != self.seq || self.phase != Phase::Running {
            return;
        }
        if let ForwardState::Ended(end) = &status.state {
            let end = end.clone();
            self.status = Some(status);
            return self.finish(end, cx);
        }
        self.status = Some(status);
        self.describe();
        cx.notify();
    }

    fn finish(&mut self, end: ForwardEnd, cx: &mut Context<Self>) {
        self.session = None;
        self.phase = Phase::Ended;
        self.end = Some(end);
        if let Some(status) = &mut self.status {
            status.connections = 0;
        }
        self.describe();
        cx.notify();
    }

    /// Stops it: the listeners and every connection close. The handle
    /// completes once the port is free.
    pub(super) fn stop(&mut self, cx: &mut Context<Self>) -> Option<JoinHandle<()>> {
        if !self.running() {
            return None;
        }
        let closing = match self.session.take() {
            Some(Session::Running { guard, .. }) => Some(guard.stop()),
            _ => None,
        };
        self.seq += 1;
        self.finish(ForwardEnd::Stopped, cx);
        closing
    }

    /// Derives what the list and the tab say.
    fn describe(&mut self) {
        let spec = &self.spec;
        let target = format!(
            "{} {}/{}",
            target_kind(&spec.target),
            spec.identity.namespace,
            spec.identity.name
        );
        let local = match self.local_port {
            Some(port) => format!("localhost:{port}"),
            None => "localhost".to_owned(),
        };
        let pod = match self.status.as_ref().map(|status| &status.state) {
            Some(ForwardState::Listening { pod, port }) if self.phase == Phase::Running => {
                match spec.target {
                    ForwardTarget::Pod { .. } => None,
                    _ => Some(format!(" · {pod}:{port}")),
                }
            }
            _ => None,
        };
        let route = format!("{local} → {}{}", spec.port, pod.unwrap_or_default());
        let connections = self.status.as_ref().map_or(0, |status| status.connections);
        let error = self
            .status
            .as_ref()
            .and_then(|status| status.last_error.clone())
            .filter(|_| self.phase == Phase::Running);
        let (tone, tag, detail) = match (&self.phase, &self.end) {
            (Phase::Starting, _) => (Tone::Unknown, "Starting", String::new()),
            (Phase::Running, _) => match self.status.as_ref().map(|status| &status.state) {
                Some(ForwardState::NoReadyPod) => (
                    Tone::Warn,
                    "No ready pod",
                    "New connections wait up to 10 s for a Ready pod".to_owned(),
                ),
                _ => (Tone::Good, "Listening", count(connections)),
            },
            (Phase::Ended, Some(ForwardEnd::Stopped)) => {
                (Tone::Unknown, "Stopped", "You stopped it".to_owned())
            }
            (Phase::Ended, Some(ForwardEnd::PodGone(reason)))
            | (Phase::Ended, Some(ForwardEnd::TargetDeleted(reason))) => {
                (Tone::Unknown, "Ended", reason.clone())
            }
            (Phase::Ended, Some(ForwardEnd::Failed(failure))) => {
                (Tone::Crit, "Failed", failure.to_string())
            }
            (Phase::Ended, None) => (Tone::Unknown, "Ended", String::new()),
        };
        let mut label = format!(
            "{target} in {}: {route}, {tag}",
            if spec.context.is_empty() {
                "this context"
            } else {
                &spec.context
            }
        );
        if !detail.is_empty() {
            label.push_str(&format!(", {detail}"));
        }
        if let Some(error) = &error {
            label.push_str(&format!(", last error: {error}"));
        }
        self.display = Display {
            target: target.into(),
            context: spec.context.clone().into(),
            route: route.into(),
            tone,
            tag: tag.into(),
            detail: detail.into(),
            error: error.map(SharedString::from),
            label: label.into(),
        };
    }
}

fn target_kind(target: &ForwardTarget) -> &'static str {
    match target {
        ForwardTarget::Pod { .. } => "Pod",
        _ => target.label(),
    }
}

fn count(connections: usize) -> String {
    match connections {
        0 => "No connections".to_owned(),
        count => pluralize(count, "connection", "connections"),
    }
}
