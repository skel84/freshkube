//! A loopback https server for the fetch tests: it answers each GET from a
//! handler and records every request, header lines included.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;

pub const CERT: &[u8] = include_bytes!("../../../testdata/team/loopback-cert.pem");
const KEY: &[u8] = include_bytes!("../../../testdata/team/loopback-key.pem");

pub struct Reply {
    pub status: u16,
    pub headers: String,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn ok(body: &str) -> Self {
        Self {
            status: 200,
            headers: String::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn redirect(to: &str) -> Self {
        Self {
            status: 302,
            headers: format!("location: {to}\r\n"),
            body: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Request {
    pub path: String,
    head: String,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<String> {
        self.head.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
    }
}

pub struct Server {
    pub port: u16,
    requests: Arc<Mutex<Vec<Request>>>,
    task: JoinHandle<()>,
}

type Handler = dyn Fn(&Request) -> Reply + Send + Sync;

impl Server {
    pub fn new(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Self {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from_pem_slice(CERT).unwrap()],
            PrivateKeyDer::from_pem_slice(KEY).unwrap(),
        )
        .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let task = tokio::spawn({
            let requests = requests.clone();
            async move {
                let listener = TcpListener::from_std(listener).unwrap();
                loop {
                    let (stream, _) = listener.accept().await.unwrap();
                    let (acceptor, requests, handler) =
                        (acceptor.clone(), requests.clone(), handler.clone());
                    tokio::spawn(async move {
                        let Ok(mut stream) = acceptor.accept(stream).await else {
                            return;
                        };
                        let mut head = Vec::new();
                        while !head.ends_with(b"\r\n\r\n") {
                            let Ok(byte) = stream.read_u8().await else {
                                return;
                            };
                            head.push(byte);
                        }
                        let head = String::from_utf8_lossy(&head).into_owned();
                        let path = head
                            .lines()
                            .next()
                            .and_then(|line| line.split_whitespace().nth(1))
                            .unwrap_or_default()
                            .to_owned();
                        let request = Request { path, head };
                        requests.lock().unwrap().push(request.clone());
                        let reply = handler(&request);
                        let head = format!(
                            "HTTP/1.1 {} X\r\ncontent-length: {}\r\nconnection: close\r\n{}\r\n",
                            reply.status,
                            reply.body.len(),
                            reply.headers
                        );
                        let _ = stream.write_all(head.as_bytes()).await;
                        let _ = stream.write_all(&reply.body).await;
                        let _ = stream.shutdown().await;
                    });
                }
            }
        });
        Self {
            port,
            requests,
            task,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{path}", self.port)
    }

    pub fn seen(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
