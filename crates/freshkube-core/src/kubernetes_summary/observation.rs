//! Coverage and read state, kept apart from the health of observed objects.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::resources::{Failure, FailureKind};

/// The independently synchronized inputs to the cluster summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    Nodes,
    Pods,
    Events,
    Deployments,
    DaemonSets,
    StatefulSets,
    Volumes,
    Claims,
    Namespaces,
    Version,
}

impl Source {
    pub const WATCHED: [Self; 9] = [
        Self::Nodes,
        Self::Pods,
        Self::Events,
        Self::Deployments,
        Self::DaemonSets,
        Self::StatefulSets,
        Self::Volumes,
        Self::Claims,
        Self::Namespaces,
    ];

    pub fn resource(self) -> &'static str {
        match self {
            Self::Nodes => "nodes",
            Self::Pods => "pods",
            Self::Events => "events",
            Self::Deployments => "deployments",
            Self::DaemonSets => "daemonsets",
            Self::StatefulSets => "statefulsets",
            Self::Volumes => "persistentvolumes",
            Self::Claims => "persistentvolumeclaims",
            Self::Namespaces => "namespaces",
            Self::Version => "version",
        }
    }

    pub fn group(self) -> &'static str {
        match self {
            Self::Deployments | Self::DaemonSets | Self::StatefulSets => "apps",
            _ => "",
        }
    }

    pub fn key(self) -> String {
        if self.group().is_empty() {
            self.resource().into()
        } else {
            format!("{}.{}", self.resource(), self.group())
        }
    }

    pub(super) fn kind(self) -> &'static str {
        match self {
            Self::Nodes => "Node",
            Self::Pods => "Pod",
            Self::Events => "Event",
            Self::Deployments => "Deployment",
            Self::DaemonSets => "DaemonSet",
            Self::StatefulSets => "StatefulSet",
            Self::Volumes => "PersistentVolume",
            Self::Claims => "PersistentVolumeClaim",
            Self::Namespaces => "Namespace",
            Self::Version => "Version",
        }
    }

    pub(super) fn namespaced(self) -> bool {
        matches!(
            self,
            Self::Pods
                | Self::Events
                | Self::Deployments
                | Self::DaemonSets
                | Self::StatefulSets
                | Self::Claims
        )
    }
}

/// One applied connection, including a reload of the same configuration path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionIdentity {
    connection: String,
    revision: u64,
}

impl SessionIdentity {
    pub fn new(connection: impl Into<String>, revision: u64) -> Self {
        Self {
            connection: connection.into(),
            revision,
        }
    }
    pub fn connection(&self) -> &str {
        &self.connection
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    Cluster,
    AllNamespaces,
    Namespace(String),
}

/// Exact compatibility contract for a retained observation. Printer Tables
/// and richer object representations must never use the compact summary key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SubscriptionKey {
    session: SessionIdentity,
    source: Source,
    version: String,
    scope: Scope,
    labels: String,
    fields: String,
    representation: String,
}

impl SubscriptionKey {
    pub fn summary(session: SessionIdentity, source: Source) -> Self {
        Self {
            session,
            source,
            version: "v1".into(),
            scope: if source.namespaced() {
                Scope::AllNamespaces
            } else {
                Scope::Cluster
            },
            labels: String::new(),
            fields: if source == Source::Events {
                "type=Warning".into()
            } else {
                String::new()
            },
            representation: "summary-v1".into(),
        }
    }
    pub fn session(&self) -> &SessionIdentity {
        &self.session
    }
    pub fn source(&self) -> Source {
        self.source
    }
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }
    pub fn with_labels(mut self, labels: impl Into<String>) -> Self {
        self.labels = labels.into();
        self
    }
    pub fn with_fields(mut self, fields: impl Into<String>) -> Self {
        self.fields = fields.into();
        self
    }
    pub fn with_representation(mut self, representation: impl Into<String>) -> Self {
        self.representation = representation.into();
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadStatus {
    Syncing,
    Live,
    Resyncing,
    Retrying,
    Refused,
    Failed,
    Limited,
}

/// Typed failure with an intentionally bounded, credential-free description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObservationFailure {
    Read(FailureKind),
    Capacity,
    InvalidObject,
}

impl ObservationFailure {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Read(FailureKind::Forbidden) => "Not allowed to read this collection",
            Self::Read(FailureKind::Unauthorized) => "Kubernetes credentials were rejected",
            Self::Read(FailureKind::NotFound) => "This API is not served",
            Self::Read(FailureKind::Timeout) => "The Kubernetes read timed out",
            Self::Read(FailureKind::Config) => "The Kubernetes configuration is unavailable",
            Self::Read(FailureKind::Unreachable) => "The Kubernetes API is unreachable",
            Self::Read(FailureKind::Other) => "The Kubernetes read failed",
            Self::Capacity => "The collection exceeds the observation memory limit",
            Self::InvalidObject => "The server returned an object without a valid identity",
        }
    }
    pub(super) fn from_kube(error: kube::Error) -> Self {
        Self::Read(Failure::from_kube(error).kind)
    }
    pub(super) fn is_permanent(&self) -> bool {
        match self {
            Self::Read(kind) => kind.is_permanent(),
            Self::Capacity | Self::InvalidObject => true,
        }
    }
}

/// Metadata for a single input. Quiet live watches remain current; timestamps
/// describe evidence received, not a periodic health check or an atomic cluster.
#[derive(Clone, Debug, PartialEq)]
pub struct Observation {
    status: ReadStatus,
    failure: Option<ObservationFailure>,
    last_success: Option<DateTime<Utc>>,
    synchronized_at: Option<DateTime<Utc>>,
    revision: u64,
}

impl Default for Observation {
    fn default() -> Self {
        Self {
            status: ReadStatus::Syncing,
            failure: None,
            last_success: None,
            synchronized_at: None,
            revision: 0,
        }
    }
}

impl Observation {
    pub fn status(&self) -> ReadStatus {
        self.status
    }
    pub fn failure(&self) -> Option<&ObservationFailure> {
        self.failure.as_ref()
    }
    pub fn last_success(&self) -> Option<DateTime<Utc>> {
        self.last_success
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn is_current(&self) -> bool {
        self.status == ReadStatus::Live
    }
    /// A live read kept as an earlier one: its data stays, it no longer
    /// says it is current.
    pub(super) fn last_known(&mut self) {
        if self.status == ReadStatus::Live {
            self.status = ReadStatus::Retrying;
        }
    }
    pub fn has_data(&self) -> bool {
        self.synchronized_at.is_some()
    }
    pub fn is_stale(&self) -> bool {
        self.has_data() && !self.is_current()
    }
    pub fn message(&self) -> Option<&'static str> {
        self.failure
            .as_ref()
            .map(ObservationFailure::message)
            .or(match self.status {
                ReadStatus::Syncing => Some("Waiting for the initial list"),
                ReadStatus::Resyncing => Some("Refreshing the collection"),
                _ => None,
            })
    }
    pub(super) fn syncing(&mut self) {
        self.status = if self.has_data() {
            ReadStatus::Resyncing
        } else {
            ReadStatus::Syncing
        };
        self.failure = None;
        self.revision += 1;
    }
    pub(super) fn success(&mut self, now: DateTime<Utc>, synchronized: bool) {
        self.status = ReadStatus::Live;
        self.failure = None;
        self.last_success = Some(now);
        if synchronized {
            self.synchronized_at = Some(now);
        }
        self.revision += 1;
    }
    pub(super) fn failed(&mut self, failure: ObservationFailure) {
        self.status = match failure {
            ObservationFailure::Read(FailureKind::Forbidden) => ReadStatus::Refused,
            ObservationFailure::Capacity => ReadStatus::Limited,
            _ if failure.is_permanent() => ReadStatus::Failed,
            _ => ReadStatus::Retrying,
        };
        self.failure = Some(failure);
        self.revision += 1;
    }
}

pub type Observations = BTreeMap<Source, Observation>;
