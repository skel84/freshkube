//! One read's data for one identity and the work that fills it.

use std::{future::Future, time::Duration};

use freshkube_core::{
    job::{OwnedJob, spawn_job},
    snapshot::{Request, Snapshot},
};
use gpui_kit::*;
use tokio::runtime::Handle;

/// One read's data for one identity, such as a screen's target: a
/// [`Snapshot`] plus the work that fills it.
pub struct Loader<T, I> {
    state: Snapshot<T, I, String>,
    job: Option<OwnedJob>,
    task: Option<Task<()>>,
    /// Rises whenever what [`data`](Self::data) or [`error`](Self::error)
    /// return may have changed, so a screen derives its display data again
    /// only then.
    revision: u64,
}

impl<T, I> Default for Loader<T, I> {
    fn default() -> Self {
        Self {
            state: Snapshot::default(),
            job: None,
            task: None,
            revision: 0,
        }
    }
}

impl<T: Send + 'static, I: Clone + Eq + 'static> Loader<T, I> {
    /// Forgets data and cancels in-flight work, e.g. when the identity changes.
    pub fn reset(&mut self) {
        let revision = self.revision + 1;
        *self = Self::default();
        self.revision = revision;
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Runs `work` on Tokio for `identity`, for at most `deadline`. The
    /// result lands only if this is still the latest request for that
    /// identity; `slot` finds the loader again inside the view when it does.
    /// `what` names the data in lowercase with its article, e.g. "the
    /// process list".
    #[allow(clippy::too_many_arguments)]
    pub fn load<V: 'static>(
        &mut self,
        identity: I,
        runtime: &Handle,
        deadline: Duration,
        what: &str,
        work: impl Future<Output = Result<T, String>> + Send + 'static,
        slot: impl Fn(&mut V) -> &mut Self + 'static,
        cx: &mut Context<V>,
    ) {
        let request = self.state.begin(identity);
        self.revision += 1;
        let (job, receiver) =
            spawn_job(runtime, deadline, format!("Loading {what} timed out"), work);
        self.job = Some(job);
        let stopped = format!("Loading {what} stopped unexpectedly");
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = receiver.await.unwrap_or_else(|_| Err(stopped));
            _ = this.update(cx, |view, cx| {
                slot(view).deliver(&request, result);
                cx.notify();
            });
        }));
    }

    /// Applies a result at once; used for example data.
    pub fn resolve(&mut self, identity: I, result: Result<T, String>) {
        self.job = None;
        self.task = None;
        let request = self.state.begin(identity);
        self.state.apply(&request, result);
        self.revision += 1;
    }

    pub fn data(&self) -> Option<&T> {
        self.state.data()
    }

    /// The data to change in place, as a newer observation arrives, without
    /// a request; the revision stays.
    pub fn data_mut(&mut self) -> Option<&mut T> {
        self.state.data_mut()
    }

    /// Starts waiting for an answer for `identity` without asking for one,
    /// for tests that hold a screen on its loading rows or answer it late
    /// through [`apply`](Self::apply).
    #[cfg(any(test, feature = "testing"))]
    pub fn begin(&mut self, identity: I) -> Request<I> {
        self.state.begin(identity)
    }

    /// Lands `result` as an answer from [`load`](Self::load) does, for tests
    /// that answer a request late.
    #[cfg(any(test, feature = "testing"))]
    pub fn apply(&mut self, request: &Request<I>, result: Result<T, String>) -> bool {
        self.deliver(request, result)
    }

    /// Lands `result` if `request` is still the latest for its identity.
    fn deliver(&mut self, request: &Request<I>, result: Result<T, String>) -> bool {
        let applied = self.state.apply(request, result);
        if applied {
            self.job = None;
            self.revision += 1;
        }
        applied
    }

    pub fn error(&self) -> Option<&str> {
        self.state.error().map(String::as_str)
    }

    pub fn is_loading(&self) -> bool {
        self.state.is_loading()
    }

    /// Whether the data is kept from an earlier answer after a later read
    /// for the same identity failed, for tests.
    #[cfg(any(test, feature = "testing"))]
    pub fn is_stale(&self) -> bool {
        self.state.is_stale()
    }

    pub fn last_successful(&self) -> Option<std::time::SystemTime> {
        self.state.last_successful()
    }

    pub fn last_failure(&self) -> Option<std::time::SystemTime> {
        self.state.last_failure()
    }
}

#[cfg(test)]
mod tests;
