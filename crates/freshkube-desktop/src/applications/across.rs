//! What a live change read needs of the workspace: where each part is
//! read, how Argo CD's destinations map, and the kubeconfig the other
//! entries are connected through. The change page and the Changes list
//! read through a pool core's `delivery::entry::pool` keeps for the one
//! read, dropped with it.
use std::path::PathBuf;

use freshkube_core::delivery::change::live::Mapping;
use freshkube_core::delivery::entry::{self, EntryReader};
use freshkube_core::workspace::{Chosen, Sources};

use crate::resources::KubeAccess;

/// The workspace as a live change read sees it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Across {
    /// How Argo CD's destinations map, and which entry is open.
    pub(crate) mapping: Mapping,
    /// Where Kargo, the builds and Argo CD are read.
    pub(crate) sources: Sources,
    /// The workspace's kubeconfig, `None` for the automatic one.
    pub(crate) kubeconfig: Option<PathBuf>,
}

impl Across {
    /// Whether every part is read on the open connection.
    pub(crate) fn alone(&self) -> bool {
        self.sources
            .iter()
            .all(|(_, chosen)| matches!(chosen, Chosen::Open))
    }

    /// The open connection's reader. The workspace of one fails whole when
    /// it can't connect; beside other entries, the open one is made within
    /// their deadline, and why it couldn't be is its own entry's.
    pub(crate) async fn open(
        &self,
        access: &KubeAccess,
    ) -> Result<Result<EntryReader, String>, String> {
        if self.alone() {
            return access
                .client()
                .await
                .map(|client| Ok(entry::reader(client)));
        }
        let client = tokio::time::timeout(entry::CONNECT_DEADLINE, access.client())
            .await
            .unwrap_or_else(|_| {
                Err(format!(
                    "no answer within {} s",
                    entry::CONNECT_DEADLINE.as_secs()
                ))
            });
        Ok(client.map(entry::reader))
    }

    /// Where the other entries' kubeconfigs are found.
    pub(crate) fn kubeconfigs(&self) -> Vec<PathBuf> {
        freshkube_core::resources::kubeconfig_sources(self.kubeconfig.as_deref())
    }
}
