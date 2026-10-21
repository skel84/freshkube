//! Why GitHub wasn't read, in words that are safe to display: no library
//! error, response body, URL or token is ever in them.

use chrono::{DateTime, Utc};
use freshkube_core::resources::{Failure, FailureKind};
use freshkube_core::setup::GitHubToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    TimedOut,
    Connect,
    Transport,
}

impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TimedOut => "timed out",
            Self::Connect => "could not connect",
            Self::Transport => "the response could not be received",
        })
    }
}

/// Why the token couldn't be had at all, so GitHub isn't read: a cause
/// with its next step, not an error to retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotSetUp {
    /// The setup names `gh`, and there is none to run.
    GhNotFound,
    /// `gh auth token` failed: not signed in.
    GhNotSignedIn,
    /// `gh auth token` didn't finish in time.
    GhSlow,
    /// The setup names `keys.json`, and the key store couldn't be read.
    KeysUnavailable,
    /// The setup names `keys.json`, and no GitHub token is saved there.
    NoSavedToken,
    /// What was saved, or what `gh` printed, isn't a bearer token.
    TokenUnreadable,
}

impl std::fmt::Display for NotSetUp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::GhNotFound => {
                "gh not found. Install the GitHub CLI, or set delivery.github.token to keys.json"
            }
            Self::GhNotSignedIn => "gh isn't signed in to GitHub. Run gh auth login",
            Self::GhSlow => "gh didn't answer in time",
            Self::KeysUnavailable => "the key store can't be read",
            Self::NoSavedToken => "no GitHub token is saved in keys.json",
            Self::TokenUnreadable => "the saved GitHub token can't be read",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The token couldn't be had, so nothing is read.
    NotSetUp(NotSetUp),
    Unreachable(Cause),
    /// 401: the token was refused, where it came from. It is never retried
    /// without it.
    Unauthorized(GitHubToken),
    /// 403 that is not a rate limit.
    Forbidden,
    NotFound {
        token_set: bool,
    },
    /// 301: the repository was renamed or moved. Never followed.
    Moved,
    /// The limit is reached until `until`; nothing is sent until then.
    Limited {
        until: DateTime<Utc>,
        secondary: bool,
    },
    /// The limit is nearly spent, and this read can wait.
    Low {
        left: u32,
        until: DateTime<Utc>,
    },
    Http(u16),
    Unreadable(&'static str),
}

fn at(time: &DateTime<Utc>) -> String {
    time.format("%H:%M UTC").to_string()
}

impl std::fmt::Display for Error {
    /// The words of the status bar and the list.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSetUp(cause) => write!(f, "GitHub isn't set up: {cause}"),
            Self::Unreachable(cause) => write!(f, "GitHub didn't answer: {cause}"),
            Self::Unauthorized(_) => f.write_str("GitHub refused the token (401)"),
            Self::Forbidden => f.write_str("GitHub refused the request (403)"),
            Self::NotFound { .. } => f.write_str("GitHub didn't find it (404)"),
            Self::Moved => f.write_str("GitHub says the repository moved (301)"),
            Self::Limited { until, secondary } => write!(
                f,
                "GitHub's {}rate limit is reached until {}",
                if *secondary { "secondary " } else { "" },
                at(until)
            ),
            Self::Low { left, until } => {
                write!(f, "GitHub's limit is low: {left} left until {}", at(until))
            }
            Self::Http(status) => write!(f, "GitHub returned HTTP {status}"),
            Self::Unreadable(why) => write!(f, "GitHub's answer could not be read: {why}"),
        }
    }
}

impl Error {
    /// The card's words, which name the repository and what to do.
    pub fn words(&self, repo: &str) -> String {
        match self {
            Self::Unauthorized(GitHubToken::KeysJson) => format!(
                "GitHub refused the token: it may have expired. It is the keys.json entry {}",
                super::ACCOUNT
            ),
            Self::Unauthorized(GitHubToken::Gh) => "GitHub refused the token: it may have \
                 expired. It is the one gh holds; run gh auth login"
                .to_owned(),
            Self::Forbidden => format!(
                "GitHub refused reading {repo} (403): the token may not be granted this repository"
            ),
            Self::NotFound { token_set: false } => {
                format!("{repo} wasn't found. If it's private, add a GitHub token")
            }
            Self::NotFound { token_set: true } => {
                format!("{repo} wasn't found, or the token can't see it")
            }
            Self::Moved => format!("GitHub says {repo} moved"),
            other => other.to_string(),
        }
    }

    pub(super) fn kind(&self) -> FailureKind {
        match self {
            Self::NotSetUp(_) => FailureKind::Config,
            Self::Unreachable(Cause::TimedOut) => FailureKind::Timeout,
            Self::Unreachable(_) => FailureKind::Unreachable,
            Self::Unauthorized(_) => FailureKind::Unauthorized,
            Self::Forbidden => FailureKind::Forbidden,
            // A missing repository is not an uninstalled API, and a limit
            // reached or nearly spent is not a refusal.
            Self::NotFound { .. }
            | Self::Moved
            | Self::Limited { .. }
            | Self::Low { .. }
            | Self::Http(_)
            | Self::Unreadable(_) => FailureKind::Other,
        }
    }
}

impl std::error::Error for Error {}

impl From<NotSetUp> for Error {
    fn from(cause: NotSetUp) -> Self {
        Self::NotSetUp(cause)
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Failure::new(error.kind(), error.to_string())
    }
}
