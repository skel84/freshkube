//! A kubeconfig context reached directly, for Kubernetes-only mode.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use freshkube_core::resources::{Connection, connect};
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
    attempt: Arc<Mutex<Option<Attempt>>>,
    connector: Connector,
}

impl DirectAccess {
    pub(crate) fn new(sources: Vec<PathBuf>, context: String) -> Self {
        Self::with_connector(sources, context, |sources, context| {
            async move {
                connect(sources, context)
                    .await
                    .map_err(|failure| failure.to_string())
            }
            .boxed()
        })
    }

    fn with_connector(sources: Vec<PathBuf>, context: String, connector: Connector) -> Self {
        Self {
            sources: sources.into(),
            context,
            attempt: Arc::default(),
            connector,
        }
    }

    /// Names the connection: the files read and the context.
    pub(crate) fn id(&self) -> String {
        let files: Vec<String> = self
            .sources
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        format!("kube:{}:{}", files.join(","), self.context)
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
                let connecting = (self.connector)(self.sources.to_vec(), context.clone());
                async move {
                    tokio::time::timeout(CONNECT_DEADLINE, connecting)
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
    /// under way is fresh, and stays.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);

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
            refused,
        );
        assert_eq!(access.id(), "kube:/a/config,/b/config:lab");
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
    }
}
