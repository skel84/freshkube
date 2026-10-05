//! How long a process takes to draw its first frame of something, for
//! comparing how fast a page or a story opens. Debug builds only, and
//! silent unless `FRESHKUBE_FIRST_FRAME=1`:
//!
//! ```ignore
//! // first thing in main
//! freshkube_probe::first_frame::start();
//! // where the frame that matters is drawn
//! static PODS: FirstFrame = FirstFrame::new("pods");
//! if PODS.pending() {
//!     window.on_next_frame(|_, _| PODS.mark());
//! }
//! ```
//!
//! `mark` prints `first frame: pods after 812 ms` to stderr, once.
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();
static ENABLED: OnceLock<bool> = OnceLock::new();

/// Marks the process's start; call it first thing in `main`.
pub fn start() {
    START.get_or_init(Instant::now);
}

/// One thing whose first frame is timed. Keep it in a `static`.
pub struct FirstFrame {
    name: &'static str,
    done: AtomicBool,
}

impl FirstFrame {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            done: AtomicBool::new(false),
        }
    }

    /// Whether the frame is still to be timed. After the first call that
    /// finds timing off or the frame marked, this is one atomic load.
    pub fn pending(&self) -> bool {
        if self.done.load(Ordering::Relaxed) {
            return false;
        }
        let on = *ENABLED
            .get_or_init(|| std::env::var("FRESHKUBE_FIRST_FRAME").is_ok_and(|value| value == "1"));
        if !on {
            self.done.store(true, Ordering::Relaxed);
        }
        on
    }

    /// Prints the time since [`start`], the first time only.
    pub fn mark(&self) {
        if !self.pending() || self.done.swap(true, Ordering::Relaxed) {
            return;
        }
        match START.get() {
            Some(start) => eprintln!(
                "first frame: {} after {} ms",
                self.name,
                start.elapsed().as_millis()
            ),
            None => eprintln!("first frame: {} (no start mark)", self.name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FirstFrame;
    use std::sync::atomic::Ordering;

    #[test]
    fn marks_once_then_is_done() {
        // The test runner doesn't set FRESHKUBE_FIRST_FRAME, so timing is
        // off and the first look settles it.
        let frame = FirstFrame::new("test");
        let on = frame.pending();
        assert_eq!(
            on,
            std::env::var("FRESHKUBE_FIRST_FRAME").as_deref() == Ok("1")
        );
        frame.mark();
        assert!(frame.done.load(Ordering::Relaxed));
        assert!(!frame.pending());
    }
}
