use super::*;

#[tokio::test]
async fn signals_empty_results_refusal_failure_and_recovery_remain_distinct() {
    let raw = serde_json::json!({"applications":[{"id":"c:prod:Deployment:api","status":"ok","category":"application","cpu":{"status":"ok","value":""},"memory":{"status":"unknown","value":""}}]});
    let server = server(vec![
        (200, raw.clone()),
        (200, serde_json::json!({"applications":[]})),
        (403, serde_json::json!({"error":"secret must not escape"})),
        (500, serde_json::json!({"error":"secret must not escape"})),
        (200, raw),
    ])
    .await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    let source = source(&provider);
    let apps = provider.applications(&source, range()).await.unwrap();
    assert_eq!(apps[0].status, Status::Ok);
    assert_eq!(apps[0].signals["cpu"].status, Status::Ok);
    assert_eq!(apps[0].signals["memory"].status, Status::Unknown);
    assert!(!apps[0].signals.contains_key("logs"));
    assert!(
        provider
            .applications(&source, range())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        provider.applications(&source, range()).await.unwrap_err(),
        ReadError::Refused
    );
    let failure = provider.applications(&source, range()).await.unwrap_err();
    assert_eq!(failure, ReadError::Failed);
    assert!(!failure.to_string().contains("secret"));
    assert_eq!(
        provider.applications(&source, range()).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn project_selection_is_explicit() {
    let server = server(vec![(
        200,
        serde_json::json!({"projects":[{"id":"p","name":"Production"}]}),
    )])
    .await;
    let provider = Provider::new(&server.url, Credentials::ApiKey("fake-key".into())).unwrap();
    let projects = provider.projects().await.unwrap();
    let source = provider.source(&projects[0]);
    let other = Provider::new(&server.url, Credentials::None).unwrap();
    assert_ne!(provider.id(), other.id());
    assert_eq!(
        other.applications(&source, range()).await.unwrap_err(),
        ReadError::InvalidSelection
    );
}

#[tokio::test(start_paused = true)]
async fn read_deadline_includes_waiting_for_a_slot() {
    let provider = Provider::new("http://127.0.0.1:1", Credentials::None).unwrap();
    let started = tokio::time::Instant::now();
    let read = || {
        provider.read(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok::<(), coroot_rs::Error>(())
        })
    };
    // The third read waits for a slot: its deadline starts with the other
    // two, rather than granting another 20 seconds after acquiring one.
    let results = tokio::join!(read(), read(), read());
    assert_eq!(
        results,
        (
            Err(ReadError::Timeout),
            Err(ReadError::Timeout),
            Err(ReadError::Timeout)
        )
    );
    assert_eq!(started.elapsed(), Duration::from_secs(20));
}

#[tokio::test(start_paused = true)]
async fn cloned_providers_share_two_read_slots() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let provider = Provider::new("http://127.0.0.1:1", Credentials::None).unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let read = || {
        let (provider, active, peak) = (provider.clone(), active.clone(), peak.clone());
        async move {
            provider
                .read(async move {
                    let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(n, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                })
                .await
                .unwrap();
        }
    };
    let start = tokio::time::Instant::now();
    tokio::join!(read(), read(), read());
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    assert_eq!(start.elapsed(), Duration::from_secs(2));
}

#[tokio::test]
async fn oversized_response_is_rejected_before_it_can_become_empty_data() {
    let server = server(vec![(
        200,
        serde_json::json!({"pad":"x".repeat(8 * 1024 * 1024)}),
    )])
    .await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    assert_eq!(
        provider
            .applications(&source(&provider), range())
            .await
            .unwrap_err(),
        ReadError::Limit
    );
}

#[tokio::test]
async fn malformed_success_is_a_decode_failure_not_an_empty_collection() {
    let server = server(vec![
        (200, serde_json::json!({})),
        (200, serde_json::json!(null)),
    ])
    .await;
    let provider = Provider::new(&server.url, Credentials::None).unwrap();
    assert_eq!(
        provider
            .applications(&source(&provider), range())
            .await
            .unwrap_err(),
        ReadError::InvalidResponse
    );
    assert_eq!(
        provider
            .service_map(&source(&provider), range())
            .await
            .unwrap_err(),
        ReadError::InvalidResponse
    );
}
