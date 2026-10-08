//! Port forwarding: a pod's port on the Mac's loopback, as `kubectl
//! port-forward` makes it (docs/PORT_FORWARD.md). Each connection creates a
//! `pods/portforward`, and only once the user started a forward.
//!
//! [`start_forward`] listens on the local port, resolves the target to a
//! pod and returns once it listens. Each local connection then gets its own
//! websocket to the pod's port, while the forward watches its pods: a pod
//! target ends when its pod goes, a Service or workload moves to another
//! Ready pod. It lasts until stopped, its pod goes or its target is
//! deleted.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures::Stream;
use k8s_openapi::api::core::v1::{Pod, Service};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use kube::api::{ApiResource, DynamicObject};
use kube::runtime::{metadata_watcher, watcher};
use kube::{Api, Client};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use self::pods::{PodSelector, Subscription};
pub(crate) use self::target::label_selector;
use self::target::{PodInfo, RemotePort, choose_pod, service_selector};
use super::events::classify;
use super::failure::{Failure, FailureKind};
use super::kinds::ResourceKind;

mod pods;
mod port;
mod relay;
mod target;
#[cfg(test)]
mod tests;

pub use pods::PodWatches;
pub use port::{Listeners, candidates, preferred_port};
pub use target::{DeclaredPort, WorkloadKind, declared_ports};

/// How long reading the target, or opening a connection's forward, may take.
const REQUEST_DEADLINE: Duration = Duration::from_secs(30);
/// How long starting waits for a Service's or workload's pod to be Ready,
/// and a connection for any pod while there is none.
pub const POD_WAIT: Duration = Duration::from_secs(10);
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// A pause after a failed accept, so a full file table doesn't spin.
const ACCEPT_PAUSE: Duration = Duration::from_millis(100);

/// What a forward reaches.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ForwardTarget {
    /// One pod; the UID makes sure it stays that pod.
    Pod { name: String, uid: String },
    /// A Service's Ready pods; the port is the Service's.
    Service { name: String },
    /// A workload's Ready pods; the port is a container's.
    Workload { kind: WorkloadKind, name: String },
}

impl ForwardTarget {
    /// The target for an object of `kind`, when it can be forwarded.
    pub fn of(kind: &ResourceKind, name: &str, uid: &str) -> Option<Self> {
        let name = name.to_owned();
        if kind.is_pod() {
            return Some(ForwardTarget::Pod {
                name,
                uid: uid.to_owned(),
            });
        }
        if kind.group.is_empty() && kind.plural == "services" {
            return Some(ForwardTarget::Service { name });
        }
        WorkloadKind::of(kind).map(|kind| ForwardTarget::Workload { kind, name })
    }

    pub fn name(&self) -> &str {
        match self {
            ForwardTarget::Pod { name, .. }
            | ForwardTarget::Service { name }
            | ForwardTarget::Workload { name, .. } => name,
        }
    }

    /// Its kind, as messages name it.
    pub fn label(&self) -> &'static str {
        match self {
            ForwardTarget::Pod { .. } => "pod",
            ForwardTarget::Service { .. } => "Service",
            ForwardTarget::Workload { kind, .. } => kind.label(),
        }
    }
}

/// A forward to start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForwardRequest {
    pub namespace: String,
    pub target: ForwardTarget,
    /// The port on the target: a container's port, or a Service's port.
    pub port: u16,
    /// A port the user typed, used exactly; `None` picks one by the
    /// automatic rule ([`preferred_port`]).
    pub local_port: Option<u16>,
}

/// Where a forward stands, as it changes.
#[derive(Clone, Debug, PartialEq)]
pub struct ForwardStatus {
    pub state: ForwardState,
    /// Local connections open now, including any waiting for a pod.
    pub connections: usize,
    /// The last thing that went wrong while the forward kept listening,
    /// such as a connection refused in the pod.
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ForwardState {
    /// Connections go to this pod's port.
    Listening {
        pod: String,
        port: u16,
    },
    /// A Service or workload has no Ready pod; a new connection waits up
    /// to [`POD_WAIT`] for one.
    NoReadyPod,
    Ended(ForwardEnd),
}

/// Why a forward ended. Its port is free by then.
#[derive(Clone, Debug, PartialEq)]
pub enum ForwardEnd {
    /// The user stopped it.
    Stopped,
    /// A pod target was deleted, replaced or finished.
    PodGone(String),
    /// The Service or workload was deleted.
    TargetDeleted(String),
    Failed(ForwardFailure),
}

/// What kind of failure a forward hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForwardFailureKind {
    /// A request failed as a read would: refused, not found, unreachable.
    Request(FailureKind),
    /// Something else listens on the local port the user typed.
    PortInUse,
    /// The system keeps the local port the user typed for itself: one in a
    /// range Windows reserves.
    PortReserved,
    /// The pod isn't running.
    NotRunning,
    /// The target selects no pods: a Service without a selector.
    NoPods,
    /// Only TCP ports can be forwarded.
    NotTcp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForwardFailure {
    pub kind: ForwardFailureKind,
    pub message: String,
}

impl ForwardFailure {
    pub fn new(kind: ForwardFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(ForwardFailureKind::Request(FailureKind::NotFound), message)
    }

    /// Retrying won't help until something outside the app changes.
    pub fn is_permanent(&self) -> bool {
        match self.kind {
            ForwardFailureKind::Request(kind) => kind.is_permanent(),
            ForwardFailureKind::PortInUse
            | ForwardFailureKind::PortReserved
            | ForwardFailureKind::NotRunning => false,
            ForwardFailureKind::NoPods | ForwardFailureKind::NotTcp => true,
        }
    }
}

impl From<Failure> for ForwardFailure {
    fn from(failure: Failure) -> Self {
        Self::new(ForwardFailureKind::Request(failure.kind), failure.message)
    }
}

impl fmt::Display for ForwardFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.kind {
            ForwardFailureKind::Request(kind) => {
                return Failure::new(kind, self.message.clone()).fmt(f);
            }
            ForwardFailureKind::PortInUse => "In use",
            ForwardFailureKind::PortReserved => "Reserved",
            ForwardFailureKind::NotRunning => "Not running",
            ForwardFailureKind::NoPods => "No pods",
            ForwardFailureKind::NotTcp => "Not TCP",
        };
        write!(f, "{kind} · {}", self.message)
    }
}

impl std::error::Error for ForwardFailure {}

/// A running forward.
pub struct Forward {
    /// The local port it listens on, on 127.0.0.1 and ::1.
    pub local_port: u16,
    /// The latest status; a burst of changes reads as the last one.
    pub status: watch::Receiver<ForwardStatus>,
    pub guard: ForwardGuard,
}

/// Owns a forward's work on Tokio. Dropping it stops the forward.
pub struct ForwardGuard {
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl ForwardGuard {
    /// Stops the forward: the listeners close and every connection with
    /// them. The handle completes once the port is free.
    pub fn stop(mut self) -> JoinHandle<()> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.task
            .take()
            .expect("a guard owns its task until stopped")
    }
}

impl Drop for ForwardGuard {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Where connections go, shared with every connection.
#[derive(Clone, Debug, Default, PartialEq)]
struct Route {
    /// Advances whenever the pod changes, so connections to the old one
    /// close.
    generation: u64,
    upstream: Option<Upstream>,
    /// The forward ended: every connection closes.
    closed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Upstream {
    pod: String,
    uid: String,
    port: u16,
}

/// Starts a forward. Returns once it listens, never before. Run it on
/// Tokio, with the pod watches of the connection the forward uses.
pub async fn start_forward(
    watches: &PodWatches,
    request: ForwardRequest,
) -> Result<Forward, ForwardFailure> {
    let listeners = listen_blocking(request.local_port, request.port).await?;
    let resolved = resolve(watches.client(), &request).await?;
    let pods = watches.subscribe(&request.namespace, resolved.selector.clone());
    let (route, _) = watch::channel(Route::default());
    let (status, receiver) = watch::channel(ForwardStatus {
        state: ForwardState::NoReadyPod,
        connections: 0,
        last_error: None,
    });
    let mut session = Session {
        watches: watches.clone(),
        api: Api::namespaced(watches.client().clone(), &request.namespace),
        request,
        remote: resolved.remote,
        selector: resolved.selector,
        pods,
        route,
        status: Arc::new(status),
    };
    match resolved.pod {
        Some(upstream) => session.set_upstream(Some(upstream)),
        None => session.wait_for_pod().await?,
    }
    let targets = resolved
        .follow
        .map(|follow| follow_target(follow, session.request.target.name().to_owned()));
    let local_port = listeners.port;
    let (stop, stopping) = oneshot::channel();
    let task = tokio::spawn(session.run(listeners, stopping, targets));
    Ok(Forward {
        local_port,
        status: receiver,
        guard: ForwardGuard {
            stop: Some(stop),
            task: Some(task),
        },
    })
}

/// [`listen_local`] on Tokio's blocking pool: on Windows, asking whether a
/// port is free can take a couple of seconds.
pub async fn listen_blocking(
    local_port: Option<u16>,
    remote: u16,
) -> Result<Listeners, ForwardFailure> {
    tokio::task::spawn_blocking(move || listen_local(local_port, remote))
        .await
        .unwrap_or_else(|error| {
            Err(ForwardFailure::new(
                ForwardFailureKind::Request(FailureKind::Other),
                format!("Couldn't listen on the loopback · {error}"),
            ))
        })
}

/// Listens on the loopback as a forward would: on `local_port` exactly, or
/// on the automatic port for `remote`. Example mode serves its own answers
/// on what this returns. It blocks, on Windows for a couple of seconds, so
/// it runs off the UI thread ([`listen_blocking`]).
pub fn listen_local(local_port: Option<u16>, remote: u16) -> Result<Listeners, ForwardFailure> {
    let bound = match local_port {
        Some(port) => port::bind_loopback(port),
        None => port::bind_automatic(remote),
    };
    bound.map_err(|error| match local_port {
        Some(port) if port::is_taken(&error) => ForwardFailure::new(
            ForwardFailureKind::PortInUse,
            format!("Port {port} is in use"),
        ),
        Some(port) if port::is_reserved(&error) => ForwardFailure::new(
            ForwardFailureKind::PortReserved,
            format!("Windows reserves port {port}"),
        ),
        _ => ForwardFailure::new(
            ForwardFailureKind::Request(FailureKind::Other),
            format!("Couldn't listen on the loopback · {error}"),
        ),
    })
}

/// What a target turned out to be.
struct Resolved {
    selector: PodSelector,
    remote: RemotePort,
    /// A pod target's pod, fixed for the forward's life.
    pod: Option<Upstream>,
    /// What to follow about a Service or workload.
    follow: Option<Follow>,
}

enum Follow {
    Service(Api<Service>),
    Workload(Api<DynamicObject>),
}

async fn resolve(client: &Client, request: &ForwardRequest) -> Result<Resolved, ForwardFailure> {
    let namespace = &request.namespace;
    match &request.target {
        ForwardTarget::Pod { name, uid } => {
            let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
            let Some(pod) = get(api.get_opt(name)).await? else {
                return Err(ForwardFailure::not_found("The pod was deleted"));
            };
            let pod = PodInfo::of(&pod);
            if pod.uid != *uid {
                return Err(ForwardFailure::not_found("The pod was replaced"));
            }
            if pod.phase != "Running" {
                let phase = if pod.phase.is_empty() {
                    "not running"
                } else {
                    &pod.phase
                };
                return Err(ForwardFailure::new(
                    ForwardFailureKind::NotRunning,
                    format!("The pod is {phase}"),
                ));
            }
            Ok(Resolved {
                selector: PodSelector::Name(name.clone()),
                remote: RemotePort::Number(request.port),
                pod: Some(Upstream {
                    pod: name.clone(),
                    uid: uid.clone(),
                    port: request.port,
                }),
                follow: None,
            })
        }
        ForwardTarget::Service { name } => {
            let api: Api<Service> = Api::namespaced(client.clone(), namespace);
            let Some(service) = get(api.get_opt(name)).await? else {
                return Err(ForwardFailure::not_found("The Service was not found"));
            };
            let (selector, remote) = service_route(&service, request.port)?;
            Ok(Resolved {
                selector: PodSelector::Labels(selector),
                remote,
                pod: None,
                follow: Some(Follow::Service(api)),
            })
        }
        ForwardTarget::Workload { kind, name } => {
            let resource = api_resource(&kind.resource());
            let api: Api<DynamicObject> =
                Api::namespaced_with(client.clone(), namespace, &resource);
            let label = kind.label();
            let Some(object) = get(api.get_opt(name)).await? else {
                return Err(ForwardFailure::not_found(format!(
                    "The {label} was not found"
                )));
            };
            let selector = object
                .data
                .get("spec")
                .and_then(|spec| spec.get("selector"))
                .and_then(|selector| {
                    serde_json::from_value::<LabelSelector>(selector.clone()).ok()
                });
            let Some(selector) = label_selector(selector.as_ref()) else {
                return Err(ForwardFailure::new(
                    ForwardFailureKind::NoPods,
                    format!("The {label} has no selector"),
                ));
            };
            Ok(Resolved {
                selector: PodSelector::Labels(selector),
                remote: RemotePort::Number(request.port),
                pod: None,
                follow: Some(Follow::Workload(api)),
            })
        }
    }
}

/// A Service's pods and the port on them for its port `port`.
fn service_route(service: &Service, port: u16) -> Result<(String, RemotePort), ForwardFailure> {
    let spec = service.spec.as_ref();
    let Some(selector) = service_selector(spec.and_then(|spec| spec.selector.as_ref())) else {
        return Err(ForwardFailure::new(
            ForwardFailureKind::NoPods,
            "The Service selects no pods, so there is nothing to forward to",
        ));
    };
    let Some(service_port) = spec
        .and_then(|spec| spec.ports.as_ref())
        .into_iter()
        .flatten()
        .find(|service_port| service_port.port == i32::from(port))
    else {
        return Err(ForwardFailure::not_found(format!(
            "The Service has no port {port}"
        )));
    };
    if service_port.protocol.as_deref().unwrap_or("TCP") != "TCP" {
        return Err(ForwardFailure::new(
            ForwardFailureKind::NotTcp,
            "Only TCP ports can be forwarded",
        ));
    }
    Ok((selector, RemotePort::of_service(service_port)))
}

pub(crate) fn api_resource(kind: &ResourceKind) -> ApiResource {
    ApiResource {
        group: kind.group.clone(),
        version: kind.version.clone(),
        api_version: kind.api_version(),
        kind: kind.kind.clone(),
        plural: kind.plural.clone(),
    }
}

async fn get<T>(
    request: impl Future<Output = Result<T, kube::Error>>,
) -> Result<T, ForwardFailure> {
    tokio::time::timeout(REQUEST_DEADLINE, request)
        .await
        .map_err(|_| Failure::timeout("Reading the target"))?
        .map_err(|error| Failure::from_kube(error).into())
}

/// A change to a Service or workload.
enum TargetChange {
    /// A Service as it is now: its selector or ports may have changed.
    Service(Box<Service>),
    Deleted,
}

/// Watches a Service or workload by name for as long as the receiver
/// lives.
fn follow_target(follow: Follow, name: String) -> TargetWatch {
    let (sink, changes) = mpsc::channel(8);
    let config = watcher::Config::default().fields(&format!("metadata.name={name}"));
    let task = match follow {
        Follow::Service(api) => tokio::spawn(follow_object(
            move || watcher(api.clone(), config.clone()),
            |service| Some(TargetChange::Service(Box::new(service))),
            sink,
        )),
        // A workload's selector can't change, so only its deletion counts.
        Follow::Workload(api) => tokio::spawn(follow_object(
            move || metadata_watcher(api.clone(), config.clone()),
            |_| None,
            sink,
        )),
    };
    TargetWatch { changes, task }
}

struct TargetWatch {
    changes: mpsc::Receiver<TargetChange>,
    task: JoinHandle<()>,
}

impl Drop for TargetWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn follow_object<T, S>(
    watch: impl Fn() -> S,
    change: impl Fn(T) -> Option<TargetChange>,
    sink: mpsc::Sender<TargetChange>,
) where
    S: Stream<Item = Result<watcher::Event<T>, watcher::Error>>,
{
    use futures::StreamExt;

    let mut backoff = MIN_BACKOFF;
    loop {
        let mut stream = std::pin::pin!(watch());
        let mut seen = false;
        let failure = loop {
            let Some(item) = stream.next().await else {
                return;
            };
            let next = match item {
                Ok(watcher::Event::Init) => {
                    seen = false;
                    None
                }
                Ok(watcher::Event::InitApply(object)) => {
                    seen = true;
                    change(object)
                }
                Ok(watcher::Event::InitDone) => {
                    backoff = MIN_BACKOFF;
                    (!seen).then_some(TargetChange::Deleted)
                }
                Ok(watcher::Event::Apply(object)) => change(object),
                Ok(watcher::Event::Delete(_)) => Some(TargetChange::Deleted),
                Err(watcher::Error::WatchError(status)) if status.code == 410 => None,
                Err(error) => break classify(error),
            };
            if let Some(next) = next
                && sink.send(next).await.is_err()
            {
                return;
            }
        };
        // Unable to watch the target, the forward still forwards; it just
        // won't notice the target's deletion.
        if failure.kind.is_permanent() {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// A running forward's state, owned by its task.
struct Session {
    watches: PodWatches,
    api: Api<Pod>,
    request: ForwardRequest,
    remote: RemotePort,
    selector: PodSelector,
    pods: Subscription,
    route: watch::Sender<Route>,
    status: Arc<watch::Sender<ForwardStatus>>,
}

impl Session {
    /// Waits up to [`POD_WAIT`] for a Service's or workload's first Ready
    /// pod. Without one it starts as [`ForwardState::NoReadyPod`]; a pod
    /// list it may never read fails the start.
    async fn wait_for_pod(&mut self) -> Result<(), ForwardFailure> {
        let deadline = tokio::time::Instant::now() + POD_WAIT;
        loop {
            let snapshot = self.pods.snapshot.borrow_and_update().clone();
            if let Some(failure) = snapshot
                .failure
                .filter(|failure| failure.kind.is_permanent())
            {
                return Err(failure.into());
            }
            if snapshot.listed
                && let Some((pod, port)) = choose_pod(&snapshot.pods, &self.remote)
            {
                let upstream = Upstream {
                    pod: pod.name.clone(),
                    uid: pod.uid.clone(),
                    port,
                };
                self.set_upstream(Some(upstream));
                return Ok(());
            }
            match tokio::time::timeout_at(deadline, self.pods.snapshot.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => return Ok(()),
            }
        }
    }

    async fn run(
        mut self,
        listeners: Listeners,
        mut stop: oneshot::Receiver<()>,
        mut targets: Option<TargetWatch>,
    ) {
        let (fatal, mut fatalities) = mpsc::channel(1);
        let mut following = true;
        let end = loop {
            tokio::select! {
                biased;
                _ = &mut stop => break ForwardEnd::Stopped,
                Some(failure) = fatalities.recv() => break ForwardEnd::Failed(failure),
                change = next_change(&mut targets) => match change {
                    Some(TargetChange::Deleted) => {
                        let label = self.request.target.label();
                        break ForwardEnd::TargetDeleted(format!("The {label} was deleted"));
                    }
                    Some(TargetChange::Service(service)) => self.service_changed(&service),
                    None => targets = None,
                },
                changed = self.pods.snapshot.changed(), if following => {
                    if changed.is_err() {
                        following = false;
                    } else if let Some(end) = self.reroute() {
                        break end;
                    }
                }
                accepted = listeners.v4.accept() => self.accepted(accepted, &fatal).await,
                accepted = accept(listeners.v6.as_ref()) => self.accepted(accepted, &fatal).await,
            }
        };
        drop(listeners);
        self.route.send_modify(|route| route.closed = true);
        self.status
            .send_modify(|status| status.state = ForwardState::Ended(end));
    }

    async fn accepted(
        &self,
        accepted: std::io::Result<(TcpStream, std::net::SocketAddr)>,
        fatal: &mpsc::Sender<ForwardFailure>,
    ) {
        match accepted {
            Ok((local, _)) => {
                let _ = local.set_nodelay(true);
                tokio::spawn(relay::connection(
                    local,
                    self.api.clone(),
                    self.route.subscribe(),
                    self.status.clone(),
                    fatal.clone(),
                ));
            }
            Err(error) => {
                tracing::warn!("A forward couldn't accept a connection: {error}");
                tokio::time::sleep(ACCEPT_PAUSE).await;
            }
        }
    }

    /// Follows the pods as they change. A pod target ends when its pod
    /// goes; a Service or workload keeps its pod while it stays Ready and
    /// otherwise moves to the next, or to none.
    fn reroute(&mut self) -> Option<ForwardEnd> {
        let snapshot = self.pods.snapshot.borrow_and_update().clone();
        if let Some(failure) = &snapshot.failure {
            let message = format!("Can't follow the pods · {failure}");
            self.status
                .send_modify(|status| status.last_error = Some(message));
        }
        if !snapshot.listed {
            return None;
        }
        if let ForwardTarget::Pod { name, uid } = &self.request.target {
            return match snapshot.pods.iter().find(|pod| pod.uid == *uid) {
                Some(pod) if pod.stopped() => Some(ForwardEnd::PodGone("The pod stopped".into())),
                Some(_) => None,
                None if snapshot.pods.iter().any(|pod| pod.name == *name) => {
                    Some(ForwardEnd::PodGone("The pod was replaced".into()))
                }
                None => Some(ForwardEnd::PodGone("The pod was deleted".into())),
            };
        }
        let current = self.route.borrow().upstream.clone();
        let keep = current.as_ref().is_some_and(|upstream| {
            snapshot.pods.iter().any(|pod| {
                pod.uid == upstream.uid
                    && pod.usable()
                    && self.remote.on(pod) == Some(upstream.port)
            })
        });
        if keep {
            return None;
        }
        let next = choose_pod(&snapshot.pods, &self.remote).map(|(pod, port)| Upstream {
            pod: pod.name.clone(),
            uid: pod.uid.clone(),
            port,
        });
        if next != current {
            self.set_upstream(next);
        }
        None
    }

    /// A Service changed: it may select other pods or map its port
    /// elsewhere. One that no longer can is shown as having no pod, not
    /// ended: only its deletion ends the forward.
    fn service_changed(&mut self, service: &Service) {
        match service_route(service, self.request.port) {
            Ok((selector, remote)) => {
                let selector = PodSelector::Labels(selector);
                if selector != self.selector {
                    self.pods = self
                        .watches
                        .subscribe(&self.request.namespace, selector.clone());
                    self.selector = selector;
                }
                self.remote = remote;
                self.reroute();
            }
            Err(failure) => {
                if self.route.borrow().upstream.is_some() {
                    self.set_upstream(None);
                }
                self.status
                    .send_modify(|status| status.last_error = Some(failure.message));
            }
        }
    }

    fn set_upstream(&mut self, upstream: Option<Upstream>) {
        let state = match &upstream {
            Some(upstream) => ForwardState::Listening {
                pod: upstream.pod.clone(),
                port: upstream.port,
            },
            None => ForwardState::NoReadyPod,
        };
        self.route.send_modify(|route| {
            route.generation += 1;
            route.upstream = upstream;
        });
        self.status.send_modify(|status| status.state = state);
    }
}

async fn next_change(targets: &mut Option<TargetWatch>) -> Option<TargetChange> {
    match targets {
        Some(targets) => targets.changes.recv().await,
        None => std::future::pending().await,
    }
}

async fn accept(
    listener: Option<&TcpListener>,
) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
    match listener {
        Some(listener) => listener.accept().await,
        None => std::future::pending().await,
    }
}
