//! Which hostnames a certificate's names cover. A name covers a host when
//! they are equal (case aside) or when the name is `*.suffix` and the host is
//! exactly one label below `suffix`, as TLS reads a wildcard: `*.example.com`
//! covers `shop.example.com`, and neither `example.com` nor
//! `a.shop.example.com`. A host that is itself a wildcard (a listener's
//! `*.example.com`) is covered only by the same wildcard.

pub fn covers(name: &str, host: &str) -> bool {
    let (name, host) = (name.trim_end_matches('.'), host.trim_end_matches('.'));
    if name.eq_ignore_ascii_case(host) {
        return true;
    }
    if host.starts_with('*') {
        return false;
    }
    let Some(suffix) = name.strip_prefix("*.") else {
        return false;
    };
    let Some(label) = host
        .len()
        .checked_sub(suffix.len() + 1)
        .and_then(|cut| host.get(..cut))
    else {
        return false;
    };
    host[label.len()..]
        .strip_prefix('.')
        .is_some_and(|rest| rest.eq_ignore_ascii_case(suffix))
        && !label.is_empty()
        && !label.contains('.')
}

/// The hosts, in order and once each, that no name covers.
pub fn uncovered(names: &[String], hosts: &[String]) -> Vec<String> {
    let mut missing: Vec<String> = Vec::new();
    for host in hosts {
        if !names.iter().any(|name| covers(name, host)) && !missing.contains(host) {
            missing.push(host.clone());
        }
    }
    missing
}
