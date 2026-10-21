use super::*;

fn incident_wire(key: &str) -> serde_json::Value {
    serde_json::json!({"key":key,"application_id":"c:prod:Deployment:api","cluster":"Production","severity":"critical","opened_at":1790000000000_i64,"resolved_at":null,"impact":0,"duration":120000,"short_description":"High latency"})
}

#[tokio::test]
async fn incident_identity_errors_limits_and_missing_evidence() {
    let mut wrong = incident_wire("k1");
    wrong["application_id"] = serde_json::json!("c:prod:Deployment:other");
    let mut oversized = incident_wire("k1");
    oversized["rca"] = serde_json::json!({"root_cause":"x".repeat(16385)});
    let envelope = |data| serde_json::json!({"context":{},"data":data});
    let server = server(vec![
        (200, envelope(serde_json::json!([incident_wire("k1")]))),
        (200, envelope(incident_wire("k1"))),
        (200, envelope(wrong)),
        (200, envelope(oversized)),
        (200, envelope(serde_json::json!({}))),
        (403, serde_json::json!({"error":"private server text"})),
        (200, envelope(serde_json::json!([]))),
    ])
    .await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let source = source(&provider);
    let app = AppId::new("c:prod:Deployment:api");
    let values = provider.incidents(&source, range()).await.unwrap();
    assert_eq!(values[0].key, "k1");
    assert_eq!(values[0].impact_percent, 0.);
    let view = provider
        .incident(&source, range(), "k1", &app)
        .await
        .unwrap();
    assert!(view.availability().is_none());
    assert!(view.incident().slo.is_none());
    assert_eq!(
        provider
            .incident(&source, range(), "k1", &app)
            .await
            .unwrap_err(),
        ReadError::InvalidResponse
    );
    assert_eq!(
        provider
            .incident(&source, range(), "k1", &app)
            .await
            .unwrap_err(),
        ReadError::Limit
    );
    assert_eq!(
        provider.incidents(&source, range()).await.unwrap_err(),
        ReadError::InvalidResponse
    );
    assert_eq!(
        provider.incidents(&source, range()).await.unwrap_err(),
        ReadError::Refused
    );
    assert!(
        provider
            .incidents(&source, range())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn incidents_enforce_collection_bounds() {
    let values = (0..101)
        .map(|ix| incident_wire(&format!("k{ix}")))
        .collect::<Vec<_>>();
    let server = server(vec![(200, serde_json::json!({"context":{},"data":values}))]).await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    // A server violating the requested bound fails visibly. Oversized response
    // bodies are independently bounded before decoding.
    assert_eq!(
        provider
            .incidents(&source(&provider), range())
            .await
            .unwrap_err(),
        ReadError::InvalidResponse
    );
}
