//! A screen's data for one target and the work that fills it.

use std::future::Future;

use gpui_kit::*;
use tokio::runtime::Handle;

use super::SCREEN_DEADLINE;
use crate::backend::{self, OwnedJob, Target};
use crate::state::{Request, Snapshot};

/// One screen's data for one target: a [`Snapshot`] plus the work that fills it.
pub(crate) struct Loader<T> {
    state: Snapshot<T, Target>,
    job: Option<OwnedJob>,
    task: Option<Task<()>>,
    /// Rises whenever what [`data`](Self::data) or [`error`](Self::error)
    /// return may have changed, so a screen derives its display data again
    /// only then.
    revision: u64,
}

impl<T> Default for Loader<T> {
    fn default() -> Self {
        Self {
            state: Snapshot::default(),
            job: None,
            task: None,
            revision: 0,
        }
    }
}

impl<T: Send + 'static> Loader<T> {
    /// Forgets data and cancels in-flight work, e.g. when the target changes.
    pub(crate) fn reset(&mut self) {
        let revision = self.revision + 1;
        *self = Self::default();
        self.revision = revision;
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Runs `work` on Tokio for `identity`. The result lands only if this is
    /// still the latest request for that identity; `slot` finds the loader
    /// again inside the screen when it does. `what` names the data in
    /// lowercase with its article, e.g. "the process list".
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn load<V: 'static>(
        &mut self,
        identity: Target,
        runtime: &Handle,
        what: &str,
        work: impl Future<Output = Result<T, String>> + Send + 'static,
        slot: fn(&mut V) -> &mut Loader<T>,
        cx: &mut Context<V>,
    ) {
        let request = self.state.begin(identity);
        self.revision += 1;
        let (job, receiver) = backend::spawn_job(
            runtime,
            SCREEN_DEADLINE,
            format!("Loading {what} timed out"),
            work,
        );
        self.job = Some(job);
        let stopped = format!("Loading {what} stopped unexpectedly");
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = receiver.await.unwrap_or_else(|_| Err(stopped));
            _ = this.update(cx, |view, cx| {
                slot(view).apply(&request, result);
                cx.notify();
            });
        }));
    }

    /// Applies a result at once; used for example data.
    pub(crate) fn resolve(&mut self, identity: Target, result: Result<T, String>) {
        self.job = None;
        self.task = None;
        let request = self.state.begin(identity);
        self.state.apply(&request, result);
        self.revision += 1;
    }

    pub(crate) fn data(&self) -> Option<&T> {
        self.state.data()
    }

    /// The data to change in place, as a newer observation arrives, without
    /// a request; the revision stays.
    pub(crate) fn data_mut(&mut self) -> Option<&mut T> {
        self.state.data_mut()
    }

    /// Starts waiting for an answer for `identity` without asking for one,
    /// for tests that hold a screen on its loading rows or answer it late
    /// through [`apply`](Self::apply).
    pub(crate) fn begin(&mut self, identity: Target) -> Request<Target> {
        self.state.begin(identity)
    }

    /// Lands `result` if `request` is still the latest for its identity, as
    /// an answer from [`load`](Self::load) does.
    pub(crate) fn apply(&mut self, request: &Request<Target>, result: Result<T, String>) -> bool {
        let applied = self.state.apply(request, result);
        if applied {
            self.job = None;
            self.revision += 1;
        }
        applied
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.state.error()
    }

    pub(crate) fn is_loading(&self) -> bool {
        self.state.is_loading()
    }

    pub(crate) fn last_successful(&self) -> Option<std::time::SystemTime> {
        self.state.last_successful()
    }

    pub(crate) fn last_failure(&self) -> Option<std::time::SystemTime> {
        self.state.last_failure()
    }
}
