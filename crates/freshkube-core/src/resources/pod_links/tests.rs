use super::*;
use serde_json::json;

fn pod() -> ObjectDocument {
    super::super::object_from_yaml(&super::super::builtin("pods").unwrap(),&serde_json::to_string(&json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"web","namespace":"shop","uid":"pod-uid","labels":{"app":"web","tier":"front"},"ownerReferences":[{"apiVersion":"apps/v1","kind":"ReplicaSet","name":"web-rs","uid":"rs-uid","controller":true}]},"spec":{"nodeName":"node-a","containers":[{"name":"app","image":"web:v2"}]}})).unwrap()).unwrap()
}
#[test]
fn service_selectors_require_every_label_and_controller_uids_match() {
    let pod = pod();
    let service = |name, selector| {
        serde_json::from_value::<Service>(json!({"metadata":{"name":name,"namespace":"shop","uid":name},"spec":{"selector":selector}})).unwrap()
    };
    let services = vec![
        service("matches", json!({"app":"web","tier":"front"})),
        service("wrong", json!({"app":"web","tier":"back"})),
        service("empty", json!({})),
    ];
    let mut rs=super::super::object_from_yaml(&super::super::builtin("replicasets.apps").unwrap(),r#"{"metadata":{"name":"web-rs","uid":"rs-uid","ownerReferences":[{"apiVersion":"apps/v1","kind":"Deployment","name":"web","uid":"deployment-uid","controller":true}]}}"#).unwrap();
    let links = PodLinks::from_sources(&pod, Ok(services.clone()), Some(Ok(rs.clone())));
    assert_eq!(
        links
            .selected_by(&pod.overview.labels)
            .map(|service| service.name.as_str())
            .collect::<Vec<_>>(),
        vec!["matches"]
    );
    assert_eq!(links.controller.unwrap().owners[0].kind, "Deployment");
    assert_eq!(
        pod.overview.pod.as_ref().unwrap().node.as_deref(),
        Some("node-a")
    );
    assert_eq!(
        pod.overview.pod.as_ref().unwrap().containers[0].image,
        "web:v2"
    );
    rs.uid = "replacement".into();
    let replaced = PodLinks::from_sources(&pod, Ok(services), Some(Ok(rs)));
    assert!(replaced.controller.is_none());
    assert!(replaced.controller_error.is_some());
    let denied = PodLinks::from_sources(
        &pod,
        Err(Failure::new(FailureKind::Forbidden, "Can't list services")),
        None,
    );
    assert!(denied.services.is_err());
}

#[tokio::test]
async fn collection_reads_only_namespace_services_and_the_referenced_replica_set() {
    use http::{Request, Response};
    use kube::client::Body;
    use std::{
        convert::Infallible,
        sync::{Arc, Mutex},
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let requests = seen.clone();
    let service = tower::service_fn(move |request: Request<Body>| {
        let uri = request.uri().to_string();
        requests.lock().unwrap().push(uri.clone());
        let body = if uri == "/api/v1/namespaces/shop/services?"
            || uri == "/api/v1/namespaces/shop/services"
        {
            json!({"apiVersion":"v1","kind":"ServiceList","metadata":{},"items":[]})
        } else {
            assert_eq!(uri, "/apis/apps/v1/namespaces/shop/replicasets/web-rs");
            json!({"apiVersion":"apps/v1","kind":"ReplicaSet","metadata":{"name":"web-rs","namespace":"shop","uid":"rs-uid"}})
        };
        async move { Ok::<_, Infallible>(Response::new(Body::from(body.to_string().into_bytes()))) }
    });
    let links = collect_pod_links(&kube::Client::new(service, "default"), &pod()).await;
    assert!(links.services.unwrap().is_empty());
    assert_eq!(links.controller.unwrap().via_uid, "rs-uid");
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn custom_owners_use_discovered_plural_and_scope() {
    use http::{Request, Response};
    use kube::client::Body;
    let service = tower::service_fn(move |request: Request<Body>| async move {
        assert_eq!(request.uri().path(), "/apis/sample.io/v2");
        Ok::<_, std::convert::Infallible>(Response::new(Body::from(br#"{"resources":[{"name":"people/status","kind":"Person","namespaced":true},{"name":"people","kind":"Person","namespaced":false}]}"#.to_vec())))
    });
    let kind = resolve_owner_kind(
        &kube::Client::new(service, "default"),
        "sample.io/v2",
        "Person",
    )
    .await
    .unwrap();
    assert_eq!(
        kind.object_path(None, "sam"),
        "/apis/sample.io/v2/people/sam"
    );
    assert!(!kind.namespaced);
}
