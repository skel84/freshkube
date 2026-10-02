//! Kubernetes resource browsing: the server-printed table for a kind, kept
//! live by a watch and shown as virtualized rows.

pub(crate) mod custom;
pub(crate) mod detail;
pub(crate) mod direct;
pub(crate) mod example;
pub(crate) mod live;
pub(crate) mod model;
pub(crate) mod navigation;
mod pane;
pub(crate) mod projection;
mod screen;
pub(crate) mod store;

pub(crate) use pane::{DetailPane, Tab, shell};
pub(crate) use screen::{KubeAccess, KubeSource, NodePodsEvent, NotServed, ResourcesScreen, title};

/// Navigation requested by an explicit relationship link in a resource pane.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ResourceLink {
    Object(
        freshkube_core::resources::ResourceKind,
        model::ObjectRef,
        Tab,
    ),
    Owner {
        api_version: String,
        kind: String,
        object: model::ObjectRef,
    },
    Node(String, crate::desktop::nodes::NodeTab),
}
