//! The reads a screen makes of a database, whatever the engine.
use async_trait::async_trait;
use dbflux_core::{DbError, Value};

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
}
