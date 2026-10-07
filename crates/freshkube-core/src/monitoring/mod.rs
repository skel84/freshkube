//! Prometheus dashboards, read from any server that answers the Prometheus
//! HTTP API: Prometheus, VictoriaMetrics, Thanos or Mimir.
//!
//! Dashboards are Grafana JSON, read and planned by grafaui's model and query
//! crates (re-exported as [`model`] and [`prometheus`]). [`discover`] finds an
//! in-cluster Service and confirms it with a query; [`Prometheus`] sends every
//! planned request as a GET, either through `services/proxy` with the chosen
//! context's credentials or to a URL the user entered, with an optional
//! bearer token; [`Source`] is either that or made-up example data. Nothing
//! here changes the cluster: the proxy only ever sees GET, which needs `get`
//! on `services/proxy` and nothing more.

pub mod builtin;
pub mod catalog;
mod discovery;
mod example;
pub mod history;
pub mod markers;
mod transport;

#[cfg(test)]
mod tests;

use std::fmt;

use crate::base_url::{BaseUrlError, parse_base_url};

pub use discovery::{
    Candidate, Discovery, LOOKED_FOR, Rank, Tried, confirm, confirm_url, discover, list_candidates,
    rank,
};
pub use example::ExampleSource;
pub use grafaui_model as model;
pub use grafaui_prometheus as prometheus;
pub use transport::{
    BuildInfo, DEFAULT_SCRAPE_INTERVAL, MAX_ANSWER, MAX_CONCURRENT, PanelResult, Prometheus,
    REQUEST_TIMEOUT, Source,
};

/// One Service port that serves the Prometheus HTTP API, under an
/// optional path prefix such as VictoriaMetrics' `select/0/prometheus`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PrometheusService {
    pub namespace: String,
    pub name: String,
    pub port: u16,
    /// The port speaks TLS, so the proxy must use `https:`.
    pub https: bool,
    /// Where the API lives under the port, without leading or trailing
    /// slashes; empty for Prometheus itself. See [`normalise_path`].
    pub path: String,
}

impl PrometheusService {
    pub fn new(namespace: impl Into<String>, name: impl Into<String>, port: u16) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            port,
            https: false,
            path: String::new(),
        }
    }

    pub fn with_path(mut self, path: &str) -> Self {
        self.path = normalise_path(path);
        self
    }

    /// The API server path that proxies to the port and its prefix, ending
    /// in a slash.
    pub fn proxy_base(&self) -> String {
        let scheme = if self.https { "https:" } else { "" };
        let path = if self.path.is_empty() {
            String::new()
        } else {
            format!("{}/", self.path)
        };
        format!(
            "/api/v1/namespaces/{}/services/{scheme}{}:{}/proxy/{path}",
            self.namespace, self.name, self.port
        )
    }

    /// `namespace/name:port` and any prefix, as the page and its status
    /// line show it.
    pub fn label(&self) -> String {
        let mut label = format!("{}/{}:{}", self.namespace, self.name, self.port);
        if !self.path.is_empty() {
            label.push('/');
            label.push_str(&self.path);
        }
        label
    }

    /// Which server this is, by the name its installers give it.
    pub fn backend(&self) -> Backend {
        Backend::by_name(&self.name, &self.path)
    }
}

/// A path prefix as the proxy and URLs use it: segments joined by single
/// slashes, with no leading or trailing slash, `.` or `..`.
pub fn normalise_path(path: &str) -> String {
    path.split('/')
        .map(str::trim)
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .collect::<Vec<_>>()
        .join("/")
}

/// The servers that answer the Prometheus HTTP API, for labels. Detection
/// is by name only; every one is confirmed by a query before use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Backend {
    Prometheus,
    VictoriaMetrics,
    Thanos,
    Mimir,
    /// A URL: whatever answers there.
    Compatible,
}

impl Backend {
    pub fn by_name(name: &str, path: &str) -> Self {
        let name = name.to_ascii_lowercase();
        if name.starts_with("vmsingle")
            || name.starts_with("vmselect")
            || name.contains("victoria-metrics")
            || name.contains("victoriametrics")
            || path.starts_with("select/")
        {
            Self::VictoriaMetrics
        } else if name.contains("thanos") {
            Self::Thanos
        } else if name.contains("mimir") || name.contains("cortex") {
            Self::Mimir
        } else {
            Self::Prometheus
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Prometheus => "Prometheus",
            Self::VictoriaMetrics => "VictoriaMetrics",
            Self::Thanos => "Thanos",
            Self::Mimir => "Mimir",
            Self::Compatible => "Prometheus API",
        }
    }
}

/// Where the Prometheus HTTP API is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Endpoint {
    /// A Service, through the Kubernetes API's service proxy.
    Service(PrometheusService),
    /// A URL reachable from the desktop, as [`normalise_url`] returns it.
    /// A token, if any, travels separately and is never part of it.
    Url(String),
}

impl Endpoint {
    pub fn label(&self) -> String {
        match self {
            Self::Service(service) => service.label(),
            Self::Url(url) => url.trim_end_matches('/').to_owned(),
        }
    }

    pub fn backend(&self) -> Backend {
        match self {
            Self::Service(service) => service.backend(),
            Self::Url(_) => Backend::Compatible,
        }
    }
}

/// Checks a typed base URL and returns it ending in a slash: `http` or
/// `https`, a host, and no user, password, query or fragment, since a
/// token is entered separately and never kept in the URL.
pub fn normalise_url(text: &str) -> Result<String, String> {
    let url = parse_base_url(text.trim()).map_err(|error| {
        match error {
            BaseUrlError::NotAUrl => "Enter a full URL, such as https://metrics.example.com",
            BaseUrlError::Scheme => "The URL must start with http:// or https://",
            BaseUrlError::NoHost => "The URL has no host",
            BaseUrlError::UserInfo => {
                "Leave the user and password out of the URL; use a token instead"
            }
            BaseUrlError::QueryOrFragment => "The URL can't have a query or fragment",
        }
        .to_owned()
    })?;
    let path = normalise_path(url.path());
    let mut base = format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default());
    if let Some(port) = url.port() {
        base.push_str(&format!(":{port}"));
    }
    base.push('/');
    if !path.is_empty() {
        base.push_str(&path);
        base.push('/');
    }
    Ok(base)
}

/// What went wrong with a Prometheus read, by category. Messages never
/// carry URLs or credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The identity may not proxy to the Service, or a URL refused the
    /// token (401 or 403).
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

    /// A URL's server refused the token, or asked for one.
    pub fn token_refused() -> Self {
        Self::new(
            ErrorKind::Refused,
            "The server refused the request: check the token",
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
