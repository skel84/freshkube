//! Exercises the real Tokio provider → GPUI publication path with sanitized HTTP.
mod connection;
mod deployments;
mod incidents;
mod logs;
mod traces;

use super::{Destination, ObservabilityPage, Report, Status, tests::mount};
use freshkube_core::coroot as api;
use freshkube_ui::real_io::RealIo;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU16, AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Server {
    url: String,
    status: Arc<AtomicU16>,
    requests: Arc<AtomicUsize>,
    paths: Arc<Mutex<Vec<String>>>,
    incident_mode: Arc<AtomicUsize>,
    incident_gate: Arc<tokio::sync::Notify>,
    /// The status the logs path answers with while the rest answer 200.
    logs_status: Arc<AtomicU16>,
    /// 1: the application's page lists two revisions; 2: also, the window
    /// around one answers Coroot's 404; 3: the window answers 500; 4: the
    /// newest revision's ±30 min window waits for `window_gate`; 5: the
    /// window answers 403.
    deploy_mode: Arc<AtomicUsize>,
    window_gate: Arc<tokio::sync::Notify>,
    /// The paths answered, once the answer is written.
    responded: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    fn new(runtime: &tokio::runtime::Runtime) -> Self {
        let _enter = runtime.enter();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        let status = Arc::new(AtomicU16::new(200));
        let requests = Arc::new(AtomicUsize::new(0));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let (state, reads, captured) = (status.clone(), requests.clone(), paths.clone());
        let incident_mode = Arc::new(AtomicUsize::new(0));
        let incident_gate = Arc::new(tokio::sync::Notify::new());
        let (mode, gate) = (incident_mode.clone(), incident_gate.clone());
        let logs_status = Arc::new(AtomicU16::new(200));
        let logs_state = logs_status.clone();
        let deploy_mode = Arc::new(AtomicUsize::new(0));
        let deploys = deploy_mode.clone();
        let window_gate = Arc::new(tokio::sync::Notify::new());
        let held = window_gate.clone();
        let responded = Arc::new(Mutex::new(Vec::new()));
        let written = responded.clone();
        let task=runtime.spawn(async move {
            loop {
                let (mut socket,_)=listener.accept().await.unwrap();
                let (state,reads,captured,mode,gate,logs_state,deploys,held,written)=(state.clone(),reads.clone(),captured.clone(),mode.clone(),gate.clone(),logs_state.clone(),deploys.clone(),held.clone(),written.clone());
                tokio::spawn(async move {
                    let mut buf=vec![0;16384]; let Ok(n)=socket.read(&mut buf).await else{return;};
                    let request=String::from_utf8_lossy(&buf[..n]);
                    reads.fetch_add(1,Ordering::SeqCst);
                    let path=request.split_whitespace().nth(1).unwrap_or("");
                    captured.lock().unwrap().push(path.to_string());
                    // Like Coroot, any API key but the right one, exactly, is refused.
                    let key=request.lines().find_map(|line| line.strip_prefix("authorization: ").or_else(|| line.strip_prefix("Authorization: ")));
                    let status=if key.is_some_and(|key| key!="Bearer sanitized-key") {401} else if path.contains("mcp") {404} else {state.load(Ordering::SeqCst)};
                    let status=if path.contains("/logs") && status==200 {logs_state.load(Ordering::SeqCst)} else {status};
                    let selected_mode=mode.load(Ordering::SeqCst);
                    if path.contains("/incident/k1") && selected_mode==5 {gate.notified().await;}
                    if path.contains(NEWEST_WINDOW) && deploys.load(Ordering::SeqCst)==4 {held.notified().await;}
                    let body=if path.contains("/incidents") {
                        let a=incident_wire("k1"); let b=incident_wire("k2");
                        match selected_mode {
                            1=>serde_json::json!([b,a]),2=>serde_json::json!([]),3=>serde_json::json!({}),4=>serde_json::json!([b]),
                            _=>serde_json::json!([a,b]),
                        }
                    } else if path.contains("/incident/") {
                        let mut v=incident_wire(if path.contains("/incident/k2"){"k2"}else{"k1"});
                        v["availability_slo"]=serde_json::json!({"objective":"99% of requests should not fail","compliance":"97.2%","violated":true,"threshold":0});
                        v["details"]=serde_json::json!({"availability_burn_rates":[{"severity":"critical","long_window":3600000,"short_window":300000,"long_window_burn_rate":18.4,"short_window_burn_rate":21,"threshold":6}]});
                        v["rca"]=serde_json::json!({"status":"OK","root_cause":"Source-reported cause","propagation_map":{"applications":[{"id":"cluster-a:prod:Deployment:worker","status":"critical","issues":["Reported problem"]}]}});
                        v
                    } else if path.contains("api/user") {
                        serde_json::json!({"projects":[{"id":"p1","name":"Same name"},{"id":"p2","name":"Same name"}]})
                    } else if path.contains("/tracing") {
                        tracing_wire(path)
                    } else if path.contains("/profiling") {
                        profiling_wire(path)
                    } else if path.contains("/logs") {
                        logs_wire(path)
                    } else if path.contains("/app/") {
                        // The application the path names, as Coroot answers for it.
                        let id=path.split("/app/").nth(1).unwrap_or("").split('?').next().unwrap_or("").replace("%3A",":");
                        let mut page=serde_json::json!({"app_map":{"application":{"id":id,"status":"warning"},"dependencies":[{"id":"cluster-a:prod:Deployment:db","status":"ok","link":{"status":"critical"}}]},"reports":[{"name":"CPU","status":"unknown","checks":[{"id":"check","title":"Measured by Coroot","status":"unknown","message":"No measurement available","threshold":80,"unit":"percent","condition_format_template":"the CPU usage > <threshold>"}],"widgets":[{"table":{"header":["Container","Usage"],"rows":[{"cells":[{"value":"api"},{"value":"0.2","unit":"cores"}]}]},"width":"100%"}]},{"name":"Logs","status":"ok","checks":[],"widgets":[{"logs":{},"width":"100%"}]}]});
                        if deploys.load(Ordering::SeqCst)>0 {
                            page["reports"].as_array_mut().unwrap().push(deployments_wire());
                        }
                        page
                    } else if path.contains("map") {
                        serde_json::json!({"map":[]})
                    } else if status==204 {
                        serde_json::json!({"applications":[]})
                    } else {
                        serde_json::json!({"applications":[{"id":"cluster-a:prod:Deployment:api","category":"application","status":"warning","cpu":{"status":"ok","value":""},"memory":{"status":"unknown","value":""}}]})
                    };
                    let body = if path.contains("api/user") {body} else {serde_json::json!({"context":{},"data":body})}.to_string();
                    let code=if status==204 {200} else {status};
                    // The window around the revision at 1789999000 s.
                    let window=path.contains("/app/") && path.contains("from=17899");
                    let (code,body,kind)=match deploys.load(Ordering::SeqCst) {
                        2 if window=>(404,"Application not found\n".to_owned(),"text/plain"),
                        3 if window=>(500,"internal error".to_owned(),"text/plain"),
                        5 if window=>(403,"forbidden".to_owned(),"text/plain"),
                        _=>(code,body,"application/json"),
                    };
                    let response=format!("HTTP/1.1 {code} Test\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                    _=socket.write_all(response.as_bytes()).await;
                    written.lock().unwrap().push(path.to_string());
                });
            }
        });
        Self {
            url,
            status,
            requests,
            paths,
            incident_mode,
            incident_gate,
            logs_status,
            deploy_mode,
            window_gate,
            responded,
            task,
        }
    }
}

async fn loaded(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ObservabilityPage>,
) {
    let page = page.clone();
    cx.wait_for_io(handle, move |_, cx| !page.read(cx).live.apps.is_loading())
        .await;
}

fn incident_wire(key: &str) -> serde_json::Value {
    serde_json::json!({"key":key,"application_id":"cluster-a:prod:Deployment:api","cluster":"Production","severity":"critical","opened_at":1790000000000_i64,"resolved_at":null,"impact":2.8,"duration":120000,"short_description":"Source SLO violation"})
}

async fn settled(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ObservabilityPage>,
) {
    let observed = page.clone();
    cx.wait_for_io(handle, move |_, cx| !observed.read(cx).live.connecting)
        .await;
}

fn span_wire(id: &str, parent: &str, at: i64, ms: f64, error: bool) -> serde_json::Value {
    serde_json::json!({"service":"api","trace_id":"t1","id":id,"parent_id":parent,
        "name":format!("op {id}"),"timestamp":at,"duration":ms,"client":"web",
        "status":{"error":error,"message":if error {"HTTP 503"} else {""}},
        "details":{"text":"","lang":""},"attributes":{"http.route":"/cart"},"events":null})
}

/// A healthy request in another trace, listed after the failed one.
fn other_trace(at: i64) -> serde_json::Value {
    let mut span = span_wire("other", "", at - 60_000, 40., false);
    span["trace_id"] = "t2".into();
    span
}

fn tracing_wire(path: &str) -> serde_json::Value {
    let at = 1_789_999_000_000_i64;
    let sources = serde_json::json!([
        {"type":"otel","name":"OpenTelemetry","selected":!path.contains("agent")},
        {"type":"agent","name":"OpenTelemetry (eBPF)","selected":path.contains("agent")}]);
    if path.contains("t1%3A%3A") {
        return serde_json::json!({"status":"ok","message":"","sources":sources,"heatmap":null,
            "spans":[span_wire("root","",at,100.,false),span_wire("child","root",at+40,20.,true)],"limit":0});
    }
    serde_json::json!({"status":"ok","message":"Using traces of <i>api</i>","sources":sources,
        "heatmap":{"ctx":{"from":at-3_600_000,"to":at,"step":60_000},"title":"",
            "series":[
                {"name":"5ms","value":"0.005","data":[1.5,null,2]},
                {"name":">5s","value":"inf","data":[null,null,0.1]},
                {"name":"errors","value":"err","data":[0,0.2,null]}],"annotations":null},
        "spans":[span_wire("root","",at,100.,true),other_trace(at)],"limit":0})
}

fn profiling_wire(path: &str) -> serde_json::Value {
    let diff = path.contains("diff");
    serde_json::json!({"status":"ok","message":"OK","services":[],
        "profiles":[{"type":"go:profile_cpu:nanoseconds","name":"CPU"},{"type":"go:heap_inuse_space:bytes","name":"Memory"}],
        "profile":{"type":"go:profile_cpu:nanoseconds","diff":diff,"flamegraph":
            {"name":"total","total":200,"self":0,"comp":100,"children":[
                {"name":"gc","total":50,"self":50,"comp":10,"children":null},
                {"name":"main","total":150,"self":50,"comp":90,"children":[
                    {"name":"handle","total":100,"self":100,"comp":70,"children":[]}]}]}},
        "chart":null,"instances":["api-0","api-1"]})
}

async fn connected(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ObservabilityPage>,
    url: String,
) {
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.url
                .update(cx, |input, cx| input.set_value(url, window, cx));
            page.secret
                .update(cx, |input, cx| input.set_value("sanitized-key", window, cx));
            page.connect(cx);
        })
    })
    .unwrap();
    settled(cx, handle, page).await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let project = page.live.projects[0].clone();
            page.select_project(&project, cx);
        })
    });
    loaded(cx, handle, page).await;
}

fn last_path(server: &Server, part: &str) -> String {
    server
        .paths
        .lock()
        .unwrap()
        .iter()
        .rev()
        // The selected trace is read after its list; skip it.
        .find(|path| path.contains(part) && !path.contains("t1%3A%3A"))
        .cloned()
        .unwrap_or_default()
}

/// Coroot's logs answer: two messages, or on a refresh the newest again
/// with one after it, as Coroot's `since` returns the newest message's own
/// millisecond.
fn logs_wire(path: &str) -> serde_json::Value {
    let refresh = !path.contains("since%22%3A%22%22");
    let t = 1_789_999_000_000_i64;
    let entry = |ms: i64, message: &str| serde_json::json!({"timestamp":ms,"severity":"info","message":message,"attributes":{"host.name":"app-a"},"trace_id":""});
    let (entries, newest) = if refresh {
        (
            vec![
                entry(t + 2000, "third"),
                entry(t + 1000, "second\n  continued"),
            ],
            t + 2000,
        )
    } else {
        (
            vec![entry(t + 1000, "second\n  continued"), entry(t, "first")],
            t + 1000,
        )
    };
    serde_json::json!({"status":"ok","message":"Using container logs","sources":["agent"],"source":"agent",
        "view":"messages","entries":entries,"patterns":null,"limit":0,"max_ts":(newest * 1_000_000).to_string(),
        "chart":{"ctx":{"from":t - 3_600_000,"to":t + 3_600_000,"step":60_000},"title":"","column":true,
            "series":[{"name":"info","color":"blue-lighten2","data":[1,2]}]}})
}

fn retained(cx: &mut TestAppContext, page: &gpui_kit::Entity<ObservabilityPage>) -> Vec<String> {
    cx.read(|cx| {
        let view = page.read(cx).live_logs.view.read(cx);
        view.retained().iter().map(|e| e.raw.clone()).collect()
    })
}

/// The newest revision's ±30 min window, as its request names it.
const NEWEST_WINDOW: &str = "from=1789997200000";

/// Coroot's Deployments report: two revisions, newest first.
fn deployments_wire() -> serde_json::Value {
    let row = |id: &str, version: &str, summaries: serde_json::Value| {
        // Without findings, Coroot sends its note as a stub.
        let summary = if summaries.as_array().is_some_and(Vec::is_empty) {
            serde_json::json!({"value":"No notable changes","is_stub":true})
        } else {
            serde_json::json!({"value":"","deployment_summaries":summaries})
        };
        serde_json::json!({"id":id,"cells":[
            {"value":version,"status":"ok"},{"value":"1h ago"},summary]})
    };
    serde_json::json!({"name":"Deployments","status":"ok","checks":null,"widgets":[{"table":{
        "header":["Deployment","Deployed","Summary"],"rows":[
            row("9c41e7:1789999000","9c41e7: example.test/shop/api:1.4.0",
                serde_json::json!([{"report":"SLO","ok":true,"message":"Availability: 100% (objective: 99%)","time":null}])),
            row("2b70aa:1789990000","2b70aa: example.test/shop/api:1.3.9",serde_json::json!([])),
        ]},"width":"100%"}]})
}
