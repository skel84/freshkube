//! The database explorer's session (#948): what a screen reads from a database.
//!
//! A [`Session`] answers four reads (schemas, a schema's tables, a table's
//! columns, a table's rows, capped) and nothing else. [`postgres::Postgres`] is
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
pub mod session;
pub mod wire;

pub use dbflux_core::{DbError, Value};
pub use limits::Limits;
pub use session::{ColumnEntry, ConnectError, Session, TableEntry, TableKind, TablePage};
