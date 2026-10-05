//! Reproducible workloads for the performance pass. The app runs in
//! Kubernetes-only mode against a synthetic Kubernetes API on 127.0.0.1, so
//! lists, watches and pod logs go through the real client, decoding and
//! batching. No cluster, kubeconfig or credentials of the user are touched.
//!
//! ```sh
//! cargo run --release -p freshkube-desktop --features stress --bin stress -- <scenario>
//! ```
//!
//! | Scenario | What it does |
//! | --- | --- |
//! | `table <pods>` | lists that many pods, then filters them by typing |
//! | `burst <pods> <changes/s>` | lists the pods, then the watch changes them at that rate |
//! | `summary` | 20,000 Pods, 2,000 Deployments and 5,000 warning Events, idle after sync |
//! | `summary-burst <changes/s>` | the summary world changes continuously |
//! | `summary-410 <changes/s>` | the same changes, with a forced Pod relist after 10 s |
//! | `pod-logs <lines/s>` | opens a pod's Logs tab while its container writes at that rate |
//! | `talos-logs <lines/s>` | example Talos logs, the collected services writing that many lines a second between them |
//! | `terminal <lines/s>` | a window with only a terminal, fed coloured lines at that rate |
//! | `terminal-top` | the terminal, redrawn whole by a `top`-like program about 60 times a second |
//! | `terminal-sample` | the terminal showing its colours, styles and wide characters, for visual checks |
//! | `monitoring <dashboard.json> [processes]` | opens that dashboard against a fake Prometheus with that many Go processes (67), then sweeps the mouse over its first panels and scrolls |
//!
//! The run quits after `FRESHKUBE_STRESS_SECONDS` (30) and prints its
//! timings to stderr; see `src/stress.rs`. `FRESHKUBE_STRESS_KEYS`,
//! `FRESHKUBE_PAGE` and `FRESHKUBE_KIND` override what each scenario sets.
//! Log lines are stamped with the time they were due, so `logs.lag` shows
//! any backlog, wherever it builds up.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use chrono::{SecondsFormat, Utc};
use futures::StreamExt;
use http_body_util::{BodyExt, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::body::Frame;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

mod summary;
use summary::summary_list;

type Body = UnsyncBoxBody<Bytes, Infallible>;

const NAMESPACES: usize = 50;
/// How often generators wake to send what is due.
const TICK: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug)]
enum Scenario {
    Summary,
    SummaryBurst { rate: u32, expire: bool },
    Table { pods: usize },
    Burst { pods: usize, rate: u32 },
    PodLogs { rate: u32 },
    TalosLogs { rate: u32 },
    Terminal { rate: u32 },
    TerminalTop,
    TerminalSample,
    Monitoring { processes: usize },
}

impl Scenario {
    fn parse(args: &[String]) -> Option<Self> {
        let number = |ix: usize, default: u64| -> Option<u64> {
            args.get(ix)
                .map_or(Some(default), |value| value.parse().ok())
        };
        Some(match args.first().map(String::as_str)? {
            "summary" => Scenario::Summary,
            "summary-burst" | "summary-410" => Scenario::SummaryBurst {
                rate: number(1, 2000)? as u32,
                expire: args[0] == "summary-410",
            },
            "table" => Scenario::Table {
                pods: number(1, 20_000)? as usize,
            },
            "burst" => Scenario::Burst {
                pods: number(1, 20_000)? as usize,
                rate: number(2, 2_000)? as u32,
            },
            "pod-logs" => Scenario::PodLogs {
                rate: number(1, 10_000)? as u32,
            },
            "talos-logs" => Scenario::TalosLogs {
                rate: number(1, 10_000)? as u32,
            },
            "terminal" => Scenario::Terminal {
                rate: number(1, 10_000)? as u32,
            },
            "terminal-top" => Scenario::TerminalTop,
            "terminal-sample" => Scenario::TerminalSample,
            // The dashboard's path is the first argument; see `main`.
            "monitoring" if args.len() > 1 => Scenario::Monitoring {
                processes: number(2, 67)? as usize,
            },
            _ => return None,
        })
    }

    /// Where the app opens and what the key script does, unless the
    /// environment already says.
    fn defaults(self) -> Vec<(&'static str, String)> {
        let pods = ("FRESHKUBE_KIND", "pods".to_owned());
        match self {
            Scenario::Summary | Scenario::SummaryBurst { .. } => vec![("FRESHKUBE_PAGE", "health".into())],
            Scenario::Table { .. } => vec![
                pods,
                (
                    "FRESHKUBE_STRESS_KEYS",
                    "wait:8000 / type:app-1 wait:1500 escape wait:1500 / type:crash wait:1500 escape wait:1500 / type:ns-4 wait:1500 escape"
                        .into(),
                ),
            ],
            Scenario::Burst { .. } => vec![pods],
            Scenario::PodLogs { .. } => vec![
                pods,
                (
                    "FRESHKUBE_STRESS_KEYS",
                    "wait:3000 down enter secondary-} secondary-} secondary-}".into(),
                ),
            ],
            Scenario::TalosLogs { rate } => vec![
                ("FRESHKUBE_PAGE", "node-logs".into()),
                ("FRESHKUBE_STRESS_TALOS_RATE", rate.to_string()),
            ],
            Scenario::Terminal { .. } | Scenario::TerminalTop | Scenario::TerminalSample => {
                Vec::new()
            }
            Scenario::Monitoring { .. } => vec![
                ("FRESHKUBE_PAGE", "monitoring".into()),
                (
                    "FRESHKUBE_STRESS_KEYS",
                    "wait:6000 \
                     hover:0.3,0.3,0.6,0.3,2000 hover:0.6,0.3,0.3,0.3,2000 \
                     hover:0.65,0.3,0.95,0.3,2000 hover:0.95,0.3,0.65,0.3,2000 \
                     hover:0.3,0.3,0.6,0.3,2000 hover:0.6,0.3,0.3,0.3,2000 \
                     scroll:0.6,0.6,-40 scroll:0.6,0.6,-40 scroll:0.6,0.6,-40 scroll:0.6,0.6,-40 \
                     scroll:0.6,0.6,-40 scroll:0.6,0.6,-40 scroll:0.6,0.6,-40 scroll:0.6,0.6,-40 \
                     scroll:0.6,0.6,-40 scroll:0.6,0.6,-40 scroll:0.6,0.6,-40 scroll:0.6,0.6,-40 \
                     wait:500 hover:0.3,0.5,0.6,0.5,2000 hover:0.6,0.5,0.3,0.5,2000"
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
            ],
        }
    }
}

fn main() -> color_eyre::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(scenario) = Scenario::parse(&args) else {
        eprintln!(
            "usage: stress summary | table [pods] | burst [pods] [changes/s] | pod-logs [lines/s] | talos-logs [lines/s] | terminal [lines/s] | terminal-top | terminal-sample | monitoring <dashboard.json> [processes]"
        );
        std::process::exit(2);
    };
    let dir = std::env::temp_dir().join(format!("freshkube-stress-{}", std::process::id()));
    if let Scenario::Monitoring { .. } = scenario {
        // A home of its own: the dashboard in a folder Monitoring reads, and
        // the fake Prometheus remembered for the context.
        let dashboards = dir.join("dashboards");
        let support = dir.join("home/Library/Application Support/Freshkube");
        std::fs::create_dir_all(&dashboards)?;
        std::fs::create_dir_all(&support)?;
        let dashboard = dashboards.join("dashboard.json");
        std::fs::copy(&args[1], &dashboard)?;
        let (namespace, name, port) = PROMETHEUS;
        std::fs::write(
            support.join("monitoring.json"),
            json!({
                "dashboards": dashboards,
                "prometheus": {"stress": {"namespace": namespace, "name": name, "port": port}}
            })
            .to_string(),
        )?;
        // SAFETY: no other thread has started yet.
        unsafe {
            std::env::set_var("HOME", dir.join("home"));
            std::env::set_var("FRESHKUBE_DASHBOARD", dashboard);
        }
    }
    for (name, value) in scenario.defaults() {
        if std::env::var_os(name).is_none() {
            // SAFETY: no other thread has started yet.
            unsafe { std::env::set_var(name, value) };
        }
    }
    eprintln!("stress scenario {scenario:?}");
    let terminal = match scenario {
        Scenario::Terminal { rate } => Some(freshkube_desktop::TerminalWorkload::Flood {
            lines_per_second: rate,
        }),
        Scenario::TerminalTop => Some(freshkube_desktop::TerminalWorkload::Top),
        Scenario::TerminalSample => Some(freshkube_desktop::TerminalWorkload::Sample),
        _ => None,
    };
    if let Some(workload) = terminal {
        return freshkube_desktop::run_terminal(workload);
    }
    let runtime = tokio::runtime::Runtime::new()?;
    if let Scenario::TalosLogs { .. } = scenario {
        return freshkube_desktop::run(
            freshkube_desktop::GpuiOptions::fixture(),
            runtime.handle().clone(),
        );
    }
    let world = Arc::new(World::new(scenario));
    let address = runtime.block_on(serve(world.clone()))?;
    std::fs::create_dir_all(&dir)?;
    let kubeconfig = dir.join("kubeconfig");
    std::fs::write(
        &kubeconfig,
        format!(
            "apiVersion: v1\nkind: Config\ncurrent-context: stress\n\
             clusters:\n- name: stress\n  cluster:\n    server: http://{address}\n\
             users:\n- name: stress\n  user: {{}}\n\
             contexts:\n- name: stress\n  context:\n    cluster: stress\n    user: stress\n"
        ),
    )?;
    let result = freshkube_desktop::run(
        freshkube_desktop::GpuiOptions::kubernetes_only(
            Some(kubeconfig),
            Some("stress".into()),
            100,
        ),
        runtime.handle().clone(),
    );
    world.summary.report();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// The synthetic cluster: pods, and the rates its watch and logs run at.
struct World {
    scenario: Scenario,
    pods: Mutex<Pods>,
    summary: summary::State,
    started: Instant,
    created: chrono::DateTime<Utc>,
}

struct Pods {
    rows: Vec<Pod>,
    version: u64,
    next: u64,
    /// A small deterministic generator, so every run changes the same pods.
    seed: u64,
}

#[derive(Clone)]
struct Pod {
    namespace: String,
    name: String,
    uid: String,
    status: &'static str,
    restarts: u32,
    version: u64,
}

impl Pods {
    fn pod(&mut self) -> Pod {
        let id = self.next;
        self.next += 1;
        self.version += 1;
        Pod {
            namespace: format!("ns-{:02}", id as usize % NAMESPACES),
            name: format!(
                "app-{}-{:08x}-{id}",
                id % 400,
                id.wrapping_mul(2_654_435_761) as u32
            ),
            uid: format!("00000000-0000-4000-8000-{id:012}"),
            status: "Running",
            restarts: 0,
            version: self.version,
        }
    }

    fn random(&mut self) -> u64 {
        self.seed = self
            .seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.seed >> 33
    }
}

impl World {
    fn new(scenario: Scenario) -> Self {
        let count = match scenario {
            Scenario::Summary | Scenario::SummaryBurst { .. } => 20_000,
            Scenario::Table { pods } | Scenario::Burst { pods, .. } => pods,
            _ => 20,
        };
        let mut pods = Pods {
            rows: Vec::new(),
            version: 1,
            next: 0,
            seed: 42,
        };
        pods.rows = (0..count).map(|_| pods.pod()).collect();
        // Some variety for filters and sorting.
        for (ix, pod) in pods.rows.iter_mut().enumerate() {
            if ix % 37 == 0 {
                pod.status = "CrashLoopBackOff";
                pod.restarts = (ix % 9) as u32 + 1;
            }
        }
        Self {
            scenario,
            pods: Mutex::new(pods),
            summary: summary::State::default(),
            started: Instant::now(),
            created: Utc::now(),
        }
    }
}

async fn serve(world: Arc<World>) -> std::io::Result<std::net::SocketAddr> {
    summary::start(world.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let world = world.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request| {
                    let world = world.clone();
                    async move { Ok::<_, Infallible>(respond(&world, request)) }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    Ok(address)
}

fn respond(world: &Arc<World>, request: Request<hyper::body::Incoming>) -> Response<Body> {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or("").to_owned();
    let param = |name: &str| {
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name).then(|| summary::decode(value))
        })
    };
    let watching = matches!(param("watch").as_deref(), Some("1" | "true"));
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    let table = request
        .headers()
        .get("accept")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| accept.contains("as=Table"));
    if summary::supported(&path) || path == "/version" {
        world.summary.count(&path, watching, table);
    }
    if watching && summary::supported(&path) {
        return summary::watch(world, &path, &param, table);
    }
    if !watching
        && !table
        && let Some(response) = summary_list(world, &path, &param)
    {
        return response;
    }

    match segments.as_slice() {
        ["version"] => json_response(json!({
            "major": "1", "minor": "32", "gitVersion": "v1.32.3", "platform": "linux/amd64"
        })),
        ["api"] => json_response(json!({"kind": "APIVersions", "versions": ["v1"]})),
        ["apis"] => {
            json_response(json!({"kind": "APIGroupList", "apiVersion": "v1", "groups": []}))
        }
        [
            "api",
            "v1",
            "namespaces",
            _,
            "services",
            _,
            "proxy",
            "api",
            "v1",
            rest @ ..,
        ] => {
            let Scenario::Monitoring { processes } = world.scenario else {
                return json_response(json!({}));
            };
            prometheus(processes, rest, &query)
        }
        ["apis", "metrics.k8s.io", "v1beta1", "nodes"] => summary::node_metrics(),
        ["api", "v1", "services"] if watching => idle_stream(),
        ["api", "v1", "services"] => json_response(service_list()),
        ["api", "v1", "namespaces"] if watching => idle_stream(),
        ["api", "v1", "namespaces"] => json_response(namespace_table()),
        ["api", "v1", "pods"] if watching => summary::watch(world, &path, &param, table),
        ["api", "v1", "pods"] => summary::pods(world, None, &param, true),
        ["api", "v1", "namespaces", _, "pods"] if watching => idle_stream(),
        ["api", "v1", "namespaces", namespace, "pods"] => {
            summary::pods(world, Some(namespace), &param, true)
        }
        ["api", "v1", "namespaces", namespace, "pods", name] => pod_object(world, namespace, name),
        ["api", "v1", "namespaces", _, "pods", _, "log"] => log_stream(world),
        [.., "events"] if watching => idle_stream(),
        [.., "events"] => json_response(json!({
            "kind": "EventList", "apiVersion": "v1",
            "metadata": {"resourceVersion": "1"}, "items": []
        })),
        _ => {
            eprintln!("stress server: no answer for {path}");
            let status = json!({
                "kind": "Status", "apiVersion": "v1", "status": "Failure",
                "reason": "NotFound", "code": 404, "message": "not served by the stress server"
            });
            let mut response = json_response(status);
            *response.status_mut() = StatusCode::NOT_FOUND;
            response
        }
    }
}

fn json_response(value: Value) -> Response<Body> {
    Response::builder()
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(value.to_string())).boxed_unsync())
        .expect("a response")
}

fn stream_response(receiver: mpsc::Receiver<Bytes>, content_type: &str) -> Response<Body> {
    let stream = futures::stream::unfold(receiver, |mut receiver| async move {
        let chunk = receiver.recv().await?;
        Some((Ok::<_, Infallible>(Frame::data(chunk)), receiver))
    });
    Response::builder()
        .header("content-type", content_type)
        .body(StreamBody::new(stream.boxed()).boxed_unsync())
        .expect("a response")
}

/// A watch that stays open and reports nothing.
fn idle_stream() -> Response<Body> {
    let (sender, receiver) = mpsc::channel(1);
    tokio::spawn(async move { sender.closed().await });
    stream_response(receiver, "application/json")
}

fn namespace_table() -> Value {
    let rows: Vec<Value> = (0..NAMESPACES)
        .map(|ix| {
            let name = format!("ns-{ix:02}");
            json!({
                "cells": [name, "Active", "1y"],
                "object": {"kind": "PartialObjectMetadata", "metadata": {
                    "name": name, "uid": format!("ns-{ix}"), "resourceVersion": "1",
                    "creationTimestamp": "2025-06-01T00:00:00Z"
                }}
            })
        })
        .collect();
    json!({
        "kind": "Table", "apiVersion": "meta.k8s.io/v1",
        "metadata": {"resourceVersion": "1"},
        "columnDefinitions": [
            {"name": "Name", "type": "string", "format": "name", "description": "", "priority": 0},
            {"name": "Status", "type": "string", "format": "", "description": "", "priority": 0},
            {"name": "Age", "type": "string", "format": "", "description": "", "priority": 0}
        ],
        "rows": rows
    })
}

fn pod_columns() -> Value {
    json!([
        {"name": "Name", "type": "string", "format": "name", "description": "", "priority": 0},
        {"name": "Ready", "type": "string", "format": "", "description": "", "priority": 0},
        {"name": "Status", "type": "string", "format": "", "description": "", "priority": 0},
        {"name": "Restarts", "type": "string", "format": "", "description": "", "priority": 0},
        {"name": "Age", "type": "string", "format": "", "description": "", "priority": 0},
        {"name": "IP", "type": "string", "format": "", "description": "", "priority": 1},
        {"name": "Node", "type": "string", "format": "", "description": "", "priority": 1}
    ])
}

fn pod_row(pod: &Pod) -> Value {
    let ready = if pod.status == "Running" {
        "1/1"
    } else {
        "0/1"
    };
    json!({
        "cells": [pod.name, ready, pod.status, pod.restarts.to_string(), "5d", "10.0.0.1", "node-1"],
        "object": summary::pod_value(pod)
    })
}

fn pod_object(world: &World, namespace: &str, name: &str) -> Response<Body> {
    let pods = world.pods.lock().expect("pods");
    let Some(pod) = pods
        .rows
        .iter()
        .find(|pod| pod.namespace == namespace && pod.name == name)
    else {
        let mut response = json_response(json!({
            "kind": "Status", "apiVersion": "v1", "status": "Failure",
            "reason": "NotFound", "code": 404, "message": format!("pods \"{name}\" not found")
        }));
        *response.status_mut() = StatusCode::NOT_FOUND;
        return response;
    };
    json_response(summary::pod_value(pod))
}

/// A container writing `rate` lines a second, each stamped with the time it
/// was due. Every fiftieth line is long, so wrapping has work to do.
fn log_stream(world: &World) -> Response<Body> {
    let Scenario::PodLogs { rate } = world.scenario else {
        return idle_stream();
    };
    let (sender, receiver) = mpsc::channel(16);
    tokio::spawn(async move {
        let started_at = Utc::now();
        let started = Instant::now();
        let mut sent = 0u64;
        let mut tick = tokio::time::interval(TICK);
        loop {
            tick.tick().await;
            let due = (started.elapsed().as_secs_f64() * f64::from(rate)) as u64;
            let mut chunk = String::new();
            while sent < due && chunk.len() < 1 << 20 {
                let at = started_at
                    + chrono::Duration::nanoseconds((sent as f64 * 1e9 / f64::from(rate)) as i64);
                let stamp = at.to_rfc3339_opts(SecondsFormat::Nanos, true);
                chunk.push_str(&stamp);
                chunk.push_str(&format!(
                    " level=info msg=\"request handled\" path=/api/v1/items/{} status=200 duration={}.{}ms seq={sent}",
                    sent % 9973,
                    sent % 17,
                    sent % 10
                ));
                if sent.is_multiple_of(50) {
                    chunk.push_str(" detail=");
                    chunk.push_str(&"abcdefghij".repeat(30));
                }
                chunk.push('\n');
                sent += 1;
            }
            if !chunk.is_empty() && sender.send(Bytes::from(chunk)).await.is_err() {
                return;
            }
        }
    });
    stream_response(receiver, "text/plain")
}

/// The fake Prometheus the `monitoring` scenario reaches through the service
/// proxy, as `monitoring/prometheus-operated:9090`.
const PROMETHEUS: (&str, &str, u16) = ("monitoring", "prometheus-operated", 9090);
/// GC duration summaries carry these quantiles, five series a process.
const QUANTILES: [&str; 5] = ["0", "0.25", "0.5", "0.75", "1"];

fn service_list() -> Value {
    let (namespace, name, port) = PROMETHEUS;
    json!({
        "kind": "ServiceList", "apiVersion": "v1",
        "metadata": {"resourceVersion": "1"},
        "items": [{
            "metadata": {
                "name": name, "namespace": namespace,
                "labels": {"operated-prometheus": "true"}
            },
            "spec": {"ports": [{"name": "web", "port": port, "protocol": "TCP"}]}
        }]
    })
}

/// One process a Go exporter reports for: the labels every series carries.
fn process(ix: usize) -> [(&'static str, String); 4] {
    let namespace = format!("ns-{:02}", ix % 12);
    [
        ("namespace", namespace),
        (
            "pod",
            format!("app-{ix}-{:05x}", ix.wrapping_mul(40_503) % 0xfffff),
        ),
        ("instance", format!("10.0.{}.{}:8080", ix / 250, ix % 250)),
        ("job", format!("app-{}", ix % 30)),
    ]
}

/// Answers `/api/v1/<rest>` as Prometheus would, for `processes` Go
/// processes: every query draws one series per process, or five for a GC
/// duration summary, whatever its matchers say.
fn prometheus(processes: usize, rest: &[&str], query: &str) -> Response<Body> {
    let params: Vec<(String, String)> = query
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((decode(key), decode(value)))
        })
        .collect();
    let param = |name: &str| {
        params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    let success = |data: Value| json_response(json!({"status": "success", "data": data}));
    match rest {
        ["status", "buildinfo"] => success(json!({"version": "3.4.0"})),
        ["targets"] => success(json!({
            "activeTargets": [{"scrapeInterval": "30s", "health": "up"}],
            "droppedTargets": []
        })),
        ["labels"] => success(json!(["__name__", "instance", "job", "namespace", "pod"])),
        ["label", label, "values"] => {
            let mut values: Vec<String> = (0..processes)
                .filter_map(|ix| {
                    process(ix)
                        .into_iter()
                        .find(|(key, _)| key == label)
                        .map(|(_, value)| value)
                })
                .collect();
            values.sort();
            values.dedup();
            success(json!(values))
        }
        ["series"] => {
            let series: Vec<Value> = (0..processes).map(|ix| labels(ix, None, None)).collect();
            success(json!(series))
        }
        ["query" | "query_range"] => {
            let expr = param("query").unwrap_or("");
            let metric = metric_name(expr);
            let quantiles: &[&str] = if metric.ends_with("_duration_seconds") {
                &QUANTILES
            } else {
                &[""]
            };
            let range = rest == ["query_range"];
            let number = |name: &str| param(name).and_then(|value| value.parse::<f64>().ok());
            let (start, end, step) = match (number("start"), number("end"), number("step")) {
                (Some(start), Some(end), Some(step)) if range && step > 0. => (start, end, step),
                _ => {
                    let time = number("time").unwrap_or(Utc::now().timestamp() as f64);
                    (time, time, 1.)
                }
            };
            let mut result = Vec::new();
            for ix in 0..processes {
                for (qx, quantile) in quantiles.iter().enumerate() {
                    let seed = (ix * 7 + qx) as f64;
                    let at = |time: f64| {
                        let wave =
                            (time / 600. + seed).sin() * 0.3 + (time / 97. + seed * 3.).sin() * 0.1;
                        let value = (1. + seed % 11.) * (1. + qx as f64) * (1.5 + wave);
                        json!([time, format!("{value:.4}")])
                    };
                    let metric =
                        labels(ix, Some(metric), (!quantile.is_empty()).then_some(quantile));
                    result.push(if range {
                        let count = ((end - start) / step).floor() as usize + 1;
                        // A quarter of the processes were replaced partway,
                        // as restarted pods are: their lines end early.
                        let count = if ix % 4 == 0 { count * 7 / 10 } else { count };
                        let values: Vec<Value> =
                            (0..count).map(|n| at(start + n as f64 * step)).collect();
                        json!({"metric": metric, "values": values})
                    } else {
                        json!({"metric": metric, "value": at(end)})
                    });
                }
            }
            let kind = if range { "matrix" } else { "vector" };
            success(json!({"resultType": kind, "result": result}))
        }
        _ => {
            eprintln!("stress prometheus: no answer for {}", rest.join("/"));
            let mut response = json_response(json!({
                "status": "error", "errorType": "not_found", "error": "not served"
            }));
            *response.status_mut() = StatusCode::NOT_FOUND;
            response
        }
    }
}

fn labels(ix: usize, name: Option<&str>, quantile: Option<&str>) -> Value {
    let mut labels: serde_json::Map<String, Value> = process(ix)
        .into_iter()
        .map(|(key, value)| (key.to_owned(), Value::String(value)))
        .collect();
    if let Some(name) = name.filter(|name| !name.is_empty()) {
        labels.insert("__name__".into(), json!(name));
    }
    if let Some(quantile) = quantile {
        labels.insert("quantile".into(), json!(quantile));
    }
    Value::Object(labels)
}

/// The first metric selector's name: the identifier before the first `{`.
fn metric_name(expr: &str) -> &str {
    let head = expr.split('{').next().unwrap_or("");
    let start = head
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':'))
        .map_or(0, |ix| ix + 1);
    &head[start..]
}

/// Percent-decoding for query parameters.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut ix = 0;
    while ix < bytes.len() {
        match bytes[ix] {
            b'%' if ix + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[ix + 1..ix + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        ix += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            byte => out.push(byte),
        }
        ix += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
