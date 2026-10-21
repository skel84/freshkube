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

#[tokio::test]
async fn every_request_carries_the_headers_and_only_a_token_adds_authorization() {
    let api = Server::new(ok(vec![pull(418)]));
    let anonymous = client(&api, None);
    let pulls = ask(&anonymous).await.unwrap();
    assert_eq!(pulls.len(), 1);
    let summary = pulls[0].summary.as_ref().unwrap();
    assert_eq!(summary.state, PullState::Open);
    assert_eq!(summary.author.as_ref().unwrap().login, "mira");
    let seen = api.seen();
    let request = &seen[0];
    assert!(request.head().starts_with("GET /repos/acme/shop/commits/"));
    assert!(
        request.path.ends_with("/pulls?per_page=100"),
        "{}",
        request.path
    );
    assert_eq!(
        request.header("accept").as_deref(),
        Some("application/vnd.github+json")
    );
    assert_eq!(
        request.header("x-github-api-version").as_deref(),
        Some("2022-11-28")
    );
    assert_eq!(
        request.header("user-agent"),
        Some(format!("freshkube/{}", env!("CARGO_PKG_VERSION")))
    );
    assert_eq!(request.header("authorization"), None, "no token, no header");
    assert_eq!(request.header("if-none-match"), None);

    let authenticated = client(&api, Some(TOKEN));
    ask(&authenticated).await.unwrap();
    assert_eq!(
        api.seen()[1].header("authorization").as_deref(),
        Some(&*format!("Bearer {TOKEN}"))
    );
}

#[tokio::test]
async fn the_token_reaches_the_api_host_and_no_address_in_an_answer_is_fetched() {
    let avatars = Server::new(|_| Reply::empty(200, ""));
    let elsewhere = avatars.address.clone();
    // Every address the answer carries points at the second host.
    let api = Server::new({
        let elsewhere = elsewhere.clone();
        move |_| {
            let mut pull = pull(418);
            pull["html_url"] = json!(format!("{elsewhere}/pull/418"));
            pull["user"]["avatar_url"] = json!(format!("{elsewhere}/u/7"));
            pull["statuses_url"] = json!(format!("{elsewhere}/statuses"));
            Reply::json(
                200,
                &format!(
                    "{}link: <{elsewhere}/next>; rel=\"next\"\r\n",
                    limit_headers(4000, in_an_hour())
                ),
                &json!([pull]),
            )
        }
    });
    let handle = client(&api, Some(TOKEN));
    let pulls = ask(&handle).await.unwrap();
    let summary = pulls[0].summary.as_ref().unwrap();
    assert!(summary.page.is_none(), "a page off github.com is dropped");
    assert!(summary.author.as_ref().unwrap().avatar.is_none());
    assert_eq!(api.seen().len(), 1, "the Link header was not followed");
    assert!(
        avatars.seen().is_empty(),
        "nothing in the answer was fetched, with or without the token"
    );
}

#[tokio::test]
async fn a_redirect_is_an_answer_and_is_not_followed() {
    let avatars = Server::new(|_| Reply::empty(200, ""));
    let target = avatars.address.clone();
    let api = Server::new(move |_| {
        Reply::empty(301, &format!("location: {target}/repos/acme/new/pulls\r\n"))
    });
    let handle = client(&api, Some(TOKEN));
    let failure = ask(&handle).await.unwrap_err();
    assert_eq!(failure.message, "GitHub says the repository moved (301)");
    assert_eq!(api.seen().len(), 1);
    assert!(
        avatars.seen().is_empty(),
        "the redirect's target saw nothing"
    );
    assert_eq!(
        Error::Moved.words("acme/shop"),
        "GitHub says acme/shop moved"
    );
}

#[tokio::test]
async fn a_refused_token_never_falls_back_to_an_anonymous_request() {
    let api = Server::new(|_| Reply::empty(401, ""));
    let handle = client(&api, Some(TOKEN));
    let failure = ask(&handle).await.unwrap_err();
    assert_eq!(failure.message, "GitHub refused the token (401)");
    assert_eq!(failure.kind, FailureKind::Unauthorized);
    // Asked again, and for another commit, it asks with the token again.
    let _ = handle
        .pulls_for_commit("acme/shop", &"a".repeat(40))
        .await
        .unwrap_err();
    let seen = api.seen();
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().all(|request| {
        request
            .header("authorization")
            .is_some_and(|value| value == format!("Bearer {TOKEN}"))
    }));
    assert_eq!(
        Error::Unauthorized(GitHubToken::KeysJson).words("acme/shop"),
        "GitHub refused the token: it may have expired. It is the keys.json entry GitHub https://api.github.com/"
    );
}

#[tokio::test]
async fn refusals_and_missing_repositories_are_worded_as_the_design_says() {
    for (status, token, card) in [
        (
            403,
            Some(TOKEN),
            "GitHub refused reading acme/shop (403): the token may not be granted this repository",
        ),
        (
            404,
            None,
            "acme/shop wasn't found. If it's private, add a GitHub token",
        ),
        (
            404,
            Some(TOKEN),
            "acme/shop wasn't found, or the token can't see it",
        ),
    ] {
        let api = Server::new(move |_| Reply::empty(status, ""));
        let error = client(&api, token)
            .client
            .pulls("acme/shop", SHA, false)
            .await
            .unwrap_err();
        assert_eq!(error.words("acme/shop"), card);
    }
}

#[tokio::test]
async fn a_304_reuses_the_kept_answer_and_sends_if_none_match_with_a_token() {
    let calls = Arc::new(AtomicUsize::new(0));
    let api = Server::new({
        let calls = calls.clone();
        move |request| {
            calls.fetch_add(1, Ordering::SeqCst);
            match request.header("if-none-match") {
                Some(etag) if etag == "\"v1\"" => {
                    Reply::empty(304, &limit_headers(4999, in_an_hour()))
                }
                _ => Reply::json(
                    200,
                    &format!("etag: \"v1\"\r\n{}", limit_headers(4000, in_an_hour())),
                    &json!([pull(418)]),
                ),
            }
        }
    });
    let handle = client(&api, Some(TOKEN));
    let first = ask(&handle).await.unwrap();
    // Fresh: no request at all.
    ask(&handle).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // Past its minute, it asks again and the 304 hands back the same answer.
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(61));
    let again = ask(&handle).await.unwrap();
    assert_eq!(first, again);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        api.seen()[1].header("if-none-match").as_deref(),
        Some("\"v1\"")
    );
    // The 304 started the expiry again.
    ask(&handle).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn without_a_token_nothing_is_revalidated_and_the_answer_lives_fifteen_minutes() {
    let api = Server::new(|request| {
        assert_eq!(request.header("if-none-match"), None);
        Reply::json(200, "etag: \"v1\"\r\n", &json!([pull(418)]))
    });
    let handle = client(&api, None);
    ask(&handle).await.unwrap();
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(14 * 60));
    ask(&handle).await.unwrap();
    assert_eq!(api.seen().len(), 1, "still fresh at 14 minutes");
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(2 * 60));
    ask(&handle).await.unwrap();
    assert_eq!(api.seen().len(), 2);
}

#[tokio::test]
async fn a_commit_in_a_merged_pull_request_is_kept_for_the_clients_life() {
    let merged = json!({
        "number": 7, "state": "closed", "merged_at": "2026-10-01T10:00:00Z",
        "merge_commit_sha": SHA, "head": {"sha": "b".repeat(40)},
    });
    let api = Server::new(ok(vec![merged]));
    let handle = client(&api, None);
    ask(&handle).await.unwrap();
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(30 * 24 * 3600));
    let pulls = ask(&handle).await.unwrap();
    assert_eq!(pulls[0].summary.as_ref().unwrap().state, PullState::Merged);
    assert_eq!(api.seen().len(), 1);
}

#[tokio::test]
async fn no_requests_are_left_until_the_reset_when_none_remain() {
    let api =
        Server::new(|_| Reply::json(200, &limit_headers(0, in_an_hour()), &json!([pull(418)])));
    let handle = client(&api, Some(TOKEN));
    ask(&handle).await.unwrap();
    let other = "b".repeat(40);
    let failure = handle
        .pulls_for_commit("acme/shop", &other)
        .await
        .unwrap_err();
    assert!(
        failure
            .message
            .starts_with("GitHub's rate limit is reached until "),
        "{}",
        failure.message
    );
    assert_eq!(api.seen().len(), 1, "nothing was sent while blocked");
    // Once the reset has passed, it asks again.
    {
        let mut state = handle.client.state.lock().unwrap();
        state.budget = Budget::default();
    }
    handle.pulls_for_commit("acme/shop", &other).await.unwrap();
    assert_eq!(api.seen().len(), 2);
}

#[tokio::test]
async fn a_403_with_nothing_left_blocks_until_the_reset() {
    let reset = in_an_hour();
    let api = Server::new(move |_| Reply::empty(403, &limit_headers(0, reset)));
    let handle = client(&api, None);
    let error = handle
        .client
        .pulls("acme/shop", SHA, false)
        .await
        .unwrap_err();
    let Error::Limited { until, secondary } = error else {
        panic!("{error:?}")
    };
    assert_eq!(until.timestamp(), reset);
    assert!(!secondary);
    handle
        .client
        .pulls("acme/shop", SHA, false)
        .await
        .unwrap_err();
    assert_eq!(api.seen().len(), 1);
}

#[tokio::test]
async fn retry_after_blocks_for_that_long_and_a_429_without_it_for_a_minute() {
    for (headers, status, seconds) in [
        ("retry-after: 120\r\n", 403, 120),
        ("retry-after: 30\r\n", 429, 30),
        ("", 429, 60),
    ] {
        let api = Server::new(move |_| Reply::empty(status, headers));
        let handle = client(&api, Some(TOKEN));
        let before = Utc::now();
        let error = handle
            .client
            .pulls("acme/shop", SHA, false)
            .await
            .unwrap_err();
        let Error::Limited { until, secondary } = error else {
            panic!("{error:?}")
        };
        assert!(secondary);
        let wait = (until - before).num_seconds();
        assert!(
            (seconds - 1..=seconds + 2).contains(&wait),
            "{wait} for {seconds}"
        );
        handle
            .client
            .pulls("acme/shop", SHA, false)
            .await
            .unwrap_err();
        assert_eq!(api.seen().len(), 1, "nothing was sent while blocked");
    }
    // A 403 with neither header is a refusal, not a limit.
    let api = Server::new(|_| Reply::empty(403, ""));
    let error = client(&api, None)
        .client
        .pulls("acme/shop", SHA, false)
        .await;
    assert_eq!(error.unwrap_err(), Error::Forbidden);
}

#[tokio::test]
async fn the_list_stops_looking_when_the_limit_is_low_and_the_page_still_asks() {
    let api =
        Server::new(|_| Reply::json(200, &limit_headers(8, in_an_hour()), &json!([pull(418)])));
    let page = client(&api, None);
    ask(&page).await.unwrap();
    let list = page.sparing();
    let other = "c".repeat(40);
    let failure = list
        .pulls_for_commit("acme/shop", &other)
        .await
        .unwrap_err();
    assert!(
        failure
            .message
            .starts_with("GitHub's limit is low: 8 left until "),
        "{}",
        failure.message
    );
    assert_eq!(api.seen().len(), 1);
    // The page asked for this change; its read goes out.
    page.pulls_for_commit("acme/shop", &other).await.unwrap();
    assert_eq!(api.seen().len(), 2);
    // What was already read is still free to the list.
    ask(&list).await.unwrap();
    assert_eq!(api.seen().len(), 2);
    assert_eq!(page.rate_limit().unwrap().remaining, 8);
}

#[tokio::test]
async fn a_limit_or_a_silent_host_leaves_the_last_answer_marked_stale() {
    // The first read answers, then every read is limited.
    let calls = Arc::new(AtomicUsize::new(0));
    let api = Server::new({
        let calls = calls.clone();
        move |_| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Reply::json(200, &limit_headers(4000, in_an_hour()), &json!([pull(418)]))
            } else {
                Reply::empty(429, "retry-after: 300\r\n")
            }
        }
    });
    let handle = client(&api, Some(TOKEN));
    assert!(
        ask(&handle).await.unwrap()[0]
            .summary
            .as_ref()
            .unwrap()
            .stale
            .is_none()
    );
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(120));
    let pulls = ask(&handle).await.unwrap();
    let stale = pulls[0].summary.as_ref().unwrap().stale.as_ref().unwrap();
    assert!(
        stale
            .why
            .starts_with("GitHub's secondary rate limit is reached until ")
    );
    // What was never read is an error, not a guess.
    let other = "d".repeat(40);
    assert!(handle.pulls_for_commit("acme/shop", &other).await.is_err());
    // A host that no longer answers leaves it too.
    let dead = Server::new(ok(vec![pull(418)]));
    let handle = client(&dead, Some(TOKEN));
    ask(&handle).await.unwrap();
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(120));
    drop(dead);
    let pulls = ask(&handle).await.unwrap();
    assert_eq!(
        pulls[0]
            .summary
            .as_ref()
            .unwrap()
            .stale
            .as_ref()
            .unwrap()
            .why,
        "GitHub didn't answer: could not connect"
    );
}

#[tokio::test]
async fn a_list_of_pull_requests_is_capped_at_one_hundred() {
    let api = Server::new(ok((1..=120).map(pull).collect()));
    let pulls = ask(&client(&api, None)).await.unwrap();
    assert_eq!(pulls.len(), 100);
    assert_eq!(pulls[99].number, 100);
}

#[tokio::test]
async fn an_unreachable_host_is_worded_without_its_address() {
    let api = Server::new(ok(vec![]));
    let handle = client(&api, Some(TOKEN));
    drop(api);
    // A dead port: nothing listens.
    let failure = ask(&handle).await.unwrap_err();
    assert_eq!(failure.message, "GitHub didn't answer: could not connect");
    assert!(!failure.message.contains("127.0.0.1"));
}

#[test]
fn a_stored_token_must_be_a_bearer_token() {
    assert!(token_header(TOKEN).is_ok());
    assert!(token_header("github_pat_11AAAA_bbbb").is_ok());
    for bad in [
        "",
        "  ",
        "has space",
        "new\nline",
        "a\"b",
        &"a".repeat(8193),
    ] {
        assert!(token_header(bad).is_err(), "{bad:?}");
    }
    assert!(token_header(TOKEN).unwrap().is_sensitive());
}

#[test]
fn a_handle_prints_whose_it_is_and_never_the_token() {
    let root = reqwest::Certificate::from_pem(CERT).unwrap();
    let client = Client::build(
        reqwest::Client::builder().add_root_certificate(root),
        Url::parse("https://127.0.0.1:1/").unwrap(),
        Some((TOKEN.into(), GitHubToken::KeysJson)),
        3,
    )
    .unwrap();
    assert_eq!(client.identity().account.as_deref(), Some(ACCOUNT));
    assert_eq!(client.identity().setup_revision, 3);
    let printed = format!(
        "{:?} {:?}",
        Handle::new(client).sparing(),
        client_identity()
    );
    assert!(!printed.contains(TOKEN), "{printed}");
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

#[tokio::test]
async fn keys_json_is_read_by_its_account_when_it_is_named() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (store, secrets) = keys(Some(TOKEN));
    let client = Client::open(GitHubToken::KeysJson, Some(secrets), 5)
        .await
        .unwrap();
    assert!(client.authenticated());
    assert_eq!(*store.0.lock().unwrap(), [ACCOUNT]);
    assert_eq!(client.identity().api.as_str(), API);
    assert_eq!(client.identity().account.as_deref(), Some(ACCOUNT));
    // Named, and the store is not there, or holds nothing, or holds what is
    // not a token: nothing is read, and the cause says which.
    for (secrets, cause) in [
        (None, NotSetUp::KeysUnavailable),
        (Some(keys(None).1), NotSetUp::NoSavedToken),
        (Some(keys(Some("bad token")).1), NotSetUp::TokenUnreadable),
        (Some(keys(Some("")).1), NotSetUp::TokenUnreadable),
    ] {
        let opened = Client::open(GitHubToken::KeysJson, secrets, 5).await;
        assert_eq!(opened.err(), Some(Error::NotSetUp(cause)), "{cause}");
    }
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

#[cfg(unix)]
#[tokio::test]
async fn gh_gives_the_token_which_reaches_the_api_host_alone() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let place = tempfile::tempdir().unwrap();
    let args = place.path().join("args");
    let gh = fake_gh(&format!(
        "echo \"$@\" > {args}\necho ghp_fromGhExample0123456789",
        args = args.display()
    ));
    let dirs = [gh.path().to_owned()];
    let token = Client::token_of(GitHubToken::Gh, None, &dirs)
        .await
        .unwrap();
    assert_eq!(token, "ghp_fromGhExample0123456789");
    // Pinned to github.com, so the token is that host's.
    assert_eq!(
        std::fs::read_to_string(&args).unwrap().trim(),
        "auth token --hostname github.com"
    );
    // The client built from it sends it to the API host and to no other.
    let avatars = Server::new(|_| Reply::empty(200, ""));
    let elsewhere = avatars.address.clone();
    let api = Server::new(move |_| {
        let mut pull = pull(418);
        pull["user"]["avatar_url"] = json!(format!("{elsewhere}/u/7"));
        Reply::json(200, "", &json!([pull]))
    });
    let handle = Handle::trusting_as(&api.address, Some(&token), GitHubToken::Gh);
    assert_eq!(handle.identity().account.as_deref(), Some(GH_ACCOUNT));
    ask(&handle).await.unwrap();
    assert_eq!(
        api.seen()[0].header("authorization").as_deref(),
        Some("Bearer ghp_fromGhExample0123456789")
    );
    assert!(avatars.seen().is_empty());
    // Refused, it names gh, not keys.json.
    let refusing = Server::new(|_| Reply::empty(401, ""));
    let handle = Handle::trusting_as(&refusing.address, Some(&token), GitHubToken::Gh);
    let error = handle
        .client
        .pulls("acme/shop", SHA, false)
        .await
        .unwrap_err();
    assert_eq!(error, Error::Unauthorized(GitHubToken::Gh));
    assert!(error.words("acme/shop").contains("gh auth login"));
    assert!(!error.words("acme/shop").contains("keys.json"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_missing_gh_is_not_set_up_and_says_how_to() {
    let empty = tempfile::tempdir().unwrap();
    let opened = Client::open_with(GitHubToken::Gh, None, 1, &[empty.path().to_owned()]).await;
    let error = opened.err().unwrap();
    assert_eq!(error, Error::NotSetUp(NotSetUp::GhNotFound));
    let words = error.to_string();
    assert!(
        words.starts_with("GitHub isn't set up: gh not found"),
        "{words}"
    );
    assert!(words.contains("keys.json"), "the next step: {words}");
    // A folder with a `gh` that can't be run is not found either.
    let plain = tempfile::tempdir().unwrap();
    std::fs::write(plain.path().join("gh"), "not executable").unwrap();
    let opened = Client::open_with(GitHubToken::Gh, None, 1, &[plain.path().to_owned()]).await;
    assert_eq!(opened.err(), Some(Error::NotSetUp(NotSetUp::GhNotFound)));
}

#[cfg(unix)]
#[tokio::test]
async fn a_gh_that_fails_says_it_is_not_signed_in_and_shows_none_of_its_output() {
    let gh = fake_gh("echo 'secret-stderr-text ghp_leaked0123456789' >&2\nexit 1");
    let dirs = [gh.path().to_owned()];
    let opened = Client::open_with(GitHubToken::Gh, None, 1, &dirs).await;
    let error = opened.err().unwrap();
    assert_eq!(error, Error::NotSetUp(NotSetUp::GhNotSignedIn));
    for shown in [
        error.to_string(),
        format!("{error:?}"),
        error.words("acme/shop"),
    ] {
        assert!(
            !shown.contains("secret-stderr-text") && !shown.contains("ghp_leaked"),
            "{shown}"
        );
    }
    assert!(error.to_string().contains("gh auth login"));
    // Even one that succeeds and writes to standard error is not read.
    let noisy = fake_gh("echo 'warning: something' >&2\necho ghp_fine0123456789");
    let token = Client::token_of(GitHubToken::Gh, None, &[noisy.path().to_owned()])
        .await
        .unwrap();
    assert_eq!(token, "ghp_fine0123456789");
    // What gh prints that is not a token is not used.
    let odd = fake_gh("echo 'not a token'");
    let opened = Client::open_with(GitHubToken::Gh, None, 1, &[odd.path().to_owned()]).await;
    assert_eq!(
        opened.err(),
        Some(Error::NotSetUp(NotSetUp::TokenUnreadable))
    );
    // An empty answer is not signed in.
    let empty = fake_gh("true");
    let opened = Client::open_with(GitHubToken::Gh, None, 1, &[empty.path().to_owned()]).await;
    assert_eq!(opened.err(), Some(Error::NotSetUp(NotSetUp::GhNotSignedIn)));
}

#[cfg(unix)]
#[tokio::test]
async fn a_gh_that_takes_too_long_is_stopped() {
    let gh = fake_gh("sleep 30");
    let started = std::time::Instant::now();
    let token = super::gh::token(&[gh.path().to_owned()], Duration::from_millis(200)).await;
    assert_eq!(token, Err(NotSetUp::GhSlow));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[cfg(unix)]
#[tokio::test]
async fn gh_runs_only_when_the_setup_names_it() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let place = tempfile::tempdir().unwrap();
    let marker = place.path().join("ran");
    let gh = fake_gh(&format!("touch {}\necho ghp_x0123456789", marker.display()));
    let dirs = [gh.path().to_owned()];
    // keys.json named, in every state: gh is not asked.
    let (_, secrets) = keys(Some(TOKEN));
    Client::open_with(GitHubToken::KeysJson, Some(secrets), 1, &dirs)
        .await
        .unwrap();
    let missing = Client::open_with(GitHubToken::KeysJson, Some(keys(None).1), 1, &dirs).await;
    assert!(missing.is_err(), "no saved token is not a reason to run gh");
    // Neither named: an anonymous client exists at once and gh is not asked.
    let anonymous = Client::anonymous(1).unwrap();
    assert!(!anonymous.authenticated());
    assert_eq!(anonymous.identity().account, None);
    assert!(!marker.exists(), "gh was not run");
    // Named, it is.
    Client::open_with(GitHubToken::Gh, None, 1, &dirs)
        .await
        .unwrap();
    assert!(marker.exists());
}

#[test]
fn gh_is_looked_for_in_the_usual_places_before_path() {
    let dirs = super::gh::default_dirs();
    assert_eq!(dirs[0], std::path::Path::new("/opt/homebrew/bin"));
    assert_eq!(dirs[1], std::path::Path::new("/usr/local/bin"));
    if let Some(path) = std::env::var_os("PATH") {
        let rest: Vec<_> = std::env::split_paths(&path).collect();
        assert_eq!(dirs[2..], rest[..]);
    }
}

#[tokio::test]
async fn a_304_for_an_answer_the_memo_dropped_reads_it_again() {
    // The server forgets nothing, but the client's memo loses the entry
    // between asking with its validator and the 304 coming back, as the cap
    // can make it.
    let held: Arc<Mutex<Option<Handle>>> = Arc::default();
    let api = Server::new({
        let held = held.clone();
        move |request| match request.header("if-none-match") {
            Some(_) => {
                let handle = held.lock().unwrap().clone().unwrap();
                handle.client.state.lock().unwrap().memo = memo::Memo::default();
                Reply::empty(304, &limit_headers(4999, in_an_hour()))
            }
            None => Reply::json(
                200,
                &format!("etag: \"v1\"\r\n{}", limit_headers(4000, in_an_hour())),
                &json!([pull(418)]),
            ),
        }
    });
    let handle = client(&api, Some(TOKEN));
    *held.lock().unwrap() = Some(handle.clone());
    ask(&handle).await.unwrap();
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(120));
    let again = ask(&handle).await.unwrap();
    assert_eq!(again.len(), 1);
    let seen = api.seen();
    assert_eq!(seen.len(), 3, "the read, the validation, the read again");
    assert!(seen[1].header("if-none-match").is_some());
    assert_eq!(seen[2].header("if-none-match"), None);
}

#[tokio::test]
async fn a_limit_reached_or_nearly_spent_is_not_a_refusal() {
    use freshkube_core::resources::FailureKind;
    let until = Utc::now() + Span::hours(1);
    for error in [
        Error::Limited {
            until,
            secondary: false,
        },
        Error::Limited {
            until,
            secondary: true,
        },
        Error::Low { left: 8, until },
    ] {
        let failure: Failure = error.clone().into();
        assert_eq!(failure.kind, FailureKind::Other, "{error}");
        let source = crate::source::Source::<()>::from_failure(failure);
        assert!(
            matches!(source, crate::source::Source::Unreadable(_)),
            "{error}"
        );
    }
    let refused: Failure = Error::Forbidden.into();
    assert_eq!(refused.kind, FailureKind::Forbidden);
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
            other => panic!("an unexpected read: {other}"),
        }
    })
}

#[tokio::test]
async fn the_card_reads_four_things_and_nothing_else() {
    let api = card(|_| None);
    let detail = client(&api, Some(TOKEN))
        .detail("acme/shop", 418)
        .await
        .unwrap();
    let paths: Vec<String> = api
        .seen()
        .iter()
        .map(|request| request.path.clone())
        .collect();
    assert_eq!(
        paths,
        [
            "/repos/acme/shop/pulls/418".to_owned(),
            "/repos/acme/shop/pulls/418/reviews?per_page=100".to_owned(),
            format!("/repos/acme/shop/commits/{HEAD}/check-runs?per_page=100"),
            format!("/repos/acme/shop/commits/{HEAD}/status?per_page=100"),
        ]
    );
    assert!(api.seen().iter().all(|request| {
        request
            .header("authorization")
            .is_some_and(|value| value == format!("Bearer {TOKEN}"))
    }));
    assert_eq!(detail.summary.state, PullState::Merged);
    assert_eq!(detail.summary.merged_by.as_ref().unwrap().login, "ken");
    assert_eq!(detail.summary.head.as_deref(), Some(HEAD));
    assert_eq!(detail.stale, None);
    // The address a status names is for the browser, kept as it is; a check
    // run's must be on github.com.
    assert_eq!(
        detail.checks.statuses[0].page.as_ref().unwrap().host(),
        "preview.example.test"
    );
    assert!(detail.checks.runs[0].page.is_some());
    assert!(detail.checks.runs[2].page.is_none());
}

#[tokio::test]
async fn the_latest_decisive_review_of_each_person_stands() {
    let api = card(|_| None);
    let detail = client(&api, None).detail("acme/shop", 418).await.unwrap();
    let states: Vec<(&str, ReviewState)> = detail
        .reviews
        .iter()
        .map(|review| (review.who.login.as_str(), review.state))
        .collect();
    // ken: a later comment never replaces a decision, a later decision
    // replaces an earlier one; zoe only commented (her pending review is
    // her own); ana's approval was dismissed.
    assert_eq!(
        states,
        [
            ("ken", ReviewState::Approved),
            ("zoe", ReviewState::Commented),
            ("ana", ReviewState::Dismissed),
        ]
    );
    assert!(!detail.reviews_capped);
}

#[tokio::test]
async fn checks_roll_up_to_one_state_and_say_when_they_stopped_at_the_cap() {
    let api = card(|_| None);
    let detail = client(&api, None).detail("acme/shop", 418).await.unwrap();
    // A run still going makes them pending; 3 of 112 runs were read.
    assert_eq!(detail.checks.state, ChecksState::Pending);
    assert_eq!(detail.checks.runs.len(), 3);
    assert_eq!(detail.checks.runs[0].state, RunState::Passed);
    assert_eq!(detail.checks.runs[1].state, RunState::Running);
    assert_eq!(detail.checks.runs[2].state, RunState::Ended);
    assert_eq!(
        detail.checks.capped,
        Some(Capped {
            read: 3,
            total: 112
        })
    );
    assert_eq!(detail.checks.statuses[0].state, StatusState::Passed);
    let roll = |runs: Value, statuses: Value| {
        super::detail::checks(&json!({"check_runs": runs}), &json!({"statuses": statuses})).state
    };
    let run = |status: &str, conclusion: Option<&str>| json!({"name": "x", "status": status, "conclusion": conclusion});
    assert_eq!(roll(json!([]), json!([])), ChecksState::None);
    assert_eq!(
        roll(json!([run("completed", Some("success"))]), json!([])),
        ChecksState::Passing
    );
    assert_eq!(
        roll(json!([run("completed", Some("neutral"))]), json!([])),
        ChecksState::Passing
    );
    assert_eq!(
        roll(
            json!([run("completed", Some("failure")), run("queued", None)]),
            json!([])
        ),
        ChecksState::Failing
    );
    assert_eq!(
        roll(
            json!([run("completed", Some("success"))]),
            json!([{"context": "c", "state": "pending"}])
        ),
        ChecksState::Pending
    );
    assert_eq!(
        roll(json!([]), json!([{"context": "c", "state": "error"}])),
        ChecksState::Failing
    );
}

#[tokio::test]
async fn more_than_a_hundred_reviews_say_the_first_hundred() {
    let reviews: Vec<Value> = (0..120)
        .map(|n| review(&format!("person{n}"), "APPROVED", "2026-09-30T09:00:00Z"))
        .collect();
    let api = card(move |request| {
        request
            .path
            .contains("/reviews")
            .then(|| Reply::json(200, "", &Value::Array(reviews.clone())))
    });
    let detail = client(&api, None).detail("acme/shop", 418).await.unwrap();
    assert_eq!(detail.reviews.len(), 100);
    assert!(detail.reviews_capped);
}

#[tokio::test]
async fn a_merged_pull_requests_reviews_and_checks_stay_ten_minutes() {
    let api = card(|_| None);
    let handle = client(&api, Some(TOKEN));
    handle.detail("acme/shop", 418).await.unwrap();
    assert_eq!(api.seen().len(), 4);
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(9 * 60));
    handle.detail("acme/shop", 418).await.unwrap();
    // The pull request itself is asked again after a minute; what it
    // labelled with 304 or 200 is not what the other three wait on.
    assert_eq!(api.seen().len(), 5, "only the pull request");
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(2 * 60));
    handle.detail("acme/shop", 418).await.unwrap();
    assert_eq!(api.seen().len(), 9, "all four after ten minutes");
}

#[tokio::test]
async fn a_card_read_the_limit_stops_halfway_stands_as_what_was_kept() {
    let blocked = Arc::new(AtomicUsize::new(0));
    let api = card({
        let blocked = blocked.clone();
        move |request| {
            (blocked.load(Ordering::SeqCst) == 1 && request.path.contains("/reviews"))
                .then(|| Reply::empty(429, "retry-after: 300\r\n"))
        }
    });
    let handle = client(&api, Some(TOKEN));
    let first = handle.detail("acme/shop", 418).await.unwrap();
    assert!(first.stale.is_none());
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(11 * 60));
    blocked.store(1, Ordering::SeqCst);
    let second = handle.detail("acme/shop", 418).await.unwrap();
    let stale = second.stale.as_ref().expect("marked stale");
    assert!(
        stale
            .why
            .starts_with("GitHub's secondary rate limit is reached until ")
    );
    assert_eq!(second.reviews, first.reviews, "what was kept stands");
    assert_eq!(second.summary.stale, second.stale);
    // Never read before, there is nothing to stand in: the words are an error.
    let fresh = client(&api, Some(TOKEN)).detail("acme/shop", 418).await;
    assert!(matches!(fresh, Err(Error::Limited { .. })));
}

#[tokio::test]
async fn the_card_names_its_failures_in_the_cards_words() {
    let api = Server::new(|_| Reply::empty(404, ""));
    let error = client(&api, None).detail("acme/shop", 1).await.unwrap_err();
    assert_eq!(
        error.words("acme/shop"),
        "acme/shop wasn't found. If it's private, add a GitHub token"
    );
    // A pull request without a usable head commit can't be asked about.
    let api = Server::new(|_| {
        let mut pull = pull(1);
        pull["head"]["sha"] = json!("../../etc");
        Reply::json(200, "", &pull)
    });
    let error = client(&api, None).detail("acme/shop", 1).await.unwrap_err();
    assert!(matches!(error, Error::Unreadable(_)));
    assert_eq!(api.seen().len(), 1, "no path is built from it");
    assert!(client(&api, None).detail("../x", 1).await.is_err());
}

#[test]
fn a_detail_key_is_the_client_the_pull_request_and_its_head() {
    let identity = client_identity();
    let mut summary = super::types::summary("acme/shop", &{
        let mut value = pull(418);
        value["head"]["sha"] = json!(HEAD);
        value
    })
    .unwrap();
    let key = DetailKey::of(&identity, &summary).unwrap();
    assert_eq!(
        (key.repo.as_str(), key.number, key.head.as_str()),
        ("acme/shop", 418, HEAD)
    );
    let mut other = identity.clone();
    other.setup_revision += 1;
    assert_ne!(DetailKey::of(&other, &summary), Some(key.clone()));
    summary.head = Some("not a sha".into());
    assert_eq!(DetailKey::of(&identity, &summary), None);
    summary.head = None;
    assert_eq!(DetailKey::of(&identity, &summary), None);
}
