//! What an application is, decided once for every screen (#484).
//!
//! In order of precedence an application is a Kargo Project, else an Argo CD
//! ApplicationSet or Application, else the `app.kubernetes.io/part-of` label
//! on a workload; a manual [`Override`] (rename, merge, split, hide) then has
//! the last word. [`derive`] is a pure function from what was read to the
//! applications, so it needs no cluster and no GUI to be tested. Reading is
//! [`read`]'s job, through the same bounded, read-only
//! [`Reader`](crate::delivery::Reader) as delivery's; nothing here writes.
//!
//! An unread source is never "no applications": every source carries how it
//! was read ([`Coverage`]), and [`Derived::unknown`] says which rules a
//! missing read may have left short. Kargo not being served is a fact.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::delivery::argocd::{Application as ArgoApplication, ApplicationSet, StageNaming};
use crate::delivery::kargo::{Stage, Warehouse};
use crate::delivery::source::{Source, Truncation};
use crate::workloads::WorkloadKind;

pub mod claims;
mod derive;
pub mod example;
mod overrides;
pub mod read;
mod rules;

#[cfg(test)]
mod tests;

pub use derive::derive;
pub use overrides::{Merge, Override, Split};

/// Which cluster something lives in, as the session registry (#44) names it.
/// Opaque here: today it is the one connection's id.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SessionKey(pub String);

impl SessionKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }
}

/// The rule that found an application, highest precedence first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Rule {
    Kargo,
    ArgoCd,
    PartOf,
    /// Made by the user: a split.
    Manual,
}

impl Rule {
    fn slug(self) -> &'static str {
        match self {
            Self::Kargo => "kargo",
            Self::ArgoCd => "argocd",
            Self::PartOf => "part-of",
            Self::Manual => "manual",
        }
    }
}

/// An application's stable id: the rule and what that rule found it by, such
/// as `kargo:checkout`, `argocd:applicationset/argocd/checkout`,
/// `argocd:application/argocd/checkout` or `part-of:checkout`. An Argo CD
/// object is named with its namespace, and an ApplicationSet apart from an
/// Application, so same-named ones never become one application. The id does
/// not follow the display name, so a rename keeps it. The same id found in
/// two clusters is one application, joined by name ([`Basis::Inferred`]).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ApplicationId(String);

impl ApplicationId {
    pub fn new(rule: Rule, name: &str) -> Self {
        Self(format!("{}:{name}", rule.slug()))
    }

    pub fn argo_application_set(namespace: &str, name: &str) -> Self {
        Self(format!("argocd:applicationset/{namespace}/{name}"))
    }

    pub fn argo_application(namespace: &str, name: &str) -> Self {
        Self(format!("argocd:application/{namespace}/{name}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a rule found an application from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Evidence {
    KargoProject {
        session: SessionKey,
        project: String,
    },
    ArgoApplicationSet {
        session: SessionKey,
        namespace: String,
        name: String,
        /// Whether the set itself was read; else an Application's owner
        /// reference named it.
        read: bool,
    },
    ArgoApplication {
        session: SessionKey,
        namespace: String,
        name: String,
    },
    PartOfLabel {
        value: String,
    },
    /// The user's override changed or made this application.
    Override(&'static str),
}

/// The kinds of object an application is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemberKind {
    KargoStage,
    KargoWarehouse,
    ArgoApplication,
    Workload(#[serde(with = "WorkloadKindDef")] WorkloadKind),
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "WorkloadKind")]
enum WorkloadKindDef {
    Deployment,
    StatefulSet,
    DaemonSet,
}

impl MemberKind {
    /// The order the kinds are listed in: Kargo, Argo CD, then workloads.
    pub fn rank(self) -> u8 {
        match self {
            Self::KargoStage => 0,
            Self::KargoWarehouse => 1,
            Self::ArgoApplication => 2,
            Self::Workload(WorkloadKind::Deployment) => 3,
            Self::Workload(WorkloadKind::StatefulSet) => 4,
            Self::Workload(WorkloadKind::DaemonSet) => 5,
        }
    }
}

/// Which object, in which cluster. Names are only unique within these four.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemberRef {
    pub session: SessionKey,
    pub kind: MemberKind,
    pub namespace: Option<String>,
    pub name: String,
}

/// Why an object is in the application it is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    /// The rule's own object names it: a Project's Stage, an ApplicationSet's
    /// Application, a workload's `part-of` label.
    Direct,
    /// Argo CD's inventory of an Application lists it, but nothing on the
    /// object names that Application back: a claim from one side.
    ManagedBy,
    /// Argo CD's inventory of an Application lists it, and the object's
    /// tracking annotation or instance label names that Application back.
    Tracked,
    /// An Argo CD Application that names a Kargo Stage or Project (its
    /// authorized-stage annotation, or the configured naming).
    NamesProject,
    /// Its `part-of` value is this application's name. A guess from the
    /// name alone, so shown as such.
    SameName,
    /// Joined to the application by a name found in more than one cluster.
    /// Within one cluster joining by a name is [`Basis::Direct`]; across
    /// clusters it is an inference, and a [`Note::JoinedAcrossSessions`]
    /// says which.
    Inferred,
    /// Placed by the user's override.
    Override,
}

/// Where an Argo CD Application sends what it deploys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    /// The cluster Argo CD runs in.
    Local,
    /// Another cluster, by address or by Argo CD's name for it. Which of
    /// the workspace's clusters that is waits for #44's mapping.
    Other {
        server: Option<String>,
        name: Option<String>,
    },
    /// The Application has no destination at all.
    Missing,
}

impl MemberRef {
    fn order(&self) -> (&SessionKey, u8, Option<&str>, &str) {
        (
            &self.session,
            self.kind.rank(),
            self.namespace.as_deref(),
            &self.name,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub at: MemberRef,
    pub basis: Basis,
    /// For an Argo CD Application only.
    pub destination: Option<Destination>,
    /// Lower rules that also found this object, kept so a conflict is seen
    /// and not lost.
    pub also_claimed_by: Vec<Rule>,
}

/// Something about an application that a reader should not have to find out
/// for themselves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// A member went to this application over a lower rule's claim.
    LowerClaim {
        member: MemberRef,
        rule: Rule,
        /// What the lower rule would have called it.
        name: String,
    },
    /// An Argo CD Application deploys to another cluster, not yet mapped.
    UnmappedDestination { member: MemberRef },
    /// Applications were merged into this one by the override.
    Merged { from: Vec<ApplicationId> },
    /// Kargo Stage and Warehouse reads for this Project did not answer or
    /// stopped at the page cap, so its members may be missing.
    MembersUnknown { session: SessionKey, why: String },
    /// An Argo CD Application names a Kargo Project that was not read, so it
    /// is not in that Project's application. Kargo's Projects were capped or
    /// unreadable.
    ProjectNotRead {
        member: MemberRef,
        project: String,
        why: String,
    },
    /// The same name was found in several clusters and joined into this one
    /// application, which is an inference.
    JoinedAcrossSessions {
        name: String,
        sessions: Vec<SessionKey>,
    },
    /// An Argo CD Application may manage this workload, but that cluster's
    /// Applications were not fully read, so it went by its label.
    ManagerUnknown { member: MemberRef },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub id: ApplicationId,
    pub name: String,
    pub rule: Rule,
    pub evidence: Vec<Evidence>,
    pub members: Vec<Member>,
    pub notes: Vec<Note>,
}

/// How one source was read in one cluster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoverageState {
    Read,
    /// The listing stopped at the page cap after this many items.
    Capped(usize),
    /// Read in these namespaces only, found as `found` says. Argo CD may
    /// allow Applications in others, which were not listed, so what is
    /// missing may exist.
    NamespaceOnly {
        namespaces: Vec<String>,
        found: ArgoFound,
    },
    /// Nothing marked where Argo CD runs, and its default namespace held no
    /// Applications: Applications elsewhere aren't listed, so none is known
    /// to be absent.
    NamespaceNotFound(String),
    /// The API isn't served there: a fact, not an error.
    NotInstalled(String),
    Refused(String),
    Unreadable(String),
}

impl CoverageState {
    fn of<T>(source: &Source<T>) -> Self {
        match source {
            Source::Read(_) => Self::Read,
            Source::Capped(_, Truncation { read }) => Self::Capped(*read),
            Source::NotInstalled(why) => Self::NotInstalled(why.clone()),
            Source::Refused(why) => Self::Refused(why.clone()),
            Source::Unreadable(why) => Self::Unreadable(why.clone()),
        }
    }

    /// Whether what is missing might exist: a refused, failed or capped read.
    pub fn is_unknown(&self) -> bool {
        matches!(
            self,
            Self::Capped(_)
                | Self::NamespaceOnly { .. }
                | Self::NamespaceNotFound(_)
                | Self::Refused(_)
                | Self::Unreadable(_)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceKind {
    KargoProjects,
    KargoStages,
    KargoWarehouses,
    ArgoApplications,
    ArgoApplicationSets,
    Workloads,
}

impl SourceKind {
    /// The rule whose applications this source feeds.
    pub fn rule(self) -> Rule {
        match self {
            Self::KargoProjects | Self::KargoStages | Self::KargoWarehouses => Rule::Kargo,
            Self::ArgoApplications | Self::ArgoApplicationSets => Rule::ArgoCd,
            Self::Workloads => Rule::PartOf,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coverage {
    pub session: SessionKey,
    pub source: SourceKind,
    /// For the Kargo Stage and Warehouse reads, which Project's.
    pub project: Option<String>,
    pub state: CoverageState,
}

/// A Deployment, StatefulSet or DaemonSet that carries
/// `app.kubernetes.io/part-of`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelledWorkload {
    pub namespace: String,
    pub name: String,
    pub kind: WorkloadKind,
    pub part_of: String,
    /// `argocd.argoproj.io/tracking-id`, which Argo CD writes on what it
    /// applies with annotation tracking.
    pub tracking_id: Option<String>,
    /// `app.kubernetes.io/instance`, which Argo CD writes with label
    /// tracking, its default.
    pub instance: Option<String>,
}

/// Where an Argo CD read looked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgoScope {
    /// Every namespace Applications may be in.
    AllNamespaces,
    /// The namespaces Argo CD runs in, found as `found` says. Applications
    /// in any namespace are not listed, so this read is not known to be
    /// whole.
    Namespace {
        namespaces: Vec<String>,
        found: ArgoFound,
    },
}

/// How the namespaces an Argo CD read looked in were found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgoFound {
    /// Where the workloads labelled `app.kubernetes.io/part-of=argocd` run,
    /// as Argo CD's controller, server and repo-server are. `skipped` are
    /// the namespaces past the cap, which were not read.
    Labelled { skipped: Vec<String> },
    /// Nothing was labelled so, or the workloads couldn't be read: Argo CD's
    /// default namespace.
    Default,
}

/// One Kargo Project as read: its Stages and Warehouses, each its own read.
#[derive(Clone, Debug, PartialEq)]
pub struct KargoProjectRead {
    pub name: String,
    pub stages: Source<Vec<Stage>>,
    pub warehouses: Source<Vec<Warehouse>>,
}

/// Everything read from one cluster.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionInputs {
    pub key: SessionKey,
    pub kargo: Source<Vec<KargoProjectRead>>,
    pub argo_scope: ArgoScope,
    pub argo_applications: Source<Vec<ArgoApplication>>,
    pub argo_application_sets: Source<Vec<ApplicationSet>>,
    pub workloads: Source<Vec<LabelledWorkload>>,
}

impl SessionInputs {
    /// A session none of whose sources were read.
    pub fn unread(key: SessionKey, why: &str) -> Self {
        fn unreadable<T>(why: &str) -> Source<T> {
            Source::Unreadable(why.to_owned())
        }
        Self {
            key,
            kargo: unreadable(why),
            argo_scope: ArgoScope::AllNamespaces,
            argo_applications: unreadable(why),
            argo_application_sets: unreadable(why),
            workloads: unreadable(why),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inputs {
    pub sessions: Vec<SessionInputs>,
    /// How an Argo CD Application names a Kargo Project besides the
    /// authorized-stage annotation, where the setup configured it.
    pub stage_naming: Option<StageNaming>,
}

/// The override's wishes that matched nothing, kept for the user to see
/// rather than dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unmatched {
    Rename(ApplicationId),
    Hide(ApplicationId),
    MergeInto(ApplicationId),
    MergeFrom(ApplicationId),
    Split(MemberRef),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Derived {
    /// Sorted by name, then id.
    pub applications: Vec<Application>,
    /// Hidden by the override, so they can be shown again.
    pub hidden: Vec<Application>,
    pub coverage: Vec<Coverage>,
    pub unmatched: Vec<Unmatched>,
}

impl Derived {
    /// The rules some read may have left short in some cluster, with the
    /// cluster. Applications may be missing for these; they are not known to
    /// be absent. A source that isn't installed is not here.
    pub fn unknown(&self) -> BTreeMap<Rule, Vec<SessionKey>> {
        let mut unknown: BTreeMap<Rule, Vec<SessionKey>> = BTreeMap::new();
        for coverage in self.coverage.iter().filter(|c| c.state.is_unknown()) {
            let sessions = unknown.entry(coverage.source.rule()).or_default();
            if !sessions.contains(&coverage.session) {
                sessions.push(coverage.session.clone());
            }
        }
        unknown
    }

    pub fn find(&self, id: &ApplicationId) -> Option<&Application> {
        self.applications.iter().find(|app| &app.id == id)
    }
}
