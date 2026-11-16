//! The network changing (#968): a VPN coming up or down, a move between
//! networks. `freshkube_core::reachability` hears the platform; this settles
//! a burst of reports into one change and does what a wake does, once: the
//! summary and the alert read relist, the Talos overview reads again and
//! the shown page refreshes, with "Reconnecting to <context>…" until a read
//! answers. Only what is already reading reads again, and nothing is
//! written or retried.
use super::*;
use freshkube_core::reachability::{Changes, SETTLE};

impl Pilot {
    /// Listens for network changes for the rest of the window's life.
    pub(in crate::desktop) fn start_network_watch(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A test sends its own changes, through `watch_network`.
        #[cfg(not(test))]
        {
            freshkube_core::reachability::start();
            self.watch_network(Changes::new(), window, cx);
        }
        #[cfg(test)]
        let _ = (window, cx);
    }

    /// Waits for a change, lets the burst it belongs to settle, and runs
    /// [`network_changed`](Self::network_changed) once for it. The loop ends
    /// with the shell.
    pub(super) fn watch_network(
        &mut self,
        mut changes: Changes,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                changes.changed().await;
                cx.background_executor().timer(SETTLE).await;
                // Everything reported meanwhile is this change.
                changes.settle();
                let alive = this.update_in(cx, |view, window, cx| {
                    view.network_changed(window, cx);
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// The network changed: the live data may have come over a path that is
    /// gone, so read again, if there is a connection to read again.
    pub(super) fn network_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        tracing::info!("the network changed");
        if self.nothing_to_reconnect() {
            return;
        }
        self.chrome.reconnecting = Some(Duration::ZERO);
        self.refresh_for_wake(true, window, cx);
        cx.notify();
    }
}
