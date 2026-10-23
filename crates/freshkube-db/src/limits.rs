//! The numbers a session runs under. DBFlux's drivers set none of them, so the
//! session sends them itself at connect (`postgres.rs`). E4 lets the team file
//! tighten them; until then these are the built-in defaults.
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Rows a table page returns; one more is read to know there were more.
    pub rows: usize,
    pub statement_timeout: Duration,
    pub lock_timeout: Duration,
    pub idle_in_transaction_timeout: Duration,
    /// Covers opening the connection and the settings sent after it.
    pub connect_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            rows: 200,
            statement_timeout: Duration::from_secs(30),
            lock_timeout: Duration::from_secs(5),
            idle_in_transaction_timeout: Duration::from_secs(60),
            connect_timeout: Duration::from_secs(10),
        }
    }
}

impl Limits {
    /// The stricter of two limits, field by field: a ceiling never loosens.
    pub fn tightened(self, other: Self) -> Self {
        Self {
            rows: self.rows.min(other.rows),
            statement_timeout: self.statement_timeout.min(other.statement_timeout),
            lock_timeout: self.lock_timeout.min(other.lock_timeout),
            idle_in_transaction_timeout: self
                .idle_in_transaction_timeout
                .min(other.idle_in_transaction_timeout),
            connect_timeout: self.connect_timeout.min(other.connect_timeout),
        }
    }
}
