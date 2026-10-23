use super::*;

#[tokio::test]
async fn a_304_reuses_the_kept_answer_and_sends_if_none_match_with_a_token() {
    let calls = Arc::new(AtomicUsize::new(0));
    let api = Server::new({
        let calls = calls.clone();
        move |request| {
            calls.fetch_add(1, Ordering::SeqCst);
            match request.header("if-none-match") {
                Some(etag) if etag == "\"v1\"" => {
                    Reply::empty(304, &limit_headers(4999, in_an_hour()))
                }
                _ => Reply::json(
                    200,
                    &format!("etag: \"v1\"\r\n{}", limit_headers(4000, in_an_hour())),
                    &json!([pull(418)]),
                ),
            }
        }
    });
    let handle = client(&api, Some(TOKEN));
    let first = ask(&handle).await.unwrap();
    // Fresh: no request at all.
    ask(&handle).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // Past its minute, it asks again and the 304 hands back the same answer.
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(61));
    let again = ask(&handle).await.unwrap();
    assert_eq!(first, again);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        api.seen()[1].header("if-none-match").as_deref(),
        Some("\"v1\"")
    );
    // The 304 started the expiry again.
    ask(&handle).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn without_a_token_nothing_is_revalidated_and_the_answer_lives_fifteen_minutes() {
    let api = Server::new(|request| {
        assert_eq!(request.header("if-none-match"), None);
        Reply::json(200, "etag: \"v1\"\r\n", &json!([pull(418)]))
    });
    let handle = client(&api, None);
    ask(&handle).await.unwrap();
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(14 * 60));
    ask(&handle).await.unwrap();
    assert_eq!(api.seen().len(), 1, "still fresh at 14 minutes");
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(2 * 60));
    ask(&handle).await.unwrap();
    assert_eq!(api.seen().len(), 2);
}

#[tokio::test]
async fn a_commit_in_a_merged_pull_request_is_kept_for_the_clients_life() {
    let merged = json!({
        "number": 7, "state": "closed", "merged_at": "2026-10-01T10:00:00Z",
        "merge_commit_sha": SHA, "head": {"sha": "b".repeat(40)},
    });
    let api = Server::new(ok(vec![merged]));
    let handle = client(&api, None);
    ask(&handle).await.unwrap();
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(30 * 24 * 3600));
    let pulls = ask(&handle).await.unwrap();
    assert_eq!(pulls[0].summary.as_ref().unwrap().state, PullState::Merged);
    assert_eq!(api.seen().len(), 1);
}

#[tokio::test]
async fn a_304_for_an_answer_the_memo_dropped_reads_it_again() {
    // The server forgets nothing, but the client's memo loses the entry
    // between asking with its validator and the 304 coming back, as the cap
    // can make it.
    let held: Arc<Mutex<Option<Handle>>> = Arc::default();
    let api = Server::new({
        let held = held.clone();
        move |request| match request.header("if-none-match") {
            Some(_) => {
                let handle = held.lock().unwrap().clone().unwrap();
                handle.client.state.lock().unwrap().memo = memo::Memo::default();
                Reply::empty(304, &limit_headers(4999, in_an_hour()))
            }
            None => Reply::json(
                200,
                &format!("etag: \"v1\"\r\n{}", limit_headers(4000, in_an_hour())),
                &json!([pull(418)]),
            ),
        }
    });
    let handle = client(&api, Some(TOKEN));
    *held.lock().unwrap() = Some(handle.clone());
    ask(&handle).await.unwrap();
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(120));
    let again = ask(&handle).await.unwrap();
    assert_eq!(again.len(), 1);
    let seen = api.seen();
    assert_eq!(seen.len(), 3, "the read, the validation, the read again");
    assert!(seen[1].header("if-none-match").is_some());
    assert_eq!(seen[2].header("if-none-match"), None);
}
