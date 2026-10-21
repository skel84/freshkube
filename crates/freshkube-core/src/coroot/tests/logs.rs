use super::*;

fn logs_view() -> serde_json::Value {
    let chart = |name: &str| {
        serde_json::json!({"ctx":{"from":1789996400000_i64,"to":1790000000000_i64,"step":60000},
            "title":"","column":true,
            "series":[{"name":name,"color":"red-darken1","data":[1,null,3]}]})
    };
    serde_json::json!({"context":{},"data":{
        "status":"ok","message":"Using OpenTelemetry logs of <i>shop-api</i>",
        "sources":["agent","otel","something-new"],"source":"otel",
        "services":["shop-api"],"service":"shop-api","view":"messages",
        "chart":chart("error"),
        "entries":[
            {"timestamp":1789999000250_i64,"severity":"info","color":"blue-lighten2",
             "message":"retrying after error\n  at queue.rs:12","attributes":{"host.name":"app-a"},"trace_id":""},
            {"timestamp":1789999000100_i64,"severity":"fatal","color":"black",
             "message":"cannot reach the store","attributes":null,"trace_id":"4f2a9c1e"},
            {"timestamp":1789999000300_i64,"severity":"severity-12","message":"odd"}],
        "patterns":null,"limit":3,"max_ts":"1789999000300000123"}})
}

#[tokio::test]
async fn logs_are_read_by_get_with_coroot_s_query_and_kept_oldest_first() {
    let server = server(vec![(200, logs_view())]).await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let app = AppId::new("c:shop:Deployment:shop-api");
    let query = LogQuery {
        origin: Some(LogOrigin::OpenTelemetry),
        limit: 20,
        since: Some(1_789_999_000_000_000_000),
        ..LogQuery::default()
    };
    let logs = provider
        .logs(&source(&provider), range(), &app, &query)
        .await
        .unwrap();
    let requests = server.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    let (method, rest) = requests[0].split_once(' ').unwrap();
    // Only ever read: a POST here would save the application's settings.
    assert_eq!(method, "GET");
    let target = url::Url::parse(&format!("http://x{}", rest.split(' ').next().unwrap())).unwrap();
    assert!(
        target.path() == "/api/project/project-id/app/c%3Ashop%3ADeployment%3Ashop-api/logs",
        "{target}"
    );
    let sent: serde_json::Value = target
        .query_pairs()
        .find(|(k, _)| k == "query")
        .map(|(_, v)| serde_json::from_str(&v).unwrap())
        .unwrap();
    assert_eq!(
        sent,
        serde_json::json!({"source":"otel","view":"messages","filters":[],"limit":20,
            "since":"1789999000000000000"})
    );

    assert_eq!(logs.status, Status::Ok);
    assert_eq!(logs.message, "Using OpenTelemetry logs of shop-api");
    assert_eq!(
        logs.origins,
        [LogOrigin::Containers, LogOrigin::OpenTelemetry]
    );
    assert_eq!(logs.origin, Some(LogOrigin::OpenTelemetry));
    assert_eq!(logs.mode, LogsMode::Messages);
    assert_eq!(logs.chart.as_ref().unwrap().series[0].name, "error");
    let times: Vec<_> = logs.lines.iter().map(|l| l.time_ms).collect();
    assert_eq!(times, [1789999000100, 1789999000250, 1789999000300]);
    // Coroot's severity decides the level, never the words in the line.
    let levels: Vec<_> = logs.lines.iter().map(|l| l.level.clone()).collect();
    assert_eq!(
        levels,
        [
            crate::types::LogLevel::Error,
            crate::types::LogLevel::Info,
            crate::types::LogLevel::Unknown
        ]
    );
    assert_eq!(
        logs.lines[1].message,
        "retrying after error\n  at queue.rs:12"
    );
    assert_eq!(logs.lines[1].attributes["host.name"], "app-a");
    assert_eq!(logs.lines[0].trace_id, "4f2a9c1e");
    assert!(logs.capped);
    assert_eq!(logs.max_ts, Some(1_789_999_000_300_000_123));
}

#[test]
fn severities_map_to_levels_and_unknown_ones_stay_unknown() {
    use crate::types::LogLevel;
    for (severity, level) in [
        ("trace", LogLevel::Debug),
        ("debug", LogLevel::Debug),
        ("info", LogLevel::Info),
        ("warning", LogLevel::Warning),
        ("error", LogLevel::Error),
        ("fatal", LogLevel::Error),
        ("unknown", LogLevel::Unknown),
        ("severity-12", LogLevel::Unknown),
        ("", LogLevel::Unknown),
    ] {
        assert_eq!(severity_level(severity), level, "{severity}");
    }
}

#[tokio::test]
async fn logs_without_a_store_answer_with_patterns_and_coroot_s_note() {
    let view = serde_json::json!({"context":{},"data":{
        "status":"unknown","message":"Clickhouse integration is not configured",
        "view":"patterns","sources":null,"entries":null,
        "patterns":[{"severity":"error","color":"red-darken1","sample":"cannot reach <store>",
            "sum":41,"hash":"a1","chart":{"ctx":{"from":1789996400000_i64,"to":1790000000000_i64,"step":60000},
            "series":[{"name":"error","data":[1,2]}]}}],
        "limit":0,"max_ts":""}});
    let server = server(vec![(200, view)]).await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let app = AppId::new("c:shop:Deployment:shop-api");
    let logs = provider
        .logs(&source(&provider), range(), &app, &LogQuery::default())
        .await
        .unwrap();
    assert_eq!(logs.status, Status::Unknown);
    assert_eq!(logs.message, "Clickhouse integration is not configured");
    assert_eq!(logs.mode, LogsMode::Patterns);
    assert!(logs.origins.is_empty() && logs.origin.is_none() && logs.lines.is_empty());
    assert_eq!(logs.patterns.len(), 1);
    // A sample is the application's text, never markup to strip.
    assert_eq!(logs.patterns[0].sample, "cannot reach <store>");
    assert_eq!(logs.patterns[0].count, 41);
    assert_eq!(logs.patterns[0].level, crate::types::LogLevel::Error);
    assert!(!logs.capped);
    assert_eq!(logs.max_ts, None);
}

#[tokio::test]
async fn bad_log_reads_fail_plainly_and_bad_queries_send_nothing() {
    let server = server(vec![
        (200, serde_json::json!({"context":{},"data":null})),
        (
            200,
            serde_json::json!({"context":{},"data":{"max_ts":"soon"}}),
        ),
        (
            200,
            serde_json::json!({"context":{},"data":{"entries":[{"severity":"info"}]}}),
        ),
        (
            200,
            serde_json::json!({"context":{},"data":{"chart":{"ctx":{"from":2,"to":1,"step":0}}}}),
        ),
    ])
    .await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let app = AppId::new("c:shop:Deployment:shop-api");
    let source = source(&provider);
    let read = |query: LogQuery| {
        let (provider, source, app) = (&provider, &source, &app);
        async move { provider.logs(source, range(), app, &query).await }
    };
    assert_eq!(
        read(LogQuery::default()).await.unwrap_err(),
        ReadError::Missing
    );
    for _ in 0..3 {
        assert_eq!(
            read(LogQuery::default()).await.unwrap_err(),
            ReadError::InvalidResponse
        );
    }
    let asked = server.requests.lock().unwrap().len();
    for query in [
        LogQuery {
            limit: 7,
            ..LogQuery::default()
        },
        LogQuery {
            since: Some(0),
            ..LogQuery::default()
        },
    ] {
        assert_eq!(
            read(query.clone()).await.unwrap_err(),
            ReadError::InvalidSelection,
            "{query:?}"
        );
    }
    assert_eq!(server.requests.lock().unwrap().len(), asked);
}

#[test]
fn log_bounds_reject_runaway_answers() {
    let line = LogLine {
        time_ms: 1,
        severity: "info".into(),
        level: crate::types::LogLevel::Info,
        message: "ok".into(),
        attributes: Default::default(),
        trace_id: String::new(),
    };
    let view = |lines: Vec<LogLine>| LogsView {
        lines,
        ..LogsView::default()
    };
    assert!(limits::logs(&view(vec![line.clone(); 1_000])).is_ok());
    assert_eq!(
        limits::logs(&view(vec![line.clone(); 1_001])),
        Err(ReadError::Limit)
    );
    let long = LogLine {
        message: "x".repeat(65_537),
        ..line.clone()
    };
    assert_eq!(limits::logs(&view(vec![long])), Err(ReadError::Limit));
    // Each message within its bound, but together too much.
    let large = LogLine {
        message: "x".repeat(65_536),
        ..line.clone()
    };
    assert_eq!(limits::logs(&view(vec![large; 129])), Err(ReadError::Limit));
    let tagged = LogLine {
        attributes: (0..129).map(|i| (i.to_string(), String::new())).collect(),
        ..line
    };
    assert_eq!(limits::logs(&view(vec![tagged])), Err(ReadError::Limit));
}

fn log_line(time_ms: i64, message: &str) -> LogLine {
    LogLine {
        time_ms,
        severity: "info".into(),
        level: crate::types::LogLevel::Info,
        message: message.into(),
        attributes: Default::default(),
        trace_id: String::new(),
    }
}

fn answer(lines: &[LogLine], max_ts: i64, capped: bool) -> LogsView {
    LogsView {
        lines: lines.to_vec(),
        max_ts: Some(max_ts),
        capped,
        ..LogsView::default()
    }
}

#[test]
fn a_refresh_asks_from_the_newest_nanosecond_and_adds_no_message_twice() {
    let mut cursor = LogCursor::default();
    assert_eq!(cursor.since(), None);
    let a = log_line(1_000, "a");
    let b = log_line(2_000, "b");
    // Two equal messages in one answer are two messages.
    let first = cursor.take(&mut answer(
        &[a.clone(), b.clone(), b.clone()],
        2_000_000_500,
        true,
    ));
    assert_eq!(first.lines.len(), 3);
    // A first read that came back capped has no earlier read to leave a gap from.
    assert!(!first.gap);
    // Coroot's since is exclusive: one nanosecond earlier takes in the newest.
    assert_eq!(cursor.since(), Some(2_000_000_499));

    // The newest millisecond comes back with a message not seen before.
    let c = log_line(2_000, "c");
    let d = log_line(3_000, "d");
    let second = cursor.take(&mut answer(
        &[b.clone(), c.clone(), d.clone()],
        3_000_000_000,
        false,
    ));
    assert_eq!(second.lines, [c, d.clone()]);
    assert!(!second.gap);
    assert_eq!(cursor.since(), Some(2_999_999_999));

    // Nothing newer: nothing added, and the cursor stays.
    let mut empty = LogsView::default();
    assert!(cursor.take(&mut empty).lines.is_empty());
    assert_eq!(cursor.since(), Some(2_999_999_999));

    // The boundary message again, and a full answer: there may be a gap.
    let e = log_line(4_000, "e");
    let third = cursor.take(&mut answer(&[d, e.clone()], 4_000_000_000, true));
    assert_eq!(third.lines, [e]);
    assert!(third.gap);
}
