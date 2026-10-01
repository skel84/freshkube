//! One object's full representation, read on demand, and a Secret's values,
//! read one key at a time when the user asks for it.

use std::fmt;

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use http::{Request, header};
use kube::Client;
use serde_yaml::Value;

use super::failure::{Failure, FailureKind};
use super::kinds::ResourceKind;

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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
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

fn json_get(path: String) -> Result<Request<Vec<u8>>, Failure> {
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

pub(crate) fn document(kind: &ResourceKind, mut object: Value) -> Result<ObjectDocument, Failure> {
    if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_mapping_mut) {
        metadata.remove("managedFields");
    }
    let secret = kind.is_secret().then(|| hide_secret(&mut object));
    let mut overview = overview(&object);
    overview.secret = secret;
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

fn text(value: Option<&Value>) -> String {
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

fn time(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let text = value?.as_str()?;
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

fn sequence(value: Option<&Value>) -> &[Value] {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::builtin;
    use crate::resources::watch::tests::server;
    use serde_json::json;

    fn deployment() -> serde_json::Value {
        json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {
                "name": "web", "namespace": "shop", "uid": "u-1", "resourceVersion": "42",
                "generation": 7, "creationTimestamp": "2026-09-30T10:00:00Z",
                "labels": {"app": "web", "tier": "front"},
                "annotations": {"note": "multi\nline"},
                "ownerReferences": [{"apiVersion": "v1", "kind": "Thing", "name": "owner",
                    "uid": "o-1", "controller": true}],
                "finalizers": ["example.com/hold"],
                "managedFields": [{"manager": "kubectl"}]
            },
            "spec": {"replicas": 3, "selector": {"matchLabels": {"app": "web"}}},
            "status": {
                "observedGeneration": 6,
                "conditions": [
                    {"type": "Available", "status": "True", "reason": "MinimumReplicasAvailable",
                     "message": "ok", "lastTransitionTime": "2026-09-30T10:05:00Z",
                     "lastUpdateTime": "2026-09-30T10:06:00Z"},
                    {"type": "Progressing", "status": "False", "lastUpdateTime": "2026-09-30T10:07:00Z"}
                ]
            }
        })
    }

    #[tokio::test]
    async fn an_object_reads_in_server_order_without_managed_fields() {
        let (client, seen) = server(|_| (200, deployment().to_string()));
        let kind = builtin("deployments.apps").unwrap();
        let document = get_object(&client, &kind, Some("shop"), "web")
            .await
            .unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            "/apis/apps/v1/namespaces/shop/deployments/web"
        );
        assert_eq!(document.kind, "Deployment");
        assert_eq!(document.api_version, "apps/v1");
        assert_eq!(document.namespace.as_deref(), Some("shop"));
        assert_eq!(
            (document.uid.as_str(), document.resource_version.as_str()),
            ("u-1", "42")
        );
        assert!(!document.yaml.contains("managedFields"));
        // The order the server wrote, not alphabetical: kind before
        // metadata, and spec before status.
        let top: Vec<&str> = document
            .yaml
            .lines()
            .filter(|line| !line.starts_with(' ') && !line.starts_with('-'))
            .collect();
        assert_eq!(
            top,
            [
                "apiVersion: apps/v1",
                "kind: Deployment",
                "metadata:",
                "spec:",
                "status:"
            ]
        );
        let overview = &document.overview;
        assert_eq!(overview.generation, Some(7));
        assert_eq!(overview.observed_generation, Some(6));
        assert_eq!(
            overview.created.unwrap().to_rfc3339(),
            "2026-09-30T10:00:00+00:00"
        );
        assert_eq!(overview.deleting, None);
        assert_eq!(overview.labels[1], ("tier".into(), "front".into()));
        assert_eq!(overview.annotations[0].1, "multi\nline");
        assert_eq!(
            overview.owners,
            [Owner {
                kind: "Thing".into(),
                name: "owner".into(),
                uid: "o-1".into(),
                controller: true
            }]
        );
        assert_eq!(overview.finalizers, ["example.com/hold"]);
        assert_eq!(overview.conditions.len(), 2);
        assert_eq!(overview.conditions[0].reason, "MinimumReplicasAvailable");
        // The transition time wins; the update time stands in without one.
        assert_eq!(
            overview.conditions[0].changed.unwrap().to_rfc3339(),
            "2026-09-30T10:05:00+00:00"
        );
        assert_eq!(
            overview.conditions[1].changed.unwrap().to_rfc3339(),
            "2026-09-30T10:07:00+00:00"
        );
        assert!(overview.secret.is_none());
    }

    #[tokio::test]
    async fn refusals_and_missing_objects_are_classified() {
        let (client, _) = server(|uri| {
            let code = if uri.contains("/gone") { 404 } else { 403 };
            let status = json!({"kind": "Status", "apiVersion": "v1", "status": "Failure",
                "message": "no", "code": code});
            (code, status.to_string())
        });
        let kind = builtin("pods").unwrap();
        let refused = get_object(&client, &kind, Some("a"), "x")
            .await
            .unwrap_err();
        assert_eq!(refused.kind, FailureKind::Forbidden);
        let gone = get_object(&client, &kind, Some("a"), "gone")
            .await
            .unwrap_err();
        assert_eq!(gone.kind, FailureKind::NotFound);
    }

    fn secret() -> serde_json::Value {
        json!({
            "apiVersion": "v1", "kind": "Secret", "type": "Opaque",
            "metadata": {"name": "s", "namespace": "ns", "uid": "s-1",
                "annotations": {LAST_APPLIED: "{\"data\":{\"token\":\"c2VjcmV0LXZhbHVl\"}}", "keep": "yes"}},
            // "secret-value", one byte, and two bytes that aren't UTF-8.
            "data": {"token": "c2VjcmV0LXZhbHVl", "one": "YQ==", "raw": "/4A="},
            "stringData": {"plain": "abc"}
        })
    }

    #[test]
    fn a_secret_document_shows_keys_and_sizes_never_values() {
        let kind = builtin("secrets").unwrap();
        let document = document(&kind, serde_json::from_value(secret()).unwrap()).unwrap();
        for leak in [
            "c2VjcmV0LXZhbHVl",
            "secret-value",
            "YQ==",
            "/4A=",
            "abc",
            LAST_APPLIED,
        ] {
            assert!(!document.yaml.contains(leak), "{leak} in {}", document.yaml);
            assert!(!format!("{:?}", document.overview).contains(leak), "{leak}");
        }
        assert!(
            document.yaml.contains("token: <hidden, 12 bytes>"),
            "{}",
            document.yaml
        );
        assert!(document.yaml.contains("one: <hidden, 1 byte>"));
        assert!(document.yaml.contains("keep: "), "{}", document.yaml);
        let summary = document.overview.secret.unwrap();
        assert_eq!(summary.secret_type, "Opaque");
        let mut keys: Vec<_> = summary
            .keys
            .iter()
            .map(|key| (key.name.as_str(), key.bytes))
            .collect();
        // Whether `json!` keeps its key order depends on workspace features.
        keys[..3].sort();
        assert_eq!(keys, [("one", 1), ("raw", 2), ("token", 12), ("plain", 3)]);
        assert_eq!(
            document.overview.annotations,
            [("keep".into(), "yes".into())]
        );
    }

    #[test]
    fn a_secret_value_is_revealed_only_for_the_same_incarnation() {
        let object = secret();
        assert_eq!(
            secret_value(&object, "s-1", "token").unwrap(),
            SecretValue::Text("secret-value".into())
        );
        assert_eq!(
            secret_value(&object, "s-1", "raw").unwrap(),
            SecretValue::Binary(2)
        );
        assert_eq!(
            secret_value(&object, "s-1", "plain").unwrap(),
            SecretValue::Text("abc".into())
        );
        let replaced = secret_value(&object, "s-0", "token").unwrap_err();
        assert_eq!(replaced.kind, FailureKind::NotFound);
        assert!(!replaced.message.contains("secret-value"));
        assert_eq!(
            secret_value(&object, "s-1", "missing").unwrap_err().kind,
            FailureKind::NotFound
        );
        let broken = json!({"metadata": {"uid": "s-1"}, "data": {"token": "c2Vj!!!"}});
        let error = secret_value(&broken, "s-1", "token").unwrap_err();
        assert!(!error.message.contains("c2Vj"), "{}", error.message);
        // Debug output names the size only.
        let debug = format!("{:?}", SecretValue::Text("secret-value".into()));
        assert!(!debug.contains("secret"), "{debug}");
    }

    #[tokio::test]
    async fn a_reveal_reads_the_secret_by_address() {
        let (client, seen) = server(|_| (200, secret().to_string()));
        let value = reveal_secret_value(&client, "ns", "s", "s-1", "token")
            .await
            .unwrap();
        assert_eq!(value, SecretValue::Text("secret-value".into()));
        assert_eq!(seen.lock().unwrap()[0], "/api/v1/namespaces/ns/secrets/s");
    }

    #[test]
    fn decoded_sizes_follow_padding() {
        for (encoded, bytes) in [
            ("", 0),
            ("YQ==", 1),
            ("YWI=", 2),
            ("YWJj", 3),
            ("YWJjZA==", 4),
        ] {
            assert_eq!(decoded_len(encoded), bytes, "{encoded}");
            assert_eq!(STANDARD.decode(encoded).unwrap().len(), bytes);
        }
    }
}
