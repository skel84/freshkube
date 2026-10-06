//! An application's logs, from Coroot's logs view: the messages a read
//! matches or the patterns Coroot found, with their count by severity over
//! the window. The path is only ever read: a POST to it saves the
//! application's log settings.
use super::app_view::{self, Chart, RawChart};
use super::*;
use crate::types::LogLevel;
use serde::Deserialize;
use std::collections::BTreeMap;

/// How many messages a read may ask for, as Coroot's own picker offers.
pub const LOG_LIMITS: [u32; 5] = [10, 20, 50, 100, 1000];

/// Where Coroot reads the messages from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LogOrigin {
    /// What Coroot's agent collected from the containers.
    Containers,
    /// What the application sent over OpenTelemetry.
    OpenTelemetry,
}

impl LogOrigin {
    fn wire(self) -> &'static str {
        match self {
            Self::Containers => "agent",
            Self::OpenTelemetry => "otel",
        }
    }
    fn parse(wire: &str) -> Option<Self> {
        match wire {
            "agent" => Some(Self::Containers),
            "otel" => Some(Self::OpenTelemetry),
            _ => None,
        }
    }
}

/// What Coroot lists below the histogram.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LogsMode {
    #[default]
    Messages,
    /// Messages grouped by their shape, most frequent first.
    Patterns,
}

impl LogsMode {
    fn wire(self) -> &'static str {
        match self {
            Self::Messages => "messages",
            Self::Patterns => "patterns",
        }
    }
}

/// One read of an application's logs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LogQuery {
    /// None lets Coroot choose: OpenTelemetry when the application has a
    /// service there, else the containers.
    pub origin: Option<LogOrigin>,
    pub mode: LogsMode,
    /// One of [`LOG_LIMITS`].
    pub limit: u32,
    /// Only messages from this epoch nanosecond on, from [`LogCursor::since`].
    pub since: Option<i64>,
}

impl Default for LogQuery {
    fn default() -> Self {
        Self {
            origin: None,
            mode: LogsMode::Messages,
            limit: 100,
            since: None,
        }
    }
}

impl LogQuery {
    /// Coroot's `query` parameter. No filters: the messages of the window.
    fn param(&self) -> Result<String, ReadError> {
        if !LOG_LIMITS.contains(&self.limit) || self.since.is_some_and(|since| since <= 0) {
            return Err(ReadError::InvalidSelection);
        }
        Ok(serde_json::json!({
            "source": self.origin.map_or("", LogOrigin::wire),
            "view": self.mode.wire(),
            "filters": [],
            "limit": self.limit,
            "since": self.since.map(|since| since.to_string()).unwrap_or_default(),
        })
        .to_string())
    }
}

/// What Coroot answered.
#[derive(Clone, Debug, Default)]
pub struct LogsView {
    /// `Unknown` when Coroot has no logs store or found no logs, which its
    /// message explains: information, not a failure. `Warning` when it could
    /// not read them.
    pub status: Status,
    /// Coroot's note, as plain text: which logs it used, or why there are none.
    pub message: String,
    /// The origins Coroot has messages from.
    pub origins: Vec<LogOrigin>,
    /// The origin these messages came from, when Coroot read any.
    pub origin: Option<LogOrigin>,
    /// Coroot answers with patterns when it can't read messages.
    pub mode: LogsMode,
    /// Messages per step by severity.
    pub chart: Option<Chart>,
    /// Oldest first.
    pub lines: Vec<LogLine>,
    pub patterns: Vec<LogPattern>,
    /// Coroot returned as many messages as were asked for, so more match.
    /// Its query has no order, so these are some of the matching messages,
    /// not the latest ones.
    pub capped: bool,
    /// The newest message's time in epoch nanoseconds, for the next read.
    pub max_ts: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub time_ms: i64,
    /// Coroot's severity, as it names it.
    pub severity: String,
    pub level: LogLevel,
    /// As the application wrote it, which may span several lines.
    pub message: String,
    pub attributes: BTreeMap<String, String>,
    pub trace_id: String,
}

#[derive(Clone, Debug)]
pub struct LogPattern {
    pub severity: String,
    pub level: LogLevel,
    /// One message of the shape.
    pub sample: String,
    /// Messages of this shape in the window.
    pub count: u64,
    /// The messages per step.
    pub chart: Option<Chart>,
}

/// The level of a severity Coroot names; trace and fatal fold into the
/// nearest level, and a severity Coroot couldn't tell, or one this reader
/// doesn't know, stays unknown rather than guessed from the words.
pub fn severity_level(severity: &str) -> LogLevel {
    match severity {
        "trace" | "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "warning" => LogLevel::Warning,
        "error" | "fatal" => LogLevel::Error,
        _ => LogLevel::Unknown,
    }
}

/// Where the reads so far left off, so a refresh asks only for newer
/// messages and adds none twice.
///
/// Coroot reads messages after `since`, exclusively, in nanoseconds, but
/// stamps each message only to the millisecond. A refresh therefore asks
/// from the newest message's own nanosecond, and drops a message of that
/// millisecond equal to one already taken. The one loss this can't avoid:
/// a new message equal in every part to one already taken, in the same
/// millisecond, is taken for the same message.
#[derive(Clone, Debug, Default)]
pub struct LogCursor {
    max_ts: Option<i64>,
    /// The millisecond of `max_ts`, and the messages taken in it.
    edge_ms: i64,
    edge: Vec<LogLine>,
}

/// The messages a read adds.
#[derive(Clone, Debug, Default)]
pub struct Fresh {
    /// Oldest first.
    pub lines: Vec<LogLine>,
    /// A refresh came back capped: newer messages may lie between these
    /// and the ones before.
    pub gap: bool,
}

impl LogCursor {
    /// The `since` of the next read: none for a first read.
    pub fn since(&self) -> Option<i64> {
        self.max_ts.map(|max_ts| max_ts - 1)
    }

    /// Takes the answer's messages that weren't taken before.
    pub fn take(&mut self, view: &mut LogsView) -> Fresh {
        let refresh = self.max_ts.is_some();
        let edge_ms = self.edge_ms;
        let edge = std::mem::take(&mut self.edge);
        let lines: Vec<LogLine> = std::mem::take(&mut view.lines)
            .into_iter()
            .filter(|line| {
                !refresh
                    || line.time_ms > edge_ms
                    || (line.time_ms == edge_ms && !edge.contains(line))
            })
            .collect();
        self.edge = edge;
        if let Some(max_ts) = view
            .max_ts
            .filter(|&t| self.max_ts.is_none_or(|old| t > old))
        {
            let ms = max_ts.div_euclid(1_000_000);
            if ms != self.edge_ms {
                self.edge.clear();
                self.edge_ms = ms;
            }
            self.edge
                .extend(lines.iter().filter(|line| line.time_ms == ms).cloned());
            self.max_ts = Some(max_ts);
        }
        Fresh {
            lines,
            gap: refresh && view.capped,
        }
    }
}

#[derive(Deserialize)]
struct RawView {
    #[serde(default)]
    status: Status,
    #[serde(default)]
    message: String,
    #[serde(default)]
    sources: Option<Vec<String>>,
    #[serde(default)]
    source: String,
    #[serde(default)]
    view: String,
    #[serde(default)]
    chart: Option<RawChart>,
    #[serde(default)]
    entries: Option<Vec<RawEntry>>,
    #[serde(default)]
    patterns: Option<Vec<RawPattern>>,
    #[serde(default)]
    limit: u32,
    #[serde(default)]
    max_ts: String,
}

#[derive(Deserialize)]
struct RawEntry {
    timestamp: i64,
    #[serde(default)]
    severity: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    attributes: Option<BTreeMap<String, String>>,
    #[serde(default)]
    trace_id: String,
}

#[derive(Deserialize)]
struct RawPattern {
    #[serde(default)]
    severity: String,
    #[serde(default)]
    sample: String,
    #[serde(default)]
    sum: u64,
    #[serde(default)]
    chart: Option<RawChart>,
}

pub(super) fn decode(data: serde_json::Value) -> Result<LogsView, ReadError> {
    if data.is_null() {
        return Err(ReadError::Missing);
    }
    let raw: RawView = serde_json::from_value(data).map_err(|_| ReadError::InvalidResponse)?;
    let max_ts = match raw.max_ts.as_str() {
        "" => None,
        ts => Some(
            ts.parse::<i64>()
                .ok()
                .filter(|&ts| ts > 0)
                .ok_or(ReadError::InvalidResponse)?,
        ),
    };
    let mut lines: Vec<LogLine> = raw
        .entries
        .unwrap_or_default()
        .into_iter()
        .map(|e| LogLine {
            time_ms: e.timestamp,
            level: severity_level(&e.severity),
            severity: e.severity,
            message: e.message,
            attributes: e.attributes.unwrap_or_default(),
            trace_id: e.trace_id,
        })
        .collect();
    lines.sort_by_key(|line| line.time_ms);
    Ok(LogsView {
        status: raw.status,
        message: tracing::plain(&raw.message),
        origins: raw
            .sources
            .unwrap_or_default()
            .iter()
            .filter_map(|s| LogOrigin::parse(s))
            .collect(),
        origin: LogOrigin::parse(&raw.source),
        mode: if raw.view == "patterns" {
            LogsMode::Patterns
        } else {
            LogsMode::Messages
        },
        chart: raw.chart.map(app_view::chart).transpose()?,
        lines,
        patterns: raw
            .patterns
            .unwrap_or_default()
            .into_iter()
            .map(|p| {
                Ok(LogPattern {
                    level: severity_level(&p.severity),
                    severity: p.severity,
                    sample: p.sample,
                    count: p.sum,
                    chart: p.chart.map(app_view::chart).transpose()?,
                })
            })
            .collect::<Result<_, ReadError>>()?,
        capped: raw.limit > 0,
        max_ts,
    })
}

impl Provider {
    /// An application's logs in `range`.
    pub async fn logs(
        &self,
        source: &Source,
        range: TimeRange,
        app: &AppId,
        query: &LogQuery,
    ) -> Result<LogsView, ReadError> {
        let project = self.project(source, range)?;
        let param = query.param()?;
        let path = format!("app/{}/logs", coroot_rs::util::encode_segment(app.as_str()));
        let envelope = self.read(project.get(&path, &[("query", param)])).await?;
        let logs = decode(envelope.data)?;
        limits::logs(&logs)?;
        Ok(logs)
    }
}
