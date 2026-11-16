//! A fake Alertmanager behind a fake service proxy, for the writer's tests
//! (behind core's `testing` feature, or in core's own tests). It answers the
//! v2 routes the writer uses through the proxy path of
//! [`service`], evaluates a request's `filter=` the way Alertmanager does for
//! the operators the tests use, keeps the silences it creates, answers access
//! reviews per verb, and records every request through
//! [`Recorder`](crate::fake_api::Recorder) so a test asserts on the list. A
//! route it doesn't know answers a 500, as the other fakes do.
//!
//! Regex matchers are evaluated in a small way: alternatives with `|`, and a
//! `.*` at either end. That is all the tests write.
use std::{
    collections::BTreeMap,
    convert::Infallible,
    sync::{Arc, Mutex},
};

use http::{Method, Request};
use kube::client::Body;
use serde_json::{Value, json};

use super::writer::AlertWriter;
use crate::fake_api::{self, Recorded, Recorder};
use crate::monitoring::PrometheusService;

pub const NAMESPACE: &str = "monitoring";
pub const NAME: &str = "alertmanager-operated";
pub const PORT: u16 = 9093;
/// The proxy path of the fake's API, ending in a slash.
pub const BASE: &str = "/api/v1/namespaces/monitoring/services/alertmanager-operated:9093/proxy/";

const SERVICE_PATH: &str = "/api/v1/namespaces/monitoring/services/alertmanager-operated";

/// The UID the fake's Service has, as a page that read it saw it.
pub fn service_uid(generation: u32) -> String {
    format!("svc-uid-{generation}")
}

/// The Service the fake stands behind.
pub fn service() -> PrometheusService {
    PrometheusService::new(NAMESPACE, NAME, PORT)
}

/// What a write or a read gets instead of its normal answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The status and body.
    Status(u16, String),
    /// The request is taken and never answered. `applied` says whether the
    /// fake did the write first, as a server that acted and then went quiet.
    Silent { applied: bool },
}

#[derive(Default)]
struct State {
    alerts: Vec<(BTreeMap<String, String>, String)>,
    silences: Vec<Value>,
    next_id: u32,
    /// Times the Service was deleted and created again: its UID changes.
    service_generation: u32,
    denied_verbs: Vec<String>,
    post: Option<Answer>,
    delete: Option<Answer>,
    get: Option<Answer>,
}

/// The fake Alertmanager and the client that reaches it.
#[derive(Clone)]
pub struct Fake {
    state: Arc<Mutex<State>>,
    pub recorder: Recorder,
    pub client: kube::Client,
}

impl Default for Fake {
    fn default() -> Self {
        Self::new()
    }
}

impl Fake {
    pub fn new() -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let recorder = Recorder::default();
        let (log, shared) = (recorder.clone(), state.clone());
        let service = tower::service_fn(move |request: Request<Body>| {
            let (log, shared) = (log.clone(), shared.clone());
            async move {
                let (parts, body) = request.into_parts();
                let bytes = http_body_util::BodyExt::collect(body)
                    .await
                    .map(|collected| collected.to_bytes())
                    .unwrap_or_default();
                let request = Request::from_parts(parts, ());
                let recorded =
                    log.record_with_body(&request, String::from_utf8_lossy(&bytes).into_owned());
                let reply = shared.lock().unwrap().route(&recorded);
                match reply {
                    Reply::Answer(status, body) => {
                        Ok::<_, Infallible>(fake_api::response((status, body)))
                    }
                    Reply::Silent => {
                        std::future::pending::<()>().await;
                        unreachable!("a silent request is never answered")
                    }
                }
            }
        });
        Self {
            state,
            recorder,
            client: kube::Client::new(service, "default"),
        }
    }

    /// A writer for the fake's Service.
    pub fn writer(&self) -> AlertWriter {
        AlertWriter::new(self.client.clone(), service())
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    /// An alert the fake holds, with a fingerprint made from its labels.
    pub fn with_alert(self, labels: &[(&str, &str)]) -> Self {
        self.add_alert(labels);
        self
    }

    pub fn add_alert(&self, labels: &[(&str, &str)]) {
        let labels: BTreeMap<String, String> = labels
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        let mut state = self.state();
        let fingerprint = format!("fp{:04}", state.alerts.len());
        state.alerts.push((labels, fingerprint));
    }

    /// `count` alerts of one rule on different pods, plus `extra` labels.
    pub fn add_many(&self, alertname: &str, count: usize, extra: &[(&str, &str)]) {
        for index in 0..count {
            let pod = format!("pod-{index}");
            let mut labels = vec![("alertname", alertname), ("pod", pod.as_str())];
            labels.extend_from_slice(extra);
            self.add_alert(&labels);
        }
    }

    /// The Service is deleted and created again: it has another UID.
    pub fn recreate_service(&self) {
        self.state().service_generation += 1;
    }

    /// The access review refuses this verb on everything.
    pub fn deny(&self, verb: &str) {
        self.state().denied_verbs.push(verb.to_owned());
    }

    /// Every `POST` gets this instead of its normal answer.
    pub fn post_answers(&self, answer: Answer) {
        self.state().post = Some(answer);
    }

    pub fn delete_answers(&self, answer: Answer) {
        self.state().delete = Some(answer);
    }

    /// Every `GET` of the Alertmanager API gets this.
    pub fn get_answers(&self, answer: Answer) {
        self.state().get = Some(answer);
    }

    /// A silence the fake holds already, as Alertmanager would list it.
    pub fn add_silence(&self, id: &str, state: &str, created_by: &str, comment: &str) {
        self.state().silences.push(json!({
            "id": id,
            "status": {"state": state},
            "matchers": [{"name": "alertname", "value": "HighLatency", "isRegex": false, "isEqual": true}],
            "createdBy": created_by,
            "comment": comment,
            "startsAt": "2026-01-01T10:00:00.000Z",
            "endsAt": "2026-01-01T12:00:00.000Z",
        }));
    }

    /// An active silence of one cluster's alert, as a shared store holds it.
    pub fn add_cluster_silence(&self, id: &str, cluster: &str) {
        self.state().silences.push(json!({
            "id": id,
            "status": {"state": "active"},
            "matchers": [
                {"name": "cluster", "value": cluster, "isRegex": false, "isEqual": true},
                {"name": "alertname", "value": "HighLatency", "isRegex": false, "isEqual": true},
            ],
            "createdBy": "someone",
            "comment": "planned",
            "startsAt": "2026-01-01T10:00:00.000Z",
            "endsAt": "2026-01-01T12:00:00.000Z",
        }));
    }

    /// The silences the fake holds now.
    pub fn silences(&self) -> Vec<Value> {
        self.state().silences.clone()
    }

    /// The state the fake holds a silence in.
    pub fn silence_state(&self, id: &str) -> Option<String> {
        self.state()
            .silences
            .iter()
            .find(|silence| silence["id"] == id)
            .and_then(|silence| silence["status"]["state"].as_str().map(str::to_owned))
    }

    /// Every request the fake saw that wasn't a `GET` or an access review.
    pub fn writes(&self) -> Vec<String> {
        self.recorder
            .requests()
            .into_iter()
            .filter(|request| {
                request.method != Method::GET && !request.path.contains("selfsubjectaccessreviews")
            })
            .map(|request| request.line())
            .collect()
    }
}

enum Reply {
    Answer(u16, String),
    Silent,
}

fn answer((status, body): (u16, String)) -> Reply {
    Reply::Answer(status, body)
}

impl State {
    fn route(&mut self, request: &Recorded) -> Reply {
        if request.path == "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews"
            && request.method == Method::POST
        {
            return answer(self.review(&request.body));
        }
        if request.method == Method::GET && request.path == SERVICE_PATH {
            return answer(fake_api::json(
                200,
                json!({"apiVersion": "v1", "kind": "Service",
                    "metadata": {"name": NAME, "namespace": NAMESPACE,
                        "uid": service_uid(self.service_generation),
                        "resourceVersion": "1"}}),
            ));
        }
        let Some(api) = request.path.strip_prefix(BASE) else {
            return answer(fake_api::unknown(request));
        };
        let silence = api.strip_prefix("api/v2/silence/");
        match (&request.method, api, silence) {
            (&Method::GET, "api/v2/alerts", _) => {
                self.read(|state| (200, state.alerts_json(&request.query).to_string()))
            }
            (&Method::GET, "api/v2/silences", _) => {
                self.read(|state| (200, Value::Array(state.silences.clone()).to_string()))
            }
            (&Method::GET, _, Some(id)) => {
                self.read(
                    |state| match state.silences.iter().find(|silence| silence["id"] == id) {
                        Some(silence) => (200, silence.to_string()),
                        None => (404, json!({"message": "silence not found"}).to_string()),
                    },
                )
            }
            (&Method::POST, "api/v2/silences", _) => self.create(&request.body),
            (&Method::DELETE, _, Some(id)) => self.expire(id),
            _ => answer(fake_api::unknown(request)),
        }
    }

    fn read(&mut self, normal: impl Fn(&Self) -> (u16, String)) -> Reply {
        match &self.get {
            Some(Answer::Status(status, body)) => Reply::Answer(*status, body.clone()),
            Some(Answer::Silent { .. }) => Reply::Silent,
            None => answer(normal(self)),
        }
    }

    fn review(&self, body: &str) -> (u16, String) {
        let value: Value = serde_json::from_str(body).unwrap_or_default();
        let verb = value["spec"]["resourceAttributes"]["verb"]
            .as_str()
            .unwrap_or_default();
        let allowed = !self.denied_verbs.iter().any(|denied| denied == verb);
        fake_api::json(
            201,
            json!({"apiVersion": "authorization.k8s.io/v1", "kind": "SelfSubjectAccessReview",
                "spec": value["spec"],
                "status": {"allowed": allowed, "reason": if allowed { "" } else { "no RBAC rule allows it" }}}),
        )
    }

    fn create(&mut self, body: &str) -> Reply {
        let override_with = self.post.clone();
        let applied = match &override_with {
            None => true,
            Some(Answer::Silent { applied }) => *applied,
            Some(Answer::Status(..)) => false,
        };
        let value: Value = serde_json::from_str(body).unwrap_or_default();
        let id = format!("sil-{:04}", self.next_id + 1);
        if applied && value["matchers"].is_array() {
            self.next_id += 1;
            let mut silence = value.clone();
            silence["id"] = json!(id);
            silence["status"] = json!({"state": "active"});
            self.silences.push(silence);
        }
        match override_with {
            Some(Answer::Status(status, body)) => Reply::Answer(status, body),
            Some(Answer::Silent { .. }) => Reply::Silent,
            None if value["matchers"].is_array() => {
                answer(fake_api::json(200, json!({"silenceID": id})))
            }
            None => answer(fake_api::json(400, json!({"message": "bad silence"}))),
        }
    }

    fn expire(&mut self, id: &str) -> Reply {
        let override_with = self.delete.clone();
        let applied = match &override_with {
            None => true,
            Some(Answer::Silent { applied }) => *applied,
            Some(Answer::Status(..)) => false,
        };
        let found = self.silences.iter_mut().find(|silence| silence["id"] == id);
        let found = match found {
            Some(silence) => {
                if applied {
                    silence["status"] = json!({"state": "expired"});
                }
                true
            }
            None => false,
        };
        match override_with {
            Some(Answer::Status(status, body)) => Reply::Answer(status, body),
            Some(Answer::Silent { .. }) => Reply::Silent,
            None if found => answer(fake_api::json(200, "{}")),
            None => answer(fake_api::json(404, json!({"message": "silence not found"}))),
        }
    }

    /// Alertmanager's `GET api/v2/alerts`, evaluating every `filter`.
    fn alerts_json(&self, query: &str) -> Value {
        let filters: Vec<String> = url::form_urlencoded::parse(query.as_bytes())
            .filter(|(name, _)| name == "filter")
            .map(|(_, value)| value.into_owned())
            .collect();
        let alerts = self
            .alerts
            .iter()
            .filter(|(labels, _)| filters.iter().all(|filter| matches_filter(labels, filter)))
            .map(|(labels, fingerprint)| {
                json!({
                    "labels": labels,
                    "annotations": {},
                    "fingerprint": fingerprint,
                    "startsAt": "2026-01-01T09:00:00.000Z",
                    "status": {"state": "active", "silencedBy": [], "inhibitedBy": []},
                })
            })
            .collect::<Vec<_>>();
        Value::Array(alerts)
    }
}

/// Whether the labels satisfy one `name<op>"value"` filter.
fn matches_filter(labels: &BTreeMap<String, String>, filter: &str) -> bool {
    let name_end = filter
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(filter.len());
    let (name, rest) = filter.split_at(name_end);
    let Some((operator, rest)) = ["=~", "!~", "!=", "="]
        .iter()
        .find_map(|operator| rest.strip_prefix(operator).map(|rest| (*operator, rest)))
    else {
        return false;
    };
    let Some(quoted) = rest
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return false;
    };
    let mut value = String::new();
    let mut chars = quoted.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                value.push(next);
            }
        } else {
            value.push(c);
        }
    }
    let actual = labels.get(name).map(String::as_str).unwrap_or("");
    match operator {
        "=" => actual == value,
        "!=" => actual != value,
        "=~" => regex_matches(&value, actual),
        _ => !regex_matches(&value, actual),
    }
}

fn regex_matches(pattern: &str, actual: &str) -> bool {
    pattern.split('|').any(|alternative| {
        match (
            alternative.strip_prefix(".*"),
            alternative.strip_suffix(".*"),
        ) {
            (Some(rest), _) if rest.strip_suffix(".*").is_some() => {
                actual.contains(rest.strip_suffix(".*").unwrap())
            }
            (Some(rest), _) => actual.ends_with(rest),
            (None, Some(rest)) => actual.starts_with(rest),
            (None, None) => actual == alternative,
        }
    })
}
