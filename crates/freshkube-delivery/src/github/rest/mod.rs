//! GitHub's REST API, read with GET requests only, for pull requests.
//!
//! One [`Client`] has one identity, one rate-limit [`Budget`](budget) and one
//! [`Memo`](memo), and sends one request at a time. What it will not do:
//!
//! - send the token anywhere but the API host: every address is built here
//!   from a checked repository, a checked SHA and a number, and no address
//!   that comes back in a response (`html_url`, `avatar_url`, `target_url`,
//!   `Link`) is ever fetched;
//! - follow a redirect: a 301 is an answer, "GitHub says the repository
//!   moved";
//! - fall back to an anonymous request when the token was refused;
//! - read an error's body.
//!
//! The client has no `Debug`, so neither its token nor its address reaches a
//! log.

mod budget;
mod detail;
mod error;
#[cfg(test)]
pub(crate) mod fake;
mod gh;
mod memo;
#[cfg(test)]
mod tests;
mod types;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use freshkube_core::resources::Failure;
use freshkube_core::secrets::Secrets;
use freshkube_core::setup::GitHubToken;
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::time::Instant;
use url::Url;

use super::{GitHub, PullRequest, check_repo, check_sha, parse_pull_request};
use budget::Budget;
use memo::Memo;

pub use budget::RateLimit;
pub use detail::{
    CheckRun, Checks, ChecksState, DetailKey, PullDetail, Review, ReviewState, RunState,
};
pub use error::{Cause, Error, NotSetUp};
pub use types::{AVATAR_HOST, AvatarUrl, Person, PullState, PullSummary, Reviewer, Stale};
pub(crate) use types::{cause_of, summary};

/// The keys.json account of the token, normalized as Results' is.
pub const ACCOUNT: &str = "GitHub https://api.github.com/";
/// How long a caller that runs [`Handle::open`] should give it: longer than
/// `gh`'s own deadline, so a `gh` that hangs is stopped by that deadline and
/// worded as gh being slow, never cut off first by the caller's.
pub const OPEN_DEADLINE: Duration = Duration::from_secs(gh::DEADLINE.as_secs() + 5);
/// What the identity calls a token that `gh` gave.
pub const GH_ACCOUNT: &str = "gh";
const API: &str = "https://api.github.com/";
const DEADLINE: Duration = Duration::from_secs(5);
/// The most of an answer read.
const MOST_BODY_BYTES: usize = 8 * 1024 * 1024;
/// The most a token may be.
const MOST_TOKEN_BYTES: usize = 8192;
/// The most an item list asks for.
const PER_PAGE: usize = 100;
/// How many calls are left before the list stops looking anything up.
const RESERVE_ANONYMOUS: u32 = 10;
const RESERVE_TOKEN: u32 = 100;
/// How long an answer that may still change is kept.
const OPEN_TOKEN: Duration = Duration::from_secs(60);
const OPEN_ANONYMOUS: Duration = Duration::from_secs(15 * 60);
/// How long a 429 without `retry-after` blocks.
const BLOCK_WITHOUT_RETRY: u64 = 60;

/// Whose answers these are: never the token itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub api: Url,
    /// Where the token was read from: the keys.json account, or `gh`;
    /// `None` when anonymous.
    pub account: Option<String>,
    pub setup_revision: u64,
}

pub struct Client {
    http: reqwest::Client,
    api: Url,
    token: Option<http::HeaderValue>,
    /// Where the token came from, which a refusal names.
    source: Option<GitHubToken>,
    identity: Identity,
    /// One request at a time, as GitHub's secondary limits ask.
    turn: Semaphore,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    budget: Budget,
    memo: Memo,
}

fn token_header(token: &str) -> Result<http::HeaderValue, Error> {
    if token.trim().is_empty() {
        return Err(Error::NotSetUp(NotSetUp::TokenUnreadable));
    }
    if token.len() > MOST_TOKEN_BYTES
        || !token.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/' | b'=')
        })
    {
        return Err(Error::NotSetUp(NotSetUp::TokenUnreadable));
    }
    let mut header = http::HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| Error::NotSetUp(NotSetUp::TokenUnreadable))?;
    header.set_sensitive(true);
    Ok(header)
}

impl Client {
    /// Opens on an I/O worker, taking the token from where the setup names
    /// it: the key store's GitHub account, or `gh auth token`. Neither is
    /// read unless it is named, and a source that can't give a token is a
    /// cause to show ([`NotSetUp`]), never a quiet downgrade to anonymous.
    pub async fn open(
        source: GitHubToken,
        secrets: Option<Secrets>,
        setup_revision: u64,
    ) -> Result<Self, Error> {
        Self::open_with(
            source,
            secrets,
            setup_revision,
            &gh::default_dirs(),
            gh::DEADLINE,
        )
        .await
    }

    /// [`open`](Self::open), looking for `gh` in `dirs` and giving it
    /// `gh_deadline`.
    async fn open_with(
        source: GitHubToken,
        secrets: Option<Secrets>,
        setup_revision: u64,
        dirs: &[std::path::PathBuf],
        gh_deadline: Duration,
    ) -> Result<Self, Error> {
        let token = Self::token_of(source, secrets, dirs, gh_deadline).await?;
        let api = Url::parse(API).map_err(|_| Error::Unreadable("the API address"))?;
        Self::build(
            reqwest::Client::builder(),
            api,
            Some((token, source)),
            setup_revision,
        )
    }

    /// The token, from where `source` says, and from nowhere else.
    async fn token_of(
        source: GitHubToken,
        secrets: Option<Secrets>,
        dirs: &[std::path::PathBuf],
        gh_deadline: Duration,
    ) -> Result<String, Error> {
        Ok(match source {
            GitHubToken::Gh => gh::token(dirs, gh_deadline).await?,
            GitHubToken::KeysJson => {
                let read = tokio::task::spawn_blocking(move || {
                    secrets
                        .ok_or(NotSetUp::KeysUnavailable)?
                        .read(ACCOUNT)
                        .map_err(|_| NotSetUp::KeysUnavailable)
                })
                .await
                .map_err(|_| NotSetUp::KeysUnavailable)??;
                read.ok_or(NotSetUp::NoSavedToken)?
            }
        })
    }

    /// A client that asks anonymously: no I/O, so it exists at once.
    pub fn anonymous(setup_revision: u64) -> Result<Self, Error> {
        let api = Url::parse(API).map_err(|_| Error::Unreadable("the API address"))?;
        Self::build(reqwest::Client::builder(), api, None, setup_revision)
    }

    fn build(
        builder: reqwest::ClientBuilder,
        api: Url,
        token: Option<(String, GitHubToken)>,
        setup_revision: u64,
    ) -> Result<Self, Error> {
        let source = token.as_ref().map(|(_, source)| *source);
        let token = token
            .as_ref()
            .map(|(token, _)| token_header(token))
            .transpose()?;
        let http = builder
            .use_rustls_tls()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(DEADLINE)
            .timeout(DEADLINE)
            .build()
            .map_err(|_| Error::Unreadable("the HTTP client could not be configured"))?;
        let identity = Identity {
            api: api.clone(),
            account: source.map(|source| match source {
                GitHubToken::Gh => GH_ACCOUNT.to_owned(),
                GitHubToken::KeysJson => ACCOUNT.to_owned(),
            }),
            setup_revision,
        };
        Ok(Self {
            http,
            api,
            token,
            source,
            identity,
            turn: Semaphore::new(1),
            state: Mutex::new(State::default()),
        })
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    pub fn authenticated(&self) -> bool {
        self.token.is_some()
    }

    /// The limit as the last response reported it.
    pub fn rate_limit(&self) -> Option<RateLimit> {
        self.state.lock().unwrap().budget.rate().cloned()
    }

    fn reserve(&self) -> u32 {
        if self.authenticated() {
            RESERVE_TOKEN
        } else {
            RESERVE_ANONYMOUS
        }
    }

    /// How long an answer that may still change stays fresh.
    fn open_for(&self) -> Duration {
        if self.authenticated() {
            OPEN_TOKEN
        } else {
            OPEN_ANONYMOUS
        }
    }

    /// The answer for `path`: kept while fresh, else asked for. `spare`
    /// stops short of asking when the limit is low; the answer a person
    /// asked for does not.
    async fn get(
        &self,
        path: &str,
        spare: bool,
        keep: impl Fn(&Value) -> Option<Duration>,
    ) -> Result<Arc<Value>, Error> {
        let fresh = |client: &Self| {
            client
                .state
                .lock()
                .unwrap()
                .memo
                .fresh(path, Instant::now())
        };
        if let Some(answer) = fresh(self) {
            return Ok(answer);
        }
        let _turn = self
            .turn
            .acquire()
            .await
            .map_err(|_| Error::Unreadable("the client is closed"))?;
        // Another read may have asked while this one waited its turn.
        if let Some(answer) = fresh(self) {
            return Ok(answer);
        }
        let etag = {
            let mut state = self.state.lock().unwrap();
            let now = Utc::now();
            if let Some(blocked) = state.budget.blocked(now) {
                return Err(Error::Limited {
                    until: blocked.until,
                    secondary: blocked.secondary,
                });
            }
            if spare && let Some((left, until)) = state.budget.low(self.reserve(), now) {
                return Err(Error::Low { left, until });
            }
            // An anonymous 304 counts against the limit as a 200 does, so
            // an anonymous client relies on a longer expiry instead.
            self.authenticated()
                .then(|| state.memo.etag(path).map(str::to_owned))
                .flatten()
        };
        let mut etag = etag;
        let (mut response, status, headers) = loop {
            let response = self.send(path, etag.as_deref()).await?;
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            self.state
                .lock()
                .unwrap()
                .budget
                .observe(&headers, self.authenticated(), Utc::now());
            if status == 304 {
                let kept = {
                    let mut state = self.state.lock().unwrap();
                    let kept = state.memo.kept(path);
                    if let Some((body, _)) = &kept {
                        state.memo.renew(path, keep(body), Instant::now());
                    }
                    kept
                };
                if let Some((body, _)) = kept {
                    return Ok(body);
                }
                // The answer it validates was dropped from the memo: read
                // it again, without the validator.
                if etag.take().is_some() {
                    continue;
                }
                return Err(Error::Unreadable("an answer that was never kept"));
            }
            break (response, status, headers);
        };
        let now = Utc::now();
        match status {
            200 => {}
            // A redirect is an answer, never a hop.
            301 | 302 | 307 | 308 => return Err(Error::Moved),
            401 => {
                return Err(Error::Unauthorized(
                    self.source.unwrap_or(GitHubToken::KeysJson),
                ));
            }
            403 | 429 => return Err(self.limit_or_refusal(status, &headers, now)),
            404 => {
                return Err(Error::NotFound {
                    token_set: self.authenticated(),
                });
            }
            other => return Err(Error::Http(other)),
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(unreachable)? {
            if body.len() + chunk.len() > MOST_BODY_BYTES {
                return Err(Error::Unreadable("it is longer than the cap"));
            }
            body.extend_from_slice(&chunk);
        }
        let value: Value =
            serde_json::from_slice(&body).map_err(|_| Error::Unreadable("it is not JSON"))?;
        let value = Arc::new(value);
        let etag = headers
            .get(http::header::ETAG)
            .and_then(|etag| etag.to_str().ok())
            .map(str::to_owned);
        self.state.lock().unwrap().memo.store(
            path,
            etag,
            value.clone(),
            keep(&value),
            Instant::now(),
        );
        Ok(value)
    }

    /// One GET of `path`, with the token when there is one and the
    /// validator when it is given.
    async fn send(&self, path: &str, etag: Option<&str>) -> Result<reqwest::Response, Error> {
        let url = self
            .api
            .join(path)
            .map_err(|_| Error::Unreadable("the address"))?;
        let mut request = self
            .http
            .get(url)
            .header(http::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header(
                http::header::USER_AGENT,
                concat!("freshkube/", env!("CARGO_PKG_VERSION")),
            );
        if let Some(token) = &self.token {
            request = request.header(http::header::AUTHORIZATION, token.clone());
        }
        if let Some(etag) = etag {
            request = request.header(http::header::IF_NONE_MATCH, etag);
        }
        request.send().await.map_err(unreachable)
    }

    /// A 403 or 429 is a limit when the budget says none is left or a
    /// `retry-after` names a time; a 429 with neither blocks for a minute;
    /// a 403 with neither is a refusal.
    fn limit_or_refusal(
        &self,
        status: u16,
        headers: &http::HeaderMap,
        now: chrono::DateTime<Utc>,
    ) -> Error {
        let mut state = self.state.lock().unwrap();
        if let Some(blocked) = state.budget.blocked(now) {
            return Error::Limited {
                until: blocked.until,
                secondary: blocked.secondary,
            };
        }
        let retry = headers
            .get(http::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let seconds = match (retry, status) {
            (Some(seconds), _) => seconds,
            (None, 429) => BLOCK_WITHOUT_RETRY,
            (None, _) => return Error::Forbidden,
        };
        state.budget.block_for(seconds, now);
        let blocked = state.budget.blocked(now).map(|blocked| blocked.until);
        Error::Limited {
            until: blocked.unwrap_or(now),
            secondary: true,
        }
    }

    /// What the memo kept, in place of an answer that couldn't be asked
    /// for: the rate limit and a host that doesn't answer leave the last
    /// read standing, marked with why it's old.
    fn or_kept(&self, path: &str, error: Error) -> Result<(Arc<Value>, Option<Stale>), Error> {
        let spent = matches!(
            error,
            Error::Limited { .. } | Error::Low { .. } | Error::Unreachable(_)
        );
        let kept = spent
            .then(|| self.state.lock().unwrap().memo.kept(path))
            .flatten();
        match kept {
            Some((body, read_at)) => Ok((
                body,
                Some(Stale {
                    read_at,
                    why: error.to_string(),
                }),
            )),
            None => Err(error),
        }
    }

    /// The pull requests GitHub associates with a commit.
    async fn pulls(&self, repo: &str, sha: &str, spare: bool) -> Result<Vec<PullRequest>, Error> {
        let path = format!("repos/{repo}/commits/{sha}/pulls?per_page={PER_PAGE}");
        let open = self.open_for();
        let sha_owned = sha.to_owned();
        let asked = self
            .get(&path, spare, move |body| {
                // A commit's pulls where one is merged with this commit as
                // its merge commit never change.
                let settled = body.as_array().into_iter().flatten().any(|pull| {
                    pull.get("merged_at").is_some_and(|at| !at.is_null())
                        && pull
                            .get("merge_commit_sha")
                            .and_then(Value::as_str)
                            .is_some_and(|merge| merge.eq_ignore_ascii_case(&sha_owned))
                });
                (!settled).then_some(open)
            })
            .await
            .map(|body| (body, None));
        let (body, stale) = asked.or_else(|error| self.or_kept(&path, error))?;
        Ok(body
            .as_array()
            .into_iter()
            .flatten()
            .take(PER_PAGE)
            .filter_map(|pull| parse_pull_request(repo, pull))
            .map(|pull| pull.stale(stale.as_ref()))
            .collect())
    }

    async fn pull(&self, repo: &str, number: u64, spare: bool) -> Result<PullRequest, Error> {
        let path = format!("repos/{repo}/pulls/{number}");
        let open = self.open_for();
        let asked = self
            .get(&path, spare, move |_| Some(open))
            .await
            .map(|body| (body, None));
        let (body, stale) = asked.or_else(|error| self.or_kept(&path, error))?;
        parse_pull_request(repo, &body)
            .map(|pull| pull.stale(stale.as_ref()))
            .ok_or(Error::Unreadable("an unexpected pull request shape"))
    }
}

fn unreachable(error: reqwest::Error) -> Error {
    // The error's text can repeat the URL; say only what kind it was.
    Error::Unreachable(if error.is_timeout() {
        Cause::TimedOut
    } else if error.is_connect() {
        Cause::Connect
    } else {
        Cause::Transport
    })
}

/// The client as the change read holds it: shared by every read of one
/// setup revision. `sparing` is the Changes list's, which stops looking
/// things up when the limit is low; the change page's is not.
#[derive(Clone)]
pub struct Handle {
    client: Arc<Client>,
    spare: bool,
}

impl Handle {
    /// An anonymous client; see [`Client::anonymous`].
    pub fn anonymous(setup_revision: u64) -> Result<Self, Error> {
        Client::anonymous(setup_revision).map(Self::new)
    }

    /// A client whose token comes from where the setup names it, read off
    /// the calling thread; see [`Client::open`].
    pub async fn open(
        source: GitHubToken,
        secrets: Option<Secrets>,
        setup_revision: u64,
    ) -> Result<Self, Error> {
        Client::open(source, secrets, setup_revision)
            .await
            .map(Self::new)
    }

    fn new(client: Client) -> Self {
        Self {
            client: Arc::new(client),
            spare: false,
        }
    }

    /// This client, as the Changes list reads through it.
    pub fn sparing(&self) -> Self {
        Self {
            client: self.client.clone(),
            spare: true,
        }
    }

    pub fn identity(&self) -> &Identity {
        self.client.identity()
    }

    pub fn rate_limit(&self) -> Option<RateLimit> {
        self.client.rate_limit()
    }
}

#[cfg(feature = "testing")]
impl Handle {
    /// An anonymous client for a fake at `address`, for other crates' tests.
    /// It can hold no token, so nothing it is pointed at can ever see one.
    pub fn anonymous_at(address: &str) -> Self {
        let url = Url::parse(&format!("{address}/")).expect("an address");
        let builder = reqwest::Client::builder().no_proxy();
        Self::new(Client::build(builder, url, None, 1).expect("a client"))
    }
}

#[cfg(test)]
impl Handle {
    /// A client for the fake API at `address`, which trusts its certificate
    /// alone and never uses a proxy.
    pub(crate) fn trusting(address: &str, token: Option<&str>) -> Self {
        Self::trusting_as(address, token, GitHubToken::KeysJson)
    }

    /// [`trusting`](Self::trusting), the token given by `source`.
    pub(crate) fn trusting_as(address: &str, token: Option<&str>, source: GitHubToken) -> Self {
        let root = reqwest::Certificate::from_pem(fake::CERT).unwrap();
        let builder = reqwest::Client::builder()
            .add_root_certificate(root)
            .no_proxy();
        let url = Url::parse(&format!("{address}/")).unwrap();
        Self::new(
            Client::build(
                builder,
                url,
                token.map(|token| (token.to_owned(), source)),
                1,
            )
            .unwrap(),
        )
    }
}

/// Same client, whichever way it is read.
impl PartialEq for Handle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.client, &other.client) && self.spare == other.spare
    }
}

impl Eq for Handle {}

/// Prints whose it is, never what it holds.
impl std::fmt::Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let identity = self.identity();
        f.debug_struct("Handle")
            .field("authenticated", &identity.account.is_some())
            .field("setup_revision", &identity.setup_revision)
            .finish()
    }
}

impl GitHub for Handle {
    async fn pulls_for_commit(&self, repo: &str, sha: &str) -> Result<Vec<PullRequest>, Failure> {
        check_repo(repo)?;
        check_sha(sha)?;
        Ok(self.client.pulls(repo, sha, self.spare).await?)
    }

    async fn pull(&self, repo: &str, number: u64) -> Result<PullRequest, Failure> {
        check_repo(repo)?;
        Ok(self.client.pull(repo, number, self.spare).await?)
    }
}
