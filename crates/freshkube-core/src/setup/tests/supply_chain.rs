use super::*;

#[test]
fn the_supply_chain_is_checked() {
    let text = |lines: &str| format!("version: 1\nsupply_chain:\n{lines}");
    assert_eq!(
        rule(&text("  public_key_file: cosign.pub\n")),
        "supply_chain.public_key_file isn’t an absolute path"
    );
    assert_eq!(
        rule(&text(
            "  evidence: {result: r, commit: commit, digest: /d}\n"
        )),
        "supply_chain.evidence.commit isn’t a JSON pointer, which starts with /"
    );
    assert_eq!(
        rule(&text("  evidence: {result: r, commit: /c, digest: ''}\n")),
        "supply_chain.evidence.digest isn’t a JSON pointer, which starts with /"
    );
    assert_eq!(
        rule(&text("  evidence: {result: '', commit: /c, digest: /d}\n")),
        "supply_chain.evidence.result is empty"
    );
    let setup = ok(&text("  transparency_log: true\n"));
    assert!(setup.supply_chain.as_ref().unwrap().transparency_log);
    let written = to_text(&setup);
    assert!(written.contains("transparency_log: true"), "{written}");
    assert_eq!(ok(&written), setup);
    // Present but empty is still a block: the person wrote it.
    assert_eq!(
        ok(&text("  {}\n")).supply_chain,
        Some(SupplyChain::default())
    );
}

#[test]
fn transparency_url_is_optional_https_and_round_trips_without_repeating_bad_values() {
    let text = |url: &str| format!("version: 1\nsupply_chain:\n  transparency_url: '{url}'\n");
    let empty = ok("version: 1\nsupply_chain: {}\n");
    assert_eq!(empty.supply_chain.unwrap().transparency_url, None);
    for url in [
        "https://rekor.example.test",
        "https://rekor.example.test:443/log/",
    ] {
        let setup = ok(&text(url));
        let chain = setup.supply_chain.as_ref().unwrap();
        assert_eq!(chain.transparency_url.as_deref(), Some(url));
        assert!(!chain.transparency_log);
        assert_eq!(ok(&to_text(&setup)), setup);
        assert!(setup.unknown_keys().is_empty());
    }
    for url in [
        "",
        "not-a-url",
        "http://127.0.0.1:8080",
        "http://rekor.example.test",
        "ftp://rekor.example.test",
        "https://invented-token@rekor.example.test",
        "https://user:invented-token@rekor.example.test",
        "https://rekor.example.test/?token=invented-token",
        "https://rekor.example.test/#entry",
    ] {
        assert_eq!(
            rule(&text(url)),
            "supply_chain.transparency_url must be an https:// URL without credentials, query or fragment"
        );
    }
}
