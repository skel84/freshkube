mod builds;
mod forward;
mod logs;
mod server;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use freshkube_core::secrets::{MemoryStore, SecretStore};
use freshkube_core::setup::{Forward, KeysJson, Results};
use serde_json::{Value, json};
use tokio::time::{Instant, timeout};

use super::*;
use server::{CERT, Reply, Server};

const SHA: &str = "0123456789012345678901234567890123456789";
const RESULT: &str = "acme/results/result-a";

async fn within<T>(work: impl Future<Output = T>) -> T {
    timeout(Duration::from_secs(30), work)
        .await
        .expect("the Results client did not answer within 30 seconds")
}

fn settings(address: &str) -> Results {
    let mut settings = Results::default();
    settings.address = Some(address.into());
    settings
}

async fn open(settings: Results, store: Arc<MemoryStore>) -> Result<Client, Error> {
    within(Client::open(Some(&settings), Some(store))).await
}

async fn client(server: &Server) -> Arc<Client> {
    Arc::new(
        open(settings(&server.address), Arc::default())
            .await
            .unwrap_or_else(|error| panic!("{error}")),
    )
}

async fn runs(client: &Client) -> Result<Listing<PipelineRun>, Error> {
    within(client.pipeline_runs(None, SHA, Instant::now() + Duration::from_secs(20))).await
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

#[tokio::test(flavor = "multi_thread")]
async fn records_reuse_the_live_run_parsers_and_get_only() {
    let pipeline = manifest("PipelineRun", "v1");
    let task = manifest("TaskRun", "v1beta1");
    let server = Server::new(
        false,
        vec![
            page(vec![record(pipeline.clone())]),
            page(vec![record(task.clone())]),
        ],
    );
    let client = client(&server).await;
    let answer = runs(&client).await.unwrap();
    assert_eq!(
        answer.records[0].run,
        crate::tekton::parse_pipeline_run(&pipeline).unwrap()
    );
    assert_eq!(answer.records[0].result, RESULT);
    let tasks = within(async move {
        client
            .task_runs(RESULT, Instant::now() + Duration::from_secs(20))
            .await
    })
    .await
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

#[tokio::test(flavor = "multi_thread")]
async fn recreated_runs_retain_distinct_uids_even_when_their_names_match() {
    let old = manifest("PipelineRun", "v1");
    let mut recreated = old.clone();
    recreated["metadata"]["uid"] = json!("recreated-run-uid");
    let mut newer = record(recreated);
    newer["name"] = json!(format!("{RESULT}/records/record-b"));
    let server = Server::new(false, vec![page(vec![record(old), newer])]);
    let client = client(&server).await;
    let answer = runs(&client).await.unwrap();
    assert_eq!(answer.records.len(), 2);
    let old = &answer.records[0].run;
    let recreated = &answer.records[1].run;
    assert_eq!(old.namespace, recreated.namespace);
    assert_eq!(old.name, recreated.name);
    assert_eq!(old.meta.uid.as_deref(), Some("run-uid"));
    assert_eq!(recreated.meta.uid.as_deref(), Some("recreated-run-uid"));
}

#[tokio::test(flavor = "multi_thread")]
async fn archived_annotations_preserve_the_bytes_checked_by_the_verifier() {
    use crate::digest::Digest;
    use crate::verify::{Trust, TrustedKey, Verification, verify};

    let mut pipeline = manifest("PipelineRun", "v1");
    pipeline["metadata"]["annotations"]["chains.tekton.dev/signature-pipelinerun-run-uid"] =
        json!(STANDARD.encode(include_bytes!("../../../testdata/cosign/checkout.att.json")));
    // The payload annotation must contain the exact signed bytes, independent
    // of how a checkout represents line endings in the readable statement.
    let envelope: Value =
        serde_json::from_slice(include_bytes!("../../../testdata/cosign/checkout.att.json"))
            .unwrap();
    pipeline["metadata"]["annotations"]["chains.tekton.dev/payload-pipelinerun-run-uid"] =
        envelope["payload"].clone();
    let server = Server::new(false, vec![page(vec![record(pipeline.clone())])]);
    let client = client(&server).await;
    let answer = runs(&client).await.unwrap();
    let archived = &answer.records[0].run;
    let live = crate::tekton::parse_pipeline_run(&pipeline).unwrap();
    assert_eq!(archived.attestations, live.attestations);
    assert_eq!(archived.attestations.read.len(), 1);
    let trust = Trust::Key(
        TrustedKey::from_pem(include_str!("../../../testdata/cosign/checkout.pub")).unwrap(),
    );
    let digest =
        Digest::parse("sha256:f4f5ecadeff863529ffa5690e4f100ee235e9a2b9cd19733697b2106ebdc2aa6")
            .unwrap();
    assert!(matches!(
        verify(&trust, &archived.attestations.read[0], &digest),
        Verification::Verified(_)
    ));
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn off_and_not_forwarded_do_no_io() {
    let store = Arc::new(MemoryStore::default());
    store.unavailable.store(true, Ordering::SeqCst);
    let other = store.clone();
    assert!(matches!(
        within(async move { Client::open(None, Some(other)).await }).await,
        Err(Error::Off)
    ));
    let mut forward = Results::default();
    forward.forward = Some(Forward::new("acme", "results", 8080));
    assert!(matches!(
        open(forward, store).await,
        Err(Error::NotForwarded)
    ));
    assert_eq!(Error::Off.state(), State::Off);
    assert_eq!(Error::NotForwarded.state(), State::NotForwarded);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_successful_empty_listing_is_empty() {
    let server = Server::new(
        false,
        vec![
            Reply::json(json!({})),
            Reply::raw(404, "invented-secret"),
            Reply::raw(200, "not JSON"),
            Reply::json(json!({"error": "invented-secret"})),
        ],
    );
    let client = client(&server).await;
    let mut snapshot = Snapshot::default();
    let request = snapshot.begin((client.id(), SHA));
    snapshot.apply(&request, runs(&client).await);
    assert_eq!(state(&snapshot), State::Empty);
    assert_eq!(runs(&client).await, Err(Error::NotInstalled));
    assert_eq!(Error::NotInstalled.state(), State::NotInstalled);
    for _ in 0..2 {
        let error = runs(&client).await.unwrap_err();
        assert_eq!(error.state(), State::Unreadable);
        assert!(!format!("{error:?} {error}").contains("invented-secret"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn refusals_name_the_status_and_missing_token() {
    let server = Server::new(
        false,
        vec![
            Reply::raw(401, "invented-secret"),
            Reply::raw(403, "invented-secret"),
        ],
    );
    let client = client(&server).await;
    for status in [401, 403] {
        let error = runs(&client).await.unwrap_err();
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

#[tokio::test(flavor = "multi_thread")]
async fn the_token_is_only_the_results_accounts_key_and_redirects_are_not_followed() {
    let target = Server::new(false, vec![]);
    let mut redirect = Reply::raw(302, "invented-secret");
    redirect.headers = format!("Location: {}/elsewhere\r\n", target.address);
    let server = Server::new(false, vec![page(vec![]), redirect]);
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
    let client = Arc::new(
        open(settings, store)
            .await
            .unwrap_or_else(|e| panic!("{e}")),
    );
    assert!(runs(&client).await.is_ok());
    assert_eq!(runs(&client).await, Err(Error::Http(302)));
    let requests = server.requests.lock().unwrap();
    assert!(requests[0].starts_with("GET /prefix/apis/"));
    assert!(
        requests
            .iter()
            .all(|r| r.contains("authorization: Bearer invented-prefix-token\r\n"))
    );
    assert!(target.requests.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_requested_missing_key_is_anonymous_but_a_store_failure_is_not() {
    let server = Server::new(false, vec![Reply::raw(403, "")]);
    let store = Arc::new(MemoryStore::default());
    let mut settings = settings(&server.address);
    settings.token = Some(KeysJson);
    let client = Arc::new(
        open(settings.clone(), store.clone())
            .await
            .unwrap_or_else(|e| panic!("{e}")),
    );
    assert_eq!(
        runs(&client).await,
        Err(Error::Refused {
            status: 403,
            token_set: false
        })
    );
    store.unavailable.store(true, Ordering::SeqCst);
    assert!(matches!(
        open(settings, store).await,
        Err(Error::Unreadable(_))
    ));
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_literal_loopback_allows_plain_http_and_settings_errors_leak_no_values() {
    for address in [
        "http://results.example.com",
        "http://localhost",
        "https://user:invented-secret@results.example.com",
        "https://results.example.com/?token=invented-secret",
        "https://results.example.com/#invented-secret",
        "file:///invented-secret",
    ] {
        let error = open(settings(address), Arc::default()).await.err().unwrap();
        assert_eq!(error.state(), State::Unreadable);
        assert!(!format!("{error:?} {error}").contains("invented-secret"));
    }
    assert!(transport::base_url("http://[::1]:8080").is_ok());
    assert_eq!(
        token_account("https://results.example.com").unwrap(),
        token_account("https://results.example.com/").unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tls_needs_a_trusted_ca_and_never_skips_verification() {
    let server = Server::new(true, vec![page(vec![])]);
    let anonymous = client(&server).await;
    assert_eq!(
        runs(&anonymous).await,
        Err(Error::Unreachable(Cause::Certificate))
    );
    assert!(server.requests.lock().unwrap().is_empty());
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("results-ca.pem");
    std::fs::write(&ca, CERT).unwrap();
    let mut settings = settings(&server.address);
    settings.ca_file = Some(ca.clone());
    let trusted = Arc::new(
        open(settings.clone(), Arc::default())
            .await
            .unwrap_or_else(|e| panic!("{e}")),
    );
    assert!(runs(&trusted).await.is_ok());
    assert_ne!(anonymous.id(), trusted.id());
    std::fs::write(&ca, "invented-secret").unwrap();
    let error = open(settings.clone(), Arc::default()).await.err().unwrap();
    assert!(!format!("{error:?} {error}").contains("invented-secret"));
    std::fs::remove_file(&ca).unwrap();
    assert!(matches!(
        open(settings, Arc::default()).await,
        Err(Error::Unreadable(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_stored_tokens_never_reach_the_server() {
    let server = Server::new(false, vec![]);
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
        let error = open(settings, store).await.err().unwrap();
        assert_eq!(error.state(), State::Unreadable);
        assert!(!format!("{error:?} {error}").contains("invented-secret"));
    }
    assert!(server.requests.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_refresh_keeps_the_last_answer_and_its_time_only_for_the_same_identity() {
    let server = Server::new(
        false,
        vec![
            page(vec![record(manifest("PipelineRun", "v1"))]),
            Reply::raw(503, "invented-secret"),
        ],
    );
    let client = client(&server).await;
    let mut snapshot = Snapshot::default();
    let identity = (client.id(), SHA);
    let request = snapshot.begin(identity);
    assert_eq!(state(&snapshot), State::Loading);
    snapshot.apply(&request, runs(&client).await);
    assert_eq!(state(&snapshot), State::Read);
    let successful = snapshot.last_successful();
    let request = snapshot.begin(identity);
    assert_eq!(state(&snapshot), State::Loading);
    snapshot.apply(&request, runs(&client).await);
    assert_eq!(state(&snapshot), State::Stale);
    assert_eq!(snapshot.data().unwrap().records.len(), 1);
    assert_eq!(snapshot.last_successful(), successful);
    assert_eq!(
        snapshot.error(),
        Some(&Error::Unreachable(Cause::Gateway(503)))
    );
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

#[tokio::test(flavor = "multi_thread")]
async fn connection_failure_is_unreachable_not_empty() {
    // Drop the std listener before the request, so this test does not race
    // the asynchronous shutdown of the fake server's listener.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let client = open(settings(&address), Arc::default())
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let error = runs(&client).await.unwrap_err();
    assert_eq!(error.state(), State::Unreachable);
}

#[tokio::test(flavor = "multi_thread")]
async fn request_deadline_covers_a_body_that_never_finishes() {
    let mut reply = Reply::raw(200, vec![b' '; 1]);
    reply.hold = true;
    let server = Server::new(false, vec![reply]);
    let client = client(&server).await;
    assert_eq!(
        runs(&client).await,
        Err(Error::Unreachable(Cause::TimedOut))
    );
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn overall_deadline_is_not_extended_and_expired_reads_make_no_request() {
    let mut reply = Reply::raw(200, vec![b' '; 1]);
    reply.hold = true;
    let server = Server::new(false, vec![reply]);
    let client = client(&server).await;
    let clone = client.clone();
    assert_eq!(
        within(async move { clone.pipeline_runs(None, SHA, Instant::now()).await }).await,
        Err(Error::Unreachable(Cause::TimedOut))
    );
    assert!(server.requests.lock().unwrap().is_empty());
    let started = Instant::now();
    assert_eq!(
        within(async move {
            client
                .pipeline_runs(None, SHA, Instant::now() + Duration::from_millis(200))
                .await
        })
        .await,
        Err(Error::Unreachable(Cause::TimedOut))
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the overall deadline was extended to the request timeout"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pagination_encodes_tokens_and_stops_at_the_run_cap() {
    let record = record(manifest("PipelineRun", "v1beta1"));
    let server = Server::new(
        false,
        vec![
            Reply::json(json!({"records": [record.clone()], "nextPageToken": "next/+?=&"})),
            Reply::json(json!({"records": vec![record; 9], "next_page_token": "more"})),
        ],
    );
    let client = client(&server).await;
    let answer = runs(&client).await.unwrap();
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

#[tokio::test(flavor = "multi_thread")]
async fn a_server_ignoring_the_page_size_cannot_bypass_the_run_cap() {
    let server = Server::new(
        false,
        vec![page(vec![record(manifest("PipelineRun", "v1")); 14])],
    );
    let client = client(&server).await;
    let answer = runs(&client).await.unwrap();
    assert_eq!(answer.records.len(), 10);
    assert_eq!(answer.cap, Some(Cap::Runs));
    assert_eq!(answer.total, Some(14));
}

#[tokio::test(flavor = "multi_thread")]
async fn page_cap_is_capped_even_if_no_records_arrived() {
    let replies = (0..crate::read::MAX_PAGES)
        .map(|ix| Reply::json(json!({"records": [], "nextPageToken": format!("page-{ix}")})))
        .collect();
    let server = Server::new(false, replies);
    let client = client(&server).await;
    let answer = runs(&client).await.unwrap();
    assert_eq!(answer.cap, Some(Cap::Pages));
    assert!(answer.records.is_empty());
    assert_eq!(answer.total, None);
    assert_eq!(
        server.requests.lock().unwrap().len(),
        crate::read::MAX_PAGES as usize
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_page_tokens_are_unreadable_not_empty() {
    let server = Server::new(
        false,
        vec![
            Reply::json(json!({"nextPageToken": "same"})),
            Reply::json(json!({"nextPageToken": "same"})),
        ],
    );
    let client = client(&server).await;
    assert!(matches!(runs(&client).await, Err(Error::Unreadable(_))));
    assert_eq!(server.requests.lock().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn both_sized_and_chunked_bodies_have_a_byte_cap() {
    for chunked in [false, true] {
        let mut reply = Reply::raw(200, vec![b' '; crate::read::MAX_BODY_BYTES + 1]);
        reply.chunked = chunked;
        let first = Reply::json(json!({
            "records": [record(manifest("PipelineRun", "v1"))], "nextPageToken": "later"
        }));
        let server = Server::new(false, vec![first, reply]);
        let client = client(&server).await;
        let answer = runs(&client).await.unwrap();
        assert_eq!(answer.cap, Some(Cap::Bytes));
        assert_eq!(answer.records.len(), 1);
        assert_eq!(answer.total, None);
        let mut snapshot = Snapshot::default();
        let request = snapshot.begin(client.id());
        snapshot.apply(&request, Ok::<_, Error>(answer));
        assert_eq!(state(&snapshot), State::Capped(Cap::Bytes));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_or_out_of_scope_records_never_become_empty_history() {
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
    let server = Server::new(false, cases.into_iter().map(|r| page(vec![r])).collect());
    let client = client(&server).await;
    for _ in 0..count {
        let error = runs(&client).await.unwrap_err();
        assert_eq!(error.state(), State::Unreadable);
        assert!(!format!("{error:?} {error}").contains("invented-secret"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn task_reads_are_scoped_and_capped() {
    let good = record(manifest("TaskRun", "v1"));
    let mut wrong = good.clone();
    wrong["name"] = json!("acme/results/other/records/record-a");
    let server = Server::new(
        false,
        vec![page(vec![wrong]), page(vec![good; MOST_RESULT_TASKS + 1])],
    );
    let client = client(&server).await;
    let clone = client.clone();
    let error = within(async move {
        clone
            .task_runs(RESULT, Instant::now() + Duration::from_secs(20))
            .await
    })
    .await
    .unwrap_err();
    assert_eq!(error.state(), State::Unreadable);
    let answer = within(async move {
        client
            .task_runs(RESULT, Instant::now() + Duration::from_secs(20))
            .await
    })
    .await
    .unwrap();
    assert_eq!(answer.records.len(), MOST_RESULT_TASKS);
    assert_eq!(answer.cap, Some(Cap::Tasks));
    assert_eq!(answer.total, Some(MOST_RESULT_TASKS + 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_query_components_make_no_request() {
    let server = Server::new(false, vec![]);
    let client = client(&server).await;
    within(async move {
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
    })
    .await;
    assert!(server.requests.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn finished_multi_page_listing_has_an_exact_total_without_a_cap() {
    let archived = record(manifest("PipelineRun", "v1"));
    let server = Server::new(
        false,
        vec![
            Reply::json(json!({"records": vec![archived.clone(); 3], "nextPageToken": "second"})),
            page(vec![archived; 2]),
        ],
    );
    let client = client(&server).await;
    let answer = runs(&client).await.unwrap();
    assert_eq!(answer.records.len(), 5);
    assert_eq!(answer.total, Some(5));
    assert_eq!(answer.cap, None);
    assert_eq!(server.requests.lock().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn unavailable_gateways_are_distinct_from_other_server_errors() {
    let statuses = [502, 503, 504, 500, 501, 505];
    let server = Server::new(
        false,
        statuses
            .iter()
            .map(|status| Reply::raw(*status, "invented-secret"))
            .collect(),
    );
    let client = client(&server).await;
    for status in statuses {
        let error = runs(&client).await.unwrap_err();
        if matches!(status, 502..=504) {
            assert_eq!(error, Error::Unreachable(Cause::Gateway(status)));
            assert_eq!(error.state(), State::Unreachable);
        } else {
            assert_eq!(error, Error::Http(status));
            assert_eq!(error.state(), State::Unreadable);
        }
        assert!(!format!("{error:?} {error}").contains("invented-secret"));
    }
}
