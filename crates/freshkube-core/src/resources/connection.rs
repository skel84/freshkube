//! A client for one named kubeconfig context, without Talos.

use std::path::PathBuf;
use std::time::Duration;

use kube::Client;
use kube::config::KubeConfigOptions;

use super::contexts::{KubeContext, load};
use super::failure::{Failure, FailureKind};
use crate::exec_plugin::{ConfigError, config_from_kubeconfig};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The server's host and port, as a person would write them: never its
/// user, password, path or query, which a kubeconfig's URL may carry.
fn server_address(url: &http::Uri) -> String {
    let host = url.host().unwrap_or_default();
    match url.port_u16() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    }
}

/// The server's version, as the first proof that it answers. A server that
/// refuses, or that never answers within `limit`, is named by its address.
async fn server_version(
    client: &Client,
    name: &str,
    server: &str,
    limit: Duration,
) -> Result<String, Failure> {
    let version = tokio::time::timeout(limit, client.apiserver_version())
        .await
        .map_err(|_| with_server(Failure::timeout(&format!("Connecting to '{name}'")), server))?
        .map_err(|error| with_server(Failure::from_kube(error), server))?;
    Ok(version.git_version)
}

/// Names the address an unreachable or timed-out server was tried at.
fn with_server(failure: Failure, server: &str) -> Failure {
    match failure.kind {
        FailureKind::Unreachable | FailureKind::Timeout if !server.is_empty() => Failure::new(
            failure.kind,
            format!("{} (tried {server})", failure.message),
        ),
        _ => failure,
    }
}

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
    let (context, client, server) = tokio::task::spawn_blocking(move || {
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
            let mut config = config_from_kubeconfig(kubeconfig, &options).await.map_err(
                |error| match error {
                    ConfigError::Plugin(failure) => failure,
                    ConfigError::Credentials => Failure::new(
                        FailureKind::Config,
                        format!("Couldn't load the credentials of context '{wanted}'"),
                    ),
                },
            )?;
            config.connect_timeout = Some(CONNECT_TIMEOUT);
            let server = server_address(&config.cluster_url);
            let client = crate::kube_client::client(config).map_err(|error| {
                Failure::new(
                    FailureKind::Config,
                    error.message(&format!("Couldn't make a client for context '{wanted}'")),
                )
            })?;
            Ok::<_, Failure>((context, client, server))
        })
    })
    .await
    .map_err(|_| Failure::new(FailureKind::Other, "The connection worker stopped"))??;
    let version = server_version(&client, &name, &server, CONNECT_TIMEOUT).await?;
    Ok(Connection {
        context,
        client,
        server_version: version,
    })
}

#[cfg(test)]
mod server_tests {
    use super::*;

    #[test]
    fn an_unreachable_server_is_named_by_host_and_port_and_nothing_else() {
        let url: http::Uri =
            "https://admin:hunter2-password@api.example.invalid:6443/base?token=s3cr3t-token"
                .parse()
                .unwrap();
        let server = server_address(&url);
        assert_eq!(server, "api.example.invalid:6443");
        let failure = with_server(
            Failure::new(FailureKind::Unreachable, "connection refused"),
            &server,
        );
        assert_eq!(
            failure.message,
            "connection refused (tried api.example.invalid:6443)"
        );
        for secret in ["admin", "hunter2", "s3cr3t", "token", "/base"] {
            assert!(!failure.message.contains(secret), "{secret}");
        }
    }

    /// A server that accepts the connection and never answers, as one that
    /// drops packets does to the client: the timeout names its address.
    #[tokio::test]
    async fn a_server_that_never_answers_is_named_when_the_wait_ends() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let _held = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok(connection) = listener.accept().await {
                held.push(connection);
            }
        });
        let config = kube::Config::new(format!("http://{address}").parse().unwrap());
        let server = server_address(&config.cluster_url);
        let client = Client::try_from(config).unwrap();
        let failure = server_version(&client, "lab", &server, Duration::from_millis(200))
            .await
            .unwrap_err();
        assert_eq!(failure.kind, FailureKind::Timeout);
        assert_eq!(
            failure.message,
            format!("Connecting to 'lab' timed out (tried {address})")
        );
    }

    #[test]
    fn only_a_server_that_did_not_answer_is_named() {
        let denied = Failure::new(FailureKind::Forbidden, "forbidden");
        assert_eq!(with_server(denied, "host:1").message, "forbidden");
        let timed = with_server(Failure::timeout("Connecting to 'lab'"), "host:1");
        assert!(timed.message.ends_with("(tried host:1)"));
        let url: http::Uri = "https://host".parse().unwrap();
        assert_eq!(server_address(&url), "host");
    }
}
