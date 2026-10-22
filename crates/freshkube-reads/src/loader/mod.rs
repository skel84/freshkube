//! One read's data for one identity and the work that fills it.

use std::{
    future::Future,
    time::{Duration, Instant},
};

use freshkube_core::{
    job::{OwnedJob, spawn_job},
    snapshot::{Request, Snapshot},
};
use gpui_kit::*;
use tokio::runtime::Handle;

use crate::OnShow;

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
    /// Whether the owner shows it; see [`set_visible`](Self::set_visible).
    visible: bool,
    /// A read hiding dropped before it answered: the snapshot still waits
    /// for it, but nothing is loading.
    cut: bool,
    /// When the last successful answer from [`load`](Self::load) or
    /// [`land`](Self::land) landed,
    /// by the executor's clock. A failure clears it: a refresh that failed
    /// doesn't make old data new.
    answered: Option<Instant>,
}

impl<T, I> Default for Loader<T, I> {
    fn default() -> Self {
        Self {
            state: Snapshot::default(),
            job: None,
            task: None,
            revision: 0,
            visible: true,
            cut: false,
            answered: None,
        }
    }
}

impl<T: Send + 'static, I: Clone + Eq + 'static> Loader<T, I> {
    /// Forgets data and cancels in-flight work, e.g. when the identity changes.
    pub fn reset(&mut self) {
        let revision = self.revision + 1;
        let visible = self.visible;
        *self = Self::default();
        self.revision = revision;
        self.visible = visible;
    }

    /// Invalidates a changed route of the same logical target, retaining its
    /// evidence as stale. Never use for access/source/setup replacement.
    pub fn retain_stale(&mut self, identity: I) {
        self.job = None;
        self.task = None;
        self.cut = true;
        self.answered = None;
        self.state.begin_retaining(identity);
        self.revision += 1;
    }

    /// Hiding drops the read in flight and keeps the data as last known;
    /// showing again returns whether the owner should read again, as
    /// `on_show` says, by the age of the last successful answer. A failed
    /// answer, an answer from [`resolve`](Self::resolve) and a read hiding
    /// cut short have no age and count as old. Returns false when nothing
    /// changed.
    pub fn set_visible(&mut self, visible: bool, on_show: OnShow, cx: &App) -> bool {
        if self.visible == visible {
            return false;
        }
        self.visible = visible;
        if !visible {
            self.job = None;
            self.task = None;
            // A read waiting, with work or held, waits no more: what shows
            // it derives again.
            if self.state.is_loading() && !self.cut {
                self.cut = true;
                self.revision += 1;
            }
            return false;
        }
        let since = self.answered.filter(|_| !self.cut);
        on_show.reads_again(since, cx.background_executor().now())
    }

    /// Whether the owner shows it, as [`set_visible`](Self::set_visible)
    /// last said; a new loader is shown. For tests: an owner keeps its own.
    #[cfg(any(test, feature = "testing"))]
    pub fn is_visible(&self) -> bool {
        self.visible
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
        let request = self.start(identity);
        let (job, receiver) =
            spawn_job(runtime, deadline, format!("Loading {what} timed out"), work);
        let stopped = format!("Loading {what} stopped unexpectedly");
        let task = cx.spawn(async move |this, cx| {
            let result = receiver.await.unwrap_or_else(|_| Err(stopped));
            _ = this.update(cx, |view, cx| {
                slot(view).land(&request, result, cx);
                cx.notify();
            });
        });
        self.run(Some(job), task);
    }

    /// Begins a read for `identity` that the owner runs and lands itself,
    /// for one whose answer needs more than [`load`](Self::load) gives:
    /// work of its own, a window, or derived data as it lands. The read in
    /// flight is dropped. Hand the work to [`run`](Self::run) and the answer
    /// to [`land`](Self::land); a read given no work, as example data held
    /// on its loading rows, waits until it is replaced or hidden.
    pub fn start(&mut self, identity: I) -> Request<I> {
        self.job = None;
        self.task = None;
        self.cut = false;
        self.revision += 1;
        self.state.begin(identity)
    }

    /// Keeps the work of the read [`start`](Self::start) began: its Tokio
    /// job, if any, and the task that brings its answer. Replacing the read,
    /// hiding or dropping the loader drops both.
    pub fn run(&mut self, job: Option<OwnedJob>, task: Task<()>) {
        self.job = job;
        self.task = Some(task);
    }

    /// Lands `result` if `request` is still the latest for its identity
    /// and hiding didn't cut it short, and returns whether it did. A successful answer is timed by the
    /// executor's clock, for [`set_visible`](Self::set_visible).
    pub fn land(&mut self, request: &Request<I>, result: Result<T, String>, cx: &App) -> bool {
        // Hiding cut the read short: its answer, however late, never lands.
        if self.cut {
            return false;
        }
        let ok = result.is_ok();
        let landed = self.deliver(request, result);
        if landed && ok {
            self.answered = Some(cx.background_executor().now());
        }
        landed
    }

    /// Counts what was read as old, as when where it is read from changes:
    /// showing again reads, whatever the policy's threshold.
    pub fn expire(&mut self) {
        self.answered = None;
    }

    /// Applies a result at once; used for example data.
    pub fn resolve(&mut self, identity: I, result: Result<T, String>) {
        self.job = None;
        self.task = None;
        self.cut = false;
        self.answered = None;
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
        self.cut = false;
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
        let failed = result.is_err();
        let applied = self.state.apply(request, result);
        if applied {
            if failed {
                self.answered = None;
            }
            self.job = None;
            self.cut = false;
            self.revision += 1;
        }
        applied
    }

    pub fn error(&self) -> Option<&str> {
        self.state.error().map(String::as_str)
    }

    pub fn is_loading(&self) -> bool {
        self.state.is_loading() && !self.cut
    }

    /// Whether the data is kept from an earlier answer after a later read
    /// for the same identity failed.
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
