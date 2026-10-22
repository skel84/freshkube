//! Waiting to ship (#842): for each Kargo project, the Freight that is built
//! or deployed somewhere but not yet in production, and how long it has
//! waited.
//!
//! Production is every Stage with nothing downstream in promotion order: no
//! other Stage takes Freight from it. A project that fans out to several (two
//! regions, say) has one gap per final Stage, and the application shows the
//! furthest behind. The gap of a final Stage is the Freight, from the
//! Warehouses it takes Freight from, made after the Freight it runs.
//!
//! Everything here is Kargo's: the Freight and each Stage's Freight history
//! and verifications, which the Changes list reads already, plus the pull
//! request GitHub names for each Freight in a gap. Nothing reads Argo CD,
//! the builds or the pods, so a gap says what Kargo records and nothing
//! about whether pods run it. Evidence that isn't there is said so, never
//! guessed: a wait that can't be dated has no time, a pull request that
//! wasn't asked for has no number.
//!
//! [`derive`] reads nothing: [`live`] reads one cluster, and [`example`]
//! invents acme's.

pub mod example;
pub mod live;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};

use super::change::{Cause, Role};
use super::changes::{Gap, MAX_FREIGHT, Note, unread};
use super::github::PullState;
use super::kargo::{Freight, Stage, promotion_order};
use super::source::Source;

/// A pull request GitHub names for a Freight's commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pull {
    pub number: u64,
    pub title: Option<String>,
    /// The author's login.
    pub author: Option<String>,
    pub state: PullState,
    pub merged_at: Option<DateTime<Utc>>,
    /// Its head commit, and the commit its merge made.
    pub head: Option<String>,
    pub merge: Option<String>,
    /// Set when GitHub couldn't be asked and this is what was kept.
    pub stale: Option<String>,
}

/// What asking GitHub for a Freight's pull request came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PullRead {
    Found(Pull),
    /// GitHub answered and names none.
    NoPull,
    /// GitHub wasn't asked, or couldn't answer, in words. A read that the
    /// sparing client skipped because the rate limit is low or reached says
    /// so, which is not a failure.
    Unread {
        rate_limit: bool,
        why: String,
    },
}

/// One Kargo project, as the page reads it.
#[derive(Clone, Debug)]
pub struct Project {
    pub name: String,
    pub freight: Source<Vec<Freight>>,
    pub stages: Source<Vec<Stage>>,
    /// The pull request of each Freight in a gap, by the Freight's name.
    /// A Freight not in a gap isn't asked about.
    pub pulls: BTreeMap<String, PullRead>,
}

/// What the page reads: the projects of the cluster that runs Kargo.
#[derive(Clone, Debug)]
pub struct Read {
    /// That cluster's name, which the notes name.
    pub cluster: String,
    pub projects: Source<Vec<Project>>,
    /// Why Kargo's entry wasn't read at all, in its words.
    pub unreached: Option<String>,
    /// Whether pull requests are read at all in this setup.
    pub pulls_read: bool,
    /// When it was read, which every wait counts to.
    pub observed_at: DateTime<Utc>,
}

/// The page's groups, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Fold {
    /// One group for each application with something in a gap.
    Waiting,
    /// Applications whose Freight or Stages weren't read.
    NotRead,
    /// Applications with nothing waiting, in one group.
    CaughtUp,
}

impl Fold {
    pub fn label(self) -> &'static str {
        match self {
            Self::Waiting => "Waiting",
            Self::NotRead => "Not read",
            Self::CaughtUp => "Caught up",
        }
    }
}

/// Why a Freight is stuck, in the change page's words: the cause, and what
/// would move it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Why {
    pub cause: Cause,
    /// What it waits for has failed, so waiting won't move it.
    pub blocked: bool,
}

impl Why {
    pub fn word(&self) -> &'static str {
        self.cause.word()
    }

    pub fn needs(&self) -> &str {
        self.cause.needs()
    }
}

/// One Freight in a gap.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub freight: String,
    pub alias: Option<String>,
    pub created: Option<DateTime<Utc>>,
    /// The Freight's first commit.
    pub commit: Option<String>,
    /// Its pull request's title, else its commit's subject.
    pub title: Option<String>,
    /// What GitHub said; none when it wasn't asked.
    pub pull: Option<PullRead>,
    /// The furthest Stage before production it has been in, and when it
    /// reached it, if Kargo records that: the start of its first
    /// verification there.
    pub in_stage: Option<String>,
    pub reached: Option<DateTime<Utc>>,
    pub why: Why,
    /// Whether promoting it now is possible: the Stage that feeds
    /// production verified it, or production takes Freight straight from
    /// its Warehouse.
    pub promotable: bool,
}

impl Item {
    /// How long it has waited: from when it reached the Stage before
    /// production, not from the merge. None when Kargo doesn't date that.
    pub fn waited(&self, now: DateTime<Utc>) -> Option<Duration> {
        self.reached
            .map(|reached| (now - reached).max(Duration::zero()))
    }

    pub fn pull(&self) -> Option<&Pull> {
        match &self.pull {
            Some(PullRead::Found(pull)) => Some(pull),
            _ => None,
        }
    }
}

/// What promoting now would ship to one final Stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Next {
    /// The Freight Kargo would promote: the newest the Stage that feeds
    /// production verified.
    pub freight: String,
    /// Every Freight in the gap it brings, oldest first, itself last.
    pub brings: Vec<String>,
}

/// One final Stage's gap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageGap {
    pub stage: String,
    /// The Freight it runs, by name.
    pub runs: Vec<String>,
    /// The Freight ahead of it, oldest first.
    pub ahead: Vec<String>,
    /// There may be more: the Freight it runs isn't among those listed.
    pub at_least: bool,
    pub next: Option<Next>,
}

/// One Stage of the project, in promotion order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageLine {
    pub name: String,
    pub is_final: bool,
    /// The Freight it runs, and every Freight its history holds.
    pub runs: Vec<String>,
    pub held: Vec<String>,
    pub verified: Vec<String>,
}

/// A listed Freight, for finding a commit among those that aren't waiting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub freight: String,
    pub alias: Option<String>,
    pub commits: Vec<String>,
    pub title: Option<String>,
}

/// Where an application stands.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Waiting,
    CaughtUp,
    /// Its Freight or Stages weren't read; the cause says why.
    NotRead {
        gap: Gap,
        cause: Cause,
        text: String,
    },
}

/// One application: a Kargo project.
#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub name: String,
    pub state: State,
    /// The final Stage the application shows, the furthest behind.
    pub furthest: Option<String>,
    /// The furthest behind's gap, oldest first.
    pub items: Vec<Item>,
    /// Every final Stage's gap, furthest behind first.
    pub finals: Vec<StageGap>,
    pub stages: Vec<StageLine>,
    pub listed: Vec<Listed>,
    /// How many older Freight the cap left out of what was read.
    pub older: usize,
}

impl App {
    pub fn fold(&self) -> Fold {
        match self.state {
            State::Waiting => Fold::Waiting,
            State::CaughtUp => Fold::CaughtUp,
            State::NotRead { .. } => Fold::NotRead,
        }
    }

    /// The gap of the final Stage the application shows.
    pub fn gap(&self) -> Option<&StageGap> {
        self.finals.first()
    }

    /// The Stages that count as production, in promotion order.
    pub fn production(&self) -> Vec<&str> {
        self.stages
            .iter()
            .filter(|stage| stage.is_final)
            .map(|stage| stage.name.as_str())
            .collect()
    }

    /// How long its oldest dated item has waited.
    pub fn longest(&self, now: DateTime<Utc>) -> Option<Duration> {
        self.items.iter().filter_map(|item| item.waited(now)).max()
    }

    /// The Freight a query names among those read: a gap's item by its pull
    /// request or its commit, any listed Freight by its commit.
    pub fn find(&self, query: &Query) -> Vec<&str> {
        let mut found: Vec<&str> = Vec::new();
        for item in &self.items {
            let hit = match query {
                Query::Pull(number) => item.pull().is_some_and(|pull| pull.number == *number),
                Query::Commit(prefix) => {
                    starts(item.commit.as_deref(), prefix)
                        || item.pull().is_some_and(|pull| {
                            starts(pull.head.as_deref(), prefix)
                                || starts(pull.merge.as_deref(), prefix)
                        })
                }
            };
            if hit {
                found.push(&item.freight);
            }
        }
        if let Query::Commit(prefix) = query {
            for listed in &self.listed {
                if listed.commits.iter().any(|id| starts(Some(id), prefix))
                    && !found.contains(&listed.freight.as_str())
                {
                    found.push(&listed.freight);
                }
            }
        }
        found
    }

    /// Where a Freight stands in each Stage, in promotion order.
    pub fn trace(&self, freight: &str) -> Vec<(String, Standing)> {
        self.stages
            .iter()
            .map(|stage| {
                let standing = if stage.runs.iter().any(|name| name == freight) {
                    Standing::Runs
                } else if stage.held.iter().any(|name| name == freight) {
                    Standing::Ran
                } else if stage.verified.iter().any(|name| name == freight) {
                    Standing::Verified
                } else {
                    Standing::Not
                };
                (stage.name.clone(), standing)
            })
            .collect()
    }

    /// Whether the Freight is in production: every final Stage runs it or
    /// ran it. Missing Stages give no.
    pub fn shipped(&self, freight: &str) -> Shipped {
        let finals: Vec<&StageLine> = self.stages.iter().filter(|s| s.is_final).collect();
        if finals.is_empty() {
            return Shipped::Unknown;
        }
        let reached = |stage: &&StageLine| {
            stage
                .runs
                .iter()
                .chain(&stage.held)
                .any(|name| name == freight)
        };
        let all = finals.iter().all(reached);
        let some = finals.iter().any(reached);
        match (all, some) {
            (true, _) => Shipped::Everywhere,
            (false, true) => Shipped::Partly,
            (false, false) => Shipped::No,
        }
    }
}

fn starts(id: Option<&str>, prefix: &str) -> bool {
    id.is_some_and(|id| id.to_ascii_lowercase().starts_with(prefix))
}

/// A Freight in one Stage, as its history and verifications record it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// The Stage runs it now.
    Runs,
    /// The Stage ran it before.
    Ran,
    /// The Stage verified it, but its history doesn't hold it.
    Verified,
    Not,
}

/// Whether a Freight reached production.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shipped {
    Everywhere,
    /// Some final Stages ran it, not all.
    Partly,
    No,
    Unknown,
}

/// What the find box asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Query {
    /// A pull request, by number.
    Pull(u64),
    /// A commit, by the start of its id, in lower case.
    Commit(String),
}

impl Query {
    /// `418`, `#418`, a pull request's address, or the start of a commit id
    /// of at least four hexadecimal digits. Anything else asks nothing.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = match text.split_once("/pull/") {
            Some((_, rest)) => rest.split(['/', '?', '#']).next().unwrap_or(rest),
            None => text.trim_start_matches('#'),
        };
        if text.is_empty() {
            return None;
        }
        if text.bytes().all(|b| b.is_ascii_digit()) && text.len() <= 6 {
            return text.parse().ok().map(Self::Pull);
        }
        (text.len() >= 4 && text.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| Self::Commit(text.to_ascii_lowercase()))
    }
}

/// The page: its applications in order, and what may be missing.
#[derive(Clone, Debug, Default)]
pub struct Waiting {
    /// By group, then the one waiting longest first.
    pub apps: Vec<App>,
    pub notes: Vec<Note>,
    pub observed_at: Option<DateTime<Utc>>,
}

impl Waiting {
    pub fn waiting(&self) -> usize {
        self.apps
            .iter()
            .filter(|app| app.state == State::Waiting)
            .count()
    }

    /// How many Freight wait, across the applications.
    pub fn items(&self) -> usize {
        self.apps.iter().map(|app| app.items.len()).sum()
    }
}

/// The page from what was read.
pub fn derive(read: &Read) -> Waiting {
    let mut waiting = Waiting {
        observed_at: Some(read.observed_at),
        ..Waiting::default()
    };
    if let Some(why) = &read.unreached {
        waiting.notes.push(Note {
            gap: Gap::Unreadable,
            project: None,
            text: why.clone(),
        });
        return waiting;
    }
    let projects = match &read.projects {
        Source::Read(projects) => projects,
        Source::Capped(projects, truncation) => {
            waiting.notes.push(Note {
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
            waiting
                .notes
                .extend(unread(other, None, "Kargo projects", &read.cluster));
            return waiting;
        }
    };
    let mut projects: Vec<&Project> = projects.iter().collect();
    projects.sort_by(|a, b| a.name.cmp(&b.name));
    for project in projects {
        waiting.apps.push(app(project, read, &mut waiting.notes));
    }
    let now = read.observed_at;
    waiting.apps.sort_by(|a, b| {
        a.fold()
            .cmp(&b.fold())
            // The longest wait first; an undated wait after the dated.
            .then_with(|| b.longest(now).cmp(&a.longest(now)))
            .then_with(|| a.name.cmp(&b.name))
    });
    if !read.pulls_read && waiting.items() > 0 {
        waiting.notes.push(Note {
            gap: Gap::NotServed,
            project: None,
            text: "Pull requests aren't read: GitHub isn't set up, so a Freight is named by its \
                   commit"
                .into(),
        });
    }
    for app in waiting.apps.iter().filter(|_| read.pulls_read) {
        let limited = app
            .items
            .iter()
            .filter(|item| {
                matches!(
                    item.pull,
                    Some(PullRead::Unread {
                        rate_limit: true,
                        ..
                    })
                )
            })
            .count();
        if limited > 0 {
            waiting.notes.push(Note {
                gap: Gap::Unreadable,
                project: Some(app.name.clone()),
                text: format!(
                    "{limited} of {}'s pull requests weren't read: GitHub's rate limit is low or \
                     reached",
                    app.name
                ),
            });
        }
        for item in &app.items {
            if let Some(PullRead::Unread {
                rate_limit: false,
                why,
            }) = &item.pull
            {
                let note = Note {
                    gap: Gap::Unreadable,
                    project: Some(app.name.clone()),
                    text: why.clone(),
                };
                if !waiting.notes.contains(&note) {
                    waiting.notes.push(note);
                }
            }
        }
    }
    waiting
}

/// The Stages with nothing downstream: no other Stage takes Freight from them.
fn finals<'a>(stages: &[&'a Stage]) -> Vec<&'a Stage> {
    stages
        .iter()
        .copied()
        .filter(|stage| {
            !stages
                .iter()
                .any(|other| other.upstream.contains(&stage.name))
        })
        .collect()
}

/// Whether a Freight belongs to the Warehouses a Stage takes Freight from.
fn feeds(stage: &Stage, freight: &Freight) -> bool {
    stage.warehouses.is_empty()
        || freight
            .warehouse
            .as_ref()
            .is_none_or(|warehouse| stage.warehouses.contains(warehouse))
}

/// The Freight a final Stage is behind, oldest first, from the project's
/// newest [`MAX_FREIGHT`] Freight `listed` (newest first), and whether there
/// may be more than it shows.
fn gap<'a>(stage: &Stage, listed: &[&'a Freight], all: &[Freight]) -> (Vec<&'a Freight>, bool) {
    let relevant = listed.iter().copied().filter(|f| feeds(stage, f));
    let runs: Vec<&Freight> = stage
        .current_freight
        .iter()
        .filter_map(|name| all.iter().find(|f| &f.name == name))
        .collect();
    let baseline = runs.iter().filter_map(|f| f.created).max();
    let in_listing = stage
        .current_freight
        .iter()
        .any(|name| listed.iter().any(|f| &f.name == name));
    let (mut ahead, at_least): (Vec<&Freight>, bool) =
        match (stage.current_freight.is_empty(), baseline) {
            // It runs nothing: everything it takes is ahead of it.
            (true, _) => (relevant.collect(), listed.len() < all.len()),
            (false, Some(baseline)) if in_listing => (
                relevant
                    .filter(|f| f.created.is_some_and(|made| made > baseline))
                    .filter(|f| !stage.current_freight.contains(&f.name))
                    .collect(),
                false,
            ),
            // What it runs isn't among the newest Freight, or has no date:
            // all those listed may be ahead of it, and more.
            _ => (
                relevant
                    .filter(|f| !stage.current_freight.contains(&f.name))
                    .collect(),
                true,
            ),
        };
    ahead.sort_by(|a, b| a.created.cmp(&b.created).then(a.name.cmp(&b.name)));
    (ahead, at_least)
}

/// The project's newest [`MAX_FREIGHT`] dated Freight, newest first, how
/// many have no date to place them by, and how many older ones were left.
fn newest(freight: &[Freight]) -> (Vec<&Freight>, usize, usize) {
    let mut newest: Vec<&Freight> = freight.iter().filter(|f| f.created.is_some()).collect();
    newest.sort_by(|a, b| b.created.cmp(&a.created).then(a.name.cmp(&b.name)));
    let undated = freight.len() - newest.len();
    let left = newest.len().saturating_sub(MAX_FREIGHT);
    newest.truncate(MAX_FREIGHT);
    (newest, undated, left)
}

/// The Freight in any final Stage's gap: those to ask GitHub about. None
/// when the Freight or Stages weren't read.
pub(crate) fn wanted(project: &Project) -> Option<Vec<Freight>> {
    let (freight, stages) = (project.freight.read()?, project.stages.read()?);
    let (newest, ..) = newest(freight);
    let ordered = promotion_order(stages);
    let mut wanted: Vec<Freight> = Vec::new();
    for stage in finals(&ordered) {
        for freight in gap(stage, &newest, freight).0 {
            if !wanted.iter().any(|held| held.name == freight.name) {
                wanted.push(freight.clone());
            }
        }
    }
    Some(wanted)
}

/// What a source that wasn't read makes of the application: not read, with
/// the cause in the change page's words. None for one that was.
fn not_read<T>(
    project: &Project,
    source: &Source<T>,
    (what, kind): (&str, &str),
    cluster: &str,
    notes: &mut Vec<Note>,
) -> Option<State> {
    let name = &project.name;
    let note = unread(source, Some(name), &format!("{name}'s {what}"), cluster)?;
    let needs = format!("Kargo's {kind} for {name}");
    let cause = match note.gap {
        Gap::NotServed => Cause::not_found(needs),
        Gap::Refused => Cause::cant_read(
            needs,
            Some(Role::new("kargo.akuity.io", kind, Some(name))),
            true,
        ),
        _ => Cause::cant_read(needs, None, false),
    };
    let state = State::NotRead {
        gap: note.gap,
        cause,
        text: note.text.clone(),
    };
    notes.push(note);
    Some(state)
}

fn app(project: &Project, read: &Read, notes: &mut Vec<Note>) -> App {
    let name = &project.name;
    let cluster = &read.cluster;
    let empty = |state| App {
        name: name.clone(),
        state,
        furthest: None,
        items: Vec::new(),
        finals: Vec::new(),
        stages: Vec::new(),
        listed: Vec::new(),
        older: 0,
    };
    let Some(freight) = project.freight.read() else {
        return not_read(
            project,
            &project.freight,
            ("Freight", "freights"),
            cluster,
            notes,
        )
        .map_or_else(|| empty(State::CaughtUp), empty);
    };
    let Some(stages) = project.stages.read() else {
        return not_read(
            project,
            &project.stages,
            ("Stages", "stages"),
            cluster,
            notes,
        )
        .map_or_else(|| empty(State::CaughtUp), empty);
    };
    if let Source::Capped(_, truncation) = &project.freight {
        notes.push(Note {
            gap: Gap::Capped,
            project: Some(name.clone()),
            text: format!(
                "{name}'s Freight list stopped at the page cap after {}; older Freight may not \
                 be listed",
                truncation.read
            ),
        });
    }
    if let Source::Capped(_, truncation) = &project.stages {
        notes.push(Note {
            gap: Gap::Capped,
            project: Some(name.clone()),
            text: format!(
                "{name}'s Stage list stopped at the page cap after {}; a Stage not read may be \
                 production",
                truncation.read
            ),
        });
    }
    let (newest, undated, left) = newest(freight);
    if undated > 0 {
        notes.push(Note {
            gap: Gap::Unreadable,
            project: Some(name.clone()),
            text: format!(
                "{undated} of {name}'s Freight have no creation time, so {} can't be placed \
                 before or after a Stage's",
                if undated == 1 { "it" } else { "they" }
            ),
        });
    }
    if left > 0 {
        notes.push(Note {
            gap: Gap::Capped,
            project: Some(name.clone()),
            text: format!(
                "{name} lists its {MAX_FREIGHT} newest Freight; {left} older {} listed",
                if left == 1 { "isn't" } else { "aren't" }
            ),
        });
    }
    let ordered = promotion_order(stages);
    let last = finals(&ordered);
    let mut gaps: Vec<(StageGap, Vec<Item>)> = last
        .iter()
        .map(|stage| stage_gap(stage, &ordered, &newest, freight, project, read))
        .collect();
    // The furthest behind first: the most Freight, then the oldest wait.
    gaps.sort_by(|(a, a_items), (b, b_items)| {
        b.ahead
            .len()
            .cmp(&a.ahead.len())
            .then_with(|| oldest(a_items).cmp(&oldest(b_items)))
            .then(a.stage.cmp(&b.stage))
    });
    let stage_lines = ordered
        .iter()
        .map(|stage| StageLine {
            name: stage.name.clone(),
            is_final: last.iter().any(|last| last.name == stage.name),
            runs: stage.current_freight.clone(),
            held: stage.history.iter().flatten().cloned().collect(),
            verified: freight
                .iter()
                .filter(|f| f.verified_in.contains(&stage.name))
                .map(|f| f.name.clone())
                .collect(),
        })
        .collect();
    let listed = newest
        .iter()
        .map(|f| Listed {
            freight: f.name.clone(),
            alias: f.alias.clone(),
            commits: f.commits.iter().map(|c| c.id.clone()).collect(),
            title: f
                .commits
                .first()
                .and_then(|c| c.subject())
                .map(str::to_owned),
        })
        .collect();
    let waiting = gaps.iter().any(|(gap, _)| !gap.ahead.is_empty());
    let (finals, mut first): (Vec<StageGap>, Vec<Vec<Item>>) = gaps.into_iter().unzip();
    App {
        name: name.clone(),
        state: if waiting {
            State::Waiting
        } else {
            State::CaughtUp
        },
        furthest: finals.first().map(|gap| gap.stage.clone()),
        items: if first.is_empty() {
            Vec::new()
        } else {
            first.swap_remove(0)
        },
        finals,
        stages: stage_lines,
        listed,
        older: left,
    }
}

/// When the oldest dated item reached its Stage; undated last.
fn oldest(items: &[Item]) -> (bool, Option<DateTime<Utc>>) {
    let at = items.iter().filter_map(|item| item.reached).min();
    (at.is_none(), at)
}

fn stage_gap(
    stage: &Stage,
    ordered: &[&Stage],
    newest: &[&Freight],
    all: &[Freight],
    project: &Project,
    read: &Read,
) -> (StageGap, Vec<Item>) {
    let (ahead, at_least) = gap(stage, newest, all);
    let mut items: Vec<Item> = ahead
        .iter()
        .map(|freight| item(freight, stage, ordered, project, read))
        .collect();
    // Oldest first: by when it reached the Stage before production, else by
    // when it was made.
    items.sort_by(|a, b| {
        (a.reached.is_none(), a.reached, a.created, &a.freight).cmp(&(
            b.reached.is_none(),
            b.reached,
            b.created,
            &b.freight,
        ))
    });
    let next = items
        .iter()
        .filter(|item| item.promotable)
        .max_by_key(|item| item.created)
        .map(|item| {
            let mut brings: Vec<&Item> = items
                .iter()
                .filter(|other| other.created <= item.created && other.freight != item.freight)
                .collect();
            brings.sort_by_key(|other| (other.created, other.freight.clone()));
            Next {
                freight: item.freight.clone(),
                brings: brings
                    .into_iter()
                    .map(|other| other.freight.clone())
                    .chain([item.freight.clone()])
                    .collect(),
            }
        });
    let gap = StageGap {
        stage: stage.name.clone(),
        runs: stage.current_freight.clone(),
        ahead: items.iter().map(|item| item.freight.clone()).collect(),
        at_least,
        next,
    };
    (gap, items)
}

fn item(
    freight: &Freight,
    last: &Stage,
    ordered: &[&Stage],
    project: &Project,
    read: &Read,
) -> Item {
    // The furthest Stage other than `last` that has held it.
    let held = |stage: &&&Stage| {
        stage
            .history
            .iter()
            .flatten()
            .any(|name| name == &freight.name)
            || stage.current_freight.contains(&freight.name)
    };
    // A final Stage is production, not the Stage before it.
    let in_stage = ordered
        .iter()
        .filter(|stage| {
            !ordered
                .iter()
                .all(|other| !other.upstream.contains(&stage.name))
        })
        .rfind(held)
        .copied();
    let reached = in_stage.and_then(|stage| {
        stage
            .verifications
            .iter()
            .filter(|v| v.freight.contains(&freight.name))
            .filter_map(|v| v.started.as_deref())
            .filter_map(|at| at.parse::<DateTime<Utc>>().ok())
            .min()
    });
    let promotable = (last.upstream.is_empty() && last.direct)
        || last
            .upstream
            .iter()
            .any(|name| freight.verified_in.contains(name));
    let commit = freight.commits.first().map(|c| c.id.clone());
    let pull = project.pulls.get(&freight.name).cloned().or_else(|| {
        (!read.pulls_read).then(|| PullRead::Unread {
            rate_limit: false,
            why: "Pull requests aren't read".into(),
        })
    });
    let title = match &pull {
        Some(PullRead::Found(pull)) => pull.title.clone(),
        _ => None,
    }
    .or_else(|| {
        freight
            .commits
            .first()
            .and_then(|commit| commit.subject())
            .map(str::to_owned)
    });
    Item {
        freight: freight.name.clone(),
        alias: freight.alias.clone(),
        created: freight.created,
        commit,
        title,
        pull,
        why: why(freight, last, in_stage, ordered),
        in_stage: in_stage.map(|stage| stage.name.clone()),
        reached,
        promotable,
    }
}

/// Why `freight` isn't in `last` yet, from Kargo's record of the Stage that
/// holds it.
fn why(freight: &Freight, last: &Stage, in_stage: Option<&Stage>, ordered: &[&Stage]) -> Why {
    let mut blocked = false;
    let cause = match in_stage {
        None => {
            let first: Vec<&str> = ordered
                .iter()
                .filter(|stage| stage.direct && stage.name != last.name)
                .map(|stage| stage.name.as_str())
                .collect();
            Cause::waiting(match first.as_slice() {
                [] => format!("a promotion into {}", last.name),
                first => format!("its first promotion, into {}", first.join(" or ")),
            })
        }
        Some(stage) => {
            let verified = freight.verified_in.contains(&stage.name);
            let latest = stage.latest_verification(&freight.name);
            let next = ordered
                .iter()
                .find(|next| next.upstream.contains(&stage.name))
                .map_or(last.name.as_str(), |next| next.name.as_str());
            match (verified, latest.and_then(|v| v.phase.as_deref())) {
                (true, _) | (_, Some("Successful")) => {
                    Cause::waiting(format!("a promotion from {} to {next}", stage.name))
                }
                (false, Some(phase @ ("Failed" | "Error" | "Aborted" | "Inconclusive"))) => {
                    blocked = true;
                    Cause::waiting(format!(
                        "{}'s verification to pass; its last one {}",
                        stage.name,
                        phase.to_ascii_lowercase()
                    ))
                }
                (false, Some(_)) => {
                    Cause::waiting(format!("{}'s verification to finish", stage.name))
                }
                (false, None) => Cause::waiting(format!(
                    "{}'s verification; none is recorded for it",
                    stage.name
                )),
            }
        }
    };
    Why { cause, blocked }
}

/// A pull request read from GitHub, kept for a Freight.
pub(crate) fn pull_of(request: &super::github::PullRequest) -> Option<Pull> {
    let summary = request.summary.as_ref()?;
    Some(Pull {
        number: summary.number,
        title: summary.title.clone().or_else(|| request.title.clone()),
        author: summary.author.as_ref().map(|person| person.login.clone()),
        state: summary.state,
        merged_at: summary.merged_at,
        head: Some(request.head_sha.clone()),
        merge: request.merge_sha.clone(),
        stale: summary.stale.as_ref().map(super::github::Stale::sentence),
    })
}

/// A failed read of a pull request, as the page words it. A limit that was
/// low or reached is a skipped read, not a failure.
pub(crate) fn pull_unread(error: &super::github::Error) -> PullRead {
    PullRead::Unread {
        rate_limit: matches!(
            error,
            super::github::Error::Low { .. } | super::github::Error::Limited { .. }
        ),
        why: format!("Pull requests weren't read: {error}"),
    }
}
