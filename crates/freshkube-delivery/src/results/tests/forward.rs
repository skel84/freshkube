use super::server::SERVICE_CERT;
use super::*;

fn server_port(server: &Server) -> NonZeroU16 {
    url::Url::parse(&server.address)
        .unwrap()
        .port()
        .unwrap()
        .try_into()
        .unwrap()
}

fn forward_settings(port: u16, ca_file: &std::path::Path) -> Results {
    let mut settings = Results::default();
    settings.forward = Some(Forward::new("acme", "results", port));
    settings.ca_file = Some(ca_file.into());
    settings
}

async fn routed(settings: &Results, server: &Server, store: Arc<MemoryStore>) -> Client {
    within(Client::open_routed(
        Some(settings),
        Some(store),
        Route::Forward {
            local_port: server_port(server),
        },
    ))
    .await
    .unwrap_or_else(|error| panic!("{error}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn forward_pins_the_local_port_and_verifies_the_service_name() {
    let configured = Server::with_certificate(Some(SERVICE_CERT), vec![]);
    let local = Server::with_certificate(Some(SERVICE_CERT), vec![page(vec![])]);
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("service-ca.pem");
    std::fs::write(&ca, SERVICE_CERT).unwrap();
    let settings = forward_settings(server_port(&configured).get(), &ca);
    let client = routed(&settings, &local, Arc::default()).await;
    assert_eq!(client.base.as_str(), "https://results.acme.svc/");
    assert_eq!(client.base.port(), None);
    assert!(runs(&client).await.is_ok());
    let requests = local.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("host: results.acme.svc\r\n"));
    assert!(!requests[0].contains("authorization:"));
    assert_eq!(*local.server_names.lock().unwrap(), ["results.acme.svc"]);
    assert!(configured.requests.lock().unwrap().is_empty());
    assert!(configured.server_names.lock().unwrap().is_empty());
}

#[test]
fn forward_ignores_environment_proxies() {
    // A separate process avoids changing proxy variables under concurrent tests.
    let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let address = format!("http://{}", proxy.local_addr().unwrap());
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "results::tests::forward::forward_pins_the_local_port_and_verifies_the_service_name",
    ]);
    for name in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        command.env(name, &address);
    }
    command.env("NO_PROXY", "").env("no_proxy", "");
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        proxy.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn forward_rejects_untrusted_and_wrong_name_certificates() {
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("service-ca.pem");
    for (certificate, service, trusted) in [
        (SERVICE_CERT, "results", false),
        (SERVICE_CERT, "other-results", true),
        (CERT, "results", true),
    ] {
        let server = Server::with_certificate(Some(certificate), vec![]);
        std::fs::write(&ca, certificate).unwrap();
        let mut settings = forward_settings(8080, &ca);
        settings.forward.as_mut().unwrap().service = service.into();
        if !trusted {
            settings.ca_file = None;
        }
        let client = routed(&settings, &server, Arc::default()).await;
        assert_eq!(
            runs(&client).await,
            Err(Error::Unreachable(Cause::Certificate))
        );
        assert!(server.requests.lock().unwrap().is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn forward_token_uses_only_the_configured_triple_across_local_ports() {
    let first = Server::with_certificate(Some(SERVICE_CERT), vec![page(vec![])]);
    let second = Server::with_certificate(Some(SERVICE_CERT), vec![page(vec![])]);
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("service-ca.pem");
    std::fs::write(&ca, SERVICE_CERT).unwrap();
    let mut settings = forward_settings(8080, &ca);
    settings.token = Some(KeysJson);
    let account = forward_token_account(settings.forward.as_ref().unwrap());
    assert_eq!(account, "Tekton Results forward://acme/results:8080");
    let store = Arc::new(MemoryStore::default());
    store.write(&account, "invented-forward-token").unwrap();
    for account in [
        token_account("https://results.acme.svc/").unwrap(),
        token_account(&first.address).unwrap(),
        forward_token_account(&Forward::new("other", "results", 8080)),
        forward_token_account(&Forward::new("acme", "other", 8080)),
        forward_token_account(&Forward::new("acme", "results", 8081)),
        forward_token_account(&Forward::new("acme", "results", server_port(&first).get())),
        "Prometheus https://results.acme.svc/".into(),
        "Coroot https://results.acme.svc/".into(),
    ] {
        assert_ne!(account, "Tekton Results forward://acme/results:8080");
        store.write(&account, "invented-other-token").unwrap();
    }
    for server in [&first, &second] {
        let client = routed(&settings, server, store.clone()).await;
        assert!(runs(&client).await.is_ok());
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].contains("authorization: Bearer invented-forward-token\r\n"));
    }
    // Changing any part of the triple cannot pick up the original account.
    let other = Server::with_certificate(Some(SERVICE_CERT), vec![page(vec![])]);
    settings.forward.as_mut().unwrap().port = 9090;
    let client = routed(&settings, &other, store).await;
    assert!(runs(&client).await.is_ok());
    assert!(!other.requests.lock().unwrap()[0].contains("authorization:"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ended_route_does_no_io_and_never_falls_back() {
    let server = Server::with_certificate(Some(SERVICE_CERT), vec![]);
    let store = Arc::new(MemoryStore::default());
    store.unavailable.store(true, Ordering::SeqCst);
    let dir = tempfile::tempdir().unwrap();
    let mut settings =
        forward_settings(server_port(&server).get(), &dir.path().join("missing.pem"));
    settings.token = Some(KeysJson);
    let error = within(Client::open_routed(
        Some(&settings),
        Some(store),
        Route::Ended("stopped"),
    ))
    .await
    .err()
    .unwrap();
    assert_eq!(error, Error::ForwardEnded("stopped"));
    assert_eq!(error.state(), State::ForwardEnded);
    assert_eq!(
        error.to_string(),
        "the Tekton Results forward ended: stopped"
    );
    assert!(server.requests.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_local_route_reports_forward_ended_without_fallback() {
    let configured = Server::with_certificate(Some(SERVICE_CERT), vec![]);
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let local_port = closed.local_addr().unwrap().port().try_into().unwrap();
    let mut settings = Results::default();
    settings.forward = Some(Forward::new(
        "acme",
        "results",
        server_port(&configured).get(),
    ));
    let client = within(Client::open_routed(
        Some(&settings),
        None,
        Route::Forward { local_port },
    ))
    .await
    .unwrap_or_else(|error| panic!("{error}"));
    drop(closed);
    assert_eq!(
        runs(&client).await,
        Err(Error::ForwardEnded("the connection failed"))
    );
    assert!(configured.requests.lock().unwrap().is_empty());
}
