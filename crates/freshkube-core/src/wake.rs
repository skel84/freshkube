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

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
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
