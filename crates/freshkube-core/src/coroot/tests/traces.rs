use super::*;

fn tracing_view() -> serde_json::Value {
    serde_json::json!({"context":{},"data":{
        "status":"ok","message":"Using traces of <i>api</i>",
        "sources":[{"type":"otel","name":"OpenTelemetry","selected":true},{"type":"agent","name":"OpenTelemetry (eBPF)","selected":false}],
        "services":[{"name":"api","linked":true}],
        "heatmap":{"ctx":{"from":1789996400000_i64,"to":1790000000000_i64,"step":60000},"title":"Latency & Errors heatmap",
            "series":[
                {"name":"5ms","title":"0-5 ms","value":"0.005","data":[1.5,null,2]},
                {"name":">10s","title":">10 s","value":"inf","data":[null,null,0.1]},
                {"name":"errors","title":"errors","value":"err","data":[0,0.2,null]}],
            "annotations":null},
        "spans":[{"service":"api","trace_id":"4f2a9c1e","id":"s1","parent_id":"","name":"GET /cart",
            "timestamp":1789999000000_i64,"duration":12.5,"client":"web",
            "status":{"error":true,"message":"HTTP 503"},"details":{"text":"http://cart/","lang":""},
            "attributes":{"http.route":"/cart"},"events":null}],
        "limit":100}})
}

#[tokio::test]
async fn tracing_reads_the_heatmap_spans_and_coroot_s_note_as_plain_text() {
    let server = server(vec![(200, tracing_view())]).await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let app = AppId::new("c:prod:Deployment:api");
    let tracing = provider
        .tracing(
            &source(&provider),
            range(),
            &app,
            "",
            &TraceSelection::Recent,
        )
        .await
        .unwrap();
    assert_eq!(tracing.status, Status::Ok);
    assert_eq!(tracing.message, "Using traces of api");
    assert_eq!(tracing.sources.len(), 2);
    assert!(tracing.sources[0].selected);
    let heatmap = tracing.heatmap.unwrap();
    assert_eq!(heatmap.step_ms, 60_000);
    assert_eq!(heatmap.rows.len(), 3);
    assert_eq!(heatmap.rows[1].points, vec![None, None, Some(0.1)]);
    assert!(heatmap.rows[2].is_errors());
    assert_eq!(tracing.spans[0].trace_id, "4f2a9c1e");
    assert!(tracing.spans[0].status.error);
    assert!(tracing.limited);
}

#[tokio::test]
async fn tracing_without_a_project_world_or_with_a_bad_answer_fails_plainly() {
    let server = server(vec![
        (200, serde_json::json!({"context":{},"data":null})),
        (
            200,
            serde_json::json!({"context":{},"data":{"heatmap":{"ctx":{"from":2,"to":1,"step":0}}}}),
        ),
    ])
    .await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let app = AppId::new("c:prod:Deployment:api");
    let source = source(&provider);
    let read = || provider.tracing(&source, range(), &app, "", &TraceSelection::Recent);
    assert_eq!(read().await.unwrap_err(), ReadError::Missing);
    assert_eq!(read().await.unwrap_err(), ReadError::InvalidResponse);
}

#[tokio::test]
async fn trace_selections_are_checked_before_anything_is_sent() {
    let provider = Provider::new("http://127.0.0.1:9", Credentials::None).unwrap();
    let app = AppId::new("c:prod:Deployment:api");
    for (trace_source, selection) in [
        ("otel:x", TraceSelection::Recent),
        ("", TraceSelection::Trace("ab:cd".into())),
        (
            "",
            TraceSelection::Errors {
                from_ms: 5,
                to_ms: 5,
            },
        ),
        (
            "",
            TraceSelection::Latency {
                from_ms: 1,
                to_ms: 2,
                above: "0.1:x".into(),
                up_to: "inf".into(),
            },
        ),
    ] {
        assert_eq!(
            provider
                .tracing(&source(&provider), range(), &app, trace_source, &selection)
                .await
                .unwrap_err(),
            ReadError::InvalidSelection,
            "{selection:?}"
        );
    }
}

#[test]
fn coroot_markup_becomes_plain_text_without_eating_comparisons() {
    let plain = super::tracing::plain;
    assert_eq!(
        plain("Requests to the <var>shop-redis</var> app, per second"),
        "Requests to the shop-redis app, per second"
    );
    assert_eq!(
        plain("latency < 500ms and > 1s"),
        "latency < 500ms and > 1s"
    );
    assert_eq!(plain("a &lt;b&gt; &amp; c&nbsp;d"), "a <b> & c d");
    assert_eq!(plain("AT&T <b>bold"), "AT&T bold");
    assert_eq!(plain("<i>x</i> <<i>y</i>"), "x <y");
}
