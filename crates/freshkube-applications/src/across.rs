//! What a live change read needs of the workspace: where each part is
//! read, how Argo CD's destinations map, and the kubeconfig the other
//! entries are connected through. The change page and the Changes list
//! read through a pool core's `delivery::entry::pool` keeps for the one
//! read, dropped with it.
use std::path::PathBuf;
use std::time::Duration;

use freshkube_core::resources::{Failure, FailureKind};
use freshkube_core::workspace::{Chosen, Sources};
use freshkube_delivery::change::live::Mapping;
use freshkube_delivery::entry::{self, Deadline, EntryReader};
use freshkube_delivery::read::ReadOnlyClient;

use crate::resources::ClusterAccess;

/// The workspace as a live change read sees it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Across {
    /// How Argo CD's destinations map, and which entry is open.
    pub mapping: Mapping,
    /// Where Kargo, the builds and Argo CD are read.
    pub sources: Sources,
    /// The top-level setup's public key, opened afresh off the UI thread.
    pub public_key_file: Option<std::path::PathBuf>,
    /// The pipeline record configured in the same setup.
    pub evidence_result: Option<freshkube_core::setup::Evidence>,
    /// The namespace the builds run in, from the same setup.
    pub build_namespace: Option<String>,
    /// How an Application with no authorized-stage annotation names its
    /// Stage, from the same setup.
    pub stage_naming: Option<freshkube_core::setup::StageNaming>,
    /// The workspace's kubeconfig, `None` for the automatic one.
    pub kubeconfig: Option<PathBuf>,
}

impl Across {
    /// Whether every part is read on the open connection.
    pub fn alone(&self) -> bool {
        self.sources
            .iter()
            .all(|(_, chosen)| matches!(chosen, Chosen::Open))
    }

    /// The open connection's reader: the workspace of one reads it as
    /// `alone` says; beside other entries, it has theirs.
    pub(crate) async fn open(
        &self,
        access: &ClusterAccess,
        alone: Alone,
    ) -> Result<Result<EntryReader, Failure>, String> {
        open_reader(access, self.alone().then_some(alone)).await
    }

    /// Where the other entries' kubeconfigs are found.
    pub(crate) fn kubeconfigs(&self) -> Vec<PathBuf> {
        freshkube_core::resources::kubeconfig_sources(self.kubeconfig.as_deref())
    }
}

/// How the workspace of one reads its open connection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Alone {
    /// Under the read's own deadline, each request too, as before there
    /// was a workspace: the change page and Applications.
    Within(Duration),
    /// With an entry's deadlines: the Changes list.
    AsEntry,
}

/// The open connection's reader. The workspace of one (`alone` given)
/// fails whole when it can't connect; beside other entries, the open one
/// is made within their deadline, so a client being made again holds up
/// no other entry, and why it couldn't be is its own entry's.
pub(crate) async fn open_reader(
    access: &ClusterAccess,
    alone: Option<Alone>,
) -> Result<Result<EntryReader, Failure>, String> {
    if alone.is_some() {
        return access
            .client()
            .await
            .map(|client| Ok(wrap(client, alone)))
            .map_err(|failure| failure.message);
    }
    let client = tokio::time::timeout(entry::CONNECT_DEADLINE, client_or_failure(access))
        .await
        .unwrap_or_else(|_| {
            Err(Failure::new(
                FailureKind::Timeout,
                format!("no answer within {} s", entry::CONNECT_DEADLINE.as_secs()),
            ))
        });
    Ok(client.map(|client| wrap(client, None)))
}

/// The open connection's client, or why it couldn't be had. Keep this
/// presentation boundary's established kind and words for delivery.
async fn client_or_failure(access: &ClusterAccess) -> Result<kube::Client, Failure> {
    access
        .client()
        .await
        .map_err(|why| Failure::new(FailureKind::Unreachable, why.message))
}

/// The reader over the open connection's client.
fn wrap(client: kube::Client, alone: Option<Alone>) -> EntryReader {
    match alone {
        Some(Alone::Within(limit)) => Deadline::new(ReadOnlyClient::new(client), limit),
        Some(Alone::AsEntry) | None => entry::reader(client),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_workspace_of_one_reads_as_it_is_told() {
        // Construction only: this test never sends a network request.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = || {
            kube::Client::try_from(kube::Config::new("http://127.0.0.1:1".parse().unwrap()))
                .unwrap()
        };
        let page = Duration::from_secs(60);
        assert_eq!(wrap(client(), Some(Alone::Within(page))).limit(), page);
        assert_eq!(
            wrap(client(), Some(Alone::AsEntry)).limit(),
            entry::REQUEST_DEADLINE
        );
        assert_eq!(wrap(client(), None).limit(), entry::REQUEST_DEADLINE);
    }
}
