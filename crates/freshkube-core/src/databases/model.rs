use chrono::{DateTime, Utc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    CloudNativePg,
    MariaDb,
}
impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Self::CloudNativePg => "PostgreSQL · CloudNativePG",
            Self::MariaDb => "MariaDB",
        }
    }
}

/// Unknown is distinct from a reported zero, false or empty collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fact<T> {
    Known(T),
    Unknown,
}
impl<T> Fact<T> {
    pub fn known(&self) -> Option<&T> {
        match self {
            Self::Known(value) => Some(value),
            Self::Unknown => None,
        }
    }
}
impl<T> From<Option<T>> for Fact<T> {
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::Unknown, Self::Known)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub group: String,
    pub version: String,
    pub kind: String,
    pub plural: String,
    pub namespace: String,
    pub name: String,
    pub uid: String,
}
/// A name reference is weaker than an owner reference carrying the incarnation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterRef {
    pub engine: Engine,
    pub namespace: String,
    pub name: String,
    pub uid: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Coverage {
    Complete,
    Partial,
    Refused,
    NotServed,
    Unsupported,
    TimedOut,
    Unreachable,
    Failed,
}
impl Coverage {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Complete => "Read",
            Self::Partial => "Partial / capped",
            Self::Refused => "Not permitted",
            Self::NotServed => "API not served",
            Self::Unsupported => "Unsupported API",
            Self::TimedOut => "Timed out",
            Self::Unreachable => "Unreachable",
            Self::Failed => "Couldn't read",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read<T> {
    pub items: Vec<T>,
    pub coverage: Coverage,
    pub observed_at: DateTime<Utc>,
    pub stale: bool,
}
impl<T> Read<T> {
    pub fn empty(coverage: Coverage, observed_at: DateTime<Utc>) -> Self {
        Self {
            items: Vec::new(),
            coverage,
            observed_at,
            stale: false,
        }
    }
    pub fn complete(&self) -> bool {
        self.coverage == Coverage::Complete && !self.stale
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Condition {
    pub kind: String,
    pub status: String,
    pub reason: String,
    pub transition_time: Option<DateTime<Utc>>,
    pub observed_generation: Option<i64>,
    pub current: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    Primary,
    Replica,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    pub name: String,
    pub role: Fact<Role>,
    pub state: Fact<String>,
    pub replication: Fact<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Database {
    pub created: Option<DateTime<Utc>>,
    pub identity: Identity,
    pub engine: Engine,
    pub desired_instances: Fact<u32>,
    pub ready_instances: Fact<u32>,
    pub current_primary: Fact<String>,
    pub target_primary: Fact<String>,
    pub instances: Vec<Instance>,
    pub phase: Fact<String>,
    pub conditions: Vec<Condition>,
    /// Presence of a backup configuration; its external dependencies remain unverified.
    pub backup_configuration: Fact<bool>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupState {
    Completed,
    Failed,
    Running,
    Pending,
    Unknown,
}
impl BackupState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::Running => "Running",
            Self::Pending => "Pending",
            Self::Unknown => "Unknown",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backup {
    pub identity: Identity,
    pub cluster: Option<ClusterRef>,
    pub state: BackupState,
    pub method: Fact<String>,
    pub started: Option<DateTime<Utc>>,
    pub completed: Option<DateTime<Utc>>,
    pub created: Option<DateTime<Utc>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub created: Option<DateTime<Utc>>,
    pub identity: Identity,
    pub cluster: Option<ClusterRef>,
    pub schedule: Fact<String>,
    pub suspended: Fact<bool>,
    pub method: Fact<String>,
    pub last: Option<DateTime<Utc>>,
    pub next: Option<DateTime<Utc>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inventory {
    pub discovery: Coverage,
    pub databases: Read<Database>,
    pub backups: Read<Backup>,
    pub schedules: Read<Schedule>,
}
impl Inventory {
    pub fn unavailable(coverage: Coverage, at: DateTime<Utc>) -> Self {
        Self {
            discovery: coverage.clone(),
            databases: Read::empty(coverage.clone(), at),
            backups: Read::empty(coverage.clone(), at),
            schedules: Read::empty(coverage, at),
        }
    }
    /// A failed kind keeps its old evidence with the new failure and old timestamp.
    pub fn retain(&mut self, previous: &Self) {
        fn retain<T: Clone>(new: &mut Read<T>, old: &Read<T>) {
            if !new.complete() && new.items.is_empty() && !old.items.is_empty() {
                new.items = old.items.clone();
                new.observed_at = old.observed_at;
                new.stale = true;
            }
        }
        retain(&mut self.databases, &previous.databases);
        retain(&mut self.backups, &previous.backups);
        retain(&mut self.schedules, &previous.schedules);
    }
}

impl crate::HasHealth for Database {
    fn health(&self) -> crate::HealthIndicator {
        let ready = self
            .conditions
            .iter()
            .find(|condition| condition.kind == "Ready" && condition.current);
        match ready.map(|condition| condition.status.as_str()) {
            Some("True") => crate::HealthIndicator::Healthy,
            Some("False") => crate::HealthIndicator::Warning,
            _ => crate::HealthIndicator::Unknown,
        }
    }
}
impl crate::HasHealth for BackupState {
    fn health(&self) -> crate::HealthIndicator {
        match self {
            Self::Completed => crate::HealthIndicator::Healthy,
            Self::Failed => crate::HealthIndicator::Error,
            Self::Running | Self::Pending => crate::HealthIndicator::Pending,
            Self::Unknown => crate::HealthIndicator::Unknown,
        }
    }
}
