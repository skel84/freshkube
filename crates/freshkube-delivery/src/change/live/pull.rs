//! The pull request a change's commit belongs to: which of the ones GitHub
//! lists the change is named by, and what the change says when GitHub
//! couldn't be asked.

use crate::change::{Change, Gap, Missing};
use crate::github::{GitHub, Handle, PullRequest, PullSummary};
use crate::join::Evidence;
use crate::source::Source;
use freshkube_core::resources::{Failure, FailureKind};

/// Where pull requests are read from: GitHub's REST API when the app opened
/// a client for this setup, else nowhere.
pub(super) enum Pulls<'a> {
    Off,
    On(&'a Handle),
}

impl<'a> Pulls<'a> {
    pub fn of(github: Option<&'a Handle>) -> Self {
        github.map_or(Self::Off, Self::On)
    }
}

fn off() -> Failure {
    Failure::new(FailureKind::Other, "pull requests aren't read")
}

impl GitHub for Pulls<'_> {
    async fn pulls_for_commit(&self, repo: &str, sha: &str) -> Result<Vec<PullRequest>, Failure> {
        match self {
            Self::Off => Err(off()),
            Self::On(github) => github.pulls_for_commit(repo, sha).await,
        }
    }

    async fn pull(&self, repo: &str, number: u64) -> Result<PullRequest, Failure> {
        match self {
            Self::Off => Err(off()),
            Self::On(github) => github.pull(repo, number).await,
        }
    }
}

/// The pull request that names the change: the one a build's annotations
/// named (`number`), else the one `commit` is the merge commit of, else
/// the one it is the head of, else the first GitHub lists.
pub(super) fn chosen(
    pulls: &[PullRequest],
    commit: Option<&str>,
    number: Option<u64>,
) -> Option<PullSummary> {
    let is = |sha: &str| commit.is_some_and(|commit| commit.eq_ignore_ascii_case(sha));
    number
        .and_then(|number| pulls.iter().find(|pull| pull.number == number))
        .or_else(|| {
            pulls
                .iter()
                .find(|pull| pull.merge_sha.as_deref().is_some_and(is))
        })
        .or_else(|| pulls.iter().find(|pull| is(&pull.head_sha)))
        .or_else(|| pulls.first())
        .and_then(|pull| pull.summary.clone())
}

/// Puts the pull request on the change's origin, or why GitHub wasn't read
/// among what the change may be missing.
pub(super) fn attach(change: &mut Change, evidence: &Evidence) {
    match &evidence.pull_requests {
        None => {}
        Some(read) => match read.read() {
            Some(pulls) => {
                let summary = chosen(
                    pulls,
                    change.origin.commit.as_deref(),
                    change.origin.pull_request,
                );
                if let Some(summary) = summary {
                    if let Some(stale) = &summary.stale {
                        change.missing.push(Missing::new(
                            Gap::Unreadable,
                            format!(
                                "{}; showing what was read at {}",
                                stale.why,
                                stale.read_at.format("%H:%M UTC")
                            ),
                        ));
                    }
                    change.origin.pull_request.get_or_insert(summary.number);
                    change.origin.pull = Some(summary);
                }
            }
            None => {
                let (gap, why) = match read {
                    Source::Refused(why) => (Gap::Refused, why),
                    Source::NotInstalled(why) => (Gap::Unreadable, why),
                    Source::Unreadable(why) => (Gap::Unreadable, why),
                    Source::Read(_) | Source::Capped(..) => return,
                };
                change.missing.push(Missing::new(gap, why.clone()));
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{NEW, OLD, OTHER_SHA, SHA, freight};
    use crate::github::example;
    use crate::github::fake::{Reply, Server};
    use serde_json::json;

    use super::super::tests::{one_cluster, place};
    use super::super::{Place, read};
    use crate::tests::observed_at;

    fn pull(number: u64, head: &str, merge: Option<&str>) -> PullRequest {
        PullRequest {
            summary: example::pull(418).map(|mut summary| {
                summary.number = number;
                summary
            }),
            ..crate::fixtures::pull_request(number, head, merge)
        }
    }

    #[test]
    fn the_pull_request_that_names_the_change_is_chosen_in_order() {
        let (head, merge) = ("1".repeat(40), "2".repeat(40));
        let pulls = [
            pull(1, &"9".repeat(40), None),
            pull(2, &head, None),
            pull(3, &"8".repeat(40), Some(&merge)),
        ];
        let number = |commit, named| chosen(&pulls, commit, named).map(|pull| pull.number);
        assert_eq!(number(Some(&merge), None), Some(3), "its merge commit");
        assert_eq!(number(Some(&head), None), Some(2), "its head commit");
        assert_eq!(number(Some("abc"), None), Some(1), "else the first");
        assert_eq!(number(Some(&merge), Some(2)), Some(2), "what a build named");
        assert_eq!(number(None, Some(99)), Some(1), "a named one GitHub lacks");
        assert_eq!(chosen(&[], Some(&head), None), None);
    }

    /// A world whose Freight is a commit on GitHub, read with `github`.
    async fn change_on_github(github: Option<Handle>) -> Change {
        let reader = one_cluster(|world| {
            let on_github = |name: &str, digest: &str, commit: &str| {
                let mut value = freight(name, digest, commit);
                value["commits"][0]["repoURL"] = json!("https://github.com/acme/shop.git");
                value
            };
            world.kargo = std::mem::take(&mut world.kargo).with(
                "freights",
                vec![
                    on_github("f-new", NEW, SHA),
                    on_github("f-old", OLD, OTHER_SHA),
                ],
            );
        });
        let place = Place {
            github,
            ..place("f-new")
        };
        read(&reader, &place, observed_at()).await.unwrap()
    }

    fn pulls_of(sha: &str) -> serde_json::Value {
        json!([{
            "number": 418, "title": "Retry idempotency keys", "state": "closed",
            "merged_at": "2026-10-01T10:00:00Z", "merge_commit_sha": sha,
            "head": {"sha": "b".repeat(40)}, "base": {"ref": "main"},
            "html_url": "https://github.com/acme/shop/pull/418",
            "user": {"login": "ken"},
        }])
    }

    #[tokio::test]
    async fn a_followed_change_names_its_pull_request_and_where_it_stands() {
        let api = Server::new(|request| {
            assert!(
                request
                    .path
                    .starts_with(&format!("/repos/acme/shop/commits/{SHA}/pulls")),
                "{}",
                request.path
            );
            Reply::json(200, "", &pulls_of(SHA))
        });
        let change = change_on_github(Some(Handle::trusting(&api.address, None))).await;
        let pull = change.origin.pull.expect("GitHub named it");
        assert_eq!(
            (pull.number, pull.state, pull.repo.as_str()),
            (418, crate::github::PullState::Merged, "acme/shop")
        );
        assert_eq!(change.origin.pull_request, Some(418));
        assert!(change.missing.is_empty(), "{:?}", change.missing);
        assert_eq!(api.seen().len(), 1);
    }

    #[tokio::test]
    async fn no_client_or_no_github_repository_reads_nothing() {
        let api = Server::new(|_| Reply::json(200, "", &pulls_of(SHA)));
        let without = change_on_github(None).await;
        assert!(without.origin.pull.is_none() && without.missing.is_empty());
        // A Freight whose commit is elsewhere is no repository to read.
        let reader = one_cluster(|_| {});
        let place = Place {
            github: Some(Handle::trusting(&api.address, None)),
            ..place("f-new")
        };
        let change = read(&reader, &place, observed_at()).await.unwrap();
        assert!(change.origin.pull.is_none());
        assert!(api.seen().is_empty(), "nothing was asked of GitHub");
    }

    #[tokio::test]
    async fn a_refused_token_is_what_the_change_may_be_missing() {
        let api = Server::new(|_| Reply::empty(401, ""));
        let change = change_on_github(Some(Handle::trusting(&api.address, Some("ghp_x")))).await;
        assert!(change.origin.pull.is_none());
        assert!(
            change
                .missing
                .iter()
                .any(|missing| missing.text.contains("GitHub refused the token (401)")),
            "{:?}",
            change.missing
        );
    }
}
