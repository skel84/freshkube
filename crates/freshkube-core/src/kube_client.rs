//! Clients for kubeconfig-derived configs.

use std::fmt;
use std::net::IpAddr;

use kube::{Client, Config};

/// Makes a client from `config`, never sending a server through a proxy
/// taken from the environment when Go's HTTP client, and so kubectl,
/// wouldn't: a loopback server, or one `NO_PROXY` names.
///
/// kube reads `HTTPS_PROXY` when the kubeconfig names no `proxy-url`, but it
/// ignores `NO_PROXY`. A kubeconfig's own `proxy-url` is kept, as kubectl
/// keeps it whatever `NO_PROXY` says. kube's `http-proxy` feature takes an
/// `http://` proxy, tunnelling an `https` server through `CONNECT`, so TLS
/// still runs to the server with the kubeconfig's certificates; any
/// `user:password@` in the proxy's URL goes to the proxy alone, as
/// `Proxy-Authorization`. kube refuses other schemes, `socks5://` (a feature
/// we don't build) and `https://` among them.
pub(crate) fn client(mut config: Config) -> Result<Client, ClientError> {
    let environment = environment_proxy();
    let no_proxy = environment_no_proxy();
    let from_environment = skip_environment_proxy(
        &mut config,
        environment.as_deref(),
        no_proxy.as_deref().unwrap_or(""),
    );
    Client::try_from(config).map_err(|error| ClientError {
        error,
        from_environment,
    })
}

/// Why a client couldn't be made. Its message names a proxy kube refused by its
/// scheme, host and port alone: kube's own message prints the whole URL,
/// with any credentials in it, so neither `Display` nor `Debug` shows it.
pub(crate) struct ClientError {
    error: kube::Error,
    /// The proxy came from `HTTPS_PROXY`, so `NO_PROXY` can bypass it.
    from_environment: bool,
}

impl ClientError {
    /// `fallback`, with the proxy that failed the client when one did.
    pub(crate) fn message(&self, fallback: &str) -> String {
        let proxy = match &self.error {
            kube::Error::ProxyProtocolDisabled { proxy_url, .. }
            | kube::Error::ProxyProtocolUnsupported { proxy_url } => proxy_url,
            _ => return fallback.to_owned(),
        };
        let proxy = redacted(proxy);
        if self.from_environment {
            format!(
                "{fallback}: it goes through the proxy {proxy} from HTTPS_PROXY, and Freshkube \
                 takes only http:// proxies. To connect directly, add the server's host to \
                 NO_PROXY."
            )
        } else {
            format!(
                "{fallback}: its kubeconfig sends it through the proxy {proxy} (proxy-url), \
                 and Freshkube takes only http:// proxies."
            )
        }
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message("Couldn't make a Kubernetes client"))
    }
}

impl fmt::Debug for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ClientError({self})")
    }
}

/// A proxy's scheme, host and port, leaving out any `user:password@`.
fn redacted(proxy: &http::Uri) -> String {
    let scheme = proxy.scheme_str().unwrap_or("http");
    let host = proxy.host().unwrap_or("");
    match proxy.port_u16() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    }
}

/// The proxy kube takes from the environment, in kube's order.
fn environment_proxy() -> Option<String> {
    first_set(&["HTTPS_PROXY", "https_proxy"])
}

/// `NO_PROXY`, else `no_proxy`, as Go reads them.
fn environment_no_proxy() -> Option<String> {
    first_set(&["NO_PROXY", "no_proxy"])
}

fn first_set(names: &[&str]) -> Option<String> {
    names
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
}

/// Drops the environment's proxy for a server Go wouldn't proxy, and says
/// whether the proxy left came from the environment.
///
/// `Config` doesn't keep where its proxy came from, so a proxy equal to the
/// environment's counts as the environment's, even when the kubeconfig names
/// the same one; for a server `NO_PROXY` names that only means a direct
/// connection.
fn skip_environment_proxy(config: &mut Config, environment: Option<&str>, no_proxy: &str) -> bool {
    let from_environment = match (&config.proxy_url, environment) {
        (Some(proxy), Some(environment)) => {
            environment.parse::<http::Uri>().ok().as_ref() == Some(proxy)
        }
        _ => false,
    };
    if from_environment
        && (is_loopback(&config.cluster_url) || bypasses(no_proxy, &config.cluster_url))
    {
        config.proxy_url = None;
    }
    from_environment && config.proxy_url.is_some()
}

fn is_loopback(url: &http::Uri) -> bool {
    let Some(host) = url.host() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.to_canonical().is_loopback())
}

/// Whether `no_proxy` sends `url` direct, as Go's `httpproxy` reads it: a
/// comma-separated list of `*`, IP addresses, CIDR blocks and host names,
/// each optionally with a port. A name matches itself and its subdomains;
/// one with a leading `.` or `*.` matches only its subdomains. Without a
/// port in the URL, https means 443 and http 80.
fn bypasses(no_proxy: &str, url: &http::Uri) -> bool {
    let Some(host) = url.host() else {
        return false;
    };
    // Go converts non-ASCII names to punycode first; we compare them as given.
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    let port = url
        .port_u16()
        .unwrap_or(if url.scheme_str() == Some("http") {
            80
        } else {
            443
        });
    let ip = host.parse::<IpAddr>().ok();
    no_proxy
        .split(',')
        .map(|entry| entry.trim().to_ascii_lowercase())
        .filter(|entry| !entry.is_empty())
        .any(|entry| matches_entry(&entry, &host, port, ip))
}

fn matches_entry(entry: &str, host: &str, port: u16, ip: Option<IpAddr>) -> bool {
    if entry == "*" {
        return true;
    }
    if let Some((network, bits)) = entry.split_once('/') {
        return match (network.parse::<IpAddr>(), bits.parse::<u8>(), ip) {
            (Ok(network), Ok(bits), Some(ip)) => in_block(ip, network, bits),
            _ => false,
        };
    }
    let (name, entry_port) = split_port(entry);
    if entry_port.is_some_and(|entry_port| entry_port != port) {
        return false;
    }
    if let Ok(entry_ip) = name.parse::<IpAddr>() {
        return ip.is_some_and(|ip| ip.to_canonical() == entry_ip.to_canonical());
    }
    // Go reads `*.name` as `.name`: subdomains only.
    let name = name
        .strip_prefix('*')
        .filter(|name| name.starts_with('.'))
        .unwrap_or(name);
    if name.is_empty() {
        return false;
    }
    if name.starts_with('.') {
        host.ends_with(name)
    } else {
        host == name
            || host
                .strip_suffix(name)
                .is_some_and(|rest| rest.ends_with('.'))
    }
}

/// Splits `host:port` or `[v6]:port`; a bare IPv6 address keeps its colons.
/// `[v6]` without a port is read as the address, more leniently than Go,
/// which takes it for a name that never matches and so proxies.
fn split_port(entry: &str) -> (&str, Option<u16>) {
    if let Some(rest) = entry.strip_prefix('[') {
        return match rest.split_once(']') {
            Some((host, tail)) => (host, tail.strip_prefix(':').and_then(|p| p.parse().ok())),
            None => (entry, None),
        };
    }
    match entry.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => match port.parse() {
            Ok(port) => (host, Some(port)),
            Err(_) => (entry, None),
        },
        _ => (entry, None),
    }
}

/// Whether `ip` is in `network/bits`, comparing like with like: an IPv4
/// address mapped into IPv6 counts as IPv4.
fn in_block(ip: IpAddr, network: IpAddr, bits: u8) -> bool {
    match (ip.to_canonical(), network.to_canonical()) {
        (IpAddr::V4(ip), IpAddr::V4(network)) if bits <= 32 => {
            let mask = u32::MAX.checked_shl(32 - u32::from(bits)).unwrap_or(0);
            u32::from(ip) & mask == u32::from(network) & mask
        }
        (IpAddr::V6(ip), IpAddr::V6(network)) if bits <= 128 => {
            let mask = u128::MAX.checked_shl(128 - u32::from(bits)).unwrap_or(0);
            u128::from(ip) & mask == u128::from(network) & mask
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROXY: &str = "http://proxy.invalid:3128";

    fn config(server: &str, proxy: Option<&str>) -> Config {
        let mut config = Config::new(server.parse().unwrap());
        config.proxy_url = proxy.map(|proxy| proxy.parse().unwrap());
        config
    }

    fn proxy_after(server: &str, proxy: Option<&str>, environment: Option<&str>) -> Option<String> {
        proxy_after_no_proxy(server, proxy, environment, "")
    }

    fn proxy_after_no_proxy(
        server: &str,
        proxy: Option<&str>,
        environment: Option<&str>,
        no_proxy: &str,
    ) -> Option<String> {
        let mut config = config(server, proxy);
        skip_environment_proxy(&mut config, environment, no_proxy);
        config.proxy_url.map(|proxy| proxy.to_string())
    }

    fn direct(no_proxy: &str, server: &str) -> bool {
        bypasses(no_proxy, &server.parse().unwrap())
    }

    #[test]
    fn a_loopback_server_skips_the_environment_proxy() {
        for server in [
            "http://127.0.0.1:6443",
            "https://127.8.9.10:6443",
            "https://[::1]:6443",
            "https://[::ffff:127.0.0.1]:6443",
            "https://localhost:6443",
            "https://LOCALHOST",
        ] {
            assert_eq!(
                proxy_after(server, Some(PROXY), Some(PROXY)),
                None,
                "{server}"
            );
        }
    }

    #[test]
    fn other_servers_keep_the_environment_proxy() {
        for server in [
            "https://10.0.0.1:6443",
            "https://[fd00::1]:6443",
            "https://cluster.example:6443",
            "https://localhost.example:6443",
        ] {
            assert!(
                proxy_after(server, Some(PROXY), Some(PROXY)).is_some(),
                "{server}"
            );
        }
    }

    #[test]
    fn a_kubeconfig_proxy_is_kept_even_for_loopback() {
        let own = "http://own-proxy.invalid:8080";
        assert!(proxy_after("https://127.0.0.1:6443", Some(own), Some(PROXY)).is_some());
        assert!(proxy_after("https://127.0.0.1:6443", Some(own), None).is_some());
    }

    #[test]
    fn no_proxy_names_hosts_and_their_subdomains() {
        let no_proxy = "example.test";
        assert!(direct(no_proxy, "https://example.test:6443"));
        assert!(direct(no_proxy, "https://api.example.test:6443"));
        assert!(direct(no_proxy, "https://API.Example.TEST"));
        assert!(!direct(no_proxy, "https://notexample.test:6443"));
        assert!(!direct(no_proxy, "https://example.test.other:6443"));
    }

    #[test]
    fn a_leading_dot_or_star_dot_names_only_subdomains() {
        assert!(direct(".example.test", "https://api.example.test"));
        assert!(!direct(".example.test", "https://example.test"));
        // Go reads `*.name` as `.name`.
        assert!(direct("*.example.test", "https://api.example.test"));
        assert!(!direct("*.example.test", "https://example.test"));
    }

    #[test]
    fn no_proxy_takes_addresses_blocks_and_star() {
        assert!(direct("10.1.2.3", "https://10.1.2.3:6443"));
        assert!(!direct("10.1.2.3", "https://10.1.2.4:6443"));
        assert!(direct("10.0.0.0/8", "https://10.200.3.4:6443"));
        assert!(!direct("10.0.0.0/8", "https://11.0.0.1:6443"));
        assert!(direct("192.0.2.0/24", "https://[::ffff:192.0.2.7]:6443"));
        assert!(direct("fd00::/8", "https://[fd12::1]:6443"));
        assert!(!direct("fd00::/8", "https://[fe80::1]:6443"));
        assert!(direct("fd00::1", "https://[fd00::1]:6443"));
        assert!(direct("[fd00::1]:6443", "https://[fd00::1]:6443"));
        // Deliberately more lenient than kubectl, which proxies here.
        assert!(direct("[fd00::1]", "https://[fd00::1]:6443"));
        assert!(direct("0.0.0.0/0", "https://203.0.113.9"));
        assert!(direct("*", "https://anything.example.test"));
        assert!(direct("*", "https://198.51.100.1"));
        // A block never matches a name, nor a name an address.
        assert!(!direct("example.test", "https://10.0.0.1"));
        assert!(!direct("10.0.0.0/8", "https://api.example.test"));
    }

    #[test]
    fn a_port_in_no_proxy_must_match_the_servers() {
        assert!(direct(
            "api.example.test:6443",
            "https://api.example.test:6443"
        ));
        assert!(!direct(
            "api.example.test:6443",
            "https://api.example.test:8443"
        ));
        // Without a port in the URL, https means 443 and http 80.
        assert!(direct("api.example.test:443", "https://api.example.test"));
        assert!(direct("api.example.test:80", "http://api.example.test"));
        assert!(direct("10.0.0.1:6443", "https://10.0.0.1:6443"));
        assert!(!direct("10.0.0.1:6443", "https://10.0.0.1:443"));
    }

    #[test]
    fn no_proxy_lists_skip_blanks_and_spaces() {
        let no_proxy = " , other.test ,, example.test ,";
        assert!(direct(no_proxy, "https://api.example.test"));
        assert!(direct(no_proxy, "https://other.test"));
        assert!(!direct(no_proxy, "https://third.test"));
        assert!(!direct("", "https://example.test"));
    }

    #[test]
    fn no_proxy_drops_only_the_environment_proxy() {
        let server = "https://api.example.test:6443";
        assert_eq!(
            proxy_after_no_proxy(server, Some(PROXY), Some(PROXY), "example.test"),
            None
        );
        assert!(proxy_after_no_proxy(server, Some(PROXY), Some(PROXY), "other.test").is_some());
        // A kubeconfig's own proxy-url wins over NO_PROXY, as in kubectl.
        let own = "http://own-proxy.invalid:8080";
        assert!(proxy_after_no_proxy(server, Some(own), Some(PROXY), "example.test").is_some());
    }

    #[test]
    fn the_error_names_a_refused_proxy_without_its_credentials() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        for secret in [
            "socks5://someone:hunter2@proxy.invalid:1080",
            "https://someone:hunter2@proxy.invalid:3128",
        ] {
            let mut environment = config("https://api.example.test:6443", Some(secret));
            let from_environment = skip_environment_proxy(&mut environment, Some(secret), "");
            assert!(from_environment);
            let error = ClientError {
                error: Client::try_from(environment).err().unwrap(),
                from_environment,
            };
            let message = error.message("Couldn't make a client for context 'example'");
            let shown = &secret[..secret.find("://").unwrap() + 3];
            assert!(
                message.contains(&format!("{shown}proxy.invalid:")),
                "{message}"
            );
            assert!(message.contains("HTTPS_PROXY") && message.contains("NO_PROXY"));
            assert!(message.contains("only http://"), "{message}");
            assert!(!message.contains("someone") && !message.contains("hunter2"));
            assert!(!error.to_string().contains("hunter2"));
            assert!(!format!("{error:?}").contains("hunter2"));

            let own = config("https://api.example.test:6443", Some(secret));
            let error = ClientError {
                error: Client::try_from(own).err().unwrap(),
                from_environment: false,
            };
            let message = error.message("Couldn't make a client");
            assert!(message.contains("proxy-url") && !message.contains("NO_PROXY"));
            assert!(!message.contains("hunter2"), "{message}");
        }
    }

    #[tokio::test]
    async fn an_http_proxy_makes_a_client() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let config = config("https://api.example.test:6443", Some(PROXY));
        assert!(Client::try_from(config).is_ok());
    }

    /// A one-connection proxy on loopback: it keeps the request head it is
    /// sent and answers with `reply`.
    async fn proxy(reply: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let head = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            let mut buffer = [0; 1024];
            while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0, "the client closed before its request ended");
                head.extend_from_slice(&buffer[..read]);
            }
            stream.write_all(reply.as_bytes()).await.unwrap();
            stream.shutdown().await.unwrap();
            String::from_utf8_lossy(&head).into_owned()
        });
        (format!("{address}"), head)
    }

    fn proxied(server: &str, proxy: &str) -> Client {
        let mut config = config(server, Some(proxy));
        config.connect_timeout = Some(std::time::Duration::from_secs(5));
        config.read_timeout = Some(std::time::Duration::from_secs(5));
        Client::try_from(config).unwrap()
    }

    #[tokio::test]
    async fn an_https_server_tunnels_through_the_proxy_with_its_credentials() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (address, head) = proxy("HTTP/1.1 407 Proxy Authentication Required\r\n\r\n").await;
        let client = proxied(
            "https://api.example.test:6443",
            &format!("http://someone:hunter2@{address}"),
        );
        let error = client.apiserver_version().await.unwrap_err();
        let error = crate::resources::Failure::from_kube(error).to_string();
        let head = head.await.unwrap();
        assert!(
            head.starts_with("CONNECT api.example.test:6443 HTTP/1.1\r\n"),
            "{head}"
        );
        // base64("someone:hunter2"), for the proxy alone.
        assert!(
            head.to_ascii_lowercase()
                .contains("proxy-authorization: basic c29tzw9uztpodw50zxiy"),
            "{head}"
        );
        assert!(error.contains("407"), "{error}");
        assert!(!error.contains("hunter2"), "{error}");
    }

    #[tokio::test]
    async fn an_http_server_is_reached_through_the_proxy() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        const VERSION: &str = r#"{"major":"1","minor":"33","gitVersion":"v1.33.0","gitCommit":"","gitTreeState":"","buildDate":"","goVersion":"","compiler":"","platform":""}"#;
        let reply: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{VERSION}",
                VERSION.len()
            )
            .into_boxed_str(),
        );
        let (address, head) = proxy(reply).await;
        let client = proxied("http://api.example.test:6443", &format!("http://{address}"));
        let version = client.apiserver_version().await.unwrap();
        assert_eq!(version.git_version, "v1.33.0");
        let head = head.await.unwrap();
        assert!(
            head.starts_with("GET http://api.example.test:6443/version"),
            "{head}"
        );
    }

    #[tokio::test]
    async fn a_client_for_loopback_skips_an_http_proxy() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut config = config("http://127.0.0.1:1", Some(PROXY));
        skip_environment_proxy(&mut config, Some(PROXY), "");
        assert_eq!(config.proxy_url, None);
        assert!(Client::try_from(config).is_ok());
    }
}
