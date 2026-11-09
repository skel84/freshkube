//! What the delivery commands share: the notes about what may be missing,
//! the spans of time the options take, and how both reach the exit code.

use chrono::Duration;
use freshkube_delivery::changes::{Gap, Note};
use serde_json::{Value, json};

use crate::exit::Findings;

/// `90s`, `30m`, `12h`, `2d`, `1w`: a span with its unit, so a bare number
/// is never guessed at.
pub fn span(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    let (number, unit) = text.split_at(digits);
    let n: i64 = number
        .parse()
        .map_err(|_| format!("\"{text}\" is not a span; write it like 90m, 12h or 2d"))?;
    let seconds = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        "w" => 604_800,
        _ => {
            return Err(format!(
                "\"{text}\" needs a unit: s, m, h, d or w (for example 2d)"
            ));
        }
    };
    n.checked_mul(seconds)
        .and_then(Duration::try_seconds)
        .ok_or_else(|| format!("\"{text}\" is too long a span"))
}

/// `3 d 4 h`, `25 m`: how long, to the two largest units.
pub fn ago(span: Duration) -> String {
    let seconds = span.num_seconds().max(0);
    let (days, hours, minutes) = (seconds / 86_400, seconds / 3600 % 24, seconds / 60 % 60);
    match (days, hours, minutes) {
        (0, 0, 0) => format!("{seconds} s"),
        (0, 0, m) => format!("{m} m"),
        (0, h, m) => format!("{h} h {m} m"),
        (d, h, _) => format!("{d} d {h} h"),
    }
}

pub fn gap_word(gap: Gap) -> &'static str {
    match gap {
        Gap::NotServed => "not-served",
        Gap::Refused => "refused",
        Gap::Unreadable => "unreadable",
        Gap::Capped => "capped",
    }
}

/// What a gap in what was read means for the exit code. Missing evidence is
/// never a failure, so a source that isn't served is as unknown as one that
/// couldn't be reached; a cap only says the list is longer.
pub fn gap_findings(gap: Gap) -> Findings {
    let mut findings = Findings::default();
    match gap {
        Gap::NotServed | Gap::Unreadable => findings.unknown = true,
        Gap::Refused => findings.refused = true,
        Gap::Capped => {}
    }
    findings
}

/// What a note means for the exit code: its gap's, unless it is about GitHub,
/// whose look-ups are optional and shown without changing the code.
pub fn note_findings(note: &Note) -> Findings {
    if note.github {
        Findings::default()
    } else {
        gap_findings(note.gap)
    }
}

pub fn notes_json<'a>(notes: impl IntoIterator<Item = &'a Note>) -> Vec<Value> {
    notes
        .into_iter()
        .map(|note| {
            json!({
                "gap": gap_word(note.gap),
                "github": note.github,
                "project": note.project,
                "text": note.text,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_need_a_unit() {
        assert_eq!(span("2d").unwrap(), Duration::days(2));
        assert_eq!(span("90m").unwrap(), Duration::minutes(90));
        assert!(span("2").is_err());
        assert!(span("d").is_err());
        assert!(span("-1d").is_err());
        assert!(span("99999999999999999999d").is_err());
    }

    #[test]
    fn spans_are_told_in_two_units() {
        assert_eq!(ago(Duration::seconds(40)), "40 s");
        assert_eq!(ago(Duration::minutes(25)), "25 m");
        assert_eq!(ago(Duration::minutes(125)), "2 h 5 m");
        assert_eq!(ago(Duration::hours(51)), "2 d 3 h");
    }
}
