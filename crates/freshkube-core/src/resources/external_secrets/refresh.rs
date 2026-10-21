//! The one confirmed ExternalSecret write. No retry or related-object access.
use super::{EXTERNAL_SECRETS_GROUP, Failure, FailureKind, ResourceKind};
use kube::Client;

/// Requests ESO reconciliation by changing only `metadata.annotations.force-sync`.
/// `seconds` is the Unix time at confirmation, as documented by ESO. The caller
/// owns confirmation, target/access validation and the submitted request lifetime.
pub async fn refresh_external_secret(
    client: &Client,
    kind: &ResourceKind,
    namespace: &str,
    name: &str,
    seconds: i64,
) -> Result<(), Failure> {
    let segment = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.'))
    };
    if kind.group != EXTERNAL_SECRETS_GROUP
        || kind.plural != "externalsecrets"
        || kind.kind != "ExternalSecret"
        || !kind.namespaced
        || !segment(&kind.version)
        || !segment(namespace)
        || !segment(name)
        || seconds < 0
    {
        return Err(Failure::new(
            FailureKind::Other,
            "Refresh requires one identified ExternalSecret",
        ));
    }
    let body = serde_json::to_vec(&serde_json::json!({
        "metadata": {"annotations": {"force-sync": seconds.to_string()}}
    }))
    .expect("annotation-only JSON is serializable");
    let request = http::Request::patch(format!(
        "{}?fieldManager=freshkube",
        kind.object_path(Some(namespace), name)
    ))
    .header(http::header::CONTENT_TYPE, "application/merge-patch+json")
    .header(http::header::ACCEPT, "application/json")
    .body(body)
    .map_err(|error| Failure::new(FailureKind::Other, error.to_string()))?;
    // The API's full-object reply may contain provider references. Only its
    // success matters here; subsequent ordinary reads observe reported status.
    client
        .request::<serde::de::IgnoredAny>(request)
        .await
        .map_err(Failure::from_kube)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{Request, Response};
    use kube::client::Body;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn exactly_one_annotation_only_merge_patch_and_refusal_cause() {
        for code in [200, 403] {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let sink = seen.clone();
            let api = tower::service_fn(move |request: Request<Body>| {
                let sink = sink.clone();
                async move {
                    let (parts, body) = request.into_parts();
                    let bytes = body.collect_bytes().await.unwrap();
                    sink.lock().unwrap().push((
                        parts.method.clone(),
                        parts.uri.to_string(),
                        bytes.clone(),
                    ));
                    assert_eq!(parts.method, http::Method::PATCH);
                    assert_eq!(
                        parts.uri.to_string(),
                        "/apis/external-secrets.io/v1/namespaces/paper-kites/externalsecrets/lantern-api?fieldManager=freshkube"
                    );
                    assert_eq!(
                        parts.headers[http::header::CONTENT_TYPE],
                        "application/merge-patch+json"
                    );
                    assert_eq!(
                        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                        serde_json::json!({"metadata":{"annotations":{"force-sync":"1767312000"}}})
                    );
                    Ok::<_, std::convert::Infallible>(Response::builder().status(code).body(Body::from(
                        if code == 200 { "{}".to_owned() } else { serde_json::json!({"kind":"Status","status":"Failure","code":403,"reason":"Forbidden","message":"invented viewer cannot patch externalsecrets"}).to_string() }.into_bytes()
                    )).unwrap())
                }
            });
            let result = refresh_external_secret(
                &Client::new(api, "paper-kites"),
                &ResourceKind::new(
                    EXTERNAL_SECRETS_GROUP,
                    "v1",
                    "ExternalSecret",
                    "externalsecrets",
                    true,
                ),
                "paper-kites",
                "lantern-api",
                1767312000,
            )
            .await;
            if code == 403 {
                let error = result.unwrap_err();
                assert_eq!(error.kind, FailureKind::Forbidden);
                assert!(error.message.contains("invented viewer cannot patch"));
            } else {
                result.unwrap();
            }
            assert_eq!(
                seen.lock().unwrap().len(),
                1,
                "no other write or Secret/credential read is permitted"
            );
        }
    }
}
