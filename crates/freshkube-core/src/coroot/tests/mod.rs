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

mod access;
mod application;
mod history;
mod incidents;
mod logs;
mod maps;
mod outcomes;
mod profiles;
mod reads;
mod revision_windows;
mod revisions;
mod traces;

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
                // A string is sent as it is, as Coroot's plain-text errors are.
                let body = match body {
                    serde_json::Value::String(text) => text,
                    body => body.to_string(),
                };
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
                    {"ctx":ctx,"title":"container: app","series":[{"name":"auth-7d9f-a1","data":[0.1,null,0.2]}],"threshold":{"name":"limit","color":"black","data":[1,1,1]},"featured":true,"stacked":false,"column":false,"color_shift":0,"annotations":[{"name":"deployment","x1":1789998000000_i64,"x2":null,"icon":"mdi-swap"}],"drill_down_link":null,"hide_legend":false},
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

fn decode_wire(wire: serde_json::Value) -> Result<AppView, ReadError> {
    let envelope = coroot_rs::Envelope {
        context: wire["context"].clone(),
        data: wire["data"].clone(),
    };
    app_view::decode_all(envelope, &AppId::new("c:shop:Deployment:auth"))
}

fn deployments_report(rows: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"name":"Deployments","status":"warning","checks":null,"widgets":[{"table":{
        "header":["Deployment","Deployed","Summary"],"rows":rows},"width":"100%"}]})
}

fn deployment_row(id: &str, version: &str, summaries: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"id":id,"cells":[
        {"value":version,"status":"warning"},
        {"value":"1h ago"},
        {"value":"","deployment_summaries":summaries}]})
}
