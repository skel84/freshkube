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
pub(crate) mod rows;
mod screen;
pub(crate) mod shell;
pub(crate) mod store;
pub(crate) mod talos;

pub(crate) use pane::{DetailEvent, DetailPane, Tab};
#[cfg(test)]
pub(crate) use screen::row_id;
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
    /// An object's logs, which open in the dock.
    Logs(LogsRequest),
    /// A shell in a pod's container, which starts in a dock tab: the
    /// user's pick from the pane's Shell menu.
    Shell(ShellRequest),
}

/// Which pod's container to run a shell in.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ShellRequest {
    pub(crate) target: detail::DetailTarget,
    pub(crate) container: String,
    /// The pod's containers as the pane read them, so the shell starts
    /// before its tab's own watch answers.
    pub(crate) containers: Option<freshkube_core::resources::PodContainers>,
}

/// Which object's logs to open in the dock, and where.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LogsRequest {
    pub(crate) target: detail::DetailTarget,
    pub(crate) at: Option<LogsAt>,
}

/// A pod's container and instance to open its logs on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LogsAt {
    pub(crate) container: String,
    pub(crate) previous: bool,
}
