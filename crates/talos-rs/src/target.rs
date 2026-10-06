//! Node addresses as talosconfig and callers write them.

use std::net::IpAddr;

/// The host part of a node target, without its port.
///
/// A port is stripped only from `host:port` and `[v6]:port`; brackets are
/// dropped, since apid adds its own port to the host. A bare IPv6 address has
/// several colons and stays whole, zone id included. An unclosed bracket is
/// malformed and comes back unchanged, for apid to refuse rather than for
/// this to guess at.
pub fn target_host(target: &str) -> &str {
    if let Some(rest) = target.strip_prefix('[') {
        return rest.split_once(']').map_or(target, |(host, _)| host);
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
    fn keeps_an_ipv6_zone_id() {
        assert_eq!(target_host("fe80::1%en0"), "fe80::1%en0");
        assert_eq!(target_host("[fe80::1%en0]:50000"), "fe80::1%en0");
    }

    #[test]
    fn empty_and_port_only_targets_have_no_host() {
        assert_eq!(target_host(""), "");
        assert_eq!(target_host(":50000"), "");
    }

    #[test]
    fn an_unclosed_bracket_comes_back_unchanged() {
        assert_eq!(target_host("[2001:db8::5"), "[2001:db8::5");
        assert_eq!(target_host("[node1:50000"), "[node1:50000");
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
