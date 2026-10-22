//! Operator-neutral, read-only database evidence. Readers never resolve credentials.
mod cnpg;
pub mod coroot;
mod coverage;
pub mod cron;
pub mod example;
mod mariadb;
mod model;
mod read;
mod state;
mod verdict;

pub use coverage::{
    Archiving, BackupCoverage, Freshness, LinkState, Restore, RestoreWindow, archiving, coverage,
    link, restore_window,
};
pub use model::*;
pub use read::{READ_BUDGET, read, valid_namespace};
pub use state::{RowState, Source, worse_of};
pub use verdict::{
    CertificateState, Finding, SWITCHOVER_STUCK_AFTER, Switchover, Verdict, certificate, lag_of,
    orphan, switchover, verdict,
};
#[cfg(test)]
mod mariadb_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod verdict_tests;
