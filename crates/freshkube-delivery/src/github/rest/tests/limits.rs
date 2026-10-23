use super::*;

#[tokio::test]
async fn no_requests_are_left_until_the_reset_when_none_remain() {
    let api =
        Server::new(|_| Reply::json(200, &limit_headers(0, in_an_hour()), &json!([pull(418)])));
    let handle = client(&api, Some(TOKEN));
    ask(&handle).await.unwrap();
    let other = "b".repeat(40);
    let failure = handle
        .pulls_for_commit("acme/shop", &other)
        .await
        .unwrap_err();
    assert!(
        failure
            .message
            .starts_with("GitHub's rate limit is reached until "),
        "{}",
        failure.message
    );
    assert_eq!(api.seen().len(), 1, "nothing was sent while blocked");
    // Once the reset has passed, it asks again.
    {
        let mut state = handle.client.state.lock().unwrap();
        state.budget = Budget::default();
    }
    handle.pulls_for_commit("acme/shop", &other).await.unwrap();
    assert_eq!(api.seen().len(), 2);
}

#[tokio::test]
async fn a_403_with_nothing_left_blocks_until_the_reset() {
    let reset = in_an_hour();
    let api = Server::new(move |_| Reply::empty(403, &limit_headers(0, reset)));
    let handle = client(&api, None);
    let error = handle
        .client
        .pulls("acme/shop", SHA, false)
        .await
        .unwrap_err();
    let Error::Limited { until, secondary } = error else {
        panic!("{error:?}")
    };
    assert_eq!(until.timestamp(), reset);
    assert!(!secondary);
    handle
        .client
        .pulls("acme/shop", SHA, false)
        .await
        .unwrap_err();
    assert_eq!(api.seen().len(), 1);
}

#[tokio::test]
async fn retry_after_blocks_for_that_long_and_a_429_without_it_for_a_minute() {
    for (headers, status, seconds) in [
        ("retry-after: 120\r\n", 403, 120),
        ("retry-after: 30\r\n", 429, 30),
        ("", 429, 60),
    ] {
        let api = Server::new(move |_| Reply::empty(status, headers));
        let handle = client(&api, Some(TOKEN));
        let before = Utc::now();
        let error = handle
            .client
            .pulls("acme/shop", SHA, false)
            .await
            .unwrap_err();
        let Error::Limited { until, secondary } = error else {
            panic!("{error:?}")
        };
        assert!(secondary);
        let wait = (until - before).num_seconds();
        assert!(
            (seconds - 1..=seconds + 2).contains(&wait),
            "{wait} for {seconds}"
        );
        handle
            .client
            .pulls("acme/shop", SHA, false)
            .await
            .unwrap_err();
        assert_eq!(api.seen().len(), 1, "nothing was sent while blocked");
    }
    // A 403 with neither header is a refusal, not a limit.
    let api = Server::new(|_| Reply::empty(403, ""));
    let error = client(&api, None)
        .client
        .pulls("acme/shop", SHA, false)
        .await;
    assert_eq!(error.unwrap_err(), Error::Forbidden);
}

#[tokio::test]
async fn the_list_stops_looking_when_the_limit_is_low_and_the_page_still_asks() {
    let api =
        Server::new(|_| Reply::json(200, &limit_headers(8, in_an_hour()), &json!([pull(418)])));
    let page = client(&api, None);
    ask(&page).await.unwrap();
    let list = page.sparing();
    let other = "c".repeat(40);
    let failure = list
        .pulls_for_commit("acme/shop", &other)
        .await
        .unwrap_err();
    assert!(
        failure
            .message
            .starts_with("GitHub's limit is low: 8 left until "),
        "{}",
        failure.message
    );
    assert_eq!(api.seen().len(), 1);
    // The page asked for this change; its read goes out.
    page.pulls_for_commit("acme/shop", &other).await.unwrap();
    assert_eq!(api.seen().len(), 2);
    // What was already read is still free to the list.
    ask(&list).await.unwrap();
    assert_eq!(api.seen().len(), 2);
    assert_eq!(page.rate_limit().unwrap().remaining, 8);
}

#[tokio::test]
async fn a_limit_or_a_silent_host_leaves_the_last_answer_marked_stale() {
    // The first read answers, then every read is limited.
    let calls = Arc::new(AtomicUsize::new(0));
    let api = Server::new({
        let calls = calls.clone();
        move |_| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Reply::json(200, &limit_headers(4000, in_an_hour()), &json!([pull(418)]))
            } else {
                Reply::empty(429, "retry-after: 300\r\n")
            }
        }
    });
    let handle = client(&api, Some(TOKEN));
    assert!(
        ask(&handle).await.unwrap()[0]
            .summary
            .as_ref()
            .unwrap()
            .stale
            .is_none()
    );
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(120));
    let pulls = ask(&handle).await.unwrap();
    let stale = pulls[0].summary.as_ref().unwrap().stale.as_ref().unwrap();
    assert!(
        stale
            .why
            .starts_with("GitHub's secondary rate limit is reached until ")
    );
    // What was never read is an error, not a guess.
    let other = "d".repeat(40);
    assert!(handle.pulls_for_commit("acme/shop", &other).await.is_err());
    // A host that no longer answers leaves it too.
    let dead = Server::new(ok(vec![pull(418)]));
    let handle = client(&dead, Some(TOKEN));
    ask(&handle).await.unwrap();
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(120));
    drop(dead);
    let pulls = ask(&handle).await.unwrap();
    assert_eq!(
        pulls[0]
            .summary
            .as_ref()
            .unwrap()
            .stale
            .as_ref()
            .unwrap()
            .why,
        "GitHub didn't answer: could not connect"
    );
}

#[tokio::test]
async fn a_limit_reached_or_nearly_spent_is_not_a_refusal() {
    use freshkube_core::resources::FailureKind;
    let until = Utc::now() + Span::hours(1);
    for error in [
        Error::Limited {
            until,
            secondary: false,
        },
        Error::Limited {
            until,
            secondary: true,
        },
        Error::Low { left: 8, until },
    ] {
        let failure: Failure = error.clone().into();
        assert_eq!(failure.kind, FailureKind::Other, "{error}");
        let source = crate::source::Source::<()>::from_failure(failure);
        assert!(
            matches!(source, crate::source::Source::Unreadable(_)),
            "{error}"
        );
    }
    let refused: Failure = Error::Forbidden.into();
    assert_eq!(refused.kind, FailureKind::Forbidden);
}

#[test]
fn a_limit_whose_reset_has_passed_is_not_said_to_be_still_waited_on() {
    let until = chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap();
    let limited = Error::Limited {
        until,
        secondary: false,
    };
    let before = until - chrono::Duration::minutes(5);
    let after = until + chrono::Duration::minutes(5);
    assert_eq!(
        limited.words_at(before),
        format!(
            "GitHub's rate limit is reached until {}",
            until.format("%H:%M UTC")
        )
    );
    let words = limited.words_at(after);
    assert!(
        words.starts_with("GitHub's rate limit was reached until ")
            && words.ends_with("; asking again"),
        "{words}"
    );
    assert!(!words.contains(" is reached"), "{words}");
}
