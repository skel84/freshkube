//! Visible-only node metrics. The workspace owns cancellation and publication;
//! Nodes' handle hears the connection and whether its page shows.
use std::{collections::HashMap, time::Duration};

use freshkube_core::resources::{FailureKind, NodeUsage, list_node_usage};
use freshkube_reads::{Loader, OnShow};

use super::*;
use crate::{backend, resources::ClusterAccess, screens::SCREEN_DEADLINE, state::Request};

const INTERVAL: Duration = Duration::from_secs(15);
const ON_SHOW: OnShow = OnShow::ReadIfOlder(Duration::ZERO);

type Usage = HashMap<String, NodeUsage>;

pub(super) struct Metrics {
    runtime: tokio::runtime::Handle,
    /// The connection the shell last handed Nodes, read while it shows.
    kube: Option<crate::resources::ClusterSource>,
    pub(super) source: Option<String>,
    /// The owner needs visibility for source changes and arriving example rows;
    /// the loader hears the same changes through `set_visible`.
    pub(super) visible: bool,
    /// Example mode answers from the rows themselves, so new rows answer at once.
    example: bool,
    pub(super) snapshot: Loader<Usage, String>,
}

impl Metrics {
    pub(super) fn new(runtime: tokio::runtime::Handle) -> Self {
        Self {
            runtime,
            kube: None,
            source: None,
            visible: false,
            example: false,
            snapshot: Loader::default(),
        }
    }

    pub(super) fn apply(
        &mut self,
        revision: u64,
        request: &Request<String>,
        result: Result<Vec<NodeUsage>, String>,
        cx: &App,
    ) -> bool {
        // Resetting a loader forgets its snapshot's request sequence. Its
        // revision also rejects a late answer after switching away and back
        // to the same source, as the old owner's generation did.
        if !self.visible || revision != self.snapshot.revision() {
            return false;
        }
        self.snapshot.land(request, result.map(usage_by_name), cx)
    }
}

fn usage_by_name(rows: Vec<NodeUsage>) -> Usage {
    rows.into_iter()
        .map(|row| (row.name.clone(), row))
        .collect()
}

impl Nodes {
    /// The connection to read nodes' use from; a new one reads again if
    /// Nodes shows.
    pub(super) fn set_metrics_source(
        &mut self,
        source: Option<crate::resources::ClusterSource>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.metrics.kube = source.clone();
        let visible = self.metrics.visible;
        self.sync_metrics(visible, source, window, cx)
    }

    /// Nodes reads its nodes' use while its page shows, whatever its pane
    /// does.
    pub(super) fn set_metrics_visible(
        &mut self,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = self.metrics.kube.clone();
        self.sync_metrics(visible, source, window, cx)
    }

    fn sync_metrics(
        &mut self,
        visible: bool,
        source: Option<crate::resources::ClusterSource>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = source.as_ref().map(|source| source.id.clone());
        let metrics = &mut self.metrics;
        if metrics.source == id && metrics.visible == visible {
            return;
        }
        if metrics.source != id {
            metrics.snapshot.reset();
        }
        let read_again = metrics.snapshot.set_visible(visible, ON_SHOW, cx);
        let changed_source = metrics.source != id;
        metrics.source = id;
        metrics.visible = visible;
        metrics.example = source
            .as_ref()
            .is_some_and(|source| matches!(source.access, ClusterAccess::Example(_)));
        self.rebuild_resource_cells();
        cx.notify();
        if visible
            && (changed_source || read_again)
            && let Some(source) = source
        {
            self.read_metrics(source, window, cx);
        }
    }

    /// Each read's loader task delivers its answer, waits the same interval,
    /// then hands ownership to the next read. Hiding also drops that wait.
    fn read_metrics(
        &mut self,
        source: crate::resources::ClusterSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let request = self.metrics.snapshot.start(source.id.clone());
        let revision = self.metrics.snapshot.revision();
        let (job, receiver) = if self.metrics.example {
            let rows = self.example_node_usage();
            self.metrics.apply(revision, &request, Ok(rows), cx);
            self.rebuild_resource_cells();
            (None, None)
        } else {
            let access = source.access.clone();
            let (job, receiver) = backend::spawn_job(
                &self.metrics.runtime,
                SCREEN_DEADLINE,
                "Reading nodes' use timed out".into(),
                async move {
                    let client = access.client().await?;
                    list_node_usage(&client)
                        .await
                        .map_err(|failure| match failure.kind {
                            FailureKind::NotFound => "metrics-server isn't installed".to_owned(),
                            FailureKind::Forbidden => "Not permitted to read nodes' use".to_owned(),
                            _ => failure.to_string(),
                        })
                },
            );
            (Some(job), Some(receiver))
        };
        let task = cx.spawn_in(window, async move |this, cx| {
            if let Some(receiver) = receiver {
                let result = receiver
                    .await
                    .unwrap_or_else(|_| Err("Reading nodes' use stopped".into()));
                let applied = this.update(cx, |nodes, cx| {
                    if !nodes.metrics.apply(revision, &request, result, cx) {
                        return false;
                    }
                    nodes.rebuild_resource_cells();
                    cx.notify();
                    true
                });
                if !matches!(applied, Ok(true)) {
                    return;
                }
            }
            cx.background_executor().timer(INTERVAL).await;
            _ = this.update_in(cx, |nodes, window, cx| {
                if nodes.metrics.visible && nodes.metrics.source.as_ref() == Some(&source.id) {
                    nodes.read_metrics(source, window, cx);
                    cx.notify();
                }
            });
        });
        self.metrics.snapshot.run(job, task);
    }

    /// Example samples follow the rows as they arrive: a summary that lands
    /// after the page shows must not wait for the next tick to draw its use.
    pub(super) fn refresh_example_metrics(&mut self) {
        if !self.metrics.example || !self.metrics.visible {
            return;
        }
        let rows = usage_by_name(self.example_node_usage());
        // This is a projection of new fixture rows, not another poll. Keep
        // the loader's existing interval task while updating its samples.
        if let Some(data) = self.metrics.snapshot.data_mut() {
            *data = rows;
        }
    }

    /// Deterministic fixture samples on the summary's node identities. No API
    /// is contacted; requests and allocatable still come from the real reducer.
    fn example_node_usage(&self) -> Vec<NodeUsage> {
        self.rows
            .iter()
            .filter_map(|row| {
                let node = row.kubernetes.as_ref()?;
                let requests = node.requests;
                // -02 runs above its requests, -01 near its allocatable memory,
                // the rest below their requests: every meter state shows.
                let ratio = if node.name.contains("02") { 1.6 } else { 0.7 };
                let allocatable = node
                    .allocatable
                    .get("memory")
                    .and_then(|q| freshkube_core::resources::quantity(&q.0));
                let memory = match allocatable {
                    Some(allocatable) if node.name.contains("01") => Some(allocatable * 0.88),
                    _ => requests.memory_bytes.map(|value| value * ratio),
                };
                Some(NodeUsage {
                    name: node.name.clone(),
                    sampled_at: None,
                    usage: freshkube_core::resources::Amounts {
                        cpu_millis: requests.cpu_millis.map(|value| (value * ratio).max(75.)),
                        memory_bytes: memory,
                    },
                })
            })
            .collect()
    }
}
