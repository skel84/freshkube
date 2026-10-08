//! An application's messages from Coroot, for the Application page's Logs
//! report. Coroot is read on request, never streamed: the page reads a
//! window, and a refresh reads only what is newer. The page owns the reads
//! and Coroot's controls; this source turns Coroot's messages into lines
//! and says what the list shows when it has none.

use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, Local, SecondsFormat, TimeZone, Utc};
use freshkube_core::coroot as api;
use freshkube_core::logs::{LogEvent, ServiceId};
use gpui_kit::{AnyElement, Context, SharedString, Window};

use freshkube_logs::{Columns, LogSource, LogView};

/// The Application page's Logs report.
pub type CorootLogView = LogView<CorootLogs>;

/// The most a line may hold; the view leaves out longer ones entirely.
const MOST_LINE_BYTES: usize = 64 * 1024;

pub struct CorootLogs {
    /// The application the lines belong to.
    service: ServiceId,
    /// Always empty: the page shows a failed read above the histogram, since
    /// it fails the patterns as much as the messages.
    errors: BTreeMap<ServiceId, String>,
    empty: SharedString,
}

impl LogSource for CorootLogs {
    fn controls(_: &CorootLogView, _: &mut Context<CorootLogView>) -> Vec<AnyElement> {
        Vec::new()
    }

    fn empty_message(view: &CorootLogView) -> SharedString {
        view.source().empty.clone()
    }

    fn errors(&self) -> &BTreeMap<ServiceId, String> {
        &self.errors
    }

    fn follow_tooltip(_: &CorootLogView) -> SharedString {
        "Pause to review. Refresh reads newer messages.".into()
    }

    fn panel_label(_: &CorootLogView) -> SharedString {
        "Coroot logs panel".into()
    }
}

/// What the Application page asks of its Logs report.
pub(crate) trait CorootPanel: Sized + 'static {
    fn for_coroot(window: &mut Window, cx: &mut Context<Self>) -> Self;

    /// Starts over on another application or query, with no lines.
    fn start(&mut self, app: &api::AppId, empty: SharedString, cx: &mut Context<Self>);

    /// Adds Coroot's messages, oldest first, after a note on what may be
    /// missing before them.
    fn add(
        &mut self,
        gap: Option<(DateTime<Utc>, String)>,
        lines: &[api::LogLine],
        cx: &mut Context<Self>,
    );

    /// What the list says while it has no line.
    fn set_empty(&mut self, empty: SharedString, cx: &mut Context<Self>);
}

impl CorootPanel for CorootLogView {
    fn for_coroot(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let source = CorootLogs {
            service: ServiceId::from(""),
            errors: BTreeMap::new(),
            empty: SharedString::default(),
        };
        Self::with_source(source, window, cx).with_columns(Columns {
            time: true,
            source: false,
        })
    }

    fn start(&mut self, app: &api::AppId, empty: SharedString, cx: &mut Context<Self>) {
        let service = ServiceId::from(app.as_str());
        self.reset_lines(app.as_str());
        self.set_shown([service.clone()].into());
        let source = self.source_mut();
        source.service = service;
        source.empty = empty;
        cx.notify();
    }

    fn add(
        &mut self,
        gap: Option<(DateTime<Utc>, String)>,
        lines: &[api::LogLine],
        cx: &mut Context<Self>,
    ) {
        let service = self.source().service.clone();
        let events = events(&service, gap, lines, &Local);
        if !events.is_empty() {
            self.ingest(events, cx);
        }
    }

    fn set_empty(&mut self, empty: SharedString, cx: &mut Context<Self>) {
        self.source_mut().empty = empty;
        cx.notify();
    }
}

/// The note on what may be missing, then Coroot's messages, all with their
/// times in `zone`: the app's local time.
fn events<Tz: TimeZone>(
    service: &ServiceId,
    gap: Option<(DateTime<Utc>, String)>,
    lines: &[api::LogLine],
    zone: &Tz,
) -> Vec<LogEvent> {
    gap.map(|(at, text)| LogEvent::marker(service.clone(), in_zone(at, zone), text))
        .into_iter()
        .chain(lines.iter().map(|line| event(service, line, zone)))
        .collect()
}

/// A time in `zone`, as both Coroot's rows and its gap note show it, so the
/// two read on one clock.
fn in_zone<Tz: TimeZone>(time: DateTime<Utc>, zone: &Tz) -> DateTime<FixedOffset> {
    time.with_timezone(zone).fixed_offset()
}

/// One message as one line: Coroot's time in `zone`, local time as the
/// page's charts show it, then the message whole, however many lines it
/// spans, at Coroot's severity rather than one its words suggest. A message
/// too long for the view is cut, and says so.
fn event<Tz: TimeZone>(service: &ServiceId, line: &api::LogLine, zone: &Tz) -> LogEvent {
    let time = DateTime::from_timestamp_millis(line.time_ms).unwrap_or_default();
    let time = in_zone(time, zone).to_rfc3339_opts(SecondsFormat::Millis, false);
    let mut text = format!("{time} {}", line.message);
    if text.len() > MOST_LINE_BYTES {
        let cut = format!(" … [cut: {} bytes more]", text.len() - MOST_LINE_BYTES);
        let mut end = MOST_LINE_BYTES - cut.len();
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(&cut);
    }
    LogEvent::new(service.clone(), text).with_level(line.level.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use freshkube_core::types::LogLevel;

    fn line(message: String) -> api::LogLine {
        api::LogLine {
            time_ms: 1_789_999_000_250,
            severity: "error".into(),
            level: LogLevel::Error,
            message,
            attributes: Default::default(),
            trace_id: String::new(),
        }
    }

    #[test]
    fn a_message_keeps_coroot_s_time_and_severity_and_fits_the_view() {
        let service = ServiceId::from("app");
        let short = event(&service, &line("ready, serving\n  on :8080".into()), &Local);
        let (time, message) = short.line.split_once(' ').unwrap();
        assert_eq!(message, "ready, serving\n  on :8080");
        let time = DateTime::parse_from_rfc3339(time).unwrap();
        assert_eq!(time.timestamp_millis(), 1_789_999_000_250);
        let local = DateTime::from_timestamp_millis(1_789_999_000_250)
            .unwrap()
            .with_timezone(&Local);
        assert_eq!(
            time.offset().local_minus_utc(),
            local.offset().local_minus_utc()
        );
        assert_eq!(short.level, Some(LogLevel::Error));
        // A name before a colon stays in the message the row shows.
        let mut logs = freshkube_core::logs::MultiServiceLogs::new("app");
        logs.append(event(
            &service,
            &line("cart-db: connection refused".into()),
            &Local,
        ));
        assert_eq!(
            logs.buffer().entries()[0].message,
            "cart-db: connection refused"
        );
        // At the bound the message is cut on a character, and says so.
        let long = event(&service, &line("é".repeat(40_000)), &Local);
        assert!(long.line.len() <= MOST_LINE_BYTES, "{}", long.line.len());
        assert!(
            long.line.ends_with("bytes more]"),
            "{}",
            &long.line[long.line.len() - 40..]
        );
    }

    /// A gap's note reads on the rows' clock, the viewer's.
    #[test]
    fn a_gap_reads_in_the_same_zone_as_the_messages() {
        let service = ServiceId::from("app");
        let east = FixedOffset::east_opt(2 * 3600).unwrap();
        let _zone = freshkube_core::logs::pin_zone(east);
        // 05:55:00 UTC, 07:55 at +02:00.
        let at = DateTime::from_timestamp_millis(1_790_834_100_000).unwrap();
        let mut message = line("ready".into());
        message.time_ms = at.timestamp_millis() + 30_000;
        let gap = Some((at, "Some messages may be missing".into()));
        let mut logs = freshkube_core::logs::MultiServiceLogs::new("app");
        logs.append_batch(events(&service, gap, &[message], &east));
        let times: Vec<_> = logs
            .buffer()
            .entries()
            .iter()
            .map(|entry| entry.timestamp.as_ref().unwrap().display.clone())
            .collect();
        assert_eq!(times, ["07:55:00", "07:55:30"]);
    }
}
