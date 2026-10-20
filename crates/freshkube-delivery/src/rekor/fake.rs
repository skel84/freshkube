//! A loopback https server that answers a log's GETs with canned replies and
//! records every request it got, header lines included.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;

pub(super) const CERT: &[u8] = include_bytes!("../../testdata/results/loopback-cert.pem");
const KEY: &[u8] = include_bytes!("../../testdata/results/loopback-key.pem");

/// What the fake answers every request with.
pub(super) struct Reply {
    pub status: u16,
    pub headers: String,
    pub body: Vec<u8>,
    /// Read the request and never answer.
    pub hold: bool,
}

pub(super) struct Log {
    pub address: String,
    pub requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl Log {
    pub fn new(reply: Reply) -> Self {
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
        let reply = Arc::new(reply);
        let task = tokio::spawn({
            let requests = requests.clone();
            async move {
                let listener = TcpListener::from_std(listener).unwrap();
                loop {
                    let (stream, _) = listener.accept().await.unwrap();
                    let (acceptor, requests, reply) =
                        (acceptor.clone(), requests.clone(), reply.clone());
                    tokio::spawn(async move {
                        let Ok(mut stream) = acceptor.accept(stream).await else {
                            return;
                        };
                        let mut request = Vec::new();
                        while !request.ends_with(b"\r\n\r\n") {
                            let Ok(byte) = stream.read_u8().await else {
                                return;
                            };
                            request.push(byte);
                        }
                        requests
                            .lock()
                            .unwrap()
                            .push(String::from_utf8_lossy(&request).into_owned());
                        if reply.hold {
                            std::future::pending::<()>().await;
                        }
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
            address: format!("https://127.0.0.1:{port}"),
            requests,
            task,
        }
    }
}

impl Drop for Log {
    fn drop(&mut self) {
        self.task.abort();
    }
}
