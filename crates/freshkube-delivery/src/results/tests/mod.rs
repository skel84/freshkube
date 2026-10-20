mod server;

use std::future::Future;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use freshkube_core::secrets::{MemoryStore, SecretStore};
use freshkube_core::setup::{Forward, KeysJson, Results};
use freshkube_ui::real_io::until_io;
use gpui_kit::TestAppContext;
use serde_json::{Value, json};
use tokio::runtime::Runtime;
use tokio::time::Instant;

use super::*;
use server::{CERT, Reply, Server};

const SHA: &str = "0123456789012345678901234567890123456789";
const RESULT: &str = "acme/results/result-a";

fn io<T: Send + 'static>(
    cx: &mut TestAppContext,
    runtime: &Runtime,
    work: impl Future<Output = T> + Send + 'static,
) -> T {
    let (send, receive) = mpsc::sync_channel(1);
    runtime.spawn(async move {
        _ = send.send(work.await);
    });
    let mut answer = None;
    until_io(cx, "the Results client answer", |_| {
        match receive.try_recv() {
            Ok(value) => {
                answer = Some(value);
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => panic!("the Results worker stopped"),
        }
    });
    answer.unwrap()
}

fn settings(address: &str) -> Results {
    let mut settings = Results::default();
    settings.address = Some(address.into());
    settings
}

fn open(
    cx: &mut TestAppContext,
    runtime: &Runtime,
    settings: Results,
    store: Arc<MemoryStore>,
) -> Result<Client, Error> {
    io(cx, runtime, async move {
        Client::open(Some(&settings), store).await
    })
}

fn client(cx: &mut TestAppContext, runtime: &Runtime, server: &Server) -> Arc<Client> {
    Arc::new(
        open(cx, runtime, settings(&server.address), Arc::default())
            .unwrap_or_else(|error| panic!("{error}")),
    )
}

fn runs(
    cx: &mut TestAppContext,
    runtime: &Runtime,
    client: &Arc<Client>,
) -> Result<Listing<PipelineRun>, Error> {
    let client = client.clone();
    io(cx, runtime, async move {
        client
            .pipeline_runs(None, SHA, Instant::now() + Duration::from_secs(20))
            .await
    })
}

fn manifest(kind: &str, version: &str) -> Value {
    json!({
        "apiVersion": format!("tekton.dev/{version}"), "kind": kind,
        "metadata": { "namespace": "acme", "name": "checkout-7", "uid": "run-uid",
            "resourceVersion": "42", "labels": {
                SHA_LABEL: SHA, "tekton.dev/pipelineRun": "checkout-7"
            }, "annotations": { "chains.tekton.dev/signed": "true" }
        },
        "status": { "conditions": [{ "type": "Succeeded", "status": "True" }],
            "results": [{"name": "IMAGE_DIGEST", "value": "sha256:abcd"}],
            "taskResults": [{"name": "IMAGE_URL", "value": "registry.example.com/acme/checkout"}],
            "podName": "checkout-7-pod", "steps": [{ "container": "step-build" }]
        }
    })
}

fn record(value: Value) -> Value {
    json!({ "name": format!("{RESULT}/records/record-a"), "data": {
        "type": format!("{}.{}", value["apiVersion"].as_str().unwrap(), value["kind"].as_str().unwrap()),
        "value": STANDARD.encode(serde_json::to_vec(&value).unwrap())
    } })
}

fn page(records: Vec<Value>) -> Reply {
    Reply::json(json!({"records": records}))
}

#[gpui_kit::test]
fn records_reuse_the_live_run_parsers_and_get_only(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let pipeline = manifest("PipelineRun", "v1");
    let task = manifest("TaskRun", "v1beta1");
    let server = Server::new(
        &runtime,
        false,
        vec![
            page(vec![record(pipeline.clone())]),
            page(vec![record(task.clone())]),
        ],
    );
    let client = client(cx, &runtime, &server);
    let answer = runs(cx, &runtime, &client).unwrap();
    assert_eq!(
        answer.records[0].run,
        crate::tekton::parse_pipeline_run(&pipeline).unwrap()
    );
    assert_eq!(answer.records[0].result, RESULT);
    let tasks = io(cx, &runtime, async move {
        client
            .task_runs(RESULT, Instant::now() + Duration::from_secs(20))
            .await
    })
    .unwrap();
    assert_eq!(
        tasks.records[0].run,
        crate::tekton::parse_task_run(&task).unwrap()
    );
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|r| r.starts_with("GET ") && !r.contains("authorization:"))
    );
    let query = url::Url::parse(&format!(
        "http://127.0.0.1{}",
        requests[0].split_whitespace().nth(1).unwrap()
    ))
    .unwrap();
    assert_eq!(
        query.path(),
        "/apis/results.tekton.dev/v1alpha2/parents/-/results/-/records"
    );
    let pairs: std::collections::BTreeMap<_, _> = query.query_pairs().collect();
    assert!(pairs["filter"].contains(SHA_LABEL));
    assert!(pairs["filter"].contains("tekton.dev/v1beta1.PipelineRun"));
    assert_eq!(pairs["page_size"], "10");
    assert!(requests[1].contains("/parents/acme/results/result-a/records"));
}

#[gpui_kit::test]
fn off_and_not_forwarded_do_no_io(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let store = Arc::new(MemoryStore::default());
    store.unavailable.store(true, Ordering::SeqCst);
    let other = store.clone();
    assert!(matches!(
        io(cx, &runtime, async move { Client::open(None, other).await }),
        Err(Error::Off)
    ));
    let mut forward = Results::default();
    forward.forward = Some(Forward::new("acme", "results", 8080));
    assert!(matches!(
        open(cx, &runtime, forward, store),
        Err(Error::NotForwarded)
    ));
    assert_eq!(Error::Off.state(), State::Off);
    assert_eq!(Error::NotForwarded.state(), State::NotForwarded);
}

#[gpui_kit::test]
fn only_a_successful_empty_listing_is_empty(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let server = Server::new(
        &runtime,
        false,
        vec![
            Reply::json(json!({})),
            Reply::raw(404, "invented-secret"),
            Reply::raw(200, "not JSON"),
            Reply::json(json!({"error": "invented-secret"})),
        ],
    );
    let client = client(cx, &runtime, &server);
    let mut snapshot = Snapshot::default();
    let request = snapshot.begin((client.id(), SHA));
    snapshot.apply(&request, runs(cx, &runtime, &client));
    assert_eq!(state(&snapshot), State::Empty);
    assert_eq!(runs(cx, &runtime, &client), Err(Error::NotInstalled));
    assert_eq!(Error::NotInstalled.state(), State::NotInstalled);
    for _ in 0..2 {
        let error = runs(cx, &runtime, &client).unwrap_err();
        assert_eq!(error.state(), State::Unreadable);
        assert!(!format!("{error:?} {error}").contains("invented-secret"));
    }
}

#[gpui_kit::test]
fn refusals_name_the_status_and_missing_token(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let server = Server::new(
        &runtime,
        false,
        vec![
            Reply::raw(401, "invented-secret"),
            Reply::raw(403, "invented-secret"),
        ],
    );
    let client = client(cx, &runtime, &server);
    for status in [401, 403] {
        let error = runs(cx, &runtime, &client).unwrap_err();
        assert_eq!(
            error,
            Error::Refused {
                status,
                token_set: false
            }
        );
        assert_eq!(error.state(), State::Refused);
        assert!(error.to_string().contains("no token is set"));
        assert!(!format!("{error:?} {error}").contains("invented-secret"));
    }
}

#[gpui_kit::test]
fn the_token_is_only_the_results_accounts_key_and_redirects_are_not_followed(
    cx: &mut TestAppContext,
) {
    let runtime = Runtime::new().unwrap();
    let target = Server::new(&runtime, false, vec![]);
    let mut redirect = Reply::raw(302, "invented-secret");
    redirect.headers = format!("Location: {}/elsewhere\r\n", target.address);
    let server = Server::new(&runtime, false, vec![page(vec![]), redirect]);
    let store = Arc::new(MemoryStore::default());
    store
        .write(
            &token_account(&server.address).unwrap(),
            "invented-results-token",
        )
        .unwrap();
    store
        .write(
            &format!("Prometheus {}/", server.address),
            "invented-other-token",
        )
        .unwrap();
    let mut settings = settings(&format!("{}/prefix", server.address));
    // Different paths have different accounts too.
    store
        .write(
            &token_account(settings.address.as_ref().unwrap()).unwrap(),
            "invented-prefix-token",
        )
        .unwrap();
    settings.token = Some(KeysJson);
    let client = Arc::new(open(cx, &runtime, settings, store).unwrap_or_else(|e| panic!("{e}")));
    assert!(runs(cx, &runtime, &client).is_ok());
    assert_eq!(runs(cx, &runtime, &client), Err(Error::Http(302)));
    let requests = server.requests.lock().unwrap();
    assert!(requests[0].starts_with("GET /prefix/apis/"));
    assert!(
        requests
            .iter()
            .all(|r| r.contains("authorization: Bearer invented-prefix-token\r\n"))
    );
    assert!(target.requests.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn a_requested_missing_key_is_anonymous_but_a_store_failure_is_not(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let server = Server::new(&runtime, false, vec![Reply::raw(403, "")]);
    let store = Arc::new(MemoryStore::default());
    let mut settings = settings(&server.address);
    settings.token = Some(KeysJson);
    let client = Arc::new(
        open(cx, &runtime, settings.clone(), store.clone()).unwrap_or_else(|e| panic!("{e}")),
    );
    assert_eq!(
        runs(cx, &runtime, &client),
        Err(Error::Refused {
            status: 403,
            token_set: false
        })
    );
    store.unavailable.store(true, Ordering::SeqCst);
    assert!(matches!(
        open(cx, &runtime, settings, store),
        Err(Error::Unreadable(_))
    ));
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[gpui_kit::test]
fn only_literal_loopback_allows_plain_http_and_settings_errors_leak_no_values(
    cx: &mut TestAppContext,
) {
    let runtime = Runtime::new().unwrap();
    for address in [
        "http://results.example.com",
        "http://localhost",
        "https://user:invented-secret@results.example.com",
        "https://results.example.com/?token=invented-secret",
        "https://results.example.com/#invented-secret",
        "file:///invented-secret",
    ] {
        let error = open(cx, &runtime, settings(address), Arc::default())
            .err()
            .unwrap();
        assert_eq!(error.state(), State::Unreadable);
        assert!(!format!("{error:?} {error}").contains("invented-secret"));
    }
    assert!(transport::base_url("http://[::1]:8080").is_ok());
    assert_eq!(
        token_account("https://results.example.com").unwrap(),
        token_account("https://results.example.com/").unwrap()
    );
}

#[gpui_kit::test]
fn tls_needs_a_trusted_ca_and_never_skips_verification(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let server = Server::new(&runtime, true, vec![page(vec![])]);
    let anonymous = client(cx, &runtime, &server);
    assert_eq!(
        runs(cx, &runtime, &anonymous),
        Err(Error::Unreachable(Cause::Certificate))
    );
    assert!(server.requests.lock().unwrap().is_empty());
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("results-ca.pem");
    std::fs::write(&ca, CERT).unwrap();
    let mut settings = settings(&server.address);
    settings.ca_file = Some(ca.clone());
    let trusted = Arc::new(
        open(cx, &runtime, settings.clone(), Arc::default()).unwrap_or_else(|e| panic!("{e}")),
    );
    assert!(runs(cx, &runtime, &trusted).is_ok());
    assert_ne!(anonymous.id(), trusted.id());
    std::fs::write(&ca, "invented-secret").unwrap();
    let error = open(cx, &runtime, settings.clone(), Arc::default())
        .err()
        .unwrap();
    assert!(!format!("{error:?} {error}").contains("invented-secret"));
    std::fs::remove_file(&ca).unwrap();
    assert!(matches!(
        open(cx, &runtime, settings, Arc::default()),
        Err(Error::Unreadable(_))
    ));
}

#[gpui_kit::test]
fn invalid_stored_tokens_never_reach_the_server(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let server = Server::new(&runtime, false, vec![]);
    for key in [
        "",
        "  ",
        "invented-secret\r\nInjected: yes",
        "invented secret",
    ] {
        let store = Arc::new(MemoryStore::default());
        store
            .write(&token_account(&server.address).unwrap(), key)
            .unwrap();
        let mut settings = settings(&server.address);
        settings.token = Some(KeysJson);
        let error = open(cx, &runtime, settings, store).err().unwrap();
        assert_eq!(error.state(), State::Unreadable);
        assert!(!format!("{error:?} {error}").contains("invented-secret"));
    }
    assert!(server.requests.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn failed_refresh_keeps_the_last_answer_and_its_time_only_for_the_same_identity(
    cx: &mut TestAppContext,
) {
    let runtime = Runtime::new().unwrap();
    let server = Server::new(
        &runtime,
        false,
        vec![
            page(vec![record(manifest("PipelineRun", "v1"))]),
            Reply::raw(503, "invented-secret"),
        ],
    );
    let client = client(cx, &runtime, &server);
    let mut snapshot = Snapshot::default();
    let identity = (client.id(), SHA);
    let request = snapshot.begin(identity);
    snapshot.apply(&request, runs(cx, &runtime, &client));
    assert_eq!(state(&snapshot), State::Read);
    let successful = snapshot.last_successful();
    let request = snapshot.begin(identity);
    snapshot.apply(&request, runs(cx, &runtime, &client));
    assert_eq!(state(&snapshot), State::Stale);
    assert_eq!(snapshot.data().unwrap().records.len(), 1);
    assert_eq!(snapshot.last_successful(), successful);
    assert_eq!(snapshot.error(), Some(&Error::Http(503)));
    assert!(snapshot.last_failure().is_some());
    let old = snapshot.begin(identity);
    let new = snapshot.begin((client.id(), "different-sha"));
    assert!(snapshot.data().is_none());
    assert!(snapshot.last_successful().is_none());
    assert!(!snapshot.apply(
        &old,
        Ok(Listing {
            records: vec![],
            cap: None,
            total: Some(0)
        })
    ));
    snapshot.apply(&new, Err(Error::Unreachable(Cause::Connect)));
    assert_eq!(state(&snapshot), State::Unreachable);
}

#[gpui_kit::test]
fn connection_failure_is_unreachable_not_empty(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let server = Server::new(&runtime, false, vec![]);
    let client = client(cx, &runtime, &server);
    drop(server);
    // Wait for the server task to drop its listener before making the request.
    // The transport still classifies either a reset or a refused connection.
    let error = runs(cx, &runtime, &client).unwrap_err();
    assert_eq!(error.state(), State::Unreachable);
}

#[gpui_kit::test]
fn request_deadline_covers_a_body_that_never_finishes(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let mut reply = Reply::raw(200, vec![b' '; 1]);
    reply.hold = true;
    let server = Server::new(&runtime, false, vec![reply]);
    let client = client(cx, &runtime, &server);
    assert_eq!(
        runs(cx, &runtime, &client),
        Err(Error::Unreachable(Cause::TimedOut))
    );
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[gpui_kit::test]
fn overall_deadline_is_not_extended_and_expired_reads_make_no_request(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let mut reply = Reply::raw(200, vec![b' '; 1]);
    reply.hold = true;
    let server = Server::new(&runtime, false, vec![reply]);
    let client = client(cx, &runtime, &server);
    let clone = client.clone();
    assert_eq!(
        io(cx, &runtime, async move {
            clone.pipeline_runs(None, SHA, Instant::now()).await
        }),
        Err(Error::Unreachable(Cause::TimedOut))
    );
    assert!(server.requests.lock().unwrap().is_empty());
    assert_eq!(
        io(cx, &runtime, async move {
            client
                .pipeline_runs(None, SHA, Instant::now() + Duration::from_millis(200))
                .await
        }),
        Err(Error::Unreachable(Cause::TimedOut))
    );
}

#[gpui_kit::test]
fn pagination_encodes_tokens_and_stops_at_the_run_cap(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let record = record(manifest("PipelineRun", "v1beta1"));
    let server = Server::new(
        &runtime,
        false,
        vec![
            Reply::json(json!({"records": [record.clone()], "nextPageToken": "next/+?=&"})),
            Reply::json(json!({"records": vec![record; 9], "next_page_token": "more"})),
        ],
    );
    let client = client(cx, &runtime, &server);
    let answer = runs(cx, &runtime, &client).unwrap();
    assert_eq!(answer.records.len(), MOST_RESULT_RUNS);
    assert_eq!(answer.cap, Some(Cap::Runs));
    assert_eq!(answer.total, None);
    let mut snapshot = Snapshot::default();
    let request = snapshot.begin(client.id());
    snapshot.apply(&request, Ok::<_, Error>(answer));
    assert_eq!(state(&snapshot), State::Capped(Cap::Runs));
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let url = url::Url::parse(&format!(
        "http://127.0.0.1{}",
        requests[1].split_whitespace().nth(1).unwrap()
    ))
    .unwrap();
    assert!(
        url.query_pairs()
            .any(|(key, value)| key == "page_token" && value == "next/+?=&")
    );
    assert!(
        url.query_pairs()
            .any(|(key, value)| key == "page_size" && value == "9")
    );
}

#[gpui_kit::test]
fn a_server_ignoring_the_page_size_cannot_bypass_the_run_cap(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let server = Server::new(
        &runtime,
        false,
        vec![page(vec![record(manifest("PipelineRun", "v1")); 14])],
    );
    let client = client(cx, &runtime, &server);
    let answer = runs(cx, &runtime, &client).unwrap();
    assert_eq!(answer.records.len(), 10);
    assert_eq!(answer.cap, Some(Cap::Runs));
    assert_eq!(answer.total, Some(14));
}

#[gpui_kit::test]
fn page_cap_is_capped_even_if_no_records_arrived(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let replies = (0..crate::read::MAX_PAGES)
        .map(|ix| Reply::json(json!({"records": [], "nextPageToken": format!("page-{ix}")})))
        .collect();
    let server = Server::new(&runtime, false, replies);
    let client = client(cx, &runtime, &server);
    let answer = runs(cx, &runtime, &client).unwrap();
    assert_eq!(answer.cap, Some(Cap::Pages));
    assert!(answer.records.is_empty());
    assert_eq!(
        server.requests.lock().unwrap().len(),
        crate::read::MAX_PAGES as usize
    );
}

#[gpui_kit::test]
fn repeated_page_tokens_are_unreadable_not_empty(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let server = Server::new(
        &runtime,
        false,
        vec![
            Reply::json(json!({"nextPageToken": "same"})),
            Reply::json(json!({"nextPageToken": "same"})),
        ],
    );
    let client = client(cx, &runtime, &server);
    assert!(matches!(
        runs(cx, &runtime, &client),
        Err(Error::Unreadable(_))
    ));
    assert_eq!(server.requests.lock().unwrap().len(), 2);
}

#[gpui_kit::test]
fn both_sized_and_chunked_bodies_have_a_byte_cap(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    for chunked in [false, true] {
        let mut reply = Reply::raw(200, vec![b' '; crate::read::MAX_BODY_BYTES + 1]);
        reply.chunked = chunked;
        let first = Reply::json(json!({
            "records": [record(manifest("PipelineRun", "v1"))], "nextPageToken": "later"
        }));
        let server = Server::new(&runtime, false, vec![first, reply]);
        let client = client(cx, &runtime, &server);
        let answer = runs(cx, &runtime, &client).unwrap();
        assert_eq!(answer.cap, Some(Cap::Bytes));
        assert_eq!(answer.records.len(), 1);
        assert_eq!(answer.total, None);
        let mut snapshot = Snapshot::default();
        let request = snapshot.begin(client.id());
        snapshot.apply(&request, Ok::<_, Error>(answer));
        assert_eq!(state(&snapshot), State::Capped(Cap::Bytes));
    }
}

#[gpui_kit::test]
fn malformed_or_out_of_scope_records_never_become_empty_history(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let good = record(manifest("PipelineRun", "v1"));
    let mut cases = vec![];
    for (pointer, value) in [
        ("/name", json!("../../results/result-a/records/record-a")),
        ("/data/type", json!("tekton.dev/v2.PipelineRun")),
        ("/data/type", json!("tekton.dev/v1.TaskRun")),
        ("/data/value", json!("not base64! invented-secret")),
        (
            "/data/value",
            json!(STANDARD.encode(b"not JSON invented-secret")),
        ),
        ("/data/value", json!("a".repeat(6 * 1024 * 1024))),
    ] {
        let mut bad = good.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        cases.push(bad);
    }
    for (pointer, value) in [
        ("/kind", json!("TaskRun")),
        ("/apiVersion", json!("tekton.dev/v1beta1")),
        ("/metadata/uid", Value::Null),
        ("/metadata/name", Value::Null),
        ("/metadata/namespace", json!("other")),
        (
            "/metadata/labels/pipelinesascode.tekton.dev~1sha",
            json!("different-sha"),
        ),
    ] {
        let mut bad = manifest("PipelineRun", "v1");
        *bad.pointer_mut(pointer).unwrap() = value;
        let mut wrapped = good.clone();
        wrapped["data"]["value"] = json!(STANDARD.encode(serde_json::to_vec(&bad).unwrap()));
        cases.push(wrapped);
    }
    let count = cases.len();
    let server = Server::new(
        &runtime,
        false,
        cases.into_iter().map(|r| page(vec![r])).collect(),
    );
    let client = client(cx, &runtime, &server);
    for _ in 0..count {
        let error = runs(cx, &runtime, &client).unwrap_err();
        assert_eq!(error.state(), State::Unreadable);
        assert!(!format!("{error:?} {error}").contains("invented-secret"));
    }
}

#[gpui_kit::test]
fn task_reads_are_scoped_and_capped(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let good = record(manifest("TaskRun", "v1"));
    let mut wrong = good.clone();
    wrong["name"] = json!("acme/results/other/records/record-a");
    let server = Server::new(
        &runtime,
        false,
        vec![page(vec![wrong]), page(vec![good; MOST_RESULT_TASKS + 1])],
    );
    let client = client(cx, &runtime, &server);
    let clone = client.clone();
    let error = io(cx, &runtime, async move {
        clone
            .task_runs(RESULT, Instant::now() + Duration::from_secs(20))
            .await
    })
    .unwrap_err();
    assert_eq!(error.state(), State::Unreadable);
    let answer = io(cx, &runtime, async move {
        client
            .task_runs(RESULT, Instant::now() + Duration::from_secs(20))
            .await
    })
    .unwrap();
    assert_eq!(answer.records.len(), MOST_RESULT_TASKS);
    assert_eq!(answer.cap, Some(Cap::Tasks));
    assert_eq!(answer.total, Some(MOST_RESULT_TASKS + 1));
}

#[gpui_kit::test]
fn invalid_query_components_make_no_request(cx: &mut TestAppContext) {
    let runtime = Runtime::new().unwrap();
    let server = Server::new(&runtime, false, vec![]);
    let client = client(cx, &runtime, &server);
    io(cx, &runtime, async move {
        let until = Instant::now() + Duration::from_secs(20);
        assert!(
            client
                .pipeline_runs(None, "' || true", until)
                .await
                .is_err()
        );
        assert!(
            client
                .pipeline_runs(Some("../other"), SHA, until)
                .await
                .is_err()
        );
        for result in [
            "https://results.example.com",
            "acme/results/../other",
            "-/results/result-a",
            "acme/results/-",
            "acme/results/result-a?x=y",
        ] {
            assert!(client.task_runs(result, until).await.is_err());
        }
    });
    assert!(server.requests.lock().unwrap().is_empty());
}
