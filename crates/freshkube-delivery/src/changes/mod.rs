//! The Changes list (#564): every Freight of each Kargo project read, as a
//! change, with how sure it is that each Stage's pods run it, in promotion
//! order.
//!
//! A row is one Freight: its pull request when a build of its commit names
//! one, else its commit, with the commit's title, its project and its age.
//! Each Stage's cell is the [`Reach`] the change page's model gives it, so
//! the confidence is the join's ([`change::live`](super::change::live)); a
//! Stage that doesn't run the Freight is not reached. A row's own state is
//! its weakest reached Stage's.
//!
//! [`derive`] reads nothing: [`live`] reads one cluster, and [`example`]
//! invents acme's. A source that couldn't be read is a [`Note`], never an
//! empty list.

pub mod example;
pub mod live;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use super::change::{self, Change, Reach};
use super::join::Confidence;
use super::kargo::{Freight, Stage, promotion_order};
use super::source::Source;

/// The most Freight a project lists, newest first. Those a Stage runs are
/// always listed, and the rest are counted in a note.
pub const MAX_FREIGHT: usize = 50;

/// One Kargo project, as the list reads it.
#[derive(Clone, Debug)]
pub struct Project {
    pub name: String,
    pub freight: Source<Vec<Freight>>,
    pub stages: Source<Vec<Stage>>,
    /// The change of each Freight a Stage runs, by the Freight's name, as
    /// the change page reads it, or why it couldn't be read.
    pub followed: BTreeMap<String, Result<Arc<Change>, String>>,
}

/// What the list reads: the projects of the cluster that runs Kargo.
#[derive(Clone, Debug)]
pub struct Read {
    /// That cluster's name, which the notes name.
    pub cluster: String,
    pub projects: Source<Vec<Project>>,
    /// How many changes weren't followed before the read's budget ran
    /// out; their Stages show Unknown.
    pub late: usize,
    /// Why Kargo's entry wasn't read at all, in its words: no entry holds
    /// Kargo, or the one that does couldn't be reached.
    pub unreached: Option<String>,
}

/// What a row names the change by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Named {
    PullRequest(u64),
    Commit(String),
    /// The Freight names no commit.
    Nothing,
}

/// The list's groups, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Fold {
    /// A Stage runs it, and at least one isn't confirmed to its pods.
    NotConfirmed,
    /// Every Stage that runs it is confirmed to its running pods.
    Confirmed,
    /// No Stage runs it now: it waits for its first promotion, or later
    /// Freight replaced it everywhere.
    NotRunning,
}

impl Fold {
    pub const ALL: [Fold; 3] = [Fold::NotConfirmed, Fold::Confirmed, Fold::NotRunning];

    pub fn label(self) -> &'static str {
        match self {
            Self::NotConfirmed => "Not fully confirmed",
            Self::Confirmed => "Confirmed to the running pods",
            Self::NotRunning => "Not running in any Stage",
        }
    }

    /// Whether the group folds while anything needs a look.
    pub fn folds(self) -> bool {
        self != Self::NotConfirmed
    }
}

/// One change.
#[derive(Clone, Debug)]
pub struct Row {
    /// `project/freight`, unique in the list; selects it.
    pub key: String,
    /// The Kargo project: the application.
    pub project: String,
    /// The Freight's name.
    pub freight: String,
    pub alias: Option<String>,
    pub named: Named,
    pub title: Option<String>,
    /// When the Warehouse made the Freight.
    pub created: Option<DateTime<Utc>>,
    /// Its project's Stages in promotion order, each with how sure it is
    /// that the Stage's pods run it; none when the Stages weren't read.
    pub stages: Vec<StageReach>,
    /// Its weakest reached Stage; not reached when none runs it.
    pub state: Reach,
    pub fold: Fold,
    /// Its change, which the Inspector shows as a trail, when a Stage runs
    /// it; else why there's none.
    pub trail: Result<Arc<Change>, String>,
}

/// One Stage of a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageReach {
    pub name: String,
    pub reach: Reach,
}

/// Why something may be missing from the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gap {
    /// Kargo, or a kind of it, isn't served.
    NotServed,
    Refused,
    /// Unreachable, timed out, or another error.
    Unreadable,
    /// The list stopped at a cap.
    Capped,
}

/// A source the list couldn't read in full, in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub gap: Gap,
    /// The project it is about, when it is about one.
    pub project: Option<String>,
    pub text: String,
}

/// The list: its rows in order, and what may be missing.
#[derive(Clone, Debug, Default)]
pub struct Changes {
    /// By group, then newest first.
    pub rows: Vec<Row>,
    pub notes: Vec<Note>,
}

impl Changes {
    /// Whether anything needs a look, which folds the other groups.
    pub fn needs_a_look(&self) -> bool {
        self.rows.iter().any(|row| row.fold == Fold::NotConfirmed)
    }
}

/// The list from what was read.
pub fn derive(read: &Read) -> Changes {
    let mut changes = Changes::default();
    if let Some(why) = &read.unreached {
        changes.notes.push(Note {
            gap: Gap::Unreadable,
            project: None,
            text: why.clone(),
        });
        return changes;
    }
    let projects = match &read.projects {
        Source::Read(projects) => projects,
        Source::Capped(projects, truncation) => {
            changes.notes.push(Note {
                gap: Gap::Capped,
                project: None,
                text: format!(
                    "Only {} Kargo projects on {} were read; the others aren't listed",
                    truncation.read, read.cluster
                ),
            });
            projects
        }
        other => {
            changes
                .notes
                .extend(unread(other, None, "Kargo projects", &read.cluster));
            return changes;
        }
    };
    if read.late > 0 {
        changes.notes.push(Note {
            gap: Gap::Unreadable,
            project: None,
            text: format!(
                "{} {} on {} weren't followed in the time a read may take, so {} Stages show \
                 Unknown",
                read.late,
                if read.late == 1 { "change" } else { "changes" },
                read.cluster,
                if read.late == 1 { "its" } else { "their" },
            ),
        });
    }
    let mut projects: Vec<&Project> = projects.iter().collect();
    projects.sort_by(|a, b| a.name.cmp(&b.name));
    for project in projects {
        rows(project, &read.cluster, &mut changes);
    }
    changes.rows.sort_by(|a, b| {
        a.fold
            .cmp(&b.fold)
            .then(b.created.cmp(&a.created))
            .then(a.key.cmp(&b.key))
    });
    changes.notes.extend(followed_missing(&changes.rows));
    changes
}

/// What the followed changes may be missing, each sentence once: an entry
/// or a destination that couldn't be read, a kind refused or not served.
fn followed_missing(rows: &[Row]) -> Vec<Note> {
    let mut seen = BTreeSet::new();
    rows.iter()
        .filter_map(|row| row.trail.as_ref().ok())
        .flat_map(|change| &change.missing)
        .filter(|line| seen.insert(line.text.as_str()))
        .map(|line| Note {
            gap: match line.gap {
                change::Gap::NotServed => Gap::NotServed,
                change::Gap::Refused => Gap::Refused,
                change::Gap::Unreadable => Gap::Unreadable,
            },
            project: None,
            text: line.text.clone(),
        })
        .collect()
}

/// A source that wasn't read, as a note; none for one that was.
fn unread<T>(source: &Source<T>, project: Option<&str>, what: &str, cluster: &str) -> Option<Note> {
    let (gap, text) = match source {
        Source::NotInstalled(_) => (Gap::NotServed, format!("{what} aren't served on {cluster}")),
        Source::Refused(why) => (
            Gap::Refused,
            format!("{what} were refused on {cluster}: {why}"),
        ),
        Source::Unreadable(why) => (
            Gap::Unreadable,
            format!("{what} couldn't be read on {cluster}: {why}"),
        ),
        Source::Read(_) | Source::Capped(..) => return None,
    };
    Some(Note {
        gap,
        project: project.map(str::to_owned),
        text,
    })
}

/// A project's rows, and its notes.
fn rows(project: &Project, cluster: &str, changes: &mut Changes) {
    let name = &project.name;
    let freight = match &project.freight {
        Source::Read(freight) => freight,
        Source::Capped(freight, truncation) => {
            changes.notes.push(Note {
                gap: Gap::Capped,
                project: Some(name.clone()),
                text: format!(
                    "{name}'s Freight list stopped at the page cap after {}; older Freight may not be listed",
                    truncation.read
                ),
            });
            freight
        }
        other => {
            let what = format!("{name}'s Freight");
            changes
                .notes
                .extend(unread(other, Some(name), &what, cluster));
            return;
        }
    };
    let stages = project.stages.read();
    match &project.stages {
        Source::Capped(_, truncation) => changes.notes.push(Note {
            gap: Gap::Capped,
            project: Some(name.clone()),
            text: format!(
                "{name}'s Stage list stopped at the page cap after {}; a Stage not read may run its Freight",
                truncation.read
            ),
        }),
        other => {
            let what = format!("{name}'s Stages");
            changes
                .notes
                .extend(unread(other, Some(name), &what, cluster));
        }
    }
    let running: BTreeSet<&str> = stages
        .into_iter()
        .flatten()
        .flat_map(|stage| &stage.current_freight)
        .map(String::as_str)
        .collect();
    let mut freight: Vec<&Freight> = freight.iter().collect();
    freight.sort_by(|a, b| b.created.cmp(&a.created).then(a.name.cmp(&b.name)));
    let mut listed = 0;
    let mut left = 0;
    for item in freight {
        let runs = running.contains(item.name.as_str());
        if !runs && listed >= MAX_FREIGHT {
            left += 1;
            continue;
        }
        listed += 1;
        changes.rows.push(row(project, item));
    }
    if left > 0 {
        changes.notes.push(Note {
            gap: Gap::Capped,
            project: Some(name.clone()),
            text: format!(
                "{name} lists its {MAX_FREIGHT} newest Freight; {left} older {} listed",
                if left == 1 { "isn't" } else { "aren't" }
            ),
        });
    }
}

fn row(project: &Project, freight: &Freight) -> Row {
    let stages = project.stages.read();
    let trail = match project.followed.get(&freight.name) {
        Some(change) => change.clone(),
        None if stages.is_none() => Err("Its Stages weren't read, so it wasn't followed".into()),
        None => Err("No Stage runs it, so it wasn't followed".into()),
    };
    let cell = |stage: &Stage| -> Reach {
        if !stage.current_freight.contains(&freight.name) {
            return Reach::NotReached;
        }
        let shown = trail.as_ref().ok().and_then(|change| {
            change
                .stages
                .iter()
                .find(|shown| shown.name == stage.name)
                .map(|shown| shown.reach.clone())
        });
        // It runs there, so not reached would be wrong: unknown, with the
        // trail saying why.
        match shown {
            Some(Reach::NotReached) | None => Reach::Linked(Confidence::Unknown),
            Some(reach) => reach,
        }
    };
    let cells: Vec<StageReach> = stages
        .map(promotion_order)
        .unwrap_or_default()
        .into_iter()
        .map(|stage| StageReach {
            name: stage.name.clone(),
            reach: cell(stage),
        })
        .collect();
    let state = match stages {
        // Where it runs isn't known.
        None => Reach::Linked(Confidence::Unknown),
        Some(_) => cells
            .iter()
            .map(|stage| &stage.reach)
            .filter(|reach| reach.rank().is_some())
            .min_by_key(|reach| reach.rank())
            .cloned()
            .unwrap_or(Reach::NotReached),
    };
    let fold = match &state {
        Reach::NotReached => Fold::NotRunning,
        Reach::Linked(Confidence::Confirmed) => Fold::Confirmed,
        _ => Fold::NotConfirmed,
    };
    let origin = trail.as_ref().ok().map(|change| &change.origin);
    let commit = origin
        .and_then(|origin| origin.commit.clone())
        .or_else(|| super::change::live::commit_of(freight).map(str::to_owned));
    let named = match (origin.and_then(|origin| origin.pull_request), commit) {
        (Some(number), _) => Named::PullRequest(number),
        (None, Some(commit)) => Named::Commit(commit),
        (None, None) => Named::Nothing,
    };
    let title = origin.and_then(|origin| origin.title.clone()).or_else(|| {
        freight
            .commits
            .first()
            .and_then(|commit| commit.subject())
            .map(str::to_owned)
    });
    Row {
        key: format!("{}/{}", project.name, freight.name),
        project: project.name.clone(),
        freight: freight.name.clone(),
        alias: freight.alias.clone(),
        named,
        title,
        created: freight.created,
        stages: cells,
        state,
        fold,
        trail,
    }
}
