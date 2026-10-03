//! Cluster facts derived from cache-backed Kubernetes lists. Objects are
//! dropped after collection; a refused list does not hide other parts.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

use crate::workloads::{PodInfo, WorkloadCollectionOutcome};

mod collect;
mod derive;
pub use collect::collect_kubernetes_summary;
pub use derive::derive;

pub const ISSUE_LIMIT: usize = 200;

/// A successful part, or the reason that this list cannot be shown.
#[derive(Clone, Debug, PartialEq)]
pub enum Part<T> {
    Loaded(T),
    Refused(String),
    Failed(String),
}

impl<T> Part<T> {
    pub fn loaded(&self) -> Option<&T> {
        match self {
            Self::Loaded(value) => Some(value),
            _ => None,
        }
    }
    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Refused(error) | Self::Failed(error) => Some(error),
            _ => None,
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
    pub taints: Vec<String>,
    pub pods: usize,
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

impl KubernetesSummary {
    pub fn refresh_failure(&self) -> Option<&str> {
        fn failed<T>(part: &Part<T>) -> Option<&str> {
            match part {
                Part::Failed(message) => Some(message),
                _ => None,
            }
        }
        failed(&self.version)
            .or_else(|| failed(&self.nodes))
            .or_else(|| failed(&self.pods))
            .or_else(|| failed(&self.events))
            .or_else(|| failed(&self.claims))
            .or_else(|| failed(&self.available_volumes))
            .or_else(|| failed(&self.namespaces))
            .or_else(|| {
                self.workloads
                    .unavailable()
                    .iter()
                    .find(|error| !error.message.contains("forbidden"))
                    .map(|error| error.message.as_str())
            })
    }
}

impl<T> Part<T> {
    fn map<U>(self, map: impl FnOnce(T) -> U) -> Part<U> {
        match self {
            Self::Loaded(value) => Part::Loaded(map(value)),
            Self::Refused(error) => Part::Refused(error),
            Self::Failed(error) => Part::Failed(error),
        }
    }
}

#[cfg(test)]
mod tests;
