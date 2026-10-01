use std::{future::Future, path::PathBuf};

use talos_pilot_core::{
    cluster_overview::{ClusterOverview, ClusterOverviewCollector},
    inspection::InspectionTarget,
};
use talos_rs::TalosClient;
use tokio::{runtime::Handle, task::JoinHandle};

#[derive(Clone)]
pub(crate) struct FeatureContext {
    pub epoch: u64,
    pub revision: u64,
    pub context: String,
    pub node: String,
    pub address: String,
    pub config_path: Option<PathBuf>,
    pub client: TalosClient,
    pub collector: ClusterOverviewCollector,
    pub cluster: ClusterOverview,
    pub runtime: Handle,
}

impl PartialEq for FeatureContext {
    fn eq(&self, other: &Self) -> bool {
        self.epoch == other.epoch
            && self.revision == other.revision
            && self.context == other.context
            && self.node == other.node
            && self.address == other.address
    }
}

/// Owns the transport even while the component future is suspended.
struct AbortOnDrop<T>(JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl FeatureContext {
    /// Revision is intentionally absent: ordinary refreshes preserve feature state.
    pub fn key(&self) -> String {
        format!(
            "{}:{:?}:{:?}:{:?}",
            self.epoch, self.context, self.node, self.address
        )
    }

    pub fn inspection_target(&self) -> InspectionTarget {
        InspectionTarget::new(self.node.clone(), self.address.clone())
    }

    pub async fn kubernetes_with_source(
        &self,
    ) -> Result<(kube::Client, String, Option<String>), String> {
        let collector = self.collector.clone();
        let cluster = self.cluster.clone();
        self.run(async move {
            tokio::time::timeout(
                std::time::Duration::from_secs(45),
                collector.kubernetes_client_with_source(&cluster),
            )
            .await
            .map_err(|_| "Selected Kubernetes source setup timed out".to_owned())?
            .map_err(|error| error.to_string())
        })
        .await?
    }

    pub async fn run<F, T>(&self, future: F) -> Result<T, String>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        run_on_runtime(&self.runtime, future).await
    }
}

pub(crate) async fn run_on_runtime<F, T>(runtime: &Handle, future: F) -> Result<T, String>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let mut task = AbortOnDrop(runtime.spawn(future));
    (&mut task.0)
        .await
        .map_err(|error| format!("Background request failed: {error}"))
}

#[derive(Clone, PartialEq)]
pub(crate) struct LogRequest {
    pub node: Option<String>,
    pub address: Option<String>,
    pub services: Vec<String>,
}

pub(crate) async fn copy_text(ctx: FeatureContext, text: String) -> Result<(), String> {
    ctx.run(async move {
        tokio::task::spawn_blocking(move || {
            arboard::Clipboard::new()
                .and_then(|mut clipboard| clipboard.set_text(text))
                .map_err(|error| format!("Clipboard unavailable: {error}"))
        })
        .await
        .map_err(|error| format!("Clipboard worker failed: {error}"))?
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[test]
    fn dropping_runtime_request_aborts_idle_transport() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        struct Mark(Arc<AtomicBool>);
        impl Drop for Mark {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let marker = Mark(dropped.clone());
        let (started, ready) = tokio::sync::oneshot::channel();
        let task = AbortOnDrop(runtime.spawn(async move {
            let _marker = marker;
            let _ = started.send(());
            std::future::pending::<()>().await;
        }));
        runtime.block_on(ready).unwrap();
        let handle = task.0.abort_handle();
        drop(task);
        runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                while !handle.is_finished() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        });
        assert!(dropped.load(Ordering::SeqCst));
    }
}
