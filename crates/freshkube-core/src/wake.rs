//! Noticing that the machine slept.
//!
//! The process's monotonic clock does not run while the machine sleeps on
//! macOS and Linux; the wall clock does. A watcher that reads both on a
//! steady tick sees the wall clock move further than the monotonic one
//! exactly when the machine slept (or the clock was set forward). That
//! needs no platform hook and works on every platform whose monotonic
//! clock stops in sleep. Windows' does not, so a wake there goes unseen.
//!
//! [`GapDetector`] is the decision alone, a function of two readings, so a
//! test steps both clocks by hand and never sleeps.
use std::time::Duration;

use futures::{Stream, StreamExt};
use tokio::sync::watch;

/// How often the watcher reads the two clocks.
pub const TICK: Duration = Duration::from_secs(5);

/// How far the wall clock must outrun the monotonic one, over one tick,
/// to count as a sleep: well above scheduling jitter, well below a lid
/// closed for a minute.
pub const GAP: Duration = Duration::from_secs(15);

/// Compares two clocks' progress between readings.
#[derive(Debug, Clone)]
pub struct GapDetector {
    last: Option<(Duration, Duration)>,
    gap: Duration,
}

impl GapDetector {
    pub fn new() -> Self {
        Self::with_gap(GAP)
    }

    pub fn with_gap(gap: Duration) -> Self {
        Self { last: None, gap }
    }

    /// Takes one reading of each clock, as the time since any fixed origin
    /// (the origins may differ between the clocks, but not between calls).
    /// Returns how long the machine was asleep when the wall clock moved
    /// on by at least the gap more than the monotonic one since the last
    /// reading. The first reading, and a wall clock set backward, say
    /// nothing.
    pub fn observe(&mut self, monotonic: Duration, wall: Duration) -> Option<Duration> {
        let previous = self.last.replace((monotonic, wall))?;
        let monotonic_progress = monotonic.saturating_sub(previous.0);
        let wall_progress = wall.saturating_sub(previous.1);
        let slept = wall_progress.checked_sub(monotonic_progress)?;
        (slept >= self.gap).then_some(slept)
    }
}

impl Default for GapDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// The wake, told to every live stream that wants to start over.
///
/// A stream that was open when the machine slept is dead but doesn't know
/// it: its TCP connection is gone and nothing says so until a timeout, a
/// minute or more later. The shell [`announce`]s a wake; each stream holds
/// a [`Wakes`] and, on one, drops what it had and opens again from where it
/// was (a watch from its last resourceVersion, a log from its last line).
/// Nothing starts that wasn't already reading: only a stream that exists
/// hears it.
#[cfg(not(test))]
fn signal() -> &'static watch::Sender<u64> {
    static SIGNAL: std::sync::OnceLock<watch::Sender<u64>> = std::sync::OnceLock::new();
    SIGNAL.get_or_init(|| watch::channel(0).0)
}

/// In a test the signal is the test thread's own (a Tokio test runs on one
/// thread), so one test's wake never reaches another's streams.
#[cfg(test)]
fn signal() -> &'static watch::Sender<u64> {
    thread_local! {
        static SIGNAL: &'static watch::Sender<u64> = Box::leak(Box::new(watch::channel(0).0));
    }
    SIGNAL.with(|signal| *signal)
}

/// Tells every [`Wakes`] that the machine woke.
pub fn announce() {
    signal().send_modify(|wakes| *wakes += 1);
}

/// What one stream hears of wakes: those after it was made.
#[derive(Debug)]
pub struct Wakes(watch::Receiver<u64>);

impl Wakes {
    pub fn new() -> Self {
        Self(signal().subscribe())
    }

    /// Resolves at the next wake, and again at each one after it.
    pub async fn woke(&mut self) {
        if self.0.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

impl Default for Wakes {
    fn default() -> Self {
        Self::new()
    }
}

/// `open()`'s stream, opened again with `open()` at each wake: a kube
/// watcher that starts over lists first, which every consumer already
/// handles as it does the list after a 410, replacing what it holds. Ends
/// when the stream does.
pub fn restarting<S, F>(open: F) -> impl Stream<Item = S::Item>
where
    S: Stream,
    F: Fn() -> S,
{
    let wakes = Wakes::new();
    let stream = Box::pin(open());
    futures::stream::unfold(
        (open, stream, wakes),
        |(open, mut stream, mut wakes)| async move {
            loop {
                tokio::select! {
                    item = stream.next() => {
                        return item.map(|item| (item, (open, stream, wakes)));
                    }
                    _ = wakes.woke() => stream = Box::pin(open()),
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[tokio::test]
    async fn a_wake_reaches_streams_made_before_it_and_not_after() {
        let mut before = Wakes::new();
        announce();
        let mut after = Wakes::new();
        tokio::time::timeout(Duration::from_secs(1), before.woke())
            .await
            .expect("heard");
        assert!(
            tokio::time::timeout(Duration::from_millis(20), after.woke())
                .await
                .is_err()
        );
        // And each wake again.
        announce();
        tokio::time::timeout(Duration::from_secs(1), after.woke())
            .await
            .expect("heard");
    }

    #[tokio::test]
    async fn a_restarting_stream_opens_again_at_a_wake_and_keeps_going() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        let opened = std::sync::Arc::new(AtomicU32::new(0));
        let counter = opened.clone();
        let stream = restarting(move || {
            let n = counter.fetch_add(1, SeqCst);
            // Each opening says its number, then goes quiet.
            Box::pin(futures::stream::iter([n]).chain(futures::stream::pending()))
                as std::pin::Pin<Box<dyn Stream<Item = u32> + Send>>
        });
        let mut stream = std::pin::pin!(stream);
        assert_eq!(stream.next().await, Some(0));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), stream.next())
                .await
                .is_err()
        );
        announce();
        assert_eq!(stream.next().await, Some(1));
        assert_eq!(opened.load(SeqCst), 2);
    }

    #[test]
    fn the_first_reading_says_nothing() {
        assert_eq!(GapDetector::new().observe(secs(5), secs(1_000)), None);
    }

    #[test]
    fn clocks_that_move_together_say_nothing() {
        let mut detector = GapDetector::new();
        detector.observe(secs(0), secs(100));
        assert_eq!(detector.observe(secs(5), secs(105)), None);
        // Jitter below the gap is not a sleep.
        assert_eq!(detector.observe(secs(10), secs(112)), None);
    }

    #[test]
    fn a_wall_clock_that_ran_on_while_the_monotonic_one_stopped_is_a_sleep() {
        let mut detector = GapDetector::new();
        detector.observe(secs(0), secs(100));
        // A tick later by the monotonic clock, ten minutes by the wall's.
        assert_eq!(detector.observe(secs(5), secs(700)), Some(secs(595)));
        // Once told, the next ordinary tick is quiet.
        assert_eq!(detector.observe(secs(10), secs(705)), None);
    }

    #[test]
    fn a_wall_clock_set_backward_says_nothing() {
        let mut detector = GapDetector::new();
        detector.observe(secs(0), secs(1_000));
        assert_eq!(detector.observe(secs(5), secs(10)), None);
    }

    #[test]
    fn the_gap_is_inclusive() {
        let mut detector = GapDetector::with_gap(secs(15));
        detector.observe(secs(0), secs(0));
        assert_eq!(detector.observe(secs(5), secs(20)), Some(secs(15)));
    }
}
