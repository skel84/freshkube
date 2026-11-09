//! The reads a screen makes of a database, whatever the engine.
use async_trait::async_trait;
use dbflux_core::{DbError, Value};
use std::{sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableKind {
    Table,
    View,
    Materialized,
    Foreign,
    Partitioned,
}

impl TableKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Table => "Table",
            Self::View => "View",
            Self::Materialized => "Materialized view",
            Self::Foreign => "Foreign table",
            Self::Partitioned => "Partitioned table",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableEntry {
    pub name: String,
    pub kind: TableKind,
    /// The planner's estimate; `None` when the engine hasn't analysed it.
    pub rows_estimate: Option<u64>,
    pub size_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnEntry {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TablePage {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    /// The table has more rows than `row_cap`; these are the first ones.
    pub capped: bool,
    pub row_cap: usize,
}

/// Why a connection could not be made. Each is a state of the connect step.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConnectError {
    /// The server answered and said no: wrong user or password, no access.
    #[error("The database refused the connection: {0}")]
    Refused(String),
    #[error("The database did not answer in time")]
    TimedOut,
    /// Nothing answered at the address (a closed forward, a reset).
    #[error("The database could not be reached: {0}")]
    Unreachable(String),
    /// TLS was offered and the server's certificate was not accepted.
    #[error("The server's certificate was not accepted: {0}")]
    Untrusted(String),
    /// The server did not accept the read-only setting; the session is not used.
    #[error("The session could not be made read-only: {0}")]
    NotReadOnly(String),
}

/// What one statement of the query tab returned.
#[derive(Clone, Debug, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    /// The statement returned more rows than `row_cap`; these are the first.
    pub capped: bool,
    pub row_cap: usize,
    pub elapsed: Duration,
}

/// A statement's plan, one line per row, as the server printed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub lines: Vec<String>,
    /// More lines than [`MAX_PLAN_LINES`]; these are the first.
    pub capped: bool,
}

pub const MAX_PLAN_LINES: usize = 2000;

/// Why a statement of the query tab did not return rows. Each is a state of
/// its result tab; none is retried by itself.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum QueryError {
    /// Freshkube did not send it: a `SET`, a `BEGIN`, several statements.
    #[error("{0}")]
    NotSent(String),
    /// The database refused it because the session is read-only.
    #[error("{0}")]
    Refused(String),
    #[error("The statement ran past the time limit and was stopped")]
    TimedOut,
    /// Stopped by Cancel.
    #[error("Cancelled")]
    Cancelled,
    /// The database's own error: a syntax error, a missing table.
    #[error("{0}")]
    Failed(String),
    /// The connection is gone.
    #[error("The connection was lost: {0}")]
    Lost(String),
}

impl From<DbError> for QueryError {
    fn from(error: DbError) -> Self {
        match error {
            DbError::Timeout => Self::TimedOut,
            DbError::Cancelled => Self::Cancelled,
            DbError::ConnectionFailed(e) => Self::Lost(e.to_string()),
            DbError::PermissionDenied(e) if e.to_string().contains("read-only") => {
                Self::Refused(e.to_string())
            }
            other => Self::Failed(other.to_string()),
        }
    }
}

#[allow(
    clippy::double_must_use,
    reason = "the async_trait expansion marks its boxed futures must_use"
)]
/// Stops the statement a session is running. It does not need the session,
/// which a running statement holds; it reaches the server another way.
#[async_trait]
pub trait Canceller: Send + Sync {
    /// Asks the server to stop whatever the session is running now.
    async fn cancel(&self) -> Result<(), String>;
}

/// A catalog listing cut at a bound, so a huge database can't fill memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed<T> {
    pub items: Vec<T>,
    /// There were more than the bound.
    pub capped: bool,
}

/// The most schemas and tables a listing returns.
pub const MAX_SCHEMAS: usize = 500;
pub const MAX_TABLES: usize = 2000;

#[allow(
    clippy::double_must_use,
    reason = "the async_trait expansion marks its boxed futures must_use"
)]
#[allow(
    clippy::result_large_err,
    reason = "DbError is DBFlux's type, and large; every read here returns it"
)]
#[async_trait]
pub trait Session: Send {
    /// True for every session this crate makes; a screen shows it and a
    /// future write path would have to be asked for separately.
    fn read_only(&self) -> bool;
    async fn schemas(&mut self) -> Result<Listed<String>, DbError>;
    async fn tables(&mut self, schema: &str) -> Result<Listed<TableEntry>, DbError>;
    async fn columns(&mut self, schema: &str, table: &str) -> Result<Vec<ColumnEntry>, DbError>;
    /// The first rows of a table, at most the session's row cap.
    async fn table_rows(&mut self, schema: &str, table: &str) -> Result<TablePage, DbError>;
    /// One statement of the query tab, in its own read-only transaction that
    /// is rolled back; at most the row cap of rows. A statement that would
    /// write is sent and refused by the database, never retried.
    async fn run(&mut self, sql: &str) -> Result<QueryResult, QueryError>;
    /// The statement's plan (`EXPLAIN`, never `ANALYZE`, which would run it).
    async fn explain(&mut self, sql: &str) -> Result<Plan, QueryError>;
    /// A handle that cancels the running statement, usable while it runs.
    fn canceller(&self) -> Arc<dyn Canceller>;
}
