use super::*;

#[test]
fn results_is_an_address_or_a_forward() {
    let both = with_cluster(
        "    results:\n      address: https://r.example.test\n      forward: {namespace: t, service: s, port: 1}\n",
    );
    assert_eq!(
        rule(&both),
        "the cluster “cluster-a” results must set exactly one of address or forward"
    );
    assert_eq!(
        rule(&with_cluster("    results: {}\n")),
        "the cluster “cluster-a” results must set exactly one of address or forward"
    );
    assert_eq!(
        rule(&with_cluster(
            "    results: {address: ftp://r.example.test}\n"
        )),
        "the cluster “cluster-a” results.address isn’t an http:// or https:// URL"
    );
    // A URL with a user is refused: it would hold a secret.
    assert_eq!(
        rule(&with_cluster(
            "    results: {address: 'https://u:p@r.example.test'}\n"
        )),
        "the cluster “cluster-a” results.address isn’t an http:// or https:// URL"
    );
    assert_eq!(
        rule(&with_cluster(
            "    results: {forward: {namespace: t, service: s, port: 0}}\n"
        )),
        "the cluster “cluster-a” results.forward.port isn’t a port"
    );
    assert_eq!(
        rule(&with_cluster(
            "    results: {forward: {namespace: ' ', service: s, port: 80}}\n"
        )),
        "the cluster “cluster-a” results.forward.namespace is empty"
    );
}

#[test]
fn results_credentials_are_typed_and_round_trip_on_both_routes() {
    for route in [
        "address: https://results.example.test",
        "forward: {namespace: acme, service: results, port: 8080}",
    ] {
        let text = with_cluster(&format!(
            "    results:\n      {route}\n      token: keys.json\n      ca_file: /home/acme/results-ca.pem\n      future: kept\n"
        ));
        let setup = ok(&text);
        let results = setup.workspace.clusters[0].results.as_ref().unwrap();
        assert_eq!(results.token, Some(KeysJson));
        assert_eq!(results.ca_file, Some(home("/home/acme/results-ca.pem")));
        assert_eq!(setup.unknown_keys(), vec!["cluster-a.results.future"]);
        assert_eq!(ok(&to_text(&setup)), setup);
        let invalid = refused(&text.replace("token: keys.json", "token: invented-secret"));
        assert!(matches!(invalid, Invalid::KeyValue(_)));
        assert!(!invalid.to_string().contains("invented-secret"));
        assert_eq!(
            rule(&text.replace("/home/acme/results-ca.pem", "relative.pem")),
            "the cluster “cluster-a” results.ca_file isn’t an absolute path"
        );
    }
}

#[test]
fn results_credentials_still_need_exactly_one_route() {
    for extra in [
        "      token: keys.json\n",
        "      ca_file: /home/acme/ca.pem\n",
    ] {
        for route in [
            "",
            "      address: https://results.example.test\n      forward: {namespace: acme, service: results, port: 8080}\n",
        ] {
            let text = with_cluster(&format!("    results:\n{route}{extra}"));
            assert_eq!(
                rule(&text),
                "the cluster “cluster-a” results must set exactly one of address or forward"
            );
        }
    }
}

#[test]
fn results_plain_http_is_only_saved_for_literal_loopback() {
    for address in [
        "http://results.example.test",
        "http://192.0.2.1",
        "http://localhost",
    ] {
        let text = with_cluster(&format!("    results: {{address: '{address}'}}\n"));
        assert_eq!(
            rule(&text),
            "the cluster “cluster-a” results.address: plain HTTP is allowed only on loopback"
        );
    }
    for address in [
        "https://results.example.test",
        "http://127.0.0.1:8080",
        "http://127.0.0.2",
        "http://[::1]:8080",
    ] {
        ok(&with_cluster(&format!(
            "    results: {{address: '{address}'}}\n"
        )));
    }
}

#[test]
fn destination_url_credentials_are_refused_by_strict_parse_and_save_without_echoing() {
    use crate::workspace::{Destination, Key};
    for address in [
        "https://invented-private-marker@example.test",
        "https://user:invented-private-marker@example.test",
        "https://user:invented-private-marker@[invalid",
    ] {
        let text = format!(
            "version: 1\nclusters: []\ndestinations: [{{server: '{address}', entry: cluster-a}}]\n"
        );
        let invalid = parse(text.as_bytes()).unwrap_err();
        assert!(!invalid.to_string().contains("invented-private-marker"));
        assert!(!format!("{invalid:?}").contains("invented-private-marker"));
        assert!(matches!(
            invalid,
            Invalid::Workspace(workspace::Invalid::Destination {
                fault: workspace::Fault::Userinfo,
                ..
            })
        ));
        let mut setup = Setup::default();
        setup
            .workspace
            .destinations
            .push(Destination::new(Key::Server(address.into()), "cluster-a"));
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("freshkube.yaml");
        std::fs::write(&file, "version: 1\n").unwrap();
        let before = std::fs::read(&file).unwrap();
        let error = commit(&file, &Seen::Bytes(before.clone()), &setup).unwrap_err();
        assert!(!format!("{error:?}").contains("invented-private-marker"));
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }
}
