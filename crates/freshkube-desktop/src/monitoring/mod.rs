//! Monitoring: Grafana dashboards drawn in the Console look from the
//! cluster's own Prometheus ([docs/MONITORING.md](../../../docs/MONITORING.md)).
//! `derive` turns an answer into display data once; `panel` draws it.
pub(crate) mod colors;
pub(crate) mod derive;
pub(crate) mod gallery;
pub(crate) mod panel;
