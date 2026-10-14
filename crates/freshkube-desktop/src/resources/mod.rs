//! Kubernetes resource browsing: the server-printed table for a kind, kept
//! live by a watch and shown as virtualized rows.

pub(crate) mod containers;
pub(crate) mod custom;
pub(crate) mod detail;
pub(crate) mod direct;
pub(crate) mod example;
pub(super) mod forwards;
pub(crate) mod live;
pub(crate) mod model;
pub(crate) mod navigation;
mod pane;
pub(crate) mod projection;
pub(crate) mod rows;
mod screen;
pub(crate) mod shell;
pub(crate) mod store;
pub(crate) mod talos;

pub(crate) use freshkube_nav::{LogsAt, LogsRequest, ResourceLink, ShellRequest};
pub(crate) use pane::{DetailEvent, DetailPane, Tab};
#[cfg(test)]
pub(crate) use screen::row_id;
pub(crate) use screen::{
    ClusterAccess, ClusterSource, NodePodsEvent, NotServed, ResourcesScreen, title,
};

/// The drawer's width: one for the page, whatever kind it shows, kept in
/// the navigation file.
pub(crate) const DRAWER_WIDTH: freshkube_ui::split_size::SizeKey =
    freshkube_ui::split_size::SizeKey::new("drawer", "resources");
