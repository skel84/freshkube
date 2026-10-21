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
mod memo;
#[cfg(test)]
mod tests;
mod types;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use freshkube_core::resources::Failure;
use freshkube_core::secrets::Secrets;
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::time::Instant;
use url::Url;

use super::{GitHub, PullRequest, check_repo, check_sha, parse_pull_request};
use budget::Budget;
use memo::Memo;

pub use budget::RateLimit;
pub use detail::{
    Capped, CheckRun, Checks, ChecksState, DetailKey, PullDetail, Review, ReviewState, RunState,
    StatusContext, StatusState,
};
pub use error::{Cause, Error};
pub(crate) use types::summary;
pub use types::{AVATAR_HOST, AvatarUrl, Person, PullState, PullSummary, Reviewer, Stale};

/// The keys.json account of the token, normalized as Results' is.
pub const ACCOUNT: &str = "GitHub https://api.github.com/";
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
    /// The keys.json account the token was read from; `None` when anonymous.
    pub account: Option<String>,
    pub setup_revision: u64,
}

pub struct Client {
    http: reqwest::Client,
    api: Url,
    token: Option<http::HeaderValue>,
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
        return Err(Error::Token("the stored token is empty"));
    }
    if token.len() > MOST_TOKEN_BYTES
        || !token.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/' | b'=')
        })
    {
        return Err(Error::Token("the stored token is not a bearer token"));
    }
    let mut header = http::HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| Error::Token("the stored token is not a bearer token"))?;
    header.set_sensitive(true);
    Ok(header)
}

impl Client {
    /// Opens on an I/O worker, reading only the key store when the setup
    /// names `keys.json`. A key store that fails is an error, never a quiet
    /// downgrade to anonymous; a store with no key is anonymous.
    pub async fn open(
        use_token: bool,
        secrets: Option<Secrets>,
        setup_revision: u64,
    ) -> Result<Self, Error> {
        let token = if use_token {
            tokio::task::spawn_blocking(move || {
                secrets
                    .ok_or(Error::Token("the key store isn't available to this read"))?
                    .read(ACCOUNT)
                    .map_err(|_| Error::Token("the key store is unavailable"))
            })
            .await
            .map_err(|_| Error::Token("the key store could not be read"))??
        } else {
            None
        };
        let api = Url::parse(API).map_err(|_| Error::Unreadable("the API address"))?;
        Self::build(reqwest::Client::builder(), api, token, setup_revision)
    }

    /// A client that asks anonymously: no I/O, so it exists at once.
    pub fn anonymous(setup_revision: u64) -> Result<Self, Error> {
        let api = Url::parse(API).map_err(|_| Error::Unreadable("the API address"))?;
        Self::build(reqwest::Client::builder(), api, None, setup_revision)
    }

    fn build(
        builder: reqwest::ClientBuilder,
        api: Url,
        token: Option<String>,
        setup_revision: u64,
    ) -> Result<Self, Error> {
        let token = token.as_deref().map(token_header).transpose()?;
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
            account: token.is_some().then(|| ACCOUNT.to_owned()),
            setup_revision,
        };
        Ok(Self {
            http,
            api,
            token,
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
        let mut response = request.send().await.map_err(unreachable)?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let now = Utc::now();
        self.state
            .lock()
            .unwrap()
            .budget
            .observe(&headers, self.authenticated(), now);
        match status {
            200 => {}
            304 => {
                let mut state = self.state.lock().unwrap();
                let kept = state.memo.kept(path);
                return match kept {
                    Some((body, _)) => {
                        state.memo.renew(path, keep(&body), Instant::now());
                        Ok(body)
                    }
                    None => Err(Error::Unreadable("an answer that was never kept")),
                };
            }
            // A redirect is an answer, never a hop.
            301 | 302 | 307 | 308 => return Err(Error::Moved),
            401 => return Err(Error::Unauthorized),
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

    /// A client that reads its token from the key store, off the calling
    /// thread; see [`Client::open`].
    pub async fn open(secrets: Option<Secrets>, setup_revision: u64) -> Result<Self, Error> {
        Client::open(true, secrets, setup_revision)
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
        let root = reqwest::Certificate::from_pem(fake::CERT).unwrap();
        let builder = reqwest::Client::builder()
            .add_root_certificate(root)
            .no_proxy();
        let url = Url::parse(&format!("{address}/")).unwrap();
        Self::new(Client::build(builder, url, token.map(str::to_owned), 1).unwrap())
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
