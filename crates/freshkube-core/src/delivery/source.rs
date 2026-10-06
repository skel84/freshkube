//! What a read of one source came back as, so an unreadable source is never
//! taken for an empty one.

use crate::resources::{Failure, FailureKind};

/// A listing stopped at the page cap; `read` items arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Truncation {
    pub read: usize,
}

/// The outcome of reading one kind of object from one cluster.
#[derive(Clone, Debug, PartialEq)]
pub enum Source<T> {
    /// Read. An empty `T` means the source answered and has none.
    Read(T),
    /// Read, but the listing stopped at the page cap: what is here is real,
    /// and anything missing may only not have been read.
    Capped(T, Truncation),
    /// The API isn't served there: the CRD isn't installed (404).
    NotInstalled(String),
    /// The identity may not read it (401/403).
    Refused(String),
    /// Anything else: unreachable, timed out, an error.
    Unreadable(String),
}

impl<T> Source<T> {
    pub fn from_result(result: Result<T, Failure>) -> Self {
        match result {
            Ok(value) => Self::Read(value),
            Err(failure) => Self::from_failure(failure),
        }
    }

    /// Items of a listing, and whether it stopped at the page cap.
    pub fn from_listing(result: Result<(T, Option<Truncation>), Failure>) -> Self {
        match result {
            Ok((value, None)) => Self::Read(value),
            Ok((value, Some(truncation))) => Self::Capped(value, truncation),
            Err(failure) => Self::from_failure(failure),
        }
    }

    /// The failure's message has the identity that made the request taken
    /// out, so it can be printed.
    pub fn from_failure(failure: Failure) -> Self {
        let message = redact_identity(&failure.message);
        match failure.kind {
            FailureKind::NotFound => Self::NotInstalled(message),
            FailureKind::Forbidden | FailureKind::Unauthorized => Self::Refused(message),
            _ => Self::Unreadable(message),
        }
    }

    pub fn read(&self) -> Option<&T> {
        match self {
            Self::Read(value) | Self::Capped(value, _) => Some(value),
            _ => None,
        }
    }

    /// Where the listing stopped, if it hit the page cap.
    pub fn capped(&self) -> Option<Truncation> {
        match self {
            Self::Capped(_, truncation) => Some(*truncation),
            _ => None,
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Source<U> {
        match self {
            Self::Read(value) => Source::Read(f(value)),
            Self::Capped(value, truncation) => Source::Capped(f(value), truncation),
            Self::NotInstalled(why) => Source::NotInstalled(why),
            Self::Refused(why) => Source::Refused(why),
            Self::Unreadable(why) => Source::Unreadable(why),
        }
    }

    /// Why nothing could be read, for a link that depends on this source.
    pub fn why_not_read(&self) -> Option<String> {
        match self {
            Self::Read(_) | Self::Capped(..) => None,
            Self::NotInstalled(why) => Some(format!("not installed there: {why}")),
            Self::Refused(why) => Some(format!("not readable (refused): {why}")),
            Self::Unreadable(why) => Some(format!("not readable: {why}")),
        }
    }
}

/// What to add to "none was found" when the listing was capped: the thing may
/// be among the items that were not read.
pub fn cap_note(truncation: Option<Truncation>) -> String {
    truncation.map_or_else(String::new, |truncation| {
        format!(
            "; the listing stopped at the page cap after {} items, so it may be among those not read",
            truncation.read
        )
    })
}

/// Takes the quoted names out of `User "…"` and `Group "…"`, and out of
/// `users "…"` and `groups "…"` (an impersonation refusal), as the API server
/// writes them, so no identity reaches the output. `API group "…"` stays.
pub fn redact_identity(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    loop {
        let found = ["User \"", "users \"", "Group \"", "groups \""]
            .iter()
            .filter_map(|marker| rest.find(marker).map(|at| (at, marker.len())))
            .min();
        let Some((at, len)) = found else {
            out.push_str(rest);
            return out;
        };
        let open = at + len;
        out.push_str(&rest[..open]);
        out.push_str("<redacted>");
        rest = match rest[open..].find('"') {
            Some(close) => &rest[open + close..],
            None => "\"",
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_is_never_an_empty_read() {
        let refused = Source::<Vec<u8>>::from_failure(Failure::new(FailureKind::Forbidden, "no"));
        assert!(matches!(refused, Source::Refused(_)));
        assert!(refused.read().is_none());
        let missing = Source::<Vec<u8>>::from_failure(Failure::new(FailureKind::NotFound, "no"));
        assert!(matches!(missing, Source::NotInstalled(_)));
        let empty = Source::from_result(Ok(Vec::<u8>::new()));
        assert_eq!(empty.read(), Some(&Vec::new()));
        assert!(empty.why_not_read().is_none());
        let capped = Source::from_listing(Ok((vec![1u8], Some(Truncation { read: 1000 }))));
        assert_eq!(capped.read(), Some(&vec![1]));
        assert_eq!(capped.capped(), Some(Truncation { read: 1000 }));
        assert!(capped.why_not_read().is_none());
    }

    #[test]
    fn a_refusal_never_prints_who_was_refused() {
        let refused = Source::<Vec<u8>>::from_failure(Failure::new(
            FailureKind::Forbidden,
            r#"pods is forbidden: User "system:serviceaccount:acme:reader" cannot list resource "pods" in API group "" in the namespace "shop""#,
        ));
        let why = refused.why_not_read().unwrap();
        assert!(!why.contains("acme:reader"), "{why}");
        assert!(
            why.contains(r#"User "<redacted>" cannot list resource "pods""#),
            "{why}"
        );
        assert!(
            why.contains(r#"in API group "" in the namespace "shop""#),
            "{why}"
        );
        assert_eq!(
            redact_identity(r#"users "a" is forbidden: User "b" in Group "c" and groups "d"#),
            r#"users "<redacted>" is forbidden: User "<redacted>" in Group "<redacted>" and groups "<redacted>""#
        );
        assert_eq!(redact_identity("no identity here"), "no identity here");
    }
}
