//! What a Builds entry's Tekton Chains and Tekton Results say, in the words
//! Delivery sources' Supply chain and Results rows and the Clusters
//! Inspector's Build history group share (DESIGN.md "Words"): what was
//! looked for, where, and which of found, not installed, not allowed to read
//! and couldn't reach it is. Nothing here reads a key, a token or a Secret.
use crate::ui;
use freshkube_core::setup::Results;
use freshkube_delivery::entry::Reach;

/// A state in a few words, the four cases kept apart.
pub(super) fn state(reach: &Reach) -> &'static str {
    match reach {
        Reach::Readable => "Found",
        Reach::NotServed(_) => "Not installed",
        Reach::Refused(_) => "Not allowed to read",
        Reach::Unreachable(_) => "Couldn’t reach",
        Reach::Missing(_) => "Not in the workspace",
    }
}

/// Found is fine, a refusal needs a look, and the rest say nothing is known.
pub(super) fn tone(reach: &Reach) -> ui::Tone {
    match reach {
        Reach::Readable => ui::Tone::Good,
        Reach::Refused(_) => ui::Tone::Warn,
        _ => ui::Tone::Unknown,
    }
}

/// What looking for `what` on `entry` found. Optional tools say so: nothing
/// is wrong when one isn't installed.
fn words(what: &str, would: &str, entry: &str, reach: &Reach) -> String {
    match reach {
        Reach::Readable => format!("Found {what} on {entry}."),
        Reach::NotServed(_) => {
            format!(
                "{what} isn’t installed on {entry}. It would add {would}; the rest of Freshkube works without it."
            )
        }
        Reach::Refused(_) => {
            format!("{entry} doesn’t allow this account to read {what}.")
        }
        Reach::Unreachable(failure) => {
            format!(
                "Couldn’t reach {entry} to look for {what}: {}.",
                failure.message
            )
        }
        Reach::Missing(_) => format!("There is no entry to look for {what} on."),
    }
}

/// The sentence for Tekton Chains.
pub(super) fn chains_words(entry: &str, reach: &Reach) -> String {
    words(
        "Tekton Chains",
        "signed attestations to each build",
        entry,
        reach,
    )
}

/// The sentence for the Service Tekton Results is forwarded to.
pub(super) fn results_words(entry: &str, service: &str, reach: &Reach) -> String {
    words(
        &format!("the Tekton Results Service {service}"),
        "builds that finished long ago",
        entry,
        reach,
    )
}

/// How an entry's Results setting reads: its kind, and what it names.
pub(super) fn route(results: Option<&Results>) -> (&'static str, String) {
    match results {
        Some(Results {
            forward: Some(forward),
            ..
        }) => (
            "Port forward",
            format!("{}/{}:{}", forward.namespace, forward.service, forward.port),
        ),
        Some(Results {
            address: Some(address),
            ..
        }) => ("Address", address.clone()),
        _ => ("Off", "Tekton Results isn’t read".to_owned()),
    }
}

/// The text a Results edit starts from: the address, or `namespace/service:port`.
pub(super) fn route_text(results: Option<&Results>) -> String {
    match route(results) {
        ("Off", _) => String::new(),
        (_, text) => text,
    }
}

/// What typed text sets Results to: an `http://` or `https://` address, or
/// `namespace/service:port` to forward to. The token and certificate file of
/// the setting it replaces are kept as they are.
pub(super) fn parse(text: &str, current: Option<&Results>) -> Result<Results, &'static str> {
    let text = text.trim();
    let mut results = current.cloned().unwrap_or_default();
    if text.starts_with("http://") || text.starts_with("https://") {
        if text.contains(char::is_whitespace) || text.split("://").nth(1).is_none_or(str::is_empty)
        {
            return Err("Enter an address such as https://results.acme.example");
        }
        results.address = Some(text.to_owned());
        results.forward = None;
        return Ok(results);
    }
    let shape = "Enter an https:// address, or namespace/service:port";
    let (namespace, rest) = text.split_once('/').ok_or(shape)?;
    let (service, port) = rest.rsplit_once(':').ok_or(shape)?;
    let port = port
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or("Enter a port from 1 to 65535")?;
    if namespace.is_empty() || service.is_empty() || text.contains(char::is_whitespace) {
        return Err(shape);
    }
    // A forward already there keeps the keys this build doesn't know.
    let mut forward = results
        .forward
        .take()
        .unwrap_or_else(|| freshkube_core::setup::Forward::new(namespace, service, port));
    forward.namespace = namespace.to_owned();
    forward.service = service.to_owned();
    forward.port = port;
    results.forward = Some(forward);
    results.address = None;
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_becomes_an_address_or_a_forward_and_keeps_the_rest() {
        let mut current = Results::default();
        current.token = Some(freshkube_core::setup::KeysJson);
        current.ca_file = Some("/etc/acme/ca.pem".into());
        let address = parse(" https://results.acme.example ", Some(&current)).unwrap();
        assert_eq!(
            address.address.as_deref(),
            Some("https://results.acme.example")
        );
        assert!(address.token.is_some() && address.ca_file.is_some());
        let forward = parse("tekton/results-api:8080", Some(&address)).unwrap();
        assert!(forward.address.is_none());
        assert_eq!(forward.forward.unwrap().port, 8080);
        for bad in [
            "", "https://", "results", "ns/svc", "ns/svc:0", "/svc:80", "ns/svc:x",
        ] {
            assert!(parse(bad, None).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_four_states_are_kept_apart() {
        let all = [
            Reach::Readable,
            Reach::not_served("x"),
            Reach::refused("x"),
            Reach::unreachable("x"),
        ];
        let words: Vec<_> = all.iter().map(state).collect();
        assert_eq!(
            words,
            [
                "Found",
                "Not installed",
                "Not allowed to read",
                "Couldn’t reach"
            ]
        );
        assert!(chains_words("prod", &all[1]).contains("rest of Freshkube works"));
    }
}
