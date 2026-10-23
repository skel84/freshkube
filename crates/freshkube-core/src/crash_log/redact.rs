//! Takes credentials out of text before it reaches the log file or the
//! diagnostics a person copies. It works on what a log line looks like, never
//! on what a value is: a name that says it holds a secret loses its value, and
//! the few shapes a secret has on its own (a bearer header, a JWT, a PEM key,
//! a URL's password) are cut wherever they appear. Over-redacting is the safe
//! side.

/// What replaces a redacted value.
pub const REDACTED: &str = "[redacted]";

/// Names whose value is a credential, compared without case. A name matches
/// whole, or as the end of a longer name joined by `_`, `-` or `.`
/// (`access_token`, `client-key-data`).
const SECRET_NAMES: &[&str] = &[
    "token",
    "password",
    "passwd",
    "passphrase",
    "secret",
    "authorization",
    "cookie",
    "apikey",
    "api_key",
    "api-key",
    "credential",
    "credentials",
    "key",
    "key-data",
    "key_data",
    "certificate-data",
    "certificate_data",
    "authority-data",
    "authority_data",
    "private_key",
    "private-key",
];

/// Prefixes of tokens that name their own kind.
const TOKEN_PREFIXES: &[&str] = &[
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "AKIA",
    "sk-",
];

/// The shortest unbroken base64-looking run that is taken for a key or a
/// token without a name. Digests are hex and shorter than this when they are
/// written as SHA-256; hex is spared anyway.
const BLOB_LEAST: usize = 64;

/// The text with every credential in it replaced by [`REDACTED`], and ANSI
/// colour codes removed (a colour code splits `token=` from its value).
pub fn redact(text: &str) -> String {
    let text = strip_ansi(text);
    let text = redact_pem(&text);
    text.split_inclusive('\n').map(redact_line).collect()
}

fn redact_line(line: &str) -> String {
    let line = redact_bearer(line);
    let line = redact_url_password(&line);
    let line = redact_named(&line);
    redact_shapes(&line)
}

/// Removes `ESC [ … letter` sequences.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A private key block, from its BEGIN line to its END line, or to the end of
/// the text when the END never came.
fn redact_pem(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("-----BEGIN ") {
        let (before, from) = rest.split_at(start);
        let header_end = from[11..].find("-----").map(|at| 11 + at + 5);
        let is_key = header_end.is_some_and(|end| {
            let label = &from[11..end - 5];
            label.contains("PRIVATE KEY") || label.contains("CERTIFICATE") || label == "KEY"
        });
        out.push_str(before);
        if !is_key {
            out.push_str("-----BEGIN ");
            rest = &from[11..];
            continue;
        }
        out.push_str(REDACTED);
        rest = match from.find("-----END ") {
            Some(end) => match from[end + 9..].find("-----") {
                Some(close) => &from[end + 9 + close + 5..],
                None => "",
            },
            None => "",
        };
    }
    out.push_str(rest);
    out
}

fn is_value_end(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\'' | '\\' | ',' | '}' | ']' | ')' | '&' | ';' | '>'
        )
}

/// `Bearer x` and `Basic x` lose x.
fn redact_bearer(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let lower = line.to_ascii_lowercase();
    let mut at = 0;
    while at < line.len() {
        let hit = ["bearer ", "basic "]
            .iter()
            .filter_map(|scheme| lower[at..].find(scheme).map(|i| (at + i, scheme.len())))
            .min();
        let Some((start, len)) = hit else { break };
        let word_start = start == 0 || !line.as_bytes()[start - 1].is_ascii_alphanumeric();
        let value_start = start + len;
        out.push_str(&line[at..value_start]);
        at = value_start;
        if !word_start {
            continue;
        }
        let value_end = line[value_start..]
            .find(is_value_end)
            .map_or(line.len(), |i| value_start + i);
        let credential = &line[value_start..value_end];
        // "basic" is also a word; only a credential-shaped value follows it.
        let plain_word = len == "basic ".len() && !looks_like_basic(credential);
        if value_end > value_start && !plain_word {
            out.push_str(REDACTED);
            at = value_end;
        }
    }
    out.push_str(&line[at..]);
    out
}

/// The base64 of `user:password`: a run with a digit or base64 punctuation,
/// which an English word after "basic" does not have.
fn looks_like_basic(value: &str) -> bool {
    value.len() >= 12
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_'))
        && value
            .bytes()
            .any(|b| b.is_ascii_digit() || matches!(b, b'+' | b'/' | b'='))
}

/// `scheme://user:password@host` loses the user and the password.
fn redact_url_password(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find("://") {
        let after = i + 3;
        out.push_str(&rest[..after]);
        rest = &rest[after..];
        let authority_end = rest
            .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace() || c == '"')
            .unwrap_or(rest.len());
        if let Some(at) = rest[..authority_end].rfind('@') {
            out.push_str(REDACTED);
            out.push('@');
            rest = &rest[at + 1..];
        }
    }
    out.push_str(rest);
    out
}

/// Whether `name` is, or ends in, one of the secret names.
fn is_secret_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    SECRET_NAMES.iter().any(|secret| {
        name == *secret
            || name
                .strip_suffix(secret)
                .is_some_and(|head| head.ends_with(['_', '-', '.']))
    })
}

/// Names whose value runs to the end of the line (or the closing quote):
/// a cookie header holds several `a=b` pairs and a space-separated scheme.
fn is_whole_line_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with("cookie") || name == "authorization"
}

/// `name=value`, `name: value`, `"name":"value"`, escaped JSON
/// (`\"name\":\"value\"`), `name: Some("value")` and `name = "two words"`
/// for a secret name lose the value. A value that opens with a quote is read
/// to the matching close quote; any other ends at a delimiter.
fn redact_named(line: &str) -> String {
    let b = line.as_bytes();
    let n = b.len();
    let mut out = String::with_capacity(n);
    let mut copied = 0;
    let mut i = 0;
    while i < n {
        if !is_name_byte(b[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && is_name_byte(b[i]) {
            i += 1;
        }
        let name = &line[start..i];
        if !is_secret_name(name) {
            continue;
        }
        // The separator: closing quotes and escapes, spaces, `=` or `:`.
        let mut j = i;
        while j < n && matches!(b[j], b'"' | b'\'' | b'\\') {
            j += 1;
        }
        while j < n && b[j] == b' ' {
            j += 1;
        }
        if j >= n || !matches!(b[j], b'=' | b':') {
            continue;
        }
        j += 1;
        while j < n && b[j] == b' ' {
            j += 1;
        }
        if line[j..].starts_with("Some(") {
            j += 5;
        }
        while j < n && b[j] == b'\\' {
            j += 1;
        }
        let quote = (j < n && matches!(b[j], b'"' | b'\'')).then(|| b[j]);
        if quote.is_some() {
            j += 1;
        }
        let value_start = j;
        let mut end = match quote {
            Some(q) => b[j..].iter().position(|c| *c == q).map_or(n, |at| j + at),
            None if is_whole_line_name(name) => b[j..]
                .iter()
                .position(|c| matches!(c, b'"' | b'\n' | b'\r'))
                .map_or(n, |at| j + at),
            None => line[j..].find(is_value_end).map_or(n, |at| j + at),
        };
        // An escaped closing quote leaves its backslash on the value.
        while end > value_start && b[end - 1] == b'\\' {
            end -= 1;
        }
        if end == value_start || line[value_start..end] == *REDACTED {
            i = end.max(i);
            continue;
        }
        out.push_str(&line[copied..value_start]);
        out.push_str(REDACTED);
        copied = end;
        i = end;
    }
    out.push_str(&line[copied..]);
    out
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')
}

/// A JWT, a token with a known prefix and a long base64 run lose themselves,
/// named or not.
fn redact_shapes(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if looks_secret(word) {
            out.push_str(REDACTED);
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in line.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-' | '.') {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

fn looks_secret(word: &str) -> bool {
    if word.len() < 12 {
        return false;
    }
    if word.starts_with("eyJ") && word.matches('.').count() >= 2 {
        return true;
    }
    if TOKEN_PREFIXES
        .iter()
        .any(|prefix| word.starts_with(prefix) && word.len() >= prefix.len() + 16)
    {
        return true;
    }
    word.len() >= BLOB_LEAST
        && !word.contains('.')
        && !word.bytes().all(|b| b.is_ascii_hexdigit())
        && word.bytes().any(|b| b.is_ascii_uppercase())
        && word.bytes().any(|b| b.is_ascii_lowercase())
        && word.bytes().any(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A value shaped like the tokens the app handles; nothing about it is
    /// special to the redactor.
    const SEED: &str = "s3cr3tTokenValue9f8e7d6c5b4a";

    fn gone(text: &str) {
        let out = redact(text);
        assert!(!out.contains(SEED), "{text:?} -> {out:?}");
    }

    #[test]
    fn a_named_secret_loses_its_value_in_every_shape() {
        for text in [
            format!("token={SEED}"),
            format!("INFO connecting token: {SEED} to host"),
            format!(r#"{{"token":"{SEED}","name":"x"}}"#),
            format!(r#"{{"access_token": "{SEED}"}}"#),
            format!("client-key-data: {SEED}"),
            format!("https://host/path?a=1&token={SEED}&b=2"),
            format!("password = '{SEED}'"),
            format!("api_key={SEED}"),
            format!("Authorization: Bearer {SEED}"),
            format!("authorization=Basic {SEED}"),
            format!("Cookie: session={SEED}; other=1"),
            format!("https://user:{SEED}@registry.example/x"),
            format!("error: token={SEED}: refused"),
        ] {
            gone(&text);
        }
    }

    #[test]
    fn what_is_not_secret_is_kept() {
        let text = "INFO Using context: prod-ams\nWARN watch of pods ended: closed by server\n";
        assert_eq!(redact(text), text);
        let text = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(redact(text), text);
        assert_eq!(
            redact("monkey=1 tokens=3 keyboard: on"),
            "monkey=1 tokens=3 keyboard: on"
        );
    }

    #[test]
    fn the_name_stays_and_the_line_keeps_its_shape() {
        assert_eq!(
            redact("a token=abc b=2\n"),
            format!("a token={REDACTED} b=2\n")
        );
    }

    #[test]
    fn a_jwt_and_a_prefixed_token_go_without_a_name() {
        let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJl";
        assert_eq!(
            redact(&format!("saw {jwt} here")),
            format!("saw {REDACTED} here")
        );
        let pat = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        assert!(!redact(&format!("x {pat}")).contains("abcdefgh"));
    }

    #[test]
    fn a_long_base64_run_goes() {
        let blob =
            "Q2VydGlmaWNhdGVEYXRhQ2VydGlmaWNhdGVEYXRhQ2VydGlmaWNhdGVEYXRhQ2VydGlmaWNhdGVEYXRh";
        assert!(!redact(&format!("data {blob}")).contains("Q2VydGlm"));
    }

    #[test]
    fn a_private_key_block_goes_whole() {
        let text = format!(
            "before\n-----BEGIN PRIVATE KEY-----\n{SEED}\nabc\n-----END PRIVATE KEY-----\nafter\n"
        );
        let out = redact(&text);
        assert!(!out.contains(SEED) && !out.contains("PRIVATE"), "{out}");
        assert!(out.contains("before") && out.contains("after"));
        // An unterminated block takes the rest.
        let cut = redact(&format!("x\n-----BEGIN RSA PRIVATE KEY-----\n{SEED}"));
        assert!(!cut.contains(SEED));
    }

    #[test]
    fn escaped_json_and_rust_debug_shapes_lose_their_value() {
        for text in [
            format!(r#"body: "{{\"token\":\"{SEED}\"}}""#),
            format!(r#"{{\"password\": \"{SEED}\", \"user\": \"x\"}}"#),
            format!(r#"auth {{ token: Some("{SEED}") }}"#),
            format!(r#"password="{SEED} and more words""#),
            format!("Cookie: a=1; b={SEED}"),
            format!("Set-Cookie: sid={SEED}; Path=/"),
        ] {
            gone(&text);
        }
        assert_eq!(
            redact(r#"password="two words" after"#),
            format!(r#"password="{REDACTED}" after"#)
        );
        assert!(redact(r#"auth { token: Some("abc") } x"#).ends_with(") } x"));
        assert!(redact(r#"{\"token\":\"abc\",\"name\":\"keep\"}"#).contains("keep"));
    }

    #[test]
    fn the_word_basic_is_not_a_credential() {
        let text = "uses basic configuration for the basic setup";
        assert_eq!(redact(text), text);
        assert!(!redact(&format!("Authorization: Basic {SEED}==")).contains(SEED));
    }

    #[test]
    fn colour_codes_do_not_hide_a_secret() {
        let coloured = format!("\u{1b}[3mtoken\u{1b}[0m\u{1b}[2m=\u{1b}[0m{SEED}");
        gone(&coloured);
        assert!(!redact(&coloured).contains('\u{1b}'));
    }
}
