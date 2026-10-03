//! Prometheus dashboards, read through the Kubernetes API's service proxy.
//!
//! Dashboards are Grafana JSON, read and planned by grafaui's model and query
//! crates (re-exported as [`model`] and [`prometheus`]). [`discover`] finds an
//! in-cluster Prometheus Service and confirms it; [`Prometheus`] sends every
//! planned request as a GET through `services/proxy` with the chosen
//! context's credentials; [`Source`] is either that or made-up example data.
//! Nothing here changes the cluster: the proxy only ever sees GET, which needs
//! `get` on `services/proxy` and nothing more.

mod discovery;
mod example;
mod transport;

#[cfg(test)]
mod tests;

use std::fmt;

pub use discovery::{
    Candidate, Discovery, LOOKED_FOR, Rank, Tried, confirm, discover, list_candidates, rank,
};
pub use example::ExampleSource;
pub use grafaui_model as model;
pub use grafaui_prometheus as prometheus;
pub use transport::{
    BuildInfo, DEFAULT_SCRAPE_INTERVAL, MAX_ANSWER, MAX_CONCURRENT, PanelResult, Prometheus,
    REQUEST_TIMEOUT, Source,
};

/// One Service port that serves the Prometheus HTTP API.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PrometheusService {
    pub namespace: String,
    pub name: String,
    pub port: u16,
    /// The port speaks TLS, so the proxy must use `https:`.
    pub https: bool,
}

impl PrometheusService {
    pub fn new(namespace: impl Into<String>, name: impl Into<String>, port: u16) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            port,
            https: false,
        }
    }

    /// The API server path that proxies to the port, ending in a slash.
    pub fn proxy_base(&self) -> String {
        let scheme = if self.https { "https:" } else { "" };
        format!(
            "/api/v1/namespaces/{}/services/{scheme}{}:{}/proxy/",
            self.namespace, self.name, self.port
        )
    }

    /// `namespace/name:port`, as the page and its status line show it.
    pub fn label(&self) -> String {
        format!("{}/{}:{}", self.namespace, self.name, self.port)
    }
}

/// What went wrong with a Prometheus read, by category. Messages never
/// carry URLs or credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The identity may not proxy to the Service (401 or 403).
    Refused,
    /// The Service, its port or the API path doesn't exist (404).
    NotFound,
    /// The Service has no ready endpoint, or the connection failed.
    Unavailable,
    /// No answer within [`REQUEST_TIMEOUT`].
    TimedOut,
    /// An answer that isn't Prometheus JSON, or one over [`MAX_ANSWER`].
    BadAnswer,
    /// Prometheus read the request and refused it, such as bad PromQL.
    Rejected,
    /// The dashboard asks for something this connection can't send.
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryError {
    pub kind: ErrorKind,
    pub message: String,
}

impl QueryError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn refused() -> Self {
        Self::new(
            ErrorKind::Refused,
            "Not allowed to query Prometheus (services/proxy)",
        )
    }

    /// A planning error from grafaui: the dashboard, not the connection.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, message)
    }

    /// Retrying won't help until RBAC or the Service changes.
    pub fn is_permanent(&self) -> bool {
        matches!(self.kind, ErrorKind::Refused | ErrorKind::NotFound)
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for QueryError {}
