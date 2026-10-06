//! Instrumentation every Freshkube crate can reach: render probes for UI
//! tests, timing spans for the `stress` binary and the time to a first drawn
//! frame. The probes and spans compile to nothing in the shipped app; the
//! first-frame timer stays, silent unless asked for, so release smoke checks
//! can see the window draw.

pub mod first_frame;
pub mod perf;
pub mod probe;
