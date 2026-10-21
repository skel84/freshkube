use super::super::{get_object, object_from_yaml};
use super::*;
use serde_json::{Value as Json, json};

fn kind() -> ResourceKind {
    ResourceKind::new(
        EXTERNAL_SECRETS_GROUP,
        "v1beta1",
        "ExternalSecret",
        "externalsecrets",
        true,
    )
}
fn object() -> Json {
    json!({"apiVersion":"external-secrets.io/v1beta1","kind":"ExternalSecret","metadata":{"name":"lantern-api","namespace":"paper-kites","uid":"es-1"},"spec":{"secretStoreRef":{"name":"meadow-store"},"data":[{"secretKey":"config","remoteRef":{"key":"invented/provider/path"},"sourceRef":{"storeRef":{"kind":"ClusterSecretStore","name":"shared-meadow"}}}],"dataFrom":[{"sourceRef":{"storeRef":{"name":"meadow-store"}}},{"sourceRef":{"generatorRef":{"kind":"Password","name":"not-read"}}}],"target":{"name":"never-read"}},"status":{"refreshTime":"2026-01-01T12:00:00Z","conditions":[{"type":"Ready","status":"True","reason":"SecretSynced","message":"Secret was synced"}]}})
}
fn project(object: Json) -> ExternalSecret {
    object_from_yaml(&kind(), &object.to_string())
        .unwrap()
        .overview
        .external_secret
        .unwrap()
}

#[test]
fn projection_retains_reported_conditions_defaults_and_unique_store_scopes() {
    let summary = project(object());
    assert_eq!(summary.conditions.len(), 1);
    assert_eq!(summary.conditions[0].kind, "Ready");
    assert_eq!(summary.conditions[0].reason, "SecretSynced");
    assert_eq!(summary.interval, "1h");
    assert_eq!(summary.policy, "Periodic");
    assert!(summary.refreshed.is_some());
    assert_eq!(summary.stores.len(), 2);
    assert_eq!(summary.stores[0].namespace.as_deref(), Some("paper-kites"));
    assert_eq!(summary.stores[1].namespace, None);
    assert!(summary.unsupported_sources);
    for (interval, policy) in [("0", "Periodic"), ("1h", "CreatedOnce"), ("1h", "OnChange")] {
        let mut value = object();
        value["spec"]["refreshInterval"] = interval.into();
        value["spec"]["refreshPolicy"] = policy.into();
        value["status"] = json!({"conditions":[{"type":"SecretSynced","status":"False","reason":"Denied","message":"Access denied"}]});
        let summary = project(value);
        assert_eq!(summary.interval, interval);
        assert_eq!(summary.policy, policy);
        assert_eq!(summary.conditions[0].kind, "SecretSynced");
        assert!(summary.refreshed.is_none());
    }
    assert!(
        object_from_yaml(
            &ResourceKind::new(
                "unrelated.io",
                "v1",
                "ExternalSecret",
                "externalsecrets",
                true
            ),
            &object().to_string()
        )
        .unwrap()
        .overview
        .external_secret
        .is_none()
    );
}

#[test]
fn missing_unknown_and_unsupported_sources_never_become_healthy() {
    let mut value = object();
    value["spec"] = json!({"secretStoreRef":{"kind":"Secret","name":"not-read"}});
    value["status"] = json!({});
    let summary = project(value);
    assert!(summary.conditions.is_empty());
    assert!(summary.stores.is_empty());
    assert!(summary.unsupported_sources);
    let mut value = object();
    value["spec"]["data"] = Json::Array(
        (0..30)
            .map(|ix| json!({"sourceRef":{"storeRef":{"name":format!("store-{ix}")}}}))
            .collect(),
    );
    let summary = project(value);
    assert_eq!(summary.stores.len(), MAX_STORES);
    assert!(summary.stores_omitted > 0);
}

use crate::fake_api::{Recorded, json as reply};

/// Closed allowlist: any mutation, Secret, credential, generator, provider or
/// other unexpected URL is answered 500 by the fake and fails the test when
/// it asserts on the recorded requests after the read.
fn allowed(request: &Recorded) -> bool {
    request.method == http::Method::GET
        && matches!(
            request.path.as_str(),
            "/apis"
                | "/apis/external-secrets.io/v1"
                | "/apis/external-secrets.io/v1beta1"
                | "/apis/external-secrets.io/v1beta1/namespaces/paper-kites/externalsecrets/lantern-api"
                | "/apis/external-secrets.io/v1/namespaces/paper-kites/secretstores/meadow-store"
                | "/apis/external-secrets.io/v1/clustersecretstores/shared-meadow"
        )
}

#[test]
fn fake_api_guard_rejects_secrets_credentials_and_all_mutation_verbs() {
    for (method, path) in [
        ("GET", "/api/v1/namespaces/paper-kites/secrets/never-read"),
        (
            "GET",
            "/api/v1/namespaces/paper-kites/secrets/provider-auth",
        ),
        ("GET", "/credentials"),
        ("POST", "/apis"),
        ("PUT", "/apis"),
        ("PATCH", "/apis"),
        ("DELETE", "/apis"),
    ] {
        let recorder = crate::fake_api::Recorder::default();
        recorder.record(
            &http::Request::builder()
                .method(method)
                .uri(path)
                .body(())
                .unwrap(),
        );
        assert!(
            std::panic::catch_unwind(|| recorder.assert_all("the allowlist", allowed)).is_err(),
            "{method} {path}"
        );
    }
}

#[tokio::test]
async fn fake_api_allows_only_external_secret_and_status_reads() {
    let (client, recorder) = crate::fake_api::client(|request| {
        if !allowed(request) {
            return None;
        }
        Some(reply(
            200,
            match request.path.as_str() {
                "/apis" => {
                    json!({"groups":[{"name":"external-secrets.io","preferredVersion":{"version":"v1"},"versions":[{"version":"v1"},{"version":"v1beta1"}]}]})
                }
                "/apis/external-secrets.io/v1" => {
                    json!({"resources":[{"name":"secretstores","kind":"SecretStore","namespaced":true,"verbs":["get","list","watch"]},{"name":"clustersecretstores","kind":"ClusterSecretStore","namespaced":false,"verbs":["get","list","watch"]}]})
                }
                "/apis/external-secrets.io/v1beta1" => json!({"resources":[]}),
                "/apis/external-secrets.io/v1beta1/namespaces/paper-kites/externalsecrets/lantern-api" => {
                    object()
                }
                _ => {
                    json!({"metadata":{"name":"not-retained"},"spec":{"provider":{"fake":{"auth":{"secretRef":{"name":"provider-auth"}},"password":"invented-sensitive-sentinel"}}},"status":{"conditions":[{"type":"Ready","status":"True","reason":"Valid","message":"Store validated"}]}})
                }
            },
        ))
    });
    let document = get_object(&client, &kind(), Some("paper-kites"), "lantern-api")
        .await
        .unwrap();
    let summary = document.overview.external_secret.unwrap();
    let reads = read_external_secret_stores(&client, &summary).await;
    assert_eq!(reads.len(), 2);
    for read in &reads {
        assert_eq!(
            read.status.as_ref().unwrap().ready.as_ref().unwrap().status,
            "True"
        );
    }
    let mut actual = recorder.lines();
    actual.sort();
    let mut expected = [
        "/apis",
        "/apis/external-secrets.io/v1",
        "/apis/external-secrets.io/v1beta1",
        "/apis/external-secrets.io/v1beta1/namespaces/paper-kites/externalsecrets/lantern-api",
        "/apis/external-secrets.io/v1/namespaces/paper-kites/secretstores/meadow-store",
        "/apis/external-secrets.io/v1/clustersecretstores/shared-meadow",
    ]
    .map(|path| format!("GET {path}"));
    expected.sort();
    assert_eq!(actual, expected);
    let projected = format!("{reads:?}");
    for forbidden in [
        "invented-sensitive-sentinel",
        "provider-auth",
        "not-retained",
        "spec",
        "password",
    ] {
        assert!(!projected.contains(forbidden));
    }
}

#[tokio::test]
async fn refused_missing_and_unknown_store_status_are_distinct() {
    for code in [200, 403, 404, 500, 0, 1] {
        let (client, recorder) = crate::fake_api::client(move |request| {
            if !allowed(request) {
                return None;
            }
            Some(match request.path.as_str() {
                "/apis" if code == 0 => reply(200, json!({"groups":[]})),
                "/apis/external-secrets.io/v1" if code == 1 => reply(200, json!({"resources":[]})),
                "/apis" => reply(
                    200,
                    json!({"groups":[{"name":"external-secrets.io","versions":[{"version":"v1"}]}]}),
                ),
                "/apis/external-secrets.io/v1" => reply(
                    200,
                    json!({"resources":[{"name":"secretstores","kind":"SecretStore","namespaced":true,"verbs":["get","list","watch"]},{"name":"clustersecretstores","kind":"ClusterSecretStore","namespaced":false,"verbs":["get","list","watch"]}]}),
                ),
                _ => reply(
                    code,
                    if code == 200 {
                        json!({"status":{"conditions":null}})
                    } else {
                        json!({"kind":"Status","status":"Failure","reason":"Synthetic","message":"Invented store read failed","code":code})
                    },
                ),
            })
        });
        let reads = read_external_secret_stores(&client, &project(object())).await;
        for read in reads {
            assert_eq!(read.api_absent, code <= 1);
            match code {
                0 | 1 => assert_eq!(read.status.unwrap_err().kind, FailureKind::NotFound),
                200 => assert!(read.status.unwrap().ready.is_none()),
                403 => assert_eq!(read.status.unwrap_err().kind, FailureKind::Forbidden),
                404 => assert_eq!(read.status.unwrap_err().kind, FailureKind::NotFound),
                _ => assert!(read.status.is_err()),
            }
        }
        recorder.assert_all("the external-secrets allowlist", allowed);
    }
}

#[tokio::test]
async fn incomplete_store_discovery_does_not_claim_the_api_is_absent() {
    let (client, recorder) = crate::fake_api::client(|request| {
        if !allowed(request) {
            return None;
        }
        match request.path.as_str() {
            "/apis" => Some(reply(
                200,
                json!({"groups":[{"name":"external-secrets.io","versions":[{"version":"v1"},{"version":"v1beta1"}]}]}),
            )),
            "/apis/external-secrets.io/v1" => Some(reply(200, json!({"resources":[]}))),
            "/apis/external-secrets.io/v1beta1" => Some(reply(
                403,
                json!({"kind":"Status","status":"Failure","code":403,"reason":"Forbidden","message":"Invented discovery refusal"}),
            )),
            // No store is known to be served, so no store read is allowed: the 500 and the
            // recorded line fail the assertion below.
            _ => None,
        }
    });
    let reads = read_external_secret_stores(&client, &project(object())).await;
    for read in reads {
        assert!(!read.api_absent);
        let error = read.status.unwrap_err();
        assert_eq!(error.kind, FailureKind::Forbidden);
        assert!(error.message.contains("discovery refusal"));
        assert!(!error.message.contains("not served"));
    }
    recorder.assert_all("discovery only, no store read", |request| {
        allowed(request) && request.path.split('/').count() <= 4
    });
}
