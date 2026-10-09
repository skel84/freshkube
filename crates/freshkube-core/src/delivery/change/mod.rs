//! One change as the change page shows it (docs/DESIGN.md, "The change
//! page"): its hops in the order the change travels, grouped by phase and
//! by Stage, each with its state, how it joins the hop before, and what the
//! Inspector says of it. A Stage keeps its three gates apart: whether the
//! Freight is eligible, how it is promoted, and the verification after.
//!
//! This is the page's model, without presentation: states are
//! [`HealthIndicator`]s and times are instants, which the page words. It
//! reads nothing; [`example`] invents one change for example mode.

pub mod example;
pub mod live;

use chrono::{DateTime, Utc};

use super::address::Address;
use super::join::Confidence;
use crate::indicators::HealthIndicator;

/// One change: a Freight of a Kargo project, and every hop it took.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    /// The Kargo project, the page's breadcrumb.
    pub project: String,
    /// The Freight, the page's title.
    pub freight: String,
    /// The cluster Kargo runs in.
    pub kargo_cluster: String,
    /// When it was all read.
    pub observed_at: DateTime<Utc>,
    /// The Freight's page in Kargo, which the header opens and copies; why
    /// not, when the cluster doesn't record Kargo's address.
    pub page: Result<Address, String>,
    pub groups: Vec<Group>,
    pub stages: Vec<Stage>,
    pub hops: Vec<Hop>,
}

impl Change {
    /// The hops of a group, in travel order.
    pub fn hops_of(&self, group: usize) -> impl Iterator<Item = &Hop> {
        self.hops.iter().filter(move |hop| hop.group == group)
    }

    pub fn hop(&self, key: &str) -> Option<&Hop> {
        self.hops.iter().find(|hop| hop.key == key)
    }
}

/// A group of the trail: a phase, or a Stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The pull request, its pipeline runs, the supply chain and the image.
    Build,
    Freight,
    /// A Stage, by index into [`Change::stages`].
    Stage(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub phase: Phase,
    /// Where its hops were read, or where the Stage deploys.
    pub detail: String,
}

/// One row of the trail.
#[derive(Clone, Debug, PartialEq)]
pub struct Hop {
    /// Unique in the change; selects it.
    pub key: String,
    /// Its group, by index into [`Change::groups`].
    pub group: usize,
    pub state: HealthIndicator,
    pub name: String,
    pub detail: String,
    /// The cluster or service it was read from.
    pub from: String,
    pub at: Option<DateTime<Utc>>,
    /// How sure its link to the hop before is. Gates have none.
    pub link: Option<Confidence>,
    pub shows: Shows,
}

/// What the Inspector shows for a row.
#[derive(Clone, Debug, PartialEq)]
pub enum Shows {
    Hop(Box<HopDetail>),
    /// A gate row shows its Stage's three gates, by index.
    Stage(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct HopDetail {
    /// Its kind, as the heading's caption.
    pub kind: String,
    /// Its name, in monospace.
    pub title: String,
    /// Its state in words.
    pub state: String,
    pub notice: Option<Notice>,
    pub fields: Vec<Field>,
    pub link: Option<LinkDetail>,
    /// Said in the link's place for a hop that has none.
    pub unlinked: Option<String>,
    pub actions: Vec<Action>,
}

/// A banner under a hop's heading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub state: HealthIndicator,
    pub lead: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub label: String,
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    /// A name, digest or commit, in monospace.
    Mono(String),
    /// An instant, which the page words in local time.
    At(DateTime<Utc>),
}

/// How a hop joins the hop before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkDetail {
    pub confidence: Confidence,
    /// What joins them: `commit`, `digest`, `revision`, `provenance`.
    pub by: String,
    /// The hop before, by name.
    pub before: String,
    pub before_says: String,
    pub here_says: String,
    /// Why the link has its confidence.
    pub why: String,
}

/// An object to open in Resources, on the cluster that holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
    pub cluster: String,
    pub group: String,
    pub version: String,
    pub kind: String,
    /// The kind's plural, as the API serves it.
    pub plural: String,
    pub namespace: String,
    pub name: String,
}

/// Where an action leads. None changes anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// An object in Resources; `what` names it on the button.
    Resource { what: String, object: Object },
    /// The tool's own page, in the browser: `label` without its arrow.
    /// `None` when the cluster doesn't record where it is; the action says
    /// why.
    Browser {
        label: String,
        address: Option<Address>,
    },
    /// An object on another workspace cluster, which only Argo CD on the
    /// open one reports: `entry` is that cluster's workspace id, never a
    /// connection, and `object.cluster` is left empty.
    OnEntry {
        what: String,
        entry: String,
        object: Object,
    },
    /// The steps' logs of the pod a TaskRun ran in, in the dock, from the
    /// cluster that ran it: `what` names the task on the button, and
    /// `container` is its first step's.
    Logs {
        what: String,
        pod: Object,
        container: Option<String>,
    },
    /// The revision Coroot keeps of a Deployment, in Observability: the one
    /// its current ReplicaSet's pod-template hash names, as Coroot names a
    /// revision by that hash. `None` when the hash isn't known; the action
    /// says why.
    Revision {
        deployment: Object,
        hash: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub target: Target,
    /// Why it can't be pressed, when it can't.
    pub disabled: Option<String>,
}

impl Action {
    pub fn new(target: Target) -> Self {
        Self {
            target,
            disabled: None,
        }
    }

    pub fn disabled(mut self, why: impl Into<String>) -> Self {
        self.disabled = Some(why.into());
        self
    }

    /// Coroot's revision of `deployment` that `hash` names, or greyed out
    /// with why the hash isn't known.
    pub fn revision(deployment: Object, hash: Result<String, String>) -> Self {
        match hash {
            Ok(hash) => Self::new(Target::Revision {
                deployment,
                hash: Some(hash),
            }),
            Err(why) => Self::new(Target::Revision {
                deployment,
                hash: None,
            })
            .disabled(why),
        }
    }

    /// A tool's page at `address`, or greyed out with why there's none.
    pub fn browser(label: impl Into<String>, address: Result<Address, String>) -> Self {
        let label = label.into();
        match address {
            Ok(address) => Self::new(Target::Browser {
                label,
                address: Some(address),
            }),
            Err(why) => Self::new(Target::Browser {
                label,
                address: None,
            })
            .disabled(why),
        }
    }
}

/// One check of a gate: a promotion step or an AnalysisRun.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub state: HealthIndicator,
    pub name: String,
    pub found: String,
    pub at: Option<DateTime<Utc>>,
}

/// How the Freight became eligible for a Stage. Times are those Kargo
/// recorded, when it recorded one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Eligible {
    /// It passed verification in the upstream Stage.
    Verified {
        upstream: String,
        at: Option<DateTime<Utc>>,
        /// What passed there: `3 of 3 analyses`, when it was read.
        checks: Option<String>,
    },
    /// Someone approved it for this Stage by hand, past its upstream.
    Approved {
        /// Who, when it was read; Kargo's record keeps no name.
        by: Option<String>,
        at: Option<DateTime<Utc>>,
        past: String,
        /// Whether the upstream has verified it since.
        upstream: Upstream,
    },
    /// The first Stage takes Freight from the Warehouse.
    Warehouse { at: Option<DateTime<Utc>> },
    /// Neither verified upstream nor approved for this Stage yet.
    NotYet { upstream: Vec<String> },
    /// What would say couldn't be read.
    Unknown { why: String },
}

/// Whether the Stage an approval went past has verified the Freight since.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Upstream {
    NotVerified,
    /// It has, at the time Kargo recorded, when it recorded one.
    Verified {
        at: Option<DateTime<Utc>>,
    },
}

/// How the Freight is promoted to a Stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Promotion {
    /// Kargo promoted it: auto-promotion is on.
    Automatic { at: Option<DateTime<Utc>> },
    /// A user promoted it.
    ByHand { at: Option<DateTime<Utc>> },
    /// Something else promoted it, in Kargo's words for its creator.
    Other {
        how: String,
        at: Option<DateTime<Utc>>,
    },
    /// Auto-promotion is off: it waits for someone who may promote.
    Waiting,
    /// No Promotion of the Freight to the Stage was read.
    NotYet,
    /// A Promotion is under way, in Kargo's word for its phase.
    Running { phase: String },
    /// The Promotion ended without promoting.
    Failed {
        phase: String,
        message: Option<String>,
    },
    /// What would say couldn't be read.
    Unknown { why: String },
}

/// Where a Stage deploys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    /// The environment cluster, by its name.
    Cluster(String),
    /// Another workspace cluster, which the person mapped the Application's
    /// destination to. It isn't read.
    Entry {
        /// The workspace entry's id.
        entry: String,
        /// What was mapped: `server https://…` or `name prod-lon`.
        via: String,
    },
    /// Not known, and why: its Application wasn't found, or its
    /// destination wasn't matched to a cluster.
    Unknown(String),
}

impl Destination {
    /// The cluster's name, when it is known.
    pub fn cluster(&self) -> Option<&str> {
        match self {
            Self::Cluster(name) | Self::Entry { entry: name, .. } => Some(name),
            Self::Unknown(_) => None,
        }
    }

    /// The Stage's group row: `deploys to prod-ams`, or why it isn't known.
    pub fn words(&self) -> String {
        match self {
            Self::Cluster(name) => format!("deploys to {name}"),
            Self::Entry { entry, .. } => format!("deploys to {entry} (mapped)"),
            Self::Unknown(why) => format!("destination unknown: {why}"),
        }
    }
}

/// A Stage and its three gates.
#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub name: String,
    /// Where it deploys, as far as what was read says.
    pub cluster: Destination,
    pub state: HealthIndicator,
    /// Its state in words: `Verified`, `Waiting for promotion`.
    pub words: String,
    /// What runs there now, when it isn't this change.
    pub running: Option<String>,
    pub eligible: Eligible,
    pub promotion: Promotion,
    /// Who may approve Freight for the Stage, past its upstream, when it
    /// was read.
    pub approvers: Option<String>,
    /// Who may promote Freight to it, when it was read.
    pub promoters: Option<String>,
    /// The promotion's own steps; empty when they weren't read.
    pub steps: Vec<Check>,
    /// Each AnalysisRun after the promotion.
    pub verification: Vec<Check>,
    /// How the Stage's record joins the Freight, when it runs it and the
    /// join read it.
    pub link: Option<LinkDetail>,
    /// The Stage itself, to open in Resources.
    pub object: Object,
}
