//! The machine waking from sleep (#968): what the shell does about it.
//!
//! `freshkube_core::wake::GapDetector` decides from the two clocks; this
//! reads them on a tick, and on a wake does what a Refresh the person asked
//! for does (the session-owned summary relists, the Talos overview reads
//! again, and the shown page refreshes) and says "Reconnecting…" in the
//! status bar until the first answer lands. Only what is already reading
//! reads again: a hidden page is not started, a stopped log stays stopped.

use super::*;
use freshkube_core::wake::{GapDetector, TICK};

/// How long the status bar keeps saying "Reconnecting…" with no answer. The
/// answer that lands, or the failure it reports, replaces it sooner; this
/// only stops a read that never answers from holding the words forever.
pub(super) const RECONNECTING_LIMIT: Duration = Duration::from_secs(45);

/// A wake this soon after the last (by the executor's clock) is the same
/// one: an NTP step right after waking, or a clock that keeps stepping,
/// relists nothing again.
pub(super) const WAKE_DEBOUNCE: Duration = Duration::from_secs(60);

impl Pilot {
    /// Watches the clocks for the rest of the window's life.
    pub(super) fn start_wake_watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A test steps the clocks itself, through `watch_wake`.
        #[cfg(not(test))]
        self.watch_wake(wall_clock(), window, cx);
        #[cfg(test)]
        let _ = (window, cx);
    }

    /// Reads the executor's clock and `wall` (the time since any fixed
    /// origin) every [`TICK`], and runs [`woke`](Self::woke) when the wall
    /// clock ran on without the executor's. The loop ends with the shell:
    /// every tick reaches it, and a dropped one stops the loop.
    pub(super) fn watch_wake(
        &mut self,
        wall: impl Fn() -> Duration + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let origin = cx.background_executor().now();
            let mut detector = GapDetector::new();
            let mut last_wake = None;
            loop {
                cx.background_executor().timer(TICK).await;
                let now = cx.background_executor().now();
                let slept = detector
                    .observe(now.duration_since(origin), wall())
                    .filter(|_| last_wake.is_none_or(|at| now.duration_since(at) >= WAKE_DEBOUNCE));
                if slept.is_some() {
                    last_wake = Some(now);
                }
                let alive = this.update_in(cx, |view, window, cx| {
                    if let Some(slept) = slept {
                        view.woke(slept, window, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// The machine woke after `slept`: the live data may be old, so say so
    /// and read again, if there is a connection to read again.
    pub(super) fn woke(&mut self, slept: Duration, window: &mut Window, cx: &mut Context<Self>) {
        tracing::info!("the machine woke after about {} s", slept.as_secs());
        if self.config_loading || self.config_error.is_some() || !self.has_connection() {
            // Nothing is connected to reconnect; the load in progress, the
            // error or the choice of a context on screen is the whole story.
            return;
        }
        self.chrome.reconnecting = Some(Duration::ZERO);
        self.refresh_now(window, cx);
        cx.notify();
    }

    /// Called from the one-second tick: stops saying "Reconnecting…" once
    /// it has gone on for [`RECONNECTING_LIMIT`].
    pub(super) fn tick_reconnecting(&mut self, cx: &mut Context<Self>) {
        let Some(waited) = self.chrome.reconnecting.as_mut() else {
            return;
        };
        *waited += Duration::from_secs(1);
        if *waited >= RECONNECTING_LIMIT {
            self.reconnected(cx);
        }
    }

    /// An answer landed (or the wait ran out): the usual status shows again.
    pub(super) fn reconnected(&mut self, cx: &mut Context<Self>) {
        if self.chrome.reconnecting.take().is_some() {
            cx.notify();
        }
    }
}

/// The wall clock as the time since the Unix epoch.
#[cfg(not(test))]
fn wall_clock() -> impl Fn() -> Duration {
    || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
