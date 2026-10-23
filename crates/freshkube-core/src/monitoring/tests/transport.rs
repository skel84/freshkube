use super::*;

#[tokio::test(start_paused = true)]
async fn a_silent_prometheus_times_out() {
    let (client, _) = fake(|_, _, _| (0, String::new()));
    let prometheus = Prometheus::new(client, PrometheusService::new("m", "p", 9090))
        .with_timeout(Duration::from_secs(2));
    let error = prometheus.read_version().await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::TimedOut);
    assert_eq!(error.message, "Prometheus didn't answer within 2 s");
}

#[tokio::test]
async fn an_oversized_answer_is_refused_whole() {
    let (client, _) = fake(|_, _, _| (200, success(json!({"version": "x".repeat(4096)}))));
    let prometheus =
        Prometheus::new(client, PrometheusService::new("m", "p", 9090)).with_limit(1024);
    let error = prometheus.read_version().await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::BadAnswer);
    assert!(
        error.message.contains("larger than 1 MiB"),
        "{}",
        error.message
    );
}

#[tokio::test]
async fn prometheus_errors_and_bad_answers_are_told_apart() {
    let (client, _) = fake(|_, path, _| match path {
        p if p.ends_with("/query") => (
            400,
            json!({"status": "error", "errorType": "bad_data", "error": "parse error at char 4"})
                .to_string(),
        ),
        p if p.ends_with("/labels") => (200, "<html>login</html>".into()),
        _ => (
            502,
            status(
                502,
                "error trying to reach service: dial tcp 10.0.0.1:9090: connect: connection refused",
            ),
        ),
    });
    let prometheus = Prometheus::new(client, PrometheusService::new("m", "p", 9090));
    let error = prometheus
        .get(&ApiRequest::new(
            "query",
            vec![("query".into(), "up{".into())],
        ))
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Rejected);
    assert!(error.message.contains("parse error"), "{}", error.message);
    let error = prometheus
        .get(&ApiRequest::new("labels", Vec::new()))
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::BadAnswer);
    let error = prometheus
        .get(&ApiRequest::new("targets", Vec::new()))
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    // No message names the proxy path.
    assert!(!error.message.contains("/proxy/"), "{}", error.message);
}

#[tokio::test]
async fn a_query_too_long_for_a_get_is_never_sent() {
    let (client, seen) = fake(|_, _, _| (200, success(json!([]))));
    let prometheus = Prometheus::new(client, PrometheusService::new("m", "p", 9090));
    let error = prometheus
        .get(&ApiRequest::new(
            "query",
            vec![("query".into(), "a".repeat(9000))],
        ))
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unsupported);
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn the_scrape_interval_is_the_most_common_one() {
    assert_eq!(
        scrape_interval_of(&json!({"activeTargets": [
            {"scrapeInterval": "30s"}, {"scrapeInterval": "1m"}, {"scrapeInterval": "1m"}, {}
        ]})),
        Some(60.)
    );
    assert_eq!(
        scrape_interval_of(&json!({"activeTargets": [
            {"scrapeInterval": "30s"}, {"scrapeInterval": "15s"}
        ]})),
        Some(15.)
    );
    assert_eq!(scrape_interval_of(&json!({"activeTargets": []})), None);
}
