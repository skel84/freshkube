//! The `Targets` an agent session reads through: its own entry and the other
//! entries of the setup. Another entry reads with its own context from the
//! setup file, never the shown one, and needs agent access of its own: the
//! gate answers `not_enabled` until it is on.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use freshkube_agent::Targets;
use freshkube_core::agent_reads::{AgentTarget, EntryId, Source};
use freshkube_core::cluster_source::ClusterAccess;
use freshkube_core::workspace::Workspace;
use freshkube_resources::example::agent_objects;

use crate::desktop::entity_pages::hosted_by;

/// Where a session's own entry reads from.
#[derive(Clone)]
pub(crate) struct Own {
    pub(crate) entry: String,
    pub(crate) context: String,
    pub(crate) access: ClusterAccess,
}

pub(crate) struct DockTargets {
    own: String,
    targets: HashMap<String, AgentTarget>,
}

impl DockTargets {
    /// The session's entry, and the other entries `workspace` lists. A
    /// context outside the workspace has no entry, so it is never here.
    pub(crate) fn new(own: &Own, workspace: Option<&Workspace>) -> Self {
        let example = matches!(own.access, ClusterAccess::Example(_));
        let mut targets = HashMap::new();
        targets.insert(own.entry.clone(), target(own, example));
        if let Some(workspace) = workspace {
            let mut kept = BTreeMap::new();
            for entry in &workspace.clusters {
                if entry.id == own.entry {
                    continue;
                }
                let access = if example {
                    Some(ClusterAccess::EXAMPLE)
                } else {
                    hosted_by(workspace, &entry.id, &mut kept).map(ClusterAccess::Direct)
                };
                if let Some(access) = access {
                    let other = Own {
                        entry: entry.id.clone(),
                        context: entry.context.clone(),
                        access,
                    };
                    targets.insert(entry.id.clone(), target(&other, example));
                }
            }
        }
        Self {
            own: own.entry.clone(),
            targets,
        }
    }

    pub(crate) fn shared(self) -> Arc<dyn Targets> {
        Arc::new(self)
    }
}

fn target(own: &Own, example: bool) -> AgentTarget {
    AgentTarget {
        entry: own.entry.clone(),
        context: own.context.clone(),
        source: if example {
            Source::Example(agent_objects(&own.context))
        } else {
            Source::Live(own.access.clone())
        },
    }
}

impl Targets for DockTargets {
    fn resolve(&self, entry: Option<&EntryId>) -> Option<AgentTarget> {
        let id = entry.map_or(self.own.as_str(), |id| id.as_str());
        self.targets.get(id).cloned()
    }
}
