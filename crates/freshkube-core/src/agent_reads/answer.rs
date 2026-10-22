//! What a read answers: the wire body, and the place the window would go to
//! show it. The bounds every answer keeps are applied here, to live and
//! example answers alike.

use serde::Serialize;
use serde_json::Value;

use super::refusal::{Code, Refusal};
use crate::constants::agent::{
    MAX_ANSWER_BYTES, MAX_EVENT_MESSAGE_BYTES, MAX_LINE_BYTES, MAX_LOG_BYTES, MAX_LOG_LINES,
    MAX_OBJECT_BYTES,
};

/// What a cut answer was cut by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CutBy {
    Items,
    Lines,
    Bytes,
}

/// `{by, limit}`: the answer is still a success, and this says it is partial.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Cut {
    pub by: CutBy,
    pub limit: u64,
}

impl Cut {
    pub fn items(limit: usize) -> Self {
        Self {
            by: CutBy::Items,
            limit: limit as u64,
        }
    }

    pub fn lines(limit: usize) -> Self {
        Self {
            by: CutBy::Lines,
            limit: limit as u64,
        }
    }

    pub fn bytes(limit: usize) -> Self {
        Self {
            by: CutBy::Bytes,
            limit: limit as u64,
        }
    }
}

/// One row of `list_objects`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Item {
    pub namespace: Option<String>,
    pub name: String,
    pub uid: String,
    pub status: String,
    pub created: Option<String>,
}

/// One row of `list_events`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EventRow {
    #[serde(rename = "type")]
    pub event_type: String,
    pub reason: String,
    /// `Kind/name`.
    pub object: String,
    pub message: String,
    pub count: u32,
    pub last: Option<String>,
}

/// The wire answer of each tool, without its read id, which the endpoint
/// gives it ([`Body::wire`]).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Body {
    List {
        cluster: String,
        kind: String,
        items: Vec<Item>,
        cut: Option<Cut>,
    },
    Get {
        cluster: String,
        kind: String,
        namespace: Option<String>,
        name: String,
        uid: String,
        object: Value,
    },
    Logs {
        cluster: String,
        namespace: String,
        pod: String,
        container: Option<String>,
        previous: bool,
        lines: Vec<String>,
        cut: Option<Cut>,
    },
    Events {
        cluster: String,
        events: Vec<EventRow>,
        cut: Option<Cut>,
    },
}

impl Body {
    /// The JSON the agent reads, with its read id first.
    pub fn wire(&self, read: &str) -> Value {
        let mut wire = serde_json::Map::new();
        wire.insert("read".into(), Value::String(read.to_owned()));
        if let Ok(Value::Object(body)) = serde_json::to_value(self) {
            wire.extend(body);
        }
        Value::Object(wire)
    }

    /// The cut, if the answer is partial.
    pub fn cut(&self) -> Option<Cut> {
        match self {
            Self::Get { .. } => None,
            Self::List { cut, .. } | Self::Logs { cut, .. } | Self::Events { cut, .. } => *cut,
        }
    }

    fn json_bytes(&self) -> usize {
        // The read id is at most a few bytes more than this.
        serde_json::to_vec(self).map_or(usize::MAX, |bytes| bytes.len() + 32)
    }
}

/// Which tab of the page the window would show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Details,
    Events,
    Logs,
}

/// Where the window would go to show what was read: the kind's key, the
/// namespace, the object and its UID when the read named one, and the tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub kind: String,
    pub namespace: Option<String>,
    pub name: Option<String>,
    pub uid: Option<String>,
    pub tab: Tab,
}

/// A successful read.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub body: Body,
    pub place: Place,
}

/// Cuts `text` to at most `max` bytes at a character boundary.
pub(super) fn cut_text(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// An event message, cut at its limit; whether it was cut.
pub(super) fn event_message(message: &str) -> (String, bool) {
    let message = message.trim_end();
    let cut = cut_text(message, MAX_EVENT_MESSAGE_BYTES);
    (cut.to_owned(), cut.len() < message.len())
}

/// A log read's text as bounded lines, the latest `tail` of them. `server_cut`
/// says the server stopped at its byte limit.
pub(super) fn log_lines(text: &str, tail: usize, server_cut: bool) -> (Vec<String>, Option<Cut>) {
    let tail = tail.min(MAX_LOG_LINES);
    let mut cut = server_cut.then(|| Cut::bytes(MAX_LOG_BYTES));
    let all: Vec<&str> = text.lines().collect();
    let kept = if all.len() > tail {
        cut.get_or_insert(Cut::lines(tail));
        &all[all.len() - tail..]
    } else {
        &all[..]
    };
    // Newest lines win the byte budget.
    let mut lines: Vec<String> = Vec::new();
    let mut bytes = 0;
    for line in kept.iter().rev() {
        let line = {
            let short = cut_text(line, MAX_LINE_BYTES);
            if short.len() < line.len() {
                cut.get_or_insert(Cut::bytes(MAX_LINE_BYTES));
            }
            short
        };
        if bytes + line.len() > MAX_LOG_BYTES {
            cut = Some(Cut::bytes(MAX_LOG_BYTES));
            break;
        }
        bytes += line.len();
        lines.push(line.to_owned());
    }
    lines.reverse();
    (lines, cut)
}

/// Checks the object limit; an object over it is `too_large`.
pub(super) fn check_object(object: &Value) -> Result<(), Refusal> {
    let size = serde_json::to_vec(object).map_or(usize::MAX, |bytes| bytes.len());
    if size > MAX_OBJECT_BYTES {
        return Err(Refusal::new(
            Code::TooLarge,
            "the object is larger than an answer may be; read its events or logs instead",
        ));
    }
    Ok(())
}

/// Holds an answer to the whole-answer limit, dropping items or lines from
/// the end of a list (the start of a log) until it fits.
pub(super) fn fit(mut body: Body) -> Result<Body, Refusal> {
    while body.json_bytes() > MAX_ANSWER_BYTES {
        match &mut body {
            Body::List { items, cut, .. } => {
                if items.pop().is_none() {
                    break;
                }
                cut.get_or_insert(Cut::bytes(MAX_ANSWER_BYTES));
            }
            Body::Events { events, cut, .. } => {
                if events.pop().is_none() {
                    break;
                }
                cut.get_or_insert(Cut::bytes(MAX_ANSWER_BYTES));
            }
            Body::Logs { lines, cut, .. } => {
                if lines.is_empty() {
                    break;
                }
                lines.remove(0);
                cut.get_or_insert(Cut::bytes(MAX_ANSWER_BYTES));
            }
            Body::Get { .. } => break,
        }
    }
    if body.json_bytes() > MAX_ANSWER_BYTES {
        return Err(Refusal::new(
            Code::TooLarge,
            "the answer is larger than an answer may be",
        ));
    }
    Ok(body)
}
