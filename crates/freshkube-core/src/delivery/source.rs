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
        let message = printable(&failure);
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

/// A failure's message as it may be printed. A kubeconfig that could not be
/// read and a server that could not be reached are named by kind alone, since
/// their messages carry file paths, context names and addresses; any other
/// message loses identities, URLs, addresses and absolute paths.
pub fn printable(failure: &Failure) -> String {
    match failure.kind {
        FailureKind::Config => {
            "the kubeconfig could not be read, or does not describe the context".into()
        }
        FailureKind::Unreachable => "the server could not be reached".into(),
        FailureKind::Timeout => "the request timed out".into(),
        _ => redact_message(&failure.message),
    }
}

/// Text a cluster object carries, such as a status message, as it may be
/// printed: without identities, URLs, addresses or absolute paths.
pub fn redact_message(message: &str) -> String {
    redact_location(&redact_identity(message))
}

/// A failure as it is printed: its kind and [`printable`] message.
pub fn shown(failure: &Failure) -> String {
    Failure::new(failure.kind, printable(failure)).to_string()
}

/// Replaces each word that names a place: a URL, an absolute or home-relative
/// path, an IP address, or a `host:port`.
pub fn redact_location(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    while !rest.is_empty() {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (word, after) = rest.split_at(end);
        out.push_str(&redact_word(word));
        let space = after.len() - after.trim_start().len();
        out.push_str(&after[..space]);
        rest = &after[space..];
    }
    out
}

fn redact_word(word: &str) -> String {
    let wrapper = |c: char| matches!(c, '"' | '\'' | '(' | ')' | '<' | '>' | ',' | ';' | '`');
    let start = word.len() - word.trim_start_matches(wrapper).len();
    let core = word[start..].trim_end_matches(|c: char| wrapper(c) || matches!(c, '.' | ':'));
    let end = start + core.len();
    let replacement = if core.contains("://") {
        "<url>"
    } else if (core.starts_with('/') || core.starts_with("~/")) && core.len() > 1 {
        "<path>"
    } else if is_address(core) {
        "<address>"
    } else {
        return word.to_owned();
    };
    format!("{}{replacement}{}", &word[..start], &word[end..])
}

/// An error body that isn't the API server's JSON, as it may be kept: a
/// proxy's or a load balancer's page names hosts without a port, so besides
/// what [`redact_message`] takes out, every bare host name goes too. API
/// groups look like host names and go with them, which only an API server's
/// own message, never such a body, needs to keep.
pub fn redact_body(body: &str) -> String {
    redact_hosts(&redact_message(body))
}

/// Replaces each run of host-name characters that is a dotted host name or
/// an IPv4 address, wherever it stands (`host=api.example.com`,
/// `<b>192.0.2.10</b>`).
fn redact_hosts(text: &str) -> String {
    let is_host_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-');
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(is_host_char) {
        out.push_str(&rest[..start]);
        let run = &rest[start..];
        let len = run.find(|c: char| !is_host_char(c)).unwrap_or(run.len());
        let (run, mut after) = run.split_at(len);
        // A fully qualified name's root dot, or a sentence's full stop.
        let name = run.trim_end_matches('.');
        if is_host_name(name) || name.parse::<std::net::Ipv4Addr>().is_ok() {
            out.push_str("<address>");
            out.push_str(&run[name.len()..]);
            // Its port goes with it, when the name runs straight into one.
            if name.len() == run.len() {
                after = after_port(after);
            }
        } else {
            out.push_str(run);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// `text` after a leading `:port` of 1 to 5 digits, or all of it.
fn after_port(text: &str) -> &str {
    let Some(digits) = text.strip_prefix(':') else {
        return text;
    };
    let len = digits.len()
        - digits
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .len();
    let next = digits[len..].chars().next();
    if (1..=5).contains(&len) && !next.is_some_and(|c| c.is_ascii_alphanumeric()) {
        &digits[len..]
    } else {
        text
    }
}

/// Two or more DNS labels, each 1 to 63 letters, digits and inner hyphens,
/// the last of them two letters or more: `proxy.internal`, `example.com`,
/// not `1.2.3`, `e.g` or `v1.25`.
fn is_host_name(name: &str) -> bool {
    let labels: Vec<&str> = name.split('.').collect();
    let label = |label: &&str| {
        (1..=63).contains(&label.len())
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    };
    labels.len() >= 2
        && labels.iter().all(label)
        && labels
            .last()
            .is_some_and(|tld| tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_alphabetic()))
}

/// An IP address, or a host and a port.
fn is_address(word: &str) -> bool {
    if word.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    let Some((host, port)) = word.rsplit_once(':') else {
        return false;
    };
    if !(1..=5).contains(&port.len()) || !port.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let bracketed = host.strip_prefix('[').and_then(|ip| ip.strip_suffix(']'));
    if bracketed
        .unwrap_or(host)
        .parse::<std::net::IpAddr>()
        .is_ok()
    {
        return true;
    }
    // A host name counts only with a port: `argoproj.io` is an API group.
    host.bytes().any(|b| b.is_ascii_alphabetic())
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
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

    #[test]
    fn a_printed_failure_never_names_who_was_refused() {
        let failure = Failure::new(
            FailureKind::Forbidden,
            r#"applications.argoproj.io is forbidden: User "jane@example.test" cannot list resource "applications""#,
        );
        let printed = shown(&failure);
        assert!(!printed.contains("jane"), "{printed}");
        assert!(printed.starts_with("Forbidden · "), "{printed}");
        assert!(
            printed.contains(r#"User "<redacted>" cannot list resource "applications""#),
            "{printed}"
        );
    }

    #[test]
    fn a_printed_failure_never_names_a_file_or_an_address() {
        for (kind, message) in [
            (
                FailureKind::Config,
                "/home/someone/.kube/config: invalid YAML",
            ),
            (
                FailureKind::Config,
                "Context 'someone@cluster' was not found",
            ),
            (
                FailureKind::Unreachable,
                "error trying to connect: tcp connect error to api.cluster.example.test:6443",
            ),
            (FailureKind::Timeout, "https://10.0.0.1:6443/apis timed out"),
        ] {
            let printed = shown(&Failure::new(kind, message));
            for word in ["home", "someone", "cluster", "10.0.0.1"] {
                assert!(!printed.contains(word), "{printed}");
            }
        }
        assert_eq!(
            redact_location(
                r#"Get "https://api.example.test:6443/api": read ~/.kube/config and /etc/x, dial 192.0.2.1:443 or [2001:db8::1]:6443 (2001:db8::1)."#
            ),
            r#"Get "<url>": read <path> and <path>, dial <address> or <address> (<address>)."#
        );
        // Names that are not addresses stay: an API group, a ratio, a time.
        let kept = r#"resource "pipelineruns" in API group "tekton.dev": 3:14, ready 1/2"#;
        assert_eq!(redact_location(kept), kept);
        assert_eq!(
            redact_location("dial api-server.internal:6443"),
            "dial <address>"
        );
    }

    #[test]
    fn a_body_loses_bare_host_names_as_it_loses_host_and_port() {
        // A bare host name.
        assert_eq!(
            redact_body("upstream proxy.internal refused the connection"),
            "upstream <address> refused the connection"
        );
        // A fully qualified one, its root dot kept, and one ending a sentence.
        assert_eq!(
            redact_body("no route to api.cluster.example.com. from gw-1.example.net."),
            "no route to <address>. from <address>."
        );
        // An IP address, alone and inside markup or a field.
        assert_eq!(
            redact_body("<p>backend 192.0.2.10 down</p> peer=198.51.100.7"),
            "<p>backend <address> down</p> peer=<address>"
        );
        assert_eq!(redact_body("dial 2001:db8::1"), "dial <address>");
        // A host and a port, as before.
        assert_eq!(
            redact_body("dial tcp api.example.com:6443: i/o timeout"),
            "dial tcp <address>: i/o timeout"
        );
        // Wherever it stands: in quotes, markup and a field.
        assert_eq!(
            redact_body(r#"<h1>502</h1> host="lb.example.org" via=edge-2.example.com"#),
            r#"<h1>502</h1> host="<address>" via=<address>"#
        );
        assert_eq!(
            redact_body("Get https://api.example.com/apis: from /etc/proxy.conf"),
            "Get <url>: from <path>"
        );
    }

    #[test]
    fn a_body_keeps_what_names_no_place() {
        for kept in [
            "502 Bad Gateway",
            "upstream connect error or disconnect/reset before headers. reset reason: overflow",
            "nginx/1.25.3 on HTTP/1.1",
            "retry in 3:14, ready 1/2, version v1.25 or 1.2.3",
            "e.g. a timeout, i.e. no answer.",
            "Service Unavailable: try-again-later",
        ] {
            assert_eq!(redact_body(kept), kept);
        }
    }
}
