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
    state: Snapshot<T, I>,
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
                slot(view).apply(&request, result);
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
    pub fn begin(&mut self, identity: I) -> Request<I> {
        self.state.begin(identity)
    }

    /// Lands `result` if `request` is still the latest for its identity, as
    /// an answer from [`load`](Self::load) does.
    pub fn apply(&mut self, request: &Request<I>, result: Result<T, String>) -> bool {
        let applied = self.state.apply(request, result);
        if applied {
            self.job = None;
            self.revision += 1;
        }
        applied
    }

    pub fn error(&self) -> Option<&str> {
        self.state.error()
    }

    pub fn is_loading(&self) -> bool {
        self.state.is_loading()
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
mod tests {
    use std::{cell::Cell, rc::Rc, time::Duration};

    use gpui_kit::{AppContext, Entity, TestAppContext};
    use tokio::{runtime::Runtime, sync::oneshot};

    use super::Loader;

    const DEADLINE: Duration = Duration::from_secs(5);

    /// A view that owns one loader, as a screen does.
    struct Screen {
        loader: Loader<u32, &'static str>,
    }

    fn mount(cx: &mut TestAppContext) -> (Runtime, Entity<Screen>) {
        // Tokio answers from its own threads.
        cx.executor().allow_parking();
        let screen = cx.new(|_| Screen {
            loader: Loader::default(),
        });
        (Runtime::new().unwrap(), screen)
    }

    fn load(
        screen: &Entity<Screen>,
        runtime: &Runtime,
        identity: &'static str,
        deadline: Duration,
        answer: oneshot::Receiver<Result<u32, String>>,
        delivered: Rc<Cell<usize>>,
        cx: &mut TestAppContext,
    ) {
        screen.update(cx, |screen, cx| {
            screen.loader.load(
                identity,
                runtime.handle(),
                deadline,
                "the number",
                async move { answer.await.unwrap_or_else(|_| Err("closed".into())) },
                move |screen: &mut Screen| {
                    delivered.set(delivered.get() + 1);
                    &mut screen.loader
                },
                cx,
            );
        });
    }

    /// Runs both executors until `done`, or fails after a few seconds.
    fn until(cx: &mut TestAppContext, done: impl Fn() -> bool) {
        for _ in 0..500 {
            cx.run_until_parked();
            if done() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the loader never answered");
    }

    #[gpui_kit::test]
    fn an_answer_lands_for_the_latest_request(cx: &mut TestAppContext) {
        let (runtime, screen) = mount(cx);
        let (send, answer) = oneshot::channel();
        let delivered = Rc::new(Cell::new(0));
        load(
            &screen,
            &runtime,
            "a",
            DEADLINE,
            answer,
            delivered.clone(),
            cx,
        );
        let started = screen.read_with(cx, |screen, _| {
            assert!(screen.loader.is_loading());
            screen.loader.revision()
        });
        send.send(Ok(7)).unwrap();
        until(cx, || delivered.get() == 1);
        screen.read_with(cx, |screen, _| {
            assert_eq!(screen.loader.data(), Some(&7));
            assert!(!screen.loader.is_loading());
            assert!(screen.loader.last_successful().is_some());
            assert!(screen.loader.revision() > started);
        });
    }

    #[gpui_kit::test]
    fn a_late_answer_for_an_old_identity_is_dropped(cx: &mut TestAppContext) {
        let (runtime, screen) = mount(cx);
        let (old, old_answer) = oneshot::channel();
        let (new, new_answer) = oneshot::channel();
        let delivered = Rc::new(Cell::new(0));
        load(
            &screen,
            &runtime,
            "a",
            DEADLINE,
            old_answer,
            delivered.clone(),
            cx,
        );
        load(
            &screen,
            &runtime,
            "b",
            DEADLINE,
            new_answer,
            delivered.clone(),
            cx,
        );
        new.send(Ok(2)).unwrap();
        until(cx, || delivered.get() == 1);
        // The replaced read was cancelled: its answer has nowhere to go.
        let _ = old.send(Ok(1));
        std::thread::sleep(Duration::from_millis(50));
        cx.run_until_parked();
        assert_eq!(delivered.get(), 1);
        screen.read_with(cx, |screen, _| assert_eq!(screen.loader.data(), Some(&2)));

        // An answer that does arrive for an identity no longer asked for is
        // refused, and the newer one's data stays.
        screen.update(cx, |screen, _| {
            let stale = screen.loader.begin("a");
            screen.loader.resolve("b", Ok(3));
            let revision = screen.loader.revision();
            assert!(!screen.loader.apply(&stale, Ok(1)));
            assert_eq!(screen.loader.data(), Some(&3));
            assert_eq!(screen.loader.revision(), revision);
        });
    }

    #[gpui_kit::test]
    fn reset_clears_data_and_drops_the_read_in_flight(cx: &mut TestAppContext) {
        let (runtime, screen) = mount(cx);
        let (send, answer) = oneshot::channel();
        let delivered = Rc::new(Cell::new(0));
        let before = screen.update(cx, |screen, _| {
            screen.loader.resolve("a", Ok(1));
            screen.loader.revision()
        });
        load(
            &screen,
            &runtime,
            "a",
            DEADLINE,
            answer,
            delivered.clone(),
            cx,
        );
        screen.update(cx, |screen, _| screen.loader.reset());
        let _ = send.send(Ok(2));
        std::thread::sleep(Duration::from_millis(50));
        cx.run_until_parked();
        assert_eq!(delivered.get(), 0);
        screen.read_with(cx, |screen, _| {
            assert!(screen.loader.data().is_none());
            assert!(screen.loader.error().is_none());
            assert!(!screen.loader.is_loading());
            assert!(screen.loader.last_successful().is_none());
            // A view that derives by revision derives again.
            assert!(screen.loader.revision() > before);
        });
    }

    #[gpui_kit::test]
    fn a_refresh_failure_keeps_the_data_marked_stale(cx: &mut TestAppContext) {
        let (runtime, screen) = mount(cx);
        let (send, answer) = oneshot::channel();
        let delivered = Rc::new(Cell::new(0));
        screen.update(cx, |screen, _| screen.loader.resolve("a", Ok(1)));
        load(
            &screen,
            &runtime,
            "a",
            DEADLINE,
            answer,
            delivered.clone(),
            cx,
        );
        screen.read_with(cx, |screen, _| {
            assert!(screen.loader.is_loading());
            assert_eq!(screen.loader.data(), Some(&1), "kept while it refreshes");
        });
        send.send(Err("offline".into())).unwrap();
        until(cx, || delivered.get() == 1);
        screen.read_with(cx, |screen, _| {
            assert_eq!(screen.loader.data(), Some(&1));
            assert_eq!(screen.loader.error(), Some("offline"));
            assert!(screen.loader.is_stale());
            assert!(screen.loader.last_failure().is_some());
        });
    }

    #[gpui_kit::test]
    fn a_read_past_its_deadline_fails_in_its_own_words(cx: &mut TestAppContext) {
        let (runtime, screen) = mount(cx);
        let (_send, answer) = oneshot::channel();
        let delivered = Rc::new(Cell::new(0));
        let deadline = Duration::from_millis(20);
        load(
            &screen,
            &runtime,
            "a",
            deadline,
            answer,
            delivered.clone(),
            cx,
        );
        until(cx, || delivered.get() == 1);
        screen.read_with(cx, |screen, _| {
            assert!(screen.loader.data().is_none());
            assert_eq!(screen.loader.error(), Some("Loading the number timed out"));
        });
    }
}
