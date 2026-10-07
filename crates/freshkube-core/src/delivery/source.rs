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
///
/// One pass: each ` "` is found once and the word before it checked, so a
/// message full of one marker never rescans for the others.
pub fn redact_identity(message: &str) -> String {
    const NAMES: [&str; 4] = ["User", "users", "Group", "groups"];
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    let mut from = 0;
    while let Some(found) = rest[from..].find(" \"") {
        let at = from + found;
        if !NAMES.iter().any(|name| rest[..at].ends_with(name)) {
            from = at + 1;
            continue;
        }
        let open = at + 2;
        out.push_str(&rest[..open]);
        out.push_str("<redacted>");
        let Some(close) = rest[open..].find('"') else {
            out.push('"');
            return out;
        };
        rest = &rest[open + close..];
        from = 0;
    }
    out.push_str(rest);
    out
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
/// printed: without identities, URLs, addresses, absolute paths or whatever
/// stands before an `@` (credentials, an email's local part), and cut as
/// [`redact_body`] cuts. API group names (`pipelineruns.tekton.dev`) stay:
/// only a body loses bare host names.
pub fn redact_message(message: &str) -> String {
    redact_capped(message, redact_places)
}

/// What [`redact_message`] takes out, on text already short enough.
fn redact_places(message: &str) -> String {
    redact_credentials(&redact_location(&redact_identity(message)))
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

/// The longest message [`redact_body`] and [`redact_message`] keep, their
/// ellipsis included.
pub const MAX_BODY_MESSAGE_BYTES: usize = 4 * 1024;
/// How much of a body [`redact_body`] reads. Redaction can shorten a body a
/// lot (a long URL becomes `<url>`), so it reads more than it keeps, but never
/// the whole of a body that may be 32 MiB.
const MAX_BODY_SCANNED_BYTES: usize = 4 * MAX_BODY_MESSAGE_BYTES;
/// How far back from its limit a cut looks for a space, or for the character
/// its caller accepts instead, before it cuts inside a word: a long word
/// never takes most of the text with it.
const MAX_CUT_STEP_BYTES: usize = 256;
/// What ends a message that was cut short.
const ELLIPSIS: char = '…';

/// An error body that isn't the API server's JSON, as it may be kept: a
/// proxy's or a load balancer's page names hosts without a port, so besides
/// what [`redact_message`] takes out, every bare host name goes too, with
/// IPv6 addresses wherever they stand and whatever stands before an `@`
/// (credentials, an email's local part). API groups look like host names and
/// go with them, which only an API server's own message, never such a body,
/// needs to keep.
///
/// Only the body's first 16 KiB are read, cut where no word that names a
/// place can be cut in two, and what is kept after redaction is at most
/// [`MAX_BODY_MESSAGE_BYTES`], cut at a space, with `…` when anything was
/// left out. Every pass is linear.
///
/// A cut steps back at most [`MAX_CUT_STEP_BYTES`] to find its space or
/// markup delimiter. A longer word that stands across the end of what is read
/// is cut inside and redacted as what is left of it: a URL or a path still
/// reads as one, and an `@` still takes the userinfo before it.
pub fn redact_body(body: &str) -> String {
    redact_capped(body, |text| {
        redact_hosts(&redact_ipv6(&redact_places(text)))
    })
}

/// `text` redacted by `redact` after it is read as [`redact_body`] reads a
/// body, and kept as it keeps one.
fn redact_capped(text: &str, redact: impl Fn(&str) -> String) -> String {
    let read = cut_at(text, MAX_BODY_SCANNED_BYTES, is_markup_delimiter);
    let redacted = redact(read.map_or(text, |end| &text[..end]));
    if read.is_none() && redacted.len() <= MAX_BODY_MESSAGE_BYTES {
        return redacted;
    }
    with_ellipsis(&redacted)
}

/// A failure's final message, after it was quoted and parsed again, at most
/// [`MAX_BODY_MESSAGE_BYTES`]: `{:?}` writes a control character as
/// `\u{1}`, which no JSON parser reads back, so the quoted, longer text stays.
pub(super) fn cap_message(message: String) -> String {
    if message.len() <= MAX_BODY_MESSAGE_BYTES {
        message
    } else {
        with_ellipsis(&message)
    }
}

/// `text` cut to leave room for [`ELLIPSIS`] within
/// [`MAX_BODY_MESSAGE_BYTES`], at a space, and that ellipsis.
fn with_ellipsis(text: &str) -> String {
    let limit = MAX_BODY_MESSAGE_BYTES - ELLIPSIS.len_utf8();
    let end = cut_at(text, limit, |_| true).unwrap_or(text.len());
    let mut kept = text[..end].trim_end().to_owned();
    kept.push(ELLIPSIS);
    kept
}

/// Where to cut `text` to keep at most `limit` bytes: before the last
/// whitespace that fits, else before the last character `fallback` accepts,
/// each looked for at most [`MAX_CUT_STEP_BYTES`] back, else at the last
/// character boundary that fits. `None` when it fits.
fn cut_at(text: &str, limit: usize, fallback: fn(char) -> bool) -> Option<usize> {
    if text.len() <= limit {
        return None;
    }
    let end = char_boundary_before(text, limit);
    let from = char_boundary_before(text, end.saturating_sub(MAX_CUT_STEP_BYTES));
    let before = |boundary: fn(char) -> bool| {
        if text[end..].starts_with(boundary) {
            Some(end)
        } else {
            text[from..end].rfind(boundary).map(|at| from + at)
        }
    };
    Some(
        before(char::is_whitespace)
            .or_else(|| before(fallback))
            .unwrap_or(end),
    )
}

/// The last character boundary of `text` at or before `at`.
fn char_boundary_before(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// A character that ends every run a redaction pass looks at: a host name,
/// an address, a credential, a quoted identity.
fn is_markup_delimiter(c: char) -> bool {
    matches!(c, '<' | '>' | '"' | '\'')
}

/// Replaces whatever stands before an `@` or its percent-encoding `%40`, and
/// the host after it, with `<address>`: `user:pass@db.example.com:5432`,
/// `user:p=ss@db.example.com`, `jane@example.com`, `jane%40example.com`. The
/// whole of the userinfo goes, never only its password; a field's `key=`
/// before it stays (`owner=jane@example.com`). An scp-style repository loses
/// its owner too (`git@git.example.com:acme/app.git` keeps `/app.git`). An
/// image digest (`app@sha256:…`) stays.
fn redact_credentials(text: &str) -> String {
    let in_userinfo = |c: char| {
        !c.is_whitespace()
            && !matches!(
                c,
                '@' | '"' | '\'' | '`' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
            )
    };
    let is_host_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-');
    let mut out = String::with_capacity(text.len());
    // Everything before `kept` is in `out`; userinfo is looked for back to
    // `floor`, the end of the last `@` or `%40`, so every character is
    // scanned back over once; the next `@` or `%` is looked for from `from`.
    let mut kept = 0;
    let mut floor = 0;
    let mut from = 0;
    while let Some(found) = text[from..].find(['@', '%']) {
        let at = from + found;
        let marker = if text[at..].starts_with('@') {
            1
        } else if text[at..].starts_with("%40") {
            3
        } else {
            from = at + 1;
            continue;
        };
        let run = floor + text[floor..at].trim_end_matches(in_userinfo).len();
        let start = run + userinfo_start(&text[run..at]);
        let host_at = at + marker;
        let host = &text[host_at..];
        from = host_at;
        floor = host_at;
        let digest = marker == 1 && (host.starts_with("sha256:") || host.starts_with("sha512:"));
        // An encoded `@` counts only before a host (`50%40` is no address).
        let encoded_alone = marker == 3 && !host.starts_with(|c: char| c.is_ascii_alphanumeric());
        if start == at || digest || encoded_alone {
            continue;
        }
        let len = host.find(|c: char| !is_host_char(c)).unwrap_or(host.len());
        let name = host[..len].trim_end_matches('.');
        let mut end = host_at + name.len();
        if name.len() == len {
            end = text.len() - after_port(&text[end..]).len();
            if end == host_at + len {
                end += scp_owner(&text[end..]);
            }
        }
        out.push_str(&text[kept..start]);
        out.push_str("<address>");
        kept = end;
        floor = end;
        from = end;
    }
    out.push_str(&text[kept..]);
    out
}

/// Where the userinfo starts in `run`, the characters before an `@`: after
/// the last `=` of a field's `key=` (`owner=jane`), but an `=` past the first
/// `:` is the password's (`user:p=ss`), so the whole of it goes.
fn userinfo_start(run: &str) -> usize {
    let user = run.find(':').map_or(run, |colon| &run[..colon]);
    user.rfind('=').map_or(0, |equals| equals + 1)
}

/// How much of `text`, right after an scp-style `user@host`, names the
/// repository's owner: its `:` and every directory before the last `/`
/// (`:acme/` of `:acme/app.git`, `:group/sub/` of `:group/sub/app.git`), or
/// nothing when it has no `/`.
fn scp_owner(text: &str) -> usize {
    let Some(path) = text.strip_prefix(':') else {
        return 0;
    };
    let len = path
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/' | '~')))
        .unwrap_or(path.len());
    path[..len].rfind('/').map_or(0, |slash| 1 + slash)
}

/// Replaces each IPv6 address, with its zone, wherever it stands
/// (`<b>2001:db8::1</b>`, `peer=fe80::1%eth0`), right after a field's `key:`
/// too (`peer:fd00::1`, `node:fd00::2`). A run that only looks like one inside
/// a word (`Error::new`, `std::fmt`) stays.
fn redact_ipv6(text: &str) -> String {
    let is_ip_char = |c: char| c.is_ascii_hexdigit() || matches!(c, ':' | '.');
    let is_word_char = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(is_ip_char) {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let len = tail.find(|c: char| !is_ip_char(c)).unwrap_or(tail.len());
        let run = &tail[..len];
        let in_word = out.chars().next_back().is_some_and(is_word_char);
        // The run, unless it stands inside a word; else what follows its
        // first colon, a field's `key:`, whose hex letters (`node:`) and
        // colon start the run.
        let whole = if in_word {
            None
        } else {
            ipv6_in(run).map(|address| (0, address))
        };
        let found = whole.or_else(|| {
            let key = run.find(':')? + 1;
            ipv6_in(&run[key..]).map(|address| (key, address))
        });
        let Some((key, address)) = found else {
            out.push_str(run);
            rest = &tail[len..];
            continue;
        };
        let mut after = &tail[key + address.len()..];
        if let Some(zone) = after.strip_prefix('%') {
            let zone_len = zone
                .find(|c: char| !(is_word_char(c) || matches!(c, '.' | '-')))
                .unwrap_or(zone.len());
            if zone_len > 0 {
                after = &zone[zone_len..];
            }
        }
        if after.starts_with(is_word_char) {
            out.push_str(run);
            rest = &tail[len..];
            continue;
        }
        out.push_str(&run[..key]);
        out.push_str("<address>");
        rest = after;
    }
    out.push_str(rest);
    out
}

/// The IPv6 address a run of address characters is, without a sentence's
/// full stop or a trailing colon.
fn ipv6_in(run: &str) -> Option<&str> {
    [run, run.trim_end_matches(['.', ':'])]
        .into_iter()
        .find(|candidate| {
            candidate.contains(':')
                && candidate.contains(|c: char| c.is_ascii_hexdigit())
                && candidate.parse::<std::net::Ipv6Addr>().is_ok()
        })
}

/// Replaces each run of host-name characters that is a dotted host name or
/// an IPv4 address, wherever it stands (`host=api.example.com`,
/// `<b>192.0.2.10</b>`), with a domain's leading dot and a wildcard's `*.`
/// (`.svc.cluster.local`, `*.apps.cluster.example`). Dashes before or after
/// it stay outside it (`--api.example.com`, `api.example.com-`).
fn redact_hosts(text: &str) -> String {
    let is_host_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-');
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(is_host_char) {
        out.push_str(&rest[..start]);
        let run = &rest[start..];
        let len = run.find(|c: char| !is_host_char(c)).unwrap_or(run.len());
        let (run, mut after) = run.split_at(len);
        // A flag's dashes, or a domain's or a wildcard's leading dot, before
        // the name; a dash, a fully qualified name's root dot, or a
        // sentence's full stop, after it.
        let name = run.trim_start_matches(['.', '-']);
        let lead = run.len() - name.len();
        let name = name.trim_end_matches(['.', '-']);
        if is_host_name(name) || name.parse::<std::net::Ipv4Addr>().is_ok() {
            // The dot goes with the name; the dashes before it stay.
            let dashes = run[..lead].trim_end_matches('.');
            if dashes.is_empty() && lead > 0 && out.ends_with('*') {
                out.pop();
            }
            out.push_str(dashes);
            out.push_str("<address>");
            out.push_str(&run[lead + name.len()..]);
            // Its port goes with it, when the name runs straight into one.
            if lead + name.len() == run.len() {
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
/// the last of them two characters or more and starting with a letter:
/// `proxy.internal`, `example.com`, `registry.k8s`, `api.dc1`,
/// `example.xn--p1ai`, not `1.2.3`, `e.g`, `v1.25` or `1.25.3-rc1`.
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
            .is_some_and(|tld| tld.len() >= 2 && tld.starts_with(|c: char| c.is_ascii_alphabetic()))
}

/// An IP address, an IPv6 one with its zone (`fe80::1%eth0`) too.
fn is_ip(text: &str) -> bool {
    match text.split_once('%') {
        None => text.parse::<std::net::IpAddr>().is_ok(),
        Some((address, zone)) => {
            !zone.is_empty()
                && zone
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
                && address.parse::<std::net::Ipv6Addr>().is_ok()
        }
    }
}

/// An IP address, or a host and a port.
fn is_address(word: &str) -> bool {
    if is_ip(word) {
        return true;
    }
    let Some((host, port)) = word.rsplit_once(':') else {
        return false;
    };
    if !(1..=5).contains(&port.len()) || !port.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let bracketed = host.strip_prefix('[').and_then(|ip| ip.strip_suffix(']'));
    if is_ip(bracketed.unwrap_or(host)) {
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
            "built with go1.21, image 1.25.3-rc1, at 12:30:45 from aa:bb:cc:dd:ee:ff",
            "Error::new failed in std::fmt",
            "pull app@sha256:0123abcd failed, ask @ops",
        ] {
            assert_eq!(redact_body(kept), kept);
        }
    }

    /// Accepted false positives: file names and API groups look like host
    /// names and go with them. Pinned, so a change is a decision.
    #[test]
    fn a_body_loses_file_names_and_api_groups_too() {
        assert_eq!(redact_body("read config.yaml"), "read <address>");
        assert_eq!(redact_body("panic at main.go:12"), "panic at <address>");
        assert_eq!(redact_body("see k8s.io/docs"), "see <address>/docs");
    }

    #[test]
    fn a_body_loses_a_domain_with_its_leading_dot_or_wildcard() {
        assert_eq!(
            redact_body("cert for *.apps.cluster.example expired"),
            "cert for <address> expired"
        );
        assert_eq!(
            redact_body("search .svc.cluster.local failed"),
            "search <address> failed"
        );
        assert_eq!(
            redact_body("dial *.apps.cluster.example:443"),
            "dial <address>"
        );
    }

    #[test]
    fn a_body_loses_a_host_whose_last_label_has_a_digit() {
        assert_eq!(
            redact_body("pull from registry.k8s failed; api.dc1 down; see shop.example.xn--p1ai"),
            "pull from <address> failed; <address> down; see <address>"
        );
    }

    #[test]
    fn a_body_loses_ipv6_wherever_it_stands_with_its_zone() {
        assert_eq!(
            redact_body(
                "<b>2001:db8::1</b> peer=fe80::1%eth0, via=[2001:db8::2]:443 src=::ffff:192.0.2.1."
            ),
            "<b><address></b> peer=<address>, via=[<address>]:443 src=<address>."
        );
        assert_eq!(
            redact_location("dial [fe80::1%eth0]:6443 or fe80::1%25"),
            "dial <address> or <address>"
        );
    }

    #[test]
    fn a_body_loses_credentials_and_an_emails_local_part_whole() {
        assert_eq!(
            redact_body("connect user:s3cret@db.example.com:5432 failed"),
            "connect <address> failed"
        );
        assert_eq!(redact_body("as admin:hunter2@localhost"), "as <address>");
        assert_eq!(
            redact_body("mail jane.doe+ops@example.com."),
            "mail <address>."
        );
        assert_eq!(
            redact_body("<td>owner=jane@example.com</td>"),
            "<td>owner=<address></td>"
        );
        assert_eq!(
            redact_body("fetch https://user:pass@git.example.com/repo.git"),
            "fetch <url>"
        );
    }

    #[test]
    fn a_long_body_is_cut_at_a_space() {
        let cut = redact_body(&"word ".repeat(10_000));
        assert!(
            cut.len() <= MAX_BODY_MESSAGE_BYTES + '…'.len_utf8(),
            "{}",
            cut.len()
        );
        assert!(cut.ends_with("word…"), "{cut}");
        // A word that straddles the end of what is read is left out whole, so
        // no part of it escapes redaction.
        let body = format!("https://{} user:secret@db.example.com", "x".repeat(16_370));
        assert_eq!(redact_body(&body), "<url>…");
    }

    #[test]
    fn an_adversarial_body_is_redacted_in_one_pass() {
        // The old identity scan looked for every marker again after each
        // match: quadratic on a message full of one of them.
        let message = "User \"x\" ".repeat(200_000);
        assert_eq!(
            redact_identity(&message).len(),
            200_000 * "User \"<redacted>\" ".len()
        );
        let body = "users \"a.b@c.example fe80::1%e :: *.x.example 192.0.2.1:1 ".repeat(200_000);
        let redacted = redact_body(&body);
        assert!(
            redacted.len() <= MAX_BODY_MESSAGE_BYTES + '…'.len_utf8(),
            "{}",
            redacted.len()
        );
        assert!(redacted.ends_with('…'));
        assert!(!redacted.contains("example"), "{redacted}");
    }

    #[test]
    fn a_message_keeps_its_api_group_and_loses_credentials() {
        assert_eq!(
            redact_message(
                r#"pipelineruns.tekton.dev is forbidden: User "system:serviceaccount:ci:runner" cannot list resource "pipelineruns" in API group "tekton.dev" at the cluster scope"#
            ),
            r#"pipelineruns.tekton.dev is forbidden: User "<redacted>" cannot list resource "pipelineruns" in API group "tekton.dev" at the cluster scope"#
        );
        assert_eq!(
            redact_message("repository not accessible: deploy:t0ken@git.example.com:8443 refused"),
            "repository not accessible: <address> refused"
        );
        assert_eq!(
            redact_message("rpc error: git@git.example.com:acme/payments.git: not found"),
            "rpc error: <address>/payments.git: not found"
        );
        assert_eq!(
            redact_message("synced by jane@example.com"),
            "synced by <address>"
        );
        let signed = "image registry.example/app@sha256:0123abcd is not signed";
        assert_eq!(redact_message(signed), signed);
    }

    #[test]
    fn a_long_message_is_cut_at_a_space() {
        let cut = redact_message(&"pipelineruns.tekton.dev ".repeat(1_000));
        assert!(cut.len() <= MAX_BODY_MESSAGE_BYTES, "{}", cut.len());
        assert!(cut.ends_with("pipelineruns.tekton.dev…"), "{cut}");
        let cut = redact_message(&"word ".repeat(1_000_000));
        assert!(cut.len() <= MAX_BODY_MESSAGE_BYTES, "{}", cut.len());
        assert!(cut.ends_with("word…"), "{cut}");
        assert_eq!(redact_message("pods is forbidden"), "pods is forbidden");
    }

    #[test]
    fn a_cut_steps_back_at_most_256_bytes() {
        // A space further back is not looked for: the cut falls in the word.
        let text = format!("a {}", "x".repeat(1_000));
        assert_eq!(cut_at(&text, 600, |_| false), Some(600));
        // A space, or the character accepted instead, near enough is.
        let text = format!("{} {}", "x".repeat(500), "x".repeat(500));
        assert_eq!(cut_at(&text, 600, |_| false), Some(500));
        let text = format!("{}<{}", "x".repeat(500), "x".repeat(500));
        assert_eq!(cut_at(&text, 600, is_markup_delimiter), Some(500));
        // Never inside a character.
        assert_eq!(cut_at(&"é".repeat(400), 601, |_| false), Some(600));
        // A body whose only space is far back keeps all that fits.
        let cut = redact_body(&format!("a {}", "x".repeat(20_000)));
        assert_eq!(cut.len(), MAX_BODY_MESSAGE_BYTES);
        assert!(cut.starts_with("a xxx") && cut.ends_with("x…"), "{cut}");
    }

    #[test]
    fn a_body_loses_ipv6_right_after_a_fields_key() {
        assert_eq!(
            redact_body("peer:fd00::1 node:fd00::2%eth0 dns:::1, in std::fmt"),
            "peer:<address> node:<address> dns:<address>, in std::fmt"
        );
    }

    #[test]
    fn a_body_loses_partial_userinfo_whole() {
        assert_eq!(
            redact_body("auth user:p=ss@db.example.com failed"),
            "auth <address> failed"
        );
        assert_eq!(
            redact_body("auth key=user:p=ss@db.example.com failed"),
            "auth key=<address> failed"
        );
        assert_eq!(
            redact_body("clone git@git.example.com:acme/app failed"),
            "clone <address>/app failed"
        );
        assert_eq!(
            redact_body("owner jane%40example.com and callback?email=jane%40example.com"),
            "owner <address> and callback?email=<address>"
        );
        assert_eq!(redact_body("cut 50%40 off"), "cut 50%40 off");
    }

    #[test]
    fn a_body_loses_a_host_between_dashes() {
        assert_eq!(
            redact_body("flag --api.example.com and api.example.com- end"),
            "flag --<address> and <address>- end"
        );
    }

    /// The identity scan the linear one replaced in #309, which looked for
    /// every marker again after each match.
    fn redact_identity_rescanning(message: &str) -> String {
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

    #[test]
    fn the_linear_identity_scan_matches_the_rescanning_one() {
        for (message, expected) in [
            (
                r#"User "jane" cannot list"#,
                r#"User "<redacted>" cannot list"#,
            ),
            (r#"in Group "ops""#, r#"in Group "<redacted>""#),
            (
                r#"users "jane" is forbidden: User "sa" cannot impersonate resource "users" in API group """#,
                r#"users "<redacted>" is forbidden: User "<redacted>" cannot impersonate resource "users" in API group """#,
            ),
            (
                r#"groups "ops" is forbidden"#,
                r#"groups "<redacted>" is forbidden"#,
            ),
            // Empty, adjacent, a nested quote, unterminated.
            (r#"User "" cannot"#, r#"User "<redacted>" cannot"#),
            (
                r#"User "a" Group "b""#,
                r#"User "<redacted>" Group "<redacted>""#,
            ),
            (
                r#"User ""User "x""#,
                r#"User "<redacted>"User "<redacted>""#,
            ),
            (r#"User "a"b" cannot"#, r#"User "<redacted>"b" cannot"#),
            (
                r#"forbidden: User "jane cannot list"#,
                r#"forbidden: User "<redacted>""#,
            ),
            ("no identity here", "no identity here"),
            ("", ""),
        ] {
            assert_eq!(redact_identity(message), expected, "{message}");
            assert_eq!(redact_identity_rescanning(message), expected, "{message}");
        }
    }
}
