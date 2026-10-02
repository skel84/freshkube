//! One object's full representation, read on demand, and a Secret's values,
//! read one key at a time when the user asks for it.

use std::fmt;

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use http::{Request, header};
use kube::Client;
use serde_yaml::Value;

use super::failure::{Failure, FailureKind};
use super::forward::{DeclaredPort, declared_ports};
use super::kinds::ResourceKind;
use super::pod_logs::{PodContainers, pod_containers};

/// kubectl apply stores the whole applied object here, a Secret's data
/// included.
const LAST_APPLIED: &str = "kubectl.kubernetes.io/last-applied-configuration";

/// An object as the server returned it, ready to show.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectDocument {
    pub kind: String,
    pub api_version: String,
    pub name: String,
    pub namespace: Option<String>,
    pub uid: String,
    pub resource_version: String,
    /// In the server's field order, without `metadata.managedFields`. A
    /// Secret's values are hidden; see [`reveal_secret_value`].
    pub yaml: String,
    pub overview: Overview,
}

/// What every kind has in common, for a summary above the raw document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overview {
    pub created: Option<DateTime<Utc>>,
    /// Set once deletion was requested; finalizers may still hold the object.
    pub deleting: Option<DateTime<Utc>>,
    pub generation: Option<i64>,
    pub observed_generation: Option<i64>,
    pub labels: Vec<(String, String)>,
    pub annotations: Vec<(String, String)>,
    pub owners: Vec<Owner>,
    pub finalizers: Vec<String>,
    pub conditions: Vec<Condition>,
    /// Present for a Secret: its type and the size of each value.
    pub secret: Option<SecretSummary>,
    /// Present for a Pod: its containers, for choosing a log.
    pub pod: Option<PodContainers>,
    /// Present for a kind that can be forwarded: the ports it declares.
    pub ports: Option<Vec<DeclaredPort>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Owner {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub uid: String,
    pub controller: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Condition {
    /// The condition's `type`, such as `Ready` or `Available`.
    pub kind: String,
    pub status: String,
    pub reason: String,
    pub message: String,
    pub changed: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretSummary {
    pub secret_type: String,
    pub keys: Vec<SecretKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretKey {
    pub name: String,
    pub bytes: usize,
}

/// One revealed Secret value. Its `Debug` output never includes the value.
#[derive(Clone, PartialEq, Eq)]
pub enum SecretValue {
    Text(String),
    /// Not UTF-8; only its size is kept.
    Binary(usize),
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretValue::Text(text) => write!(f, "Text(<{} bytes>)", text.len()),
            SecretValue::Binary(bytes) => write!(f, "Binary({bytes})"),
        }
    }
}

pub(crate) fn json_get(path: String) -> Result<Request<Vec<u8>>, Failure> {
    Request::get(path)
        .header(header::ACCEPT, "application/json")
        .body(Vec::new())
        .map_err(|error| Failure::new(FailureKind::Other, error.to_string()))
}

/// Reads one object in full. A Secret's values are hidden before the
/// document leaves this function.
pub async fn get_object(
    client: &Client,
    kind: &ResourceKind,
    namespace: Option<&str>,
    name: &str,
) -> Result<ObjectDocument, Failure> {
    // A YAML value keeps the server's field order, which JSON maps don't.
    let object: Value = client
        .request(json_get(kind.object_path(namespace, name))?)
        .await
        .map_err(Failure::from_kube)?;
    document(kind, object)
}

/// Builds a document from an object written out as YAML, as example data
/// does, exactly as if the server had returned it.
pub fn object_from_yaml(kind: &ResourceKind, yaml: &str) -> Result<ObjectDocument, Failure> {
    let object: Value = serde_yaml::from_str(yaml)
        .map_err(|error| Failure::new(FailureKind::Other, error.to_string()))?;
    document(kind, object)
}

pub(crate) fn document(kind: &ResourceKind, mut object: Value) -> Result<ObjectDocument, Failure> {
    if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_mapping_mut) {
        metadata.remove("managedFields");
    }
    let secret = kind.is_secret().then(|| hide_secret(&mut object));
    let mut overview = overview(&object);
    overview.secret = secret;
    overview.pod = kind.is_pod().then(|| pod_containers(&object));
    overview.ports = declared_ports(kind, &object);
    let yaml = serde_yaml::to_string(&object)
        .map_err(|error| Failure::new(FailureKind::Other, error.to_string()))?;
    let metadata = object.get("metadata");
    Ok(ObjectDocument {
        kind: text(object.get("kind")),
        api_version: text(object.get("apiVersion")),
        name: text(metadata.and_then(|metadata| metadata.get("name"))),
        namespace: metadata
            .and_then(|metadata| metadata.get("namespace"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        uid: text(metadata.and_then(|metadata| metadata.get("uid"))),
        resource_version: text(metadata.and_then(|metadata| metadata.get("resourceVersion"))),
        yaml,
        overview,
    })
}

/// Reads one value of a Secret, checking that the Secret is still the
/// incarnation the user chose. Errors never quote the value.
pub async fn reveal_secret_value(
    client: &Client,
    namespace: &str,
    name: &str,
    uid: &str,
    key: &str,
) -> Result<SecretValue, Failure> {
    let kind = ResourceKind::new("", "v1", "Secret", "secrets", true);
    let object: serde_json::Value = client
        .request(json_get(kind.object_path(Some(namespace), name))?)
        .await
        .map_err(Failure::from_kube)?;
    secret_value(&object, uid, key)
}

fn secret_value(object: &serde_json::Value, uid: &str, key: &str) -> Result<SecretValue, Failure> {
    if object
        .pointer("/metadata/uid")
        .and_then(|value| value.as_str())
        != Some(uid)
    {
        return Err(Failure::new(
            FailureKind::NotFound,
            "This Secret was deleted and created again; select it again to read the new one",
        ));
    }
    if let Some(encoded) = object
        .get("data")
        .and_then(|data| data.get(key))
        .and_then(|value| value.as_str())
    {
        let bytes = STANDARD.decode(encoded).map_err(|_| {
            Failure::new(
                FailureKind::Other,
                format!("The value of '{key}' isn't valid base64"),
            )
        })?;
        return Ok(match String::from_utf8(bytes) {
            Ok(text) => SecretValue::Text(text),
            Err(error) => SecretValue::Binary(error.into_bytes().len()),
        });
    }
    // The API server folds stringData into data, but keep reading it anyway.
    if let Some(text) = object
        .get("stringData")
        .and_then(|data| data.get(key))
        .and_then(|value| value.as_str())
    {
        return Ok(SecretValue::Text(text.to_owned()));
    }
    Err(Failure::new(
        FailureKind::NotFound,
        format!("This Secret no longer has a key '{key}'"),
    ))
}

/// The placeholder a hidden value is shown as.
pub fn hidden_value(bytes: usize) -> String {
    match bytes {
        1 => "<hidden, 1 byte>".into(),
        bytes => format!("<hidden, {bytes} bytes>"),
    }
}

/// Replaces every value with its size and drops the last-applied copy.
fn hide_secret(object: &mut Value) -> SecretSummary {
    let mut keys = Vec::new();
    for (field, encoded) in [("data", true), ("stringData", false)] {
        let Some(values) = object.get_mut(field).and_then(Value::as_mapping_mut) else {
            continue;
        };
        for (key, value) in values.iter_mut() {
            let content = value.as_str().unwrap_or_default();
            let bytes = if encoded {
                decoded_len(content)
            } else {
                content.len()
            };
            keys.push(SecretKey {
                name: key.as_str().unwrap_or_default().to_owned(),
                bytes,
            });
            *value = Value::String(hidden_value(bytes));
        }
    }
    if let Some(annotations) = object
        .get_mut("metadata")
        .and_then(|metadata| metadata.get_mut("annotations"))
        .and_then(Value::as_mapping_mut)
    {
        annotations.remove(LAST_APPLIED);
    }
    SecretSummary {
        secret_type: text(object.get("type")),
        keys,
    }
}

/// The decoded size of canonical base64, worked out without decoding it.
fn decoded_len(encoded: &str) -> usize {
    let encoded = encoded.trim();
    let padding = encoded
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'=')
        .count();
    (encoded.len() / 4 * 3).saturating_sub(padding.min(2))
}

pub(super) fn text(value: Option<&Value>) -> String {
    value.map(scalar).unwrap_or_default()
}

/// A scalar as text; anything else as compact YAML.
fn scalar(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        other => serde_yaml::to_string(other)
            .map(|yaml| yaml.trim_end().to_owned())
            .unwrap_or_default(),
    }
}

pub(super) fn time(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let text = value?.as_str()?;
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

pub(super) fn sequence(value: Option<&Value>) -> &[Value] {
    value
        .and_then(Value::as_sequence)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn pairs(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_mapping)
        .map(|mapping| {
            mapping
                .iter()
                .map(|(key, value)| (scalar(key), scalar(value)))
                .collect()
        })
        .unwrap_or_default()
}

fn overview(object: &Value) -> Overview {
    let metadata = object.get("metadata");
    let field = |name: &str| metadata.and_then(|metadata| metadata.get(name));
    let status = object.get("status");
    Overview {
        created: time(field("creationTimestamp")),
        deleting: time(field("deletionTimestamp")),
        generation: field("generation").and_then(Value::as_i64),
        observed_generation: status
            .and_then(|status| status.get("observedGeneration"))
            .and_then(Value::as_i64),
        labels: pairs(field("labels")),
        annotations: pairs(field("annotations")),
        owners: sequence(field("ownerReferences"))
            .iter()
            .map(|owner| Owner {
                api_version: text(owner.get("apiVersion")),
                kind: text(owner.get("kind")),
                name: text(owner.get("name")),
                uid: text(owner.get("uid")),
                controller: owner.get("controller").and_then(Value::as_bool) == Some(true),
            })
            .collect(),
        finalizers: sequence(field("finalizers")).iter().map(scalar).collect(),
        conditions: sequence(status.and_then(|status| status.get("conditions")))
            .iter()
            .map(|condition| Condition {
                kind: text(condition.get("type")),
                status: text(condition.get("status")),
                reason: text(condition.get("reason")),
                message: text(condition.get("message")),
                changed: time(condition.get("lastTransitionTime"))
                    .or_else(|| time(condition.get("lastUpdateTime"))),
            })
            .collect(),
        secret: None,
        pod: None,
        ports: None,
    }
}

#[cfg(test)]
mod tests;
