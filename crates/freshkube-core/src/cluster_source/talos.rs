//! Talos access for a connected context, and the Kubernetes access boundary
//! pinned to the local files the shell inspected.
use std::{future::Future, path::PathBuf, sync::Arc, time::Duration};

use talos_rs::TalosClient;

use crate::cluster_overview::{
    ClusterOverview, ClusterOverviewCollector, K8sError, KubeconfigSelection,
};
use crate::failure::{Failure, FailureKind};
use crate::{AccessIdentity, ConfigurationRevision};

const KUBERNETES_SETUP: Duration = Duration::from_secs(45);

/// Talos access for a connected context. Clone it into a screen's Tokio work.
#[derive(Clone)]
pub struct LiveSource {
    /// Context client; pin it with `with_node` (core collectors do this themselves).
    pub client: TalosClient,
    pub cluster: Arc<ClusterOverview>,
    pub collector: ClusterOverviewCollector,
    /// The talosconfig in use; `None` means TALOSCONFIG or ~/.talos/config.
    pub config_path: Option<PathBuf>,
    /// The access the shell applied with this source, once its overview has
    /// answered. Operations pins its preview and run to it.
    pub applied: Option<AppliedAccess>,
}

impl LiveSource {
    /// An identity-validated Kubernetes client for this cluster, using the
    /// kubeconfig source chosen in Settings. Never an ambient kubeconfig.
    pub async fn kubernetes(&self) -> Result<kube::Client, Failure> {
        self.kubernetes_with_source()
            .await
            .map(|(client, _, _)| client)
    }

    /// Drops the reused Kubernetes client and prepared kubeconfig, so the next
    /// call revalidates and rebuilds. Call it when a Kubernetes request made
    /// with a client from [`Self::kubernetes`] fails.
    pub fn forget_kubernetes(&self) {
        self.collector.forget_kubernetes_client(&self.cluster);
    }

    /// As [`Self::kubernetes`], with the kubeconfig actually used and, when a
    /// selected file was rejected, why.
    ///
    /// The identity-validated client is reused for the same context, pinned node
    /// and kubeconfig selection for a couple of minutes.
    pub async fn kubernetes_with_source(
        &self,
    ) -> Result<(kube::Client, String, Option<String>), Failure> {
        kubernetes_setup(self.collector.kubernetes_client_with_source(&self.cluster)).await
    }
}

async fn kubernetes_setup<T>(
    work: impl Future<Output = Result<T, K8sError>>,
) -> Result<T, Failure> {
    tokio::time::timeout(KUBERNETES_SETUP, work)
        .await
        .map_err(|_| Failure::timeout("Setting up the Kubernetes client"))?
        // K8sError has already erased the causes into text. Do not infer a
        // kind from its words; upstream typing is still tracked in #584.
        .map_err(|error| Failure::new(FailureKind::Other, error.to_string()))
}

#[derive(Clone)]
pub struct TalosAccess {
    pub live: LiveSource,
    pub selection: KubeconfigSelection,
    pub configuration: ConfigurationRevision,
}

impl TalosAccess {
    pub(crate) async fn client(&self) -> Result<kube::Client, Failure> {
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
pub struct AppliedAccess {
    /// The talosconfig; `None` means TALOSCONFIG or ~/.talos/config.
    pub config_path: Option<PathBuf>,
    pub selection: KubeconfigSelection,
    pub configuration: ConfigurationRevision,
    pub identity: AccessIdentity,
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
    pub async fn client<T>(
        &self,
        connect: impl Future<Output = Result<T, Failure>>,
    ) -> Result<T, Failure> {
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
    connect: impl Future<Output = Result<T, Failure>>,
) -> Result<T, Failure> {
    check_configuration(path.clone(), selection.clone(), configuration).await?;
    let client = connect.await?;
    check_configuration(path, selection, configuration).await?;
    Ok(client)
}

async fn check_configuration(
    path: Option<PathBuf>,
    selection: KubeconfigSelection,
    configuration: ConfigurationRevision,
) -> Result<(), Failure> {
    let revision = tokio::task::spawn_blocking(move || {
        ConfigurationRevision::for_talos_sources(path.as_deref(), &selection)
    })
    .await
    .map_err(|_| {
        Failure::new(
            FailureKind::Other,
            "Inspecting the access configuration stopped",
        )
    })?;
    if revision != configuration {
        return Err(Failure::new(
            FailureKind::Config,
            "Access configuration changed; refresh to reconnect",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn opaque_setup_errors_keep_their_complete_text() {
        for (error, message) in [
            (
                K8sError::KubeconfigFetch("down".into()),
                "Failed to get kubeconfig from Talos: down",
            ),
            (
                K8sError::KubeconfigParse("invalid".into()),
                "Failed to parse kubeconfig: invalid",
            ),
            (
                K8sError::ClientCreate("worker stopped".into()),
                "Failed to create K8s client: worker stopped",
            ),
            (
                K8sError::ApiError("timed out".into()),
                "K8s API error: timed out",
            ),
        ] {
            let failure = kubernetes_setup(async { Err::<(), _>(error) })
                .await
                .unwrap_err();
            assert_eq!(failure, Failure::new(FailureKind::Other, message));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_setup_deadline_keeps_its_message() {
        let failure = kubernetes_setup(std::future::pending::<Result<(), K8sError>>())
            .await
            .unwrap_err();
        assert_eq!(
            failure,
            Failure::new(
                FailureKind::Timeout,
                "Setting up the Kubernetes client timed out"
            ),
        );
    }

    #[tokio::test]
    async fn pinned_access_preserves_a_connectors_failure() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("talosconfig");
        let kubeconfig = directory.path().join("kubeconfig");
        std::fs::write(&path, "configuration-a").unwrap();
        std::fs::write(&kubeconfig, "configuration-a").unwrap();
        let selection = KubeconfigSelection::File {
            path: kubeconfig,
            context: None,
        };
        let configuration = ConfigurationRevision::for_talos_sources(Some(&path), &selection);
        let failure = Failure::new(FailureKind::Forbidden, "setup refused");
        assert_eq!(
            pinned(Some(path), selection, configuration, async {
                Err::<(), _>(failure.clone())
            })
            .await,
            Err(failure),
        );
    }

    #[tokio::test]
    async fn replaced_access_keeps_its_message_and_does_not_connect() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("talosconfig");
        let kubeconfig = directory.path().join("kubeconfig");
        std::fs::write(&path, "configuration-a").unwrap();
        std::fs::write(&kubeconfig, "configuration-a").unwrap();
        let selection = KubeconfigSelection::File {
            path: kubeconfig,
            context: None,
        };
        let configuration = ConfigurationRevision::for_talos_sources(Some(&path), &selection);
        std::fs::write(&path, "configuration-b").unwrap();
        let connected = std::cell::Cell::new(false);
        let failure = pinned(Some(path), selection, configuration, async {
            connected.set(true);
            Ok(())
        })
        .await
        .unwrap_err();
        assert!(
            !connected.get(),
            "replaced access must not poll the connector"
        );
        assert_eq!(
            failure,
            Failure::new(
                FailureKind::Config,
                "Access configuration changed; refresh to reconnect"
            ),
        );
    }
}
