//! The cluster a read is made on: an entry of the workspace, the context it
//! opens, and where its objects come from.

use std::fmt;
use std::sync::Arc;

use serde_json::Value;

use crate::cluster_source::ClusterAccess;
use crate::resources::ResourceKind;

/// Objects, log lines and events for example data, raw as a server would
/// answer them. Core applies the same parsing, bounds, Secret rule and places
/// to them as to a live answer, so what a test sees is what a cluster gets.
pub trait ExampleObjects: Send + Sync {
    /// Kinds beyond the built-in ones that the example cluster serves.
    fn kinds(&self) -> Vec<ResourceKind> {
        Vec::new()
    }

    /// Every object of `kind` in `namespace` (all of them for `None`), as
    /// the API returns it, `managedFields` and all.
    fn objects(&self, kind: &ResourceKind, namespace: Option<&str>) -> Vec<Value>;

    /// A container's log lines, oldest first; `None` when the pod or
    /// container does not exist. A container of `None` is the pod's default.
    fn log_lines(
        &self,
        namespace: &str,
        pod: &str,
        container: Option<&str>,
        previous: bool,
    ) -> Option<Vec<String>>;

    /// Every event in `namespace` (all of them for `None`), as the API
    /// returns them.
    fn events(&self, namespace: Option<&str>) -> Vec<Value>;
}

/// Where a target's answers come from.
#[derive(Clone)]
pub enum Source {
    /// The cluster the person's own access reaches.
    Live(ClusterAccess),
    /// Made-up objects; nothing is contacted.
    Example(Arc<dyn ExampleObjects>),
}

/// A cluster entry an agent may be asked to read: its id, the context it
/// opens now, and its source.
#[derive(Clone)]
pub struct AgentTarget {
    pub entry: String,
    pub context: String,
    pub source: Source,
}

impl fmt::Debug for AgentTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentTarget")
            .field("entry", &self.entry)
            .field(
                "source",
                &match self.source {
                    Source::Live(_) => "Live",
                    Source::Example(_) => "Example",
                },
            )
            .finish_non_exhaustive()
    }
}
