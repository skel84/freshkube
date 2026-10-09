//! Where a Stage's change comes from: example data's, or a live
//! Applications read's, which records each Kargo Stage's current Freight
//! for the change page to follow.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use freshkube_core::applications::{ArgoScope, Inputs};
use freshkube_core::delivery::change::{self as delivery_change, live::Place};

use super::Fetch;
use crate::resources::{KubeAccess, KubeSource};

/// Where a Stage's change comes from.
#[derive(Clone, Debug)]
pub(in crate::applications) enum Changes {
    /// Nothing reads a change.
    None,
    /// Example data's, its times counted from when it was read.
    Example(DateTime<Utc>),
    /// The open connection's, from the Freight each Stage runs.
    Live(Arc<LiveChanges>),
}

/// What a live read found to follow: each Kargo Stage's current Freight,
/// and where to read the rest.
#[derive(Debug)]
pub(in crate::applications) struct LiveChanges {
    /// The connection's id, which objects to open carry.
    pub(super) cluster: String,
    /// The context's name, which the page shows.
    pub(super) label: String,
    /// The namespace Argo CD's Applications were first read in.
    pub(in crate::applications) argocd_namespace: String,
    /// By project and Stage.
    pub(super) current: BTreeMap<(String, String), String>,
}

impl LiveChanges {
    pub(in crate::applications) fn of(inputs: &Inputs, source: &KubeSource) -> Self {
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
    pub(in crate::applications) fn live(inputs: &Inputs, source: &KubeSource) -> Self {
        Self::Live(Arc::new(LiveChanges::of(inputs, source)))
    }

    /// The Freight a Kargo project's Stage carries, when a change was read.
    pub(in crate::applications) fn freight_of(&self, project: &str, stage: &str) -> Option<String> {
        match self {
            Self::None => None,
            Self::Example(_) => {
                delivery_change::example::freight_of(project, stage).map(str::to_owned)
            }
            Self::Live(live) => live
                .current
                .get(&(project.to_owned(), stage.to_owned()))
                .cloned(),
        }
    }

    /// What reads the change a Kargo project's Stage carries, when it
    /// carries one; a live one reads through `access`.
    pub(in crate::applications) fn fetch(
        &self,
        project: &str,
        stage: &str,
        runtime: &tokio::runtime::Handle,
        access: &KubeAccess,
        delay: Duration,
    ) -> Option<Fetch> {
        let freight = self.freight_of(project, stage)?;
        match self {
            Self::None => None,
            Self::Example(now) => Some(Fetch::Example {
                project: project.to_owned(),
                stage: stage.to_owned(),
                now: *now,
                delay,
            }),
            Self::Live(live) => Some(Fetch::Live {
                runtime: runtime.clone(),
                access: access.clone(),
                place: Place {
                    cluster: live.cluster.clone(),
                    label: live.label.clone(),
                    project: project.to_owned(),
                    freight,
                    argocd_namespace: live.argocd_namespace.clone(),
                },
            }),
        }
    }
}
