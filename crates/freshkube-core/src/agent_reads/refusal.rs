//! Why a read was not answered, in the codes the wire contract names.

use std::fmt;

use serde::Serialize;

use crate::failure::{Failure, FailureKind};
use crate::write::scrub;

/// The code of a refusal, as the agent reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    /// An argument the call cannot be bounded by. Names it, never its value.
    Invalid,
    /// The cluster is not one the session may read.
    UnknownCluster,
    /// Agent access is off for the cluster; nothing was sent.
    NotEnabled,
    /// The cluster refused the person's own access (401 or 403).
    Refused,
    NotFound,
    /// The cluster does not serve the kind.
    NotServed,
    /// An object past the limit.
    TooLarge,
    Unreachable,
    TimedOut,
    Failed,
    /// The session has used its reads.
    LimitReached,
}

impl Code {
    pub const ALL: [Self; 11] = [
        Self::Invalid,
        Self::UnknownCluster,
        Self::NotEnabled,
        Self::Refused,
        Self::NotFound,
        Self::NotServed,
        Self::TooLarge,
        Self::Unreachable,
        Self::TimedOut,
        Self::Failed,
        Self::LimitReached,
    ];

    /// The word on the wire and in the logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Invalid => "invalid",
            Self::UnknownCluster => "unknown_cluster",
            Self::NotEnabled => "not_enabled",
            Self::Refused => "refused",
            Self::NotFound => "not_found",
            Self::NotServed => "not_served",
            Self::TooLarge => "too_large",
            Self::Unreachable => "unreachable",
            Self::TimedOut => "timed_out",
            Self::Failed => "failed",
            Self::LimitReached => "limit_reached",
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A read that was not answered. The message is passed through the audit
/// log's `scrub` when it is built, so no refusal carries a token or an echoed
/// value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: Code,
    message: String,
    /// The failure may be the shared client's fault (the server could not be
    /// reached, or rejected its credentials), so the access should build it
    /// again. A 403 is the person's role speaking, not the client's.
    drops_client: bool,
}

impl Refusal {
    pub fn new(code: Code, message: &str) -> Self {
        Self {
            code,
            message: scrub(message),
            drops_client: false,
        }
    }

    /// An argument that cannot be bounded: it names the argument and says
    /// what is wrong, and never repeats the value. `what` must not hold a
    /// quote or a backtick, which `scrub` would replace.
    pub fn invalid(argument: &str, what: &str) -> Self {
        Self::new(Code::Invalid, &format!("argument {argument} {what}"))
    }

    pub fn not_enabled() -> Self {
        Self::new(
            Code::NotEnabled,
            "agent access is off for this cluster; nothing was read",
        )
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// Whether the access should forget the client this read used.
    pub(super) fn drops_client(&self) -> bool {
        self.drops_client
    }

    /// A kind the cluster does not serve.
    pub fn not_served() -> Self {
        Self::new(Code::NotServed, "the cluster does not serve this kind")
    }

    /// Maps a failed read onto its code. One match, no wildcard: a new
    /// `FailureKind` stops the build until it is placed here.
    pub fn from_failure(failure: &Failure) -> Self {
        let code = match failure.kind {
            FailureKind::Config => Code::Failed,
            FailureKind::Unreachable => Code::Unreachable,
            FailureKind::Timeout => Code::TimedOut,
            FailureKind::Unauthorized => Code::Refused,
            FailureKind::Forbidden => Code::Refused,
            FailureKind::NotFound => Code::NotFound,
            FailureKind::Other => Code::Failed,
        };
        let mut refusal = Self::new(code, &failure.message);
        // Decided from the failure, not the code: both 401 and 403 are
        // `refused`, and only the first says anything of the client.
        refusal.drops_client = matches!(
            failure.kind,
            FailureKind::Unreachable | FailureKind::Unauthorized
        );
        refusal
    }

    /// A failure of looking up a kind, where a 404 means the API is not
    /// served rather than that an object is missing.
    pub fn from_kind_failure(failure: &Failure) -> Self {
        if failure.kind == FailureKind::NotFound {
            Self::not_served()
        } else {
            Self::from_failure(failure)
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Refusal {}
