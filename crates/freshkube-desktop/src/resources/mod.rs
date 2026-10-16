//! Kubernetes resource browsing: the server-printed table for a kind, kept
//! live by a watch and shown as virtualized rows.

pub mod containers;
pub mod custom;
pub mod detail;
pub mod direct;
pub mod example;
pub mod forwards;
pub mod live;
pub mod model;
pub mod navigation;
mod node_facts;
mod pane;
pub(crate) mod projection;
pub mod rows;
mod screen;
pub mod shell;
pub mod store;
pub mod talos;

pub use freshkube_nav::{LogsAt, LogsRequest, ResourceLink, ShellRequest};
pub use node_facts::{NodeFacts, NodeMemory};
pub use pane::{DetailEvent, DetailPane, Tab};
#[cfg(test)]
pub use screen::row_id;
pub use screen::{ClusterAccess, ClusterSource, NodePodsEvent, NotServed, ResourcesScreen, title};

/// The drawer's width: one for the page, whatever kind it shows, kept in
/// the navigation file.
pub const DRAWER_WIDTH: freshkube_ui::split_size::SizeKey =
    freshkube_ui::split_size::SizeKey::new("drawer", "resources");
