//! Monitoring: Grafana dashboards drawn in the Console look from the
//! cluster's own Prometheus ([docs/MONITORING.md](../../../docs/MONITORING.md)).
//! `derive` turns an answer into display data once; `panel` draws it;
//! `page` is the rail area's page that reads and lays out a dashboard;
//! `request` runs the reads of the page and of `history`.
pub(crate) mod colors;
pub(crate) mod derive;
pub(crate) mod history;
pub(crate) mod page;
pub(crate) mod panel;
mod request;
pub(crate) mod store;
