//! The list read across the workspace: the Kargo projects' Freight and
//! Stages from Kargo's entry, a few projects at a time, and as each
//! answers, the change of each Freight a Stage runs, followed as the change
//! page follows it
//! ([`change::live::follow_in`](crate::change::live::follow_in)),
//! each part from its own entry, a few at a time. Freight no Stage runs
//! isn't followed: its cells are all not reached, and nothing about it
//! needs reading.
//!
//! Every request goes through one [`Pool`], which connects to each entry
//! once and asks each entry's questions through its own
//! [`SharedReader`](crate::shared::SharedReader), so what many
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
use crate::change::live::{self, Mapping, Never, Place};
use crate::entry::pool::{Open, Pool};
use crate::kargo::{Freight, parse_freight, parse_stage, read_kind, read_project_names};
use crate::read::Reader;
use crate::source::{Source, Truncation};
use freshkube_core::resources::{Failure, FailureKind};
use freshkube_core::workspace::{Chosen, Entry, Part, Sources};

/// How many projects, then changes, are read at once.
pub(crate) const AT_ONCE: usize = 4;

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
    /// The top-level setup's public key, opened afresh off the UI thread.
    pub public_key_file: Option<std::path::PathBuf>,
    /// Whether the log must hold each verified signature. Off by default.
    pub transparency_log: bool,
    /// The setup's log address; absent means read Chains' configuration.
    pub transparency_url: Option<String>,
    /// The pipeline record configured in the same setup.
    pub evidence_result: Option<freshkube_core::setup::Evidence>,
    /// The namespace the builds run in, from the same setup.
    pub build_namespace: Option<String>,
    /// How an Application with no authorized-stage annotation names its
    /// Stage, from the same setup.
    pub stage_naming: Option<freshkube_core::setup::StageNaming>,
    /// GitHub's REST client for this setup revision. The list reads
    /// through it sparingly: it stops looking things up when the limit is
    /// low.
    pub github: Option<crate::github::Handle>,
}

impl Where {
    /// Each followed change keeps the list's supply-chain settings.
    pub(super) fn place(&self, project: &str, freight: &str) -> Place {
        Place {
            cluster: self.cluster.clone(),
            label: self.label.clone(),
            project: project.to_owned(),
            freight: freight.to_owned(),
            argocd_namespace: self.argocd_namespace.clone(),
            mapping: self.mapping.clone(),
            sources: self.sources.clone(),
            public_key_file: self.public_key_file.clone(),
            transparency_log: self.transparency_log,
            transparency_url: self.transparency_url.clone(),
            evidence_result: self.evidence_result.clone(),
            build_namespace: self.build_namespace.clone(),
            stage_naming: self.stage_naming.clone(),
            github: self.github.as_ref().map(crate::github::Handle::sparing),
        }
    }

    /// The name of the cluster Kargo is read on, which the notes name.
    pub(crate) fn kargo(&self) -> String {
        match self.sources.get(Part::Kargo) {
            Chosen::Entry { entry, .. } if self.mapping.open.as_deref() != Some(&entry.id) => {
                entry.id.clone()
            }
            _ => self.label.clone(),
        }
    }
}

/// Why a part wasn't read: the budget ran out first, as the pool says it.
pub(crate) fn late(budget: Duration) -> String {
    format!("no answer within the read's {} s", budget.as_secs())
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
            std::future::ready(Err::<Never, _>(Failure::new(
                FailureKind::Config,
                "the workspace of one reads the open cluster alone",
            )))
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
    Fut: Future<Output = Result<C, Failure>>,
{
    let deadline = Instant::now() + budget;
    let reader = match pool.part(Part::Kargo).await {
        Ok((reader, _)) => reader,
        Err(unread) => {
            // The open cluster is named by its context, as the notes do.
            let unread = unread.named(at.kargo());
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
    Fut: Future<Output = Result<C, Failure>>,
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
            let place = at.place(&project.name, freight);
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
            Err(Failure::new(
                FailureKind::Timeout,
                format!("It wasn't followed: {}", late(turns.budget)),
            ))
        });
        project.followed.insert(freight, change.map(Arc::new));
    }
    cut
}
