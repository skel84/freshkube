//! Example mode's forwards: a real loopback listener, bound by the same
//! rule as a live forward, that answers each connection with a short fixed
//! HTTP page naming the example pod and port. The example pods listen on
//! 8080 only, so any other pod port answers like a refused connection. An
//! app without a Ready pod (the batch jobs) keeps listening as "No ready
//! pod", as a live forward does.

use std::sync::Arc;
use std::time::Duration;

use freshkube_core::resources::{
    FailureKind, ForwardEnd, ForwardFailure, ForwardFailureKind, ForwardRequest, ForwardState,
    ForwardStatus, ForwardTarget, Listeners, POD_WAIT, listen_local,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::resources::example;
use crate::resources::model::ResourceIdentity;

/// The one port the example pods serve.
const SERVED: u16 = 8080;
/// How long a connection may take to send its request.
const READ_DEADLINE: Duration = Duration::from_secs(2);

/// Where an example forward's connections go: a pod and the port on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Route {
    pub(super) pod: String,
    pub(super) port: u16,
}

/// Resolves a target to an example pod of `context` as a live forward
/// would: a pod is itself once it runs, a Service or workload of an example
/// app its first Ready pod. `None` is an app without one.
pub(super) fn route(
    context: &str,
    identity: &ResourceIdentity,
    target: &ForwardTarget,
    port: u16,
) -> Result<Option<Route>, ForwardFailure> {
    let name = match target {
        ForwardTarget::Pod { name, .. } => {
            let Some(phase) = example::pod_phase(context, name) else {
                return Err(ForwardFailure::new(
                    ForwardFailureKind::Request(FailureKind::NotFound),
                    "The pod was deleted",
                ));
            };
            if phase != "Running" {
                return Err(ForwardFailure::new(
                    ForwardFailureKind::NotRunning,
                    format!("The pod is {phase}"),
                ));
            }
            return Ok(Some(Route {
                pod: name.clone(),
                port,
            }));
        }
        ForwardTarget::Service { name } | ForwardTarget::Workload { name, .. } => name,
    };
    if !example::runs_app(&identity.namespace, name) {
        return Err(ForwardFailure::new(
            ForwardFailureKind::NoPods,
            format!("The {} selects no pods", target.label()),
        ));
    }
    let port = match target {
        ForwardTarget::Service { .. } => service_target(identity, port)?,
        _ => port,
    };
    Ok(example::ready_pod(context, &identity.namespace, name).map(|pod| Route { pod, port }))
}

/// The pod port behind an example Service's port.
fn service_target(identity: &ResourceIdentity, port: u16) -> Result<u16, ForwardFailure> {
    let declared = example::document(identity, crate::resources::live::now())
        .and_then(|document| document.overview.ports)
        .unwrap_or_default();
    let Some(declared) = declared.into_iter().find(|declared| declared.port == port) else {
        return Err(ForwardFailure::new(
            ForwardFailureKind::Request(FailureKind::NotFound),
            format!("The Service has no port {port}"),
        ));
    };
    Ok(declared.target.parse().unwrap_or(port))
}

/// A running example forward, shaped like a live one.
pub(super) struct ExampleForward {
    pub(super) local_port: u16,
    pub(super) status: watch::Receiver<ForwardStatus>,
    pub(super) guard: ExampleGuard,
}

pub(super) struct ExampleGuard {
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl ExampleGuard {
    /// Stops listening; the handle completes once the port is free.
    pub(super) fn stop(mut self) -> JoinHandle<()> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.task
            .take()
            .expect("a guard owns its task until stopped")
    }
}

impl Drop for ExampleGuard {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Listens for `request` on Tokio and answers as `route` would.
pub(super) fn start(
    runtime: &Handle,
    request: &ForwardRequest,
    route: Option<Route>,
) -> Result<ExampleForward, ForwardFailure> {
    let listeners = {
        let _tokio = runtime.enter();
        listen_local(request.local_port, request.port)?
    };
    let local_port = listeners.port;
    let state = match &route {
        Some(route) => ForwardState::Listening {
            pod: route.pod.clone(),
            port: route.port,
        },
        None => ForwardState::NoReadyPod,
    };
    let (status, receiver) = watch::channel(ForwardStatus {
        state,
        connections: 0,
        last_error: None,
    });
    let (stop, stopping) = oneshot::channel();
    let task = runtime.spawn(serve(listeners, route, Arc::new(status), stopping));
    Ok(ExampleForward {
        local_port,
        status: receiver,
        guard: ExampleGuard {
            stop: Some(stop),
            task: Some(task),
        },
    })
}

async fn serve(
    listeners: Listeners,
    route: Option<Route>,
    status: Arc<watch::Sender<ForwardStatus>>,
    mut stop: oneshot::Receiver<()>,
) {
    let route = Arc::new(route);
    loop {
        let accepted = tokio::select! {
            biased;
            _ = &mut stop => break,
            accepted = listeners.v4.accept() => accepted,
            accepted = accept(listeners.v6.as_ref()) => accepted,
        };
        if let Ok((stream, _)) = accepted {
            tokio::spawn(answer(stream, route.clone(), status.clone()));
        }
    }
    drop(listeners);
    status.send_modify(|status| status.state = ForwardState::Ended(ForwardEnd::Stopped));
}

async fn accept(
    listener: Option<&TcpListener>,
) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
    match listener {
        Some(listener) => listener.accept().await,
        None => std::future::pending().await,
    }
}

/// One connection: the page, a refusal for a port the pods don't serve, or
/// with no Ready pod a wait that ends with nothing.
async fn answer(
    mut stream: TcpStream,
    route: Arc<Option<Route>>,
    status: Arc<watch::Sender<ForwardStatus>>,
) {
    status.send_modify(|status| status.connections += 1);
    let Some(route) = route.as_ref() else {
        tokio::time::sleep(POD_WAIT).await;
        drop(stream);
        status.send_modify(|status| status.connections = status.connections.saturating_sub(1));
        return;
    };
    // Read the request either way, as a live relay would, so closing sends
    // an end rather than a reset.
    let mut request = [0; 1024];
    let _ = tokio::time::timeout(READ_DEADLINE, stream.read(&mut request)).await;
    if route.port == SERVED {
        let _ = stream.write_all(page(route).as_bytes()).await;
        let _ = stream.shutdown().await;
    } else {
        let message = format!("Connection refused in the pod on port {}", route.port);
        status.send_modify(|status| status.last_error = Some(message));
    }
    drop(stream);
    status.send_modify(|status| status.connections = status.connections.saturating_sub(1));
}

/// The fixed answer, as HTTP so a browser shows it.
pub(super) fn page(route: &Route) -> String {
    let body = format!(
        "Example forward to {} port {}.\nFreshkube's example data answers here; nothing reaches a cluster.\n",
        route.pod, route.port
    );
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
