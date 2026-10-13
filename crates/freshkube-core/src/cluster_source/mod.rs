//! A cluster as a page outside Resources needs it: an id that tells one
//! connection's answers from another's, the context it names, and a way
//! to reach its Kubernetes API.
//!
//! The desktop app builds one from its own connection type, so a page that
//! takes a [`ClusterSource`] depends on neither that type nor the UI.

use futures::future::BoxFuture;
use std::fmt;
use std::sync::Arc;

/// Hands out a Kubernetes client for one cluster. An implementation may
/// build it once and reuse it; it decides when to build it again.
pub trait KubeClientSource: Send + Sync {
    fn client(&self) -> BoxFuture<'_, Result<kube::Client, String>>;

    /// Drops a reused client after a failure, so the next ask builds it
    /// again. A source that keeps nothing has nothing to drop.
    fn forget(&self) {}
}

/// How a cluster is reached.
#[derive(Clone)]
pub enum ClusterAccess {
    /// Made-up data for `--fixture`; nothing is contacted.
    Example,
    /// A live cluster, through the app's own client cache.
    Live(Arc<dyn KubeClientSource>),
}

impl ClusterAccess {
    pub fn is_example(&self) -> bool {
        matches!(self, ClusterAccess::Example)
    }

    pub async fn client(&self) -> Result<kube::Client, String> {
        match self {
            ClusterAccess::Example => Err("Example data has no Kubernetes client".into()),
            ClusterAccess::Live(source) => source.client().await,
        }
    }

    /// As [`KubeClientSource::forget`]; example data keeps nothing.
    pub fn forget(&self) {
        if let ClusterAccess::Live(source) = self {
            source.forget();
        }
    }
}

impl fmt::Debug for ClusterAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClusterAccess::Example => f.write_str("Example"),
            ClusterAccess::Live(_) => f.write_str("Live"),
        }
    }
}

/// One cluster connection. `id` is the connection's identity: answers read
/// for one id never show for another, and the same id keeps what was found.
#[derive(Clone, Debug)]
pub struct ClusterSource {
    /// The opaque access key derived from [`crate::AccessIdentity`], as the
    /// shell's `KubeSource.id` carries it; never a context name. A context
    /// keeps its name when its credentials change, and its key does not.
    pub id: String,
    /// The context the page names.
    pub context: String,
    pub access: ClusterAccess,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts the asks and answers with an error, so no client is built,
    /// and counts the forgets.
    #[derive(Default)]
    struct Counting(AtomicUsize, AtomicUsize);

    impl KubeClientSource for Counting {
        fn client(&self) -> BoxFuture<'_, Result<kube::Client, String>> {
            let asked = self.0.fetch_add(1, Ordering::SeqCst) + 1;
            Box::pin(async move { Err(format!("asked {asked}")) })
        }

        fn forget(&self) {
            self.1.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn example_has_no_client() {
        let access = ClusterAccess::Example;
        assert!(access.is_example());
        assert_eq!(
            access.client().await.err().as_deref(),
            Some("Example data has no Kubernetes client")
        );
    }

    #[tokio::test]
    async fn live_asks_its_source_each_time() {
        let counting = Arc::new(Counting::default());
        let source = ClusterSource {
            id: "live".into(),
            context: "acme-prod".into(),
            access: ClusterAccess::Live(counting.clone()),
        };
        assert!(!source.access.is_example());
        assert_eq!(
            source.access.client().await.err().as_deref(),
            Some("asked 1")
        );
        // A clone shares the source, so its caching is the source's.
        let copy = source.clone();
        assert_eq!(copy.access.client().await.err().as_deref(), Some("asked 2"));
        assert_eq!(counting.0.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn forget_reaches_a_live_source_through_a_clone() {
        let counting = Arc::new(Counting::default());
        let access = ClusterAccess::Live(counting.clone());
        access.forget();
        access.clone().forget();
        assert_eq!(counting.1.load(Ordering::SeqCst), 2);
        // Example data keeps no client.
        ClusterAccess::Example.forget();
    }
}
