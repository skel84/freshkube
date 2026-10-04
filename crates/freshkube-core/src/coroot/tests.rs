use super::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
struct Server {
    url: String,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(responses: Vec<(u16, serde_json::Value)>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let responses = responses.clone();
            tokio::spawn(async move {
                let mut bytes = vec![0; 16384];
                let n = socket.read(&mut bytes).await.unwrap();
                assert!(n < bytes.len());
                let (status, body) = responses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or((404, serde_json::json!({})));
                let body = if body.get("applications").is_some() || body.get("map").is_some() {
                    serde_json::json!({"context":{},"data":body})
                } else {
                    body
                };
                let body = body.to_string();
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    Server { url, task }
}
fn range() -> TimeRange {
    let to = chrono::DateTime::from_timestamp(1_790_000_000, 0).unwrap();
    TimeRange::between(to - chrono::Duration::hours(1), to)
}
fn source(provider: &Provider) -> Source {
    provider.source(&ProjectInfo {
        id: "project-id".into(),
        name: "Display name".into(),
    })
}

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
async fn project_selection_and_report_capabilities_are_explicit() {
    let server=server(vec![(200,serde_json::json!({"projects":[{"id":"p","name":"Production"}]})),(200,serde_json::json!({"app_map":{"application":{"status":"warning"}},"reports":[{"name":"CPU","status":"warning","checks":[{"id":"CPU","title":"CPU","status":"warning","message":"Coroot finding"}]}]})),(404,serde_json::json!({}))]).await;
    let provider = Provider::new(&server.url, Credentials::ApiKey("fake-key".into())).unwrap();
    let projects = provider.projects().await.unwrap();
    let source = provider.source(&projects[0]);
    let id = AppId::new("c:prod:Deployment:api");
    let rest = provider
        .reports(&source, range(), &id, false)
        .await
        .unwrap();
    assert_eq!(rest.reports[0].issues[0].message, "Coroot finding");
    let extended = provider.reports(&source, range(), &id, true).await;
    assert_eq!(extended.unwrap_err(), ReadError::Unsupported);
    assert_eq!(rest.status, Status::Warning);
    let other = Provider::new(&server.url, Credentials::None).unwrap();
    assert_ne!(provider.id(), other.id());
    assert_eq!(
        other.applications(&source, range()).await.unwrap_err(),
        ReadError::InvalidSelection
    );
    assert_eq!(
        other
            .reports(&self::source(&other), range(), &id, true)
            .await
            .unwrap_err(),
        ReadError::Unsupported
    );
}
#[test]
fn association_requires_exact_access_cluster_and_kubernetes_kind() {
    let provider = Provider::new("https://coroot.example.com", Credentials::None).unwrap();
    let source = source(&provider);
    let id = AppId::new("cluster-a:prod:Deployment:api");
    assert!(ObjectSubject::for_app(&source, &id).is_none());
    let source = source.with_association(Some(Association::new(
        "access:opaque-a".into(),
        "cluster-a".into(),
    )));
    let subject = ObjectSubject::for_app(&source, &id).unwrap();
    assert_eq!(subject.access(), "access:opaque-a");
    assert_eq!(subject.namespace(), "prod");
    assert_eq!(subject.name(), "api");
    assert_eq!(subject.kind().kind, "Deployment");
    for id in [
        "cluster-a:prod:Deployment:../api",
        "cluster-a:prod/other:Pod:api",
        "cluster-a:prod:Pod:",
    ] {
        assert!(ObjectSubject::for_app(&source, &AppId::new(id)).is_none());
    }
    assert!(
        ObjectSubject::for_app(&source, &AppId::new("cluster-b:prod:Deployment:api")).is_none()
    );
    assert!(
        ObjectSubject::for_app(
            &source,
            &AppId::new("external:external:ExternalService:api:443")
        )
        .is_none()
    );
}

#[test]
fn empty_credentials_do_not_silently_select_anonymous_access() {
    for credentials in [
        Credentials::ApiKey(String::new()),
        Credentials::Session(" ".into()),
    ] {
        assert!(matches!(
            Provider::new("https://coroot.example.com", credentials),
            Err(ReadError::Authentication)
        ));
    }
    assert!(Provider::new("https://coroot.example.com", Credentials::None).is_ok());
}
#[test]
fn url_policy_and_bounds_fail_without_leaking_inputs() {
    for url in [
        "coroot.example.com",
        "https://user:secret@example.com",
        "https://example.com/?secret=x",
        "file:///etc/passwd",
    ] {
        assert!(matches!(
            Provider::new(url, Credentials::None),
            Err(ReadError::InvalidUrl)
        ));
    }
    let app = Application {
        id: AppId::new("c:prod:Deployment:api"),
        cluster: String::new(),
        category: String::new(),
        app_type: String::new(),
        status: Status::Unknown,
        signals: Default::default(),
    };
    assert_eq!(
        limits::applications(&vec![app.clone(); 2001]),
        Err(ReadError::Limit)
    );
    assert_eq!(
        limits::applications(&[app.clone(), app]),
        Err(ReadError::InvalidResponse)
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
async fn maps_keep_unknown_links_and_empty_maps_are_successful() {
    let server=server(vec![(200,serde_json::json!({"map":[
        {"id":"c:ns:Deployment:a","status":"ok","upstreams":[{"id":"c:ns:Deployment:b","status":"unknown"}]},
        {"id":"c:ns:Deployment:b","status":"unknown"}
    ]})),(200,serde_json::json!({"map":[]}))]).await;
    let provider = Provider::new(&server.url, Credentials::Session("fake-session".into())).unwrap();
    let source = source(&provider);
    let map = provider.service_map(&source, range()).await.unwrap();
    assert_eq!(map.nodes.len(), 2);
    assert_eq!(map.edges[0].status, Status::Unknown);
    assert!(map.edges[0].rps.is_none());
    assert!(
        provider
            .service_map(&source, range())
            .await
            .unwrap()
            .nodes
            .is_empty()
    );
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

#[test]
fn report_display_fields_are_bounded_individually() {
    let base = serde_json::json!({"id":"c:ns:Deployment:api","status":"unknown",
        "reports":[{"name":"Logs","status":"unknown","issues":[{"id":"i","title":"Title","status":"unknown","message":"Evidence"}],"log_patterns":[{"hash":"hash","severity":"info","sample":"Sanitized sample","messages":1}]}],
        "dependencies":[{"id":"c:ns:Service:db","connectivity":"unknown","connectivity_message":"Not reported","protocols":["postgres"]}],
        "clients":[{"id":"c:ns:Deployment:web","status":"ok"}]
    });
    let health: AppHealth = serde_json::from_value(base.clone()).unwrap();
    assert_eq!(limits::health(&health), Ok(()));
    for (path, limit) in [
        ("/reports/0/issues/0/id", 1024),
        ("/reports/0/log_patterns/0/hash", 256),
        ("/reports/0/log_patterns/0/severity", 64),
        ("/dependencies/0/id", 1024),
        ("/dependencies/0/connectivity_message", 4096),
        ("/dependencies/0/protocols/0", 64),
        ("/clients/0/id", 1024),
    ] {
        let mut oversized = base.clone();
        *oversized.pointer_mut(path).unwrap() = serde_json::json!("x".repeat(limit + 1));
        let health: AppHealth = serde_json::from_value(oversized).unwrap();
        assert_eq!(limits::health(&health), Err(ReadError::Limit), "{path}");
    }
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
