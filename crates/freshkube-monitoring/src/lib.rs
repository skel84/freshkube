//! Monitoring: Grafana dashboards drawn in the Console look from the
//! cluster's own Prometheus ([docs/MONITORING.md](../../../docs/MONITORING.md)).
//! `derive` turns an answer into display data once; `panel` draws it;
//! `page` is the rail area's page that reads and lays out a dashboard;
//! `request` runs the reads of the page and of `history`.
pub mod colors;
pub(crate) mod derive;
pub mod history;
pub mod page;
pub mod panel;
mod request;
pub(crate) mod store;

use freshkube_probe::{perf, probe};
// The look lives in freshkube-ui; the moved code reaches it by its old paths.
use freshkube_ui::{palette, ui};
#[cfg(test)]
use freshkube_ui::{text_size, theme};
