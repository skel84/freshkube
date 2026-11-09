//! A token that expires, and an exec plugin that refreshes it, in the middle
//! of a session (#968). kube asks the plugin again when its token is within
//! sixty seconds of expiring (and does nothing on a 401), so a plugin that
//! issues tokens good for five seconds is asked at every request: no test
//! waits for a clock.
//!
//! The server is a real loopback socket (`Client::new` over a service skips
//! the auth layer), and it records the `Authorization` each request carried.
//! A failed refresh must read as a connection state, `Unauthorized`, never a
//! crash, and must not carry what the plugin wrote to its stderr.
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{Arc, Mutex},
};

use kube::config::{KubeConfigOptions, Kubeconfig};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use crate::{
    exec_plugin::config_from_kubeconfig,
    failure::{Failure, FailureKind},
    kube_client,
};

const VERSION: &str = r#"{"major":"1","minor":"30","gitVersion":"v1.30.0","gitCommit":"","gitTreeState":"","buildDate":"","goVersion":"","compiler":"","platform":""}"#;
const UNAUTHORIZED: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"Unauthorized","reason":"Unauthorized","code":401}"#;

/// A loopback API server: `judge` sees each request's `Authorization` and
/// answers 200 or 401. Every one is recorded.
struct Server {
    url: String,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Server {
    async fn start(judge: impl Fn(&str) -> bool + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let judge = Arc::new(judge);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let (log, judge) = (log.clone(), judge.clone());
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut chunk = [0; 1024];
                    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => head.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let head = String::from_utf8_lossy(&head).into_owned();
                    let auth = head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("authorization")
                                .then(|| value.trim().to_owned())
                        })
                        .unwrap_or_default();
                    log.lock().unwrap().push(auth.clone());
                    let (status, body) = if judge(&auth) {
                        ("200 OK", VERSION)
                    } else {
                        ("401 Unauthorized", UNAUTHORIZED)
                    };
                    let reply = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(reply.as_bytes()).await;
                });
            }
        });
        Self { url, seen }
    }

    fn tokens(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

/// An exec plugin in `dir`: it counts its runs in `dir/count`, issues
/// `tok-<run>` good for five seconds, and, once `dir/fail-from` names a run
/// number, fails from that run on with a line on stderr.
fn plugin(dir: &Path) -> String {
    let path = dir.join("login");
    let dir = dir.display();
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\n\
             n=$(cat '{dir}/count' 2>/dev/null || echo 0); n=$((n+1)); echo $n > '{dir}/count'\n\
             if [ -e '{dir}/fail-from' ] && [ \"$n\" -ge \"$(cat '{dir}/fail-from')\" ]; then\n\
               echo 'login failed: the-secret-detail' >&2; exit 1; fi\n\
             t=$(( $(date -u +%s) + 5 ))\n\
             exp=$(date -u -d \"@$t\" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -r \"$t\" +%Y-%m-%dT%H:%M:%SZ)\n\
             printf '{{\"apiVersion\":\"client.authentication.k8s.io/v1beta1\",\"kind\":\"ExecCredential\",\"status\":{{\"token\":\"tok-%s\",\"expirationTimestamp\":\"%s\"}}}}' \"$n\" \"$exp\"\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.display().to_string()
}

async fn client(server: &Server, plugin: &str) -> kube::Client {
    let kubeconfig: Kubeconfig = serde_yaml::from_str(&format!(
        "apiVersion: v1\nkind: Config\ncurrent-context: dev\n\
         clusters:\n- name: c\n  cluster:\n    server: {}\n\
         contexts:\n- name: dev\n  context:\n    cluster: c\n    user: u\n\
         users:\n- name: u\n  user:\n    exec:\n      \
         apiVersion: client.authentication.k8s.io/v1beta1\n      command: {plugin}\n",
        server.url
    ))
    .unwrap();
    let options = KubeConfigOptions {
        context: Some("dev".into()),
        ..KubeConfigOptions::default()
    };
    let mut config = config_from_kubeconfig(kubeconfig, &options)
        .await
        .expect("the plugin ran and its credential loaded");
    // The app drops the environment's proxy for a loopback server
    // (`kube_client::client`, from the process's own ambient values); a test
    // may run behind a proxy that kube reads from the real environment.
    config.proxy_url = None;
    kube_client::client(config).expect("a client")
}

async fn read(client: &kube::Client) -> Result<(), Failure> {
    client
        .apiserver_version()
        .await
        .map(|_| ())
        .map_err(Failure::from_kube)
}

/// The run number of a `Bearer tok-<n>`.
fn run(auth: &str) -> u32 {
    auth.strip_prefix("Bearer tok-")
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("not a plugin token: {auth:?}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_near_expiry_is_asked_for_again_at_each_request() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(|_| true).await;
    let client = client(&server, &plugin(dir.path())).await;
    for _ in 0..3 {
        read(&client).await.expect("served");
    }
    let runs: Vec<u32> = server.tokens().iter().map(|auth| run(auth)).collect();
    assert_eq!(runs.len(), 3);
    assert!(
        runs.windows(2).all(|pair| pair[0] < pair[1]),
        "each request carried a newer token than the one before: {runs:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_token_reads_as_unauthorized_and_recovers_once_it_is_due_for_refresh() {
    let dir = tempfile::tempdir().unwrap();
    // The server refuses the first token it is shown (revoked, or expired
    // early) and honours every later one.
    let asked = std::sync::atomic::AtomicUsize::new(0);
    let server =
        Server::start(move |_| asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0).await;
    let client = client(&server, &plugin(dir.path())).await;
    let first = read(&client).await.expect_err("the first token is refused");
    assert_eq!(first.kind, FailureKind::Unauthorized, "{first:?}");
    // The session is still there. kube does nothing on a 401: the next
    // request carries a new token only because these tokens are always due
    // for refresh (a one-hour token the server revoked would be sent again
    // until it neared expiry).
    read(&client).await.expect("recovered with a new token");
    let runs: Vec<u32> = server.tokens().iter().map(|auth| run(auth)).collect();
    assert_eq!(runs.len(), 2);
    assert!(runs[0] < runs[1], "{runs:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_refresh_is_a_connection_state_not_a_crash_and_leaks_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(|_| true).await;
    let client = client(&server, &plugin(dir.path())).await;
    // The plugin worked once (the client was made); now its login lapsed.
    std::fs::write(dir.path().join("fail-from"), "2").unwrap();
    let failure = read(&client).await.expect_err("the refresh fails");
    assert_eq!(failure.kind, FailureKind::Unauthorized, "{failure:?}");
    assert!(
        !failure.message.contains("the-secret-detail"),
        "the plugin's stderr reached the message: {}",
        failure.message
    );
    assert!(
        server.tokens().is_empty(),
        "nothing was sent without a token"
    );
    // The person logs in again; the next request recovers on the same client.
    std::fs::remove_file(dir.path().join("fail-from")).unwrap();
    read(&client)
        .await
        .expect("served after the login came back");
}
