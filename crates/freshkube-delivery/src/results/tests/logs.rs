use super::builds::{change, evidence, pipeline, reader};
use super::*;
use crate::change::{Shows, Target};
use crate::results::logs::MOST_LOG_BYTES;

fn log(uid: &str, stored: bool) -> Value {
    let value = json!({"apiVersion": "results.tekton.dev/v1alpha3", "kind": "Log",
        "metadata": {"name": "checkout-7-log", "namespace": "acme", "uid": "log-uid"},
        "spec": {"resource": {"uid": uid, "kind": "TaskRun", "namespace": "acme", "name": "checkout-7"}},
        "status": {"isStored": stored, "size": 42}});
    record(value)
}

fn text(bytes: impl Into<Vec<u8>>) -> Reply {
    let mut reply = Reply::raw(200, bytes);
    reply.headers = "Content-Type: text/plain; charset=utf-8\r\n".into();
    reply
}

#[tokio::test(flavor = "multi_thread")]
async fn metadata_is_paged_and_content_is_a_whole_log_requested_only_on_demand() {
    let server = Server::new(
        false,
        vec![
            Reply::json(json!({"records": [log("other-task", true)], "nextPageToken": "second"})),
            page(vec![log("task-uid", true)]),
            text("[prepare] ready\n[build] complete\n"),
        ],
    );
    let client = super::client(&server).await;
    let listing = within(client.logs(RESULT, Instant::now() + Duration::from_secs(20)))
        .await
        .unwrap();
    assert_eq!(listing.records.len(), 2);
    assert_eq!(listing.records[1].run.task_uid, "task-uid");
    assert!(listing.records[1].run.stored);
    assert_eq!(server.requests.lock().unwrap().len(), 2);
    let content = within(client.log(
        RESULT,
        &listing.records[1].run.name,
        Instant::now() + Duration::from_secs(20),
    ))
    .await
    .unwrap();
    assert_eq!(content.text, "[prepare] ready\n[build] complete\n");
    assert_eq!(content.note(), None);
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].contains("/parents/acme/results/result-a/logs?"));
    assert!(requests[1].contains("page_token=second"));
    assert!(requests[2].starts_with(
        "GET /apis/results.tekton.dev/v1alpha2/parents/acme/results/result-a/logs/record-a "
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn log_content_distinguishes_exactly_one_mib_from_longer_streams() {
    for chunked in [false, true] {
        for length in [MOST_LOG_BYTES, MOST_LOG_BYTES + 1, MOST_LOG_BYTES * 2] {
            let mut reply = text(vec![b'x'; length]);
            reply.chunked = chunked;
            let server = Server::new(false, vec![reply]);
            let client = super::client(&server).await;
            let content =
                within(client.log(RESULT, "record-a", Instant::now() + Duration::from_secs(20)))
                    .await
                    .unwrap();
            assert_eq!(content.text.len(), MOST_LOG_BYTES);
            assert_eq!(content.capped, length > MOST_LOG_BYTES);
            assert_eq!(
                content.note(),
                (length > MOST_LOG_BYTES).then_some("Showing the first 1 MiB")
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_log_content_and_unserved_log_listing_have_separate_reasons() {
    let server = Server::new(
        false,
        vec![
            Reply::raw(404, "invented-error"),
            Reply::raw(501, "invented-error"),
            Reply::raw(404, "invented-error"),
            Reply::raw(403, "invented-error"),
        ],
    );
    let client = super::client(&server).await;
    for _ in 0..2 {
        assert_eq!(
            within(client.logs(RESULT, Instant::now() + Duration::from_secs(20))).await,
            Err(Error::LogsNotServed)
        );
    }
    let error = within(client.log(RESULT, "record-a", Instant::now() + Duration::from_secs(20)))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Tekton Results kept no log for this task"
    );
    let error = within(client.logs(RESULT, Instant::now() + Duration::from_secs(20)))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Refused { status: 403, .. }));
    assert!(!error.to_string().contains("invented-error"));
}

#[tokio::test(flavor = "multi_thread")]
async fn log_paths_and_deadlines_cannot_escape_the_configured_results_read() {
    let server = Server::new(false, vec![]);
    let client = super::client(&server).await;
    for name in [
        "-",
        "..",
        "../outside",
        "https://example.com/log",
        "log?token=x",
    ] {
        assert!(
            within(client.log(RESULT, name, Instant::now() + Duration::from_secs(20)))
                .await
                .is_err()
        );
    }
    assert!(
        within(client.log(RESULT, "record-a", Instant::now() - Duration::from_secs(1)))
            .await
            .is_err()
    );
    assert!(server.requests.lock().unwrap().is_empty());
    let mut reply = text("waiting");
    reply.hold = true;
    let server = Server::new(false, vec![reply]);
    let client = super::client(&server).await;
    assert_eq!(
        within(client.log(
            RESULT,
            "record-a",
            Instant::now() + Duration::from_millis(60)
        ))
        .await,
        Err(Error::Unreachable(Cause::TimedOut))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn log_availability_matches_task_uid_and_always_disables_the_pod_action() {
    for (uid, stored, expected) in [
        (
            "task-uid",
            true,
            "Tekton Results keeps this log; opening it here comes in a later update",
        ),
        (
            "other-task",
            true,
            "Tekton Results kept no log for this task",
        ),
        (
            "task-uid",
            false,
            "Tekton Results kept no log for this task",
        ),
    ] {
        let mut task = manifest("TaskRun", "v1");
        task["metadata"]["uid"] = json!("task-uid");
        let server = Server::new(
            false,
            vec![
                page(vec![record(pipeline("run-uid"))]),
                page(vec![record(task)]),
                page(vec![log(uid, stored)]),
            ],
        );
        let client = super::client(&server).await;
        let evidence = evidence(&reader(vec![], vec![]), Ok(&client)).await;
        let change = change(&evidence);
        let row = change
            .hops
            .iter()
            .find(|row| row.name == "Run checkout-7")
            .unwrap();
        let Shows::Hop(detail) = &row.shows else {
            panic!();
        };
        let logs = detail
            .actions
            .iter()
            .find(|action| matches!(action.target, Target::Logs { .. }))
            .unwrap();
        assert_eq!(logs.disabled.as_deref(), Some(expected));
        assert_eq!(server.requests.lock().unwrap().len(), 3);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn absent_store_is_an_error_only_when_the_address_requests_a_token() {
    let server = Server::new(false, vec![]);
    let mut settings = settings(&server.address);
    assert!(within(Client::open(Some(&settings), None)).await.is_ok());
    settings.token = Some(KeysJson);
    assert!(matches!(
        within(Client::open(Some(&settings), None)).await,
        Err(Error::Unreadable(
            "the key store isn't available to this read"
        ))
    ));
    assert!(server.requests.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn capped_or_refused_log_metadata_never_claims_the_task_log_is_missing() {
    for status in [403, 404, 501] {
        let mut task = manifest("TaskRun", "v1");
        task["metadata"]["uid"] = json!("task-uid");
        let server = Server::new(
            false,
            vec![
                page(vec![record(pipeline("run-uid"))]),
                page(vec![record(task)]),
                Reply::raw(status, "invented-error"),
            ],
        );
        let client = super::client(&server).await;
        let evidence = evidence(&reader(vec![], vec![]), Ok(&client)).await;
        let change = change(&evidence);
        let row = change
            .hops
            .iter()
            .find(|row| row.name == "Run checkout-7")
            .unwrap();
        let Shows::Hop(detail) = &row.shows else {
            panic!();
        };
        let action = detail
            .actions
            .iter()
            .find(|action| matches!(action.target, Target::Logs { .. }))
            .unwrap();
        let why = action.disabled.as_deref().unwrap();
        assert!(!why.contains("kept no log"));
        if status == 403 {
            assert!(why.contains("refused"));
        } else {
            assert_eq!(why, "this Tekton Results doesn't serve logs");
        }
        assert!(
            change
                .missing
                .iter()
                .any(|line| line.text.contains("Tekton Results on cluster-b"))
        );
    }
    let server = Server::new(
        false,
        (0..5)
            .map(|n| Reply::json(json!({"records": [], "nextPageToken": format!("next-{n}")})))
            .collect(),
    );
    let client = super::client(&server).await;
    let listing = within(client.logs(RESULT, Instant::now() + Duration::from_secs(20)))
        .await
        .unwrap();
    assert_eq!(listing.cap, Some(Cap::Pages));
    assert_eq!(listing.total, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_log_metadata_and_foreign_results_are_unreadable() {
    let mut missing_uid = log("task-uid", true);
    let mut value: Value = serde_json::from_slice(
        &STANDARD
            .decode(missing_uid["data"]["value"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    value["spec"]["resource"]
        .as_object_mut()
        .unwrap()
        .remove("uid");
    missing_uid["data"]["value"] = json!(STANDARD.encode(serde_json::to_vec(&value).unwrap()));
    let mut foreign = log("task-uid", true);
    foreign["name"] = json!("acme/results/other/records/log-a");
    let server = Server::new(false, vec![page(vec![missing_uid]), page(vec![foreign])]);
    let client = super::client(&server).await;
    for _ in 0..2 {
        assert!(matches!(
            within(client.logs(RESULT, Instant::now() + Duration::from_secs(20))).await,
            Err(Error::Unreadable(_))
        ));
    }
}
