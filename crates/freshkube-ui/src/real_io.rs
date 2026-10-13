//! Waiting in a test for work that runs on a real Tokio runtime or a real
//! socket: a fake server, a file written by a worker thread.
//!
//! `TestAppContextExt::wait_for` counts its timeout on the test clock, which
//! steps 10 ms per poll without waiting for anything. Against a real
//! runtime that burns the whole timeout in a few real milliseconds, so a
//! slow runner fails the test before the answer lands. [`RealIo::wait_for_io`]
//! polls the same way, but gives each poll 10 ms of real time too (a test
//! clock timer in the code under test still fires, at the rate the app would
//! see) and fails only after [`REAL_IO_ALLOWANCE`] of wall-clock time.
//!
//! The predicate must name what the assertion needs: the drawn element, the
//! fake's recorded request, or the entity state the test then asserts. Never
//! a proxy for it, and never `run_until_parked` alone after work that runs
//! on a real runtime, which can return before that work answers.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, App, AppContext, TestAppContext, Window};
use std::time::{Duration, Instant};

/// The only wall-clock time a test lets real I/O take.
///
/// The fakes answer on loopback in milliseconds, so the wait normally ends
/// at once. The allowance is for a runner that stalls (a loaded CI machine
/// or a descheduled process pauses a test for seconds); 30 s is several
/// times the worst stall seen, and a real hang costs only this long before
/// it fails with a message.
pub const REAL_IO_ALLOWANCE: Duration = Duration::from_secs(30);

/// One poll's step, on the test clock and in real time.
const POLL: Duration = Duration::from_millis(10);

/// Runs the executor until `done`, for work on a real runtime that needs no
/// window. Panics after [`REAL_IO_ALLOWANCE`] of real time, naming `what`.
pub fn until_io(
    cx: &mut TestAppContext,
    what: &str,
    mut done: impl FnMut(&mut TestAppContext) -> bool,
) {
    let started = Instant::now();
    loop {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(
            started.elapsed() < REAL_IO_ALLOWANCE,
            "{what} never held within {REAL_IO_ALLOWANCE:?} of real time"
        );
        std::thread::sleep(POLL);
    }
}

pub trait RealIo {
    /// Draws frames until `ready` holds, panicking after [`REAL_IO_ALLOWANCE`]
    /// of real time.
    fn wait_for_io(
        &mut self,
        window: AnyWindowHandle,
        ready: impl FnMut(&mut Window, &mut App) -> bool,
    ) -> impl std::future::Future<Output = ()>;
}

impl RealIo for TestAppContext {
    async fn wait_for_io(
        &mut self,
        window: AnyWindowHandle,
        mut ready: impl FnMut(&mut Window, &mut App) -> bool,
    ) {
        let started = Instant::now();
        loop {
            let done = self
                .update_window(window, |_, window, cx| {
                    window.render_frame(cx);
                    ready(window, cx)
                })
                .expect("test window closed while waiting");
            if done {
                return;
            }
            assert!(
                started.elapsed() < REAL_IO_ALLOWANCE,
                "the awaited state never held within {REAL_IO_ALLOWANCE:?} of real time"
            );
            std::thread::sleep(POLL);
            self.executor().timer(POLL).await;
        }
    }
}
