//! The list read from one cluster: its Kargo projects' Freight and Stages,
//! a few projects at a time, and as each answers, the change of each
//! Freight a Stage runs, followed as the change page follows it
//! ([`change::live::follow`](crate::delivery::change::live::follow)), a few
//! at a time. Freight no Stage runs isn't followed: its cells are all not
//! reached, and nothing about it needs reading.
//!
//! Every request goes through one [`SharedReader`], so what many changes
//! need (the project's Kargo kinds, Argo CD's Applications, the ConfigMaps,
//! the workloads of a namespace, discovery) is read once per read, and only
//! each change's builds are its own. What hasn't answered within the budget
//! is left out with why, and the rest is kept.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::Semaphore;
use tokio::time::{Instant, timeout_at};

use super::{Project, Read};
use crate::applications::read::MAX_PROJECTS;
use crate::delivery::change::live::{self, Mapping, Place};
use crate::delivery::kargo::{Freight, parse_freight, parse_stage, read_kind, read_project_names};
use crate::delivery::read::Reader;
use crate::delivery::shared::SharedReader;
use crate::delivery::source::{Source, Truncation};

/// How many projects, then changes, are read at once.
const AT_ONCE: usize = 4;

/// How long a read may take before what hasn't answered is left out.
pub const BUDGET: Duration = Duration::from_secs(90);

/// How long one request may take ([`Deadline`](crate::delivery::entry::Deadline)):
/// a listing may run to [`MAX_PAGES`](crate::delivery::read::MAX_PAGES)
/// pages, each given the change page's GET deadline, and a little more.
pub const REQUEST_DEADLINE: Duration =
    Duration::from_secs(crate::delivery::change::live::pages::DEADLINE.as_secs() * 6);

/// Where the list is read: the cluster that runs Kargo, and how its
/// changes are followed past it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Where {
    /// The cluster's key, which the objects to open carry.
    pub cluster: String,
    /// Its name, which the page shows.
    pub label: String,
    /// The namespace Argo CD's Applications are read in.
    pub argocd_namespace: String,
    pub mapping: Mapping,
}

/// Why a part wasn't read: the budget ran out first.
fn late(budget: Duration) -> String {
    format!(
        "no answer within the {} s a read may take",
        budget.as_secs()
    )
}

/// Reads the list through `reader` alone, within `budget`.
///
/// Every project starts at once and takes one of [`AT_ONCE`] turns to read
/// its Freight and Stages, in whatever order they answer, so one that
/// hangs holds one turn, never the others. Its changes are followed as
/// soon as it answers, [`AT_ONCE`] at a time across the read, without
/// waiting for the other projects.
pub async fn read<R: Reader>(
    reader: &R,
    at: &Where,
    observed_at: DateTime<Utc>,
    budget: Duration,
) -> Read {
    let deadline = Instant::now() + budget;
    let reader = &SharedReader::new(reader);
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
            let late = follow(reader, at, observed_at, turns, &mut project).await;
            (project, late)
        }))
        .await;
    let late = read.iter().map(|(_, late)| late).sum();
    let projects = read.into_iter().map(|(project, _)| project).collect();
    Read {
        cluster: at.label.clone(),
        projects: match capped {
            Some(truncation) => Source::Capped(projects, truncation),
            None => Source::Read(projects),
        },
        late,
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
        cluster: at.label.clone(),
        projects,
        late: 0,
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
async fn follow<R: Reader>(
    reader: &R,
    at: &Where,
    observed_at: DateTime<Utc>,
    turns: &Turns,
    project: &mut Project,
) -> usize {
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
            };
            (place, listed)
        })
        .collect();
    let read = futures::future::join_all(wanted.into_iter().map(|(place, listed)| async move {
        let change = async {
            let _turn = turns.changes.acquire().await;
            match &listed {
                Some(freight) => Ok(live::follow(reader, &place, freight, observed_at).await),
                None => live::read(reader, &place, observed_at).await,
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
