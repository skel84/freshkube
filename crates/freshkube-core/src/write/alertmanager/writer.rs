//! [`AlertWriter`]: the only thing that sends a `POST` or a `DELETE` to an
//! Alertmanager, through the Kubernetes API's service proxy with the person's
//! own `kube::Client`. No stored credential, no URL: an Alertmanager reached
//! by address has no `services/proxy` review to gate on, so it gets no writer.
//!
//! It also holds the reads a silence needs: the preview of what a draft would
//! mute now (Alertmanager evaluates the matchers itself, `GET api/v2/alerts`
//! with `filter=`, bounded in bytes and in alerts) and the read-back of a
//! silence by id. Those are GETs, and the slice 1 alert `Client` stays
//! GET-only.
//!
//! One call is one request, never retried. A `POST` whose answer never came,
//! or came unusable, ends [`WriteError::Unknown`] and is never sent again: a
//! silence isn't idempotent, so a second send could make a duplicate. The
//! caller finds out with [`find_landed`] on the next read of the silences.
use std::{collections::BTreeSet, time::Duration};

use chrono::{DateTime, Utc};
use http::{Method, Request};
use http_body_util::BodyExt;
use kube::client::Body;
use serde_json::{Value, json};

use super::silence::{Draft, Mode, Refusal, validate};
use crate::monitoring::PrometheusService;
use crate::monitoring::alerts::{
    Alert, Silence, SilenceMatcher, parse_alertmanager_alerts, parse_silences,
};
use crate::write::AccessQuestion;

/// Alerts a preview counts. More than this is "at least this many", never
/// listed, and a silence that matches them is broad.
pub const MAX_PREVIEW_ALERTS: usize = 200;
/// The largest preview answer read. A bigger one is not parsed: it counts as
/// more than [`MAX_PREVIEW_ALERTS`].
pub const MAX_PREVIEW_BYTES: usize = 1024 * 1024;
/// Alerts a preview lists for the dialog to show.
pub const MAX_PREVIEW_LISTED: usize = 10;
/// The largest answer to a write or a read-back.
const MAX_SMALL_ANSWER: usize = 256 * 1024;
/// How long one request may take.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// The prefix of every silence's `createdBy`.
pub const CREATED_BY_PREFIX: &str = "freshkube";
const MAX_CREATED_BY: usize = 128;
const MAX_ID: usize = 64;

/// How many alerts a draft mutes now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Muted {
    Exactly(usize),
    /// More than the bound, or an answer too large to count: not the number.
    AtLeast(usize),
}

impl Muted {
    pub fn is_exact(self) -> bool {
        matches!(self, Self::Exactly(_))
    }
}

/// One alert a draft would mute, as the dialog lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewedAlert {
    pub name: String,
    pub fingerprint: String,
    pub labels: std::collections::BTreeMap<String, String>,
    /// The value of the central cluster label, when it has one.
    pub cluster: Option<String>,
}

/// What a draft would mute now, as Alertmanager evaluated its matchers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub muted: Muted,
    /// The first [`MAX_PREVIEW_LISTED`], by name then fingerprint.
    pub alerts: Vec<PreviewedAlert>,
    /// Every fingerprint counted (up to [`MAX_PREVIEW_ALERTS`]), sorted: what
    /// "the same preview" is compared by.
    pub fingerprints: Vec<String>,
    /// The distinct values of the cluster label among the counted alerts, in
    /// central mode.
    pub clusters: Vec<String>,
    /// How many of all the counted alerts (not only the listed ones) are not
    /// this entry's: their cluster label is another value, or missing. Zero
    /// outside central mode. When `muted` isn't exact it counts only what was
    /// counted.
    pub other_clusters: usize,
}

impl Preview {
    /// A preview of nothing, for tests and an empty answer.
    pub fn none() -> Self {
        Self {
            muted: Muted::Exactly(0),
            alerts: Vec::new(),
            fingerprints: Vec::new(),
            clusters: Vec::new(),
            other_clusters: 0,
        }
    }

    /// Whether `other` muted the same alerts: the same set, counted the same
    /// way. A preview that wasn't counted exactly is never the same as
    /// another: what it muted is not known.
    pub fn same_as(&self, other: &Self) -> bool {
        self.muted.is_exact()
            && other.muted.is_exact()
            && self.muted == other.muted
            && self.fingerprints == other.fingerprints
    }
}

/// The silence to create, with the moment it was made at, so the same request
/// can be looked for afterwards.
#[derive(Clone, PartialEq, Eq)]
pub struct CreateRequest {
    pub matchers: Vec<SilenceMatcher>,
    pub comment: String,
    pub created_by: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

// Never prints the matchers or the comment: they echo what the person typed.
impl std::fmt::Debug for CreateRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CreateRequest({} matchers)", self.matchers.len())
    }
}

impl CreateRequest {
    /// The request for a draft sent at `now` by `actor`: it ends `duration`
    /// after `now`, never at a time a page chose. Refuses what no class can
    /// send.
    pub fn new(draft: &Draft, actor: &str, now: DateTime<Utc>) -> Result<Self, Refusal> {
        validate(draft)?;
        let duration =
            chrono::Duration::from_std(draft.duration).map_err(|_| Refusal::DurationTooLong)?;
        let mut created_by = format!("{CREATED_BY_PREFIX}: {}", actor.trim());
        created_by = created_by.chars().take(MAX_CREATED_BY).collect();
        Ok(Self {
            matchers: draft.matchers.clone(),
            comment: draft.comment.trim().to_owned(),
            created_by,
            starts_at: now,
            ends_at: now + duration,
        })
    }

    fn body(&self) -> Value {
        json!({
            "matchers": self.matchers.iter().map(|matcher| json!({
                "name": matcher.name,
                "value": matcher.value,
                "isRegex": matcher.regex,
                "isEqual": matcher.equal,
            })).collect::<Vec<_>>(),
            "startsAt": self.starts_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "endsAt": self.ends_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "createdBy": self.created_by,
            "comment": self.comment,
        })
    }
}

/// A silence looked up by id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadBack {
    Found(Silence),
    /// Alertmanager has no silence with that id.
    NotFound,
}

/// Why a write or one of its reads didn't succeed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteError {
    /// Refused before any request.
    Invalid(Refusal),
    /// A read failed. Nothing was sent.
    Unreadable(String),
    /// The server said no, in so many words. Nothing changed.
    Rejected { status: u16, message: String },
    /// The request went and no usable answer came back: the silence may or
    /// may not exist. Never sent again.
    Unknown(String),
}

impl WriteError {
    pub fn words(&self) -> String {
        match self {
            Self::Invalid(refusal) => refusal.words(),
            Self::Unreadable(message) => message.clone(),
            Self::Rejected { status, message } => {
                format!("Alertmanager refused it (HTTP {status}): {message}")
            }
            Self::Unknown(message) => format!(
                "No answer came back ({message}). The silence may exist: look in Silences. \
                 It isn't sent again."
            ),
        }
    }
}

/// Sends silences and Expires to one Alertmanager Service.
#[derive(Clone)]
pub struct AlertWriter {
    client: kube::Client,
    service: PrometheusService,
    timeout: Duration,
}

/// The access question for one HTTP method on `services/proxy` of the
/// Service, named as the API server authorizes it
/// ([`PrometheusService::proxy_name`]): the proxy's own check maps a `POST` to `create`, a `DELETE` to
/// `delete` and a `GET` to `get`. Any other method is never sent, so it has
/// no question.
pub fn access_question(service: &PrometheusService, method: &Method) -> Option<AccessQuestion> {
    let verb = match *method {
        Method::POST => "create",
        Method::DELETE => "delete",
        Method::GET => "get",
        _ => return None,
    };
    Some(AccessQuestion {
        verb,
        group: String::new(),
        resource: "services".into(),
        subresource: Some("proxy".into()),
        namespace: Some(service.namespace.clone()),
        // The proxy request names the Service in its URL, so the authorizer
        // sees the name for every verb, a POST too: it is the URL segment
        // `[https:]name:port`, not the Service's own name.
        name: Some(service.proxy_name()),
    })
}

impl AlertWriter {
    pub fn new(client: kube::Client, service: PrometheusService) -> Self {
        Self {
            client,
            service,
            timeout: REQUEST_TIMEOUT,
        }
    }

    /// How long each request may take; [`REQUEST_TIMEOUT`] by default.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn service(&self) -> &PrometheusService {
        &self.service
    }

    /// What a silence needs before it is offered: the preview and the
    /// read-back are GETs, the silence is a POST.
    pub fn silence_questions(&self) -> Vec<AccessQuestion> {
        self.questions(&Method::POST)
    }

    /// What an Expire needs: the reads, and a DELETE, which is `delete`, not
    /// `create`.
    pub fn expire_questions(&self) -> Vec<AccessQuestion> {
        self.questions(&Method::DELETE)
    }

    fn questions(&self, write: &Method) -> Vec<AccessQuestion> {
        [Method::GET, write.clone()]
            .iter()
            .filter_map(|method| access_question(&self.service, method))
            .collect()
    }

    /// What `draft` mutes now, read from Alertmanager with the draft's own
    /// matchers. Bounded: see [`MAX_PREVIEW_ALERTS`] and
    /// [`MAX_PREVIEW_BYTES`]. A read that fails is an error, never an empty
    /// preview.
    pub async fn preview(&self, draft: &Draft) -> Result<Preview, WriteError> {
        validate(draft).map_err(WriteError::Invalid)?;
        // In a block of its own: the serializer isn't `Sync`, so it must not
        // be held across an await, and the runner's futures run on a runtime.
        let query = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            for matcher in &draft.matchers {
                query.append_pair("filter", &filter_text(matcher));
            }
            query
                .append_pair("active", "true")
                .append_pair("unprocessed", "true")
                .append_pair("silenced", "true")
                .append_pair("inhibited", "true")
                .finish()
        };
        let target = format!("api/v2/alerts?{query}");
        let cluster = match &draft.mode {
            Mode::Central { label, value } => Some((label.as_str(), value.as_str())),
            Mode::Cluster => None,
        };
        match self
            .send(Method::GET, &target, None, MAX_PREVIEW_BYTES)
            .await
        {
            Ok((status, _)) if !(200..300).contains(&status) => {
                Err(WriteError::Unreadable(read_failure(status)))
            }
            Ok((_, bytes)) => {
                let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
                    WriteError::Unreadable("Alertmanager's alerts aren't JSON".into())
                })?;
                let alerts = parse_alertmanager_alerts(&value).map_err(WriteError::Unreadable)?;
                Ok(preview_of(alerts, cluster))
            }
            Err(Transport::TooLarge) => Ok(Preview {
                muted: Muted::AtLeast(MAX_PREVIEW_ALERTS),
                ..Preview::none()
            }),
            Err(error) => Err(WriteError::Unreadable(error.words())),
        }
    }

    /// One silence by id.
    pub async fn read_back(&self, id: &str) -> Result<ReadBack, WriteError> {
        let id = check_id(id)?;
        let target = format!("api/v2/silence/{id}");
        let (status, bytes) = self
            .send(Method::GET, &target, None, MAX_SMALL_ANSWER)
            .await
            .map_err(|error| WriteError::Unreadable(error.words()))?;
        match status {
            404 => Ok(ReadBack::NotFound),
            200..=299 => {
                let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
                    WriteError::Unreadable("Alertmanager's silence isn't JSON".into())
                })?;
                let mut silences =
                    parse_silences(&Value::Array(vec![value])).map_err(WriteError::Unreadable)?;
                match silences.pop() {
                    Some(silence) if silence.id == id => Ok(ReadBack::Found(silence)),
                    _ => Err(WriteError::Unreadable(
                        "Alertmanager answered with another silence".into(),
                    )),
                }
            }
            status => Err(WriteError::Unreadable(read_failure(status))),
        }
    }

    /// `POST api/v2/silences`: the id of the silence it made. One request,
    /// never sent again: no usable answer is [`WriteError::Unknown`].
    pub async fn create(&self, request: &CreateRequest) -> Result<String, WriteError> {
        validate_request(request)?;
        let body = request.body().to_string().into_bytes();
        let (status, bytes) = self
            .send(
                Method::POST,
                "api/v2/silences",
                Some(body),
                MAX_SMALL_ANSWER,
            )
            .await
            .map_err(|error| WriteError::Unknown(error.words()))?;
        if (200..300).contains(&status) {
            let id = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|value| value.get("silenceID")?.as_str().map(str::to_owned))
                .filter(|id| check_id(id).is_ok());
            return id.ok_or_else(|| WriteError::Unknown("the answer named no silence".into()));
        }
        Err(failure(status, &bytes))
    }

    /// `DELETE api/v2/silence/{id}`. One request, never sent again.
    pub async fn expire(&self, id: &str) -> Result<(), WriteError> {
        let id = check_id(id)?;
        let target = format!("api/v2/silence/{id}");
        let (status, bytes) = self
            .send(Method::DELETE, &target, None, MAX_SMALL_ANSWER)
            .await
            .map_err(|error| WriteError::Unknown(error.words()))?;
        if (200..300).contains(&status) {
            return Ok(());
        }
        Err(failure(status, &bytes))
    }

    /// One request through the proxy: the status and the body, bounded by
    /// the timeout and `limit`. Errors name neither the server nor a token.
    async fn send(
        &self,
        method: Method,
        target: &str,
        body: Option<Vec<u8>>,
        limit: usize,
    ) -> Result<(u16, Vec<u8>), Transport> {
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("{}{target}", self.service.proxy_base()))
            .header(http::header::ACCEPT, "application/json");
        let body = match body {
            Some(bytes) => {
                builder = builder.header(http::header::CONTENT_TYPE, "application/json");
                Body::from(bytes)
            }
            None => Body::empty(),
        };
        let request = builder.body(body).map_err(|_| Transport::Build)?;
        let answer = async {
            let response = self
                .client
                .send(request)
                .await
                .map_err(|error| match error {
                    kube::Error::Auth(_) => Transport::Unauthorized,
                    _ => Transport::Unreachable,
                })?;
            let status = response.status().as_u16();
            let mut body = response.into_body();
            let mut bytes = Vec::new();
            while let Some(frame) = body.frame().await {
                let frame = frame.map_err(|_| Transport::CutOff)?;
                if let Some(data) = frame.data_ref() {
                    if bytes.len() + data.len() > limit {
                        // The status comes first: an oversized error answer
                        // is that error, not a too-large success. Only the
                        // body of a success is held to the bound.
                        if (200..300).contains(&status) {
                            return Err(Transport::TooLarge);
                        }
                        break;
                    }
                    bytes.extend_from_slice(data);
                }
            }
            Ok((status, bytes))
        };
        tokio::time::timeout(self.timeout, answer)
            .await
            .map_err(|_| Transport::TimedOut)?
    }
}

/// How a request failed before a status came back.
enum Transport {
    Build,
    Unauthorized,
    Unreachable,
    CutOff,
    TooLarge,
    TimedOut,
}

impl Transport {
    fn words(&self) -> String {
        match self {
            Self::Build => "the request couldn't be built",
            Self::Unauthorized => "the context's credentials were refused",
            Self::Unreachable => "couldn't reach the Kubernetes API",
            Self::CutOff => "the answer was cut off",
            Self::TooLarge => "the answer was too large",
            Self::TimedOut => "Alertmanager didn't answer in time",
        }
        .to_owned()
    }
}

fn read_failure(status: u16) -> String {
    match status {
        401 => "The context's credentials were refused".into(),
        403 => "The cluster refused the read through the Service proxy".into(),
        404 => "Alertmanager wasn't found at that Service".into(),
        408 | 504 => "The proxy timed out".into(),
        502 | 503 => "Alertmanager isn't available".into(),
        code => format!("HTTP {code}"),
    }
}

/// A 4xx is the server saying no; anything else that isn't a success is an
/// answer that doesn't say whether the request was acted on (a proxy's 502
/// or 504, an Alertmanager 500), so it's [`WriteError::Unknown`].
fn failure(status: u16, bytes: &[u8]) -> WriteError {
    let detail = String::from_utf8_lossy(bytes);
    let detail: String = serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| detail.trim().to_owned())
        .chars()
        .take(300)
        .collect();
    let detail = crate::write::scrub(&detail);
    if (400..500).contains(&status) {
        let message = match status {
            403 => "the cluster refused the request through the Service proxy".to_owned(),
            401 => "the context's credentials were refused".to_owned(),
            _ if detail.is_empty() => "no reason given".to_owned(),
            _ => detail,
        };
        WriteError::Rejected { status, message }
    } else {
        WriteError::Unknown(format!("HTTP {status}"))
    }
}

/// An id Alertmanager makes (a UUID), and nothing that could change the path.
fn check_id(id: &str) -> Result<&str, WriteError> {
    let ok = !id.is_empty()
        && id.len() <= MAX_ID
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if ok {
        Ok(id)
    } else {
        Err(WriteError::Unreadable("that isn't a silence id".into()))
    }
}

fn validate_request(request: &CreateRequest) -> Result<(), WriteError> {
    let draft = Draft {
        matchers: request.matchers.clone(),
        comment: request.comment.clone(),
        duration: super::silence::MIN_DURATION,
        mode: Mode::Cluster,
    };
    validate(&draft).map_err(WriteError::Invalid)?;
    if request.ends_at <= request.starts_at {
        return Err(WriteError::Invalid(Refusal::DurationTooShort));
    }
    Ok(())
}

/// A matcher as Alertmanager's `filter` parameter reads it.
fn filter_text(matcher: &SilenceMatcher) -> String {
    let operator = match (matcher.equal, matcher.regex) {
        (true, false) => "=",
        (false, false) => "!=",
        (true, true) => "=~",
        (false, true) => "!~",
    };
    let value = matcher.value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("{}{operator}\"{value}\"", matcher.name)
}

fn preview_of(mut alerts: Vec<Alert>, cluster: Option<(&str, &str)>) -> Preview {
    alerts.sort_by(|a, b| (&a.name, &a.fingerprint).cmp(&(&b.name, &b.fingerprint)));
    alerts.dedup_by(|a, b| !a.fingerprint.is_empty() && a.fingerprint == b.fingerprint);
    let total = alerts.len();
    let counted = &alerts[..total.min(MAX_PREVIEW_ALERTS)];
    let cluster_of =
        |alert: &Alert| cluster.and_then(|(label, _)| alert.label(label).map(str::to_owned));
    let other_clusters = cluster.map_or(0, |(label, own)| {
        counted
            .iter()
            .filter(|alert| alert.label(label) != Some(own))
            .count()
    });
    let mut fingerprints: Vec<String> = counted
        .iter()
        .map(|alert| alert.fingerprint.clone())
        .collect();
    fingerprints.sort();
    let clusters: BTreeSet<String> = counted.iter().filter_map(cluster_of).collect();
    Preview {
        muted: if total > MAX_PREVIEW_ALERTS {
            Muted::AtLeast(MAX_PREVIEW_ALERTS)
        } else {
            Muted::Exactly(total)
        },
        alerts: counted
            .iter()
            .take(MAX_PREVIEW_LISTED)
            .map(|alert| PreviewedAlert {
                name: alert.name.clone(),
                fingerprint: alert.fingerprint.clone(),
                labels: alert.labels.clone(),
                cluster: cluster_of(alert),
            })
            .collect(),
        fingerprints,
        clusters: clusters.into_iter().collect(),
        other_clusters,
    }
}

/// The silence a [`CreateRequest`] made, found among the silences read
/// afterwards: by who created it and its comment, since a send with no answer
/// has no id. The newest active or pending one that also has the same
/// matchers; `None` when it isn't there.
pub fn find_landed<'a>(silences: &'a [Silence], request: &CreateRequest) -> Option<&'a Silence> {
    use crate::monitoring::alerts::SilenceState;
    silences
        .iter()
        .filter(|silence| {
            silence.state != SilenceState::Expired
                && silence.created_by == request.created_by
                && silence.comment == request.comment
                && same_matchers(&silence.matchers, &request.matchers)
        })
        .max_by_key(|silence| silence.starts_at)
}

/// Whether two matcher lists hold the same matchers, in any order:
/// Alertmanager may list them in another order than they were sent.
pub(crate) fn same_matchers(a: &[SilenceMatcher], b: &[SilenceMatcher]) -> bool {
    let key = |matchers: &[SilenceMatcher]| {
        let mut keys: Vec<_> = matchers
            .iter()
            .map(|m| (m.name.clone(), m.value.clone(), m.regex, m.equal))
            .collect();
        keys.sort();
        keys
    };
    key(a) == key(b)
}
