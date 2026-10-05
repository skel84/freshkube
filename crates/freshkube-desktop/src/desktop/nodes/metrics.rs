//! Visible-only node metrics. The workspace owns cancellation and publication.
use std::{collections::HashMap, time::Duration};

use freshkube_core::resources::{FailureKind, NodeUsage, list_node_usage};

use super::*;
use crate::{
    backend::{self, OwnedJob},
    resources::KubeAccess,
    screens::SCREEN_DEADLINE,
    state::{Request, Snapshot},
};

const INTERVAL: Duration = Duration::from_secs(15);

pub(super) struct Metrics {
    runtime: tokio::runtime::Handle,
    pub(super) source: Option<String>,
    pub(super) visible: bool,
    /// Example mode answers from the rows themselves, so new rows answer at once.
    example: bool,
    pub(super) generation: u64,
    pub(super) snapshot: Snapshot<HashMap<String, NodeUsage>, String>,
    pub(super) job: Option<OwnedJob>,
    pub(super) delivery: Option<Task<()>>,
}

impl Metrics {
    pub(super) fn new(runtime: tokio::runtime::Handle) -> Self {
        Self {
            runtime,
            source: None,
            visible: false,
            example: false,
            generation: 0,
            snapshot: Snapshot::default(),
            job: None,
            delivery: None,
        }
    }

    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.delivery = None;
        self.job = None;
    }

    pub(super) fn apply(
        &mut self,
        generation: u64,
        request: &Request<String>,
        result: Result<Vec<NodeUsage>, String>,
    ) -> bool {
        if !self.visible || generation != self.generation {
            return false;
        }
        let result = result.map(|rows| {
            rows.into_iter()
                .map(|row| (row.name.clone(), row))
                .collect()
        });
        let applied = self.snapshot.apply(request, result);
        if applied {
            self.job = None;
        }
        applied
    }
}

impl Pilot {
    pub(in crate::desktop) fn sync_node_metrics(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let visible = self.page == Page::Nodes;
        let source = self.kube_source();
        let id = source.as_ref().map(|source| source.id.clone());
        let metrics = &mut self.node_workspace.metrics;
        if metrics.source == id && metrics.visible == visible {
            return;
        }
        metrics.cancel();
        if metrics.source != id {
            metrics.snapshot = Snapshot::default();
        }
        metrics.source = id;
        metrics.visible = visible;
        metrics.example = source
            .as_ref()
            .is_some_and(|source| matches!(source.access, KubeAccess::Example));
        self.node_workspace.rebuild_resource_cells();
        if !visible {
            return;
        }
        let Some(source) = source else {
            return;
        };
        let generation = self.node_workspace.metrics.generation;
        let example = matches!(source.access, KubeAccess::Example);
        if example {
            let request = self
                .node_workspace
                .metrics
                .snapshot
                .begin(source.id.clone());
            let rows = self.node_workspace.example_node_usage();
            self.node_workspace
                .metrics
                .apply(generation, &request, Ok(rows));
            self.node_workspace.rebuild_resource_cells();
        }
        self.node_workspace.metrics.delivery = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                if example {
                    cx.background_executor().timer(INTERVAL).await;
                }
                let started = this.update(cx, |pilot, cx| {
                    let metrics = &mut pilot.node_workspace.metrics;
                    if !metrics.visible
                        || metrics.generation != generation
                        || metrics.source.as_ref() != Some(&source.id)
                    {
                        return None;
                    }
                    let request = metrics.snapshot.begin(source.id.clone());
                    if example {
                        let rows = pilot.node_workspace.example_node_usage();
                        pilot
                            .node_workspace
                            .metrics
                            .apply(generation, &request, Ok(rows));
                        pilot.node_workspace.rebuild_resource_cells();
                        cx.notify();
                        return Some(None);
                    }
                    let access = source.access.clone();
                    let (job, receiver) = backend::spawn_job(
                        &metrics.runtime,
                        SCREEN_DEADLINE,
                        "Reading nodes' use timed out".into(),
                        async move {
                            let client = access.client().await?;
                            list_node_usage(&client)
                                .await
                                .map_err(|failure| match failure.kind {
                                    FailureKind::NotFound => {
                                        "metrics-server isn't installed".to_owned()
                                    }
                                    FailureKind::Forbidden => {
                                        "Not permitted to read nodes' use".to_owned()
                                    }
                                    _ => failure.to_string(),
                                })
                        },
                    );
                    metrics.job = Some(job);
                    Some(Some((request, receiver)))
                });
                let Ok(Some(started)) = started else {
                    break;
                };
                if let Some((request, receiver)) = started {
                    let result = receiver
                        .await
                        .unwrap_or_else(|_| Err("Reading nodes' use stopped".into()));
                    let applied = this.update(cx, |pilot, cx| {
                        if !pilot
                            .node_workspace
                            .metrics
                            .apply(generation, &request, result)
                        {
                            return false;
                        }
                        pilot.node_workspace.rebuild_resource_cells();
                        cx.notify();
                        true
                    });
                    if !matches!(applied, Ok(true)) {
                        break;
                    }
                    cx.background_executor().timer(INTERVAL).await;
                }
            }
        }));
    }
}

impl Nodes {
    /// Example samples follow the rows as they arrive: a summary that lands
    /// after the page shows must not wait for the next tick to draw its use.
    pub(super) fn refresh_example_metrics(&mut self) {
        let metrics = &self.metrics;
        let Some(source) = metrics
            .source
            .clone()
            .filter(|_| metrics.example && metrics.visible)
        else {
            return;
        };
        let generation = metrics.generation;
        let rows = self.example_node_usage();
        let request = self.metrics.snapshot.begin(source);
        self.metrics.apply(generation, &request, Ok(rows));
    }

    /// Deterministic fixture samples on the summary's node identities. No API
    /// is contacted; requests and allocatable still come from the real reducer.
    fn example_node_usage(&self) -> Vec<NodeUsage> {
        self.rows
            .iter()
            .filter_map(|row| {
                let node = row.kubernetes.as_ref()?;
                let requests = node.requests;
                let ratio = if node.name.contains("02") { 1.6 } else { 0.7 };
                Some(NodeUsage {
                    name: node.name.clone(),
                    sampled_at: None,
                    usage: freshkube_core::resources::Amounts {
                        cpu_millis: requests.cpu_millis.map(|value| (value * ratio).max(75.)),
                        memory_bytes: row
                            .talos
                            .as_ref()
                            .and_then(|node| node.memory.as_ref())
                            .map(|memory| memory.used as f64)
                            .or(requests.memory_bytes.map(|value| value * ratio)),
                    },
                })
            })
            .collect()
    }
}
