//! Invented signed runs behind an in-process Kubernetes service and a
//! loopback Results server. Neither reads configuration or credentials.
use std::convert::Infallible;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use freshkube_core::cluster_source::{ClusterAccess, KubeClientSource};
use freshkube_core::fake_api::{Recorded, Recorder, response, unknown};
use freshkube_core::resources::Failure;
use futures::future::BoxFuture;
use kube::client::Body;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;

pub(super) const KEY: &str =
    include_str!("../../../freshkube-delivery/testdata/cosign/checkout.pub");
pub(super) const OTHER_KEY: &str =
    include_str!("../../../freshkube-delivery/testdata/cosign/other.pub");
const ENVELOPE: &str =
    include_str!("../../../freshkube-delivery/testdata/cosign/checkout.att.json");
const STATEMENT: &str =
    include_str!("../../../freshkube-delivery/testdata/cosign/checkout.statement.json");
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const DIGEST: &str = "sha256:f4f5ecadeff863529ffa5690e4f100ee235e9a2b9cd19733697b2106ebdc2aa6";
pub(super) const TOKEN: &str = "invented-results-token";

pub(super) fn run() -> Value {
    json!({"apiVersion":"tekton.dev/v1", "kind":"PipelineRun",
    "metadata":{"name":"checkout-7", "namespace":"acme", "uid":"run-a",
        "labels":{"pipelinesascode.tekton.dev/sha":SHA},
        "annotations":{
            "chains.tekton.dev/signed":"true",
            "chains.tekton.dev/signature-pipelinerun-run-a": STANDARD.encode(ENVELOPE),
            "chains.tekton.dev/payload-pipelinerun-run-a": STANDARD.encode(STATEMENT)
        }},
    "status":{"conditions":[{"type":"Succeeded","status":"True"}], "results":[
        {"name":"IMAGE_DIGEST","value":DIGEST},
        {"name":"IMAGE_URL","value":"registry.example.com/acme/checkout"},
        {"name":"ACME_EVIDENCE","value":json!({"source":{"commit":SHA},"output":{"digest":DIGEST}}).to_string()}
    ]}})
}

#[derive(Default)]
pub(super) struct Gate {
    pub hold: AtomicBool,
    pub asked: AtomicUsize,
    pub release: Notify,
}

struct Client(kube::Client);
impl KubeClientSource for Client {
    fn client(&self) -> BoxFuture<'_, Result<kube::Client, Failure>> {
        Box::pin(std::future::ready(Ok(self.0.clone())))
    }
}

/// What the test keeps beside the access: on drop it fails if the service saw
/// anything but a GET. The check is here, not in the
/// service, because a panic in the service only kills the client's worker.
pub(super) struct Reads(Recorder);

impl Drop for Reads {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        self.0
            .assert_all("an allowed GET: no Secret, no other verb", allowed);
    }
}

/// The reads the delivery chain makes of this fake, and no other: GETs of
/// discovery, Tekton and Kargo kinds and the Kargo API ConfigMap. No Secret.
fn allowed(request: &Recorded) -> bool {
    request.method == http::Method::GET
        && matches!(
            request.path.as_str(),
            "/apis"
                | "/apis/kargo.akuity.io/v1alpha1"
                | "/apis/tekton.dev/v1"
                | "/api/v1/namespaces/kargo/configmaps/kargo-api"
                | "/apis/kargo.akuity.io/v1alpha1/projects"
                | "/apis/kargo.akuity.io/v1alpha1/namespaces/checkout/freights"
                | "/apis/kargo.akuity.io/v1alpha1/namespaces/checkout/freights/freight-a"
                | "/apis/kargo.akuity.io/v1alpha1/namespaces/checkout/promotions"
                | "/apis/kargo.akuity.io/v1alpha1/namespaces/checkout/stages"
                | "/apis/kargo.akuity.io/v1alpha1/namespaces/checkout/warehouses"
                | "/apis/tekton.dev/v1/pipelineruns"
                | "/apis/tekton.dev/v1/namespaces/acme/taskruns"
                | "/apis/tekton.dev/v1/namespaces/acme-builds/pipelineruns"
                | "/apis/tekton.dev/v1/namespaces/acme-builds/taskruns"
        )
}

pub(super) fn access(live_runs: bool, gate: Arc<Gate>) -> (ClusterAccess, Reads) {
    let recorder = Recorder::default();
    let log = recorder.clone();
    let service = tower::service_fn(move |request: http::Request<Body>| {
        let recorded = log.record(&request);
        let path = recorded.path.as_str();
        let runs = path.ends_with("/pipelineruns");
        // An allowed read gets `route`'s answer (an empty list for a kind with no
        // items, as before); anything else is recorded and answered 500, and the
        // test's `Reads` fails on it.
        let answer = match allowed(&recorded) {
            true => (200, route(path, live_runs).to_string()),
            false => unknown(&recorded),
        };
        let gate = gate.clone();
        let hold = runs && gate.hold.load(Ordering::SeqCst);
        if runs {
            gate.asked.fetch_add(1, Ordering::SeqCst);
        }
        async move {
            if hold {
                gate.release.notified().await;
            }
            Ok::<_, Infallible>(response(answer))
        }
    });
    let client = Client(kube::Client::new(service, "acme"));
    (ClusterAccess::Live(Arc::new(client)), Reads(recorder))
}

pub(super) fn route(path: &str, live_runs: bool) -> Value {
    if path == "/apis" {
        return json!({"groups":[
            {"name":"tekton.dev","versions":[{"version":"v1"}]},
            {"name":"kargo.akuity.io","versions":[{"version":"v1alpha1"}]}
        ]});
    }
    if path == "/apis/tekton.dev/v1" || path == "/apis/kargo.akuity.io/v1alpha1" {
        return json!({"resources":[{"name":"pipelineruns"},{"name":"taskruns"},
            {"name":"projects"},{"name":"freights"},{"name":"stages"},{"name":"warehouses"},{"name":"promotions"}]});
    }
    let freight = json!({"metadata":{"name":"freight-a","namespace":"checkout"},
        "origin":{"kind":"Warehouse","name":"checkout"},
        "commits":[{"repoURL":"https://git.example.com/acme/checkout","id":SHA}],
        "images":[{"repoURL":"registry.example.com/acme/checkout","digest":DIGEST}]});
    if path.ends_with("/freights/freight-a") {
        return freight;
    }
    let items = if path.ends_with("/pipelineruns") && live_runs {
        vec![run()]
    } else if path.ends_with("/projects") {
        vec![json!({"metadata":{"name":"checkout"}})]
    } else if path.ends_with("/freights") {
        vec![freight]
    } else if path.ends_with("/stages") {
        vec![json!({"metadata":{"name":"dev","namespace":"checkout"},
            "spec":{"requestedFreight":[{"origin":{"kind":"Warehouse","name":"checkout"}}]},
            "status":{"freightHistory":[{"items":{"Warehouse/checkout/freight-a":{"name":"freight-a"}}}]}})]
    } else {
        vec![]
    };
    json!({"apiVersion":"v1","kind":"List","metadata":{},"items":items})
}

pub(super) struct Results {
    pub address: String,
    pub authenticated: Arc<AtomicUsize>,
    token: Arc<Mutex<String>>,
    task: tokio::task::JoinHandle<()>,
}
impl Results {
    pub fn replace_token(&self, token: &str) {
        *self.token.lock().unwrap() = token.to_owned();
    }

    pub fn new(runtime: &tokio::runtime::Handle) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let authenticated = Arc::new(AtomicUsize::new(0));
        let token = Arc::new(Mutex::new(TOKEN.to_owned()));
        let expected = token.clone();
        let count = authenticated.clone();
        let task = runtime.spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let mut clients = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut stream, _) = accepted.unwrap();
                        let count = count.clone();
                        let expected = expected.clone();
                        clients.spawn(async move {
                            let mut head = Vec::new();
                            while !head.ends_with(b"\r\n\r\n") {
                                let Ok(byte) = stream.read_u8().await else {return};
                                head.push(byte);
                                assert!(head.len() < 16 * 1024);
                            }
                            let head = String::from_utf8(head).unwrap();
                            assert!(head.starts_with("GET "));
                            // Never format a header or a token, even on failure.
                            let authorized = head.lines().any(|line| line == format!("authorization: Bearer {}", expected.lock().unwrap()));
                            if authorized { count.fetch_add(1, Ordering::SeqCst); }
                            let body = if head.lines().next().unwrap().contains("PipelineRun") {
                                json!({"records":[{"name":"acme/results/result-a/records/run-a", "data":{
                                    "type":"tekton.dev/v1.PipelineRun", "value":STANDARD.encode(run().to_string())}}]})
                            } else {json!({"records":[],"logs":[]})}.to_string();
                            let status = if authorized {200} else {403};
                            let reply = format!("HTTP/1.1 {status} Fake\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                            _ = stream.write_all(reply.as_bytes()).await;
                        });
                    }
                    _ = clients.join_next(), if !clients.is_empty() => {}
                }
            }
        });
        Self {
            address,
            authenticated,
            token,
            task,
        }
    }
}
impl Drop for Results {
    fn drop(&mut self) {
        self.task.abort();
    }
}
