//! What the crate's tests share: waiting for work on a real Tokio runtime.

use gpui_kit::TestAppContext;
use std::time::{Duration, Instant};

/// The only wall-clock time a test lets real I/O take: the same allowance
/// as `freshkube_ui::real_io::REAL_IO_ALLOWANCE`, which this crate cannot
/// name. The fakes answer on loopback or a channel in milliseconds, so the
/// wait normally ends at once; the allowance is for a runner that stalls for
/// seconds, and a real hang costs only this long before it fails.
const REAL_IO_ALLOWANCE: Duration = Duration::from_secs(30);

/// Runs GPUI's executor until `done`, while Tokio works on its own threads,
/// or fails after [`REAL_IO_ALLOWANCE`] of real time.
pub(crate) fn until(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    let started = Instant::now();
    loop {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(
            started.elapsed() < REAL_IO_ALLOWANCE,
            "the awaited state never held within {REAL_IO_ALLOWANCE:?} of real time"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
