use super::*;

#[tokio::test]
async fn every_request_carries_the_headers_and_only_a_token_adds_authorization() {
    let api = Server::new(ok(vec![pull(418)]));
    let anonymous = client(&api, None);
    let pulls = ask(&anonymous).await.unwrap();
    assert_eq!(pulls.len(), 1);
    let summary = pulls[0].summary.as_ref().unwrap();
    assert_eq!(summary.state, PullState::Open);
    assert_eq!(summary.author.as_ref().unwrap().login, "mira");
    let seen = api.seen();
    assert_eq!(seen.len(), 1, "one read, and nothing else: {seen:?}");
    let request = &seen[0];
    assert!(request.head().starts_with("GET /repos/acme/shop/commits/"));
    assert!(
        request.path.ends_with("/pulls?per_page=100"),
        "{}",
        request.path
    );
    assert_eq!(
        request.header("accept").as_deref(),
        Some("application/vnd.github+json")
    );
    assert_eq!(
        request.header("x-github-api-version").as_deref(),
        Some("2022-11-28")
    );
    assert_eq!(
        request.header("user-agent"),
        Some(format!("freshkube/{}", env!("CARGO_PKG_VERSION")))
    );
    assert_eq!(request.header("authorization"), None, "no token, no header");
    assert_eq!(request.header("if-none-match"), None);

    let authenticated = client(&api, Some(TOKEN));
    ask(&authenticated).await.unwrap();
    let seen = api.seen();
    assert_eq!(
        seen.len(),
        2,
        "the anonymous read and the token's: {seen:?}"
    );
    assert_eq!(
        seen[1].header("authorization").as_deref(),
        Some(&*format!("Bearer {TOKEN}"))
    );
}

#[tokio::test]
async fn the_token_reaches_the_api_host_and_no_address_in_an_answer_is_fetched() {
    let avatars = Server::new(|_| Reply::empty(200, ""));
    let elsewhere = avatars.address.clone();
    // Every address the answer carries points at the second host.
    let api = Server::new({
        let elsewhere = elsewhere.clone();
        move |_| {
            let mut pull = pull(418);
            pull["html_url"] = json!(format!("{elsewhere}/pull/418"));
            pull["user"]["avatar_url"] = json!(format!("{elsewhere}/u/7"));
            pull["statuses_url"] = json!(format!("{elsewhere}/statuses"));
            Reply::json(
                200,
                &format!(
                    "{}link: <{elsewhere}/next>; rel=\"next\"\r\n",
                    limit_headers(4000, in_an_hour())
                ),
                &json!([pull]),
            )
        }
    });
    let handle = client(&api, Some(TOKEN));
    let pulls = ask(&handle).await.unwrap();
    let summary = pulls[0].summary.as_ref().unwrap();
    assert!(summary.page.is_none(), "a page off github.com is dropped");
    assert!(summary.author.as_ref().unwrap().avatar.is_none());
    assert_eq!(api.seen().len(), 1, "the Link header was not followed");
    assert!(
        avatars.seen().is_empty(),
        "nothing in the answer was fetched, with or without the token"
    );
}

#[tokio::test]
async fn a_redirect_is_an_answer_and_is_not_followed() {
    let avatars = Server::new(|_| Reply::empty(200, ""));
    let target = avatars.address.clone();
    let api = Server::new(move |_| {
        Reply::empty(301, &format!("location: {target}/repos/acme/new/pulls\r\n"))
    });
    let handle = client(&api, Some(TOKEN));
    let failure = ask(&handle).await.unwrap_err();
    assert_eq!(failure.message, "GitHub says the repository moved (301)");
    assert_eq!(api.seen().len(), 1);
    assert!(
        avatars.seen().is_empty(),
        "the redirect's target saw nothing"
    );
    assert_eq!(
        Error::Moved.words("acme/shop"),
        "GitHub says acme/shop moved"
    );
}

#[tokio::test]
async fn a_refused_token_never_falls_back_to_an_anonymous_request() {
    let api = Server::new(|_| Reply::empty(401, ""));
    let handle = client(&api, Some(TOKEN));
    let failure = ask(&handle).await.unwrap_err();
    assert_eq!(failure.message, "GitHub refused the token (401)");
    assert_eq!(failure.kind, FailureKind::Unauthorized);
    // Asked again, and for another commit, it asks with the token again.
    let _ = handle
        .pulls_for_commit("acme/shop", &"a".repeat(40))
        .await
        .unwrap_err();
    let seen = api.seen();
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().all(|request| {
        request
            .header("authorization")
            .is_some_and(|value| value == format!("Bearer {TOKEN}"))
    }));
    assert_eq!(
        Error::Unauthorized(GitHubToken::KeysJson).words("acme/shop"),
        "GitHub refused the token: it may have expired. It is the keys.json entry GitHub https://api.github.com/"
    );
}

#[tokio::test]
async fn refusals_and_missing_repositories_are_worded_as_the_design_says() {
    for (status, token, card) in [
        (
            403,
            Some(TOKEN),
            "GitHub refused reading acme/shop (403): the token may not be granted this repository",
        ),
        (
            404,
            None,
            "acme/shop wasn't found. If it's private, add a GitHub token",
        ),
        (
            404,
            Some(TOKEN),
            "acme/shop wasn't found, or the token can't see it",
        ),
    ] {
        let api = Server::new(move |_| Reply::empty(status, ""));
        let error = client(&api, token)
            .client
            .pulls("acme/shop", SHA, false)
            .await
            .unwrap_err();
        assert_eq!(error.words("acme/shop"), card);
    }
}

#[tokio::test]
async fn a_list_of_pull_requests_is_capped_at_one_hundred() {
    let api = Server::new(ok((1..=120).map(pull).collect()));
    let pulls = ask(&client(&api, None)).await.unwrap();
    assert_eq!(pulls.len(), 100);
    assert_eq!(pulls[99].number, 100);
}

#[tokio::test]
async fn an_unreachable_host_is_worded_without_its_address() {
    let api = Server::new(ok(vec![]));
    let handle = client(&api, Some(TOKEN));
    drop(api);
    // A dead port: nothing listens.
    let failure = ask(&handle).await.unwrap_err();
    assert_eq!(failure.message, "GitHub didn't answer: could not connect");
    assert!(!failure.message.contains("127.0.0.1"));
}
