use chrono::{DateTime, Utc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Engine {
    CloudNativePg,
    MariaDb,
}
impl Engine {
    /// The operator, as the Sources group and the Engine tooltip name it.
    pub fn label(self) -> &'static str {
        match self {
            Self::CloudNativePg => "PostgreSQL · CloudNativePG",
            Self::MariaDb => "MariaDB · MariaDB operator",
        }
    }
    /// The database itself, as the Engine column starts its words.
    pub fn product(self) -> &'static str {
        match self {
            Self::CloudNativePg => "PostgreSQL",
            Self::MariaDb => "MariaDB",
        }
    }
}

/// Unknown is distinct from a reported zero, false or empty collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// Whether the operator reports the instance ready.
    pub ready: Fact<bool>,
    /// The Kubernetes node it runs on, when the operator names it.
    pub node: Fact<String>,
    /// Replication lag of a replica, with the source that measured it.
    pub lag: Fact<Lag>,
}
impl Instance {
    /// An instance of which only the name is known.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            role: Fact::Unknown,
            state: Fact::Unknown,
            replication: Fact::Unknown,
            ready: Fact::Unknown,
            node: Fact::Unknown,
            lag: Fact::Unknown,
        }
    }
}

/// Where a measurement came from. Prometheus is not read in this slice, so a
/// lag only Prometheus could give stays Unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LagSource {
    Coroot,
    /// The MariaDB operator's own replica status (`secondsBehindMaster`).
    Operator,
}
impl LagSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Coroot => "Coroot",
            Self::Operator => "the operator",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lag {
    pub seconds: f64,
    pub source: LagSource,
}
/// Above this a replica's lag is a warning.
/// Lag is only ever a finite, non-negative number of seconds.
impl Eq for Lag {}
pub const LAG_WARNING_SECONDS: f64 = 30.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyKind {
    /// CloudNativePG's Pooler (PgBouncer).
    Pooler,
    MaxScale,
}
impl ProxyKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pooler => "Pooler",
            Self::MaxScale => "MaxScale",
        }
    }
}
/// The proxy in front of the instances, as its own custom resource counts
/// its pods: `status.instances` over `spec.instances` for a Pooler (which
/// reports pods scheduled, not ready) and `status.replicas` over
/// `spec.replicas` for MaxScale, whose Ready condition says the rest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proxy {
    pub kind: ProxyKind,
    pub running: u32,
    pub desired: u32,
    /// MaxScale's Ready condition; a Pooler has none.
    pub ready: Fact<bool>,
}

/// A certificate the operator reports on its status. Only the secret's name
/// and the expiry are kept; no Secret is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Certificate {
    pub name: String,
    pub expires: Option<DateTime<Utc>>,
}
/// A certificate warns this long before it expires.
pub const CERTIFICATE_WARNING_DAYS: i64 = 14;
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
    /// The database server's version, as the operator's image names it.
    pub version: Fact<String>,
    /// Pooler or MaxScale readiness, from the proxy's own status.
    /// Known(None) when the read established there is none.
    pub proxy: Fact<Option<Proxy>>,
    /// From the Cluster's status; MariaDB's are Unknown.
    pub certificates: Fact<Vec<Certificate>>,
    /// When the operator last changed the primary or started to: the
    /// transition time of the condition that reports it.
    pub primary_changed: Option<DateTime<Utc>>,
}
impl Database {
    /// "PostgreSQL 16", or just the product when the version isn't reported.
    pub fn engine_label(&self) -> String {
        match self.version.known() {
            Some(version) => format!("{} {version}", self.engine.product()),
            None => self.engine.product().to_owned(),
        }
    }
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
    /// Complete when any operator's API was found and read; otherwise why not.
    pub discovery: Coverage,
    /// What discovery found of each operator's API.
    pub engines: Vec<(Engine, Coverage)>,
    pub databases: Read<Database>,
    pub backups: Read<Backup>,
    pub schedules: Read<Schedule>,
}
impl Inventory {
    pub fn unavailable(coverage: Coverage, at: DateTime<Utc>) -> Self {
        Self {
            engines: Vec::new(),
            discovery: coverage.clone(),
            databases: Read::empty(coverage.clone(), at),
            backups: Read::empty(coverage.clone(), at),
            schedules: Read::empty(coverage, at),
        }
    }
    /// One inventory from each operator's own. An operator whose API is not
    /// served is simply not installed and leaves the others' coverage alone;
    /// one whose discovery failed makes every kind's coverage that failure.
    pub fn merge(parts: Vec<(Engine, Inventory)>, at: DateTime<Utc>) -> Self {
        let engines: Vec<_> = parts
            .iter()
            .map(|(engine, inventory)| (*engine, inventory.discovery.clone()))
            .collect();
        let considered: Vec<&Inventory> = parts
            .iter()
            .map(|(_, inventory)| inventory)
            .filter(|inventory| inventory.discovery != Coverage::NotServed)
            .collect();
        if considered.is_empty() {
            return Self {
                engines,
                ..Self::unavailable(Coverage::NotServed, at)
            };
        }
        let discovery = if considered.iter().any(|i| i.discovery == Coverage::Complete) {
            Coverage::Complete
        } else {
            considered[0].discovery.clone()
        };
        fn join<T: Clone>(
            considered: &[&Inventory],
            kind: fn(&Inventory) -> &Read<T>,
            at: DateTime<Utc>,
        ) -> Read<T> {
            let mut out = Read::empty(Coverage::NotServed, at);
            let mut coverages = Vec::new();
            for inventory in considered {
                let read = kind(inventory);
                // A kind the API doesn't serve says nothing of the others.
                if read.coverage != Coverage::NotServed || inventory.discovery != Coverage::Complete
                {
                    coverages.push(read.coverage.clone());
                }
                out.items.extend(read.items.iter().cloned());
            }
            out.coverage = match coverages.iter().find(|c| **c != Coverage::Complete) {
                Some(bad) => bad.clone(),
                None if coverages.is_empty() => Coverage::NotServed,
                None => Coverage::Complete,
            };
            out
        }
        Self {
            engines,
            discovery,
            databases: join(&considered, |i| &i.databases, at),
            backups: join(&considered, |i| &i.backups, at),
            schedules: join(&considered, |i| &i.schedules, at),
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
