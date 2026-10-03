//! A kubeconfig context reached directly, for Kubernetes-only mode.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use freshkube_core::resources::{Connection, connect};
use freshkube_core::{AccessIdentity, AccessSessionId, ConfigurationRevision};
use futures::future::{BoxFuture, FutureExt, Shared};

/// Loading credentials can run an auth plugin; anything slower is stuck.
const CONNECT_DEADLINE: Duration = Duration::from_secs(30);

type Attempt = Shared<BoxFuture<'static, Result<Connection, String>>>;
type Connector = fn(Vec<PathBuf>, String) -> BoxFuture<'static, Result<Connection, String>>;

/// One context of the merged kubeconfig files and its client. Every read
/// of the context joins one connection attempt and, once it succeeds,
/// shares its client. A finished attempt, failed or not, stays until
/// [`Self::forget`], so readers that fail together retry once, not each.
#[derive(Clone)]
pub(crate) struct DirectAccess {
    sources: Arc<[PathBuf]>,
    context: String,
    identity: AccessIdentity,
    attempt: Arc<Mutex<Option<Attempt>>>,
    connector: Connector,
}

impl DirectAccess {
    pub(crate) fn new(
        sources: Vec<PathBuf>,
        context: String,
        revision: ConfigurationRevision,
    ) -> Self {
        Self::with_connector(sources, context, revision, |sources, context| {
            async move {
                connect(sources, context)
                    .await
                    .map_err(|failure| failure.to_string())
            }
            .boxed()
        })
    }

    fn with_connector(
        sources: Vec<PathBuf>,
        context: String,
        revision: ConfigurationRevision,
        connector: Connector,
    ) -> Self {
        Self {
            sources: sources.into(),
            context,
            identity: AccessIdentity::new(AccessSessionId::new(), revision),
            attempt: Arc::default(),
            connector,
        }
    }

    /// One session and inspected configuration revision. Recreating access
    /// changes it even when the path and context names are the same.
    pub(crate) fn id(&self) -> String {
        self.identity.key()
    }

    pub(crate) fn revision(&self) -> ConfigurationRevision {
        self.identity.configuration()
    }

    pub(crate) fn context(&self) -> &str {
        &self.context
    }

    /// The connection, joining an attempt already under way.
    pub(crate) async fn connect(&self) -> Result<Connection, String> {
        let attempt = {
            let mut slot = self.attempt.lock().expect("connection slot");
            slot.get_or_insert_with(|| {
                let context = self.context.clone();
                let sources = self.sources.clone();
                let revision = self.revision();
                let connecting = (self.connector)(self.sources.to_vec(), context.clone());
                async move {
                    tokio::time::timeout(CONNECT_DEADLINE, async move {
                        check_revision(sources.clone(), revision).await?;
                        let connection = connecting.await?;
                        check_revision(sources, revision).await?;
                        Ok(connection)
                    })
                    .await
                    .unwrap_or_else(|_| Err(format!("Connecting to '{context}' timed out")))
                }
                .boxed()
                .shared()
            })
            .clone()
        };
        attempt.await
    }

    /// Drops a finished attempt, so the next read connects again. One still
    /// under way is fresh, and stays. A transport retry keeps this session's
    /// identity and durable object selection; configuration replacement must
    /// create new access instead.
    pub(crate) fn forget(&self) {
        let mut slot = self.attempt.lock().expect("connection slot");
        if slot
            .as_ref()
            .is_some_and(|attempt| attempt.peek().is_some())
        {
            *slot = None;
        }
    }
}

async fn check_revision(
    sources: Arc<[PathBuf]>,
    expected: ConfigurationRevision,
) -> Result<(), String> {
    let current = tokio::task::spawn_blocking(move || {
        ConfigurationRevision::from_kubeconfig_sources(&sources)
    })
    .await
    .map_err(|_| "Reading the Kubernetes configuration stopped".to_owned())?;
    if current == expected {
        Ok(())
    } else {
        Err("Kubernetes configuration changed; refresh to load it".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);

    fn replace_during_connect(
        sources: Vec<PathBuf>,
        context: String,
    ) -> BoxFuture<'static, Result<Connection, String>> {
        async move {
            std::fs::write(&sources[0], "new configuration").unwrap();
            Ok(Connection {
                context: freshkube_core::resources::KubeContext {
                    name: context,
                    cluster: "synthetic".into(),
                    namespace: None,
                    source: sources[0].clone(),
                },
                // Construction only: this test never sends a network request.
                client: kube::Client::try_from(kube::Config::new(
                    "http://127.0.0.1:1".parse().unwrap(),
                ))
                .unwrap(),
                server_version: "synthetic".into(),
            })
        }
        .boxed()
    }

    #[tokio::test]
    async fn a_connection_completing_after_configuration_replacement_is_rejected() {
        let directory = std::env::temp_dir().join(format!(
            "freshkube-late-access-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("config");
        std::fs::write(&path, "old configuration").unwrap();
        let sources = vec![path];
        let revision = ConfigurationRevision::from_kubeconfig_sources(&sources);
        let access =
            DirectAccess::with_connector(sources, "lab".into(), revision, replace_during_connect);
        let result = access.connect().await;
        std::fs::remove_dir_all(directory).unwrap();
        assert_eq!(
            result.err().as_deref(),
            Some("Kubernetes configuration changed; refresh to load it")
        );
    }

    #[test]
    fn same_path_configuration_replacement_changes_resource_identity() {
        let directory = std::env::temp_dir().join(format!(
            "freshkube-access-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("config");
        std::fs::write(&path, "old configuration").unwrap();
        let sources = vec![path];
        let previous = DirectAccess::new(
            sources.clone(),
            "lab".into(),
            ConfigurationRevision::from_kubeconfig_sources(&sources),
        );
        assert_eq!(previous.id(), previous.clone().id());
        std::fs::write(&sources[0], "new configuration").unwrap();
        let current = DirectAccess::new(
            sources.clone(),
            "lab".into(),
            ConfigurationRevision::from_kubeconfig_sources(&sources),
        );
        std::fs::remove_dir_all(directory).unwrap();
        assert_ne!(previous.id(), current.id());
        assert_ne!(previous.revision(), current.revision());
    }

    /// Fails after a moment, counting attempts.
    fn refused(_: Vec<PathBuf>, context: String) -> BoxFuture<'static, Result<Connection, String>> {
        ATTEMPTS.fetch_add(1, Ordering::SeqCst);
        async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Err(format!("{context} refused"))
        }
        .boxed()
    }

    #[tokio::test]
    async fn readers_share_one_attempt_until_it_is_forgotten() {
        let access = DirectAccess::with_connector(
            vec!["/a/config".into(), "/b/config".into()],
            "lab".into(),
            ConfigurationRevision::from_kubeconfig_sources(&[
                "/a/config".into(),
                "/b/config".into(),
            ]),
            refused,
        );
        let identity = access.id();
        assert_eq!(identity, access.clone().id());
        assert_eq!(access.context(), "lab");
        let other = access.clone();
        // Forgetting an attempt under way keeps it.
        let (first, second) = tokio::join!(access.connect(), async {
            other.forget();
            other.connect().await
        });
        assert_eq!(first.err().as_deref(), Some("lab refused"));
        assert_eq!(second.err().as_deref(), Some("lab refused"));
        assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 1);
        // A finished attempt answers until forgotten.
        assert!(access.connect().await.is_err());
        assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 1);
        other.forget();
        assert!(access.connect().await.is_err());
        assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 2);
        assert_eq!(identity, access.id());
    }
}
