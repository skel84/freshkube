use super::*;

#[test]
fn a_stored_token_must_be_a_bearer_token() {
    assert!(token_header(TOKEN).is_ok());
    assert!(token_header("github_pat_11AAAA_bbbb").is_ok());
    for bad in [
        "",
        "  ",
        "has space",
        "new\nline",
        "a\"b",
        &"a".repeat(8193),
    ] {
        assert!(token_header(bad).is_err(), "{bad:?}");
    }
    assert!(token_header(TOKEN).unwrap().is_sensitive());
}

#[test]
fn a_handle_prints_whose_it_is_and_never_the_token() {
    let root = reqwest::Certificate::from_pem(CERT).unwrap();
    let client = Client::build(
        reqwest::Client::builder().add_root_certificate(root),
        Url::parse("https://127.0.0.1:1/").unwrap(),
        Some((TOKEN.into(), GitHubToken::KeysJson)),
        3,
    )
    .unwrap();
    assert_eq!(client.identity().account.as_deref(), Some(ACCOUNT));
    assert_eq!(client.identity().setup_revision, 3);
    let printed = format!(
        "{:?} {:?}",
        Handle::new(client).sparing(),
        client_identity()
    );
    assert!(!printed.contains(TOKEN), "{printed}");
}

#[tokio::test]
async fn keys_json_is_read_by_its_account_when_it_is_named() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (store, secrets) = keys(Some(TOKEN));
    let client = Client::open(GitHubToken::KeysJson, Some(secrets), 5)
        .await
        .unwrap();
    assert!(client.authenticated());
    assert_eq!(*store.0.lock().unwrap(), [ACCOUNT]);
    assert_eq!(client.identity().api.as_str(), API);
    assert_eq!(client.identity().account.as_deref(), Some(ACCOUNT));
    // Named, and the store is not there, or holds nothing, or holds what is
    // not a token: nothing is read, and the cause says which.
    for (secrets, cause) in [
        (None, NotSetUp::KeysUnavailable),
        (Some(keys(None).1), NotSetUp::NoSavedToken),
        (Some(keys(Some("bad token")).1), NotSetUp::TokenUnreadable),
        (Some(keys(Some("")).1), NotSetUp::TokenUnreadable),
    ] {
        let opened = Client::open(GitHubToken::KeysJson, secrets, 5).await;
        assert_eq!(opened.err(), Some(Error::NotSetUp(cause)), "{cause}");
    }
}
