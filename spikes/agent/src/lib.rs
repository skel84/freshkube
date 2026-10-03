//! Spike: an investigation agent embedded in Freshkube on the rpi agent loop.
//! See README.md for what it checks and what it found.

pub mod acp;
pub mod investigation;
pub mod mcp;
pub mod scenario;
pub mod script;
pub mod session;
pub mod tools;

#[cfg(test)]
mod tests;
