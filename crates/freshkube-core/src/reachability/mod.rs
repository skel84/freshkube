//! The network changing under a running app (#968): a VPN coming up or
//! down, a laptop moving from one Wi-Fi to another.
//!
//! The decision is plain data: [`Flags`] turns the platform's
//! reachability flags into "the network changed and can be used". The
//! platform watcher ([`start`], macOS only) feeds [`announce`]; the shell
//! holds a [`Changes`], settles a burst of events into one
//! (`SETTLE`), and refreshes what is shown once. Nothing reads on its own:
//! a change with nothing connected does nothing, and a loss of the network
//! is found by the read that fails, as it was.
use std::time::Duration;

use tokio::sync::watch;

#[cfg(target_os = "macos")]
mod macos;

/// Reachability flag: the default route can be reached
/// (`kSCNetworkReachabilityFlagsReachable`).
pub const REACHABLE: u32 = 1 << 1;
/// Reachability flag: a connection must first be made
/// (`kSCNetworkReachabilityFlagsConnectionRequired`).
pub const CONNECTION_REQUIRED: u32 = 1 << 2;

/// How long a burst of events is waited out before it counts as one change:
/// a network coming up reports several times in a second or two.
pub const SETTLE: Duration = Duration::from_secs(2);

/// Reads the platform's flags one report at a time.
#[derive(Debug, Default, Clone)]
pub struct Flags {
    last: Option<u32>,
}

impl Flags {
    /// Takes the flags as they were when watching began, so the first
    /// report is a change from them and not from nothing.
    pub fn seeded(flags: u32) -> Self {
        Self { last: Some(flags) }
    }

    /// One report. True when the flags differ from the last and the network
    /// can be used: a change that leaves it unreachable says nothing a read
    /// does not (and a refresh then would only show errors).
    pub fn observe(&mut self, flags: u32) -> bool {
        let changed = self.last.replace(flags) != Some(flags);
        changed && flags & REACHABLE != 0 && flags & CONNECTION_REQUIRED == 0
    }
}

#[cfg(not(test))]
fn signal() -> &'static watch::Sender<u64> {
    static SIGNAL: std::sync::OnceLock<watch::Sender<u64>> = std::sync::OnceLock::new();
    SIGNAL.get_or_init(|| watch::channel(0).0)
}

/// In a test the signal is the test thread's own, so one test's change never
/// reaches another's shell.
#[cfg(test)]
fn signal() -> &'static watch::Sender<u64> {
    thread_local! {
        static SIGNAL: &'static watch::Sender<u64> = Box::leak(Box::new(watch::channel(0).0));
    }
    SIGNAL.with(|signal| *signal)
}

/// Tells every [`Changes`] that the network changed.
pub fn announce() {
    signal().send_modify(|changes| *changes += 1);
}

/// Starts the platform's watcher, once for the process; where there is none
/// (everywhere but macOS) it does nothing.
pub fn start() {
    #[cfg(target_os = "macos")]
    macos::start();
}

/// What one shell hears of network changes: those after it was made.
#[derive(Debug)]
pub struct Changes(watch::Receiver<u64>);

impl Changes {
    pub fn new() -> Self {
        Self(signal().subscribe())
    }

    /// Resolves at the next change, and again at each one after it. Changes
    /// that came while nobody waited count as one.
    pub async fn changed(&mut self) {
        if self.0.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// Takes every change heard so far as seen: a burst settled into one.
    pub fn settle(&mut self) {
        self.0.borrow_and_update();
    }
}

impl Default for Changes {
    fn default() -> Self {
        Self::new()
    }
}

/// A signal of its own, for a test that must not reach other shells.
#[cfg(any(test, feature = "testing"))]
#[derive(Debug)]
pub struct OwnSignal(watch::Sender<u64>);

#[cfg(any(test, feature = "testing"))]
impl OwnSignal {
    pub fn new() -> Self {
        Self(watch::channel(0).0)
    }

    pub fn changes(&self) -> Changes {
        Changes(self.0.subscribe())
    }

    pub fn announce(&self) {
        self.0.send_modify(|changes| *changes += 1);
    }
}

#[cfg(any(test, feature = "testing"))]
impl Default for OwnSignal {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UP: u32 = REACHABLE;
    const DOWN: u32 = 0;
    const NEEDS_CONNECTION: u32 = REACHABLE | CONNECTION_REQUIRED;

    #[test]
    fn a_change_to_a_usable_network_counts() {
        let mut flags = Flags::seeded(DOWN);
        assert!(flags.observe(UP), "a network that came up");
    }

    #[test]
    fn the_same_flags_again_are_no_change() {
        let mut flags = Flags::seeded(UP);
        assert!(!flags.observe(UP));
        assert!(flags.observe(UP | 1 << 17), "another path while up");
        assert!(!flags.observe(UP | 1 << 17));
    }

    #[test]
    fn a_network_going_away_or_needing_a_connection_counts_for_nothing() {
        let mut flags = Flags::seeded(UP);
        assert!(!flags.observe(DOWN), "a read that fails says so");
        assert!(!flags.observe(NEEDS_CONNECTION), "not usable yet");
        assert!(flags.observe(UP), "usable again");
    }

    #[test]
    fn without_a_seed_the_first_report_is_a_change() {
        assert!(Flags::default().observe(UP));
        assert!(!Flags::default().observe(DOWN));
    }

    #[tokio::test]
    async fn changes_are_heard_after_a_shell_is_made_and_a_burst_is_one() {
        announce();
        let mut changes = Changes::new();
        let waiting = tokio::time::timeout(Duration::from_millis(20), changes.changed()).await;
        assert!(waiting.is_err(), "a change before it was made is not heard");
        announce();
        announce();
        announce();
        changes.changed().await;
        changes.settle();
        let again = tokio::time::timeout(Duration::from_millis(20), changes.changed()).await;
        assert!(again.is_err(), "the burst was one change");
        announce();
        changes.changed().await;
    }
}
