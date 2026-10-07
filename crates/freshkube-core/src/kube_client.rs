//! Clients for kubeconfig-derived configs.

use std::net::IpAddr;

use kube::{Client, Config};

/// Makes a client from `config`, never sending a loopback server through a
/// proxy taken from the environment.
///
/// kube reads `HTTPS_PROXY` when the kubeconfig names no `proxy-url`, but it
/// ignores `NO_PROXY`, and without its `http-proxy` feature an `http://` proxy
/// fails the client outright. Go's HTTP client, and so kubectl, never proxies
/// loopback; neither does this. A kubeconfig's own `proxy-url` is kept.
pub(crate) fn client(mut config: Config) -> Result<Client, kube::Error> {
    skip_proxy_for_loopback(&mut config, environment_proxy().as_deref());
    Client::try_from(config)
}

/// The proxy kube takes from the environment, in kube's order.
fn environment_proxy() -> Option<String> {
    ["HTTPS_PROXY", "https_proxy"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
}

fn skip_proxy_for_loopback(config: &mut Config, environment: Option<&str>) {
    let from_environment = match (&config.proxy_url, environment) {
        (Some(proxy), Some(environment)) => {
            environment.parse::<http::Uri>().ok().as_ref() == Some(proxy)
        }
        _ => false,
    };
    if from_environment && is_loopback(&config.cluster_url) {
        config.proxy_url = None;
    }
}

fn is_loopback(url: &http::Uri) -> bool {
    let Some(host) = url.host() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
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
        let mut config = config(server, proxy);
        skip_proxy_for_loopback(&mut config, environment);
        config.proxy_url.map(|proxy| proxy.to_string())
    }

    #[test]
    fn a_loopback_server_skips_the_environment_proxy() {
        for server in [
            "http://127.0.0.1:6443",
            "https://127.8.9.10:6443",
            "https://[::1]:6443",
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

    #[tokio::test]
    async fn a_client_for_loopback_builds_behind_an_http_proxy() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut config = config("http://127.0.0.1:1", Some(PROXY));
        assert!(
            Client::try_from(config.clone()).is_err(),
            "kube without http-proxy refuses it"
        );
        skip_proxy_for_loopback(&mut config, Some(PROXY));
        assert!(Client::try_from(config).is_ok());
    }
}
