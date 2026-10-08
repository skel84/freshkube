//! Coroot's logs answer for an example application, through the same
//! decoding limits and cursor as a live one. The messages are dated once,
//! from the first read's window, and never again: a later read finds only
//! the lines written since, one every 30 seconds of the executor's clock.
//! Every name and figure is invented.
use super::super::example;
use freshkube_core::coroot as api;
use freshkube_core::types::LogLevel;

/// How often the example writes a line after its first read.
const TICK_MS: i64 = 30_000;
/// The histogram's columns.
const COLUMNS: i64 = 60;

/// The example's messages: minutes before the first read, severity, text.
const MESSAGES: [(i64, &str, &str); 12] = [
    (52, "info", "worker started, version 1.8.2"),
    (51, "info", "connected to queue payments.settle"),
    (47, "debug", "polling queue, 0 messages waiting"),
    (40, "info", "settled 14 payments in 212 ms"),
    (
        33,
        "warning",
        "ledger responded slowly: 1.9 s for POST /entries",
    ),
    (28, "info", "settled 9 payments in 187 ms"),
    (
        21,
        "warning",
        "ledger responded slowly: 2.4 s for POST /entries",
    ),
    (17, "unknown", "GC forced after 2m0s"),
    (
        14,
        "error",
        "cannot reach ledger-db:6432: connection refused",
    ),
    (
        12,
        "error",
        "retrying batch 4127 after error, attempt 3 of 5",
    ),
    (9, "fatal", ""),
    (3, "info", "worker restarted, version 1.8.2"),
];

/// The application whose logs Coroot has; ledger-db's Coroot has no logs
/// store, and the others found no messages.
pub(super) fn logs(
    app: &api::AppId,
    query: &api::LogQuery,
    range: api::TimeRange,
    anchor_ms: i64,
) -> api::LogsView {
    let (from_ms, to_ms) = (
        range.from.map_or(0, |t| t.timestamp_millis()),
        range.to.map_or(anchor_ms, |t| t.timestamp_millis()),
    );
    if *app == example::id("payments/ledger-db") {
        return not_configured(from_ms, to_ms);
    }
    if *app != example::id(example::WORKER) {
        return api::LogsView {
            status: api::Status::Unknown,
            message: "No logs found in ClickHouse".into(),
            ..Default::default()
        };
    }
    let all: Vec<api::LogLine> = written(anchor_ms, to_ms)
        .into_iter()
        .filter(|line| (from_ms..=to_ms).contains(&line.time_ms))
        .collect();
    let chart = histogram(&all, from_ms, to_ms);
    let origins = vec![api::LogOrigin::Containers, api::LogOrigin::OpenTelemetry];
    let origin = query.origin.unwrap_or(api::LogOrigin::OpenTelemetry);
    if query.mode == api::LogsMode::Patterns {
        return api::LogsView {
            status: api::Status::Ok,
            message: "Using OpenTelemetry logs of worker".into(),
            origins,
            origin: Some(origin),
            mode: api::LogsMode::Patterns,
            chart: Some(chart),
            patterns: patterns(&all, from_ms, to_ms),
            ..Default::default()
        };
    }
    // Coroot reads after `since`, in nanoseconds; each example message
    // sits half a millisecond into its millisecond.
    let mut lines: Vec<api::LogLine> = all
        .into_iter()
        .filter(|line| query.since.is_none_or(|since| nanos(line) > since))
        .take(query.limit as usize)
        .collect();
    lines.sort_by_key(|line| line.time_ms);
    api::LogsView {
        status: api::Status::Ok,
        message: format!(
            "Using {} logs of worker",
            if origin == api::LogOrigin::Containers {
                "container"
            } else {
                "OpenTelemetry"
            }
        ),
        origins,
        origin: Some(origin),
        mode: api::LogsMode::Messages,
        chart: Some(chart),
        capped: lines.len() >= query.limit as usize,
        max_ts: lines.iter().map(nanos).max(),
        lines,
        patterns: vec![],
    }
}

fn nanos(line: &api::LogLine) -> i64 {
    line.time_ms * 1_000_000 + 500_000
}

/// Every message written by `to_ms`, oldest first.
fn written(anchor_ms: i64, to_ms: i64) -> Vec<api::LogLine> {
    let mut lines: Vec<api::LogLine> = MESSAGES
        .iter()
        .map(|&(minutes, severity, message)| {
            let message = if severity == "fatal" {
                trace()
            } else {
                message.to_owned()
            };
            line(anchor_ms - minutes * 60_000, severity, message)
        })
        .collect();
    let ticks = (to_ms - anchor_ms).max(0) / TICK_MS;
    lines.extend((1..=ticks).map(|tick| {
        line(
            anchor_ms + tick * TICK_MS,
            "info",
            format!(
                "settled {} payments in {} ms",
                3 + tick % 11,
                150 + tick % 70
            ),
        )
    }));
    lines
}

fn line(time_ms: i64, severity: &str, message: String) -> api::LogLine {
    api::LogLine {
        time_ms,
        severity: severity.into(),
        level: api::severity_level(severity),
        message,
        attributes: [("service.name".into(), "worker".into())].into(),
        trace_id: String::new(),
    }
}

/// The example panic's size: just under the view's 64 KiB a line, with
/// room for the time before it.
pub(super) const TRACE_BYTES: usize = 63 * 1024;

/// A Go panic of 200 lines and [`TRACE_BYTES`], each frame's arguments
/// wider than any window, whose last line alone names `drainSettlementQueue`.
pub(super) fn trace() -> String {
    const FRAMES: usize = 98;
    let head = "panic: ledger-db: connection refused after 5 attempts\n\ngoroutine 1 [running]:";
    let tail = "\npayments/worker/cmd/worker.drainSettlementQueue(...)";
    let budget = (TRACE_BYTES - head.len() - tail.len()) / FRAMES;
    let mut text = String::from(head);
    for frame in 0..FRAMES {
        let mut call =
            format!("\npayments/worker/internal/settle.(*Batch).step{frame:02}(0xc000{frame:06x}");
        let file = format!(
            ")\n\t/src/payments/worker/internal/settle/batch.go:{} +0x{:x}",
            100 + frame,
            0x1f4 + frame
        );
        while call.len() + file.len() + 5 <= budget {
            call.push_str(", 0x0");
        }
        text.push_str(&call);
        text.push_str(&file);
    }
    text.push_str(tail);
    text
}

/// Messages per column, by severity, as Coroot's histogram counts them.
fn histogram(lines: &[api::LogLine], from_ms: i64, to_ms: i64) -> api::AppChart {
    let step_ms = ((to_ms - from_ms) / COLUMNS).max(1);
    let series = ["error", "warning", "info", "debug", "unknown"]
        .into_iter()
        .filter_map(|severity| {
            let mut points = vec![None; COLUMNS as usize];
            let mut any = false;
            for l in lines.iter().filter(|l| fold(&l.severity) == severity) {
                let ix = (((l.time_ms - from_ms) / step_ms) as usize).min(COLUMNS as usize - 1);
                *points[ix].get_or_insert(0.) += 1.;
                any = true;
            }
            any.then(|| api::Series {
                name: severity.into(),
                color: color(severity).into(),
                points,
                ..Default::default()
            })
        })
        .collect();
    api::AppChart {
        from_ms,
        to_ms,
        step_ms,
        series,
        column: true,
        stacked: true,
        ..Default::default()
    }
}

/// Coroot's colour for a severity (`model/severity.go`).
fn color(severity: &str) -> &'static str {
    match severity {
        "error" => "red-darken1",
        "warning" => "orange-lighten1",
        "info" => "blue-lighten2",
        "debug" => "green-lighten2",
        _ => "grey-lighten1",
    }
}

/// The severity a histogram column counts a message under.
fn fold(severity: &str) -> &'static str {
    match api::severity_level(severity) {
        LogLevel::Error => "error",
        LogLevel::Warning => "warning",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
        LogLevel::Unknown => "unknown",
    }
}

fn patterns(lines: &[api::LogLine], from_ms: i64, to_ms: i64) -> Vec<api::LogPattern> {
    [
        ("info", "settled <*> payments in <*> ms"),
        (
            "warning",
            "ledger responded slowly: <*> s for POST /entries",
        ),
        ("error", "cannot reach ledger-db:<*>: connection refused"),
        (
            "error",
            "retrying batch <*> after error, attempt <*> of <*>",
        ),
    ]
    .into_iter()
    .map(|(severity, sample)| {
        let head = sample.split('<').next().unwrap_or(sample);
        let matching: Vec<api::LogLine> = lines
            .iter()
            .filter(|l| l.message.starts_with(head))
            .cloned()
            .collect();
        let mut chart = histogram(&matching, from_ms, to_ms);
        chart.title = sample.into();
        api::LogPattern {
            severity: severity.into(),
            level: api::severity_level(severity),
            sample: sample.into(),
            count: matching.len() as u64,
            chart: Some(chart),
        }
    })
    .collect()
}

/// What Coroot answers without a logs store: its reason, and the patterns
/// its agent counted.
fn not_configured(from_ms: i64, to_ms: i64) -> api::LogsView {
    let wave = |phase: usize| api::Series {
        name: "error".into(),
        color: color("error").into(),
        points: (0..COLUMNS as usize)
            .map(|p| Some(((p + phase) % 5) as f32))
            .collect(),
        ..Default::default()
    };
    let chart = |title: &str, phase: usize| api::AppChart {
        title: title.into(),
        from_ms,
        to_ms,
        step_ms: ((to_ms - from_ms) / COLUMNS).max(1),
        series: vec![wave(phase)],
        column: true,
        ..Default::default()
    };
    api::LogsView {
        status: api::Status::Unknown,
        message: "Clickhouse integration is not configured".into(),
        mode: api::LogsMode::Patterns,
        patterns: vec![
            api::LogPattern {
                severity: "error".into(),
                level: LogLevel::Error,
                sample: "could not receive data from client: Connection reset by peer".into(),
                count: 118,
                chart: Some(chart("could not receive data", 0)),
            },
            api::LogPattern {
                severity: "warning".into(),
                level: LogLevel::Warning,
                sample: "checkpoints are occurring too frequently (<*> seconds apart)".into(),
                count: 41,
                chart: Some(chart("checkpoints", 2)),
            },
        ],
        ..Default::default()
    }
}
