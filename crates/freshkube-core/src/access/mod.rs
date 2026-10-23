//! Your access (#854): what the signed-in person may do in each namespace,
//! read from the cluster, never decided here.
//!
//! Two kinds of call meet in this module and stay apart:
//!
//! - The table reads one `SelfSubjectRulesReview` per namespace, and for the
//!   cluster-wide Nodes row a few `SelfSubjectAccessReview`s. These guide:
//!   they say what to expect and why a thing is off. They are cached in a
//!   [`Board`] per access identity and dropped when it changes.
//! - The gate before a write stays core's `write` preflight, a
//!   `SelfSubjectAccessReview` on the exact object. Nothing here is read by
//!   it, and a cell saying "yes" never skips it.
//!
//! No UI types. Everything here only reads: a review creates no object
//! anyone could list, and nothing changes the cluster.

mod board;
mod columns;
pub mod example;
mod request;
mod review;

pub use board::{Answer, Board, Changed, PARALLEL, RowKey, RowView, Ticket};
pub use columns::{COLUMNS, Cell, Cells, Column, Need, Rule, RulesAnswer};
pub use request::request_text;
pub use review::{Who, list_namespaces, nodes, rules, who};

#[cfg(test)]
mod tests;

/// The most namespaces one read lists. A cluster with more shows these and
/// says so.
pub const NAMESPACE_CAP: usize = 500;
