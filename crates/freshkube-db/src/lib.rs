//! The database explorer's session (#948): what a screen reads from a database.
//!
//! A [`Session`] answers four catalog reads (schemas, a schema's tables, a
//! table's columns, a table's rows, capped), runs a statement of the query tab
//! ([`Session::run`]) and explains one, and nothing else. [`postgres::Postgres`] is
//! the Postgres session; it enforces read-only itself ([`readonly`]) and sets
//! its own timeouts ([`Limits`]). [`fake`] answers example data and tests.
//!
//! Cells and errors use DBFlux's core (`Value`, `DbError`). No GPUI, no
//! Kubernetes and nothing of desktop live here: the caller brings a connected
//! [`wire::Wire`] and, for a password, a [`credentials::Password`] that it read
//! once for this connection.
pub mod credentials;
pub mod fake;
pub mod limits;
pub mod postgres;
pub mod readonly;
#[cfg(test)]
mod safety;
pub mod session;
pub mod statements;
pub mod teleport;
pub mod wire;

pub use dbflux_core::{DbError, Value};
pub use limits::Limits;
pub use session::{
    Canceller, ColumnEntry, ConnectError, Listed, Plan, QueryError, QueryResult, Session,
    TableEntry, TableKind, TablePage,
};
