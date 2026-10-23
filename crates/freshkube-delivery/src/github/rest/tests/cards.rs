use super::*;

#[tokio::test]
async fn the_card_reads_four_things_and_nothing_else() {
    let api = card(|_| None);
    let detail = client(&api, Some(TOKEN))
        .detail("acme/shop", 418)
        .await
        .unwrap();
    let paths: Vec<String> = api
        .seen()
        .iter()
        .map(|request| request.path.clone())
        .collect();
    assert_eq!(
        paths,
        [
            "/repos/acme/shop/pulls/418".to_owned(),
            "/repos/acme/shop/pulls/418/reviews?per_page=100".to_owned(),
            format!("/repos/acme/shop/commits/{HEAD}/check-runs?per_page=100"),
            format!("/repos/acme/shop/commits/{HEAD}/status?per_page=100"),
        ]
    );
    assert!(api.seen().iter().all(|request| {
        request
            .header("authorization")
            .is_some_and(|value| value == format!("Bearer {TOKEN}"))
    }));
    assert_eq!(detail.summary.state, PullState::Merged);
    assert_eq!(detail.summary.merged_by.as_ref().unwrap().login, "ken");
    assert_eq!(detail.summary.head.as_deref(), Some(HEAD));
    assert_eq!(detail.stale, None);
    // What `pulls/{n}` also says of it is kept for the full card.
    assert_eq!(
        detail.changed,
        Some(Changed {
            additions: 84,
            deletions: 12,
            files: 5
        })
    );
    let labels: Vec<(&str, Option<&str>)> = detail
        .labels
        .iter()
        .map(|label| (label.name.as_str(), label.color.as_deref()))
        .collect();
    assert_eq!(
        labels,
        [("reliability", Some("0e8a16")), ("odd", None)],
        "a colour that isn't six hex digits is none"
    );
    assert_eq!(detail.merge_commit, Some("e".repeat(40)));
    // The address a status names is for the browser, kept as it is; a check
    // run's must be on github.com.
    assert_eq!(
        detail.checks.statuses[0].page.as_ref().unwrap().host(),
        "preview.example.test"
    );
    assert!(detail.checks.runs[0].page.is_some());
    assert!(detail.checks.runs[2].page.is_none());
}

#[tokio::test]
async fn the_latest_decisive_review_of_each_person_stands() {
    let api = card(|_| None);
    let detail = client(&api, None).detail("acme/shop", 418).await.unwrap();
    let states: Vec<(&str, ReviewState)> = detail
        .reviews
        .iter()
        .map(|review| (review.who.login.as_str(), review.state))
        .collect();
    // ken: a later comment never replaces a decision, a later decision
    // replaces an earlier one; zoe only commented (her pending review is
    // her own); ana's approval was dismissed.
    assert_eq!(
        states,
        [
            ("ken", ReviewState::Approved),
            ("zoe", ReviewState::Commented),
            ("ana", ReviewState::Dismissed),
        ]
    );
    assert!(!detail.reviews_capped);
}

#[tokio::test]
async fn more_than_a_hundred_reviews_say_the_first_hundred() {
    let reviews: Vec<Value> = (0..120)
        .map(|n| review(&format!("person{n}"), "APPROVED", "2026-09-30T09:00:00Z"))
        .collect();
    let api = card(move |request| {
        request
            .path
            .contains("/reviews")
            .then(|| Reply::json(200, "", &Value::Array(reviews.clone())))
    });
    let detail = client(&api, None).detail("acme/shop", 418).await.unwrap();
    assert_eq!(detail.reviews.len(), 100);
    assert!(detail.reviews_capped);
}

#[tokio::test]
async fn a_merged_pull_requests_reviews_and_checks_stay_ten_minutes() {
    let api = card(|_| None);
    let handle = client(&api, Some(TOKEN));
    handle.detail("acme/shop", 418).await.unwrap();
    assert_eq!(api.seen().len(), 4);
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(9 * 60));
    handle.detail("acme/shop", 418).await.unwrap();
    // The pull request itself is asked again after a minute; what it
    // labelled with 304 or 200 is not what the other three wait on.
    assert_eq!(api.seen().len(), 5, "only the pull request");
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(2 * 60));
    handle.detail("acme/shop", 418).await.unwrap();
    assert_eq!(api.seen().len(), 9, "all four after ten minutes");
}

#[tokio::test]
async fn a_card_read_the_limit_stops_halfway_stands_as_what_was_kept() {
    let blocked = Arc::new(AtomicUsize::new(0));
    let api = card({
        let blocked = blocked.clone();
        move |request| {
            (blocked.load(Ordering::SeqCst) == 1 && request.path.contains("/reviews"))
                .then(|| Reply::empty(429, "retry-after: 300\r\n"))
        }
    });
    let handle = client(&api, Some(TOKEN));
    let first = handle.detail("acme/shop", 418).await.unwrap();
    assert!(first.stale.is_none());
    handle
        .client
        .state
        .lock()
        .unwrap()
        .memo
        .age(Duration::from_secs(11 * 60));
    blocked.store(1, Ordering::SeqCst);
    let second = handle.detail("acme/shop", 418).await.unwrap();
    let stale = second.stale.as_ref().expect("marked stale");
    assert!(
        stale
            .why
            .starts_with("GitHub's secondary rate limit is reached until ")
    );
    assert_eq!(second.reviews, first.reviews, "what was kept stands");
    assert_eq!(second.summary.stale, second.stale);
    // Never read before, there is nothing to stand in: the words are an error.
    let fresh = client(&api, Some(TOKEN)).detail("acme/shop", 418).await;
    assert!(matches!(fresh, Err(Error::Limited { .. })));
}

#[tokio::test]
async fn the_card_names_its_failures_in_the_cards_words() {
    let api = Server::new(|_| Reply::empty(404, ""));
    let error = client(&api, None).detail("acme/shop", 1).await.unwrap_err();
    assert_eq!(
        error.words("acme/shop"),
        "acme/shop wasn't found. If it's private, add a GitHub token"
    );
    let paths: Vec<String> = api
        .seen()
        .iter()
        .map(|request| request.path.clone())
        .collect();
    assert_eq!(paths, ["/repos/acme/shop/pulls/1"], "it stops at the first");
    // A pull request without a usable head commit can't be asked about.
    let api = Server::new(|_| {
        let mut pull = pull(1);
        pull["head"]["sha"] = json!("../../etc");
        Reply::json(200, "", &pull)
    });
    let error = client(&api, None).detail("acme/shop", 1).await.unwrap_err();
    assert!(matches!(error, Error::Unreadable(_)));
    assert_eq!(api.seen().len(), 1, "no path is built from it");
    assert!(client(&api, None).detail("../x", 1).await.is_err());
}

#[test]
fn a_detail_key_is_the_client_the_pull_request_and_its_head() {
    let identity = client_identity();
    let mut summary = super::types::summary("acme/shop", &{
        let mut value = pull(418);
        value["head"]["sha"] = json!(HEAD);
        value
    })
    .unwrap();
    let key = DetailKey::of(&identity, &summary).unwrap();
    assert_eq!(
        (key.repo.as_str(), key.number, key.head.as_str()),
        ("acme/shop", 418, HEAD)
    );
    let mut other = identity.clone();
    other.setup_revision += 1;
    assert_ne!(DetailKey::of(&other, &summary), Some(key.clone()));
    summary.head = Some("not a sha".into());
    assert_eq!(DetailKey::of(&identity, &summary), None);
    summary.head = None;
    assert_eq!(DetailKey::of(&identity, &summary), None);
}
