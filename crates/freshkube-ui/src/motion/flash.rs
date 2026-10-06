//! Which changes flash: each changed key while the window is quiet, none of
//! a batch that takes the last [`FADE`] past [`FLASH_BURST`] changes.

use super::{FADE, FLASH_BURST};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// One changed key's flash. `seq` is new for every change, so a key that
/// changes again while it flashes starts its fade again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flash<K> {
    pub key: K,
    pub seq: usize,
    pub at: Instant,
}

/// The flashes a list shows and the changes it saw lately, timed by the
/// caller's clock (`cx.background_executor().now()`).
pub struct Flashes<K> {
    live: Vec<Flash<K>>,
    /// Each batch in the last window: when, and how many changes.
    recent: VecDeque<(Instant, usize)>,
    next: usize,
    burst: usize,
    window: Duration,
}

impl<K: Clone + Eq> Default for Flashes<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Clone + Eq> Flashes<K> {
    /// [`FLASH_BURST`] changes per [`FADE`].
    pub fn new() -> Self {
        Self::with_burst(FLASH_BURST)
    }

    pub fn with_burst(burst: usize) -> Self {
        Self {
            live: Vec::new(),
            recent: VecDeque::new(),
            next: 0,
            burst,
            window: FADE,
        }
    }

    /// Records one batch of changed keys at `now`, and flashes them unless
    /// the batch takes the window past the burst. Returns whether it did.
    /// A suppressed batch still counts, so changes just after a reload
    /// don't flash either.
    pub fn changed(&mut self, keys: impl IntoIterator<Item = K>, now: Instant) -> bool {
        let keys: Vec<K> = keys.into_iter().collect();
        if keys.is_empty() {
            return false;
        }
        self.prune(now);
        let seen: usize = self.recent.iter().map(|(_, count)| count).sum();
        self.recent.push_back((now, keys.len()));
        if seen + keys.len() > self.burst {
            return false;
        }
        for key in keys {
            self.live.retain(|flash| flash.key != key);
            self.live.push(Flash {
                key,
                seq: self.next,
                at: now,
            });
            self.next += 1;
        }
        true
    }

    /// The flashes still fading at `now`.
    pub fn live(&self, now: Instant) -> impl Iterator<Item = &Flash<K>> {
        self.live
            .iter()
            .filter(move |flash| now.saturating_duration_since(flash.at) < self.window)
    }

    /// When the last live flash ends, if any is live.
    pub fn ends(&self) -> Option<Instant> {
        self.live.iter().map(|flash| flash.at + self.window).max()
    }

    /// Forgets the flashes and batches older than the window.
    pub fn prune(&mut self, now: Instant) {
        let window = self.window;
        let fresh = |at: Instant| now.saturating_duration_since(at) < window;
        self.live.retain(|flash| fresh(flash.at));
        while self.recent.front().is_some_and(|(at, _)| !fresh(*at)) {
            self.recent.pop_front();
        }
    }

    /// Forgets everything, as when the list is read again.
    pub fn clear(&mut self) {
        self.live.clear();
        self.recent.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(flashes: &Flashes<u32>, now: Instant) -> Vec<u32> {
        flashes.live(now).map(|flash| flash.key).collect()
    }

    #[test]
    fn a_change_flashes_until_the_fade_ends() {
        let start = Instant::now();
        let mut flashes = Flashes::new();
        assert!(flashes.changed([1], start));
        assert_eq!(keys(&flashes, start + FADE / 2), [1]);
        assert_eq!(flashes.ends(), Some(start + FADE));
        assert!(keys(&flashes, start + FADE).is_empty());
    }

    #[test]
    fn a_key_that_changes_again_starts_again() {
        let start = Instant::now();
        let mut flashes = Flashes::new();
        flashes.changed([1], start);
        let first = flashes.live(start).next().unwrap().seq;
        let later = start + FADE / 2;
        flashes.changed([1], later);
        let again: Vec<_> = flashes.live(later).collect();
        assert_eq!(again.len(), 1);
        assert_ne!(again[0].seq, first);
        assert_eq!(keys(&flashes, start + FADE), [1]);
    }

    #[test]
    fn a_batch_past_the_burst_does_not_flash_at_all() {
        let start = Instant::now();
        let mut flashes = Flashes::new();
        assert!(!flashes.changed(0..(FLASH_BURST as u32 + 1), start));
        assert!(keys(&flashes, start).is_empty());
        // Right after a reload, a single change is still part of the burst.
        assert!(!flashes.changed([100], start + FADE / 2));
        // Once the window has passed, changes are news again.
        assert!(flashes.changed([100], start + FADE * 2));
        assert_eq!(keys(&flashes, start + FADE * 2), [100]);
    }

    #[test]
    fn a_trickle_flashes_up_to_the_burst_and_keeps_what_it_flashed() {
        let start = Instant::now();
        let step = FADE / 20;
        let mut flashes = Flashes::new();
        let flashed: Vec<bool> = (0..12u32)
            .map(|n| flashes.changed([n], start + step * n))
            .collect();
        assert_eq!(flashed.iter().filter(|f| **f).count(), FLASH_BURST);
        assert!(flashed[..FLASH_BURST].iter().all(|f| *f));
        let now = start + step * 11;
        assert_eq!(
            keys(&flashes, now),
            (0..FLASH_BURST as u32).collect::<Vec<_>>()
        );
    }
}
