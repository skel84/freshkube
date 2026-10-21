//! An invented Kubernetes API relays a portforward websocket to a TLS server.
use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use freshkube_core::{
    cluster_source::{ClusterAccess, KubeClientSource},
    resources::Failure,
};
use futures::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full, StreamBody, combinators::BoxBody};
use hyper::{
    Request, Response,
    body::{Frame, Incoming},
};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, handshake::derive_accept_key, protocol::Role},
};

pub(super) const CERT: &[u8] =
    include_bytes!("../../../../freshkube-delivery/testdata/results/service-cert.pem");
const KEY: &[u8] =
    include_bytes!("../../../../freshkube-delivery/testdata/results/loopback-key.pem");
type Body = BoxBody<Bytes, Infallible>;
struct Client(kube::Client);
impl KubeClientSource for Client {
    fn client(&self) -> futures::future::BoxFuture<'_, Result<kube::Client, Failure>> {
        Box::pin(std::future::ready(Ok(self.0.clone())))
    }
}

pub(super) struct Server {
    pub access: ClusterAccess,
    pub hold_start: Arc<AtomicBool>,
    pub release_start: Arc<tokio::sync::Notify>,
    pub ready: tokio::sync::watch::Sender<bool>,
    pub forwards: Arc<AtomicUsize>,
    pub results: Arc<AtomicUsize>,
    pub hold: Arc<AtomicBool>,
    pub release: Arc<tokio::sync::Notify>,
    pub status: Arc<AtomicU16>,
    pub completed: Arc<AtomicUsize>,
    pub token: Arc<AtomicBool>,
    pub truncate: Arc<AtomicBool>,
    api: tokio::task::JoinHandle<()>,
    tls: tokio::task::JoinHandle<()>,
}

impl Server {
    pub fn new(runtime: &tokio::runtime::Handle) -> Self {
        let _entered = runtime.enter();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let tls_address = listener.local_addr().unwrap();
        let results = Arc::new(AtomicUsize::new(0));
        let count = results.clone();
        let hold = Arc::new(AtomicBool::new(false));
        let release = Arc::new(tokio::sync::Notify::new());
        let status = Arc::new(AtomicU16::new(200));
        let completed = Arc::new(AtomicUsize::new(0));
        let token = Arc::new(AtomicBool::new(false));
        let truncate = Arc::new(AtomicBool::new(false));
        let (held, released, code, done, require_token, broken) = (
            hold.clone(),
            release.clone(),
            status.clone(),
            completed.clone(),
            token.clone(),
            truncate.clone(),
        );
        let tls = runtime.spawn(async move {
            use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
            let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions().unwrap().with_no_client_auth()
                .with_single_cert(vec![CertificateDer::from_pem_slice(CERT).unwrap()], PrivateKeyDer::from_pem_slice(KEY).unwrap()).unwrap();
            let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
            let listener = TcpListener::from_std(listener).unwrap();
            let mut clients = tokio::task::JoinSet::new();
            loop { tokio::select! {
                accepted = listener.accept() => {
                    let (stream, _) = accepted.unwrap(); let acceptor = acceptor.clone(); let count = count.clone();
                    let (held, released, code, done, require_token, broken) = (held.clone(), released.clone(), code.clone(), done.clone(), require_token.clone(), broken.clone());
                    clients.spawn(async move {
                        let Ok(mut stream) = acceptor.accept(stream).await else { return; };
                        assert_eq!(stream.get_ref().1.server_name(), Some("results.acme.svc"));
                        let mut head = Vec::new();
                        while !head.ends_with(b"\r\n\r\n") { let Ok(byte) = stream.read_u8().await else { return; }; head.push(byte); assert!(head.len() < 16384); }
                        let head = String::from_utf8(head).unwrap();
                        assert!(head.starts_with("GET "));
                        let auth = head.lines().any(|line| line == format!("authorization: Bearer {}", super::super::fake::TOKEN));
                        assert_eq!(auth, require_token.load(Ordering::SeqCst));
                        count.fetch_add(1, Ordering::SeqCst);
                        if held.load(Ordering::SeqCst) { released.notified().await; }
                        let body = if head.lines().next().unwrap().contains("PipelineRun") {
                            json!({"records":[{"name":"acme/results/result-a/records/run-a", "data":{
                                "type":"tekton.dev/v1.PipelineRun", "value":STANDARD.encode(super::super::fake::run().to_string())}}]})
                        } else { json!({"records":[], "logs":[]}) }.to_string();
                        let code = code.load(Ordering::SeqCst);
                        let reply = format!("HTTP/1.1 {code} Fake\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len() + usize::from(broken.load(Ordering::SeqCst)));
                        _ = stream.write_all(reply.as_bytes()).await;
                        done.fetch_add(1, Ordering::SeqCst);
                    });
                }
                _ = clients.join_next(), if !clients.is_empty() => {}
            } }
        });
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let forwards = Arc::new(AtomicUsize::new(0));
        let count = forwards.clone();
        let hold_start = Arc::new(AtomicBool::new(false));
        let release_start = Arc::new(tokio::sync::Notify::new());
        let (ready, _) = tokio::sync::watch::channel(true);
        let (start_held, start_released, pod_ready) =
            (hold_start.clone(), release_start.clone(), ready.clone());
        let api = runtime.spawn(async move {
            let listener = TcpListener::from_std(listener).unwrap();
            let mut clients = tokio::task::JoinSet::new();
            loop { tokio::select! {
                accepted = listener.accept() => {
                    let (stream, _) = accepted.unwrap(); let count = count.clone();
                    let (start_held, start_released, pod_ready) = (start_held.clone(), start_released.clone(), pod_ready.clone());
                    clients.spawn(async move {
                        let service = hyper::service::service_fn(move |mut request: Request<Incoming>| {
                            let path = request.uri().path().to_owned(); let query = request.uri().query().unwrap_or_default().to_owned();
                            let wait_start = path.ends_with("/services/results") && start_held.load(Ordering::SeqCst);
                            let start_released = start_released.clone();
                            let response = if path.ends_with("/portforward") {
                                count.fetch_add(1, Ordering::SeqCst);
                                assert!(query.split('&').any(|part| part == "ports=8080"));
                                assert!(path.ends_with("/pods/results-pod/portforward"));
                                let key = request.headers()["sec-websocket-key"].as_bytes().to_vec();
                                let upgraded = hyper::upgrade::on(&mut request);
                                tokio::spawn(async move {
                                    let Ok(upgraded) = upgraded.await else {return;};
                                    let mut socket = WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
                                    let [low, high] = 8080u16.to_le_bytes();
                                    _ = socket.send(Message::binary(vec![0, low, high])).await;
                                    _ = socket.send(Message::binary(vec![1, low, high])).await;
                                    let mut remote = TcpStream::connect(tls_address).await.unwrap();
                                    let mut buffer = [0u8; 16384];
                                    loop { tokio::select! {
                                        message = socket.next() => match message {
                                            Some(Ok(Message::Binary(data))) if data.first() == Some(&0) => { if remote.write_all(&data[1..]).await.is_err() {break;} }
                                            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                                            _ => {}
                                        },
                                        read = remote.read(&mut buffer) => match read {
                                            Ok(0) | Err(_) => break,
                                            Ok(n) => { let mut data = vec![0]; data.extend_from_slice(&buffer[..n]); if socket.send(Message::binary(data)).await.is_err() {break;} }
                                        }
                                    } }
                                    _ = socket.close(None).await;
                                });
                                Response::builder().status(101).header("upgrade", "websocket").header("connection", "Upgrade")
                                    .header("sec-websocket-accept", derive_accept_key(&key)).header("sec-websocket-protocol", "v4.channel.k8s.io")
                                    .body(Full::new(Bytes::new()).boxed()).unwrap()
                            } else {
                                assert_eq!(request.method(), http::Method::GET);
                                if query.split('&').any(|part| part == "watch=true") && path.ends_with("/pods") {
                                    let receiver = pod_ready.subscribe();
                                    let stream = futures::stream::unfold((receiver, 1u64), |(mut receiver, version)| async move {
                                        receiver.changed().await.ok()?;
                                        let ready = *receiver.borrow_and_update();
                                        let mut object = pod(ready);
                                        object["metadata"]["resourceVersion"] = (version + 1).to_string().into();
                                        let frame = Frame::data(Bytes::from(format!("{}\n", json!({"type":"MODIFIED","object":object}))));
                                        Some((Ok::<_,Infallible>(frame), (receiver, version + 1)))
                                    });
                                    Response::new(BodyExt::boxed(StreamBody::new(stream)))
                                } else if query.split('&').any(|part| part == "watch=true") {
                                    Response::new(BodyExt::boxed(StreamBody::new(futures::stream::pending::<Result<Frame<Bytes>, Infallible>>())))
                                } else {
                                    reply(api_answer(&path, *pod_ready.borrow()))
                                }
                            };
                            async move { if wait_start { start_released.notified().await; } Ok::<_, Infallible>(response) }
                        });
                        _ = hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream), service).with_upgrades().await;
                    });
                }
                _ = clients.join_next(), if !clients.is_empty() => {}
            } }
        });
        let config = kube::Config::new(format!("http://{address}").parse().unwrap());
        let client = kube::Client::try_from(config).unwrap();
        Self {
            access: ClusterAccess::Live(Arc::new(Client(client))),
            hold_start,
            release_start,
            ready,
            forwards,
            results,
            hold,
            release,
            status,
            completed,
            token,
            truncate,
            api,
            tls,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.api.abort();
        self.tls.abort();
    }
}

fn reply(value: Value) -> Response<Body> {
    Response::builder()
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(value.to_string())).boxed())
        .unwrap()
}
fn api_answer(path: &str, ready: bool) -> Value {
    let service = json!({"apiVersion":"v1", "kind":"Service", "metadata":{"name":"results","namespace":"acme","uid":"service-a","resourceVersion":"1"},
        "spec":{"selector":{"app":"results"}, "ports":[{"port":8443, "targetPort":8080}]}});
    let pod = pod(ready);
    if path.contains("/services/") {
        return service;
    }
    if path.ends_with("/pods/results-pod") {
        return pod;
    }
    if path.ends_with("/services") || path.ends_with("/pods") {
        return json!({"kind":"List", "apiVersion":"v1", "metadata":{"resourceVersion":"1"},"items":[if path.ends_with("/services") {service} else {pod}]});
    }
    super::super::fake::route(path, false)
}

fn pod(ready: bool) -> Value {
    json!({"apiVersion":"v1", "kind":"Pod", "metadata":{"name":"results-pod","namespace":"acme","uid":"pod-a","resourceVersion":"1","labels":{"app":"results"}},
        "spec":{"containers":[{"name":"results","ports":[{"containerPort":8080}]}]},
        "status":{"phase":"Running","conditions":[{"type":"Ready","status":if ready {"True"} else {"False"},"lastTransitionTime":"2026-01-01T00:00:00Z"}]}})
}
