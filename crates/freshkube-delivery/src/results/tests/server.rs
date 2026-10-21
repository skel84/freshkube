//! A loopback-only server with canned replies; no kubeconfig or key-store I/O.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};
use tokio_rustls::TlsAcceptor;

pub(super) const CERT: &[u8] = include_bytes!("../../../testdata/results/loopback-cert.pem");
pub(super) const SERVICE_CERT: &[u8] = include_bytes!("../../../testdata/results/service-cert.pem");
const KEY: &[u8] = include_bytes!("../../../testdata/results/loopback-key.pem");

pub(super) struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub headers: String,
    pub chunked: bool,
    pub hold: bool,
    matching: Option<String>,
}

impl Reply {
    pub fn json(value: serde_json::Value) -> Self {
        Self::raw(200, serde_json::to_vec(&value).unwrap())
    }
    pub fn raw(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: body.into(),
            headers: String::new(),
            chunked: false,
            hold: false,
            matching: None,
        }
    }

    pub fn matching(mut self, request: &str) -> Self {
        self.matching = Some(request.into());
        self
    }
}

pub(super) struct Server {
    pub address: String,
    pub requests: Arc<Mutex<Vec<String>>>,
    pub server_names: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl Server {
    pub fn new(tls: bool, replies: Vec<Reply>) -> Self {
        Self::with_certificate(tls.then_some(CERT), replies)
    }

    pub fn with_certificate(certificate: Option<&[u8]>, replies: Vec<Reply>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let server_names = Arc::new(Mutex::new(Vec::new()));
        let queued = Arc::new(Mutex::new(VecDeque::from(replies)));
        let acceptor = certificate.map(|certificate| {
            use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
            let config = rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from_pem_slice(certificate).unwrap()],
                PrivateKeyDer::from_pem_slice(KEY).unwrap(),
            )
            .unwrap();
            TlsAcceptor::from(Arc::new(config))
        });
        let task = tokio::spawn({
            let requests = requests.clone();
            let server_names = server_names.clone();
            let replies = queued.clone();
            async move {
                let listener = TcpListener::from_std(listener).unwrap();
                let mut connections = JoinSet::new();
                loop {
                    tokio::select! {
                        accepted = listener.accept() => {
                            let (stream, _) = accepted.unwrap();
                            let requests = requests.clone();
                            let server_names = server_names.clone();
                            let replies = replies.clone();
                            let acceptor = acceptor.clone();
                            connections.spawn(async move {
                                if let Some(acceptor) = acceptor {
                                    if let Ok(stream) = acceptor.accept(stream).await {
                                        if let Some(name) = stream.get_ref().1.server_name() {
                                            server_names.lock().unwrap().push(name.to_owned());
                                        }
                                        serve(stream, requests, replies).await;
                                    }
                                } else {
                                    serve(stream, requests, replies).await;
                                }
                            });
                        },
                        _ = connections.join_next(), if !connections.is_empty() => {},
                    }
                }
            }
        });
        Self {
            address: format!(
                "{}://127.0.0.1:{port}",
                if certificate.is_some() {
                    "https"
                } else {
                    "http"
                }
            ),
            requests,
            server_names,
            task,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    requests: Arc<Mutex<Vec<String>>>,
    replies: Arc<Mutex<VecDeque<Reply>>>,
) {
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        let Ok(byte) = stream.read_u8().await else {
            return;
        };
        request.push(byte);
        assert!(
            request.len() <= 16 * 1024,
            "unexpectedly long fake-server request"
        );
    }
    let request = String::from_utf8(request).unwrap();
    let reply = {
        let mut replies = replies.lock().unwrap();
        replies
            .iter()
            .position(|reply| {
                reply
                    .matching
                    .as_ref()
                    .is_none_or(|part| request.contains(part))
            })
            .and_then(|index| replies.remove(index))
            .unwrap_or_else(|| Reply::raw(500, Vec::new()))
    };
    requests.lock().unwrap().push(request);
    let framing = if reply.chunked {
        "Transfer-Encoding: chunked\r\n".to_owned()
    } else {
        format!("Content-Length: {}\r\n", reply.body.len())
    };
    let head = format!(
        "HTTP/1.1 {} Fake\r\nConnection: close\r\n{framing}{}\r\n",
        reply.status, reply.headers
    );
    if stream.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    if reply.hold {
        std::future::pending::<()>().await;
    }
    if reply.chunked {
        for chunk in reply.body.chunks(16 * 1024) {
            let length = format!("{:x}\r\n", chunk.len());
            if stream.write_all(length.as_bytes()).await.is_err()
                || stream.write_all(chunk).await.is_err()
                || stream.write_all(b"\r\n").await.is_err()
            {
                return;
            }
        }
        _ = stream.write_all(b"0\r\n\r\n").await;
    } else {
        _ = stream.write_all(&reply.body).await;
    }
    _ = stream.shutdown().await;
}
