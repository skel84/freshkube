//! What the pull request card reads on demand: the pull request itself
//! (for who merged it), its reviews, and the checks and statuses of its head
//! commit. Four calls, none of them looked up by an address from an answer:
//! the head commit is the one the pull request's own answer names, checked as
//! a SHA before it is put in a path.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::types::{PullState, PullSummary, page, person, summary};
use super::{Client, Error, Handle, Identity, PER_PAGE, Person, Stale};
use crate::address::Address;
use crate::digest::{is_full_sha, text};

/// How long a merged pull request's reviews and checks stay fresh.
const MERGED_FOR: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub who: Person,
    pub state: ReviewState,
    pub submitted_at: Option<DateTime<Utc>>,
    /// The commit it was given on; a review of an older commit is older than
    /// the head.
    pub commit: Option<String>,
}

/// A check run's own state: how far it got, and how it ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    Queued,
    Running,
    Passed,
    Failed,
    /// Ended without passing or failing, by its own choice: neutral or
    /// skipped.
    Neutral,
    /// Stopped before it finished; it says nothing of the code.
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckRun {
    pub name: String,
    pub state: RunState,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    /// Opened in the browser only.
    pub page: Option<Address>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusState {
    Pending,
    Passed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusContext {
    pub context: String,
    pub state: StatusState,
    pub description: Option<String>,
    /// Opened in the browser only.
    pub page: Option<Address>,
}

/// Where the checks as a whole stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksState {
    None,
    Pending,
    Passing,
    Failing,
    /// Nothing failed and nothing is running, but nothing passed either, or
    /// a check was cancelled: it did not all run.
    Ended,
    /// Not all of them were read, and none read so far failed: the ones
    /// not read could.
    Partial,
}

/// What a list that stopped at its cap read, of how many there are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capped {
    pub read: usize,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checks {
    pub runs: Vec<CheckRun>,
    pub statuses: Vec<StatusContext>,
    pub state: ChecksState,
    /// The check runs that were not all read: "100 of 112 checks".
    pub capped: Option<Capped>,
    /// The commit statuses that were not all read, by the combined status's
    /// own count.
    pub statuses_capped: Option<Capped>,
}

/// How much a pull request changes, as `pulls/{n}` counts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Changed {
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
}

/// A label of the pull request. Its colour is six hexadecimal digits or
/// none, so it is only ever drawn, never followed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub name: String,
    pub color: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullDetail {
    pub summary: PullSummary,
    /// Lines and files changed, when the answer counted them. Linked issues
    /// are not in REST's `pulls/{n}` and are not read.
    pub changed: Option<Changed>,
    pub labels: Vec<Label>,
    /// The commit the merge made on the target branch, once merged.
    pub merge_commit: Option<String>,
    pub reviews: Vec<Review>,
    /// There may be more reviews than the first 100 that were read.
    pub reviews_capped: bool,
    pub checks: Checks,
    pub read_at: DateTime<Utc>,
    /// Set when any part is what was kept and not read now.
    pub stale: Option<Stale>,
}

/// Which read a pull request's detail is: whose answers (the client's
/// identity), which pull request and which head commit. A detail read for
/// another key is never shown as this one's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetailKey {
    pub identity: Identity,
    pub repo: String,
    pub number: u64,
    pub head: String,
}

impl DetailKey {
    /// The key for `summary` read through the client of `identity`; none
    /// when the summary names no head commit to look at.
    pub fn of(identity: &Identity, summary: &PullSummary) -> Option<Self> {
        let head = summary.head.clone().filter(|head| is_full_sha(head))?;
        Some(Self {
            identity: identity.clone(),
            repo: summary.repo.clone(),
            number: summary.number,
            head,
        })
    }
}

fn time(value: &Value, pointer: &str) -> Option<DateTime<Utc>> {
    text(value, pointer)
        .and_then(|at| DateTime::parse_from_rfc3339(&at).ok())
        .map(|at| at.with_timezone(&Utc))
}

/// The latest review of each person, in the order they first reviewed. A
/// comment only stands when it is all that person said: a later approval or
/// request for changes replaces it, and a later comment does not replace
/// either. Pending reviews are the reviewer's own and are not read.
pub(super) fn reviews(value: &Value) -> Vec<Review> {
    let mut kept: Vec<Review> = Vec::new();
    for review in value.as_array().into_iter().flatten().take(PER_PAGE) {
        let state = match text(review, "/state").as_deref() {
            Some("APPROVED") => ReviewState::Approved,
            Some("CHANGES_REQUESTED") => ReviewState::ChangesRequested,
            Some("COMMENTED") => ReviewState::Commented,
            Some("DISMISSED") => ReviewState::Dismissed,
            _ => continue,
        };
        let Some(who) = review.get("user").and_then(person) else {
            continue;
        };
        let parsed = Review {
            state,
            submitted_at: time(review, "/submitted_at"),
            commit: text(review, "/commit_id"),
            who,
        };
        match kept
            .iter_mut()
            .find(|held| held.who.login == parsed.who.login)
        {
            Some(_) if state == ReviewState::Commented => {}
            Some(held) => *held = parsed,
            None => kept.push(parsed),
        }
    }
    kept
}

fn run_state(run: &Value) -> RunState {
    match (
        text(run, "/status").as_deref(),
        text(run, "/conclusion").as_deref(),
    ) {
        (Some("completed"), Some("success")) => RunState::Passed,
        (
            Some("completed"),
            Some("failure" | "timed_out" | "action_required" | "startup_failure"),
        ) => RunState::Failed,
        (Some("completed"), Some("cancelled")) => RunState::Cancelled,
        (Some("completed"), _) => RunState::Neutral,
        (Some("in_progress"), _) => RunState::Running,
        _ => RunState::Queued,
    }
}

fn status_state(status: &Value) -> StatusState {
    match text(status, "/state").as_deref() {
        Some("success") => StatusState::Passed,
        Some("failure" | "error") => StatusState::Failed,
        _ => StatusState::Pending,
    }
}

fn capped(read: usize, total: Option<u64>) -> Option<Capped> {
    let total = usize::try_from(total?).ok()?;
    (total > read).then_some(Capped { read, total })
}

pub(super) fn checks(runs: &Value, statuses: &Value) -> Checks {
    let run_list: Vec<CheckRun> = runs
        .get("check_runs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(PER_PAGE)
        .filter_map(|run| {
            Some(CheckRun {
                name: text(run, "/name")?,
                state: run_state(run),
                started_at: time(run, "/started_at"),
                completed_at: time(run, "/completed_at"),
                page: text(run, "/html_url").and_then(|raw| page(&raw)),
            })
        })
        .collect();
    let status_list: Vec<StatusContext> = statuses
        .get("statuses")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(PER_PAGE)
        .filter_map(|status| {
            Some(StatusContext {
                context: text(status, "/context")?,
                state: status_state(status),
                description: text(status, "/description"),
                page: text(status, "/target_url").and_then(|raw| Address::parse(&raw).ok()),
            })
        })
        .collect();
    let failing = run_list.iter().any(|run| run.state == RunState::Failed)
        || status_list
            .iter()
            .any(|status| status.state == StatusState::Failed);
    let pending = run_list
        .iter()
        .any(|run| matches!(run.state, RunState::Queued | RunState::Running))
        || status_list
            .iter()
            .any(|status| status.state == StatusState::Pending);
    let passed = run_list.iter().any(|run| run.state == RunState::Passed)
        || status_list
            .iter()
            .any(|status| status.state == StatusState::Passed);
    let cancelled = run_list.iter().any(|run| run.state == RunState::Cancelled);
    let runs_capped = capped(
        run_list.len(),
        runs.get("total_count").and_then(Value::as_u64),
    );
    let statuses_capped = capped(
        status_list.len(),
        statuses.get("total_count").and_then(Value::as_u64),
    );
    // What was read says failing or pending whatever was not; passing needs
    // all of it read, and something to have passed.
    let state = if run_list.is_empty() && status_list.is_empty() {
        ChecksState::None
    } else if failing {
        ChecksState::Failing
    } else if pending {
        ChecksState::Pending
    } else if runs_capped.is_some() || statuses_capped.is_some() {
        ChecksState::Partial
    } else if cancelled || !passed {
        ChecksState::Ended
    } else {
        ChecksState::Passing
    };
    Checks {
        runs: run_list,
        statuses: status_list,
        state,
        capped: runs_capped,
        statuses_capped,
    }
}

/// What `pulls/{n}` says of the change's size, its labels and its merge
/// commit.
fn size_and_labels(pull: &Value, merged: bool) -> (Option<Changed>, Vec<Label>, Option<String>) {
    let count = |name: &str| pull.get(name).and_then(Value::as_u64);
    let changed = match (
        count("additions"),
        count("deletions"),
        count("changed_files"),
    ) {
        (Some(additions), Some(deletions), Some(files)) => Some(Changed {
            additions,
            deletions,
            files,
        }),
        _ => None,
    };
    let labels = pull
        .get("labels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(50)
        .filter_map(|label| {
            let name = text(label, "/name").filter(|name| name.chars().count() <= 100)?;
            let color = text(label, "/color")
                .filter(|color| color.len() == 6 && color.bytes().all(|b| b.is_ascii_hexdigit()));
            Some(Label { name, color })
        })
        .collect();
    let merge_commit = text(pull, "/merge_commit_sha").filter(|sha| merged && is_full_sha(sha));
    (changed, labels, merge_commit)
}

impl Client {
    /// The card's read: the pull request, its reviews, and its head
    /// commit's check runs and status. `spare` as for the list: stops short
    /// of asking when the limit is low.
    pub(super) async fn detail(
        &self,
        repo: &str,
        number: u64,
        spare: bool,
    ) -> Result<PullDetail, Error> {
        let open = self.open_for();
        let pull_path = format!("repos/{repo}/pulls/{number}");
        let (pull, pull_stale) = self.part(&pull_path, spare, move |_| Some(open)).await?;
        let parsed =
            summary(repo, &pull).ok_or(Error::Unreadable("an unexpected pull request shape"))?;
        let head = parsed
            .head
            .clone()
            .filter(|sha| is_full_sha(sha))
            .ok_or(Error::Unreadable("a pull request without a head commit"))?;
        // A merged pull request's reviews and checks no longer move.
        let merged = parsed.state == PullState::Merged;
        let keep = move |_: &Value| Some(if merged { MERGED_FOR } else { open });
        let reviews_path = format!("repos/{repo}/pulls/{number}/reviews?per_page={PER_PAGE}");
        let runs_path = format!("repos/{repo}/commits/{head}/check-runs?per_page={PER_PAGE}");
        let status_path = format!("repos/{repo}/commits/{head}/status?per_page={PER_PAGE}");
        let (reviews_body, reviews_stale) = self.part(&reviews_path, spare, keep).await?;
        let (runs_body, runs_stale) = self.part(&runs_path, spare, keep).await?;
        let (status_body, status_stale) = self.part(&status_path, spare, keep).await?;
        let stale = [pull_stale, reviews_stale, runs_stale, status_stale]
            .into_iter()
            .flatten()
            .min_by_key(|stale| stale.read_at);
        let (changed, labels, merge_commit) = size_and_labels(&pull, merged);
        let mut summary = parsed;
        summary.stale = stale.clone();
        Ok(PullDetail {
            summary,
            changed,
            labels,
            merge_commit,
            reviews: reviews(&reviews_body),
            reviews_capped: reviews_body
                .as_array()
                .is_some_and(|all| all.len() >= PER_PAGE),
            checks: checks(&runs_body, &status_body),
            read_at: stale.as_ref().map_or_else(Utc::now, |stale| stale.read_at),
            stale,
        })
    }

    /// One answer of the detail, or the kept one marked stale.
    async fn part(
        &self,
        path: &str,
        spare: bool,
        keep: impl Fn(&Value) -> Option<Duration>,
    ) -> Result<(std::sync::Arc<Value>, Option<Stale>), Error> {
        self.get(path, spare, keep)
            .await
            .map(|body| (body, None))
            .or_else(|error| self.or_kept(path, error))
    }
}

impl Handle {
    /// The card's read for `repo`'s pull request `number`.
    pub async fn detail(&self, repo: &str, number: u64) -> Result<PullDetail, Error> {
        super::super::check_repo(repo).map_err(|_| Error::Unreadable("the repository"))?;
        self.client.detail(repo, number, self.spare).await
    }
}
