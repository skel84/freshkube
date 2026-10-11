//! The list read across the workspace: the Kargo projects' Freight and
//! Stages from Kargo's entry, a few projects at a time, and as each
//! answers, the change of each Freight a Stage runs, followed as the change
//! page follows it
//! ([`change::live::follow_in`](crate::delivery::change::live::follow_in)),
//! each part from its own entry, a few at a time. Freight no Stage runs
//! isn't followed: its cells are all not reached, and nothing about it
//! needs reading.
//!
//! Every request goes through one [`Pool`], which connects to each entry
//! once and asks each entry's questions through its own
//! [`SharedReader`](crate::delivery::shared::SharedReader), so what many
//! changes need (the project's Kargo kinds, Argo CD's Applications, the
//! ConfigMaps, the workloads of a namespace, discovery) is read once per
//! read, and only each change's builds are its own. What hasn't answered
//! within the budget is left out with why, and the rest is kept. The
//! workspace of one ([`read`]) reads everything from the open connection.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::Semaphore;
use tokio::time::{Instant, timeout_at};

use super::{Project, Read};
use crate::applications::read::MAX_PROJECTS;
use crate::delivery::change::live::{self, Mapping, Never, Place};
use crate::delivery::entry::pool::{Open, Pool};
use crate::delivery::kargo::{Freight, parse_freight, parse_stage, read_kind, read_project_names};
use crate::delivery::read::Reader;
use crate::delivery::source::{Source, Truncation};
use crate::workspace::{Chosen, Entry, Part, Sources};

/// How many projects, then changes, are read at once.
const AT_ONCE: usize = 4;

/// How long a read may take before what hasn't answered is left out.
pub const BUDGET: Duration = Duration::from_secs(90);

/// Where the list is read: the open cluster, where each part is, and how
/// its changes are followed past Kargo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Where {
    /// The open cluster's key, which the objects to open on it carry.
    pub cluster: String,
    /// Its name, which the page shows.
    pub label: String,
    /// The namespace Argo CD's Applications are read in.
    pub argocd_namespace: String,
    pub mapping: Mapping,
    /// Where Kargo, the builds and Argo CD are read; the workspace of one's
    /// reads everything from the open cluster.
    pub sources: Sources,
}

impl Where {
    /// The name of the cluster Kargo is read on, which the notes name.
    fn kargo(&self) -> String {
        match self.sources.get(Part::Kargo) {
            Chosen::Entry { entry, .. } if self.mapping.open.as_deref() != Some(&entry.id) => {
                entry.id.clone()
            }
            _ => self.label.clone(),
        }
    }
}

/// Why a part wasn't read: the budget ran out first.
fn late(budget: Duration) -> String {
    format!(
        "no answer within the {} s a read may take",
        budget.as_secs()
    )
}

/// Reads the list through `reader` alone, within `budget`: the workspace
/// of one, whatever `at.sources` says.
pub async fn read<R: Reader>(
    reader: &R,
    at: &Where,
    observed_at: DateTime<Utc>,
    budget: Duration,
) -> Read {
    let alone = Where {
        sources: Sources::default(),
        ..at.clone()
    };
    let pool = Pool::new(
        &alone.sources,
        Some(Open {
            entry: alone.mapping.open.as_deref(),
            reader: Ok(reader),
        }),
        |_: Entry| {
            std::future::ready(Err::<Never, _>(
                "the workspace of one reads the open cluster alone".to_owned(),
            ))
        },
        budget,
    );
    read_in(&pool, &alone, observed_at, budget).await
}

/// Reads the list across the workspace through `pool`, within `budget`.
///
/// Kargo's entry is read first; one that can't be reached, or that no
/// entry holds, is the list's own state, in its words. Every project
/// starts at once and takes one of [`AT_ONCE`] turns to read its Freight
/// and Stages, in whatever order they answer, so one that hangs holds one
/// turn, never the others. Its changes are followed as soon as it answers,
/// [`AT_ONCE`] at a time across the read, without waiting for the other
/// projects.
pub async fn read_in<R, C, F, Fut>(
    pool: &Pool<'_, R, C, F>,
    at: &Where,
    observed_at: DateTime<Utc>,
    budget: Duration,
) -> Read
where
    R: Reader,
    C: Reader,
    F: Fn(Entry) -> Fut,
    Fut: Future<Output = Result<C, String>>,
{
    let deadline = Instant::now() + budget;
    let reader = match pool.part(Part::Kargo).await {
        Ok((reader, _)) => reader,
        Err(unread) => {
            return Read {
                cluster: at.kargo(),
                projects: Source::Unreadable(unread.words()),
                late: 0,
                unreached: Some(unread.words()),
            };
        }
    };
    let reader = &reader;
    let names = timeout_at(deadline, read_project_names(reader))
        .await
        .unwrap_or_else(|_| Source::Unreadable(late(budget)));
    let (names, capped) = match names {
        Source::Read(names) => (names, None),
        Source::Capped(names, truncation) => (names, Some(truncation)),
        Source::NotInstalled(why) => return unread(at, Source::NotInstalled(why)),
        Source::Refused(why) => return unread(at, Source::Refused(why)),
        Source::Unreadable(why) => return unread(at, Source::Unreadable(why)),
    };
    let capped = match names.len() > MAX_PROJECTS {
        true => Some(Truncation { read: MAX_PROJECTS }),
        false => capped,
    };
    let turns = &Turns {
        projects: Semaphore::new(AT_ONCE),
        changes: Semaphore::new(AT_ONCE),
        deadline,
        budget,
    };
    let read: Vec<(Project, usize)> =
        futures::future::join_all(names.into_iter().take(MAX_PROJECTS).map(|name| async move {
            let mut project = timeout_at(deadline, async {
                let _turn = turns.projects.acquire().await;
                project(reader, &name).await
            })
            .await
            .unwrap_or_else(|_| Project {
                name,
                freight: Source::Unreadable(late(budget)),
                stages: Source::Unreadable(late(budget)),
                followed: BTreeMap::new(),
            });
            let late = follow(pool, at, observed_at, turns, &mut project).await;
            (project, late)
        }))
        .await;
    let late = read.iter().map(|(_, late)| late).sum();
    let projects = read.into_iter().map(|(project, _)| project).collect();
    Read {
        cluster: at.kargo(),
        projects: match capped {
            Some(truncation) => Source::Capped(projects, truncation),
            None => Source::Read(projects),
        },
        late,
        unreached: None,
    }
}

/// The read's turns, shared by every project, and when it ends.
struct Turns {
    projects: Semaphore,
    changes: Semaphore,
    deadline: Instant,
    budget: Duration,
}

fn unread(at: &Where, projects: Source<Vec<Project>>) -> Read {
    Read {
        cluster: at.kargo(),
        projects,
        late: 0,
        unreached: None,
    }
}

/// A project's Freight and Stages.
async fn project<R: Reader>(reader: &R, name: &str) -> Project {
    let (freight, stages) = futures::join!(
        read_kind(reader, "freights", name, parse_freight),
        read_kind(reader, "stages", name, parse_stage),
    );
    Project {
        name: name.to_owned(),
        freight: Source::from_listing(freight),
        stages: Source::from_listing(stages),
        followed: BTreeMap::new(),
    }
}

/// Follows the change of every Freight a Stage of `project` runs, each in
/// one of the read's change turns. A Freight the project's listing doesn't
/// hold is read by name. Returns how many weren't followed within the
/// budget.
async fn follow<R, C, F, Fut>(
    pool: &Pool<'_, R, C, F>,
    at: &Where,
    observed_at: DateTime<Utc>,
    turns: &Turns,
    project: &mut Project,
) -> usize
where
    R: Reader,
    C: Reader,
    F: Fn(Entry) -> Fut,
    Fut: Future<Output = Result<C, String>>,
{
    let running: BTreeSet<&String> = project
        .stages
        .read()
        .into_iter()
        .flatten()
        .flat_map(|stage| &stage.current_freight)
        .collect();
    let wanted: Vec<(Place, Option<Freight>)> = running
        .into_iter()
        .map(|freight| {
            let listed = project
                .freight
                .read()
                .and_then(|all| all.iter().find(|item| &item.name == freight))
                .cloned();
            let place = Place {
                cluster: at.cluster.clone(),
                label: at.label.clone(),
                project: project.name.clone(),
                freight: freight.clone(),
                argocd_namespace: at.argocd_namespace.clone(),
                mapping: at.mapping.clone(),
                sources: at.sources.clone(),
            };
            (place, listed)
        })
        .collect();
    let read = futures::future::join_all(wanted.into_iter().map(|(place, listed)| async move {
        let change = async {
            let _turn = turns.changes.acquire().await;
            match &listed {
                Some(freight) => Ok(live::follow_in(pool, &place, freight, observed_at).await),
                None => live::read_in(pool, &place, observed_at).await,
            }
        };
        let change = timeout_at(turns.deadline, change).await;
        (place.freight, change)
    }))
    .await;
    let mut cut = 0;
    for (freight, change) in read {
        let change = change.unwrap_or_else(|_| {
            cut += 1;
            Err(format!("It wasn't followed: {}", late(turns.budget)))
        });
        project.followed.insert(freight, change.map(Arc::new));
    }
    cut
}
