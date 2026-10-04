//! A Kubernetes access boundary pinned to the local files inspected by the shell.
use freshkube_core::{ConfigurationRevision, cluster_overview::KubeconfigSelection};

use crate::screens::LiveSource;

#[derive(Clone)]
pub(crate) struct TalosAccess {
    pub(crate) live: LiveSource,
    pub(crate) selection: KubeconfigSelection,
    pub(crate) configuration: ConfigurationRevision,
}

impl TalosAccess {
    async fn check_configuration(&self) -> Result<(), String> {
        let path = self.live.config_path.clone();
        let selection = self.selection.clone();
        let revision = tokio::task::spawn_blocking(move || {
            ConfigurationRevision::for_talos_sources(path.as_deref(), &selection)
        })
        .await
        .map_err(|_| "Inspecting the access configuration stopped".to_owned())?;
        if revision != self.configuration {
            return Err("Access configuration changed; refresh to reconnect".into());
        }
        Ok(())
    }

    pub(crate) async fn client(&self) -> Result<kube::Client, String> {
        self.check_configuration().await?;
        let client = self.live.kubernetes().await?;
        self.check_configuration().await?;
        Ok(client)
    }

    pub(crate) fn forget(&self) {
        self.live.forget_kubernetes();
    }
}
