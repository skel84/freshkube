//! The page read across the workspace: the Kargo projects' Freight and
//! Stages from Kargo's entry, [`AT_ONCE`] projects at a time, then one
//! GitHub call for each Freight in a gap, through the sparing client
//! (which stops looking things up when the rate limit is low).
//!
//! It reads no Argo CD, builds or pods: the gap is Kargo's. It reads the
//! Changes list's own kinds (a project's Freight and Stages) from the same
//! entry, within the same budget, and at most [`MAX_FREIGHT`] Freight a
//! project count, so at most that many GitHub calls a project. What hasn't
//! answered within the budget is left out with why, and the rest is kept.

use std::collections::BTreeMap;
use std::future::Future;

use chrono::{DateTime, Utc};
use tokio::sync::Semaphore;
use tokio::time::{Instant, timeout_at};

use super::{Project, PullRead, Read, pull_of, pull_unread};
use crate::applications::read::MAX_PROJECTS;
use crate::change::live::pull::chosen_request;
use crate::changes::live::{AT_ONCE, Where, late};
use crate::entry::pool::{Open, Pool};
use crate::github::repo_of;
use crate::kargo::{Freight, parse_freight, parse_stage, read_kind, read_project_names};
use crate::read::Reader;
use crate::source::{Source, Truncation};
use freshkube_core::resources::{Failure, FailureKind};
use freshkube_core::workspace::{Entry, Part, Sources};

pub use crate::changes::live::BUDGET;

/// Reads the page through `reader` alone, within `budget`: the workspace of
/// one, whatever `at.sources` says.
pub async fn read<R: Reader>(
    reader: &R,
    at: &Where,
    observed_at: DateTime<Utc>,
    budget: std::time::Duration,
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
            std::future::ready(Err::<crate::change::live::Never, _>(Failure::new(
                FailureKind::Config,
                "the workspace of one reads the open cluster alone",
            )))
        },
        budget,
    );
    read_in(&pool, &alone, observed_at, budget).await
}

/// Reads the page across the workspace through `pool`, within `budget`.
///
/// Kargo's entry is read first; one that can't be reached is the page's
/// own state, in its words. Every project starts at once and takes one of
/// [`AT_ONCE`] turns to read its Freight and Stages, in whatever order they
/// answer, so one that hangs holds one turn, never the others. Its gap's
/// pull requests are asked for as soon as it answers, [`AT_ONCE`] at a time
/// across the read.
pub async fn read_in<R, C, F, Fut>(
    pool: &Pool<'_, R, C, F>,
    at: &Where,
    observed_at: DateTime<Utc>,
    budget: std::time::Duration,
) -> Read
where
    R: Reader,
    C: Reader,
    F: Fn(Entry) -> Fut,
    Fut: Future<Output = Result<C, Failure>>,
{
    let deadline = Instant::now() + budget;
    let cluster = at.kargo();
    let pulls_read = at.github.is_some();
    let nothing = |projects, unreached| Read {
        cluster: cluster.clone(),
        projects,
        unreached,
        pulls_read,
        observed_at,
    };
    let reader = match pool.part(Part::Kargo).await {
        Ok((reader, _)) => reader,
        Err(unread) => {
            let unread = unread.named(cluster.clone());
            return nothing(Source::Unreadable(unread.words()), Some(unread.words()));
        }
    };
    let reader = &reader;
    let names = timeout_at(deadline, read_project_names(reader))
        .await
        .unwrap_or_else(|_| Source::Unreadable(late(budget)));
    let (names, capped) = match names {
        Source::Read(names) => (names, None),
        Source::Capped(names, truncation) => (names, Some(truncation)),
        Source::NotInstalled(why) => return nothing(Source::NotInstalled(why), None),
        Source::Refused(why) => return nothing(Source::Refused(why), None),
        Source::Unreadable(why) => return nothing(Source::Unreadable(why), None),
    };
    let capped = match names.len() > MAX_PROJECTS {
        true => Some(Truncation { read: MAX_PROJECTS }),
        false => capped,
    };
    let projects = &Semaphore::new(AT_ONCE);
    let calls = &Semaphore::new(AT_ONCE);
    let read: Vec<Project> =
        futures::future::join_all(names.into_iter().take(MAX_PROJECTS).map(|name| async move {
            let mut project = timeout_at(deadline, async {
                let _turn = projects.acquire().await;
                project(reader, &name).await
            })
            .await
            .unwrap_or_else(|_| Project {
                name,
                freight: Source::Unreadable(late(budget)),
                stages: Source::Unreadable(late(budget)),
                pulls: BTreeMap::new(),
            });
            ask(at, calls, deadline, budget, &mut project).await;
            project
        }))
        .await;
    nothing(
        match capped {
            Some(truncation) => Source::Capped(read, truncation),
            None => Source::Read(read),
        },
        None,
    )
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
        pulls: BTreeMap::new(),
    }
}

/// Asks GitHub for the pull request of each Freight in `project`'s gaps,
/// one call each, in one of the read's turns. A Freight with no GitHub
/// commit isn't asked about.
async fn ask(
    at: &Where,
    calls: &Semaphore,
    deadline: Instant,
    budget: std::time::Duration,
    project: &mut Project,
) {
    let Some(github) = at.github.as_ref().map(crate::github::Handle::sparing) else {
        return;
    };
    let Some(wanted) = super::wanted(project) else {
        return;
    };
    let github = &github;
    let asked = futures::future::join_all(wanted.into_iter().map(|freight: Freight| async move {
        let answer = async {
            let (Some(repo), Some(commit)) = (
                repo_of(&freight),
                freight.commits.first().map(|c| c.id.clone()),
            ) else {
                return None;
            };
            let _turn = calls.acquire().await;
            Some(match github.pulls_for_commit_or(&repo, &commit).await {
                Ok(pulls) => match chosen_request(&pulls, Some(&commit), None).and_then(pull_of) {
                    Some(pull) => PullRead::Found(pull),
                    None => PullRead::NoPull,
                },
                Err(error) => pull_unread(&error),
            })
        };
        let answer = match timeout_at(deadline, answer).await {
            Ok(answer) => answer,
            Err(_) => Some(PullRead::Unread {
                rate_limit: false,
                why: format!("Pull requests weren't read: {}", late(budget)),
            }),
        };
        (freight.name, answer)
    }))
    .await;
    project.pulls.extend(
        asked
            .into_iter()
            .filter_map(|(name, answer)| Some((name, answer?))),
    );
}
