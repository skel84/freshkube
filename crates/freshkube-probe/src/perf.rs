//! Timings for the `stress` binary. Without the `stress` feature every call
//! here is empty, so instrumented code costs nothing in the shipped app.
//!
//! Samples are kept per thread; the app records them on the main thread and
//! the `stress` binary reports them from there.

#[cfg(feature = "stress")]
use std::{cell::RefCell, collections::BTreeMap, time::Instant};

#[cfg(feature = "stress")]
thread_local! {
    static SAMPLES: RefCell<BTreeMap<&'static str, Vec<f64>>> =
        const { RefCell::new(BTreeMap::new()) };
}

/// Times its scope in milliseconds under `name`.
#[must_use]
pub struct Span {
    #[cfg(feature = "stress")]
    name: &'static str,
    #[cfg(feature = "stress")]
    started: Instant,
}

#[cfg(feature = "stress")]
#[inline]
pub fn span(name: &'static str) -> Span {
    Span {
        name,
        started: Instant::now(),
    }
}

#[cfg(not(feature = "stress"))]
#[inline(always)]
pub fn span(_: &'static str) -> Span {
    Span {}
}

#[cfg(feature = "stress")]
impl Drop for Span {
    fn drop(&mut self) {
        value(self.name, self.started.elapsed().as_secs_f64() * 1000.);
    }
}

/// Records one sample under `name`.
#[cfg(feature = "stress")]
pub fn value(name: &'static str, value: f64) {
    SAMPLES.with(|samples| samples.borrow_mut().entry(name).or_default().push(value));
}

#[cfg(not(feature = "stress"))]
#[inline(always)]
pub fn value(_: &'static str, _: f64) {}

/// How far behind the newest line of a batch is, from the RFC 3339 time it
/// starts with, in milliseconds.
#[cfg(feature = "stress")]
pub fn line_lag(name: &'static str, line: Option<&str>) {
    let Some(token) = line.and_then(|line| line.split(' ').next()) else {
        return;
    };
    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(token) {
        let lag = chrono::Utc::now().signed_duration_since(time);
        value(name, lag.num_microseconds().unwrap_or(0) as f64 / 1000.);
    }
}

#[cfg(not(feature = "stress"))]
#[inline(always)]
pub fn line_lag(_: &'static str, _: Option<&str>) {}

/// Takes every sample recorded on this thread so far.
#[cfg(feature = "stress")]
pub fn drain() -> BTreeMap<&'static str, Vec<f64>> {
    SAMPLES.with(|samples| std::mem::take(&mut *samples.borrow_mut()))
}
