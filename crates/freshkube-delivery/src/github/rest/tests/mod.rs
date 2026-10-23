use std::sync::Arc;

use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{Duration as Span, Utc};

use serde_json::{Value, json};

use freshkube_core::resources::FailureKind;

use super::detail::{Capped, StatusState};

use super::fake::{CERT, Reply, Request, Server};

use super::*;

const TOKEN: &str = "ghp_exampleToken0123456789";

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

fn client(api: &Server, token: Option<&str>) -> Handle {
    Handle::trusting(&api.address, token)
}

fn pull(number: u64) -> Value {
    json!({
        "number": number, "title": "Add retries", "state": "open",
        "html_url": format!("https://github.com/acme/shop/pull/{number}"),
        "head": {"sha": SHA}, "base": {"ref": "main"},
        "user": {"login": "mira", "avatar_url": "https://avatars.githubusercontent.com/u/7?v=4"},
    })
}

fn limit_headers(remaining: u32, reset: i64) -> String {
    format!(
        "x-ratelimit-limit: 5000\r\nx-ratelimit-remaining: {remaining}\r\nx-ratelimit-reset: {reset}\r\n"
    )
}

fn in_an_hour() -> i64 {
    (Utc::now() + Span::hours(1)).timestamp()
}

fn ok(pulls: Vec<Value>) -> impl Fn(&Request) -> Reply + Send + Sync + 'static {
    move |_| {
        Reply::json(
            200,
            &limit_headers(4000, in_an_hour()),
            &Value::Array(pulls.clone()),
        )
    }
}

async fn ask(handle: &Handle) -> Result<Vec<PullRequest>, Failure> {
    handle.pulls_for_commit("acme/shop", SHA).await
}

fn client_identity() -> Identity {
    Identity {
        api: Url::parse(API).unwrap(),
        account: None,
        setup_revision: 0,
    }
}

struct Keys(Mutex<Vec<String>>, Option<String>);

impl freshkube_core::secrets::SecretStore for Keys {
    fn read(&self, account: &str) -> Result<Option<String>, String> {
        self.0.lock().unwrap().push(account.to_owned());
        Ok(self.1.clone())
    }
    fn write(&self, _: &str, _: &str) -> Result<(), String> {
        unreachable!()
    }
    fn forget(&self, _: &str) -> Result<(), String> {
        unreachable!()
    }
}

fn keys(token: Option<&str>) -> (Arc<Keys>, Secrets) {
    let keys = Arc::new(Keys(Mutex::default(), token.map(str::to_owned)));
    let secrets: Secrets = keys.clone();
    (keys, secrets)
}

/// A `gh` in a temporary folder that runs `script`, and the folder.
#[cfg(unix)]
fn fake_gh(script: &str) -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gh");
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

const HEAD: &str = "b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0";

fn review(login: &str, state: &str, at: &str) -> Value {
    json!({"user": {"login": login}, "state": state, "submitted_at": at, "commit_id": HEAD})
}

/// A fake that answers the card's four reads for pull request 418.
fn card(extra: impl Fn(&Request) -> Option<Reply> + Send + Sync + 'static) -> Server {
    Server::new(move |request| {
        if let Some(reply) = extra(request) {
            return reply;
        }
        let limits = limit_headers(4000, in_an_hour());
        let path = request.path.split('?').next().unwrap();
        match path {
            "/repos/acme/shop/pulls/418" => {
                let mut pull = pull(418);
                pull["head"]["sha"] = json!(HEAD);
                pull["state"] = json!("closed");
                pull["merged_at"] = json!("2026-10-01T10:00:00Z");
                pull["merged_by"] = json!({"login": "ken"});
                pull["merge_commit_sha"] = json!("e".repeat(40));
                pull["additions"] = json!(84);
                pull["deletions"] = json!(12);
                pull["changed_files"] = json!(5);
                pull["labels"] = json!([
                    {"name": "reliability", "color": "0e8a16"},
                    {"name": "odd", "color": "not-a-colour"},
                ]);
                Reply::json(200, &limits, &pull)
            }
            "/repos/acme/shop/pulls/418/reviews" => Reply::json(
                200,
                &limits,
                &json!([
                    review("ken", "COMMENTED", "2026-09-30T09:00:00Z"),
                    review("ken", "CHANGES_REQUESTED", "2026-09-30T10:00:00Z"),
                    review("ken", "COMMENTED", "2026-09-30T11:00:00Z"),
                    review("ken", "APPROVED", "2026-09-30T12:00:00Z"),
                    review("zoe", "COMMENTED", "2026-09-30T12:30:00Z"),
                    review("zoe", "PENDING", "2026-09-30T12:40:00Z"),
                    review("ana", "APPROVED", "2026-09-30T13:00:00Z"),
                    review("ana", "DISMISSED", "2026-09-30T14:00:00Z"),
                ]),
            ),
            p if p == format!("/repos/acme/shop/commits/{HEAD}/check-runs") => Reply::json(
                200,
                &limits,
                &json!({"total_count": 112, "check_runs": [
                    {"name": "build", "status": "completed", "conclusion": "success",
                     "started_at": "2026-09-30T09:00:00Z", "completed_at": "2026-09-30T09:05:00Z",
                     "html_url": "https://github.com/acme/shop/runs/1"},
                    {"name": "lint", "status": "in_progress", "conclusion": null},
                    {"name": "e2e", "status": "completed", "conclusion": "skipped",
                     "html_url": "https://ci.example.test/elsewhere"},
                ]}),
            ),
            p if p == format!("/repos/acme/shop/commits/{HEAD}/status") => Reply::json(
                200,
                &limits,
                &json!({"state": "success", "total_count": 1, "statuses": [
                    {"context": "ci/deploy-preview", "state": "success",
                     "description": "Ready", "target_url": "https://preview.example.test/418"}]}),
            ),
            // Anything the card is not meant to ask for is not found, and
            // the test sees it in the requests the server recorded.
            _ => Reply::empty(404, ""),
        }
    })
}

mod cache;
mod cards;
mod checks;
mod gh_cli;
mod limits;
mod requests;
mod tokens;
