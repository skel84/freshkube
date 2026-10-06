//! Cluster facts derived from session-owned compact Kubernetes reflectors.
//! Each collection keeps its own coverage, freshness and typed failure.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

use crate::workloads::{PodInfo, WorkloadCollectionOutcome};

mod driver;
mod observation;
mod project;
mod requests;
mod retained;
mod session;
mod summarize;
pub use driver::DEBOUNCE;
pub use observation::{
    Observation, ObservationFailure, Observations, ReadStatus, Scope, SessionIdentity, Source,
    SubscriptionKey,
};
pub use retained::{RetainedObject, SummaryResource};
pub use session::{Limits, Publication, Session, Subscription};

pub const ISSUE_LIMIT: usize = 200;

/// A successful part, or the reason that this list cannot be shown.
#[derive(Clone, Debug, PartialEq)]
pub enum Part<T> {
    Loaded(T),
    Refused(Unavailable<T>),
    Failed(Unavailable<T>),
}

impl<T> Part<T> {
    /// Last completely observed value, which may be stale. Check `is_current`
    /// before using a missing object or a zero to establish current absence.
    pub fn loaded(&self) -> Option<&T> {
        match self {
            Self::Loaded(value) => Some(value),
            Self::Refused(unavailable) | Self::Failed(unavailable) => {
                unavailable.last_good.as_ref()
            }
        }
    }
    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Refused(error) | Self::Failed(error) => Some(&error.message),
            _ => None,
        }
    }
}

/// A read failure and the last completely observed value, when one exists.
/// Keeping a value never turns a refused read into a successful read.
#[derive(Clone, Debug, PartialEq)]
pub struct Unavailable<T> {
    pub failure: Option<ObservationFailure>,
    pub message: String,
    pub last_good: Option<T>,
}
impl<T> From<String> for Unavailable<T> {
    fn from(message: String) -> Self {
        Self {
            failure: None,
            message,
            last_good: None,
        }
    }
}
impl<T> From<&str> for Unavailable<T> {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
impl<T> std::fmt::Display for Unavailable<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}
impl<T> Part<T> {
    pub fn is_current(&self) -> bool {
        matches!(self, Self::Loaded(_))
    }
    fn observed(value: T, observation: &Observation) -> Self {
        if observation.is_current() {
            return Self::Loaded(value);
        }
        let unavailable = Unavailable {
            failure: observation.failure().cloned(),
            message: observation.message().unwrap_or("Read unavailable").into(),
            last_good: observation.has_data().then_some(value),
        };
        if observation.status() == ReadStatus::Refused {
            Self::Refused(unavailable)
        } else {
            Self::Failed(unavailable)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NodeCondition {
    pub kind: String,
    pub status: String,
    pub reason: String,
    pub message: String,
    pub since: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NodeSummary {
    pub uid: String,
    pub name: String,
    pub conditions: Vec<NodeCondition>,
    pub unschedulable: bool,
    pub roles: Vec<String>,
    pub addresses: Vec<(String, String)>,
    pub kubelet_version: String,
    pub capacity: BTreeMap<String, Quantity>,
    pub allocatable: BTreeMap<String, Quantity>,
    /// Effective requests of assigned, non-terminated pods. Unknown until Pods
    /// has been observed; `pods_current` describes this value's freshness too.
    pub requests: crate::resources::Amounts,
    pub taints: Vec<String>,
    pub pods: usize,
    pub pods_current: bool,
    pub pods_observed: bool,
}

impl NodeSummary {
    pub fn ready(&self) -> Option<&NodeCondition> {
        self.conditions
            .iter()
            .find(|condition| condition.kind == "Ready")
    }
    pub fn is_ready(&self) -> bool {
        self.ready()
            .is_some_and(|condition| condition.status == "True")
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PodSummary {
    pub total: usize,
    /// Counts from the same complete Pod observation, for namespace navigation.
    pub by_namespace: BTreeMap<String, usize>,
    pub phases: BTreeMap<String, usize>,
    pub issues_by_status: BTreeMap<String, usize>,
    pub on_not_ready: usize,
    pub issues: Vec<PodInfo>,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Warning {
    pub namespace: String,
    pub kind: String,
    pub name: String,
    pub reason: String,
    pub message: String,
    pub at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct EventSummary {
    pub total: usize,
    pub reasons: BTreeMap<String, usize>,
    pub newest: Vec<Warning>,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PendingClaim {
    pub namespace: String,
    pub name: String,
    pub since: Option<DateTime<Utc>>,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ClaimSummary {
    pub pending_count: usize,
    pub pending: Vec<PendingClaim>,
    pub bound: usize,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct KubernetesSummary {
    pub observations: Observations,
    /// Identities of subjects retained by the summary; no full objects remain.
    pub references: BTreeMap<(String, String, String), String>,
    pub version: Part<String>,
    pub nodes: Part<Vec<NodeSummary>>,
    pub pods: Part<PodSummary>,
    pub workloads: WorkloadCollectionOutcome,
    pub events: Part<EventSummary>,
    pub claims: Part<ClaimSummary>,
    pub available_volumes: Part<usize>,
    pub namespaces: Part<usize>,
}

#[cfg(test)]
mod session_tests;

impl KubernetesSummary {
    /// Last committed roster plus explicit coverage. Partial evidence is useful
    /// for retaining rows, but cannot prove that an unobserved node is absent.
    pub fn node_roster(
        &self,
    ) -> crate::security_lifecycle::SourceSnapshot<
        Vec<crate::security_lifecycle::KubernetesNodeRosterEntry>,
    > {
        use crate::security_lifecycle::{KubernetesNodeRosterEntry, SourceSnapshot};
        let Some(nodes) = self.nodes.loaded() else {
            return SourceSnapshot::Unavailable {
                reason: self.nodes.error().unwrap_or("Waiting for Nodes").into(),
            };
        };
        let value = nodes
            .iter()
            .map(|node| KubernetesNodeRosterEntry {
                name: node.name.clone(),
                internal_address: node
                    .addresses
                    .iter()
                    .find(|(kind, _)| kind == "InternalIP")
                    .map(|(_, address)| address.clone()),
                is_control_plane: node
                    .roles
                    .iter()
                    .any(|role| matches!(role.as_str(), "control-plane" | "master")),
            })
            .collect();
        if self.nodes.is_current() {
            SourceSnapshot::Available(value)
        } else {
            SourceSnapshot::Partial {
                value,
                warnings: vec![format!(
                    "{} · showing last known Nodes",
                    self.nodes.error().unwrap_or("Refreshing Nodes")
                )],
            }
        }
    }
}

#[cfg(test)]
mod driver_tests;
