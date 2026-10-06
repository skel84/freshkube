use super::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
struct Server {
    url: String,
    task: JoinHandle<()>,
    /// Each request's first line: method, path and query.
    requests: Arc<Mutex<Vec<String>>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(responses: Vec<(u16, serde_json::Value)>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
    let requests = Arc::new(Mutex::new(vec![]));
    let seen = requests.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let responses = responses.clone();
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut bytes = vec![0; 16384];
                let n = socket.read(&mut bytes).await.unwrap();
                assert!(n < bytes.len());
                let head = String::from_utf8_lossy(&bytes[..n]);
                seen.lock()
                    .unwrap()
                    .push(head.lines().next().unwrap_or_default().to_owned());
                let (status, body) = responses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or((404, serde_json::json!({})));
                let body = if body.get("applications").is_some() || body.get("map").is_some() {
                    serde_json::json!({"context":{},"data":body})
                } else {
                    body
                };
                let body = body.to_string();
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    Server {
        url,
        task,
        requests,
    }
}
fn range() -> TimeRange {
    let to = chrono::DateTime::from_timestamp(1_790_000_000, 0).unwrap();
    TimeRange::between(to - chrono::Duration::hours(1), to)
}
fn source(provider: &Provider) -> Source {
    provider.source(&ProjectInfo {
        id: "project-id".into(),
        name: "Display name".into(),
    })
}

#[tokio::test]
async fn signals_empty_results_refusal_failure_and_recovery_remain_distinct() {
    let raw = serde_json::json!({"applications":[{"id":"c:prod:Deployment:api","status":"ok","category":"application","cpu":{"status":"ok","value":""},"memory":{"status":"unknown","value":""}}]});
    let server = server(vec![
        (200, raw.clone()),
        (200, serde_json::json!({"applications":[]})),
        (403, serde_json::json!({"error":"secret must not escape"})),
        (500, serde_json::json!({"error":"secret must not escape"})),
        (200, raw),
    ])
    .await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let source = source(&provider);
    let apps = provider.applications(&source, range()).await.unwrap();
    assert_eq!(apps[0].status, Status::Ok);
    assert_eq!(apps[0].signals["cpu"].status, Status::Ok);
    assert_eq!(apps[0].signals["memory"].status, Status::Unknown);
    assert!(!apps[0].signals.contains_key("logs"));
    assert!(
        provider
            .applications(&source, range())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        provider.applications(&source, range()).await.unwrap_err(),
        ReadError::Refused
    );
    let failure = provider.applications(&source, range()).await.unwrap_err();
    assert_eq!(failure, ReadError::Failed);
    assert!(!failure.to_string().contains("secret"));
    assert_eq!(
        provider.applications(&source, range()).await.unwrap().len(),
        1
    );
}
#[tokio::test]
async fn project_selection_is_explicit() {
    let server = server(vec![(
        200,
        serde_json::json!({"projects":[{"id":"p","name":"Production"}]}),
    )])
    .await;
    let provider = Provider::new(&server.url, Credentials::ApiKey("fake-key".into())).unwrap();
    let projects = provider.projects().await.unwrap();
    let source = provider.source(&projects[0]);
    let other = Provider::new(&server.url, Credentials::None).unwrap();
    assert_ne!(provider.id(), other.id());
    assert_eq!(
        other.applications(&source, range()).await.unwrap_err(),
        ReadError::InvalidSelection
    );
}
#[test]
fn association_requires_exact_access_cluster_and_kubernetes_kind() {
    let provider = Provider::new("https://coroot.example.com", Credentials::None).unwrap();
    let source = source(&provider);
    let id = AppId::new("cluster-a:prod:Deployment:api");
    assert!(ObjectSubject::for_app(&source, &id).is_none());
    let source = source.with_association(Some(Association::new(
        "access:opaque-a".into(),
        "cluster-a".into(),
    )));
    let subject = ObjectSubject::for_app(&source, &id).unwrap();
    assert_eq!(subject.access(), "access:opaque-a");
    assert_eq!(subject.namespace(), "prod");
    assert_eq!(subject.name(), "api");
    assert_eq!(subject.kind().kind, "Deployment");
    for id in [
        "cluster-a:prod:Deployment:../api",
        "cluster-a:prod/other:Pod:api",
        "cluster-a:prod:Pod:",
    ] {
        assert!(ObjectSubject::for_app(&source, &AppId::new(id)).is_none());
    }
    assert!(
        ObjectSubject::for_app(&source, &AppId::new("cluster-b:prod:Deployment:api")).is_none()
    );
    assert!(
        ObjectSubject::for_app(
            &source,
            &AppId::new("external:external:ExternalService:api:443")
        )
        .is_none()
    );
}

#[test]
fn empty_credentials_do_not_silently_select_anonymous_access() {
    for credentials in [
        Credentials::ApiKey(String::new()),
        Credentials::Session(" ".into()),
    ] {
        assert!(matches!(
            Provider::new("https://coroot.example.com", credentials),
            Err(ReadError::Authentication)
        ));
    }
    assert!(Provider::new("https://coroot.example.com", Credentials::None).is_ok());
}
#[test]
fn url_policy_and_bounds_fail_without_leaking_inputs() {
    for url in [
        "coroot.example.com",
        "https://user:secret@example.com",
        "https://example.com/?secret=x",
        "file:///etc/passwd",
    ] {
        assert!(matches!(
            Provider::new(url, Credentials::None),
            Err(ReadError::InvalidUrl)
        ));
    }
    let app = Application {
        id: AppId::new("c:prod:Deployment:api"),
        cluster: String::new(),
        category: String::new(),
        app_type: String::new(),
        status: Status::Unknown,
        signals: Default::default(),
    };
    assert_eq!(
        limits::applications(&vec![app.clone(); 2001]),
        Err(ReadError::Limit)
    );
    assert_eq!(
        limits::applications(&[app.clone(), app]),
        Err(ReadError::InvalidResponse)
    );
}
#[tokio::test(start_paused = true)]
async fn read_deadline_includes_waiting_for_a_slot() {
    let provider = Provider::new("http://127.0.0.1:1", Credentials::None).unwrap();
    let started = tokio::time::Instant::now();
    let read = || {
        provider.read(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok::<(), coroot_rs::Error>(())
        })
    };
    // The third read waits for a slot: its deadline starts with the other
    // two, rather than granting another 20 seconds after acquiring one.
    let results = tokio::join!(read(), read(), read());
    assert_eq!(
        results,
        (
            Err(ReadError::Timeout),
            Err(ReadError::Timeout),
            Err(ReadError::Timeout)
        )
    );
    assert_eq!(started.elapsed(), Duration::from_secs(20));
}

#[tokio::test(start_paused = true)]
async fn cloned_providers_share_two_read_slots() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let provider = Provider::new("http://127.0.0.1:1", Credentials::None).unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let read = || {
        let (provider, active, peak) = (provider.clone(), active.clone(), peak.clone());
        async move {
            provider
                .read(async move {
                    let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(n, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                })
                .await
                .unwrap();
        }
    };
    let start = tokio::time::Instant::now();
    tokio::join!(read(), read(), read());
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    assert_eq!(start.elapsed(), Duration::from_secs(2));
}

#[tokio::test]
async fn maps_keep_unknown_links_and_empty_maps_are_successful() {
    let server=server(vec![(200,serde_json::json!({"map":[
        {"id":"c:ns:Deployment:a","status":"ok","upstreams":[{"id":"c:ns:Deployment:b","status":"unknown"}]},
        {"id":"c:ns:Deployment:b","status":"unknown"}
    ]})),(200,serde_json::json!({"map":[]}))]).await;
    let provider = Provider::new(&server.url, Credentials::Session("fake-session".into())).unwrap();
    let source = source(&provider);
    let map = provider.service_map(&source, range()).await.unwrap();
    assert_eq!(map.nodes.len(), 2);
    assert_eq!(map.edges[0].status, Status::Unknown);
    assert!(map.edges[0].rps.is_none());
    assert!(
        provider
            .service_map(&source, range())
            .await
            .unwrap()
            .nodes
            .is_empty()
    );
}

#[tokio::test]
async fn oversized_response_is_rejected_before_it_can_become_empty_data() {
    let server = server(vec![(
        200,
        serde_json::json!({"pad":"x".repeat(8 * 1024 * 1024)}),
    )])
    .await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    assert_eq!(
        provider
            .applications(&source(&provider), range())
            .await
            .unwrap_err(),
        ReadError::Limit
    );
}

#[tokio::test]
async fn malformed_success_is_a_decode_failure_not_an_empty_collection() {
    let server = server(vec![
        (200, serde_json::json!({})),
        (200, serde_json::json!(null)),
    ])
    .await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    assert_eq!(
        provider
            .applications(&source(&provider), range())
            .await
            .unwrap_err(),
        ReadError::InvalidResponse
    );
    assert_eq!(
        provider
            .service_map(&source(&provider), range())
            .await
            .unwrap_err(),
        ReadError::InvalidResponse
    );
}

fn incident_wire(key: &str) -> serde_json::Value {
    serde_json::json!({"key":key,"application_id":"c:prod:Deployment:api","cluster":"Production","severity":"critical","opened_at":1790000000000_i64,"resolved_at":null,"impact":0,"duration":120000,"short_description":"High latency"})
}
#[tokio::test]
async fn incident_identity_errors_limits_and_missing_evidence() {
    let mut wrong = incident_wire("k1");
    wrong["application_id"] = serde_json::json!("c:prod:Deployment:other");
    let mut oversized = incident_wire("k1");
    oversized["rca"] = serde_json::json!({"root_cause":"x".repeat(16385)});
    let envelope = |data| serde_json::json!({"context":{},"data":data});
    let server = server(vec![
        (200, envelope(serde_json::json!([incident_wire("k1")]))),
        (200, envelope(incident_wire("k1"))),
        (200, envelope(wrong)),
        (200, envelope(oversized)),
        (200, envelope(serde_json::json!({}))),
        (403, serde_json::json!({"error":"private server text"})),
        (200, envelope(serde_json::json!([]))),
    ])
    .await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let source = source(&provider);
    let app = AppId::new("c:prod:Deployment:api");
    let values = provider.incidents(&source, range()).await.unwrap();
    assert_eq!(values[0].key, "k1");
    assert_eq!(values[0].impact_percent, 0.);
    let view = provider
        .incident(&source, range(), "k1", &app)
        .await
        .unwrap();
    assert!(view.availability().is_none());
    assert!(view.incident().slo.is_none());
    assert_eq!(
        provider
            .incident(&source, range(), "k1", &app)
            .await
            .unwrap_err(),
        ReadError::InvalidResponse
    );
    assert_eq!(
        provider
            .incident(&source, range(), "k1", &app)
            .await
            .unwrap_err(),
        ReadError::Limit
    );
    assert_eq!(
        provider.incidents(&source, range()).await.unwrap_err(),
        ReadError::InvalidResponse
    );
    assert_eq!(
        provider.incidents(&source, range()).await.unwrap_err(),
        ReadError::Refused
    );
    assert!(
        provider
            .incidents(&source, range())
            .await
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn incidents_enforce_collection_bounds() {
    let values = (0..101)
        .map(|ix| incident_wire(&format!("k{ix}")))
        .collect::<Vec<_>>();
    let server = server(vec![(200, serde_json::json!({"context":{},"data":values}))]).await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    // A server violating the requested bound fails visibly. Oversized response
    // bodies are independently bounded before decoding.
    assert_eq!(
        provider
            .incidents(&source(&provider), range())
            .await
            .unwrap_err(),
        ReadError::InvalidResponse
    );
}

#[test]
fn a_cluster_sized_map_is_accepted_and_a_runaway_one_is_not() {
    let node = |ix: usize| MapNode {
        id: AppId::new(format!("c:ns:Deployment:app-{ix}")),
        cluster: String::new(),
        category: String::new(),
        status: Status::Ok,
        custom: false,
        labels: Default::default(),
        indicators: Default::default(),
        distance: None,
    };
    let nodes: Vec<_> = (0..400).map(node).collect();
    let edges: Vec<_> = (0..1_200)
        .map(|ix| MapEdge {
            from: nodes[ix % 400].id.clone(),
            to: nodes[(ix / 400 + ix + 1) % 400].id.clone(),
            status: Status::Ok,
            rps: None,
            latency_seconds: None,
            sent_bytes_per_second: None,
            received_bytes_per_second: None,
            issue: String::new(),
        })
        .collect();
    let map = ServiceMap { nodes, edges };
    assert_eq!(limits::map(&map), Ok(()));
    let runaway = ServiceMap {
        nodes: (0..2_001).map(node).collect(),
        edges: Vec::new(),
    };
    assert_eq!(limits::map(&runaway), Err(ReadError::Limit));
}

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

fn profile_view(flamegraph: serde_json::Value, diff: bool) -> serde_json::Value {
    serde_json::json!({"context":{},"data":{
        "status":"ok","message":"OK",
        "services":[{"name":"api","linked":true}],
        "profiles":[{"type":"go:profile_cpu:nanoseconds","name":"CPU"},{"type":"go:heap_inuse_space:bytes","name":"Memory (in-use bytes)"}],
        "profile":{"type":"go:profile_cpu:nanoseconds","diff":diff,"flamegraph":flamegraph},
        "chart":null,"instances":["api-0","api-1"]}})
}

#[tokio::test]
async fn a_profile_is_flattened_with_narrow_frames_left_out_and_changes_compared() {
    // Previous window: main 60, gc 40. Current: main 90, gc 10.
    let graph = serde_json::json!({"name":"total","total":200,"self":0,"comp":100,"children":[
        {"name":"gc","total":50,"self":50,"comp":10,"children":null},
        {"name":"main","total":150,"self":50,"comp":90,"children":[
            {"name":"handle","total":100,"self":100,"comp":70,"children":[]},
            {"name":"tiny","total":0,"self":0,"comp":0,"children":[]}]}]});
    let server = server(vec![(200, profile_view(graph, true))]).await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let app = AppId::new("c:prod:Deployment:api");
    let query = ProfileQuery {
        kind: Some("go:profile_cpu:nanoseconds".into()),
        compare: true,
        instance: None,
    };
    let profiling = provider
        .profiling(&source(&provider), range(), &app, &query)
        .await
        .unwrap();
    assert_eq!(profiling.kinds.len(), 2);
    assert_eq!(profiling.instances, vec!["api-0", "api-1"]);
    let graph = profiling.graph.unwrap();
    assert!(graph.compared);
    assert_eq!(graph.omitted, 1);
    let names: Vec<_> = graph.frames.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["total", "gc", "main", "handle"]);
    let main = &graph.frames[2];
    assert_eq!((main.parent, main.depth), (Some(0), 1));
    assert!((main.x - 0.25).abs() < 1e-9 && (main.width - 0.75).abs() < 1e-9);
    assert!((main.change.unwrap() - 30.).abs() < 1e-9);
    assert!((graph.frames[1].change.unwrap() + 30.).abs() < 1e-9);
    assert_eq!(
        ProfileKind::unit("go:heap_inuse_space:bytes"),
        ProfileUnit::Bytes
    );
}

#[test]
fn a_deep_profile_parses_and_a_runaway_one_is_refused() {
    let deep = |levels: usize| {
        let mut body = String::from(
            r#"{"context":{},"data":{"status":"ok","profile":{"type":"t","flamegraph":"#,
        );
        for ix in 0..levels {
            body.push_str(&format!(
                r#"{{"name":"f{ix}","total":10,"self":0,"comp":0,"children":["#
            ));
        }
        body.push_str(&"]}".repeat(levels));
        body.push_str("}}}");
        body.into_bytes()
    };
    let profiling = profiling::decode(deep(1_000)).unwrap();
    let graph = profiling.graph.unwrap();
    assert_eq!(graph.frames.len(), 1_000);
    assert_eq!(graph.frames[999].depth, 999);
    assert_eq!(
        profiling::decode(deep(3_000)).unwrap_err(),
        ReadError::Limit
    );
    assert_eq!(
        profiling::decode(b"{\"data\":".to_vec()).unwrap_err(),
        ReadError::InvalidResponse
    );
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

fn app_view_wire() -> serde_json::Value {
    let ctx = serde_json::json!({"from":1789996400000_i64,"to":1790000000000_i64,"step":60000,"raw_step":60000,"truncated":false});
    serde_json::json!({"context":{},"data":{
        "app_map":{
            "application":{"id":"c:shop:Deployment:auth","cluster":"main","category":"application","status":"ok","icon":"java","labels":{"ns":"shop"}},
            "instances":[{"id":"auth-7d9f-a1"},{"id":"auth-7d9f-b2","labels":{"role":"primary"}}],
            "clients":[{"id":"c:shop:Deployment:shop-api","status":"ok","icon":"golang","link_status":"ok","link_direction":"to","link_stats":["12 rps","3ms"],"link_weight":12.0}],
            "dependencies":[
                {"id":"c:shop:StatefulSet:auth-db","status":"warning","icon":"postgres","link_status":"critical","link_status_reason":"<b>failed</b> connections","link_direction":"both","link_stats":null,"link_weight":0},
                {"id":"c:shop:Deployment:shop-redis","status":"ok","icon":"redis","link_status":"unknown"}],
            "custom_applications":null,"categories":null},
        "reports":[
            {"name":"SLO","status":"unknown","checks":null,"widgets":null},
            {"name":"CPU","status":"ok","checks":[
                {"id":"CPUNode","title":"Node CPU utilization","status":"ok","message":"","threshold":80,"unit":"percent","condition_format_template":"the CPU usage of a node > <threshold>"},
                {"id":"CPUContainer","title":"Container CPU utilization","status":"warning","message":"high CPU utilization","threshold":80,"unit":"percent","condition_format_template":"the CPU usage of a container > <threshold> of its CPU limit"}],
             "widgets":[
                {"chart_group":{"title":"CPU usage <selector>, cores","charts":[
                    {"ctx":ctx,"title":"container: app","series":[{"name":"auth-7d9f-a1","data":[0.1,null,0.2]}],"threshold":{"name":"limit","color":"black","data":[1,1,1]},"featured":true,"stacked":false,"column":false,"color_shift":0,"annotations":[{"name":"deployment","x1":1789998000000_i64,"x2":0,"icon":"mdi-swap"}],"drill_down_link":null,"hide_legend":false},
                    {"ctx":ctx,"title":"overview","series":null,"threshold":null}]},
                 "doc_link":{"group":"inspections","item":"cpu","hash":""}},
                {"chart":{"ctx":ctx,"title":"Node CPU usage, %","series":[{"name":"node-a","color":"red","fill":true,"data":[10,null,12]}],"column":true}},
                {"group_header":"Disks","width":"100%"}]},
            {"name":"Instances","status":"ok","checks":[
                {"id":"InstanceAvailability","title":"Instance availability","status":"ok","threshold":0.75,"unit":"percent","condition_format_template":"the number of available instances < <threshold> of the desired"}],
             "widgets":[{"table":{"header":["Instance","Status","Restarts","Node"],"rows":[
                {"id":"auth-7d9f-a1","cells":[{"value":"auth-7d9f-a1"},{"status":"ok","value":"up (running)"},{"is_stub":true,"value":"—"},{"value":"node-a","link":{"title":"node-a","name":"overview","params":{"view":"nodes","id":"node-a"}}}]}]},
                "width":"100%"}]},
            {"name":"Logs","status":"ok","checks":[{"id":"LogErrors","title":"Errors","status":"ok","threshold":0,"unit":"","condition_format_template":"the number of messages with the ERROR and CRITICAL severity levels > <threshold>"}],
             "widgets":[{"logs":{"application_id":"c:shop:Deployment:auth","check":{"id":"LogErrors","title":"Errors","status":"ok","threshold":0,"condition_format_template":"x > <threshold>"}},"width":"100%"}]},
            {"name":"DNS","status":"ok","checks":[{"id":"DNSLatency","title":"DNS latency","status":"ok","threshold":0.1,"unit":"second","condition_format_template":"the 95th percentile of DNS response times > <threshold>"}],"widgets":[{"dependency_map":{"nodes":[],"links":[]}}]},
            {"name":"Profiling","status":"unknown","widgets":[{"profiling":{"application_id":"c:shop:Deployment:auth"},"width":"100%"}]},
            {"name":"Tracing","status":"unknown","widgets":[{"tracing":{"application_id":"c:shop:Deployment:auth"},"width":"100%"}]}]
    }})
}

#[tokio::test]
async fn app_view_reads_the_map_reports_checks_and_widgets() {
    let server = server(vec![(200, app_view_wire())]).await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let app = AppId::new("c:shop:Deployment:auth");
    let view = provider
        .app_view(&source(&provider), range(), &app)
        .await
        .unwrap();

    let map = &view.map;
    assert_eq!(map.app.id, app);
    assert!(map.app.link.is_none());
    assert_eq!(map.instances.len(), 2);
    assert_eq!(map.instances[1].labels["role"], "primary");
    let client = map.clients[0].link.as_ref().unwrap();
    assert_eq!(client.stats, ["12 rps", "3ms"]);
    assert_eq!(client.weight, Some(12.));
    assert!(!client.both_ways);
    let db = map.dependencies[0].link.as_ref().unwrap();
    assert_eq!(db.status, Status::Critical);
    assert_eq!(db.reason, "failed connections");
    assert!(db.both_ways);
    assert_eq!(db.weight, None);
    assert_eq!(
        map.dependencies[1].link.as_ref().unwrap().status,
        Status::Unknown
    );

    let names: Vec<_> = view.reports.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "SLO",
            "CPU",
            "Instances",
            "Logs",
            "DNS",
            "Profiling",
            "Tracing"
        ]
    );
    assert!(!view.reports[0].judged());

    let cpu = &view.reports[1];
    assert!(cpu.judged());
    assert_eq!(cpu.checks[1].status, Status::Warning);
    assert_eq!(cpu.checks[1].message, "high CPU utilization");
    assert_eq!(
        cpu.checks[1].condition(),
        (
            "the CPU usage of a container > ",
            "80%".into(),
            " of its CPU limit"
        )
    );
    let WidgetKind::ChartGroup { title, charts } = &cpu.widgets[0].kind else {
        panic!("a chart group");
    };
    assert_eq!(title, "CPU usage <selector>, cores");
    assert_eq!(cpu.widgets[0].width, 0.5);
    assert_eq!(charts.len(), 2);
    assert!(charts[0].featured);
    assert_eq!(
        (charts[0].step_ms, charts[0].from_ms),
        (60_000, 1_789_996_400_000)
    );
    assert_eq!(charts[0].series[0].points, [Some(0.1), None, Some(0.2)]);
    assert_eq!(charts[0].threshold.as_ref().unwrap().name, "limit");
    assert_eq!(charts[0].annotations[0].from_ms, 1_789_998_000_000);
    assert!(charts[1].series.is_empty());
    let WidgetKind::Chart(node) = &cpu.widgets[1].kind else {
        panic!("a chart");
    };
    assert!(node.column && node.stacked);
    assert_eq!(node.series[0].color, "red");
    assert!(node.series[0].fill);
    assert!(matches!(&cpu.widgets[2].kind, WidgetKind::Header(h) if h == "Disks"));
    assert_eq!(cpu.widgets[2].width, 1.);

    let instances = &view.reports[2];
    assert_eq!(
        instances.checks[0].condition().1,
        "0.75%",
        "the threshold is shown as Coroot sends it"
    );
    let WidgetKind::Table(table) = &instances.widgets[0].kind else {
        panic!("a table");
    };
    assert_eq!(table.header.len(), 4);
    let row = &table.rows[0];
    assert_eq!(row[1].status, Some(Status::Ok));
    assert!(row[2].stub);
    let link = row[3].link.as_ref().unwrap();
    assert_eq!((link.view.as_str(), link.id.as_str()), ("nodes", "node-a"));

    assert!(
        matches!(&view.reports[3].widgets[0].kind, WidgetKind::Logs(Some(c)) if c.id == "LogErrors")
    );
    let dns = &view.reports[4];
    assert_eq!(dns.checks[0].condition().1, "100ms");
    assert!(matches!(
        dns.widgets[0].kind,
        WidgetKind::Other("dependency map")
    ));
    assert!(matches!(
        view.reports[5].widgets[0].kind,
        WidgetKind::Profiling
    ));
    assert!(matches!(
        view.reports[6].widgets[0].kind,
        WidgetKind::Tracing
    ));
}

#[tokio::test]
async fn app_view_failures_stay_distinct_from_an_empty_page() {
    let mut other = app_view_wire();
    other["data"]["app_map"]["application"]["id"] = "c:shop:Deployment:other".into();
    let mut bad_chart = app_view_wire();
    bad_chart["data"]["reports"][1]["widgets"][1]["chart"]["ctx"]["step"] = 0.into();
    let mut no_map = app_view_wire();
    no_map["data"]["app_map"] = serde_json::Value::Null;
    let mut empty = app_view_wire();
    empty["data"]["reports"] = serde_json::Value::Null;
    let server = server(vec![
        (200, serde_json::json!({"context":{},"data":null})),
        (200, other),
        (200, bad_chart),
        (200, no_map),
        (403, serde_json::json!({})),
        (200, empty),
    ])
    .await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let app = AppId::new("c:shop:Deployment:auth");
    let source = source(&provider);
    let read = || provider.app_view(&source, range(), &app);
    assert_eq!(read().await.unwrap_err(), ReadError::Missing);
    assert_eq!(read().await.unwrap_err(), ReadError::InvalidResponse);
    assert_eq!(read().await.unwrap_err(), ReadError::InvalidResponse);
    assert_eq!(read().await.unwrap_err(), ReadError::InvalidResponse);
    assert_eq!(read().await.unwrap_err(), ReadError::Refused);
    let view = read().await.unwrap();
    assert!(view.reports.is_empty());
    assert_eq!(view.map.instances.len(), 2);
}

#[test]
fn app_view_bounds_reject_runaway_answers() {
    let view = |data: serde_json::Value| app_view::decode(data["data"].clone()).unwrap();
    let mut wire = app_view_wire();
    assert_eq!(limits::app_view(&view(wire.clone())), Ok(()));
    wire["data"]["app_map"]["instances"] = (0..2_001)
        .map(|i| serde_json::json!({"id": format!("i{i}")}))
        .collect();
    assert_eq!(limits::app_view(&view(wire)), Err(ReadError::Limit));

    let mut wire = app_view_wire();
    wire["data"]["reports"][1]["widgets"][1]["chart"]["series"][0]["data"] =
        vec![1.0; 4_097].into();
    assert_eq!(limits::app_view(&view(wire)), Err(ReadError::Limit));

    let mut wire = app_view_wire();
    wire["data"]["reports"][2]["checks"][0]["message"] = "x".repeat(8_193).into();
    assert_eq!(limits::app_view(&view(wire)), Err(ReadError::Limit));
}

#[test]
fn check_thresholds_read_as_coroot_writes_them() {
    let check = |threshold: f32, unit: &str| {
        Check {
            id: String::new(),
            title: String::new(),
            status: Status::Ok,
            message: String::new(),
            threshold,
            unit: unit.into(),
            condition: "x > <threshold>".into(),
        }
        .condition()
        .1
    };
    assert_eq!(check(0., ""), "0");
    assert_eq!(check(1., ""), "1");
    assert_eq!(check(0.01, "second"), "10ms");
    assert_eq!(check(0.05, "second"), "50ms");
    assert_eq!(check(2.5, "second"), "2.5s");
    assert_eq!(check(0.02, "seconds/second"), "0.02 seconds/second");
    assert_eq!(check(75., "percent"), "75%");
    assert_eq!(check(10., "percent"), "10%");
    let none = Check {
        id: String::new(),
        title: String::new(),
        status: Status::Ok,
        message: String::new(),
        threshold: 0.,
        unit: String::new(),
        condition: "IO or SQL replication thread is not running".into(),
    };
    assert_eq!(
        none.condition(),
        (
            "IO or SQL replication thread is not running",
            "0".into(),
            ""
        )
    );
}

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
