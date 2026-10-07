//! A client for one named kubeconfig context, without Talos.

use std::path::PathBuf;
use std::time::Duration;

use kube::Client;
use kube::config::{Config, KubeConfigOptions};

use super::contexts::{KubeContext, load};
use super::failure::{Failure, FailureKind};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A client for one context whose server answered. Cloning shares the client.
#[derive(Clone)]
pub struct Connection {
    pub context: KubeContext,
    pub client: Client,
    pub server_version: String,
}

/// Builds a client for exactly the context named `name` in the merged
/// `sources`, and checks that its server answers.
pub async fn connect(sources: Vec<PathBuf>, name: String) -> Result<Connection, Failure> {
    // Without Talos nothing else may have chosen the TLS provider yet.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let runtime = tokio::runtime::Handle::current();
    let wanted = name.clone();
    // Reading files, credential files and auth plugins block; keep them off
    // the async workers, as the Talos-derived clients do.
    let (context, client) = tokio::task::spawn_blocking(move || {
        let (report, kubeconfig) = load(&sources);
        let context = report.context(&wanted).cloned().ok_or_else(|| {
            let message = match report.errors.first() {
                Some((path, error)) if report.contexts.is_empty() => {
                    format!("{}: {error}", path.display())
                }
                _ => format!("Context '{wanted}' was not found"),
            };
            Failure::new(FailureKind::Config, message)
        })?;
        let kubeconfig = kubeconfig
            .ok_or_else(|| Failure::new(FailureKind::Config, "No kubeconfig file could be read"))?;
        let options = KubeConfigOptions {
            context: Some(wanted.clone()),
            ..KubeConfigOptions::default()
        };
        runtime.block_on(async move {
            // kube's messages can quote credential sources; keep them out.
            let mut config = Config::from_custom_kubeconfig(kubeconfig, &options)
                .await
                .map_err(|_| {
                    Failure::new(
                        FailureKind::Config,
                        format!("Couldn't load the credentials of context '{wanted}'"),
                    )
                })?;
            config.connect_timeout = Some(CONNECT_TIMEOUT);
            let client = crate::kube_client::client(config).map_err(|error| {
                Failure::new(
                    FailureKind::Config,
                    error.message(&format!("Couldn't make a client for context '{wanted}'")),
                )
            })?;
            Ok::<_, Failure>((context, client))
        })
    })
    .await
    .map_err(|_| Failure::new(FailureKind::Other, "The connection worker stopped"))??;
    let version = tokio::time::timeout(CONNECT_TIMEOUT, client.apiserver_version())
        .await
        .map_err(|_| Failure::timeout(&format!("Connecting to '{name}'")))?
        .map_err(Failure::from_kube)?;
    Ok(Connection {
        context,
        client,
        server_version: version.git_version,
    })
}
