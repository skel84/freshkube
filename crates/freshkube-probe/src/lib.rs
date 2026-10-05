//! Instrumentation every Freshkube crate can reach: render probes for UI
//! tests, timing spans for the `stress` binary and, in debug builds, the
//! time to a first drawn frame. All compile to nothing in the shipped app.

#[cfg(debug_assertions)]
pub mod first_frame;
pub mod perf;
pub mod probe;
