use std::{
    fmt::Display,
    future::Future,
    path::{Path, PathBuf},
    time::Duration,
};

use freshkube_core::cluster_overview::{ClusterOverviewCollector, KubeconfigSelection};
use freshkube_core::{
    cluster_overview::{ClusterOverview, ConfigIdentity},
    logs::ServiceId,
};
use futures::{Stream, StreamExt};
use talos_rs::TalosConfig;
use talos_rs::{ServiceInfo, TalosClient};
use tokio::{
    runtime::Handle,
    sync::{mpsc, oneshot},
    task::{JoinHandle, JoinSet},
};

/// Room for log lines between a stream's task and the view, and the most the
/// view takes in one turn. Turns come every 16 ms, so up to about 64,000
/// lines a second reach the view, and one turn's append stays within a few
/// milliseconds.
pub(crate) const STREAM_QUEUE_CAPACITY: usize = 1024;
pub(crate) const MAX_STREAM_SERVICES: usize = 16;
const MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_TAIL_LINES: i32 = 1000;
const OPEN_DEADLINE: Duration = Duration::from_secs(12);
const COLLECT_DEADLINE: Duration = Duration::from_secs(60);
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AppliedConfig {
    pub(crate) path: Option<PathBuf>,
    pub(crate) context: Option<String>,
}

#[derive(Debug)]
pub(crate) struct ContextCatalog {
    pub(crate) path: PathBuf,
    pub(crate) names: Vec<String>,
    pub(crate) current: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    pub(crate) epoch: u64,
    pub(crate) context: String,
    pub(crate) node: String,
    pub(crate) address: String,
}

/// The UI owns this guard for exactly as long as the request is relevant.
pub(crate) struct OwnedJob {
    task: JoinHandle<()>,
}

impl OwnedJob {
    pub(crate) fn new(task: JoinHandle<()>) -> Self {
        Self { task }
    }
}

impl Drop for OwnedJob {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug)]
pub(crate) struct StreamEvent {
    pub(crate) target: Target,
    pub(crate) service: ServiceId,
    pub(crate) result: Result<String, String>,
}

#[cfg(test)]
fn read_config_file(path: &Path) -> Result<TalosConfig, String> {
    read_config_file_with_identity(path).map(|(config, _)| config)
}

/// Also fingerprints the exact bytes parsed, so a connection can be reused only
/// while the talosconfig is unchanged.
fn read_config_file_with_identity(path: &Path) -> Result<(TalosConfig, ConfigIdentity), String> {
    // Core's reader opens nonblocking and caps the read on the descriptor.
    let bytes =
        freshkube_core::read_bounded_regular_file(path, MAX_CONFIG_BYTES).map_err(|reason| {
            match reason {
                "path is not a regular file" => {
                    "Talos configuration must be a regular file".to_owned()
                }
                "file exceeds the supported size limit" => {
                    "Talos configuration exceeds the 4 MiB limit".to_owned()
                }
                reason => format!("Cannot read talosconfig: {reason}"),
            }
        })?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "Talos configuration is not valid UTF-8".to_string())?;
    // serde_yaml diagnostics may echo credential values; never send them to UI.
    let identity = ConfigIdentity::from_bytes(&bytes);
    TalosConfig::parse(text)
        .map(|config| (config, identity))
        .map_err(|_| "Talos configuration could not be parsed".into())
}

#[cfg(test)]
fn read_applied_config(
    applied: AppliedConfig,
) -> Result<(PathBuf, TalosConfig, ContextCatalog), String> {
    read_applied_config_with_identity(applied)
        .map(|(path, config, catalog, _)| (path, config, catalog))
}

fn read_applied_config_with_identity(
    applied: AppliedConfig,
) -> Result<(PathBuf, TalosConfig, ContextCatalog, ConfigIdentity), String> {
    let path = match applied.path {
        Some(path) => path,
        None => TalosConfig::default_path().map_err(|error| error.to_string())?,
    };
    let path = std::path::absolute(path)
        .map_err(|error| format!("Cannot resolve talosconfig path: {error}"))?;
    let (loaded, identity) = read_config_file_with_identity(&path)?;
    let current = applied.context.unwrap_or_else(|| loaded.context.clone());
    if !loaded.contexts.contains_key(&current) {
        return Err(format!("Talos context '{current}' was not found"));
    }
    let mut names: Vec<_> = loaded.contexts.keys().cloned().collect();
    names.sort();
    Ok((
        path.clone(),
        loaded,
        ContextCatalog {
            path,
            names,
            current,
        },
        identity,
    ))
}

/// All path resolution, bounded reading, parsing, and validation is off GPUI.
async fn read_config(
    runtime: &Handle,
    config: AppliedConfig,
) -> Result<(PathBuf, TalosConfig, ContextCatalog, ConfigIdentity), String> {
    runtime
        .spawn_blocking(move || read_applied_config_with_identity(config))
        .await
        .map_err(|error| format!("Talos configuration worker failed: {error}"))?
}

pub(crate) fn load_contexts(
    runtime: Handle,
    config: AppliedConfig,
) -> (OwnedJob, oneshot::Receiver<Result<ContextCatalog, String>>) {
    let (mut sender, receiver) = oneshot::channel();
    let worker_runtime = runtime.clone();
    let task = runtime.spawn(async move {
        let operation = async move {
            let (_, _, catalog, _) = read_config(&worker_runtime, config).await?;
            Ok(catalog)
        };
        tokio::select! {
            biased;
            _ = sender.closed() => {}
            result = tokio::time::timeout(OPEN_DEADLINE, operation) => {
                let result = result.unwrap_or_else(|_| Err("Loading Talos contexts timed out".into()));
                let _ = sender.send(result);
            }
        }
    });
    (OwnedJob::new(task), receiver)
}

pub(crate) struct CollectedOverview {
    pub(crate) configuration: freshkube_core::ConfigurationRevision,
    pub(crate) access: Option<freshkube_core::AccessIdentity>,
    pub(crate) cluster: Result<ClusterOverview, String>,
}

pub(crate) struct ObservedNodes {
    pub(crate) access: freshkube_core::AccessIdentity,
    pub(crate) nodes: freshkube_core::kubernetes_summary::Part<
        Vec<freshkube_core::kubernetes_summary::NodeSummary>,
    >,
}

impl ObservedNodes {
    fn for_access(
        self,
        access: freshkube_core::AccessIdentity,
    ) -> Option<
        freshkube_core::kubernetes_summary::Part<
            Vec<freshkube_core::kubernetes_summary::NodeSummary>,
        >,
    > {
        (self.access == access).then_some(self.nodes)
    }
}

async fn configuration_revision(
    config: &AppliedConfig,
    selection: &KubeconfigSelection,
) -> Result<freshkube_core::ConfigurationRevision, String> {
    let path = config.path.clone();
    let selection = selection.clone();
    tokio::task::spawn_blocking(move || {
        freshkube_core::ConfigurationRevision::for_talos_sources(path.as_deref(), &selection)
    })
    .await
    .map_err(|_| "Inspecting the access configuration stopped".to_owned())
}

pub(crate) fn collect(
    runtime: Handle,
    config: AppliedConfig,
    kubeconfig: KubeconfigSelection,
    nodes: Option<ObservedNodes>,
) -> (
    OwnedJob,
    oneshot::Receiver<Result<CollectedOverview, String>>,
) {
    let (mut sender, receiver) = oneshot::channel();
    let worker_runtime = runtime.clone();
    let task = runtime.spawn(async move {
        let operation = async move {
            let configuration = configuration_revision(&config, &kubeconfig).await?;
            let mut access = None;
            let cluster = async {
            let (path, loaded, catalog, identity) = read_config(&worker_runtime, config.clone()).await?;
            // Never refresh a previously populated snapshot: optional-source failures
            // in the shared collector retain data, which must not be labeled fresh here.
            let mut collector = ClusterOverviewCollector::new(Some(path), Some(catalog.current));
            collector.set_kubeconfig_selection(kubeconfig.clone());
            // The Talos connection is reused while path, content, context and endpoints
            // are unchanged, and dropped by the collector after transport failures.
            let mut snapshots = tokio::time::timeout(OPEN_DEADLINE, collector.connect_from_config_reusing(&loaded, identity))
                .await
                .map_err(|_| "Connecting to the Talos context timed out".to_string())?
                .map_err(|error| error.to_string())?;
            let mut snapshot = snapshots.pop().ok_or_else(|| "No Talos context was loaded".to_string())?;
            if snapshot.client.is_none() {
                return Err(snapshot.connection.error().unwrap_or("Talos context is disconnected").to_string());
            }
            let current = freshkube_core::AccessIdentity::for_talos(snapshot.client.as_ref().unwrap(), &kubeconfig);
            access = Some(current);
            collector.set_observed_nodes(nodes.and_then(|nodes| nodes.for_access(current)));
            collector.refresh(&mut snapshot).await;
            if current != freshkube_core::AccessIdentity::for_talos(snapshot.client.as_ref().unwrap(), &kubeconfig) {
                access = None;
                return Err("Access configuration changed during refresh; refresh again".into());
            }
            require_fresh_node_data(snapshot)
            }.await;
            let after = configuration_revision(&config, &kubeconfig).await?;
            if after != configuration {
                return Ok(CollectedOverview { configuration: after, access: None, cluster: Err("Access configuration changed during refresh; refresh again".into()) });
            }
            Ok(CollectedOverview { configuration, access, cluster })
        };
        tokio::select! {
            biased;
            _ = sender.closed() => {}
            result = tokio::time::timeout(COLLECT_DEADLINE, operation) => {
                let result = result.unwrap_or_else(|_| Err("Refreshing the Talos overview timed out".into()));
                let _ = sender.send(result);
            }
        }
    });
    (OwnedJob::new(task), receiver)
}

/// Runs `work` on Tokio under `deadline`. Dropping the job or the receiver
/// cancels it, so a screen that moves on never receives a late result.
pub(crate) fn spawn_job<T, F>(
    runtime: &Handle,
    deadline: Duration,
    timeout_message: String,
    work: F,
) -> (OwnedJob, oneshot::Receiver<Result<T, String>>)
where
    T: Send + 'static,
    F: Future<Output = Result<T, String>> + Send + 'static,
{
    let (mut sender, receiver) = oneshot::channel();
    let task = runtime.spawn(async move {
        tokio::select! {
            biased;
            _ = sender.closed() => {}
            result = tokio::time::timeout(deadline, work) => {
                let _ = sender.send(result.unwrap_or_else(|_| Err(timeout_message)));
            }
        }
    });
    (OwnedJob::new(task), receiver)
}

fn require_fresh_node_data(snapshot: ClusterOverview) -> Result<ClusterOverview, String> {
    if snapshot.versions.is_empty() && snapshot.services.is_empty() {
        return Err("No fresh node version or service responses were received; roster data alone does not establish node health".into());
    }
    Ok(snapshot)
}

pub(crate) fn services(
    runtime: Handle,
    client: TalosClient,
    target: Target,
) -> (
    OwnedJob,
    oneshot::Receiver<Result<Vec<ServiceInfo>, String>>,
) {
    let (mut sender, receiver) = oneshot::channel();
    let task = runtime.spawn(async move {
        let operation = async move {
            if target.address.is_empty() {
                return Err("No node address is selected".to_string());
            }
            let responses = client.with_node(&target.address).services().await.map_err(|error| error.to_string())?;
            if responses.is_empty() {
                return Err("The selected node returned no service response".to_string());
            }
            let mut catalog: Vec<_> = responses.into_iter().flat_map(|node| node.services).collect();
            catalog.sort_by(|left, right| left.id.cmp(&right.id));
            catalog.dedup_by(|left, right| left.id == right.id);
            Ok(catalog)
        };
        tokio::select! {
            biased;
            _ = sender.closed() => {}
            result = tokio::time::timeout(OPEN_DEADLINE, operation) => {
                let result = result.unwrap_or_else(|_| Err("Loading node services timed out".into()));
                let _ = sender.send(result);
            }
        }
    });
    (OwnedJob::new(task), receiver)
}

pub(crate) fn stream(
    runtime: Handle,
    client: TalosClient,
    target: Target,
    services: Vec<ServiceId>,
    tail: i32,
) -> (OwnedJob, mpsc::Receiver<StreamEvent>) {
    let (sender, receiver) = mpsc::channel(STREAM_QUEUE_CAPACITY);
    let tail = bounded_tail(tail);
    let (unique, overflow) = bounded_services(services);
    if overflow || target.address.is_empty() {
        let message = if target.address.is_empty() {
            "No node address is selected"
        } else {
            "At most 16 services can be followed at once"
        };
        let task = runtime.spawn(async move {
            if let Some(service) = unique.into_iter().next() {
                let _ = sender
                    .send(StreamEvent {
                        target,
                        service,
                        result: Err(message.into()),
                    })
                    .await;
            }
        });
        return (OwnedJob::new(task), receiver);
    }
    let node_client = client.with_node(&target.address);
    let pumps = unique
        .into_iter()
        .map(|service| {
            let sender = sender.clone();
            let client = node_client.clone();
            let target = target.clone();
            async move {
                let open = async { client.logs_follow(service.as_ref(), tail).await };
                pump(open, sender, target, service.clone()).await;
            }
        })
        .collect::<Vec<_>>();
    drop(sender);
    (supervise(&runtime, pumps), receiver)
}

fn bounded_tail(tail: i32) -> i32 {
    tail.clamp(0, MAX_TAIL_LINES)
}

fn bounded_services(services: Vec<ServiceId>) -> (Vec<ServiceId>, bool) {
    let mut unique = Vec::new();
    for service in services {
        if !unique.contains(&service) {
            if unique.len() == MAX_STREAM_SERVICES {
                return (unique, true);
            }
            unique.push(service);
        }
    }
    (unique, false)
}

/// JoinSet owns every child, including idle transports. Aborting the owner
/// drops the set and aborts children; closing the receiver wakes every pump.
fn supervise<F>(runtime: &Handle, pumps: Vec<F>) -> OwnedJob
where
    F: Future<Output = ()> + Send + 'static,
{
    OwnedJob::new(runtime.spawn(async move {
        let mut children = JoinSet::new();
        for pump in pumps {
            children.spawn(pump);
        }
        while children.join_next().await.is_some() {}
    }))
}

/// Generic factoring is for lifecycle tests, not a mock Talos transport.
async fn pump<O, S, E>(
    open: O,
    sender: mpsc::Sender<StreamEvent>,
    target: Target,
    service: ServiceId,
) where
    O: Future<Output = Result<S, E>>,
    S: Stream<Item = Result<String, E>>,
    E: Display,
{
    let opened = tokio::select! {
        biased;
        _ = sender.closed() => return,
        result = tokio::time::timeout(OPEN_DEADLINE, open) => result,
    };
    let source = match opened {
        Ok(Ok(source)) => source,
        Ok(Err(error)) => {
            send_event(&sender, &target, &service, Err(error.to_string())).await;
            return;
        }
        Err(_) => {
            send_event(
                &sender,
                &target,
                &service,
                Err("Opening the log stream timed out".into()),
            )
            .await;
            return;
        }
    };
    futures::pin_mut!(source);
    loop {
        let next = tokio::select! {
            biased;
            _ = sender.closed() => return,
            line = source.next() => line,
        };
        let Some(line) = next else { return };
        let result = line.map_err(|error| error.to_string()).and_then(|line| {
            if line.len() > MAX_LINE_BYTES {
                Err("Log line exceeded the 64 KiB limit".into())
            } else {
                Ok(line)
            }
        });
        let failed = result.is_err();
        if !send_event(&sender, &target, &service, result).await || failed {
            return;
        }
    }
}

async fn send_event(
    sender: &mpsc::Sender<StreamEvent>,
    target: &Target,
    service: &ServiceId,
    result: Result<String, String>,
) -> bool {
    let result = match result {
        Err(error) if error.len() > MAX_LINE_BYTES => {
            Err("Log stream error exceeded the 64 KiB limit".into())
        }
        result => result,
    };
    tokio::select! {
        biased;
        _ = sender.closed() => false,
        sent = sender.send(StreamEvent { target: target.clone(), service: service.clone(), result }) => sent.is_ok(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };

    use super::*;

    struct ConfigDirectory(PathBuf);

    impl ConfigDirectory {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let directory = std::env::temp_dir().join(format!(
                "freshkube-desktop-config-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ));
            std::fs::create_dir(&directory).unwrap();
            Self(directory)
        }
    }

    impl Drop for ConfigDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn config_text(current: &str) -> String {
        format!(
            "context: {current}\ncontexts:\n  alpha: &entry\n    endpoints: [192.0.2.1]\n    ca: YQ==\n    crt: Yg==\n    key: Yw==\n  beta: *entry\n"
        )
    }

    #[test]
    fn retained_nodes_belong_to_both_the_access_session_and_revision() {
        use freshkube_core::kubernetes_summary::Part;
        use freshkube_core::{AccessIdentity, AccessSessionId, ConfigurationRevision};
        let directory = ConfigDirectory::new();
        let path = directory.0.join("config");
        std::fs::write(&path, "first").unwrap();
        let before = ConfigurationRevision::from_kubeconfig_sources(std::slice::from_ref(&path));
        let session = AccessSessionId::new();
        let current = AccessIdentity::new(session, before);
        let nodes = || ObservedNodes {
            access: current,
            nodes: Part::Refused("synthetic refusal".into()),
        };
        assert!(matches!(
            nodes().for_access(current),
            Some(Part::Refused(_))
        ));
        assert!(
            nodes()
                .for_access(AccessIdentity::new(AccessSessionId::new(), before))
                .is_none()
        );
        std::fs::write(&path, "other").unwrap();
        let after = ConfigurationRevision::from_kubeconfig_sources(&[path]);
        assert!(
            nodes()
                .for_access(AccessIdentity::new(session, after))
                .is_none()
        );
    }

    #[tokio::test]
    async fn an_unreadable_replacement_still_reports_its_new_revision() {
        let directory = ConfigDirectory::new();
        let path = directory.0.join("config");
        std::fs::write(&path, config_text("alpha")).unwrap();
        let config = AppliedConfig {
            path: Some(path.clone()),
            context: Some("alpha".into()),
        };
        let selection = KubeconfigSelection::TalosControlPlane;
        let before = configuration_revision(&config, &selection).await.unwrap();
        std::fs::write(&path, "invalid: [").unwrap();
        let (_job, receiver) = collect(Handle::current(), config, selection, None);
        let result = receiver.await.unwrap().unwrap();
        assert_ne!(before, result.configuration);
        assert!(result.access.is_none());
        assert!(result.cluster.is_err());
    }

    #[test]
    fn catalog_validates_and_freezes_requested_or_current_context() {
        let directory = ConfigDirectory::new();
        let path = directory.0.join("config");
        std::fs::write(&path, config_text("alpha")).unwrap();
        let (resolved, loaded, catalog) = read_applied_config(AppliedConfig {
            path: Some(path.clone()),
            context: None,
        })
        .unwrap();
        assert_eq!(resolved, path);
        assert_eq!(loaded.context, "alpha");
        assert_eq!(catalog.names, vec!["alpha", "beta"]);
        assert_eq!(catalog.current, "alpha");

        std::fs::write(&path, config_text("beta")).unwrap();
        let (_, _, frozen) = read_applied_config(AppliedConfig {
            path: Some(path.clone()),
            context: Some(catalog.current),
        })
        .unwrap();
        assert_eq!(frozen.current, "alpha");
        assert!(
            read_applied_config(AppliedConfig {
                path: Some(path.clone()),
                context: Some("missing".into()),
            })
            .is_err()
        );

        std::fs::write(&path, config_text("missing")).unwrap();
        assert!(
            read_applied_config(AppliedConfig {
                path: Some(path.clone()),
                context: None,
            })
            .is_err()
        );
        let (_, _, requested) = read_applied_config(AppliedConfig {
            path: Some(path),
            context: Some("beta".into()),
        })
        .unwrap();
        assert_eq!(requested.current, "beta");
    }

    #[test]
    fn bounded_config_reader_rejects_special_oversized_and_invalid_files() {
        let directory = ConfigDirectory::new();
        assert!(
            read_config_file(&directory.0)
                .unwrap_err()
                .contains("regular file")
        );
        let oversized = directory.0.join("oversized");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_CONFIG_BYTES + 1)
            .unwrap();
        assert!(read_config_file(&oversized).unwrap_err().contains("4 MiB"));
        let invalid = directory.0.join("invalid");
        std::fs::write(&invalid, [0xff]).unwrap();
        assert!(read_config_file(&invalid).unwrap_err().contains("UTF-8"));
        std::fs::write(&invalid, "context: alpha\ncontexts: TOP_SECRET_CREDENTIAL").unwrap();
        let error = read_config_file(&invalid).unwrap_err();
        assert!(error.contains("could not be parsed"));
        assert!(!error.contains("TOP_SECRET_CREDENTIAL"));
    }

    #[cfg(unix)]
    #[test]
    fn config_fifo_and_symlink_to_fifo_are_rejected_without_a_writer() {
        use std::os::unix::{ffi::OsStrExt, fs::symlink};
        let directory = ConfigDirectory::new();
        let fifo = directory.0.join("fifo");
        let c_path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: CString provides a valid terminated path for this test-owned FIFO.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        assert!(
            read_config_file(&fifo)
                .unwrap_err()
                .contains("regular file")
        );
        let link = directory.0.join("link");
        symlink(&fifo, &link).unwrap();
        assert!(
            read_config_file(&link)
                .unwrap_err()
                .contains("regular file")
        );
    }

    struct IdleStream {
        dropped: Option<oneshot::Sender<()>>,
    }

    impl Stream for IdleStream {
        type Item = Result<String, String>;

        fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Pending
        }
    }

    impl Drop for IdleStream {
        fn drop(&mut self) {
            if let Some(sender) = self.dropped.take() {
                let _ = sender.send(());
            }
        }
    }

    fn target() -> Target {
        Target {
            epoch: 3,
            context: "fixture".into(),
            node: "node-a".into(),
            address: "192.0.2.1".into(),
        }
    }

    #[test]
    fn service_count_and_tail_are_bounded_without_normalizing_ids() {
        let (unique, overflow) =
            bounded_services(vec!["Kubelet".into(), "kubelet".into(), "Kubelet".into()]);
        assert!(!overflow);
        assert_eq!(
            unique,
            vec![ServiceId::from("Kubelet"), ServiceId::from("kubelet")]
        );
        let (unique, overflow) = bounded_services(
            (0..100)
                .map(|ix| ServiceId::from(format!("service-{ix}")))
                .collect(),
        );
        assert!(overflow);
        assert_eq!(unique.len(), MAX_STREAM_SERVICES);
        assert_eq!(bounded_tail(-1), 0);
        assert_eq!(bounded_tail(i32::MAX), MAX_TAIL_LINES);
        assert_eq!(bounded_tail(17), 17);
    }

    #[test]
    fn roster_only_is_not_a_successful_refresh() {
        let snapshot = ClusterOverview {
            node_ips: [("node-a".into(), "192.0.2.1".into())].into(),
            ..Default::default()
        };
        assert!(require_fresh_node_data(snapshot).is_err());
        let mut snapshot = ClusterOverview::default();
        snapshot.services.push(talos_rs::NodeServices {
            node: "node-a".into(),
            services: vec![],
        });
        assert!(require_fresh_node_data(snapshot).is_ok());
    }

    #[tokio::test]
    async fn dropping_owner_aborts_idle_children_and_drops_source() {
        let (dropped, source_dropped) = oneshot::channel();
        let (started, source_started) = oneshot::channel();
        let (sender, _receiver) = mpsc::channel(STREAM_QUEUE_CAPACITY);
        let open = async {
            let _ = started.send(());
            Ok::<_, String>(IdleStream {
                dropped: Some(dropped),
            })
        };
        let job = supervise(
            &Handle::current(),
            vec![pump(open, sender, target(), ServiceId::from("kubelet"))],
        );
        source_started.await.unwrap();
        // Let the child enter the pending source.next() before cancellation.
        tokio::task::yield_now().await;
        drop(job);
        tokio::time::timeout(Duration::from_secs(1), source_dropped)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn receiver_close_cancels_idle_source() {
        let (dropped, source_dropped) = oneshot::channel();
        let (started, source_started) = oneshot::channel();
        let (sender, mut receiver) = mpsc::channel(STREAM_QUEUE_CAPACITY);
        let open = async {
            let _ = started.send(());
            Ok::<_, String>(IdleStream {
                dropped: Some(dropped),
            })
        };
        let job = supervise(
            &Handle::current(),
            vec![pump(open, sender, target(), ServiceId::from("kubelet"))],
        );
        source_started.await.unwrap();
        receiver.close();
        tokio::time::timeout(Duration::from_secs(1), source_dropped)
            .await
            .unwrap()
            .unwrap();
        drop(job);
    }

    #[tokio::test]
    async fn receiver_close_cancels_pending_open() {
        let (dropped, opening_dropped) = oneshot::channel();
        let (started, opening_started) = oneshot::channel();
        let (sender, mut receiver) = mpsc::channel(STREAM_QUEUE_CAPACITY);
        let open = async {
            let _guard = IdleStream {
                dropped: Some(dropped),
            };
            let _ = started.send(());
            futures::future::pending::<Result<IdleStream, String>>().await
        };
        let job = supervise(
            &Handle::current(),
            vec![pump(open, sender, target(), ServiceId::from("kubelet"))],
        );
        opening_started.await.unwrap();
        receiver.close();
        tokio::time::timeout(Duration::from_secs(1), opening_dropped)
            .await
            .unwrap()
            .unwrap();
        drop(job);
    }

    #[tokio::test]
    async fn queue_is_bounded_and_close_cancels_blocked_send() {
        let (sender, mut receiver) = mpsc::channel(STREAM_QUEUE_CAPACITY);
        let (dropped, source_dropped) = oneshot::channel();
        let source = futures::stream::repeat_with(|| Ok::<_, String>("line".into()));
        let open = async move {
            // The guard stays with the source while its sender is backpressured.
            let guard = IdleStream {
                dropped: Some(dropped),
            };
            Ok::<_, String>(source.map(move |line| {
                let _ = &guard;
                line
            }))
        };
        let job = supervise(
            &Handle::current(),
            vec![pump(open, sender, target(), ServiceId::from("kubelet"))],
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            while receiver.len() < STREAM_QUEUE_CAPACITY {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        tokio::task::yield_now().await;
        assert_eq!(receiver.len(), STREAM_QUEUE_CAPACITY);
        receiver.close();
        tokio::time::timeout(Duration::from_secs(1), source_dropped)
            .await
            .unwrap()
            .unwrap();
        drop(job);
    }

    #[tokio::test]
    async fn oversized_line_is_one_target_tagged_error_then_stream_ends() {
        let (sender, mut receiver) = mpsc::channel(STREAM_QUEUE_CAPACITY);
        let source = futures::stream::iter([
            Ok::<_, String>("x".repeat(MAX_LINE_BYTES + 1)),
            Ok("not sent".into()),
        ]);
        let identity = target();
        let job = supervise(
            &Handle::current(),
            vec![pump(
                futures::future::ready(Ok(source)),
                sender,
                identity.clone(),
                ServiceId::from("kubelet"),
            )],
        );
        let event = receiver.recv().await.unwrap();
        assert_eq!(event.target, identity);
        assert_eq!(event.service, ServiceId::from("kubelet"));
        assert!(event.result.unwrap_err().contains("64 KiB"));
        assert!(receiver.recv().await.is_none());
        drop(job);
    }
}
