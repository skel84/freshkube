//! The app's log sources. The view they feed, `LogView<S>`, is the
//! `freshkube-logs` crate; a source reaches it only through that crate's
//! source contract.
//!
//! - `talos`: Talos service logs: the node's catalog, collection and
//!   delivery. The Logs page is `LogView<TalosLogs>`, named [`LogPanel`].
//! - `pod`: one pod container's log, for the pane's Logs tab.
//! - `coroot`: an application's messages from Coroot, for the
//!   Application page's Logs report.

mod coroot;
mod pod;
mod talos;

#[cfg(test)]
mod tests;

pub(crate) use coroot::{CorootLogView, CorootPanel};
pub(crate) use freshkube_logs::{ClearSelection, Columns, LogSource, LogView};
pub(crate) use pod::{PodLogPanel, PodLogView, choice_label, role_heading};
pub(crate) use talos::{TalosLogs, TalosPanel};

/// The Talos Logs page.
pub(crate) type LogPanel = LogView<TalosLogs>;
