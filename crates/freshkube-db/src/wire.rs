//! The connection under a session: one text statement in, text rows out.
//!
//! [`TokioPostgres`] is the real one. The session's rules (read-only, timeouts,
//! caps) live above this line in `postgres.rs`, so a fake wire in `fake.rs`
//! tests them without a server.
use crate::{credentials::Password, limits::Limits, session::ConnectError};
use async_trait::async_trait;
use dbflux_core::DbError;
use rustls::{
    ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::CryptoProvider,
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use std::sync::Arc;
use tokio_postgres::{Client, Config, SimpleQueryMessage, config::SslMode};
use tokio_postgres_rustls::MakeRustlsConnect;

/// Rows of text, as the simple query protocol returns them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}

#[async_trait]
pub trait Wire: Send {
    async fn simple(&mut self, sql: &str) -> Result<Rows, DbError>;

    /// Has the server parse `sql` and refuse it unless it is exactly one
    /// statement. Our own splitter is a first line; this is the server's word.
    async fn single_statement(&mut self, sql: &str) -> Result<(), DbError>;
}

pub struct Target<'a> {
    /// Where the forward listens, normally `127.0.0.1`.
    pub address: std::net::IpAddr,
    pub port: u16,
    pub user: &'a str,
    pub database: &'a str,
}

/// How the server is to be recognised. Through a port forward the address is
/// loopback, so the name checked against the certificate is the Service's.
pub struct TlsPlan {
    /// The Service's DNS name, such as `orders-db-ro.staging.svc`.
    pub server_name: String,
    /// The cluster CA's `ca.crt` (a certificate, never a key), when it could
    /// be read; without it the connection is encrypted but not verified.
    pub ca_pem: Option<String>,
}

/// What protects the Postgres leg, as the connect step reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encryption {
    /// TLS, and the server's certificate was checked against the cluster CA.
    Verified,
    /// TLS, but no CA was available to check the certificate against.
    Unverified,
    /// The server did not offer TLS.
    Plain,
}

impl Encryption {
    pub fn label(self) -> &'static str {
        match self {
            Self::Verified => "encrypted, certificate verified",
            Self::Unverified => "encrypted, certificate not verified",
            Self::Plain => "not encrypted",
        }
    }
}

pub struct TokioPostgres {
    client: Client,
    connection: tokio::task::JoinHandle<()>,
}

impl TokioPostgres {
    /// Opens the connection within `limits.connect_timeout`, trying TLS first
    /// and falling back to plain only when the server doesn't offer it
    /// (`sslmode=prefer`). A server that offers TLS and fails the handshake or
    /// the certificate check is an error, never a silent downgrade.
    pub async fn connect(
        target: Target<'_>,
        password: &Password,
        limits: &Limits,
        tls: &TlsPlan,
    ) -> Result<(Self, Encryption), ConnectError> {
        let verifying = tls.ca_pem.is_some();
        let connector = make_connector(tls)?;
        let mut config = Config::new();
        config
            // The host is what TLS checks; the address is where to go.
            .host(&tls.server_name)
            .hostaddr(target.address)
            .port(target.port)
            .user(target.user)
            .password(password.expose())
            .dbname(target.database)
            .application_name("freshkube")
            .ssl_mode(SslMode::Prefer)
            .connect_timeout(limits.connect_timeout);
        let connect = tokio::time::timeout(limits.connect_timeout, config.connect(connector));
        let (client, connection) = match connect.await {
            Err(_) => return Err(ConnectError::TimedOut),
            Ok(Err(e)) => return Err(classify(&e)),
            Ok(Ok(pair)) => pair,
        };
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        let this = Self { client, connection };
        let encrypted = this.encrypted().await;
        let encryption = match (encrypted, verifying) {
            (false, _) => Encryption::Plain,
            (true, true) => Encryption::Verified,
            (true, false) => Encryption::Unverified,
        };
        Ok((this, encryption))
    }

    /// Whether the server says this session uses TLS.
    async fn encrypted(&self) -> bool {
        let answer = self
            .client
            .simple_query("SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()")
            .await;
        let Ok(messages) = answer else {
            return false;
        };
        messages
            .iter()
            .any(|m| matches!(m, SimpleQueryMessage::Row(row) if row.get(0) == Some("t")))
    }
}

fn make_connector(tls: &TlsPlan) -> Result<MakeRustlsConnect, ConnectError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| ConnectError::Unreachable(e.to_string()))?;
    let config = match &tls.ca_pem {
        Some(pem) => {
            let mut roots = RootCertStore::empty();
            let certs: Vec<_> = rustls_pemfile::certs(&mut pem.as_bytes())
                .filter_map(Result::ok)
                .collect();
            let (added, _) = roots.add_parsable_certificates(certs);
            if added == 0 {
                return Err(ConnectError::Untrusted(
                    "the cluster CA could not be read as a certificate".into(),
                ));
            }
            builder.with_root_certificates(roots).with_no_client_auth()
        }
        None => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAny(provider)))
            .with_no_client_auth(),
    };
    Ok(MakeRustlsConnect::new(config))
}

/// Encrypts without checking who answered: used only when there is no CA to
/// check against, and reported as such.
#[derive(Debug)]
struct AcceptAny(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

impl Drop for TokioPostgres {
    fn drop(&mut self) {
        self.connection.abort();
    }
}

fn classify(error: &tokio_postgres::Error) -> ConnectError {
    if let Some(db) = error.as_db_error() {
        return ConnectError::Refused(db.message().to_string());
    }
    let text = format!("{error}: {}", source_chain(error));
    if text.contains("certificate") || text.contains("invalid peer") {
        ConnectError::Untrusted(text)
    } else {
        ConnectError::Unreachable(text)
    }
}

fn source_chain(error: &dyn std::error::Error) -> String {
    let mut out = Vec::new();
    let mut next = error.source();
    while let Some(e) = next {
        out.push(e.to_string());
        next = e.source();
    }
    out.join(": ")
}

pub(crate) fn to_db_error(error: &tokio_postgres::Error) -> DbError {
    use tokio_postgres::error::SqlState;
    let Some(db) = error.as_db_error() else {
        return DbError::connection_failed(error.to_string());
    };
    let code = db.code();
    if *code == SqlState::QUERY_CANCELED || *code == SqlState::LOCK_NOT_AVAILABLE {
        DbError::Timeout
    } else if *code == SqlState::INSUFFICIENT_PRIVILEGE {
        DbError::permission_denied(db.message())
    } else if *code == SqlState::UNDEFINED_TABLE || *code == SqlState::UNDEFINED_COLUMN {
        DbError::object_not_found(db.message())
    } else if code.code().starts_with("28") {
        DbError::auth_failed(db.message())
    } else {
        DbError::query_failed(db.message())
    }
}

#[async_trait]
impl Wire for TokioPostgres {
    async fn single_statement(&mut self, sql: &str) -> Result<(), DbError> {
        // The extended protocol allows one statement per Parse.
        self.client
            .prepare(sql)
            .await
            .map(|_| ())
            .map_err(|e| to_db_error(&e))
    }

    async fn simple(&mut self, sql: &str) -> Result<Rows, DbError> {
        let messages = self
            .client
            .simple_query(sql)
            .await
            .map_err(|e| to_db_error(&e))?;
        let mut out = Rows::default();
        for message in messages {
            match message {
                SimpleQueryMessage::RowDescription(columns) => {
                    out.columns = columns.iter().map(|c| c.name().to_string()).collect();
                }
                SimpleQueryMessage::Row(row) => {
                    out.rows.push(
                        (0..row.len())
                            .map(|i| row.get(i).map(str::to_string))
                            .collect(),
                    );
                }
                _ => {}
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_failed_connect_never_shows_the_password() {
        // A port nothing listens on: bind one, then let it go.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let password = Password::new("hunter2");
        let limits = Limits {
            connect_timeout: std::time::Duration::from_secs(2),
            ..Limits::default()
        };
        let plan = TlsPlan {
            server_name: "orders-db-ro.staging.svc".into(),
            ca_pem: None,
        };
        let error = TokioPostgres::connect(
            Target {
                address: "127.0.0.1".parse().unwrap(),
                port,
                user: "shop_reader",
                database: "shop",
            },
            &password,
            &limits,
            &plan,
        )
        .await
        .err()
        .expect("nothing listens there");
        for text in [format!("{error:?}"), error.to_string()] {
            assert!(!text.contains("hunter2"), "{text}");
        }
    }

    #[test]
    fn an_unreadable_ca_is_refused_before_connecting() {
        let plan = TlsPlan {
            server_name: "x".into(),
            ca_pem: Some("not a certificate".into()),
        };
        assert!(matches!(
            make_connector(&plan),
            Err(ConnectError::Untrusted(_))
        ));
    }
}
