use super::*;

/// Each request's target and Authorization header.
type Heard = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// A local HTTP server for one direct-URL test: it records each request's
/// path and Authorization header and answers by path.
async fn http_server(
    route: impl Fn(&str) -> (u16, String) + Send + Sync + 'static,
) -> (String, Heard) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen: Heard = Default::default();
    let log = seen.clone();
    let route = Arc::new(route);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let log = log.clone();
            let route = route.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0; 16 * 1024];
                let mut read = 0;
                while !buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buffer[read..]).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => read += n,
                    }
                }
                let head = String::from_utf8_lossy(&buffer[..read]).into_owned();
                let target = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                let auth = head
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("authorization: ")
                            .or_else(|| line.strip_prefix("Authorization: "))
                    })
                    .map(str::to_owned);
                let (status, body) = route(&target);
                log.lock().unwrap().push((target, auth));
                let answer = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(answer.as_bytes()).await;
            });
        }
    });
    (format!("http://{address}"), seen)
}

#[tokio::test]
async fn a_url_is_confirmed_with_its_token_and_prefix() {
    let (base, seen) = http_server(|target| match target {
        "/select/0/prometheus/api/v1/query?query=1" => (200, query_one()),
        // VictoriaMetrics answers buildinfo with the version it imitates.
        "/select/0/prometheus/api/v1/status/buildinfo" => (200, build_info()),
        _ => (404, "not found".into()),
    })
    .await;
    let url = normalise_url(&format!("{base}/select/0/prometheus")).unwrap();
    let (prometheus, version) = confirm_url(url.clone(), Some(" s3cret ".into()))
        .await
        .unwrap();
    assert_eq!(prometheus.endpoint(), &Endpoint::Url(url));
    assert!(prometheus.service().is_none());
    assert_eq!(version.as_deref(), Some("2.53.0"));
    // No targets endpoint: the default interval stays.
    assert_eq!(prometheus.scrape_interval(), DEFAULT_SCRAPE_INTERVAL);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].0, "/select/0/prometheus/api/v1/query?query=1");
    assert!(
        seen.iter()
            .all(|(_, auth)| auth.as_deref() == Some("Bearer s3cret")),
        "{seen:?}"
    );
}

#[tokio::test]
async fn a_url_without_buildinfo_is_still_confirmed_and_errors_hide_the_token() {
    let (base, _) = http_server(|target| match target {
        "/api/v1/query?query=1" => (200, query_one()),
        _ => (404, "404 page not found".into()),
    })
    .await;
    let (_, version) = confirm_url(normalise_url(&base).unwrap(), None)
        .await
        .unwrap();
    assert_eq!(version, None);

    let (base, _) = http_server(|_| (401, "unauthorized".into())).await;
    let error = confirm_url(normalise_url(&base).unwrap(), Some("s3cret".into()))
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::Refused);
    assert!(!error.message.contains("s3cret"), "{}", error.message);
    assert!(!error.message.contains("127.0.0.1"), "{}", error.message);

    // Something that isn't a Prometheus API.
    let (base, _) = http_server(|_| (200, "<html></html>".into())).await;
    let error = confirm_url(normalise_url(&base).unwrap(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::BadAnswer);

    // Nothing listening.
    let error = confirm_url("http://127.0.0.1:1/".into(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    assert_eq!(error.message, "Couldn't connect to the server");
}
