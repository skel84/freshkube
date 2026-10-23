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
    c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '}' | ']' | ')' | '&' | ';' | '>')
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
        if value_end > value_start {
            out.push_str(REDACTED);
            at = value_end;
        }
    }
    out.push_str(&line[at..]);
    out
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

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
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

/// `name=value`, `name: value`, `"name":"value"` and `name = "value"` for a
/// secret name lose the value.
fn redact_named(line: &str) -> String {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut out = String::with_capacity(line.len());
    let mut copied = 0;
    let mut i = 0;
    while i < chars.len() {
        if !is_name_char(chars[i].1) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_name_char(chars[i].1) {
            i += 1;
        }
        let name = &line[chars[start].0..chars.get(i).map_or(line.len(), |c| c.0)];
        if !is_secret_name(name) {
            continue;
        }
        // The separator: an optional closing quote, spaces, `=` or `:`.
        let mut j = i;
        while j < chars.len() && matches!(chars[j].1, '"' | '\'') {
            j += 1;
        }
        while j < chars.len() && chars[j].1 == ' ' {
            j += 1;
        }
        if j >= chars.len() || !matches!(chars[j].1, '=' | ':') {
            continue;
        }
        j += 1;
        while j < chars.len() && (chars[j].1 == ' ' || matches!(chars[j].1, '"' | '\'')) {
            j += 1;
        }
        let value_start = chars.get(j).map_or(line.len(), |c| c.0);
        let mut k = j;
        while k < chars.len() && !is_value_end(chars[k].1) {
            k += 1;
        }
        let value_end = chars.get(k).map_or(line.len(), |c| c.0);
        if value_end == value_start || line[value_start..value_end] == *REDACTED {
            i = k.max(i);
            continue;
        }
        out.push_str(&line[copied..value_start]);
        out.push_str(REDACTED);
        copied = value_end;
        i = k;
    }
    out.push_str(&line[copied..]);
    out
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
    fn colour_codes_do_not_hide_a_secret() {
        let coloured = format!("\u{1b}[3mtoken\u{1b}[0m\u{1b}[2m=\u{1b}[0m{SEED}");
        gone(&coloured);
        assert!(!redact(&coloured).contains('\u{1b}'));
    }
}
