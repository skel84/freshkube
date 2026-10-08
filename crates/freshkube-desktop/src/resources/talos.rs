//! A Kubernetes access boundary pinned to the local files inspected by the shell.
use std::{future::Future, path::PathBuf};

use freshkube_core::{
    AccessIdentity, ConfigurationRevision, cluster_overview::KubeconfigSelection,
};

use crate::screens::LiveSource;

#[derive(Clone)]
pub(crate) struct TalosAccess {
    pub(crate) live: LiveSource,
    pub(crate) selection: KubeconfigSelection,
    pub(crate) configuration: ConfigurationRevision,
}

impl TalosAccess {
    pub(crate) async fn client(&self) -> Result<kube::Client, String> {
        pinned(
            self.live.config_path.clone(),
            self.selection.clone(),
            self.configuration,
            self.live.kubernetes(),
        )
        .await
    }

    pub(crate) fn forget(&self) {
        self.live.forget_kubernetes();
    }
}

/// The access the shell applied with a source: the local files it inspected,
/// their revision, and the access identity it connected with. Operations
/// takes it with a preview and builds the run's client only through it.
#[derive(Clone)]
pub(crate) struct AppliedAccess {
    /// The talosconfig; `None` means TALOSCONFIG or ~/.talos/config.
    pub(crate) config_path: Option<PathBuf>,
    pub(crate) selection: KubeconfigSelection,
    pub(crate) configuration: ConfigurationRevision,
    pub(crate) identity: AccessIdentity,
}

impl std::fmt::Debug for AppliedAccess {
    /// The revision alone; paths and identities stay out of debug output.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppliedAccess")
            .field("configuration", &self.configuration)
            .finish_non_exhaustive()
    }
}

impl AppliedAccess {
    /// Builds a client with `connect` while the local files stay at this
    /// revision, by the rule [`TalosAccess::client`] follows.
    pub(crate) async fn client<T>(
        &self,
        connect: impl Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        pinned(
            self.config_path.clone(),
            self.selection.clone(),
            self.configuration,
            connect,
        )
        .await
    }
}

/// Builds a client with `connect` for access pinned to `configuration`. The
/// local files are inspected before and after, so a replacement made before
/// or while the client was built is refused.
async fn pinned<T>(
    path: Option<PathBuf>,
    selection: KubeconfigSelection,
    configuration: ConfigurationRevision,
    connect: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    check_configuration(path.clone(), selection.clone(), configuration).await?;
    let client = connect.await?;
    check_configuration(path, selection, configuration).await?;
    Ok(client)
}

async fn check_configuration(
    path: Option<PathBuf>,
    selection: KubeconfigSelection,
    configuration: ConfigurationRevision,
) -> Result<(), String> {
    let revision = tokio::task::spawn_blocking(move || {
        ConfigurationRevision::for_talos_sources(path.as_deref(), &selection)
    })
    .await
    .map_err(|_| "Inspecting the access configuration stopped".to_owned())?;
    if revision != configuration {
        return Err("Access configuration changed; refresh to reconnect".into());
    }
    Ok(())
}
