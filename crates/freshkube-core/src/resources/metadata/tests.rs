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
