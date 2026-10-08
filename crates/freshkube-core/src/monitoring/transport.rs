//! The Prometheus HTTP API through the Kubernetes API's service proxy or at
//! a URL, GET only.
use std::sync::Arc;
use std::time::Duration;

use futures::future::try_join_all;
use grafaui_model::{PanelSpec, Variable, data::Frame, data::QueryContext, time::TimeWindow};
use grafaui_prometheus::{Variables, api::ApiRequest, plan::VariablePlan, query, response};
use http::Request;
use http_body_util::BodyExt;
use kube::client::Body;
use serde_json::Value;
use tokio::sync::Semaphore;

use super::{Endpoint, ErrorKind, ExampleSource, PrometheusService, QueryError};

/// How long one request may take, as grafaui's client allows.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// The largest answer read; a bigger one is refused, never cut.
pub const MAX_ANSWER: usize = 64 * 1024 * 1024;
/// Requests in flight at once per [`Prometheus`] and its clones.
pub const MAX_CONCURRENT: usize = 4;
/// Seconds, when the targets report no scrape interval.
pub const DEFAULT_SCRAPE_INTERVAL: f64 = 30.;

/// One confirmed Prometheus API, behind the service proxy or at a URL.
/// Clones share the client and the request slots.
#[derive(Clone)]
pub struct Prometheus {
    endpoint: Endpoint,
    transport: Transport,
    scrape_interval: f64,
    slots: Arc<Semaphore>,
    timeout: Duration,
    limit: usize,
}

/// How requests travel.
#[derive(Clone)]
enum Transport {
    Proxy(kube::Client),
    /// The bearer token is only ever sent to this endpoint's URL: the
    /// client follows no redirects.
    Direct {
        http: reqwest::Client,
        token: Option<Arc<str>>,
    },
}

/// A panel's data, with the PromQL that was sent for it, after
/// interpolation, for the page to show on hover.
#[derive(Clone, Debug, Default)]
pub struct PanelResult {
    pub frame: Frame,
    pub warnings: Vec<String>,
    /// `(refId, expression)` for each request, in the dashboard's order.
    pub expressions: Vec<(String, String)>,
}

impl Prometheus {
    pub fn new(client: kube::Client, service: PrometheusService) -> Self {
        Self::with_transport(Endpoint::Service(service), Transport::Proxy(client))
    }

    /// A server at a URL from [`super::normalise_url`], with an optional
    /// bearer token.
    pub fn direct(url: String, token: Option<String>) -> Result<Self, QueryError> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| {
                QueryError::new(ErrorKind::Unavailable, "Couldn't set up an HTTP client")
            })?;
        let token = token
            .map(|token| token.trim().to_owned())
            .filter(|token| !token.is_empty())
            .map(Arc::from);
        Ok(Self::with_transport(
            Endpoint::Url(url),
            Transport::Direct { http, token },
        ))
    }

    fn with_transport(endpoint: Endpoint, transport: Transport) -> Self {
        Self {
            endpoint,
            transport,
            scrape_interval: DEFAULT_SCRAPE_INTERVAL,
            slots: Arc::new(Semaphore::new(MAX_CONCURRENT)),
            timeout: REQUEST_TIMEOUT,
            limit: MAX_ANSWER,
        }
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// The Service, when requests go through the proxy.
    pub fn service(&self) -> Option<&PrometheusService> {
        match &self.endpoint {
            Endpoint::Service(service) => Some(service),
            Endpoint::Url(_) => None,
        }
    }

    pub fn scrape_interval(&self) -> f64 {
        self.scrape_interval
    }

    pub fn with_scrape_interval(mut self, seconds: f64) -> Self {
        if seconds.is_finite() && seconds > 0. {
            self.scrape_interval = seconds;
        }
        self
    }

    /// How long each request may take; [`REQUEST_TIMEOUT`] by default.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[cfg(test)]
    pub(super) fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Sends one planned request as a GET and returns its JSON. Prometheus'
    /// own error envelopes come back as [`ErrorKind::Rejected`].
    pub async fn get(&self, request: &ApiRequest) -> Result<Value, QueryError> {
        let target = request.get_target().map_err(QueryError::unsupported)?;
        let _slot = self
            .slots
            .acquire()
            .await
            .map_err(|_| QueryError::new(ErrorKind::Unavailable, "The connection closed"))?;
        let answer = async {
            match (&self.transport, &self.endpoint) {
                (Transport::Proxy(client), Endpoint::Service(service)) => {
                    let request = Request::get(format!("{}{target}", service.proxy_base()))
                        .header(http::header::ACCEPT, "application/json")
                        .body(Body::empty())
                        .map_err(|_| QueryError::unsupported("The request couldn't be built"))?;
                    self.send(client, request).await
                }
                (Transport::Direct { http, token }, Endpoint::Url(url)) => {
                    self.send_direct(http, token.as_deref(), &format!("{url}{target}"))
                        .await
                }
                _ => Err(QueryError::unsupported(
                    "The connection doesn't match its endpoint",
                )),
            }
        };
        let (status, bytes) =
            tokio::time::timeout(self.timeout, answer)
                .await
                .map_err(|_| {
                    QueryError::new(
                        ErrorKind::TimedOut,
                        format!(
                            "Prometheus didn't answer within {} s",
                            self.timeout.as_secs()
                        ),
                    )
                })??;
        let direct = matches!(self.transport, Transport::Direct { .. });
        classify(status, &bytes).map_err(|error| {
            if direct && error.kind == ErrorKind::Refused {
                QueryError::token_refused()
            } else {
                error
            }
        })
    }

    /// One GET to the URL's server. Errors name neither the URL nor the
    /// token.
    async fn send_direct(
        &self,
        http: &reqwest::Client,
        token: Option<&str>,
        url: &str,
    ) -> Result<(u16, Vec<u8>), QueryError> {
        let mut request = http
            .get(url)
            .header(http::header::ACCEPT, "application/json");
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let mut response = request.send().await.map_err(|error| {
            let message = if error.is_timeout() {
                "The server didn't answer in time"
            } else if error.is_connect() {
                "Couldn't connect to the server"
            } else {
                "Couldn't reach the server"
            };
            QueryError::new(ErrorKind::Unavailable, message)
        })?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(QueryError::new(
                ErrorKind::Unavailable,
                format!("The server redirected (HTTP {status}); enter the address it redirects to"),
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| QueryError::new(ErrorKind::Unavailable, "The answer was cut off"))?
        {
            if bytes.len() + chunk.len() > self.limit {
                return Err(too_large(self.limit));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((status, bytes))
    }

    async fn send(
        &self,
        client: &kube::Client,
        request: Request<Body>,
    ) -> Result<(u16, Vec<u8>), QueryError> {
        let response = client.send(request).await.map_err(|error| {
            // kube's transport errors can name the server; keep them out.
            let message = match error {
                kube::Error::Auth(_) => {
                    return QueryError::new(
                        ErrorKind::Refused,
                        "The context's credentials were refused",
                    );
                }
                _ => "Couldn't reach the Kubernetes API",
            };
            QueryError::new(ErrorKind::Unavailable, message)
        })?;
        let status = response.status().as_u16();
        let mut body = response.into_body();
        let mut bytes = Vec::new();
        while let Some(frame) = body.frame().await {
            let frame = frame
                .map_err(|_| QueryError::new(ErrorKind::Unavailable, "The answer was cut off"))?;
            if let Some(data) = frame.data_ref() {
                if bytes.len() + data.len() > self.limit {
                    return Err(too_large(self.limit));
                }
                bytes.extend_from_slice(data);
            }
        }
        Ok((status, bytes))
    }

    /// Confirms the server answers the Prometheus API: a query for `1`.
    /// Unlike a readiness check, this needs the same access as a dashboard,
    /// so a wrong token or a missing path prefix fails here.
    pub async fn probe(&self) -> Result<(), QueryError> {
        let value = self
            .get(&ApiRequest::new(
                "query",
                vec![("query".into(), "1".into())],
            ))
            .await?;
        let (data, _) = response::envelope(&value).map_err(bad_answer)?;
        if data.get("resultType").and_then(Value::as_str).is_none() {
            return Err(bad_answer("The answer isn't a Prometheus query result"));
        }
        Ok(())
    }

    /// The version from `/api/v1/status/buildinfo`. Servers other than
    /// Prometheus may not answer it, or answer with the Prometheus version
    /// they imitate.
    pub async fn read_version(&self) -> Result<String, QueryError> {
        let value = self
            .get(&ApiRequest::new("status/buildinfo", Vec::new()))
            .await?;
        let (data, _) = response::envelope(&value).map_err(bad_answer)?;
        let version = data
            .get("version")
            .and_then(Value::as_str)
            .filter(|version| !version.is_empty())
            .ok_or_else(|| bad_answer("The answer has no Prometheus version"))?;
        Ok(version.to_owned())
    }

    /// The scrape interval most active targets use, from `/api/v1/targets`,
    /// or `None` when no target reports one.
    pub async fn read_scrape_interval(&self) -> Result<Option<f64>, QueryError> {
        let value = self
            .get(&ApiRequest::new(
                "targets",
                vec![("state".into(), "active".into())],
            ))
            .await?;
        let (data, _) = response::envelope(&value).map_err(bad_answer)?;
        Ok(scrape_interval(data))
    }

    /// Resolves the dashboard's variables in order, one request at a time,
    /// since each may depend on the one before.
    pub async fn resolve_variables(
        &self,
        definitions: &[Variable],
        selections: &[String],
        window: TimeWindow,
    ) -> Result<(Variables, Vec<String>), QueryError> {
        let mut plan = VariablePlan::new(definitions, selections, window, self.scrape_interval);
        while let Some(request) = plan.next_request().map_err(QueryError::unsupported)? {
            let value = self.get(&request).await.map_err(|error| QueryError {
                message: plan.failed(&error.message),
                ..error
            })?;
            plan.answer(&value).map_err(classify_plan)?;
        }
        Ok(plan.finish())
    }

    /// Reads a panel: every target's requests at once, up to the slots.
    pub async fn query_panel(
        &self,
        panel: &PanelSpec,
        context: &QueryContext,
        variables: &Variables,
    ) -> Result<PanelResult, QueryError> {
        let requests = query::requests(panel, context, variables, self.scrape_interval)
            .map_err(QueryError::unsupported)?;
        let expressions = expressions(&requests);
        let answers = try_join_all(requests.iter().map(|request| async move {
            let value = self.get(&request.api()).await.map_err(|error| QueryError {
                message: format!("{}: {}", request.ref_id(), error.message),
                ..error
            })?;
            response::convert(&value, request)
                .map_err(|message| bad_answer(format!("{}: {message}", request.ref_id())))
        }))
        .await?;
        let mut frames = Vec::with_capacity(answers.len());
        let mut warnings = Vec::new();
        for (frame, notes) in answers {
            frames.push(frame);
            warnings.extend(notes);
        }
        warnings.extend(panel_warnings(panel));
        Ok(PanelResult {
            frame: response::merge(frames),
            warnings,
            expressions,
        })
    }
}

/// Where a dashboard's data comes from: a confirmed Prometheus, or made-up
/// series for `--fixture` and tests.
#[derive(Clone)]
pub enum Source {
    Example(ExampleSource),
    Prometheus(Prometheus),
}

impl Source {
    pub fn scrape_interval(&self) -> f64 {
        match self {
            Self::Example(_) => DEFAULT_SCRAPE_INTERVAL,
            Self::Prometheus(prometheus) => prometheus.scrape_interval(),
        }
    }

    pub async fn resolve_variables(
        &self,
        definitions: &[Variable],
        selections: &[String],
        window: TimeWindow,
    ) -> Result<(Variables, Vec<String>), QueryError> {
        match self {
            Self::Example(example) => example.resolve_variables(definitions, selections, window),
            Self::Prometheus(prometheus) => {
                prometheus
                    .resolve_variables(definitions, selections, window)
                    .await
            }
        }
    }

    pub async fn query_panel(
        &self,
        panel: &PanelSpec,
        context: &QueryContext,
        variables: &Variables,
    ) -> Result<PanelResult, QueryError> {
        match self {
            Self::Example(example) => example.query_panel(panel, context, variables),
            Self::Prometheus(prometheus) => prometheus.query_panel(panel, context, variables).await,
        }
    }
}

pub(super) fn expressions(requests: &[query::Request]) -> Vec<(String, String)> {
    let mut expressions: Vec<(String, String)> = Vec::new();
    for request in requests {
        // A target sent as both range and instant is one expression.
        if !expressions
            .iter()
            .any(|(id, text)| id == request.ref_id() && text == request.query())
        {
            expressions.push((request.ref_id().to_owned(), request.query().to_owned()));
        }
    }
    expressions
}

/// What a panel asks for that this connection reads differently, as
/// grafaui's own client reports it.
pub(super) fn panel_warnings(panel: &PanelSpec) -> Vec<String> {
    let mut warnings = Vec::new();
    if panel.queries.iter().any(|query| query.request.exemplar) {
        warnings.push("Exemplars are not requested by this connection".into());
    }
    if panel.queries.iter().any(|query| {
        query.request.format.as_deref() == Some("table")
            && query
                .request
                .range
                .unwrap_or(!query.request.instant.unwrap_or(false))
    }) {
        warnings.push("Range table targets currently reduce each series to one row".into());
    }
    warnings
}

fn too_large(limit: usize) -> QueryError {
    QueryError::new(
        ErrorKind::BadAnswer,
        format!(
            "Prometheus's answer is larger than {} MiB",
            limit.div_ceil(1024 * 1024)
        ),
    )
}

fn bad_answer(message: impl Into<String>) -> QueryError {
    QueryError::new(ErrorKind::BadAnswer, message)
}

/// A variable answer that failed to read: Prometheus' own error envelope is
/// a refusal of the query; anything else is a bad answer.
fn classify_plan(message: String) -> QueryError {
    if message.contains("bad_data") || message.contains("execution") {
        QueryError::new(ErrorKind::Rejected, message)
    } else {
        bad_answer(message)
    }
}

/// Reads an answer by status. Prometheus' error envelope wins over the
/// status code; then a Kubernetes Status from the API server or proxy.
fn classify(status: u16, bytes: &[u8]) -> Result<Value, QueryError> {
    let value = serde_json::from_slice::<Value>(bytes).ok();
    if (200..300).contains(&status) {
        let value = value.ok_or_else(|| bad_answer("Prometheus's answer isn't JSON"))?;
        if let Err(message) = response::envelope(&value) {
            return Err(QueryError::new(ErrorKind::Rejected, message));
        }
        return Ok(value);
    }
    if let Some(value) = &value
        && value.get("status").and_then(Value::as_str) == Some("error")
    {
        let message = response::envelope(value)
            .err()
            .unwrap_or_else(|| "Prometheus refused the request".into());
        return Err(QueryError::new(ErrorKind::Rejected, message));
    }
    let detail = value
        .as_ref()
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            String::from_utf8_lossy(&bytes[..bytes.len().min(200)])
                .trim()
                .to_owned()
        });
    Err(match status {
        401 | 403 => QueryError::refused(),
        404 => QueryError::new(
            ErrorKind::NotFound,
            if detail.is_empty() {
                "Not found".to_owned()
            } else {
                format!("Not found: {detail}")
            },
        ),
        408 | 504 => QueryError::new(ErrorKind::TimedOut, "The proxy timed out"),
        502 | 503 => QueryError::new(
            ErrorKind::Unavailable,
            if detail.is_empty() {
                "Prometheus isn't available".to_owned()
            } else {
                format!("Prometheus isn't available: {detail}")
            },
        ),
        code => bad_answer(format!("HTTP {code}: {detail}")),
    })
}

/// The most common `scrapeInterval` among active targets.
pub(super) fn scrape_interval(data: &Value) -> Option<f64> {
    let mut counts: Vec<(u64, usize)> = Vec::new();
    for target in data.get("activeTargets")?.as_array()? {
        let Some(seconds) = target
            .get("scrapeInterval")
            .and_then(Value::as_str)
            .and_then(|text| query::duration(text).ok())
        else {
            continue;
        };
        let millis = (seconds * 1000.).round() as u64;
        match counts.iter_mut().find(|(value, _)| *value == millis) {
            Some((_, count)) => *count += 1,
            None => counts.push((millis, 1)),
        }
    }
    // Most common, then the shortest on a tie.
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(millis, _)| millis as f64 / 1000.)
}
