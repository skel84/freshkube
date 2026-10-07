use super::get_metadata;
use http::{Request, Response, header};
use kube::client::Body;
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};

#[tokio::test]
async fn identity_reads_negotiate_only_metadata_and_preserve_refusal() {
    for status in [200, 403, 406] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let service = tower::service_fn(move |request: Request<Body>| {
            seen.lock().unwrap().push((
                request.uri().to_string(),
                request.headers()[header::ACCEPT]
                    .to_str()
                    .unwrap()
                    .to_owned(),
            ));
            async move {
                Ok::<_,Infallible>(Response::builder().status(status).body(Body::from((if status==200 {r#"{"apiVersion":"meta.k8s.io/v1","kind":"PartialObjectMetadata","metadata":{"name":"credential","namespace":"default","uid":"stable"}}"#} else {r#"{"kind":"Status","status":"Failure","reason":"Forbidden","message":"metadata unavailable","code":403}"#}).as_bytes().to_vec())).unwrap())
            }
        });
        let client = kube::Client::new(service, "default");
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
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].0,
            "/api/v1/namespaces/default/secrets/credential"
        );
        assert_eq!(
            requests[0].1,
            "application/json;as=PartialObjectMetadata;g=meta.k8s.io;v=v1"
        );
    }
}

#[tokio::test]
async fn secret_name_search_is_one_bounded_cache_metadata_list_with_no_fallback() {
    for status in [200, 403, 406] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let service = tower::service_fn(move |request: Request<Body>| {
            seen.lock().unwrap().push((
                request.uri().to_string(),
                request.headers()[header::ACCEPT]
                    .to_str()
                    .unwrap()
                    .to_owned(),
            ));
            async move {
                Ok::<_,Infallible>(Response::builder().status(status).body(Body::from((if status==200 {r#"{"apiVersion":"meta.k8s.io/v1","kind":"PartialObjectMetadataList","metadata":{"continue":"next"},"items":[{"metadata":{"name":"credential","namespace":"shop","uid":"stable"}}]}"#}else{r#"{"kind":"Status","status":"Failure","reason":"Forbidden","message":"metadata unavailable","code":403}"#}).as_bytes().to_vec())).unwrap())
            }
        });
        let result = super::list_metadata(
            &kube::Client::new(service, "default"),
            &super::super::builtin("secrets").unwrap(),
        )
        .await;
        assert_eq!(result.is_ok(), status == 200);
        if let Ok(names) = result {
            assert!(names.capped);
            assert_eq!(names.objects.len(), 1);
            assert_eq!(names.objects[0].name.as_deref(), Some("credential"));
        }
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].0,
            "/api/v1/secrets?limit=2000&resourceVersion=0"
        );
        assert_eq!(
            requests[0].1,
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
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let service = tower::service_fn(move |request: Request<Body>| {
            seen.lock().unwrap().push(request.uri().to_string());
            async move {
                Ok::<_, Infallible>(Response::new(Body::from(
                    br#"{"metadata":{"name":"n1"}}"#.to_vec(),
                )))
            }
        });
        get_metadata(
            &kube::Client::new(service, "default"),
            &super::super::builtin(key).unwrap(),
            namespace,
            "n1",
        )
        .await
        .unwrap();
        assert_eq!(*requests.lock().unwrap(), [path]);
    }
}
