//! Archived builds supplement a successful cluster read. The cluster's
//! manifest, tasks and outcome always win for an equal nonempty UID.
use std::collections::BTreeMap;

use tokio::time::Instant;

use super::{Cap, Client, Error, Listing, Log, Record, Skip};
use crate::source::Source;
use crate::tekton::{Build, Origin, TaskRun};

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
        let (read_tasks, tasks_unread) = match &tasks[&result] {
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
        };
        builds.push(Build {
            origin: Origin::Results,
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
