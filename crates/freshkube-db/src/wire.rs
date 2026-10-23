//! The connection under a session: one text statement in, text rows out.
//!
//! [`TokioPostgres`] is the real one. The session's rules (read-only, timeouts,
//! caps) live above this line in `postgres.rs`, so a fake wire in `fake.rs`
//! tests them without a server.
use crate::{credentials::Password, limits::Limits, session::ConnectError};
use async_trait::async_trait;
use dbflux_core::DbError;
use tokio_postgres::{Client, Config, NoTls, SimpleQueryMessage};

/// Rows of text, as the simple query protocol returns them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}

#[async_trait]
pub trait Wire: Send {
    async fn simple(&mut self, sql: &str) -> Result<Rows, DbError>;
}

pub struct Target<'a> {
    pub host: &'a str,
    pub port: u16,
    pub user: &'a str,
    pub database: &'a str,
}

pub struct TokioPostgres {
    client: Client,
    connection: tokio::task::JoinHandle<()>,
}

impl TokioPostgres {
    /// Opens the connection within `limits.connect_timeout`. The traffic runs
    /// over a loopback port forward, which is TLS to the API server already,
    /// so the Postgres leg is plain; a server that insists on TLS is `Refused`.
    pub async fn connect(
        target: Target<'_>,
        password: &Password,
        limits: &Limits,
    ) -> Result<Self, ConnectError> {
        let mut config = Config::new();
        config
            .host(target.host)
            .port(target.port)
            .user(target.user)
            .password(password.expose())
            .dbname(target.database)
            .application_name("freshkube")
            .connect_timeout(limits.connect_timeout);
        let connect = tokio::time::timeout(limits.connect_timeout, config.connect(NoTls));
        let (client, connection) = match connect.await {
            Err(_) => return Err(ConnectError::TimedOut),
            Ok(Err(e)) => return Err(classify(&e)),
            Ok(Ok(pair)) => pair,
        };
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok(Self { client, connection })
    }
}

impl Drop for TokioPostgres {
    fn drop(&mut self) {
        self.connection.abort();
    }
}

fn classify(error: &tokio_postgres::Error) -> ConnectError {
    match error.as_db_error() {
        Some(db) => ConnectError::Refused(db.message().to_string()),
        None => ConnectError::Unreachable(error.to_string()),
    }
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
