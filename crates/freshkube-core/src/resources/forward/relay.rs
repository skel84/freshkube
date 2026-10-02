//! One local connection: its own websocket to the pod's port, bytes both
//! ways, and closing both ends when either side ends, the pod changes or
//! the forward ends.

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::FusedFuture;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::client::UpgradeConnectionError;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};

use super::{
    ForwardFailure, ForwardFailureKind, ForwardStatus, POD_WAIT, REQUEST_DEADLINE, Route, Upstream,
};
use crate::resources::failure::{Failure, FailureKind};

/// How long the websocket may take to close once the connection is done.
pub(super) const CLOSE_DEADLINE: Duration = Duration::from_secs(1);

/// Counts a connection as open for as long as it lives.
struct Open<'a>(&'a watch::Sender<ForwardStatus>);

impl<'a> Open<'a> {
    fn new(status: &'a watch::Sender<ForwardStatus>) -> Self {
        status.send_modify(|status| status.connections += 1);
        Self(status)
    }
}

impl Drop for Open<'_> {
    fn drop(&mut self) {
        self.0
            .send_modify(|status| status.connections = status.connections.saturating_sub(1));
    }
}

/// Relays one accepted connection until it ends. A refusal that every
/// connection would meet goes to `fatal`, which ends the forward.
pub(super) async fn connection(
    mut local: TcpStream,
    api: Api<Pod>,
    mut route: watch::Receiver<Route>,
    status: Arc<watch::Sender<ForwardStatus>>,
    fatal: mpsc::Sender<ForwardFailure>,
) {
    let _open = Open::new(&status);
    let Some((generation, upstream)) = wait_for_upstream(&mut route).await else {
        return;
    };
    let ports = [upstream.port];
    let opened = tokio::select! {
        opened = tokio::time::timeout(REQUEST_DEADLINE, api.portforward(&upstream.pod, &ports)) => opened,
        _ = moved(&mut route, generation) => return,
    };
    let mut forwarder = match opened {
        Ok(Ok(forwarder)) => forwarder,
        Ok(Err(error)) => return refused(error, &upstream, &status, &fatal),
        Err(_) => return set_error(&status, "Opening the forward timed out".to_owned()),
    };
    let (Some(mut remote), Some(errors)) = (
        forwarder.take_stream(upstream.port),
        forwarder.take_error(upstream.port),
    ) else {
        forwarder.abort();
        return;
    };
    let mut errors = Box::pin(errors.fuse());
    tokio::select! {
        _ = tokio::io::copy_bidirectional(&mut local, &mut remote) => {}
        Some(message) = &mut errors => set_error(&status, port_error(&message, upstream.port)),
        _ = moved(&mut route, generation) => {}
    }
    drop(local);
    // Dropping our end of the port makes kube close the websocket, which
    // tells the API server to close the connection in the pod. The error
    // channel closes once kube's side has finished.
    drop(remote);
    if !errors.is_terminated() {
        let _ = tokio::time::timeout(CLOSE_DEADLINE, &mut errors).await;
    }
    forwarder.abort();
}

/// The pod to connect to, waiting up to [`POD_WAIT`] while there is none.
/// `None` once the forward ended or nothing turned up.
async fn wait_for_upstream(route: &mut watch::Receiver<Route>) -> Option<(u64, Upstream)> {
    let wait = async {
        loop {
            {
                let now = route.borrow_and_update();
                if now.closed {
                    return None;
                }
                if let Some(upstream) = &now.upstream {
                    return Some((now.generation, upstream.clone()));
                }
            }
            route.changed().await.ok()?;
        }
    };
    tokio::time::timeout(POD_WAIT, wait).await.ok().flatten()
}

/// Resolves once the forward moved to another pod, or ended.
pub(super) async fn moved(route: &mut watch::Receiver<Route>, generation: u64) {
    loop {
        {
            let now = route.borrow_and_update();
            if now.closed || now.generation != generation {
                return;
            }
        }
        if route.changed().await.is_err() {
            return;
        }
    }
}

/// A refused forward. Being forbidden holds for every connection, so it
/// ends the forward; anything else is this connection's error.
fn refused(
    error: kube::Error,
    upstream: &Upstream,
    status: &watch::Sender<ForwardStatus>,
    fatal: &mpsc::Sender<ForwardFailure>,
) {
    let message = match error {
        kube::Error::UpgradeConnection(UpgradeConnectionError::ProtocolSwitch(code)) => {
            match code.as_u16() {
                401 => {
                    let failure = Failure::new(
                        FailureKind::Unauthorized,
                        "The API server rejected the credentials",
                    );
                    let _ = fatal.try_send(failure.into());
                    return;
                }
                403 => {
                    let failure = ForwardFailure::new(
                        ForwardFailureKind::Request(FailureKind::Forbidden),
                        "Not allowed to forward ports of this pod",
                    );
                    let _ = fatal.try_send(failure);
                    return;
                }
                404 => format!("The pod {} was not found", upstream.pod),
                _ => format!("The API server refused the forward ({code})"),
            }
        }
        kube::Error::UpgradeConnection(error) => format!("The forward didn't open · {error}"),
        other => Failure::from_kube(other).to_string(),
    };
    set_error(status, message);
}

fn set_error(status: &watch::Sender<ForwardStatus>, message: String) {
    status.send_modify(|status| status.last_error = Some(message));
}

/// What the server said about the port, shortened when it is the usual
/// "nothing listens there".
pub(super) fn port_error(message: &str, port: u16) -> String {
    if message.to_lowercase().contains("connection refused") {
        return format!("Connection refused in the pod on port {port}");
    }
    message.lines().next().unwrap_or_default().trim().to_owned()
}
