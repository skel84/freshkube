//! Instrumentation every Freshkube crate can reach: render probes for UI
//! tests and timing spans for the `stress` binary. Both compile to nothing
//! in the shipped app.

pub mod perf;
pub mod probe;
