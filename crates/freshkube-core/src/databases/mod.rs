//! Operator-neutral, read-only database evidence. Readers never resolve credentials.
mod cnpg;
mod coverage;
pub mod example;
mod model;
mod read;

pub use coverage::{BackupCoverage, Freshness, LinkState, Restore, coverage, link};
pub use model::*;
pub use read::{read, valid_namespace};
#[cfg(test)]
mod tests;
