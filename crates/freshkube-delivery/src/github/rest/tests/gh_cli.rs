#[cfg(unix)]
use super::*;

#[cfg(unix)]
#[tokio::test]
async fn gh_gives_the_token_which_reaches_the_api_host_alone() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let place = tempfile::tempdir().unwrap();
    let args = place.path().join("args");
    let gh = fake_gh(&format!(
        "echo \"$@\" > {args}\necho ghp_fromGhExample0123456789",
        args = args.display()
    ));
    let dirs = [gh.path().to_owned()];
    let token = Client::token_of(GitHubToken::Gh, None, &dirs, gh::DEADLINE)
        .await
        .unwrap();
    assert_eq!(token, "ghp_fromGhExample0123456789");
    // Pinned to github.com, so the token is that host's.
    assert_eq!(
        std::fs::read_to_string(&args).unwrap().trim(),
        "auth token --hostname github.com"
    );
    // The client built from it sends it to the API host and to no other.
    let avatars = Server::new(|_| Reply::empty(200, ""));
    let elsewhere = avatars.address.clone();
    let api = Server::new(move |_| {
        let mut pull = pull(418);
        pull["user"]["avatar_url"] = json!(format!("{elsewhere}/u/7"));
        Reply::json(200, "", &json!([pull]))
    });
    let handle = Handle::trusting_as(&api.address, Some(&token), GitHubToken::Gh);
    assert_eq!(handle.identity().account.as_deref(), Some(GH_ACCOUNT));
    ask(&handle).await.unwrap();
    let seen = api.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(
        seen[0].header("authorization").as_deref(),
        Some("Bearer ghp_fromGhExample0123456789")
    );
    assert!(avatars.seen().is_empty());
    // Refused, it names gh, not keys.json.
    let refusing = Server::new(|_| Reply::empty(401, ""));
    let handle = Handle::trusting_as(&refusing.address, Some(&token), GitHubToken::Gh);
    let error = handle
        .client
        .pulls("acme/shop", SHA, false)
        .await
        .unwrap_err();
    assert_eq!(error, Error::Unauthorized(GitHubToken::Gh));
    assert!(error.words("acme/shop").contains("gh auth login"));
    assert!(!error.words("acme/shop").contains("keys.json"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_missing_gh_is_not_set_up_and_says_how_to() {
    let empty = tempfile::tempdir().unwrap();
    let opened = Client::open_with(
        GitHubToken::Gh,
        None,
        1,
        &[empty.path().to_owned()],
        gh::DEADLINE,
    )
    .await;
    let error = opened.err().unwrap();
    assert_eq!(error, Error::NotSetUp(NotSetUp::GhNotFound));
    let words = error.to_string();
    assert!(
        words.starts_with("GitHub isn't set up: gh not found"),
        "{words}"
    );
    assert!(words.contains("keys.json"), "the next step: {words}");
    // A folder with a `gh` that can't be run is not found either.
    let plain = tempfile::tempdir().unwrap();
    std::fs::write(plain.path().join("gh"), "not executable").unwrap();
    let opened = Client::open_with(
        GitHubToken::Gh,
        None,
        1,
        &[plain.path().to_owned()],
        gh::DEADLINE,
    )
    .await;
    assert_eq!(opened.err(), Some(Error::NotSetUp(NotSetUp::GhNotFound)));
}

#[cfg(unix)]
#[tokio::test]
async fn a_gh_that_fails_says_it_is_not_signed_in_and_shows_none_of_its_output() {
    let gh = fake_gh("echo 'secret-stderr-text ghp_leaked0123456789' >&2\nexit 1");
    let dirs = [gh.path().to_owned()];
    let opened = Client::open_with(GitHubToken::Gh, None, 1, &dirs, gh::DEADLINE).await;
    let error = opened.err().unwrap();
    assert_eq!(error, Error::NotSetUp(NotSetUp::GhNotSignedIn));
    for shown in [
        error.to_string(),
        format!("{error:?}"),
        error.words("acme/shop"),
    ] {
        assert!(
            !shown.contains("secret-stderr-text") && !shown.contains("ghp_leaked"),
            "{shown}"
        );
    }
    assert!(error.to_string().contains("gh auth login"));
    // Even one that succeeds and writes to standard error is not read.
    let noisy = fake_gh("echo 'warning: something' >&2\necho ghp_fine0123456789");
    let token = Client::token_of(
        GitHubToken::Gh,
        None,
        &[noisy.path().to_owned()],
        gh::DEADLINE,
    )
    .await
    .unwrap();
    assert_eq!(token, "ghp_fine0123456789");
    // What gh prints that is not a token is not used.
    let odd = fake_gh("echo 'not a token'");
    let opened = Client::open_with(
        GitHubToken::Gh,
        None,
        1,
        &[odd.path().to_owned()],
        gh::DEADLINE,
    )
    .await;
    assert_eq!(
        opened.err(),
        Some(Error::NotSetUp(NotSetUp::TokenUnreadable))
    );
    // An empty answer is not signed in.
    let empty = fake_gh("true");
    let opened = Client::open_with(
        GitHubToken::Gh,
        None,
        1,
        &[empty.path().to_owned()],
        gh::DEADLINE,
    )
    .await;
    assert_eq!(opened.err(), Some(Error::NotSetUp(NotSetUp::GhNotSignedIn)));
}

#[cfg(unix)]
#[tokio::test]
async fn a_gh_that_takes_too_long_is_stopped() {
    let gh = fake_gh("sleep 30");
    let started = std::time::Instant::now();
    let token = super::gh::token(&[gh.path().to_owned()], Duration::from_millis(200)).await;
    assert_eq!(token, Err(NotSetUp::GhSlow));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[cfg(unix)]
#[tokio::test]
async fn a_gh_that_hangs_is_worded_as_gh_being_slow_not_as_the_key_store() {
    let gh = fake_gh("sleep 30");
    let dirs = [gh.path().to_owned()];
    // Through the same door the app uses, with gh's deadline cut short.
    let opened =
        Client::open_with(GitHubToken::Gh, None, 1, &dirs, Duration::from_millis(200)).await;
    let error = opened.err().unwrap();
    assert_eq!(error, Error::NotSetUp(NotSetUp::GhSlow));
    assert_eq!(
        error.to_string(),
        "GitHub isn't set up: gh didn't answer in time"
    );
    assert!(!error.to_string().contains("key store"));
    // The caller's own deadline is longer than gh's, so gh's fires first;
    // and what a caller says when it does give up is chosen by the source.
    assert!(OPEN_DEADLINE > gh::DEADLINE);
    assert_eq!(
        Error::not_had(GitHubToken::Gh),
        Error::NotSetUp(NotSetUp::GhSlow)
    );
    assert_eq!(
        Error::not_had(GitHubToken::KeysJson),
        Error::NotSetUp(NotSetUp::KeysUnavailable)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn gh_runs_only_when_the_setup_names_it() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let place = tempfile::tempdir().unwrap();
    let marker = place.path().join("ran");
    let gh = fake_gh(&format!("touch {}\necho ghp_x0123456789", marker.display()));
    let dirs = [gh.path().to_owned()];
    // keys.json named, in every state: gh is not asked.
    let (_, secrets) = keys(Some(TOKEN));
    Client::open_with(GitHubToken::KeysJson, Some(secrets), 1, &dirs, gh::DEADLINE)
        .await
        .unwrap();
    let missing = Client::open_with(
        GitHubToken::KeysJson,
        Some(keys(None).1),
        1,
        &dirs,
        gh::DEADLINE,
    )
    .await;
    assert!(missing.is_err(), "no saved token is not a reason to run gh");
    // Neither named: an anonymous client exists at once and gh is not asked.
    let anonymous = Client::anonymous(1).unwrap();
    assert!(!anonymous.authenticated());
    assert_eq!(anonymous.identity().account, None);
    assert!(!marker.exists(), "gh was not run");
    // Named, it is.
    Client::open_with(GitHubToken::Gh, None, 1, &dirs, gh::DEADLINE)
        .await
        .unwrap();
    assert!(marker.exists());
}

#[test]
fn gh_is_looked_for_in_the_usual_places_before_path() {
    let dirs = super::gh::default_dirs();
    assert_eq!(dirs[0], std::path::Path::new("/opt/homebrew/bin"));
    assert_eq!(dirs[1], std::path::Path::new("/usr/local/bin"));
    if let Some(path) = std::env::var_os("PATH") {
        let rest: Vec<_> = std::env::split_paths(&path).collect();
        assert_eq!(dirs[2..], rest[..]);
    }
}
