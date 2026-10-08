//! Work the UI starts on Tokio and owns: dropping the owner cancels it.

use std::{future::Future, time::Duration};

use tokio::{runtime::Handle, sync::oneshot, task::JoinHandle};

/// The UI owns this guard for exactly as long as the request is relevant.
pub struct OwnedJob {
    task: JoinHandle<()>,
}

impl OwnedJob {
    pub fn new(task: JoinHandle<()>) -> Self {
        Self { task }
    }
}

impl Drop for OwnedJob {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Runs `work` on Tokio under `deadline`. Dropping the job or the receiver
/// cancels it, so a screen that moves on never receives a late result.
pub fn spawn_job<T, F>(
    runtime: &Handle,
    deadline: Duration,
    timeout_message: String,
    work: F,
) -> (OwnedJob, oneshot::Receiver<Result<T, String>>)
where
    T: Send + 'static,
    F: Future<Output = Result<T, String>> + Send + 'static,
{
    let (mut sender, receiver) = oneshot::channel();
    let task = runtime.spawn(async move {
        tokio::select! {
            biased;
            _ = sender.closed() => {}
            result = tokio::time::timeout(deadline, work) => {
                let _ = sender.send(result.unwrap_or_else(|_| Err(timeout_message)));
            }
        }
    });
    (OwnedJob::new(task), receiver)
}
