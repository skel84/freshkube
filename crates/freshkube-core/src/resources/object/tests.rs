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
            api_version: "v1".into(),
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

#[test]
fn written_objects_read_like_served_ones() {
    let yaml = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: db\n  namespace: shop\n  uid: s-1\n  resourceVersion: '3'\ntype: Opaque\ndata:\n  password: aHVudGVyMg==\n";
    let document = object_from_yaml(&builtin("secrets").unwrap(), yaml).unwrap();
    assert_eq!(
        (document.uid.as_str(), document.resource_version.as_str()),
        ("s-1", "3")
    );
    assert!(document.yaml.starts_with("apiVersion: v1\nkind: Secret\n"));
    assert!(!document.yaml.contains("aHVudGVyMg"));
    assert_eq!(document.overview.secret.unwrap().keys[0].bytes, 7);
    assert!(object_from_yaml(&builtin("pods").unwrap(), "a: [").is_err());
}

#[test]
fn a_forwardable_kind_lists_its_declared_ports() {
    let yaml = "apiVersion: v1\nkind: Service\nmetadata:\n  name: web\n  uid: s-1\nspec:\n  ports:\n  - name: http\n    port: 80\n    targetPort: http\n";
    let document = object_from_yaml(&builtin("services").unwrap(), yaml).unwrap();
    let ports = document.overview.ports.unwrap();
    assert_eq!((ports[0].name.as_str(), ports[0].port), ("http", 80));
    let secret = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: db\n";
    let document = object_from_yaml(&builtin("secrets").unwrap(), secret).unwrap();
    assert!(document.overview.ports.is_none());
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
