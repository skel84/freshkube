//! Makes `tsh`'s stderr fit to show. It may name a profile directory, a
//! certificate or key path, or carry a token or a login URL; none of that
//! reaches the screen, a log or an error.

/// The most lines and characters kept: the end of the output, where the
/// reason is.
const MAX_LINES: usize = 8;
const MAX_LINE: usize = 200;
const MAX_TOTAL: usize = 1000;

const SECRET_WORDS: [&str; 9] = [
    "password",
    "passwd",
    "token",
    "secret",
    "key",
    "cert",
    "auth",
    "credential",
    "bearer",
];

/// `text` with secrets, key and certificate paths and the `private` values
/// (such as the home directory) removed, cut to its last lines.
pub fn redact(text: &str, private: &[&str]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_pem = false;
    for line in text.lines() {
        if line.contains("-----BEGIN") {
            in_pem = true;
            out.push("<certificate removed>".into());
        }
        if in_pem {
            if line.contains("-----END") {
                in_pem = false;
            }
            continue;
        }
        let mut line = line.to_owned();
        for value in private.iter().filter(|v| v.len() > 2) {
            line = line.replace(value, "<path>");
        }
        let line = redact_line(&line);
        if !line.trim().is_empty() {
            out.push(cut(&line, MAX_LINE));
        }
    }
    let start = out.len().saturating_sub(MAX_LINES);
    cut(&out[start..].join("\n"), MAX_TOTAL)
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut kept: String = text.chars().take(max).collect();
    kept.push('…');
    kept
}

fn redact_line(line: &str) -> String {
    let mut previous = String::new();
    let mut words = Vec::new();
    for word in line.split_whitespace() {
        let lower = word.to_lowercase();
        let replaced = if SECRET_WORDS.iter().any(|w| previous.ends_with(w)) && !is_plain(&lower) {
            "<redacted>".to_owned()
        } else {
            redact_word(word)
        };
        previous = lower
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_owned();
        words.push(replaced);
    }
    words.join(" ")
}

/// A word that is only a word after a secret's name ("token expired").
fn is_plain(lower: &str) -> bool {
    const PLAIN: [&str; 24] = [
        "is", "was", "not", "no", "has", "have", "had", "expired", "invalid", "missing", "denied",
        "required", "provided", "found", "for", "to", "in", "of", "or", "and", "failed",
        "rejected", "mismatch", "error",
    ];
    PLAIN.contains(&lower.trim_matches(|c: char| !c.is_alphanumeric()))
}

fn redact_word(word: &str) -> String {
    let bare = word.trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';' | '(' | ')' | '.'));
    // key=value and key:value where the key names a secret.
    if let Some((key, value)) = bare.split_once(['=', ':'])
        && !value.is_empty()
        && !value.starts_with("//")
    {
        let key = key.to_lowercase();
        if SECRET_WORDS.iter().any(|w| key.contains(w)) {
            return format!("{}=<redacted>", key.trim_matches('-'));
        }
    }
    // A URL that carries a login.
    if bare.contains("://") && bare.contains('@') {
        return "<url>".into();
    }
    if is_path(bare) {
        return "<path>".into();
    }
    if looks_like_secret(bare) {
        return "<redacted>".into();
    }
    word.to_owned()
}

fn is_path(word: &str) -> bool {
    let lower = word.to_lowercase();
    lower.contains(".tsh")
        || lower.starts_with("~/")
        || (lower.starts_with('/') && lower.matches('/').count() >= 2)
        || (lower.contains(":\\") && lower.contains('\\'))
        || [".pem", ".key", ".crt", ".cert", ".p12"]
            .iter()
            .any(|ext| lower.ends_with(ext))
}

/// A long run of base64, hex or a JWT.
fn looks_like_secret(word: &str) -> bool {
    if word.starts_with("eyJ") && word.len() > 20 {
        return true;
    }
    word.len() >= 24
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-'))
        && word.chars().any(|c| c.is_ascii_digit())
        && word.chars().any(|c| c.is_ascii_alphabetic())
}
