//! Where each part of the delivery chain is read: which workspace entry
//! holds Kargo, the builds and Argo CD. The entries' roles choose, so a
//! person who splits them over several clusters needs to say nothing more:
//!
//! - Kargo and the builds come from the first `cicd` entry, else the first
//!   `core` entry;
//! - Argo CD comes from the first `core` entry.
//!
//! The first entry listed wins a tie, so reordering the entries changes the
//! choice. `workspace.json`'s optional `sources` object pins a part to an
//! entry by id (`{"kargo": "cluster-b"}`); a pin to an entry the workspace
//! no longer lists is kept and says so, never falling back to a role. The
//! implicit workspace of one, with no entries, reads every part from the
//! open connection. Nothing is guessed: no part comes from a kubeconfig's
//! current context or from matching servers.
//!
//! The workloads aren't one part: each Argo CD destination's mapped entry
//! (`destinations`) holds its own ([`Sources::workloads`]).

use serde::{Deserialize, Serialize};

use super::{Entry, Invalid, Role, Workspace};

/// A part of the delivery chain that is read from one entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Part {
    /// Projects, Warehouses, Freight, Stages and Promotions.
    Kargo,
    /// Pipelines as Code's PipelineRuns and TaskRuns.
    Builds,
    /// Applications, ApplicationSets and `argocd-cm`.
    ArgoCd,
}

impl Part {
    pub const ALL: [Part; 3] = [Part::Kargo, Part::Builds, Part::ArgoCd];

    /// Its key in `sources`.
    pub fn key(self) -> &'static str {
        match self {
            Part::Kargo => "kargo",
            Part::Builds => "builds",
            Part::ArgoCd => "argocd",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Part::Kargo => "Kargo",
            Part::Builds => "Builds",
            Part::ArgoCd => "Argo CD",
        }
    }

    /// What is read for it.
    pub fn reads(self) -> &'static str {
        match self {
            Part::Kargo => "Projects, Warehouses, Freight, Stages, Promotions",
            Part::Builds => "PipelineRuns and TaskRuns from Pipelines as Code",
            Part::ArgoCd => "Applications, ApplicationSets, argocd-cm",
        }
    }

    /// Select once in input order, preserving role precedence over file order.
    pub(crate) fn role_index<T>(
        self,
        rows: &[T],
        role_of: impl Fn(&T) -> Option<Role>,
    ) -> Option<usize> {
        self.roles()
            .iter()
            .find_map(|role| rows.iter().position(|row| role_of(row) == Some(*role)))
    }

    /// The roles that hold it, in the order they are tried.
    pub fn roles(self) -> &'static [Role] {
        match self {
            Part::Kargo | Part::Builds => &[Role::Cicd, Role::Core],
            Part::ArgoCd => &[Role::Core],
        }
    }
}

/// Recovery information that affects source availability, kept beside Setup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceRecovery {
    pub all_clusters_skipped: bool,
    skipped_roles: Vec<(Part, SkippedRow)>,
}

impl SourceRecovery {
    pub(crate) fn new(all_clusters_skipped: bool) -> Self {
        Self {
            all_clusters_skipped,
            ..Self::default()
        }
    }

    pub(crate) fn skip_role(&mut self, part: Part, row: SkippedRow) {
        self.skipped_roles.push((part, row));
    }
}

/// A role's original input row, whose configuration could not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedRow {
    pub index: usize,
    pub id: Option<String>,
}

impl SkippedRow {
    pub fn label(&self) -> String {
        let id = self
            .id
            .as_ref()
            .map(|id| format!(" ({id})"))
            .unwrap_or_default();
        format!("clusters[{}]{id}", self.index)
    }
}

/// Why an entry was chosen for a part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// The first entry with this role.
    Role(Role),
    /// `sources` pins the part to it.
    Pinned,
    /// An Argo CD destination maps to it, for the workloads.
    Destination,
}

impl Why {
    /// `CI/CD role`, `pinned`.
    pub fn words(self) -> String {
        match self {
            Why::Role(role) => format!("{} role", role.label()),
            Why::Pinned => "pinned in workspace.json".to_owned(),
            Why::Destination => "an Argo CD destination".to_owned(),
        }
    }
}

/// Where one part is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chosen {
    /// From this entry.
    Entry { entry: Entry, why: Why },
    /// The implicit workspace of one: from the open connection.
    Open,
    /// No entry has a role that holds the part.
    NoEntry(&'static [Role]),
    /// `sources` pins the part to an entry the workspace no longer lists.
    Gone(String),
    /// The role selected this input row; recovery must not select another.
    Skipped(SkippedRow),
}

impl Chosen {
    /// The entry it is read from, when there is one.
    pub fn entry(&self) -> Option<&Entry> {
        match self {
            Chosen::Entry { entry, .. } => Some(entry),
            _ => None,
        }
    }

    /// Why nothing can be read, for a part with no entry: "No CI/CD or Core
    /// entry in the workspace", "cluster-b is no longer in the workspace".
    pub fn missing(&self) -> Option<String> {
        match self {
            Chosen::NoEntry(roles) => {
                let roles: Vec<&str> = roles.iter().map(|role| role.label()).collect();
                Some(format!("No {} entry in the workspace", roles.join(" or ")))
            }
            Chosen::Gone(id) => Some(format!("{id} is no longer in the workspace")),
            Chosen::Skipped(row) => Some(format!("{} was skipped in freshkube.yaml", row.label())),
            Chosen::Entry { .. } | Chosen::Open => None,
        }
    }
}

/// Where each part is read, resolved from a workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sources {
    kargo: Chosen,
    builds: Chosen,
    argocd: Chosen,
    workloads: Vec<Chosen>,
}

/// The workspace of one's: everything from the open connection.
impl Default for Sources {
    fn default() -> Self {
        Self::of(&Workspace::default())
    }
}

impl Sources {
    pub fn of(workspace: &Workspace) -> Self {
        Self::of_recovered(workspace, &SourceRecovery::default())
    }

    /// A configured list whose rows were all skipped is unavailable, even
    /// though its effective cluster list is empty.
    pub fn of_recovered(workspace: &Workspace, recovery: &SourceRecovery) -> Self {
        let choose = |part| choose_recovered(workspace, part, recovery);
        Self {
            kargo: choose(Part::Kargo),
            builds: choose(Part::Builds),
            argocd: choose(Part::ArgoCd),
            workloads: workloads(workspace, recovery.all_clusters_skipped),
        }
    }

    /// Where the workloads are read: each entry an Argo CD destination maps
    /// to, once, in the destinations' order, an entry the workspace no
    /// longer lists as [`Chosen::Gone`]; the open connection for the
    /// workspace of one. Empty when no destination is mapped.
    pub fn workloads(&self) -> &[Chosen] {
        &self.workloads
    }

    pub fn get(&self, part: Part) -> &Chosen {
        match part {
            Part::Kargo => &self.kargo,
            Part::Builds => &self.builds,
            Part::ArgoCd => &self.argocd,
        }
    }

    /// The entry `id`, when a part or a destination is read from it.
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        Part::ALL
            .into_iter()
            .map(|part| self.get(part))
            .chain(&self.workloads)
            .filter_map(Chosen::entry)
            .find(|entry| entry.id == id)
    }

    /// Every part and where it is read, in [`Part::ALL`]'s order.
    pub fn iter(&self) -> impl Iterator<Item = (Part, &Chosen)> {
        Part::ALL.into_iter().map(|part| (part, self.get(part)))
    }
}

fn workloads(workspace: &Workspace, all_clusters_skipped: bool) -> Vec<Chosen> {
    if workspace.clusters.is_empty() && !all_clusters_skipped {
        return vec![Chosen::Open];
    }
    let mut chosen: Vec<Chosen> = Vec::new();
    for destination in &workspace.destinations {
        let id = destination.entry.trim();
        let seen = chosen.iter().any(|chosen| match chosen {
            Chosen::Entry { entry, .. } => entry.id == id,
            Chosen::Gone(gone) => gone == id,
            _ => false,
        });
        if id.is_empty() || seen {
            continue;
        }
        chosen.push(
            match workspace.clusters.iter().find(|entry| entry.id == id) {
                Some(entry) => Chosen::Entry {
                    entry: entry.clone(),
                    why: Why::Destination,
                },
                None => Chosen::Gone(id.to_owned()),
            },
        );
    }
    chosen
}

pub(super) fn choose(workspace: &Workspace, part: Part) -> Chosen {
    choose_recovered(workspace, part, &SourceRecovery::default())
}

fn choose_recovered(workspace: &Workspace, part: Part, recovery: &SourceRecovery) -> Chosen {
    if workspace.clusters.is_empty() && !recovery.all_clusters_skipped {
        return Chosen::Open;
    }
    if let Some(pinned) = workspace.sources.pin(part) {
        return match workspace.clusters.iter().find(|entry| entry.id == pinned) {
            Some(entry) => Chosen::Entry {
                entry: entry.clone(),
                why: Why::Pinned,
            },
            None => Chosen::Gone(pinned.to_owned()),
        };
    }
    if let Some((_, row)) = recovery
        .skipped_roles
        .iter()
        .find(|(selected, _)| *selected == part)
    {
        return Chosen::Skipped(row.clone());
    }
    part.role_index(&workspace.clusters, |entry| Some(entry.role))
        .map(|index| {
            let entry = &workspace.clusters[index];
            Chosen::Entry {
                entry: entry.clone(),
                why: Why::Role(entry.role),
            }
        })
        .unwrap_or(Chosen::NoEntry(part.roles()))
}

/// `workspace.json`'s `sources`: a part pinned to an entry by id. Every
/// key is optional; keys this version doesn't know are written back. Boxed,
/// since most workspaces pin nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Pins(Box<Pinned>);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Pinned {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kargo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    builds: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    argocd: Option<String>,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

impl Pins {
    /// The entry `part` is pinned to.
    pub fn pin(&self, part: Part) -> Option<&str> {
        match part {
            Part::Kargo => self.0.kargo.as_deref(),
            Part::Builds => self.0.builds.as_deref(),
            Part::ArgoCd => self.0.argocd.as_deref(),
        }
    }

    /// Pins `part` to `entry`, or unpins it.
    pub fn set(&mut self, part: Part, entry: Option<String>) {
        let slot = match part {
            Part::Kargo => &mut self.0.kargo,
            Part::Builds => &mut self.0.builds,
            Part::ArgoCd => &mut self.0.argocd,
        };
        *slot = entry;
    }

    pub fn is_empty(&self) -> bool {
        Part::ALL.iter().all(|&part| self.pin(part).is_none()) && self.0.extra.is_empty()
    }

    /// Keys this version doesn't know.
    pub(super) fn unknown_keys(&self) -> impl Iterator<Item = &String> {
        self.0.extra.keys()
    }

    pub(super) fn validate(&self) -> Result<(), Invalid> {
        for part in Part::ALL {
            if self.pin(part).is_some_and(|id| id.trim().is_empty()) {
                return Err(Invalid::EmptySource(part.key()));
            }
        }
        Ok(())
    }
}
