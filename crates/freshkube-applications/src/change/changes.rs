//! Where a Stage's change comes from: example data's, or a live
//! Applications read's, which records each Kargo Stage's current Freight
//! for the change page to follow.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use freshkube_delivery::applications::{ArgoScope, Inputs};
use freshkube_delivery::change::example::Shape;
use freshkube_delivery::change::{self as delivery_change, live::Place};

use super::Fetch;
use crate::across::Across;
use crate::resources::{ClusterAccess, ClusterSource};

/// Where a Stage's change comes from.
#[derive(Clone, Debug)]
pub enum Changes {
    /// Nothing reads a change.
    None,
    /// Example data's, its times counted from when it was read, in the
    /// example's shape.
    Example(DateTime<Utc>, Shape),
    /// The open connection's, from the Freight each Stage runs.
    Live(Arc<LiveChanges>),
}

/// What a live read found to follow: each Kargo Stage's current Freight,
/// wherever Kargo was read, and the open connection the rest is read
/// from.
#[derive(Debug)]
pub struct LiveChanges {
    /// The connection's id, which objects to open carry.
    pub(super) cluster: String,
    /// The context's name, which the page shows.
    pub(super) label: String,
    /// The namespace Argo CD's Applications were first read in.
    pub argocd_namespace: String,
    /// By project and Stage.
    pub(super) current: BTreeMap<(String, String), String>,
}

impl LiveChanges {
    pub fn of(inputs: &Inputs, source: &ClusterSource) -> Self {
        let mut current = BTreeMap::new();
        let mut argocd_namespace = None;
        for session in &inputs.sessions {
            if let ArgoScope::Namespace { namespaces, .. } = &session.argo_scope {
                argocd_namespace = argocd_namespace.or_else(|| namespaces.first().cloned());
            }
            for project in session.kargo.read().into_iter().flatten() {
                for stage in project.stages.read().into_iter().flatten() {
                    if let Some(freight) = stage.current_freight.first() {
                        current.insert((project.name.clone(), stage.name.clone()), freight.clone());
                    }
                }
            }
        }
        Self {
            cluster: source.id.clone(),
            label: source.context.clone(),
            argocd_namespace: argocd_namespace.unwrap_or_else(|| "argocd".into()),
            current,
        }
    }
}

impl Changes {
    /// A live read's: each Stage's current Freight on `source`.
    pub(crate) fn live(inputs: &Inputs, source: &ClusterSource) -> Self {
        Self::Live(Arc::new(LiveChanges::of(inputs, source)))
    }

    /// The Freight a Kargo project's Stage carries, when a change was read.
    pub fn freight_of(&self, project: &str, stage: &str) -> Option<String> {
        match self {
            Self::None => None,
            Self::Example(..) => {
                delivery_change::example::freight_of(project, stage).map(str::to_owned)
            }
            Self::Live(live) => live
                .current
                .get(&(project.to_owned(), stage.to_owned()))
                .cloned(),
        }
    }

    /// What reads the change a Kargo project's Stage carries, when it
    /// carries one; a live one reads from `access` across the workspace,
    /// each part where `across` says, its Stages' destinations resolved
    /// through its mapping.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fetch(
        &self,
        project: &str,
        stage: &str,
        runtime: &tokio::runtime::Handle,
        access: &ClusterAccess,
        secrets: Option<freshkube_core::secrets::Secrets>,
        across: Across,
        delay: Duration,
    ) -> Option<Fetch> {
        let freight = self.freight_of(project, stage)?;
        match self {
            Self::None => None,
            Self::Example(now, shape) => Some(Fetch::Example {
                project: project.to_owned(),
                freight,
                now: *now,
                shape: *shape,
                delay,
            }),
            Self::Live(live) => Some(Fetch::Live {
                runtime: runtime.clone(),
                access: access.clone(),
                secrets,
                // A new page has no earlier setup reads to invalidate.
                setup_revision: 0,
                place: Box::new(Place {
                    cluster: live.cluster.clone(),
                    label: live.label.clone(),
                    project: project.to_owned(),
                    freight,
                    argocd_namespace: live.argocd_namespace.clone(),
                    mapping: across.mapping,
                    sources: across.sources,
                    public_key_file: across.public_key_file,
                    evidence_result: across.evidence_result,
                    build_namespace: across.build_namespace,
                }),
                kubeconfig: across.kubeconfig,
            }),
        }
    }
}
