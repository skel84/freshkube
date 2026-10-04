//! Render counters for tests that prove a view was (not) redrawn.
//!
//! They count only with the `counting` feature, which crates turn on from
//! their dev-dependencies; otherwise `hit` is empty and `count` is absent.

#[cfg(feature = "counting")]
thread_local! {
    static HITS: std::cell::RefCell<std::collections::BTreeMap<&'static str, usize>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

/// Records one render of the named view.
#[cfg(feature = "counting")]
pub fn hit(name: &'static str) {
    HITS.with(|hits| *hits.borrow_mut().entry(name).or_default() += 1);
}

#[cfg(not(feature = "counting"))]
#[inline(always)]
pub fn hit(_: &'static str) {}

/// Renders of the named view so far on this thread.
#[cfg(feature = "counting")]
pub fn count(name: &'static str) -> usize {
    HITS.with(|hits| hits.borrow().get(name).copied().unwrap_or(0))
}
