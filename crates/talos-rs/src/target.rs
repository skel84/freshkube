//! Node addresses as talosconfig and callers write them.

use std::net::IpAddr;

/// The host part of a node target, without its port.
///
/// A port is stripped only from `host:port` and `[v6]:port`; brackets are
/// dropped, since apid adds its own port to the host. A bare IPv6 address has
/// several colons and stays whole.
pub fn target_host(target: &str) -> &str {
    if let Some(rest) = target.strip_prefix('[')
        && let Some((host, _)) = rest.split_once(']')
    {
        return host;
    }
    match target.split_once(':') {
        Some((host, port)) if !port.contains(':') => host,
        _ => target,
    }
}

/// A host that names the machine making the request, never a Talos node.
pub(crate) fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_a_port_from_host_and_ipv4() {
        assert_eq!(target_host("node1:50000"), "node1");
        assert_eq!(target_host("10.5.0.2:50000"), "10.5.0.2");
    }

    #[test]
    fn keeps_hosts_without_a_port() {
        assert_eq!(target_host("node1.example.com"), "node1.example.com");
        assert_eq!(target_host("10.5.0.2"), "10.5.0.2");
    }

    #[test]
    fn keeps_a_bare_ipv6_address_whole() {
        assert_eq!(target_host("2001:db8::5"), "2001:db8::5");
        assert_eq!(target_host("::1"), "::1");
    }

    #[test]
    fn unbrackets_ipv6_with_or_without_a_port() {
        assert_eq!(target_host("[2001:db8::5]:50000"), "2001:db8::5");
        assert_eq!(target_host("[2001:db8::5]"), "2001:db8::5");
    }

    #[test]
    fn loopback_covers_names_and_both_families() {
        for host in ["localhost", "LOCALHOST", "127.0.0.1", "127.0.1.1", "::1"] {
            assert!(is_loopback(host), "{host}");
        }
        for host in ["node1", "10.5.0.2", "2001:db8::5", "localhost.example.com"] {
            assert!(!is_loopback(host), "{host}");
        }
    }
}
