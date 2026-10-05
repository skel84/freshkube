//! An application's traces, from Coroot's tracing view: a latency and error
//! heatmap, up to 100 matching spans, and the spans of one trace by id.
use super::*;
use serde::Deserialize;

/// Which spans Coroot lists beside the heatmap.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum TraceSelection {
    /// The latest spans in the window.
    #[default]
    Recent,
    /// Failed requests between two epoch milliseconds.
    Errors { from_ms: i64, to_ms: i64 },
    /// Requests between two epoch milliseconds that took longer than
    /// `above` and up to `up_to`: heatmap row bounds, in seconds or `inf`.
    Latency {
        from_ms: i64,
        to_ms: i64,
        above: String,
        up_to: String,
    },
    /// Every span of one trace.
    Trace(String),
}

impl TraceSelection {
    /// Coroot's `trace` parameter: `source:id:from-to:above-up_to`.
    fn param(&self, source: &str) -> Result<String, ReadError> {
        if !matches!(source, "" | "otel" | "agent") {
            return Err(ReadError::InvalidSelection);
        }
        Ok(match self {
            Self::Recent => format!("{source}:::"),
            Self::Errors { from_ms, to_ms } => {
                window(*from_ms, *to_ms)?;
                format!("{source}::{from_ms}-{to_ms}:inf-err")
            }
            Self::Latency {
                from_ms,
                to_ms,
                above,
                up_to,
            } => {
                window(*from_ms, *to_ms)?;
                if !bound(above) || !bound(up_to) {
                    return Err(ReadError::InvalidSelection);
                }
                format!("{source}::{from_ms}-{to_ms}:{above}-{up_to}")
            }
            Self::Trace(id) => {
                if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric())
                {
                    return Err(ReadError::InvalidSelection);
                }
                format!("{source}:{id}::")
            }
        })
    }
}

fn window(from_ms: i64, to_ms: i64) -> Result<(), ReadError> {
    if 0 < from_ms && from_ms < to_ms {
        Ok(())
    } else {
        Err(ReadError::InvalidSelection)
    }
}

/// A heatmap row bound: empty (zero), `inf`, or seconds.
fn bound(value: &str) -> bool {
    value.is_empty()
        || value == "inf"
        || (value.len() <= 16 && value.parse::<f64>().is_ok_and(|v| v.is_finite() && v >= 0.))
}

/// What Coroot answered for one application.
#[derive(Clone, Debug, Default)]
pub struct Tracing {
    /// `Unknown` when Coroot found no traces, `Warning` when it could not read them.
    pub status: Status,
    /// Coroot's note, as plain text: which service's traces, or why there are none.
    pub message: String,
    /// OpenTelemetry and eBPF, when Coroot has spans from them.
    pub sources: Vec<TraceSource>,
    pub heatmap: Option<Heatmap>,
    /// Spans matching the selection, or every span of the selected trace.
    pub spans: Vec<Span>,
    /// Coroot stopped at its limit, so more spans match.
    pub limited: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceSource {
    /// `otel` or `agent`.
    pub kind: String,
    pub name: String,
    pub selected: bool,
}

/// Requests per second over time, one row per duration bucket.
#[derive(Clone, Debug)]
pub struct Heatmap {
    pub from_ms: i64,
    pub to_ms: i64,
    pub step_ms: i64,
    /// Fastest first; the failed requests' row last.
    pub rows: Vec<HeatRow>,
}

#[derive(Clone, Debug)]
pub struct HeatRow {
    /// Coroot's label, such as `50ms` or `>5s`.
    pub name: String,
    /// The row's upper bound in seconds, `inf`, or `err` for failed requests.
    pub value: String,
    pub points: Vec<Option<f32>>,
}

impl HeatRow {
    pub fn is_errors(&self) -> bool {
        self.value == "err"
    }
}

#[derive(Deserialize)]
struct RawView {
    #[serde(default)]
    status: Status,
    #[serde(default)]
    message: String,
    #[serde(default)]
    sources: Option<Vec<RawSource>>,
    #[serde(default)]
    heatmap: Option<RawHeatmap>,
    #[serde(default)]
    spans: Option<Vec<Span>>,
    #[serde(default)]
    limit: usize,
}

#[derive(Deserialize)]
struct RawSource {
    #[serde(rename = "type")]
    kind: String,
    name: String,
    #[serde(default)]
    selected: bool,
}

#[derive(Deserialize)]
struct RawHeatmap {
    ctx: RawContext,
    #[serde(default)]
    series: Option<Vec<RawSeries>>,
}

#[derive(Deserialize)]
struct RawContext {
    from: i64,
    to: i64,
    step: i64,
}

#[derive(Deserialize)]
struct RawSeries {
    name: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    data: Option<Vec<Option<f32>>>,
}

pub(super) fn decode(data: serde_json::Value) -> Result<Tracing, ReadError> {
    if data.is_null() {
        return Err(ReadError::Missing);
    }
    let raw: RawView = serde_json::from_value(data).map_err(|_| ReadError::InvalidResponse)?;
    let heatmap = match raw.heatmap {
        Some(h) if h.ctx.step > 0 && h.ctx.from < h.ctx.to => Some(Heatmap {
            from_ms: h.ctx.from,
            to_ms: h.ctx.to,
            step_ms: h.ctx.step,
            rows: h
                .series
                .unwrap_or_default()
                .into_iter()
                .map(|s| HeatRow {
                    name: s.name,
                    value: s.value,
                    points: s.data.unwrap_or_default(),
                })
                .collect(),
        }),
        Some(_) => return Err(ReadError::InvalidResponse),
        None => None,
    };
    Ok(Tracing {
        status: raw.status,
        message: plain(&raw.message),
        sources: raw
            .sources
            .unwrap_or_default()
            .into_iter()
            .map(|s| TraceSource {
                kind: s.kind,
                name: s.name,
                selected: s.selected,
            })
            .collect(),
        heatmap,
        spans: raw.spans.unwrap_or_default(),
        limited: raw.limit > 0,
    })
}

/// Coroot's notes and chart titles carry a little HTML, such as
/// `<i>service</i>` or `<var>app</var>`. Only something shaped like a tag is
/// dropped, so `latency < 500ms` keeps its `<`; the common entities are decoded.
pub(super) fn plain(message: &str) -> String {
    if !message.contains(['<', '&']) {
        return message.into();
    }
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(at) = rest.find(['<', '&']) {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        if rest.starts_with('<') {
            let tag = rest[1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '/');
            match rest.find('>').filter(|_| tag) {
                Some(end) if !rest[1..end].contains('<') => rest = &rest[end + 1..],
                _ => {
                    out.push('<');
                    rest = &rest[1..];
                }
            }
            continue;
        }
        let entity = [
            ("&amp;", '&'),
            ("&lt;", '<'),
            ("&gt;", '>'),
            ("&quot;", '"'),
            ("&#39;", '\''),
            ("&nbsp;", ' '),
        ]
        .into_iter()
        .find(|(name, _)| rest.starts_with(name));
        match entity {
            Some((name, c)) => {
                out.push(c);
                rest = &rest[name.len()..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

impl Provider {
    /// An application's traces in `range`. `source` picks OpenTelemetry
    /// (`otel`) or eBPF (`agent`); empty lets Coroot choose.
    pub async fn tracing(
        &self,
        source: &Source,
        range: TimeRange,
        app: &AppId,
        trace_source: &str,
        selection: &TraceSelection,
    ) -> Result<Tracing, ReadError> {
        let project = self.project(source, range)?;
        let param = selection.param(trace_source)?;
        let path = format!(
            "app/{}/tracing",
            coroot_rs::util::encode_segment(app.as_str())
        );
        let envelope = self.read(project.get(&path, &[("trace", param)])).await?;
        let tracing = decode(envelope.data)?;
        limits::tracing(&tracing)?;
        Ok(tracing)
    }
}
