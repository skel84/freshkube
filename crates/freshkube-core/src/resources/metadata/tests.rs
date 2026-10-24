use super::get_metadata;
use http::{Request, Response};
use kube::client::Body;
use std::convert::Infallible;

#[tokio::test]
async fn identity_reads_negotiate_only_metadata_and_preserve_refusal() {
    for status in [200, 403, 406] {
        let (client, recorder) = crate::fake_api::client(move |_| {
            Some(crate::fake_api::json(
                status,
                if status == 200 {
                    r#"{"apiVersion":"meta.k8s.io/v1","kind":"PartialObjectMetadata","metadata":{"name":"credential","namespace":"default","uid":"stable"}}"#
                } else {
                    r#"{"kind":"Status","status":"Failure","reason":"Forbidden","message":"metadata unavailable","code":403}"#
                },
            ))
        });
        let result = get_metadata(
            &client,
            &super::super::builtin("secrets").unwrap(),
            Some("default"),
            "credential",
        )
        .await;
        assert_eq!(result.is_ok(), status == 200);
        if let Ok(metadata) = result {
            assert_eq!(metadata.uid.as_deref(), Some("stable"));
        }
        // One metadata-only GET of the Secret, and nothing else.
        recorder.assert_lines(&["GET /api/v1/namespaces/default/secrets/credential"]);
        assert_eq!(
            recorder.requests()[0].accept,
            "application/json;as=PartialObjectMetadata;g=meta.k8s.io;v=v1"
        );
    }
}

#[tokio::test]
async fn secret_name_search_is_one_bounded_cache_metadata_list_with_no_fallback() {
    for status in [200, 403, 406] {
        let (client, recorder) = crate::fake_api::client(move |_| {
            Some(crate::fake_api::json(
                status,
                if status == 200 {
                    r#"{"apiVersion":"meta.k8s.io/v1","kind":"PartialObjectMetadataList","metadata":{"continue":"next"},"items":[{"metadata":{"name":"credential","namespace":"shop","uid":"stable"}}]}"#
                } else {
                    r#"{"kind":"Status","status":"Failure","reason":"Forbidden","message":"metadata unavailable","code":403}"#
                },
            ))
        });
        let result =
            super::list_metadata(&client, &super::super::builtin("secrets").unwrap()).await;
        assert_eq!(result.is_ok(), status == 200);
        if let Ok(names) = result {
            assert!(names.capped);
            assert_eq!(names.objects.len(), 1);
            assert_eq!(names.objects[0].name.as_deref(), Some("credential"));
        }
        // One bounded metadata-only list, with no fallback to a full read.
        recorder.assert_lines(&["GET /api/v1/secrets?limit=2000&resourceVersion=0"]);
        assert_eq!(
            recorder.requests()[0].accept,
            "application/json;as=PartialObjectMetadataList;g=meta.k8s.io;v=v1"
        );
    }
}

#[tokio::test]
async fn metadata_names_remain_bounded_if_a_server_ignores_the_limit() {
    let service = tower::service_fn(move |_: Request<Body>| async move {
        let items=(0..2_010).map(|ix|serde_json::json!({"metadata":{"name":format!("pod-{ix}"),"uid":format!("uid-{ix}")}})).collect::<Vec<_>>();
        Ok::<_, Infallible>(Response::new(Body::from(
            serde_json::json!({"items":items}).to_string().into_bytes(),
        )))
    });
    let names = super::list_metadata(
        &kube::Client::new(service, "default"),
        &super::super::builtin("pods").unwrap(),
    )
    .await
    .unwrap();
    assert!(names.capped);
    assert_eq!(names.objects.len(), 2_000);
}

#[tokio::test]
async fn identity_reads_use_the_kinds_scope_and_group_path() {
    for (key, namespace, path) in [
        ("nodes", Some("ignored"), "/api/v1/nodes/n1"),
        ("nodes", None, "/api/v1/nodes/n1"),
        (
            "deployments.apps",
            Some("shop"),
            "/apis/apps/v1/namespaces/shop/deployments/n1",
        ),
    ] {
        let (client, recorder) = crate::fake_api::client(|_| {
            Some(crate::fake_api::json(200, r#"{"metadata":{"name":"n1"}}"#))
        });
        get_metadata(
            &client,
            &super::super::builtin(key).unwrap(),
            namespace,
            "n1",
        )
        .await
        .unwrap();
        recorder.assert_lines(&[&format!("GET {path}")]);
    }
}

#[tokio::test]
async fn a_metadata_list_with_null_items_reads_as_empty() {
    let (client, _recorder) = crate::fake_api::client(|_| {
        Some(crate::fake_api::json(
            200,
            r#"{"apiVersion":"meta.k8s.io/v1","kind":"PartialObjectMetadataList","metadata":{},"items":null}"#,
        ))
    });
    let names = super::list_metadata(&client, &super::super::builtin("secrets").unwrap())
        .await
        .unwrap();
    assert!(names.objects.is_empty());
    assert!(!names.capped);
}
