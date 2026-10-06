//! Forwards against a fake API server on the loopback that speaks HTTP/1,
//! lists and watches, and upgrades to the port-forward websocket as the
//! real one does. Its pods answer each message with their name, so a test
//! sees which pod a connection reached.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use futures::{SinkExt, StreamExt};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;

use super::*;

const PODS: &str = "/api/v1/namespaces/shop/pods";
const SERVICES: &str = "/api/v1/namespaces/shop/services";
const DEPLOYMENTS: &str = "/apis/apps/v1/namespaces/shop/deployments";
/// How long a test waits for something that should happen at once.
const SOON: Duration = Duration::from_secs(5);

// These fixtures share the automatic local port candidates. Keep one case
// from taking a port another case has just released before its assertion.
// Concurrent connections and forwards within each fixture still run together.
static PORT_CASE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// How a pod answers a forward.
#[derive(Clone)]
enum Answer {
    /// Each message comes back as `<pod>:<message>`.
    Echo,
    /// The upgrade is refused with this status.
    Refuse(u16),
    /// The port's error channel says this, as when nothing listens.
    PortError(&'static str),
}

#[derive(Default)]
struct State {
    /// Objects by collection path.
    collections: HashMap<String, Vec<Value>>,
    version: u64,
    /// Open watches: their collection, the name they follow, if one, and
    /// where their events go.
    watches: Vec<(String, Option<String>, UnboundedSender<Bytes>)>,
    /// Every event, by version, collection and name, so a watch replays
    /// what came after the version it starts from.
    events: Vec<(u64, String, String, Bytes)>,
    answers: HashMap<String, Answer>,
    /// Every forward request, as `<pod>?<query>`.
    forwards: Vec<String>,
    /// Websockets open now.
    open: usize,
    /// Websockets the client closed with a Close frame.
    closed_cleanly: usize,
}

#[derive(Clone)]
struct Fake {
    _case: Arc<tokio::sync::MutexGuard<'static, ()>>,
    state: Arc<Mutex<State>>,
    watches: PodWatches,
}

impl Fake {
    async fn start() -> Self {
        let case = Arc::new(PORT_CASE.lock().await);
        let state = Arc::new(Mutex::new(State::default()));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let served = state.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let state = served.clone();
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |request| {
                        let state = state.clone();
                        async move { Ok::<_, Infallible>(handle(request, &state)) }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .with_upgrades()
                        .await;
                });
            }
        });
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut config = kube::Config::new(format!("http://{address}").parse().unwrap());
        // The loopback HTTP fixture needs no system trust store.
        config.root_cert = Some(Vec::new());
        let client = Client::try_from(config).unwrap();
        Self {
            _case: case,
            state,
            watches: PodWatches::new(client),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    /// Adds or changes an object and tells its watches.
    fn apply(&self, collection: &str, mut object: Value) {
        let mut state = self.state();
        state.version += 1;
        object["metadata"]["resourceVersion"] = json!(state.version.to_string());
        let name = object["metadata"]["name"].as_str().unwrap().to_owned();
        let objects = state.collections.entry(collection.to_owned()).or_default();
        let kind = match objects
            .iter_mut()
            .find(|known| known["metadata"]["name"] == name)
        {
            Some(known) => {
                *known = object.clone();
                "MODIFIED"
            }
            None => {
                objects.push(object.clone());
                "ADDED"
            }
        };
        state.tell(collection, &name, kind, object);
    }

    fn delete(&self, collection: &str, name: &str) {
        let mut state = self.state();
        state.version += 1;
        let objects = state.collections.entry(collection.to_owned()).or_default();
        let index = objects
            .iter()
            .position(|known| known["metadata"]["name"] == name)
            .unwrap();
        let mut object = objects.remove(index);
        object["metadata"]["resourceVersion"] = json!(state.version.to_string());
        state.tell(collection, name, "DELETED", object);
    }

    fn answer(&self, pod: &str, answer: Answer) {
        self.state().answers.insert(pod.to_owned(), answer);
    }

    fn open(&self) -> usize {
        self.state().open
    }

    async fn start_forward(&self, target: ForwardTarget, port: u16) -> Forward {
        start_forward(&self.watches, request(target, port))
            .await
            .unwrap()
    }
}

impl State {
    fn tell(&mut self, collection: &str, name: &str, kind: &str, object: Value) {
        let mut line = json!({"type": kind, "object": object}).to_string();
        line.push('\n');
        let line = Bytes::from(line);
        self.events.push((
            self.version,
            collection.to_owned(),
            name.to_owned(),
            line.clone(),
        ));
        self.watches.retain(|(path, follows, sink)| {
            if path != collection || follows.as_deref().is_some_and(|follows| follows != name) {
                return !sink.is_closed();
            }
            sink.unbounded_send(line.clone()).is_ok()
        });
    }
}

type Body = BoxBody<Bytes, Infallible>;

fn handle(request: Request<Incoming>, state: &Arc<Mutex<State>>) -> Response<Body> {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    if let Some(pod) = path
        .strip_suffix("/portforward")
        .and_then(|pod| pod.strip_prefix(&format!("{PODS}/")))
    {
        return upgrade(request, pod.to_owned(), &query, state);
    }
    let follows = parameter(&query, "fieldSelector")
        .and_then(|selector| selector.strip_prefix("metadata.name=").map(str::to_owned));
    let mut state = state.lock().unwrap();
    if parameter(&query, "watch").as_deref() == Some("true") {
        let (sink, events) = unbounded();
        let since: u64 = parameter(&query, "resourceVersion")
            .and_then(|version| version.parse().ok())
            .unwrap_or_default();
        for (version, collection, name, line) in &state.events {
            if *version > since
                && *collection == path
                && follows.as_ref().is_none_or(|follows| follows == name)
            {
                sink.unbounded_send(line.clone()).unwrap();
            }
        }
        state.watches.push((path, follows, sink));
        let body = StreamBody::new(events.map(|line| Ok(Frame::data(line))));
        return Response::new(BodyExt::boxed(body));
    }
    if let Some(objects) = state.collections.get(&path) {
        let items: Vec<&Value> = objects
            .iter()
            .filter(|object| {
                follows
                    .as_ref()
                    .is_none_or(|name| object["metadata"]["name"] == **name)
            })
            .collect();
        let list = json!({"kind": "List", "apiVersion": "v1",
            "metadata": {"resourceVersion": state.version.to_string()}, "items": items});
        return reply(200, list);
    }
    let (collection, name) = path.rsplit_once('/').unwrap();
    let found = state.collections.get(collection).and_then(|objects| {
        objects
            .iter()
            .find(|object| object["metadata"]["name"] == name)
    });
    match found {
        Some(object) => reply(200, object.clone()),
        None => reply(
            404,
            json!({"kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Failure",
                "message": format!("\"{name}\" not found"), "reason": "NotFound", "code": 404}),
        ),
    }
}

fn reply(status: u16, body: Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(BodyExt::boxed(Full::new(Bytes::from(body.to_string()))))
        .unwrap()
}

fn parameter(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.replace("%3D", "=").replace("%2C", ","))
    })
}

fn upgrade(
    mut request: Request<Incoming>,
    pod: String,
    query: &str,
    state: &Arc<Mutex<State>>,
) -> Response<Body> {
    let answer = {
        let mut state = state.lock().unwrap();
        state
            .forwards
            .push(format!("{pod}?{}", query.trim_start_matches('&')));
        state.answers.get(&pod).cloned().unwrap_or(Answer::Echo)
    };
    if let Answer::Refuse(status) = answer {
        return reply(status, json!({}));
    }
    let port: u16 = parameter(query, "ports").unwrap().parse().unwrap();
    let key = request.headers()["sec-websocket-key"].as_bytes().to_owned();
    let upgraded = hyper::upgrade::on(&mut request);
    let state = state.clone();
    tokio::spawn(async move {
        let io = TokioIo::new(upgraded.await.unwrap());
        let socket = WebSocketStream::from_raw_socket(io, Role::Server, None).await;
        serve_port(socket, pod, port, answer, state).await;
    });
    Response::builder()
        .status(101)
        .header("upgrade", "websocket")
        .header("connection", "Upgrade")
        .header("sec-websocket-accept", derive_accept_key(&key))
        .header("sec-websocket-protocol", "v4.channel.k8s.io")
        .body(BodyExt::boxed(Full::new(Bytes::new())))
        .unwrap()
}

/// One port over the channel protocol: each channel's first frame names
/// the port, data goes on channel 0 and errors on channel 1.
async fn serve_port(
    mut socket: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
    pod: String,
    port: u16,
    answer: Answer,
    state: Arc<Mutex<State>>,
) {
    state.lock().unwrap().open += 1;
    let [low, high] = port.to_le_bytes();
    let _ = socket.send(Message::binary(vec![0, low, high])).await;
    let _ = socket.send(Message::binary(vec![1, low, high])).await;
    if let Answer::PortError(message) = answer {
        let mut frame = vec![1];
        frame.extend_from_slice(message.as_bytes());
        let _ = socket.send(Message::binary(frame)).await;
    }
    while let Some(Ok(message)) = socket.next().await {
        match message {
            Message::Binary(data) if data.first() == Some(&0) => {
                let mut frame = vec![0];
                frame.extend_from_slice(format!("{pod}:").as_bytes());
                frame.extend_from_slice(&data[1..]);
                let _ = socket.send(Message::binary(frame)).await;
            }
            Message::Close(_) => state.lock().unwrap().closed_cleanly += 1,
            _ => {}
        }
    }
    state.lock().unwrap().open -= 1;
}

fn request(target: ForwardTarget, port: u16) -> ForwardRequest {
    ForwardRequest {
        namespace: "shop".into(),
        target,
        port,
        local_port: None,
    }
}

fn pod_target(uid: &str) -> ForwardTarget {
    ForwardTarget::Pod {
        name: "web-0".into(),
        uid: uid.into(),
    }
}

fn service_target() -> ForwardTarget {
    ForwardTarget::Service { name: "web".into() }
}

/// A pod of app `web` with a port `http` on 8080, Ready since `ready` when
/// given.
fn pod(name: &str, uid: &str, ready: Option<&str>) -> Value {
    let condition = match ready {
        Some(since) => json!({"type": "Ready", "status": "True", "lastTransitionTime": since}),
        None => json!({"type": "Ready", "status": "False"}),
    };
    json!({"apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "namespace": "shop", "uid": uid, "labels": {"app": "web"}},
        "spec": {"containers": [{"name": "app",
            "ports": [{"name": "http", "containerPort": 8080}]}]},
        "status": {"phase": "Running", "conditions": [condition]}})
}

fn service(ports: Value) -> Value {
    json!({"apiVersion": "v1", "kind": "Service",
        "metadata": {"name": "web", "namespace": "shop", "uid": "s-1"},
        "spec": {"selector": {"app": "web"}, "ports": ports}})
}

fn web_service() -> Value {
    service(json!([{"name": "http", "port": 80, "targetPort": "http"}]))
}

/// Waits until the status passes `check`.
async fn status_until(
    status: &mut watch::Receiver<ForwardStatus>,
    check: impl Fn(&ForwardStatus) -> bool,
) -> ForwardStatus {
    let status = tokio::time::timeout(SOON, status.wait_for(|status| check(status)))
        .await
        .expect("the status changes in time")
        .expect("the forward keeps its status");
    status.clone()
}

async fn eventually(what: &str, check: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + SOON;
    while !check() {
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn connect(forward: &Forward) -> TcpStream {
    TcpStream::connect(("127.0.0.1", forward.local_port))
        .await
        .unwrap()
}

/// Sends `message` and returns the answer.
async fn exchange(stream: &mut TcpStream, message: &str) -> String {
    stream.write_all(message.as_bytes()).await.unwrap();
    let mut answer = vec![0; 256];
    let read = tokio::time::timeout(SOON, stream.read(&mut answer))
        .await
        .expect("an answer in time")
        .unwrap();
    String::from_utf8(answer[..read].to_vec()).unwrap()
}

/// Reads until the connection closes; true when it closed in time.
async fn closes(stream: &mut TcpStream) -> bool {
    let mut rest = Vec::new();
    matches!(
        tokio::time::timeout(SOON, stream.read_to_end(&mut rest)).await,
        Ok(Ok(_)) | Ok(Err(_))
    )
}

fn port_is_free(port: u16) -> bool {
    port::bind_loopback(port).is_ok()
}

#[tokio::test]
async fn a_pod_forward_relays_bytes_both_ways_on_the_automatic_port() {
    let fake = Fake::start().await;
    fake.apply(PODS, pod("web-0", "u-1", Some("2026-10-02T09:00:00Z")));
    let mut forward = fake.start_forward(pod_target("u-1"), 8080).await;
    assert!(candidates(8080).contains(&forward.local_port) || forward.local_port > 0);
    assert_eq!(
        forward.status.borrow().state,
        ForwardState::Listening {
            pod: "web-0".into(),
            port: 8080
        }
    );

    let mut first = connect(&forward).await;
    let mut second = connect(&forward).await;
    assert_eq!(exchange(&mut first, "ping").await, "web-0:ping");
    assert_eq!(exchange(&mut second, "pong").await, "web-0:pong");
    status_until(&mut forward.status, |status| status.connections == 2).await;
    assert_eq!(fake.open(), 2, "each connection has its own websocket");
    assert_eq!(
        fake.state().forwards,
        vec!["web-0?ports=8080", "web-0?ports=8080"]
    );

    drop(first);
    status_until(&mut forward.status, |status| status.connections == 1).await;
    eventually("the closed connection's websocket closes", || {
        fake.open() == 1
    })
    .await;
    assert_eq!(fake.state().closed_cleanly, 1, "with a Close frame");
    assert_eq!(exchange(&mut second, "again").await, "web-0:again");
}

#[tokio::test]
#[cfg_attr(target_os = "linux", ignore = "Linux port reuse, #212")]
async fn stopping_frees_the_port_and_closes_every_websocket() {
    let fake = Fake::start().await;
    fake.apply(PODS, pod("web-0", "u-1", Some("2026-10-02T09:00:00Z")));
    let forward = fake.start_forward(pod_target("u-1"), 8080).await;
    let port = forward.local_port;
    let mut stream = connect(&forward).await;
    assert_eq!(exchange(&mut stream, "ping").await, "web-0:ping");
    assert!(!port_is_free(port));

    let Forward {
        mut status, guard, ..
    } = forward;
    guard.stop().await.unwrap();
    assert!(port_is_free(port), "free once the stop completes");
    assert!(closes(&mut stream).await, "the local connection closes");
    eventually("the websocket closes", || fake.open() == 0).await;
    assert_eq!(fake.state().closed_cleanly, 1);
    let status = status_until(&mut status, |status| status.connections == 0).await;
    assert_eq!(status.state, ForwardState::Ended(ForwardEnd::Stopped));
}

#[tokio::test]
#[cfg_attr(target_os = "linux", ignore = "Linux port reuse, #212")]
async fn dropping_the_guard_stops_the_forward() {
    let fake = Fake::start().await;
    fake.apply(PODS, pod("web-0", "u-1", Some("2026-10-02T09:00:00Z")));
    let forward = fake.start_forward(pod_target("u-1"), 8080).await;
    let port = forward.local_port;
    let mut stream = connect(&forward).await;
    assert_eq!(exchange(&mut stream, "ping").await, "web-0:ping");
    drop(forward);
    eventually("the port is free", || port_is_free(port)).await;
    assert!(closes(&mut stream).await);
    eventually("the websocket closes", || fake.open() == 0).await;
}

#[tokio::test]
async fn a_typed_port_is_used_exactly_or_refused_when_taken() {
    let fake = Fake::start().await;
    fake.apply(PODS, pod("web-0", "u-1", Some("2026-10-02T09:00:00Z")));
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let mut typed = request(pod_target("u-1"), 8080);
    typed.local_port = Some(port);
    let Err(failure) = start_forward(&fake.watches, typed.clone()).await else {
        panic!("the port is taken");
    };
    assert_eq!(failure.kind, ForwardFailureKind::PortInUse);
    assert_eq!(
        failure.to_string(),
        format!("In use · Port {port} is in use")
    );
    assert!(fake.state().forwards.is_empty());

    drop(taken);
    let forward = start_forward(&fake.watches, typed).await.unwrap();
    assert_eq!(forward.local_port, port);
}

#[tokio::test]
async fn a_pod_that_changed_since_it_was_chosen_isnt_forwarded() {
    let fake = Fake::start().await;
    fake.apply(PODS, pod("web-0", "u-2", Some("2026-10-02T09:00:00Z")));
    let replaced = start_forward(&fake.watches, request(pod_target("u-1"), 8080)).await;
    assert_eq!(replaced.err().unwrap().message, "The pod was replaced");

    let gone = start_forward(
        &fake.watches,
        request(
            ForwardTarget::Pod {
                name: "web-9".into(),
                uid: "u-9".into(),
            },
            8080,
        ),
    )
    .await;
    let failure = gone.err().unwrap();
    assert_eq!(
        failure.kind,
        ForwardFailureKind::Request(FailureKind::NotFound)
    );
    assert_eq!(failure.message, "The pod was deleted");

    let mut pending = pod("web-0", "u-2", None);
    pending["status"]["phase"] = json!("Pending");
    fake.apply(PODS, pending);
    let not_running = start_forward(&fake.watches, request(pod_target("u-2"), 8080)).await;
    let failure = not_running.err().unwrap();
    assert_eq!(failure.kind, ForwardFailureKind::NotRunning);
    assert_eq!(failure.message, "The pod is Pending");
}

#[tokio::test]
async fn a_port_nothing_listens_on_is_reported_and_the_forward_listens_on() {
    let fake = Fake::start().await;
    fake.apply(PODS, pod("web-0", "u-1", Some("2026-10-02T09:00:00Z")));
    fake.answer(
        "web-0",
        Answer::PortError(
            "error forwarding port 8080 to pod 1f2e, uid : failed to execute portforward in \
             network namespace \"/var/run/netns/cni-1\": failed to connect to localhost:8080 \
             inside namespace \"1f2e\", IPv4: dial tcp4 127.0.0.1:8080: connect: connection \
             refused IPv6 dial tcp6 [::1]:8080: connect: connection refused",
        ),
    );
    let mut forward = fake.start_forward(pod_target("u-1"), 8080).await;
    let mut stream = connect(&forward).await;
    assert!(closes(&mut stream).await, "the connection closes");
    let status = status_until(&mut forward.status, |status| {
        status.last_error.is_some() && status.connections == 0
    })
    .await;
    assert_eq!(
        status.last_error.as_deref(),
        Some("Connection refused in the pod on port 8080")
    );
    assert!(matches!(status.state, ForwardState::Listening { .. }));
    eventually("the websocket closes", || fake.open() == 0).await;
}

#[tokio::test]
#[cfg_attr(target_os = "linux", ignore = "Linux port reuse, #212")]
async fn being_forbidden_ends_the_forward_and_frees_its_port() {
    let fake = Fake::start().await;
    fake.apply(PODS, pod("web-0", "u-1", Some("2026-10-02T09:00:00Z")));
    fake.answer("web-0", Answer::Refuse(403));
    let mut forward = fake.start_forward(pod_target("u-1"), 8080).await;
    let port = forward.local_port;
    let mut stream = connect(&forward).await;
    let status = status_until(&mut forward.status, |status| {
        matches!(status.state, ForwardState::Ended(_))
    })
    .await;
    let ForwardState::Ended(ForwardEnd::Failed(failure)) = status.state else {
        panic!("{status:?}");
    };
    assert_eq!(
        failure.kind,
        ForwardFailureKind::Request(FailureKind::Forbidden)
    );
    assert!(closes(&mut stream).await);
    assert!(port_is_free(port));
}

#[tokio::test]
async fn another_refusal_is_the_connections_error_only() {
    let fake = Fake::start().await;
    fake.apply(PODS, pod("web-0", "u-1", Some("2026-10-02T09:00:00Z")));
    fake.answer("web-0", Answer::Refuse(500));
    let mut forward = fake.start_forward(pod_target("u-1"), 8080).await;
    let mut stream = connect(&forward).await;
    assert!(closes(&mut stream).await);
    let status = status_until(&mut forward.status, |status| status.last_error.is_some()).await;
    assert_eq!(
        status.last_error.as_deref(),
        Some("The API server refused the forward (500 Internal Server Error)")
    );
    assert!(matches!(status.state, ForwardState::Listening { .. }));
}

#[tokio::test]
#[cfg_attr(target_os = "linux", ignore = "Linux port reuse, #212")]
async fn a_pod_forward_ends_when_its_pod_is_deleted_or_replaced() {
    let fake = Fake::start().await;
    fake.apply(PODS, pod("web-0", "u-1", Some("2026-10-02T09:00:00Z")));
    let mut forward = fake.start_forward(pod_target("u-1"), 8080).await;
    let port = forward.local_port;
    let mut stream = connect(&forward).await;
    assert_eq!(exchange(&mut stream, "ping").await, "web-0:ping");
    fake.delete(PODS, "web-0");
    let status = status_until(&mut forward.status, |status| {
        matches!(status.state, ForwardState::Ended(_))
    })
    .await;
    assert_eq!(
        status.state,
        ForwardState::Ended(ForwardEnd::PodGone("The pod was deleted".into()))
    );
    assert!(closes(&mut stream).await);
    assert!(port_is_free(port));
    eventually("the websocket closes", || fake.open() == 0).await;

    fake.apply(PODS, pod("web-0", "u-2", Some("2026-10-02T09:00:00Z")));
    let mut forward = fake.start_forward(pod_target("u-2"), 8080).await;
    // A new incarnation seen without its predecessor's deletion, as after
    // the watch lists again.
    fake.apply(PODS, pod("web-0", "u-3", Some("2026-10-02T09:00:00Z")));
    let status = status_until(&mut forward.status, |status| {
        matches!(status.state, ForwardState::Ended(_))
    })
    .await;
    assert_eq!(
        status.state,
        ForwardState::Ended(ForwardEnd::PodGone("The pod was replaced".into()))
    );
}

#[tokio::test]
async fn a_pod_forward_ends_when_its_pod_finishes() {
    let fake = Fake::start().await;
    fake.apply(PODS, pod("web-0", "u-1", Some("2026-10-02T09:00:00Z")));
    let mut forward = fake.start_forward(pod_target("u-1"), 8080).await;
    let mut done = pod("web-0", "u-1", None);
    done["status"]["phase"] = json!("Succeeded");
    fake.apply(PODS, done);
    let status = status_until(&mut forward.status, |status| {
        matches!(status.state, ForwardState::Ended(_))
    })
    .await;
    assert_eq!(
        status.state,
        ForwardState::Ended(ForwardEnd::PodGone("The pod stopped".into()))
    );
}

#[tokio::test]
async fn a_service_forwards_to_its_longest_ready_pod_and_moves_when_it_goes() {
    let fake = Fake::start().await;
    fake.apply(SERVICES, web_service());
    fake.apply(PODS, pod("web-a", "u-a", Some("2026-10-02T10:00:00Z")));
    fake.apply(PODS, pod("web-b", "u-b", Some("2026-10-02T09:00:00Z")));
    fake.apply(PODS, pod("web-c", "u-c", None));
    let mut forward = fake.start_forward(service_target(), 80).await;
    assert!(candidates(80).contains(&forward.local_port) || forward.local_port > 0);
    assert_eq!(
        forward.status.borrow().state,
        ForwardState::Listening {
            pod: "web-b".into(),
            port: 8080
        },
        "the named target port, on the pod Ready the longest"
    );
    let mut old = connect(&forward).await;
    assert_eq!(exchange(&mut old, "ping").await, "web-b:ping");
    assert_eq!(fake.state().forwards, vec!["web-b?ports=8080"]);

    fake.apply(PODS, pod("web-b", "u-b", None));
    let status = status_until(&mut forward.status, |status| {
        status.state
            == ForwardState::Listening {
                pod: "web-a".into(),
                port: 8080,
            }
    })
    .await;
    assert!(status.last_error.is_none());
    assert!(closes(&mut old).await, "a connection to the old pod closes");
    let mut new = connect(&forward).await;
    assert_eq!(exchange(&mut new, "ping").await, "web-a:ping");

    fake.apply(PODS, pod("web-b", "u-b", Some("2026-10-02T11:00:00Z")));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        exchange(&mut new, "still").await,
        "web-a:still",
        "a usable pod is kept while others come back"
    );
}

#[tokio::test]
async fn without_a_ready_pod_a_forward_waits_and_says_so() {
    let fake = Fake::start().await;
    fake.apply(SERVICES, web_service());
    fake.apply(PODS, pod("web-a", "u-a", None));
    let starting = {
        let watches = fake.watches.clone();
        tokio::spawn(async move { start_forward(&watches, request(service_target(), 80)).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!starting.is_finished(), "the start waits for a Ready pod");
    fake.apply(PODS, pod("web-a", "u-a", Some("2026-10-02T09:00:00Z")));
    let mut forward = tokio::time::timeout(SOON, starting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    fake.apply(PODS, pod("web-a", "u-a", None));
    status_until(&mut forward.status, |status| {
        status.state == ForwardState::NoReadyPod
    })
    .await;
    let mut stream = connect(&forward).await;
    status_until(&mut forward.status, |status| status.connections == 1).await;
    stream.write_all(b"queued").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        fake.state().forwards.is_empty(),
        "nothing to forward to yet"
    );
    fake.apply(PODS, pod("web-a", "u-a", Some("2026-10-02T10:00:00Z")));
    let mut answer = vec![0; 64];
    let read = tokio::time::timeout(SOON, stream.read(&mut answer))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        &answer[..read],
        b"web-a:queued",
        "the waiting connection goes on"
    );
}

#[tokio::test]
async fn a_service_change_moves_the_forward_and_its_deletion_ends_it() {
    let fake = Fake::start().await;
    fake.apply(SERVICES, web_service());
    fake.apply(PODS, pod("web-a", "u-a", Some("2026-10-02T09:00:00Z")));
    let mut forward = fake.start_forward(service_target(), 80).await;
    let port = forward.local_port;

    fake.apply(
        SERVICES,
        service(json!([{"name": "http", "port": 80, "targetPort": 9090}])),
    );
    status_until(&mut forward.status, |status| {
        status.state
            == ForwardState::Listening {
                pod: "web-a".into(),
                port: 9090,
            }
    })
    .await;

    fake.apply(SERVICES, service(json!([{"name": "other", "port": 81}])));
    let status = status_until(&mut forward.status, |status| {
        status.state == ForwardState::NoReadyPod
    })
    .await;
    assert_eq!(
        status.last_error.as_deref(),
        Some("The Service has no port 80")
    );

    fake.delete(SERVICES, "web");
    let status = status_until(&mut forward.status, |status| {
        matches!(status.state, ForwardState::Ended(_))
    })
    .await;
    assert_eq!(
        status.state,
        ForwardState::Ended(ForwardEnd::TargetDeleted("The Service was deleted".into()))
    );
    assert!(port_is_free(port));
}

#[tokio::test]
async fn a_workload_forwards_to_its_pods_until_it_is_deleted() {
    let fake = Fake::start().await;
    fake.apply(
        DEPLOYMENTS,
        json!({"apiVersion": "apps/v1", "kind": "Deployment",
            "metadata": {"name": "web", "namespace": "shop", "uid": "d-1"},
            "spec": {"selector": {"matchLabels": {"app": "web"}}}}),
    );
    fake.apply(PODS, pod("web-a", "u-a", Some("2026-10-02T09:00:00Z")));
    let target = ForwardTarget::Workload {
        kind: WorkloadKind::Deployment,
        name: "web".into(),
    };
    let mut forward = fake.start_forward(target, 8080).await;
    let mut stream = connect(&forward).await;
    assert_eq!(exchange(&mut stream, "ping").await, "web-a:ping");

    fake.delete(DEPLOYMENTS, "web");
    let status = status_until(&mut forward.status, |status| {
        matches!(status.state, ForwardState::Ended(_))
    })
    .await;
    assert_eq!(
        status.state,
        ForwardState::Ended(ForwardEnd::TargetDeleted(
            "The Deployment was deleted".into()
        ))
    );
    assert!(closes(&mut stream).await);
}

#[tokio::test]
async fn forwards_of_the_same_pods_share_one_watch() {
    let fake = Fake::start().await;
    fake.apply(SERVICES, web_service());
    fake.apply(PODS, pod("web-a", "u-a", Some("2026-10-02T09:00:00Z")));
    let first = fake.start_forward(service_target(), 80).await;
    let second = fake.start_forward(service_target(), 80).await;
    assert_ne!(first.local_port, second.local_port);
    assert_eq!(fake.watches.running(), 1);
    drop(first);
    assert_eq!(fake.watches.running(), 1);
    drop(second);
    eventually("the watch stops with its last forward", || {
        fake.watches.running() == 0
    })
    .await;
}

#[tokio::test]
async fn a_missing_target_or_port_fails_the_start() {
    let fake = Fake::start().await;
    let missing = start_forward(&fake.watches, request(service_target(), 80)).await;
    assert_eq!(missing.err().unwrap().message, "The Service was not found");

    fake.apply(SERVICES, web_service());
    let wrong_port = start_forward(&fake.watches, request(service_target(), 81)).await;
    let failure = wrong_port.err().unwrap();
    assert_eq!(
        failure.kind,
        ForwardFailureKind::Request(FailureKind::NotFound)
    );
    assert_eq!(failure.message, "The Service has no port 81");
    assert!(fake.state().forwards.is_empty());
}

fn typed_service(value: Value) -> Service {
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_service_route_needs_a_selector_and_a_tcp_port() {
    let (selector, remote) = service_route(&typed_service(web_service()), 80).unwrap();
    assert_eq!(selector, "app=web");
    assert_eq!(remote, RemotePort::Named("http".into()));

    let mut headless = web_service();
    headless["spec"].as_object_mut().unwrap().remove("selector");
    let failure = service_route(&typed_service(headless), 80).unwrap_err();
    assert_eq!(failure.kind, ForwardFailureKind::NoPods);
    assert!(failure.is_permanent());

    let udp = service(json!([{"port": 53, "protocol": "UDP"}]));
    let failure = service_route(&typed_service(udp), 53).unwrap_err();
    assert_eq!(failure.kind, ForwardFailureKind::NotTcp);
}

#[test]
fn a_target_comes_from_a_forwardable_kind() {
    let pod_kind = crate::resources::builtin("pods").unwrap();
    assert_eq!(
        ForwardTarget::of(&pod_kind, "web-0", "u-1"),
        Some(pod_target("u-1"))
    );
    let services = crate::resources::builtin("services").unwrap();
    assert_eq!(
        ForwardTarget::of(&services, "web", "s-1"),
        Some(service_target())
    );
    let statefulsets = crate::resources::builtin("statefulsets.apps").unwrap();
    assert_eq!(
        ForwardTarget::of(&statefulsets, "db", "x").unwrap().label(),
        "StatefulSet"
    );
    let configmaps = crate::resources::builtin("configmaps").unwrap();
    assert_eq!(ForwardTarget::of(&configmaps, "settings", "c"), None);
}

#[test]
fn the_usual_refusal_reads_shortly_and_others_keep_their_first_line() {
    assert_eq!(
        relay::port_error(
            "dial tcp4 127.0.0.1:5432: connect: Connection Refused",
            5432
        ),
        "Connection refused in the pod on port 5432"
    );
    assert_eq!(
        relay::port_error("  timed out\nmore detail", 5432),
        "timed out"
    );
}
