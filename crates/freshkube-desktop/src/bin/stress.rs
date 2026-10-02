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
//! | `pod-logs <lines/s>` | opens a pod's Logs tab while its container writes at that rate |
//! | `talos-logs <lines/s>` | example Talos logs, the collected services writing that many lines a second between them |
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

type Body = UnsyncBoxBody<Bytes, Infallible>;

const NAMESPACES: usize = 50;
/// How often generators wake to send what is due.
const TICK: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug)]
enum Scenario {
    Table { pods: usize },
    Burst { pods: usize, rate: u32 },
    PodLogs { rate: u32 },
    TalosLogs { rate: u32 },
}

impl Scenario {
    fn parse(args: &[String]) -> Option<Self> {
        let number = |ix: usize, default: u64| -> Option<u64> {
            args.get(ix)
                .map_or(Some(default), |value| value.parse().ok())
        };
        Some(match args.first().map(String::as_str)? {
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
            _ => return None,
        })
    }

    /// Where the app opens and what the key script does, unless the
    /// environment already says.
    fn defaults(self) -> Vec<(&'static str, String)> {
        let pods = ("FRESHKUBE_KIND", "pods".to_owned());
        match self {
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
                ("FRESHKUBE_PAGE", "logs".into()),
                ("FRESHKUBE_STRESS_TALOS_RATE", rate.to_string()),
            ],
        }
    }
}

fn main() -> color_eyre::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(scenario) = Scenario::parse(&args) else {
        eprintln!(
            "usage: stress table [pods] | burst [pods] [changes/s] | pod-logs [lines/s] | talos-logs [lines/s]"
        );
        std::process::exit(2);
    };
    for (name, value) in scenario.defaults() {
        if std::env::var_os(name).is_none() {
            // SAFETY: no other thread has started yet.
            unsafe { std::env::set_var(name, value) };
        }
    }
    eprintln!("stress scenario {scenario:?}");
    let runtime = tokio::runtime::Runtime::new()?;
    if let Scenario::TalosLogs { .. } = scenario {
        return freshkube_desktop::run(
            freshkube_desktop::GpuiOptions::fixture(),
            runtime.handle().clone(),
        );
    }
    let world = Arc::new(World::new(scenario));
    let address = runtime.block_on(serve(world))?;
    let dir = std::env::temp_dir().join(format!("freshkube-stress-{}", std::process::id()));
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
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// The synthetic cluster: pods, and the rates its watch and logs run at.
struct World {
    scenario: Scenario,
    pods: Mutex<Pods>,
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
            Scenario::Table { pods } | Scenario::Burst { pods, .. } => pods,
            Scenario::PodLogs { .. } | Scenario::TalosLogs { .. } => 20,
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
        }
    }
}

async fn serve(world: Arc<World>) -> std::io::Result<std::net::SocketAddr> {
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
            (key == name).then(|| value.to_owned())
        })
    };
    let watching = matches!(param("watch").as_deref(), Some("1" | "true"));
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    match segments.as_slice() {
        ["version"] => json_response(json!({
            "major": "1", "minor": "32", "gitVersion": "v1.32.3", "platform": "linux/amd64"
        })),
        ["api"] => json_response(json!({"kind": "APIVersions", "versions": ["v1"]})),
        ["apis"] => {
            json_response(json!({"kind": "APIGroupList", "apiVersion": "v1", "groups": []}))
        }
        ["api", "v1", "namespaces"] if watching => idle_stream(),
        ["api", "v1", "namespaces"] => json_response(namespace_table()),
        ["api", "v1", "pods"] if watching => pod_watch(world),
        ["api", "v1", "pods"] => json_response(pod_table(world, None, &param)),
        ["api", "v1", "namespaces", _, "pods"] if watching => idle_stream(),
        ["api", "v1", "namespaces", namespace, "pods"] => {
            json_response(pod_table(world, Some(namespace), &param))
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
        "object": {"kind": "PartialObjectMetadata", "metadata": {
            "name": pod.name, "namespace": pod.namespace, "uid": pod.uid,
            "resourceVersion": pod.version.to_string(),
            "creationTimestamp": "2026-09-27T00:00:00Z",
            "labels": {"app": pod.name.split('-').take(2).collect::<Vec<_>>().join("-")}
        }}
    })
}

fn pod_table(
    world: &World,
    namespace: Option<&str>,
    param: &dyn Fn(&str) -> Option<String>,
) -> Value {
    let pods = world.pods.lock().expect("pods");
    let limit: usize = param("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);
    let from: usize = param("continue").and_then(|v| v.parse().ok()).unwrap_or(0);
    let matching: Vec<&Pod> = pods
        .rows
        .iter()
        .filter(|pod| namespace.is_none_or(|namespace| pod.namespace == namespace))
        .collect();
    let to = from.saturating_add(limit).min(matching.len());
    let rows: Vec<Value> = matching[from.min(to)..to]
        .iter()
        .map(|pod| pod_row(pod))
        .collect();
    let next = if to < matching.len() {
        to.to_string()
    } else {
        String::new()
    };
    json!({
        "kind": "Table", "apiVersion": "meta.k8s.io/v1",
        "metadata": {"resourceVersion": pods.version.to_string(), "continue": next},
        "columnDefinitions": pod_columns(),
        "rows": rows
    })
}

/// The pods watch: with a burst rate, changes that many pods a second; one
/// change in ten replaces a pod, as a rollout would.
fn pod_watch(world: &Arc<World>) -> Response<Body> {
    let Scenario::Burst { rate, .. } = world.scenario else {
        return idle_stream();
    };
    let (sender, receiver) = mpsc::channel(16);
    let world = world.clone();
    tokio::spawn(async move {
        let started = Instant::now();
        let mut sent = 0u64;
        let mut tick = tokio::time::interval(TICK);
        loop {
            tick.tick().await;
            let due = (started.elapsed().as_secs_f64() * f64::from(rate)) as u64;
            let mut chunk = String::new();
            {
                let mut pods = world.pods.lock().expect("pods");
                while sent < due && chunk.len() < 1 << 20 {
                    sent += 1;
                    let ix = pods.random() as usize % pods.rows.len();
                    if sent.is_multiple_of(10) {
                        let gone = pods.rows[ix].clone();
                        let new = pods.pod();
                        pods.rows[ix] = new.clone();
                        push_event(&mut chunk, "DELETED", &gone, pods.version);
                        push_event(&mut chunk, "ADDED", &new, pods.version);
                    } else {
                        pods.version += 1;
                        let version = pods.version;
                        let pod = &mut pods.rows[ix];
                        pod.version = version;
                        if pod.status == "Running" {
                            pod.status = "CrashLoopBackOff";
                            pod.restarts += 1;
                        } else {
                            pod.status = "Running";
                        }
                        let pod = pod.clone();
                        push_event(&mut chunk, "MODIFIED", &pod, version);
                    }
                }
            }
            if !chunk.is_empty() && sender.send(Bytes::from(chunk)).await.is_err() {
                return;
            }
        }
    });
    stream_response(receiver, "application/json")
}

fn push_event(chunk: &mut String, kind: &str, pod: &Pod, version: u64) {
    let event = json!({
        "type": kind,
        "object": {
            "kind": "Table", "apiVersion": "meta.k8s.io/v1",
            "metadata": {"resourceVersion": version.to_string()},
            "columnDefinitions": null,
            "rows": [pod_row(pod)]
        }
    });
    chunk.push_str(&event.to_string());
    chunk.push('\n');
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
    json_response(json!({
        "kind": "Pod", "apiVersion": "v1",
        "metadata": {
            "name": pod.name, "namespace": pod.namespace, "uid": pod.uid,
            "resourceVersion": pod.version.to_string(),
            "creationTimestamp": "2026-09-27T00:00:00Z"
        },
        "spec": {"containers": [{"name": "app", "image": "example.invalid/app:1"}]},
        "status": {
            "phase": "Running",
            "containerStatuses": [{
                "name": "app", "ready": true, "restartCount": 0,
                "image": "example.invalid/app:1", "imageID": "",
                "state": {"running": {"startedAt": "2026-09-27T00:00:10Z"}}
            }]
        }
    }))
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
