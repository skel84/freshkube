//! Why GitHub wasn't read, in words that are safe to display: no library
//! error, response body, URL or token is ever in them.

use chrono::{DateTime, Utc};
use freshkube_core::resources::{Failure, FailureKind};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The stored token can't be used; why, in a fixed phrase.
    Token(&'static str),
    Unreachable(Cause),
    /// 401: the token was refused. It is never retried without it.
    Unauthorized,
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
            Self::Token(why) => write!(f, "The GitHub token can't be used: {why}"),
            Self::Unreachable(cause) => write!(f, "GitHub didn't answer: {cause}"),
            Self::Unauthorized => f.write_str("GitHub refused the token (401)"),
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
            Self::Unauthorized => format!(
                "GitHub refused the token: it may have expired. It is the keys.json entry {}",
                super::ACCOUNT
            ),
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
            Self::Token(_) => FailureKind::Config,
            Self::Unreachable(Cause::TimedOut) => FailureKind::Timeout,
            Self::Unreachable(_) => FailureKind::Unreachable,
            Self::Unauthorized => FailureKind::Unauthorized,
            Self::Forbidden | Self::Limited { .. } | Self::Low { .. } => FailureKind::Forbidden,
            // A missing repository is not an uninstalled API.
            Self::NotFound { .. } | Self::Moved | Self::Http(_) | Self::Unreadable(_) => {
                FailureKind::Other
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Failure::new(error.kind(), error.to_string())
    }
}
