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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    /// Sets its flag when the future holding it is dropped.
    struct Dropped(Arc<AtomicBool>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn a_job_answers_within_its_deadline() {
        let (_job, receiver) = spawn_job(
            &Handle::current(),
            Duration::from_secs(5),
            "late".into(),
            async { Ok::<_, String>(7) },
        );
        assert_eq!(receiver.await.unwrap(), Ok(7));
    }

    #[tokio::test(start_paused = true)]
    async fn a_job_past_its_deadline_answers_with_the_timeout_message() {
        let (_job, receiver) = spawn_job(
            &Handle::current(),
            Duration::from_secs(5),
            "timed out".into(),
            std::future::pending::<Result<(), String>>(),
        );
        assert_eq!(receiver.await.unwrap(), Err("timed out".into()));
    }

    /// Starts work that never ends, holding a flag that its drop sets.
    fn endless() -> (
        OwnedJob,
        oneshot::Receiver<Result<(), String>>,
        Arc<AtomicBool>,
    ) {
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Dropped(dropped.clone());
        let (job, receiver) = spawn_job(
            &Handle::current(),
            Duration::from_secs(60),
            "late".into(),
            async move {
                let _guard = guard;
                std::future::pending().await
            },
        );
        (job, receiver, dropped)
    }

    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn dropping_the_job_cancels_the_work() {
        let (job, _receiver, dropped) = endless();
        settle().await;
        drop(job);
        settle().await;
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn dropping_the_receiver_cancels_the_work() {
        let (_job, receiver, dropped) = endless();
        settle().await;
        drop(receiver);
        settle().await;
        assert!(dropped.load(Ordering::SeqCst));
    }
}
