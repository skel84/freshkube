//! What the framework carries for a silence or an Expire (#884, W2): the two
//! [`Change`](crate::write::Change) payloads, the scope the runner resolved
//! and the description a page hands over. A page never picks the tier, the
//! mode or the actor: the runner sets them from what it resolved.
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::silence::{Draft, Mode};
use super::writer::{AlertWriter, CreateRequest, WriteError};
use crate::monitoring::PrometheusService;
use crate::monitoring::alerts::{Silence, SilenceMatcher};
use crate::write::{
    AccessQuestion, ActionDescription, Change, Depends, Destination, ObjectChange, Observation,
    Refusal, Target, Tier,
};

/// How the open entry's alerts are told apart, as the runner resolved it
/// (never as a page says). Without one, no silence is sent.
#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    /// The cluster's own Alertmanager.
    Cluster,
    /// A shared one whose alerts carry `label`, `value` for this entry.
    /// `entries` are every entry that resolves to the same store, each with
    /// its cluster label value: a silence without this entry's exact matcher
    /// is held to the strictest of them.
    Central {
        label: String,
        value: String,
        entries: Vec<(String, Destination)>,
    },
    /// A shared store whose alerts carry no cluster label: no write at all.
    NoClusterLabel,
}

impl Scope {
    pub(crate) fn mode(&self) -> Option<Mode> {
        match self {
            Self::Cluster => Some(Mode::Cluster),
            Self::Central { label, value, .. } => Some(Mode::Central {
                label: label.clone(),
                value: value.clone(),
            }),
            Self::NoClusterLabel => None,
        }
    }
}

/// A silence to create. `draft.mode` and `actor` are overwritten by the
/// runner (`Runner::with_alert_scope`, `Who::actor`).
#[derive(Clone, PartialEq, Eq)]
pub struct SilenceChange {
    pub service: PrometheusService,
    pub draft: Draft,
    pub actor: String,
}

/// An Expire of one silence the app has read, as it read it.
#[derive(Clone, PartialEq, Eq)]
pub struct ExpireChange {
    pub service: PrometheusService,
    pub id: String,
    pub matchers: Vec<SilenceMatcher>,
    pub ends_at: Option<DateTime<Utc>>,
}

// Matcher values and the comment echo what the person typed: never printed.
impl std::fmt::Debug for SilenceChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SilenceChange({} matchers)", self.draft.matchers.len())
    }
}

impl std::fmt::Debug for ExpireChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ExpireChange({} matchers)", self.matchers.len())
    }
}

impl SilenceChange {
    /// What the audit records: matcher names, never values.
    pub(crate) fn field_paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = self
            .draft
            .matchers
            .iter()
            .map(|matcher| format!("matchers.{}", matcher.name))
            .collect();
        paths.extend(["comment".to_owned(), "endsAt".to_owned()]);
        paths.sort();
        paths.dedup();
        paths
    }

    /// The request for this silence, sent at `now`.
    pub(crate) fn request(&self, now: DateTime<Utc>) -> Result<CreateRequest, super::Refusal> {
        CreateRequest::new(&self.draft, &self.actor, now)
    }
}

impl ExpireChange {
    /// An Expire of `silence`, as it was read.
    pub fn of(service: PrometheusService, silence: &Silence) -> Self {
        Self {
            service,
            id: silence.id.clone(),
            matchers: silence.matchers.clone(),
            ends_at: silence.ends_at,
        }
    }

    pub(crate) fn field_paths(&self) -> Vec<String> {
        vec![format!("silence/{}", self.id)]
    }
}

/// The Service's identity as the person saw it: what a recreated Service
/// can't match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceSeen {
    pub uid: String,
    pub resource_version: String,
}

fn service_target(service: &PrometheusService, seen: &ServiceSeen) -> Target {
    Target {
        group: String::new(),
        version: "v1".into(),
        kind: "Service".into(),
        resource: "services".into(),
        namespace: Some(service.namespace.clone()),
        name: service.name.clone(),
        uid: seen.uid.clone(),
        resource_version: seen.resource_version.clone(),
        subresource: None,
    }
}

/// The part of an alert action that depends on the Service it goes through.
pub(crate) fn depends_on(target: &Target) -> Vec<Depends> {
    vec![Depends {
        target: target.clone(),
        verb: None,
        holds: vec![],
    }]
}

fn description(
    kind: &str,
    verb: &str,
    sentence: String,
    change: ObjectChange,
) -> ActionDescription {
    ActionDescription {
        kind: kind.to_owned(),
        // The runner classes a silence from what it mutes now; Expire is
        // reversible with no way back.
        tier: Tier::Reversible,
        verb: verb.to_owned(),
        sentence,
        changes: vec![change],
        destinations: vec![],
        no_confirmation: false,
        title: None,
        subjects: vec![],
        preview: vec![],
    }
}

/// The description of a silence. Its tier, mode, actor, sentence and preview
/// are set by `Runner::preflight`, not by the page.
pub fn describe_silence(
    service: &PrometheusService,
    seen: &ServiceSeen,
    draft: Draft,
) -> ActionDescription {
    let target = service_target(service, seen);
    let change = ObjectChange {
        depends: depends_on(&target),
        target,
        change: Change::Silence(Box::new(SilenceChange {
            service: service.clone(),
            draft,
            actor: String::new(),
        })),
        observe: Observation::SilenceActive,
        inverse: None,
    };
    description("silence", "Silence", "Silence an alert".into(), change)
}

/// The description of an Expire of `silence`, which must be active or pending.
pub fn describe_expire(
    service: &PrometheusService,
    seen: &ServiceSeen,
    silence: &Silence,
) -> ActionDescription {
    let target = service_target(service, seen);
    let change = ObjectChange {
        depends: depends_on(&target),
        target,
        change: Change::Expire(Box::new(ExpireChange::of(service.clone(), silence))),
        observe: Observation::SilenceExpired,
        inverse: None,
    };
    description(
        "silence-expire",
        "Expire",
        "Expire a silence".into(),
        change,
    )
}

/// The one request a `Silence` change sends, as a value for the runner.
pub(crate) fn created_reply(id: &str, request: &CreateRequest) -> Value {
    json!({
        "silenceID": id,
        // As Alertmanager stores it: to the millisecond.
        "endsAt": request.ends_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    })
}

/// The access questions a silence or Expire needs, per HTTP method of the
/// requests it makes: the reads (`get`) and the write (`create` for the
/// POST, `delete` for the DELETE). Empty for any other change.
pub(crate) fn questions(change: &Change) -> Vec<AccessQuestion> {
    use super::writer::access_question;
    use http::Method;
    let (service, write) = match change {
        Change::Silence(c) => (&c.service, Method::POST),
        Change::Expire(c) => (&c.service, Method::DELETE),
        _ => return vec![],
    };
    [Method::GET, write]
        .iter()
        .filter_map(|method| access_question(service, method))
        .collect()
}

fn refusal(error: WriteError) -> Refusal {
    match error {
        WriteError::Rejected {
            status: 403,
            message,
        } => Refusal::Denied(message),
        WriteError::Rejected { status, message } => Refusal::Rejected {
            code: status,
            message,
        },
        WriteError::Invalid(refusal) => Refusal::Rejected {
            code: 400,
            message: refusal.words(),
        },
        WriteError::Unreadable(message) => Refusal::Rejected { code: 400, message },
        // The request went and no usable answer came: never sent again.
        WriteError::Unknown(message) => Refusal::NoAnswer(message),
    }
}

/// Sends a silence or an Expire: one request through the Service proxy,
/// never retried. There is no dry run: with `dry_run` nothing is sent at all.
pub(crate) async fn send(
    client: &kube::Client,
    change: &Change,
    dry_run: bool,
) -> Result<Value, Refusal> {
    if dry_run {
        return Ok(Value::Null);
    }
    match change {
        Change::Silence(c) => {
            let request = c.request(Utc::now()).map_err(|r| Refusal::Rejected {
                code: 400,
                message: r.words(),
            })?;
            let writer = AlertWriter::new(client.clone(), c.service.clone());
            let id = writer.create(&request).await.map_err(refusal)?;
            Ok(created_reply(&id, &request))
        }
        Change::Expire(c) => {
            let writer = AlertWriter::new(client.clone(), c.service.clone());
            writer.expire(&c.id).await.map_err(refusal)?;
            Ok(json!({}))
        }
        _ => Ok(Value::Null),
    }
}
