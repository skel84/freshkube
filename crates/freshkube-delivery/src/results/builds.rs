//! Archived builds supplement a successful cluster read. The cluster's
//! manifest, tasks and outcome always win for an equal nonempty UID.
use std::collections::BTreeMap;

use tokio::time::Instant;

use super::{Cap, Client, Error, Listing, Log, Record, Skip};
use crate::digest::Digest;
use crate::source::Source;
use crate::tekton::{Build, FoundBy, Origin, TaskRun, built_images};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Archive {
    pub(crate) record: String,
    pub(crate) result: String,
    /// Not requested for a run still on the cluster.
    pub(crate) logs: Option<Result<Listing<Log>, Error>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct History {
    pub(crate) runs: usize,
    pub(crate) cap: Option<Cap>,
    pub(crate) total: Option<usize>,
}

pub(crate) async fn supplement(
    mut live: Source<Vec<Build>>,
    client: Result<&Client, Error>,
    namespace: Option<&str>,
    sha: &str,
    until: Instant,
) -> (Source<Vec<Build>>, Result<History, Error>) {
    if matches!(client, Err(Error::Off)) {
        return (live, Err(Error::Off));
    }
    let builds = match &mut live {
        Source::Read(builds) => builds,
        Source::Capped(..) => return (live, Err(Error::Skipped(Skip::LiveCapped))),
        _ => return (live, Err(Error::Skipped(Skip::LiveFailed))),
    };
    let client = match client {
        Ok(client) => client,
        Err(error) => return (live, Err(error)),
    };
    let listing = match client.pipeline_runs(namespace, sha, until).await {
        Ok(listing) => listing,
        Err(error) => return (live, Err(error)),
    };
    let mut history = History {
        runs: listing.records.len(),
        cap: listing.cap,
        total: listing.total,
    };
    let mut tasks: BTreeMap<String, Result<Listing<TaskRun>, Error>> = BTreeMap::new();
    let mut logs: BTreeMap<String, Result<Listing<Log>, Error>> = BTreeMap::new();
    for Record { name, result, run } in listing.records {
        let existing = run
            .meta
            .uid
            .as_deref()
            .filter(|uid| !uid.is_empty())
            .and_then(|uid| {
                builds
                    .iter_mut()
                    .find(|build| build.run.meta.uid.as_deref() == Some(uid))
            });
        if let Some(build) = existing {
            build.archive.get_or_insert(Archive {
                record: name,
                result,
                logs: None,
            });
            continue;
        }
        if !tasks.contains_key(&result) {
            tasks.insert(result.clone(), client.task_runs(&result, until).await);
        }
        if !logs.contains_key(&result) {
            logs.insert(result.clone(), client.logs(&result, until).await);
        }
        let (read_tasks, tasks_unread) = tasks_of(&tasks[&result], &run, &mut history);
        builds.push(Build {
            origin: Origin::Results,
            found_by: FoundBy::Commit,
            archive: Some(Archive {
                record: name,
                result: result.clone(),
                logs: Some(logs[&result].clone()),
            }),
            run,
            tasks: read_tasks,
            tasks_unread,
        });
    }
    (live, Ok(history))
}

/// The TaskRuns of `run` out of its result's listing, and why some may be
/// missing.
fn tasks_of(
    listing: &Result<Listing<TaskRun>, Error>,
    run: &crate::tekton::PipelineRun,
    history: &mut History,
) -> (Vec<TaskRun>, Option<String>) {
    match listing {
        Ok(listing) => {
            if listing.cap == Some(Cap::Tasks) {
                history.cap = history.cap.or(Some(Cap::Tasks));
            }
            let read = listing
                .records
                .iter()
                .filter(|task| {
                    task.run.namespace == run.namespace
                        && task.run.pipeline_run.as_deref() == Some(run.name.as_str())
                })
                .map(|task| task.run.clone())
                .collect();
            (
                read,
                listing
                    .cap
                    .map(|cap| format!("more TaskRuns weren't read: {cap}")),
            )
        }
        Err(error) => (Vec::new(), Some(error.to_string())),
    }
}

/// Archived builds whose results report one of `digests`, found among the
/// namespace's latest [`MOST_RESULT_RUNS`](super::MOST_RESULT_RUNS) records
/// only: a result can't be selected on, so each record's run and TaskRuns are
/// compared in memory, and a build older than those is not found. Like
/// [`supplement`], it adds to a successful live read and never replaces a
/// live build.
pub(crate) async fn supplement_digests(
    mut live: Source<Vec<Build>>,
    client: Result<&Client, Error>,
    namespace: &str,
    digests: &[Digest],
    until: Instant,
) -> (Source<Vec<Build>>, Result<History, Error>) {
    if matches!(client, Err(Error::Off)) {
        return (live, Err(Error::Off));
    }
    let builds = match &mut live {
        Source::Read(builds) => builds,
        Source::Capped(..) => return (live, Err(Error::Skipped(Skip::LiveCapped))),
        _ => return (live, Err(Error::Skipped(Skip::LiveFailed))),
    };
    let client = match client {
        Ok(client) => client,
        Err(error) => return (live, Err(error)),
    };
    let listing = match client.first_pipeline_runs(namespace, until).await {
        Ok(listing) => listing,
        Err(error) => return (live, Err(error)),
    };
    let mut history = History {
        runs: listing.records.len(),
        cap: listing.cap,
        total: listing.total,
    };
    let reports = |results: &BTreeMap<String, String>| {
        built_images(results)
            .iter()
            .any(|image| digests.contains(&image.digest))
    };
    for Record { name, result, run } in listing.records {
        let known = run
            .meta
            .uid
            .as_deref()
            .filter(|uid| !uid.is_empty())
            .is_some_and(|uid| {
                builds
                    .iter()
                    .any(|build| build.run.meta.uid.as_deref() == Some(uid))
            });
        if known {
            continue;
        }
        let listed = client.task_runs(&result, until).await;
        let (tasks, tasks_unread) = tasks_of(&listed, &run, &mut history);
        if !reports(&run.results) && !tasks.iter().any(|task| reports(&task.results)) {
            continue;
        }
        let logs = client.logs(&result, until).await;
        builds.push(Build {
            origin: Origin::Results,
            found_by: FoundBy::Digest,
            archive: Some(Archive {
                record: name,
                result,
                logs: Some(logs),
            }),
            run,
            tasks,
            tasks_unread,
        });
    }
    (live, Ok(history))
}
