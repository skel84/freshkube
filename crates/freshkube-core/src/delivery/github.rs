//! GitHub pull requests, read through the `gh` CLI with GET requests only.
//!
//! A pull request joins the rest of the chain on commit SHAs: its head
//! commit is what a pull-request build ran on, its merge commit is what a
//! push build on the target branch ran on. Nothing here comments, reviews,
//! merges or otherwise writes, and no token is read or printed: `gh` holds
//! its own credentials.

use serde_json::Value;

use super::digest::{is_full_sha, text};
use crate::resources::{Failure, FailureKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    pub title: Option<String>,
    pub state: Option<String>,
    pub merged: bool,
    pub head_sha: String,
    /// The commit the merge produced on the target branch; with a squash or
    /// rebase merge it is not the head commit.
    pub merge_sha: Option<String>,
    pub base_branch: Option<String>,
}

pub fn parse_pull_request(repo: &str, value: &Value) -> Option<PullRequest> {
    let merged = value.get("merged_at").is_some_and(|at| !at.is_null())
        || value
            .get("merged")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    Some(PullRequest {
        repo: repo.to_owned(),
        number: value.get("number")?.as_u64()?,
        title: text(value, "/title"),
        state: text(value, "/state"),
        merged,
        head_sha: text(value, "/head/sha")?,
        // GitHub fills this for an open PR too, with a throwaway test merge.
        merge_sha: text(value, "/merge_commit_sha").filter(|_| merged),
        base_branch: text(value, "/base/ref"),
    })
}

/// Where pull requests come from.
pub trait GitHub {
    /// The pull requests GitHub associates with a commit.
    fn pulls_for_commit(
        &self,
        repo: &str,
        sha: &str,
    ) -> impl Future<Output = Result<Vec<PullRequest>, Failure>>;
    fn pull(&self, repo: &str, number: u64) -> impl Future<Output = Result<PullRequest, Failure>>;
}

fn check_repo(repo: &str) -> Result<(), Failure> {
    let part = |part: &str| {
        !part.is_empty()
            && !matches!(part, "." | "..")
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    match repo.split_once('/') {
        Some((owner, name)) if part(owner) && part(name) => Ok(()),
        _ => Err(Failure::new(
            FailureKind::Other,
            "a repository is written owner/name",
        )),
    }
}

fn check_sha(sha: &str) -> Result<(), Failure> {
    if is_full_sha(sha) {
        Ok(())
    } else {
        Err(Failure::new(
            FailureKind::Other,
            "a commit SHA is 40 or 64 hexadecimal digits",
        ))
    }
}

/// The `gh` command line, and the only one that exists: `gh api` with an
/// explicit GET to a path built here from a checked repository and SHA or
/// number.
pub(super) fn gh_args(path: &str) -> Vec<String> {
    vec![
        "api".into(),
        "--method".into(),
        "GET".into(),
        path.to_owned(),
    ]
}

fn failure_from_gh(stderr: &str) -> Failure {
    // gh prints e.g. "gh: Not Found (HTTP 404)"; keep only that, never
    // anything about how it authenticated.
    let line = stderr
        .lines()
        .find(|line| line.contains("HTTP "))
        .unwrap_or("gh failed");
    let kind = if line.contains("HTTP 404") {
        FailureKind::NotFound
    } else if line.contains("HTTP 403") || line.contains("HTTP 401") {
        FailureKind::Forbidden
    } else {
        FailureKind::Other
    };
    Failure::new(kind, line.trim().to_owned())
}

/// Runs `gh` for the user, who has signed in to it themselves.
#[derive(Clone, Copy, Debug, Default)]
pub struct GhCli;

/// The most of `gh`'s error output kept; only its status line is used.
const MAX_GH_STDERR: u64 = 64 * 1024;

/// Reads at most `limit` bytes of `stream`, and one more to tell whether it
/// went on.
async fn read_capped(
    stream: Option<impl tokio::io::AsyncRead + Unpin>,
    limit: u64,
) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut out = Vec::new();
    if let Some(stream) = stream {
        stream.take(limit + 1).read_to_end(&mut out).await?;
    }
    Ok(out)
}

impl GhCli {
    async fn get(&self, path: String) -> Result<Value, Failure> {
        let not_run = || Failure::new(FailureKind::Other, "the gh CLI could not be run");
        let mut child = tokio::process::Command::new("gh")
            .args(gh_args(&path))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| not_run())?;
        let limit = super::read::MAX_BODY_BYTES as u64;
        let (stdout, stderr) = tokio::join!(
            read_capped(child.stdout.take(), limit),
            read_capped(child.stderr.take(), MAX_GH_STDERR),
        );
        let (stdout, stderr) = (
            stdout.map_err(|_| not_run())?,
            stderr.map_err(|_| not_run())?,
        );
        if stdout.len() as u64 > limit {
            return Err(Failure::new(
                FailureKind::Other,
                format!("gh answered more than {limit} bytes, which were not read"),
            ));
        }
        let status = child.wait().await.map_err(|_| not_run())?;
        if !status.success() {
            return Err(failure_from_gh(&String::from_utf8_lossy(&stderr)));
        }
        serde_json::from_slice(&stdout)
            .map_err(|_| Failure::new(FailureKind::Other, "gh answered something that is not JSON"))
    }
}

impl GitHub for GhCli {
    async fn pulls_for_commit(&self, repo: &str, sha: &str) -> Result<Vec<PullRequest>, Failure> {
        check_repo(repo)?;
        check_sha(sha)?;
        let body = self
            .get(format!("repos/{repo}/commits/{sha}/pulls?per_page=30"))
            .await?;
        Ok(body
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|pr| parse_pull_request(repo, pr))
            .collect())
    }

    async fn pull(&self, repo: &str, number: u64) -> Result<PullRequest, Failure> {
        check_repo(repo)?;
        let body = self.get(format!("repos/{repo}/pulls/{number}")).await?;
        parse_pull_request(repo, &body)
            .ok_or_else(|| Failure::new(FailureKind::Other, "an unexpected pull request shape"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_merge_commit_counts_only_once_merged() {
        let open = parse_pull_request(
            "acme/storefront",
            &json!({"number": 7, "state": "open", "head": {"sha": "1".repeat(40)},
                    "merge_commit_sha": "2".repeat(40), "merged_at": null}),
        )
        .unwrap();
        assert!(!open.merged);
        assert_eq!(open.merge_sha, None);
        let merged = parse_pull_request(
            "acme/storefront",
            &json!({"number": 7, "state": "closed", "head": {"sha": "1".repeat(40)},
                    "merge_commit_sha": "2".repeat(40), "merged_at": "2026-01-01T00:00:00Z",
                    "base": {"ref": "main"}}),
        )
        .unwrap();
        assert_eq!(merged.merge_sha, Some("2".repeat(40)));
        assert_eq!(merged.base_branch.as_deref(), Some("main"));
        assert!(parse_pull_request("r/r", &json!({"number": 1})).is_none());
    }

    #[test]
    fn the_only_command_is_an_explicit_get_of_a_checked_path() {
        assert_eq!(
            gh_args("repos/acme/storefront/pulls/7"),
            ["api", "--method", "GET", "repos/acme/storefront/pulls/7"]
        );
        assert!(check_repo("acme/storefront").is_ok());
        for bad in [
            "acme",
            "acme/store front",
            "../x/y",
            "a/b/c",
            "/x",
            "./storefront",
            "acme/..",
            "../..",
        ] {
            assert!(check_repo(bad).is_err(), "{bad}");
        }
        assert!(check_sha("abc,def").is_err());
        assert!(check_sha("abcdef1").is_err(), "a short SHA is refused");
        assert!(check_sha(&"a".repeat(40)).is_ok());
        assert!(check_sha(&"a".repeat(64)).is_ok());
    }

    #[tokio::test]
    async fn output_is_read_only_up_to_its_limit() {
        let long = read_capped(Some(&[7u8; 100][..]), 10).await.unwrap();
        assert_eq!(long.len(), 11, "one byte past the limit says it went on");
        let short = read_capped(Some(&[7u8; 5][..]), 10).await.unwrap();
        assert_eq!(short.len(), 5);
    }

    #[test]
    fn gh_errors_keep_only_the_status_line() {
        let failure = failure_from_gh("gh: Not Found (HTTP 404)\nsomething else");
        assert_eq!(failure.kind, FailureKind::NotFound);
        assert_eq!(failure.message, "gh: Not Found (HTTP 404)");
        assert_eq!(failure_from_gh("boom").kind, FailureKind::Other);
        assert_eq!(
            failure_from_gh("gh: Forbidden (HTTP 403)").kind,
            FailureKind::Forbidden
        );
    }
}
