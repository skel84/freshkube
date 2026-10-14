//! What a page asks the shell to open.

use crate::{DetailTarget, NodeTab, ObjectRef, Tab};

/// Navigation requested by an explicit relationship link in a resource pane.
#[derive(Clone, Debug, PartialEq)]
pub enum ResourceLink {
    Object(freshkube_core::resources::ResourceKind, ObjectRef, Tab),
    Owner {
        api_version: String,
        kind: String,
        object: ObjectRef,
    },
    Node(String, NodeTab),
    /// An object's logs, which open in the dock.
    Logs(LogsRequest),
    /// A shell in a pod's container, which starts in a dock tab: the
    /// user's pick from the pane's Shell menu.
    Shell(ShellRequest),
}

/// Which pod's container to run a shell in.
#[derive(Clone, Debug, PartialEq)]
pub struct ShellRequest {
    pub target: DetailTarget,
    pub container: String,
    /// The pod's containers as the pane read them, so the shell starts
    /// before its tab's own watch answers.
    pub containers: Option<freshkube_core::resources::PodContainers>,
}

/// Which object's logs to open in the dock, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct LogsRequest {
    pub target: DetailTarget,
    pub at: Option<LogsAt>,
}

/// A pod's container and instance to open its logs on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogsAt {
    pub container: String,
    pub previous: bool,
    /// Every app container at once; `container` is the one a pod with a
    /// single app container falls back to.
    pub all: bool,
}
